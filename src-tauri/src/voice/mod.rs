//! Voice: talk to Aloe and hear it answer, without needing to see the screen or use the mouse.
//!
//! 1. hotkey.rs watches Left Ctrl + Left Alt. Hold it to talk, or tap it to talk hands-free:
//!    then listening ends by itself when you stop speaking (audio.rs `Endpointer`). The microphone
//!    opens at once and realtime.rs streams it to OpenAI's live transcriber through a short-lived
//!    key from the backend. The overlay glows and shows the words as they are heard.
//! 2. What was said goes to Aloe as a chat turn from this device, in one standing conversation, so
//!    it has the same local tools as a chat typed here. A few short phrases are handled here
//!    instead ("say that again", "stop", "slower": prompts.rs `local_command`).
//! 3. The reply streams into the overlay and out of the speaker (speaker.rs), a sentence at a time.
//! 4. When the turn stops for the user (a question, an email to approve, a command to allow),
//!    the prompt is read out and answered by voice (prompts.rs). If the reply ends by asking
//!    something, Aloe listens for the answer without a key press.
//!
//! Earcons mark each step for someone who can't see the glow. One session at a time: talking
//! again interrupts whatever Aloe is saying or doing, except while it is waiting for an answer,
//! when talking *is* the answer. Every async piece belongs to a generation, and a piece from an
//! older one can no longer touch the overlay.

pub mod audio;
mod backend;
pub mod hotkey;
pub mod overlay;
mod prompts;
mod realtime;
mod speaker;
mod speech;

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use serde_json::Value;
use tauri::{async_runtime::JoinHandle, AppHandle, Manager};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    oneshot,
};

use crate::config::{debug_log, save_config, AppState, VOICE_MAX_SECONDS};
use crate::models::PendingApproval;
use audio::{Capture, Earcon, Ended};
use backend::Backend;
use hotkey::HotkeyEvent;
use overlay::{Phase, PromptView, View};
use prompts::{local_command, Answer, LocalCommand, Prompt};
use speaker::Speaker;
use speech::SpeechStream;

enum Command {
    Hotkey(HotkeyEvent),
    /// The panel's ✕, Esc, or voice switched off.
    Dismiss,
    /// The longest allowed recording ran out, for the listen with this id.
    TimeUp(u64),
    /// Hands-free listening heard the speaker finish, or nobody spoke.
    Ended(u64, Ended),
    /// A prompt is about to be read out: the next thing said answers it.
    Expect { generation: u64, answer: oneshot::Sender<String> },
    /// The prompt has been read out: open the microphone for the answer.
    ListenNow(u64),
    /// The reply ended by asking something: listen for the reply hands-free.
    FollowUp(u64),
}

static COMMANDS: OnceLock<UnboundedSender<Command>> = OnceLock::new();
static GENERATION: AtomicU64 = AtomicU64::new(0);
static LISTEN_ID: AtomicU64 = AtomicU64::new(0);
/// Where a command waiting for approval goes while a voice reply is running, with that reply's
/// generation. Approvals arriving with no voice reply in progress are left to the usual UI.
static PROMPTS: Mutex<Option<(u64, UnboundedSender<Prompt>)>> = Mutex::new(None);
/// The last reply's sentences, for "say that again".
static LAST_REPLY: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Installs the keyboard watcher and starts the session loop. Called once, from setup.
pub fn start(app: &AppHandle) {
    let (commands, receiver) = unbounded_channel();
    if COMMANDS.set(commands.clone()).is_err() {
        return;
    }
    let (hotkeys, mut hotkey_events) = unbounded_channel();
    hotkey::spawn(hotkeys);
    tauri::async_runtime::spawn(async move {
        while let Some(event) = hotkey_events.recv().await {
            if commands.send(Command::Hotkey(event)).is_err() {
                return;
            }
        }
    });
    tauri::async_runtime::spawn(run(app.clone(), receiver));
}

/// Ends whatever voice is doing and hides the overlay.
pub fn dismiss() {
    send(Command::Dismiss);
}

fn send(command: Command) {
    if let Some(commands) = COMMANDS.get() {
        let _ = commands.send(command);
    }
}

/// A command just went into the approval queue (executor.rs). During a voice reply it is read out
/// and approved or denied by voice; otherwise this does nothing.
pub fn approval_queued(approval: &PendingApproval) {
    let prompts = PROMPTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((generation, sender)) = prompts.as_ref() {
        if *generation == GENERATION.load(Ordering::SeqCst) {
            let _ = sender.send(Prompt::CommandApproval { job_id: approval.job_id.clone(), command: approval.command.clone(), reason: approval.reason.clone() });
        }
    }
}

#[derive(Debug)]
enum Purpose {
    /// A new message to Aloe. `follow_up`: Aloe asked something and is listening for the reply, so
    /// silence just ends it quietly.
    Chat { follow_up: bool },
    /// The answer to a prompt the reply is waiting on.
    Answer(oneshot::Sender<String>),
}

struct Listening {
    id: u64,
    generation: u64,
    capture: Capture,
    transcribe: JoinHandle<Result<String, String>>,
    meter: JoinHandle<()>,
    backend: Backend,
    purpose: Purpose,
    hands_free: bool,
}

struct Replying {
    task: JoinHandle<()>,
    /// Set once the backend has finished the turn; until then interrupting must stop it there too.
    finished: Arc<AtomicBool>,
    backend: Backend,
    /// The conversation the turn went to, once known.
    conversation: Arc<Mutex<Option<String>>>,
}

#[derive(Default)]
struct Session {
    listening: Option<Listening>,
    replying: Option<Replying>,
    /// A prompt is being read out and the next thing said answers it.
    expecting: Option<(u64, oneshot::Sender<String>)>,
}

fn voice_on(app: &AppHandle) -> bool {
    app.state::<AppState>().config.lock().unwrap_or_else(std::sync::PoisonError::into_inner).voice_enabled
}

fn prefs(app: &AppHandle) -> crate::models::VoicePrefs {
    app.state::<AppState>().config.lock().unwrap_or_else(std::sync::PoisonError::into_inner).voice.clone()
}

async fn run(app: AppHandle, mut commands: UnboundedReceiver<Command>) {
    let speaker = Speaker::start(&app);
    {
        let app = app.clone();
        speaker.on_error(move |error| update(&app, GENERATION.load(Ordering::SeqCst), |view| view.error = Some(error)));
    }
    let mut session = Session::default();

    while let Some(command) = commands.recv().await {
        handle(&app, &speaker, &mut session, command).await;
    }
}

/// One command, start to finish. The next is not read until this returns.
async fn handle(app: &AppHandle, speaker: &Speaker, session: &mut Session, command: Command) {
    let app = app.clone();
    {
        match command {
            Command::Hotkey(HotkeyEvent::Start) => {
                if let Some((generation, answer)) = session.expecting.take() {
                    // Talking over a question being read out answers it.
                    speaker.mute();
                    session.listening = listen(&app, speaker, generation, Purpose::Answer(answer), false).await;
                } else if session.listening.is_some() {
                    // Already listening (hands-free): holding just carries on, and letting go ends it.
                } else if voice_on(&app) {
                    let generation = new_generation(session, speaker);
                    session.listening = listen(&app, speaker, generation, Purpose::Chat { follow_up: false }, false).await;
                }
            }
            Command::Hotkey(HotkeyEvent::Stop) => {
                if let Some(listening) = session.listening.take() {
                    finish(&app, speaker, session, listening);
                }
            }
            Command::Hotkey(HotkeyEvent::Tap) => {
                let tap_to_talk = prefs(&app).tap_to_talk;
                if let Some((generation, answer)) = session.expecting.take() {
                    speaker.mute();
                    session.listening = listen(&app, speaker, generation, Purpose::Answer(answer), true).await;
                } else if let Some(listening) = session.listening.take() {
                    // A second tap ends hands-free listening early.
                    if listening.hands_free {
                        finish(&app, speaker, session, listening);
                    } else {
                        session.listening = Some(listening);
                    }
                } else if speaker.is_busy() {
                    speaker.mute();
                } else if tap_to_talk && voice_on(&app) {
                    let generation = new_generation(session, speaker);
                    session.listening = listen(&app, speaker, generation, Purpose::Chat { follow_up: false }, true).await;
                } else if overlay::current().phase != Phase::Idle {
                    new_generation(session, speaker);
                    overlay::show(&app, View::default());
                }
            }
            Command::TimeUp(id) => {
                if session.listening.as_ref().is_some_and(|l| l.id == id) {
                    let listening = session.listening.take().expect("checked");
                    finish(&app, speaker, session, listening);
                }
            }
            Command::Ended(id, reason) => {
                if !session.listening.as_ref().is_some_and(|l| l.id == id) {
                    return;
                }
                let listening = session.listening.take().expect("checked");
                if reason == Ended::NoSpeech {
                    give_up(&app, speaker, listening);
                } else {
                    finish(&app, speaker, session, listening);
                }
            }
            Command::Hotkey(HotkeyEvent::Cancel) => {
                // Another key joined the chord: it was a shortcut, not speech.
                if let Some(listening) = session.listening.take() {
                    let fresh = matches!(listening.purpose, Purpose::Chat { follow_up: false });
                    let generation = listening.generation;
                    discard(listening);
                    overlay::level(&app, 0.0);
                    if fresh {
                        update(&app, generation, |view| *view = View::default());
                    }
                }
            }
            Command::Expect { generation, answer } if generation == GENERATION.load(Ordering::SeqCst) => {
                session.expecting = Some((generation, answer));
            }
            Command::Expect { .. } => {}
            Command::ListenNow(generation) => {
                if let Some((expected, answer)) = session.expecting.take() {
                    if expected == generation && session.listening.is_none() {
                        session.listening = listen(&app, speaker, generation, Purpose::Answer(answer), true).await;
                    }
                }
            }
            Command::FollowUp(generation) => {
                if generation == GENERATION.load(Ordering::SeqCst) && session.listening.is_none() && session.expecting.is_none() && voice_on(&app) {
                    // A new generation, so the finished reply's fade-out can't hide this listen.
                    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
                    session.listening = listen(&app, speaker, generation, Purpose::Chat { follow_up: true }, true).await;
                }
            }
            Command::Hotkey(HotkeyEvent::Escape) => {
                if overlay::current().phase != Phase::Idle || speaker.is_busy() {
                    dismiss_all(&app, speaker, session);
                }
            }
            Command::Dismiss => dismiss_all(&app, speaker, session),
        }
    }
}

fn dismiss_all(app: &AppHandle, speaker: &Speaker, session: &mut Session) {
    new_generation(session, speaker);
    overlay::show(app, View::default());
}

/// Ends the current session outright and starts a new generation for whatever comes next.
fn new_generation(session: &mut Session, speaker: &Speaker) -> u64 {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    speaker.mute();
    // Dropping the sender ends a prompt waiting for an answer, which then declines it.
    session.expecting = None;
    if let Some(listening) = session.listening.take() {
        discard(listening);
    }
    if let Some(reply) = session.replying.take() {
        reply.task.abort();
        let conversation = reply.conversation.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        if let (false, Some(conversation)) = (reply.finished.load(Ordering::SeqCst), conversation) {
            // Or the next voice turn would be refused as "still working on your last message".
            let backend = reply.backend;
            tauri::async_runtime::spawn(async move { backend.stop_turn(&conversation).await });
        }
    }
    generation
}

/// Drops a recording without sending it anywhere. A prompt waiting on it hears silence.
fn discard(listening: Listening) {
    listening.transcribe.abort();
    listening.meter.abort();
    listening.capture.stop();
    if let Purpose::Answer(answer) = listening.purpose {
        let _ = answer.send(String::new());
    }
}

/// Hands-free listening where nobody said anything. A prompt hears the silence and asks again; a
/// follow-up nobody answered leaves the reply up to read; a tap with no words just ends.
fn give_up(app: &AppHandle, speaker: &Speaker, listening: Listening) {
    let generation = listening.generation;
    let follow_up = matches!(listening.purpose, Purpose::Chat { follow_up: true });
    let chat = matches!(listening.purpose, Purpose::Chat { .. });
    discard(listening);
    overlay::level(app, 0.0);
    if !chat {
        return;
    }
    speaker.tone(Earcon::Stopped);
    if follow_up {
        update(app, generation, |view| view.phase = Phase::Done);
        linger(app, generation);
    } else {
        update(app, generation, |view| *view = View::default());
    }
}

/// Pushes a change to the overlay, unless a newer session has taken it over.
fn update(app: &AppHandle, generation: u64, change: impl FnOnce(&mut View)) {
    if generation != GENERATION.load(Ordering::SeqCst) {
        return;
    }
    let mut view = overlay::current();
    change(&mut view);
    overlay::show(app, view);
}

/// Shows and says an error, then fades out.
fn fail(app: &AppHandle, speaker: &Speaker, generation: u64, message: String) {
    debug_log("voice", "error", &message);
    if generation == GENERATION.load(Ordering::SeqCst) {
        speaker.tone(Earcon::Error);
        speaker.say_always(message.clone());
    }
    update(app, generation, |view| {
        view.phase = Phase::Error;
        view.error = Some(message);
        view.status = None;
        view.prompt = None;
    });
    linger(app, generation);
}

/// Fades the overlay out once it has been up as long as the user asked for.
fn linger(app: &AppHandle, generation: u64) {
    let seconds = match prefs(app).linger.as_str() {
        "short" => 6,
        "until_dismissed" => return,
        _ => 20,
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(seconds)).await;
        update(&app, generation, |view| *view = View::default());
    });
}

fn backend_for(app: &AppHandle) -> Option<Backend> {
    let state = app.state::<AppState>();
    let config = state.config.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    Backend::from_config(state.client.clone(), &config)
}

/// Opens the microphone for one thing said. `hands_free` listens until the speaker stops (or never
/// starts); otherwise until the chord is let go.
async fn listen(app: &AppHandle, speaker: &Speaker, generation: u64, purpose: Purpose, hands_free: bool) -> Option<Listening> {
    let Some(backend) = backend_for(app) else {
        let _ = place_overlay(app).await;
        fail(app, speaker, generation, "Sign in to Aloe Desktop to talk to Aloe.".to_string());
        if let Purpose::Answer(answer) = purpose {
            let _ = answer.send(String::new());
        }
        return None;
    };
    let id = LISTEN_ID.fetch_add(1, Ordering::SeqCst) + 1;

    // Hands-free, the cue plays before the microphone opens, so the transcriber never hears it and
    // the end-of-speech detector doesn't mistake it for talking. Holding, the person is already
    // talking: open at once and play the cue alongside.
    speaker.tone(Earcon::Listening);
    if hands_free {
        speaker.flush().await;
    }

    let (chunks, audio) = unbounded_channel();
    let (ended_tx, ended_rx) = oneshot::channel();
    let ended = hands_free.then_some(ended_tx);
    let capture = tauri::async_runtime::spawn_blocking(move || Capture::start(chunks, ended));
    let placing = {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || overlay::prepare(&app))
    };
    let capture = capture.await.map_err(|e| e.to_string()).and_then(|started| started);
    let _ = placing.await;

    let capture = match capture {
        Ok(capture) => capture,
        Err(error) => {
            fail(app, speaker, generation, error);
            if let Purpose::Answer(answer) = purpose {
                let _ = answer.send(String::new());
            }
            return None;
        }
    };
    let fresh = matches!(purpose, Purpose::Chat { follow_up: false });
    let answering = matches!(purpose, Purpose::Answer(_));
    update(app, generation, |view| {
        if fresh {
            *view = View { phase: Phase::Listening, ..View::default() };
        } else {
            // An answer or a follow-up: what Aloe just said stays on screen to read.
            view.phase = if answering { Phase::Asking } else { Phase::Listening };
            view.transcript.clear();
            view.error = None;
        }
    });

    if hands_free {
        tauri::async_runtime::spawn(async move {
            if let Ok(reason) = ended_rx.await {
                send(Command::Ended(id, reason));
            }
        });
    }
    let transcribe = {
        let (app, backend) = (app.clone(), backend.clone());
        tauri::async_runtime::spawn(async move {
            realtime::transcribe(&backend, audio, |text| {
                let text = text.to_string();
                update(&app, generation, |view| view.transcript = text);
            })
            .await
        })
    };
    let meter = {
        let (app, meter) = (app.clone(), capture.meter());
        tauri::async_runtime::spawn(async move {
            loop {
                overlay::level(&app, audio::read_level(&meter));
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
    };
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(VOICE_MAX_SECONDS)).await;
        send(Command::TimeUp(id));
    });
    Some(Listening { id, generation, capture, transcribe, meter, backend, purpose, hands_free })
}

/// Creates or moves the overlay windows (blocking work, so off the async threads).
async fn place_overlay(app: &AppHandle) {
    let app = app.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || overlay::prepare(&app)).await;
}

/// Listening is over: close the microphone, wait for the final words, and act on them.
fn finish(app: &AppHandle, speaker: &Speaker, session: &mut Session, listening: Listening) {
    let Listening { generation, capture, transcribe, meter, backend, purpose, .. } = listening;
    meter.abort();
    overlay::level(app, 0.0);
    // Stopping the capture closes the audio channel, which is the transcriber's cue to commit.
    capture.stop();

    match purpose {
        Purpose::Answer(answer) => {
            // The prompt that asked is waiting on this inside the reply task.
            update(app, generation, |view| view.status = Some("Got it…".to_string()));
            tauri::async_runtime::spawn(async move {
                let heard = transcribe.await.ok().and_then(Result::ok).unwrap_or_default();
                let _ = answer.send(heard);
            });
        }
        Purpose::Chat { follow_up } => {
            update(app, generation, |view| view.phase = Phase::Transcribing);
            speaker.tone(Earcon::Thinking);
            let finished = Arc::new(AtomicBool::new(false));
            let conversation = Arc::new(Mutex::new(None));
            let task = {
                let (app, speaker, backend, finished, conversation) = (app.clone(), speaker.clone(), backend.clone(), finished.clone(), conversation.clone());
                tauri::async_runtime::spawn(async move {
                    let said = match transcribe.await {
                        Ok(Ok(text)) => text,
                        Ok(Err(error)) => return fail(&app, &speaker, generation, error),
                        Err(_) => return,
                    };
                    if said.trim().is_empty() {
                        if follow_up {
                            speaker.tone(Earcon::Stopped);
                            update(&app, generation, |view| *view = View::default());
                            return;
                        }
                        return fail(&app, &speaker, generation, "I didn't catch that. Please try again.".to_string());
                    }
                    if let Some(command) = local_command(&said) {
                        return run_local_command(&app, &speaker, generation, command);
                    }
                    let conversation_id = match voice_conversation(&app) {
                        Ok(id) => id,
                        Err(error) => return fail(&app, &speaker, generation, error),
                    };
                    *conversation.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(conversation_id.clone());
                    update(&app, generation, |view| {
                        view.phase = Phase::Thinking;
                        view.transcript = said.clone();
                    });
                    match reply(&app, &speaker, generation, &backend, &conversation_id, &said, &finished).await {
                        Ok(asked) => {
                            update(&app, generation, |view| {
                                view.phase = Phase::Done;
                                view.status = None;
                                view.prompt = None;
                            });
                            if asked && prefs(&app).follow_ups {
                                send(Command::FollowUp(generation));
                            } else {
                                linger(&app, generation);
                            }
                        }
                        Err(error) => {
                            finished.store(true, Ordering::SeqCst);
                            fail(&app, &speaker, generation, error);
                        }
                    }
                })
            };
            session.replying = Some(Replying { task, finished, backend, conversation });
        }
    }
}

/// "Say that again", "stop", "slower", "faster": answered here, never sent to Aloe.
fn run_local_command(app: &AppHandle, speaker: &Speaker, generation: u64, command: LocalCommand) {
    debug_log("voice", "local_command", format!("{command:?}"));
    match command {
        LocalCommand::Repeat => {
            let sentences = LAST_REPLY.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
            if sentences.is_empty() {
                speaker.say_always("I haven't said anything yet.");
            }
            update(app, generation, |view| {
                view.phase = Phase::Speaking;
                view.transcript = "Say that again".to_string();
            });
            for sentence in sentences {
                speaker.say_always(sentence);
            }
        }
        LocalCommand::Quiet => {
            speaker.mute();
            update(app, generation, |view| *view = View::default());
            return;
        }
        LocalCommand::Slower | LocalCommand::Faster => {
            let slower = command == LocalCommand::Slower;
            let pace = {
                let state = app.state::<AppState>();
                let mut config = state.config.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let next = match (config.voice.pace.as_str(), slower) {
                    ("fast", true) => "normal",
                    (_, true) => "slow",
                    ("slow", false) => "normal",
                    (_, false) => "fast",
                };
                config.voice.pace = next.to_string();
                let _ = save_config(&config);
                next
            };
            update(app, generation, |view| view.status = Some(format!("Speaking {pace}")));
            speaker.say_always(if slower { "Okay, I'll speak more slowly." } else { "Okay, I'll speak faster." });
        }
    }
    let (app, speaker) = (app.clone(), speaker.clone());
    tauri::async_runtime::spawn(async move {
        speaker.flush().await;
        update(&app, generation, |view| view.phase = Phase::Done);
        linger(&app, generation);
    });
}

/// The standing Voice conversation's id, minted (as a client id the backend adopts) on first use.
fn voice_conversation(app: &AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let mut config = state.config.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(id) = config.voice_conversation_id.clone().filter(|id| !id.is_empty()) {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    config.voice_conversation_id = Some(id.clone());
    save_config(&config)?;
    Ok(id)
}

/// While a voice reply runs, commands waiting for approval come to it. Cleared when the reply ends
/// or is dropped, whichever way.
struct PromptRoute(u64);

impl PromptRoute {
    fn open(generation: u64) -> (PromptRoute, UnboundedReceiver<Prompt>) {
        let (sender, receiver) = unbounded_channel();
        *PROMPTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((generation, sender));
        (PromptRoute(generation), receiver)
    }
}

impl Drop for PromptRoute {
    fn drop(&mut self) {
        let mut prompts = PROMPTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if prompts.as_ref().is_some_and(|(generation, _)| *generation == self.0) {
            *prompts = None;
        }
    }
}

/// Spoken progress at most this often while Aloe works, so silence never means "is it stuck?".
const PROGRESS_EVERY: Duration = Duration::from_secs(8);

/// Streams Aloe's answer into the overlay and out of the speaker, stopping for prompts along the
/// way. Returns once everything has been said, with whether the reply ended by asking something.
async fn reply(app: &AppHandle, speaker: &Speaker, generation: u64, backend: &Backend, conversation_id: &str, message: &str, finished: &Arc<AtomicBool>) -> Result<bool, String> {
    let response = backend.chat(conversation_id, message).await?;
    let (_route, mut queued_commands) = PromptRoute::open(generation);
    LAST_REPLY.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();

    let mut stream = response.bytes_stream();
    let mut pending: Vec<u8> = Vec::new();
    let mut text = String::new();
    let mut speech = SpeechStream::default();
    let mut deferred: Vec<Prompt> = Vec::new();
    // So the first progress line comes a few seconds in, not at once.
    let mut last_progress = Instant::now() - PROGRESS_EVERY + Duration::from_secs(3);
    let say = |sentence: String| {
        LAST_REPLY.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(sentence.clone());
        speaker.say(sentence);
    };

    'stream: loop {
        let chunk = tokio::select! {
            chunk = stream.next() => chunk,
            Some(prompt) = queued_commands.recv() => {
                ask(app, speaker, generation, backend, prompt).await;
                continue;
            }
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(|e| format!("Lost the connection to Aloe: {e}"))?;
        pending.extend_from_slice(&chunk);
        while let Some(end) = pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim().strip_prefix("data: ") else { continue };
            let Ok(event) = serde_json::from_str::<Value>(data) else { continue };
            let field = |name: &str| event.get(name).and_then(Value::as_str).unwrap_or_default().to_string();
            match event.get("type").and_then(Value::as_str).unwrap_or_default() {
                "text" => {
                    let content = field("content");
                    text.push_str(&content);
                    for sentence in speech.push(&content) {
                        say(sentence);
                    }
                    let shown = text.clone();
                    update(app, generation, |view| {
                        view.phase = Phase::Speaking;
                        view.reply = shown;
                        view.status = None;
                    });
                }
                kind @ ("status" | "notice" | "tool_start") => {
                    if kind == "tool_start" {
                        // What Aloe said before a tool call is finished, but its last sentence has
                        // no whitespace after it yet to mark the end. Without this it waited out
                        // the whole tool call and came out joined to the final answer.
                        for sentence in speech.flush() {
                            say(sentence);
                        }
                    }
                    let status = if kind == "tool_start" { field("label") } else { field("content") };
                    if status.is_empty() {
                        continue;
                    }
                    if last_progress.elapsed() >= PROGRESS_EVERY && !speaker.is_busy() {
                        last_progress = Instant::now();
                        speaker.say(status.trim_end_matches(['.', '…']).to_string());
                    }
                    update(app, generation, |view| view.status = Some(status));
                }
                "widget" => match event.get("widget").and_then(Prompt::from_widget) {
                    // The turn is blocked on this card until it is answered.
                    Some(prompt @ Prompt::Question { .. }) => ask(app, speaker, generation, backend, prompt).await,
                    // Approvals don't block the turn; Aloe will say what it is waiting for first.
                    Some(prompt) => deferred.push(prompt),
                    None => {}
                },
                // The backend restarted the answer (a model fallback); what was shown is void.
                "reset_assistant" => {
                    text.clear();
                    speech = SpeechStream::default();
                    update(app, generation, |view| view.reply.clear());
                }
                "reply_complete" | "done" => break 'stream,
                _ => {}
            }
        }
    }
    finished.store(true, Ordering::SeqCst);
    for sentence in speech.flush() {
        say(sentence);
    }
    for prompt in deferred {
        ask(app, speaker, generation, backend, prompt).await;
    }
    speaker.flush().await;
    let asked = text.trim_end().trim_end_matches(['"', '\'', ')', '*']).ends_with('?');
    Ok(asked)
}

/// Reads a prompt out, takes the answer by voice (asking once more if it wasn't clear), and sends
/// the decision. Anything short of a clear answer declines: a question is cancelled, an approval
/// is refused.
async fn ask(app: &AppHandle, speaker: &Speaker, generation: u64, backend: &Backend, prompt: Prompt) {
    debug_log("voice", "prompt", format!("{prompt:?}"));
    speaker.flush().await;
    let view = match &prompt {
        Prompt::Question { question, options, .. } => PromptView {
            text: question.clone(),
            options: options.iter().map(|o| o.label.clone()).collect(),
            hint: if options.is_empty() { "Say your answer".to_string() } else { "Say the number or the name".to_string() },
        },
        Prompt::ConnectorApproval { summary, .. } => PromptView { text: summary.clone(), options: Vec::new(), hint: "Say yes or no".to_string() },
        Prompt::CommandApproval { command, reason, .. } => PromptView {
            text: if reason.trim().is_empty() { format!("Run: {command}") } else { format!("{reason}\n\nRun: {command}") },
            options: Vec::new(),
            hint: "Say yes or no".to_string(),
        },
    };
    update(app, generation, |v| {
        v.phase = Phase::Asking;
        v.prompt = Some(view);
        v.transcript.clear();
        v.status = None;
    });

    let mut answer = Answer::Unclear;
    for attempt in 0..2 {
        let (answer_tx, heard) = oneshot::channel();
        send(Command::Expect { generation, answer: answer_tx });
        speaker.tone(Earcon::YourTurn);
        speaker.say_always(if attempt == 0 { prompt.read_back() } else { prompt.ask_again().to_string() });
        speaker.flush().await;
        send(Command::ListenNow(generation));
        let Ok(heard) = heard.await else { break };
        debug_log("voice", "prompt_answer", format!("heard={heard:?}"));
        answer = prompt.understand(&heard);
        if answer != Answer::Unclear {
            break;
        }
    }

    let unclear = answer == Answer::Unclear;
    let (outcome, said) = match (&prompt, answer) {
        (Prompt::Question { id, .. }, Answer::Text(value)) => (backend.answer_question(id, &value).await, "Okay.".to_string()),
        (Prompt::Question { id, .. }, _) => (
            backend.cancel_question(id).await,
            if unclear { "I didn't get an answer, so I'll skip that question." } else { "Okay, I'll skip that." }.to_string(),
        ),
        (Prompt::ConnectorApproval { id, .. }, Answer::Yes) => (backend.decide_approval(id, true).await, "Okay, done.".to_string()),
        (Prompt::ConnectorApproval { id, .. }, _) => (
            backend.decide_approval(id, false).await,
            if unclear { "I didn't hear a yes, so I won't do it." } else { "Okay, I won't." }.to_string(),
        ),
        (Prompt::CommandApproval { job_id, .. }, Answer::Yes) => (crate::executor::resolve_pending_approval(app, job_id, true).await, "Okay, running it.".to_string()),
        (Prompt::CommandApproval { job_id, .. }, _) => (
            crate::executor::resolve_pending_approval(app, job_id, false).await,
            if unclear { "I didn't hear a yes, so I won't run it." } else { "Okay, I won't run it." }.to_string(),
        ),
    };
    match outcome {
        Ok(()) => speaker.say_always(said),
        Err(error) => {
            debug_log("voice", "prompt_error", &error);
            speaker.tone(Earcon::Error);
            speaker.say_always(error);
        }
    }
    update(app, generation, |v| {
        v.phase = Phase::Thinking;
        v.prompt = None;
        v.status = None;
    });
}
