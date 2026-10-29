//! Everything voice says, in order, through one queue: reply sentences, questions read back,
//! progress, errors and the earcons between them.
//!
//! One queue because order matters to someone listening rather than looking: a question must not
//! start over the end of the reply that led to it, and the microphone must not open until Aloe has
//! stopped talking (or it would transcribe Aloe). `flush` waits for exactly that. `mute` drops what
//! is queued so far and silences what is playing, but not what is queued after it, so stopping a
//! reply never swallows the approval question that follows.

use std::{
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
    latency,
    speech::split_for_speech,
};
use crate::config::{debug_log, AppState, VOICE_MAX_SPEAK_CHARS};
use crate::models::VoicePrefs;

/// The speaker closes the audio device after this long with nothing to say.
const IDLE_CLOSE: Duration = Duration::from_secs(5);

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
            on_error: Mutex::new(None),
        });
        tauri::async_runtime::spawn(run(app.clone(), receiver, shared.clone()));
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
                        latency::mark("first_audio");
                    }
                }
            }
            Item::Say { .. } => {}
        }
        shared.queued.fetch_sub(1, Ordering::SeqCst);
    }
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
            *shared.player.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(started.clone());
            Some(started)
        }
        Err(error) => {
            debug_log("voice", "speaker_error", &error);
            None
        }
    }
}
