//! Everything voice says, in order, through one queue: reply sentences, questions read back,
//! progress, errors and the earcons between them.
//!
//! One queue because order matters to someone listening rather than looking: a question must not
//! start over the end of the reply that led to it, and the microphone must not open until Aloe has
//! stopped talking (or it would transcribe Aloe). `flush` waits for exactly that. `mute` drops what
//! is queued so far and silences what is playing, but not what is queued after it, so stopping a
//! reply never swallows the approval question that follows.

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use futures_util::StreamExt;
use tauri::{AppHandle, Manager};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    oneshot,
};

use super::{
    audio::{earcon, Earcon, Pcm16Decoder, Player},
    backend::Backend,
    latency, overlay,
    speech::split_for_speech,
};
use crate::config::{debug_log, AppState, VOICE_MAX_SPEAK_CHARS};
use crate::models::VoicePrefs;

/// The speaker closes the audio device after this long with nothing to say.
const IDLE_CLOSE: Duration = Duration::from_secs(5);
/// How often the overlay hears how far through the current sentence Aloe is.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);
/// A rough speaking pace, for guessing a sentence's length while its audio is still arriving.
const SECONDS_PER_CHAR: f64 = 0.065;

/// A piece of text queued on the player, and where its audio sits in the player's samples.
struct Said {
    text: String,
    start: u64,
    end: u64,
    /// All of its audio has arrived, so `end` is final.
    complete: bool,
}

enum Item {
    /// `always`: said even with "read replies aloud" off (questions, approvals, errors).
    Say { text: String, always: bool },
    Tone(Earcon),
    Flush(oneshot::Sender<()>),
}

#[derive(Clone)]
pub struct Speaker {
    items: UnboundedSender<(u64, Item)>,
    shared: Arc<Shared>,
}

struct Shared {
    next: AtomicU64,
    /// Items numbered at or below this are dropped unsaid.
    skip_through: AtomicU64,
    queued: AtomicUsize,
    player: Mutex<Option<Arc<Player>>>,
    /// What is queued on the player and not yet heard, oldest first, so the overlay can follow along.
    said: Mutex<VecDeque<Said>>,
    /// Shown once per failure streak, not per sentence.
    on_error: Mutex<Option<Box<dyn Fn(String) + Send + Sync>>>,
}

impl Speaker {
    pub fn start(app: &AppHandle) -> Speaker {
        let (items, receiver) = unbounded_channel();
        let shared = Arc::new(Shared {
            next: AtomicU64::new(1),
            skip_through: AtomicU64::new(0),
            queued: AtomicUsize::new(0),
            player: Mutex::new(None),
            said: Mutex::new(VecDeque::new()),
            on_error: Mutex::new(None),
        });
        tauri::async_runtime::spawn(run(app.clone(), receiver, shared.clone()));
        tauri::async_runtime::spawn(follow(app.clone(), shared.clone()));
        Speaker { items, shared }
    }

    fn push(&self, item: Item) {
        let number = self.shared.next.fetch_add(1, Ordering::SeqCst);
        self.shared.queued.fetch_add(1, Ordering::SeqCst);
        if self.items.send((number, item)).is_err() {
            self.shared.queued.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// A sentence of a reply, or progress: skipped when reading aloud is off.
    pub fn say(&self, text: impl Into<String>) {
        self.push(Item::Say { text: text.into(), always: false });
    }

    /// A question, an approval, an error: the user can't act without hearing it.
    pub fn say_always(&self, text: impl Into<String>) {
        self.push(Item::Say { text: text.into(), always: true });
    }

    pub fn tone(&self, kind: Earcon) {
        self.push(Item::Tone(kind));
    }

    /// Resolves once everything queued before it has been heard (or skipped).
    pub async fn flush(&self) {
        let (done, heard) = oneshot::channel();
        self.push(Item::Flush(done));
        let _ = heard.await;
    }

    /// Stops what is playing and drops what is queued, but not anything queued after this call.
    pub fn mute(&self) {
        let last = self.shared.next.load(Ordering::SeqCst).saturating_sub(1);
        self.shared.skip_through.fetch_max(last, Ordering::SeqCst);
        if let Some(player) = self.shared.player.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
            player.clear();
        }
        lock(&self.shared.said).clear();
    }

    /// Talking, or about to.
    pub fn is_busy(&self) -> bool {
        let playing = self.shared.player.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().is_some_and(|p| !p.is_idle());
        playing || self.shared.queued.load(Ordering::SeqCst) > 0
    }

    pub fn on_error(&self, report: impl Fn(String) + Send + Sync + 'static) {
        *self.shared.on_error.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Box::new(report));
    }
}

async fn run(app: AppHandle, mut items: UnboundedReceiver<(u64, Item)>, shared: Arc<Shared>) {
    let mut failing = false;
    let mut samples = Vec::new();
    loop {
        let next = match tokio::time::timeout(IDLE_CLOSE, items.recv()).await {
            Ok(Some(next)) => next,
            Ok(None) => return,
            Err(_) => {
                // Nothing to say for a while: let the audio device go.
                let mut player = shared.player.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if player.as_ref().is_some_and(|p| p.is_idle()) {
                    *player = None;
                }
                continue;
            }
        };
        let (number, item) = next;
        let skipped = || number <= shared.skip_through.load(Ordering::SeqCst);
        let (prefs, backend) = settings(&app);
        match item {
            Item::Flush(done) => {
                while !skipped() && player(&shared).is_some_and(|p| !p.is_idle()) {
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
                // The device still holds its own last buffer after the queue empties.
                if player(&shared).is_some() && !skipped() {
                    tokio::time::sleep(Duration::from_millis(180)).await;
                }
                let _ = done.send(());
            }
            Item::Tone(kind) if prefs.sound_cues && !skipped() => {
                if let Some(player) = ensure_player(&shared).await {
                    player.push(&earcon(kind, prefs.cue_volume));
                }
            }
            Item::Tone(_) => {}
            Item::Say { text, always } if !skipped() && (always || prefs.read_aloud) && backend.is_some() => {
                let backend = backend.expect("checked above");
                for part in split_for_speech(&text, VOICE_MAX_SPEAK_CHARS) {
                    if skipped() {
                        break;
                    }
                    latency::mark("speak_sent");
                    let response = match backend.speak(&part, &prefs.pace).await {
                        Ok(response) => {
                            latency::mark("speak_headers");
                            failing = false;
                            response
                        }
                        Err(error) => {
                            debug_log("voice", "speak_error", &error);
                            if !failing {
                                failing = true;
                                if let Some(report) = shared.on_error.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
                                    report(error);
                                }
                                // Something the listener can still hear when speech itself is down.
                                if let Some(player) = ensure_player(&shared).await {
                                    player.push(&earcon(Earcon::Error, prefs.cue_volume.max(0.3)));
                                }
                            }
                            break;
                        }
                    };
                    let Some(player) = ensure_player(&shared).await else { break };
                    let start = player.pushed();
                    lock(&shared.said).push_back(Said { text: part.clone(), start, end: start, complete: false });
                    let mut decoder = Pcm16Decoder::default();
                    let mut body = response.bytes_stream();
                    while let Some(Ok(chunk)) = body.next().await {
                        if skipped() {
                            player.clear();
                            break;
                        }
                        samples.clear();
                        decoder.push(&chunk, &mut samples);
                        player.push(&samples);
                        if let Some(said) = lock(&shared.said).iter_mut().rev().find(|said| said.start == start) {
                            said.end = player.pushed();
                        }
                        latency::mark("first_audio");
                    }
                    if let Some(said) = lock(&shared.said).iter_mut().rev().find(|said| said.start == start) {
                        said.end = player.pushed();
                        said.complete = true;
                    }
                }
            }
            Item::Say { .. } => {}
        }
        shared.queued.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Tells the overlay which text is being heard and how far into it the voice is, so it can light
/// up the words as they are said. Silent while nothing is playing.
async fn follow(app: AppHandle, shared: Arc<Shared>) {
    let mut last: Option<(String, u16)> = None;
    loop {
        tokio::time::sleep(PROGRESS_INTERVAL).await;
        let now = hearing(&shared).map(|(text, at)| (text, (at * 1000.0).round() as u16));
        if now == last {
            continue;
        }
        overlay::spoken(&app, now.as_ref().map(|(text, at)| (text.as_str(), f32::from(*at) / 1000.0)));
        last = now;
    }
}

/// The text playing right now, and how far through it playback is, 0–1.
fn hearing(shared: &Shared) -> Option<(String, f64)> {
    let player = player(shared)?;
    let played = player.played();
    let mut said = lock(&shared.said);
    while said.front().is_some_and(|front| front.complete && front.end <= played) {
        said.pop_front();
    }
    let current = said.front()?;
    if played < current.start || current.end <= current.start {
        // An earcon, or a sentence whose audio hasn't started arriving.
        return None;
    }
    let heard = (played - current.start) as f64;
    let length = (current.end - current.start) as f64;
    let at = if current.complete {
        heard / length
    } else {
        // Still downloading: guess its full length, and never claim the end before it arrives.
        let guess = current.text.chars().count() as f64 * SECONDS_PER_CHAR * f64::from(player.rate());
        (heard / length.max(guess)).min(0.95)
    };
    Some((current.text.clone(), at.clamp(0.0, 1.0)))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn settings(app: &AppHandle) -> (VoicePrefs, Option<Backend>) {
    let state = app.state::<AppState>();
    let config = state.config.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    (config.voice.clone(), Backend::from_config(state.client.clone(), &config))
}

fn player(shared: &Shared) -> Option<Arc<Player>> {
    shared.player.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

async fn ensure_player(shared: &Arc<Shared>) -> Option<Arc<Player>> {
    if let Some(player) = player(shared) {
        return Some(player);
    }
    match tauri::async_runtime::spawn_blocking(Player::start).await.map_err(|e| e.to_string()).and_then(|started| started) {
        Ok(started) => {
            let started = Arc::new(started);
            // Positions on the old player mean nothing on the new one.
            lock(&shared.said).clear();
            *shared.player.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(started.clone());
            Some(started)
        }
        Err(error) => {
            debug_log("voice", "speaker_error", &error);
            None
        }
    }
}
