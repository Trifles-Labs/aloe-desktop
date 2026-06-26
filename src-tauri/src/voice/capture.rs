use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures_util::StreamExt;
use tauri::{AppHandle, Emitter, Manager};
use vosk::{DecodingState, Model, Recognizer};

use super::wakeword;
use crate::config::AppState;

pub const WAKE_WORD_EVENT: &str = "wake-word-detected";
pub const VOICE_THINKING_EVENT: &str = "voice-thinking";
pub const VOICE_STATUS_EVENT: &str = "voice-status";
pub const VOICE_RESPONSE_EVENT: &str = "voice-response";
pub const VOICE_ERROR_EVENT: &str = "voice-error";
const ORB_WINDOW_LABEL: &str = "orb";
/// Safety net only: normally a natural pause in speech finalizes the question via Vosk's
/// own endpointer well before this.
const MAX_CAPTURE_DURATION: Duration = Duration::from_secs(15);
/// Below this much audio there's not enough to bother transcribing.
const MIN_CAPTURE_DURATION_SECS: f32 = 0.3;
/// Transcription is a single short audio clip — it should always be fast, so this timeout
/// is just a safety net against a stalled connection.
const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(60);
/// The chat call can legitimately run long on complex, multi-tool-call agentic turns, so it
/// gets a much more generous bound than transcription — just enough to stop a truly hung
/// connection from waiting forever.
const CHAT_TIMEOUT: Duration = Duration::from_secs(180);
/// Defensive auto-hide watchdog, renewed on every wake word / auto-relisten so it only fires
/// after this much *inactivity*, not as a cap on total voice-mode session length. Comfortably
/// longer than TRANSCRIBE_TIMEOUT + CHAT_TIMEOUT combined so it never races a real, if slow,
/// turn — the frontend's own hide-after-response is what normally fires first.
const ORB_FALLBACK_AUTO_HIDE: Duration = Duration::from_secs(270);

pub(super) fn log(event: &str, detail: impl AsRef<str>) {
    println!("[voice] {event} {}", detail.as_ref());
}

/// Owns the background thread that keeps the microphone stream and Vosk recognizers alive.
/// The thread is kept alive by simply not letting the `cpal::Stream` (which isn't safe to
/// hand around between threads) drop until `stop` is observed.
pub struct VoiceEngineHandle {
    stop: Arc<AtomicBool>,
    muted: Arc<AtomicBool>,
    shared: ListenerShared,
    thread: Option<JoinHandle<()>>,
}

impl VoiceEngineHandle {
    pub fn set_muted(&self, muted: bool) {
        log("muted", muted.to_string());
        self.muted.store(muted, Ordering::SeqCst);
    }

    pub fn resume_listening(&self) {
        self.muted.store(false, Ordering::SeqCst);
        rearm_capturing_if_idle(&self.shared);
    }

    pub fn stop_current_turn(&self) {
        self.muted.store(false, Ordering::SeqCst);
        if let Some(handle) = self.shared.current_dispatch.lock().expect("dispatch mutex").take() {
            log("stop_current_turn", "aborting in-flight request");
            handle.abort();
        }
        reset_to_wake_word_only(&self.shared);
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Either listening for the wake word, capturing the question that follows (legacy path), or
/// streaming audio continuously to a Gemini Live session (Live path).
enum ListenerMode {
    WakeWord(Recognizer),
    Capturing { recognizer: Recognizer, buffer: Vec<i16>, started_at: Instant },
    /// Active Gemini Live session: audio is forwarded directly over WebSocket without buffering.
    Live { pcm_sender: tokio::sync::mpsc::Sender<Vec<i16>> },
}

/// Shared handles every closure/async task needs to read or mutate listener state. Bundled
/// into one struct instead of threading five separate Arcs through every function signature.
#[derive(Clone)]
pub(super) struct ListenerShared {
    app: AppHandle,
    mode: Arc<Mutex<ListenerMode>>,
    model: Arc<Model>,
    sample_rate: f32,
    conversation_id: Arc<String>,
    /// Bumped on every wake word / auto-relisten; used to make the orb auto-hide watchdog
    /// renewable instead of a one-shot timer that could fire mid-conversation.
    activity: Arc<AtomicU64>,
    /// The in-flight transcribe/chat/speak task, if any — aborted whenever a fresh wake word
    /// interrupts, so an old request can't keep generating (or speaking) after the user has
    /// already moved on to a new question. Dropping the task mid-`.await` closes the
    /// underlying HTTP connection, which the backend observes as a client disconnect and
    /// uses to cancel the Gemini call (see `abortSignal: c.req.raw.signal` in chat.ts).
    current_dispatch: Arc<Mutex<Option<tauri::async_runtime::JoinHandle<()>>>>,
}

fn model_dir() -> Result<PathBuf, String> {
    // Dev builds resolve the model next to the crate; production bundling still needs
    // this wired through Tauri's resource resolver (tracked as Phase 1 follow-up).
    let dev_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/vosk-model-small-en-us-0.15");
    if dev_path.exists() {
        return Ok(dev_path);
    }
    Err("Vosk model not found. Run scripts/fetch-voice-assets.ps1 first.".to_string())
}

/// Voice conversations are meant to be one ongoing thread, not a fresh one every time the
/// listener (re)starts — reuse the persisted id if we already have one, otherwise mint a new
/// one and save it so it survives toggling the setting off/on or restarting the app.
fn resolve_conversation_id(app: &AppHandle) -> String {
    let state = app.state::<AppState>();
    let mut config = state.config.lock().expect("config mutex");
    if let Some(id) = &config.voice_conversation_id {
        return id.clone();
    }
    let id = uuid::Uuid::new_v4().to_string();
    config.voice_conversation_id = Some(id.clone());
    if let Err(error) = crate::config::save_config(&config) {
        log("save_config_failed", &error);
    }
    id
}

pub fn spawn_listener(app: AppHandle) -> Result<VoiceEngineHandle, String> {
    let model_path = model_dir()?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = stop.clone();
    let muted = Arc::new(AtomicBool::new(false));
    let muted_for_thread = muted.clone();
    let conversation_id = resolve_conversation_id(&app);
    let (ready_tx, ready_rx) = mpsc::channel::<Result<ListenerShared, String>>();

    log("spawn_listener", format!("conversation_id={conversation_id}"));

    let thread = std::thread::Builder::new()
        .name("aloe-voice-listener".into())
        .spawn(move || run_listener(app, model_path, conversation_id, stop_for_thread, muted_for_thread, ready_tx))
        .map_err(|e| e.to_string())?;

    let shared = ready_rx
        .recv()
        .map_err(|_| "Voice listener thread exited before starting.".to_string())??;

    Ok(VoiceEngineHandle { stop, muted, shared, thread: Some(thread) })
}

fn run_listener(
    app: AppHandle,
    model_path: PathBuf,
    conversation_id: String,
    stop: Arc<AtomicBool>,
    muted: Arc<AtomicBool>,
    ready_tx: mpsc::Sender<Result<ListenerShared, String>>,
) {
    let setup = (|| -> Result<(cpal::Stream, ListenerShared), String> {
        log("model", format!("loading from {}", model_path.display()));
        let model = Arc::new(wakeword::load_model(&model_path)?);
        let device = cpal::default_host()
            .default_input_device()
            .ok_or_else(|| "No microphone input device found.".to_string())?;
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        let sample_rate = config.sample_rate() as f32;
        let channels = config.channels();
        log("device", format!("sample_rate={sample_rate} channels={channels} format={:?}", config.sample_format()));

        let recognizer = wakeword::new_recognizer(&model, sample_rate)?;
        let shared = ListenerShared {
            app: app.clone(),
            mode: Arc::new(Mutex::new(ListenerMode::WakeWord(recognizer))),
            model: model.clone(),
            sample_rate,
            conversation_id: Arc::new(conversation_id),
            activity: Arc::new(AtomicU64::new(0)),
            current_dispatch: Arc::new(Mutex::new(None)),
        };
        let stream_config: cpal::StreamConfig = config.clone().into();
        let err_fn = |err| eprintln!("[voice] microphone stream error: {err}");

        let stream = match config.sample_format() {
            cpal::SampleFormat::I16 => {
                let shared = shared.clone();
                let muted = muted.clone();
                device.build_input_stream(
                    stream_config.clone(),
                    move |data: &[i16], _| process_samples(&shared, &muted, data, channels),
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I32 => {
                let shared = shared.clone();
                let muted = muted.clone();
                device.build_input_stream(
                    stream_config.clone(),
                    move |data: &[i32], _| {
                        let converted: Vec<i16> = data.iter().map(|&s| (s >> 16) as i16).collect();
                        process_samples(&shared, &muted, &converted, channels);
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::F32 => {
                let shared = shared.clone();
                let muted = muted.clone();
                device.build_input_stream(
                    stream_config.clone(),
                    move |data: &[f32], _| {
                        let converted: Vec<i16> = data
                            .iter()
                            .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                            .collect();
                        process_samples(&shared, &muted, &converted, channels);
                    },
                    err_fn,
                    None,
                )
            }
            other => return Err(format!("Unsupported microphone sample format: {other:?}")),
        }
        .map_err(|e| e.to_string())?;

        stream.play().map_err(|e| e.to_string())?;
        log("listening", "wake-word stream started");
        Ok((stream, shared))
    })();

    let (stream, _shared) = match setup {
        Ok(pair) => {
            let _ = ready_tx.send(Ok(pair.1.clone()));
            pair
        }
        Err(err) => {
            log("setup_failed", &err);
            let _ = ready_tx.send(Err(err));
            return;
        }
    };

    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(150));
    }
    log("listening", "stopped");
    drop(stream);
}

fn process_samples(shared: &ListenerShared, muted: &Arc<AtomicBool>, data: &[i16], channels: cpal::ChannelCount) {
    if muted.load(Ordering::Relaxed) {
        return;
    }

    let mono = downmix_to_mono(data, channels);
    let mut guard = match shared.mode.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };

    match &mut *guard {
        ListenerMode::WakeWord(recognizer) => {
            let state = match recognizer.accept_waveform(&mono) {
                Ok(state) => state,
                Err(_) => return,
            };
            let DecodingState::Finalized = state else { return };
            let Some(heard) = recognizer.result().single().map(|r| r.text.to_string()) else { return };
            if heard.trim().is_empty() {
                return;
            }
            log("heard", format!("{heard:?}"));
            if !wakeword::is_wake_phrase(&heard) {
                return;
            }

            log("wake_word", "detected, trying Live session first");
            drop(guard);
            on_wake_word(shared);
        }
        ListenerMode::Capturing { recognizer, buffer, started_at } => {
            buffer.extend_from_slice(&mono);
            let timed_out = started_at.elapsed() > MAX_CAPTURE_DURATION;
            let state = if timed_out {
                DecodingState::Finalized
            } else {
                match recognizer.accept_waveform(&mono) {
                    Ok(state) => state,
                    Err(_) => return,
                }
            };
            let DecodingState::Finalized = state else { return };
            if timed_out {
                log("capture", "max duration reached, finalizing");
            }

            let captured = std::mem::take(buffer);
            log("captured", format!("{} samples (~{:.1}s)", captured.len(), captured.len() as f32 / shared.sample_rate));

            // Drop back to WakeWord while the request is in flight so a fresh "Hey Aloe" can
            // interrupt; the frontend re-arms Capturing after playback actually finishes.
            // Any failure — here or in dispatch_audio — just exits voice mode back to
            // wake-word-only listening instead of re-arming, so a broken turn doesn't leave
            // the mic silently listening with no clear feedback.
            match wakeword::new_recognizer(&shared.model, shared.sample_rate) {
                Ok(wake_recognizer) => *guard = ListenerMode::WakeWord(wake_recognizer),
                Err(error) => log("wake_recognizer_failed", &error),
            }
            drop(guard);

            if (captured.len() as f32) < shared.sample_rate * MIN_CAPTURE_DURATION_SECS {
                let _ = shared.app.emit(VOICE_ERROR_EVENT, VoicePayload::text_only("I didn't catch a question.", false));
                return;
            }

            match encode_wav(&captured, shared.sample_rate as u32) {
                Ok(wav_bytes) => {
                    let shared_for_task = shared.clone();
                    let handle = tauri::async_runtime::spawn(async move {
                        dispatch_audio(shared_for_task, wav_bytes).await;
                    });
                    *shared.current_dispatch.lock().expect("dispatch mutex") = Some(handle);
                }
                Err(error) => {
                    log("encode_wav_failed", &error);
                    let _ = shared.app.emit(VOICE_ERROR_EVENT, VoicePayload::text_only("Something went wrong capturing that.", false));
                }
            }
        }
        ListenerMode::Live { pcm_sender } => {
            let resampled = super::live::resample_to_16k(&mono, shared.sample_rate);
            // try_send never blocks the audio callback; silently drop frames if channel is full
            let _ = pcm_sender.try_send(resampled);
        }
    }
}

fn encode_wav(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    {
        let cursor = std::io::Cursor::new(&mut bytes);
        let mut writer = hound::WavWriter::new(cursor, spec).map_err(|e| e.to_string())?;
        for &sample in samples {
            writer.write_sample(sample).map_err(|e| e.to_string())?;
        }
        writer.finalize().map_err(|e| e.to_string())?;
    }
    Ok(bytes)
}

fn downmix_to_mono(data: &[i16], channels: cpal::ChannelCount) -> Vec<i16> {
    let channels = channels as usize;
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks_exact(channels)
        .map(|frame| (frame.iter().map(|&s| s as i32).sum::<i32>() / channels as i32) as i16)
        .collect()
}

fn on_wake_word(shared: &ListenerShared) {
    if let Some(handle) = shared.current_dispatch.lock().expect("dispatch mutex").take() {
        log("interrupt", "aborting in-flight request");
        handle.abort();
    }

    let _ = shared.app.emit(WAKE_WORD_EVENT, ());

    let Some(orb) = shared.app.get_webview_window(ORB_WINDOW_LABEL) else { return };
    let _ = orb.show();
    let _ = orb.set_focus();
    renew_orb_watchdog(shared);

    // Spawn an async task that tries Gemini Live first (×2 with 1s gap) then falls back to
    // the legacy buffered-capture HTTP pipeline if both attempts fail.
    let shared_clone = shared.clone();
    let handle = tauri::async_runtime::spawn(async move {
        try_dispatch_live_or_fallback(shared_clone).await;
    });
    *shared.current_dispatch.lock().expect("dispatch mutex") = Some(handle);
}

async fn try_dispatch_live_or_fallback(shared: ListenerShared) {
    let (api_url, user_token) = {
        let state = shared.app.state::<AppState>();
        let config = state.config.lock().expect("config mutex");
        (config.api_url.clone(), config.user_token.clone())
    };

    let Some(user_token) = user_token else {
        log("live_dispatch", "not signed in, falling back to legacy path");
        arm_legacy_capture(&shared);
        return;
    };

    let conversation_id = (*shared.conversation_id).clone();

    match super::live::connect_live(shared.app.clone(), shared.clone(), &conversation_id, &api_url, &user_token).await {
        Ok(session) => {
            install_live_session(&shared, session);
            return;
        }
        Err(e) => log("live_connect_attempt1", &e),
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    match super::live::connect_live(shared.app.clone(), shared.clone(), &conversation_id, &api_url, &user_token).await {
        Ok(session) => install_live_session(&shared, session),
        Err(e) => {
            log("live_connect_attempt2", &e);
            arm_legacy_capture(&shared);
        }
    }
}

fn install_live_session(shared: &ListenerShared, session: super::live::LiveSession) {
    // If stop_current_turn fired between connect_live returning and here, dispatch is None —
    // don't install the session over a user-initiated dismissal.
    if shared.current_dispatch.lock().expect("dispatch mutex").is_none() {
        log("live_session", "skipped — superseded by stop_current_turn");
        session.task.abort();
        return;
    }
    {
        let mut guard = shared.mode.lock().expect("mode mutex");
        *guard = ListenerMode::Live { pcm_sender: session.pcm_sender };
    }
    *shared.current_dispatch.lock().expect("dispatch mutex") = Some(session.task);
    log("live_session", "installed, streaming started");
}

fn arm_legacy_capture(shared: &ListenerShared) {
    // If stop_current_turn fired while the Live connection was being attempted, dispatch
    // is None — don't switch to Capturing over a user-initiated dismissal.
    if shared.current_dispatch.lock().expect("dispatch mutex").is_none() {
        log("legacy_capture", "skipped — superseded by stop_current_turn");
        return;
    }
    let mut guard = shared.mode.lock().expect("mode mutex");
    match wakeword::new_open_recognizer(&shared.model, shared.sample_rate) {
        Ok(recognizer) => {
            *guard = ListenerMode::Capturing { recognizer, buffer: Vec::new(), started_at: Instant::now() };
            log("legacy_capture", "armed");
        }
        Err(e) => log("arm_legacy_capture_failed", &e),
    }
}

/// Schedules an auto-hide tied to the *current* activity generation. Renewing the watchdog
/// (on every wake word and auto-relisten) bumps the generation, so older, now-stale timers
/// no-op instead of hiding the orb out from under an active conversation.
fn renew_orb_watchdog(shared: &ListenerShared) {
    let generation = shared.activity.fetch_add(1, Ordering::SeqCst) + 1;
    let activity = shared.activity.clone();
    let Some(orb) = shared.app.get_webview_window(ORB_WINDOW_LABEL) else { return };
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(ORB_FALLBACK_AUTO_HIDE).await;
        if activity.load(Ordering::SeqCst) == generation {
            let _ = orb.hide();
        }
    });
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(super) struct VoicePayload {
    pub(super) text: String,
    pub(super) continue_listening: bool,
    pub(super) audio_base64: Option<String>,
}

impl VoicePayload {
    pub(super) fn text_only(text: impl Into<String>, continue_listening: bool) -> Self {
        Self { text: text.into(), continue_listening, audio_base64: None }
    }
}

fn emit_status(shared: &ListenerShared, status: &str) {
    log("status_emit", status);
    let _ = shared.app.emit(VOICE_STATUS_EVENT, status);
}

async fn dispatch_audio(shared: ListenerShared, wav_bytes: Vec<u8>) {
    let started_at = Instant::now();
    log("dispatch", format!("audio={} bytes", wav_bytes.len()));
    let _ = shared.app.emit(VOICE_THINKING_EVENT, ());
    emit_status(&shared, "Sending your voice to Aloe...");

    let (client, api_url, user_token) = {
        let state = shared.app.state::<AppState>();
        let config = state.config.lock().expect("config mutex");
        (state.client.clone(), config.api_url.clone(), config.user_token.clone())
    };

    let Some(user_token) = user_token else {
        log("dispatch_failed", "not signed in");
        let _ = shared.app.emit(VOICE_ERROR_EVENT, VoicePayload::text_only("Aloe Desktop isn't signed in.", false));
        return;
    };

    match send_chat_message(&shared, &client, &api_url, &user_token, &wav_bytes).await {
        Ok(result) if !result.text.trim().is_empty() => {
            log("response", format!("{} chars ended={}", result.text.len(), result.ended));
            emit_status(&shared, "Preparing spoken reply...");
            let audio_base64 = match synthesize_speech(&client, &api_url, &user_token, &result.text).await {
                Ok(audio) => Some(audio),
                Err(error) => {
                    log("speak_failed", &error);
                    None
                }
            };
            log("dispatch_done", format!("elapsed_ms={} audio_base64={}", started_at.elapsed().as_millis(), audio_base64.as_ref().map(|audio| audio.len()).unwrap_or(0)));
            let _ = shared.app.emit(VOICE_RESPONSE_EVENT, VoicePayload { text: result.text, continue_listening: !result.ended, audio_base64 });
        }
        Ok(_) => {
            log("response", "empty");
            let _ = shared.app.emit(VOICE_ERROR_EVENT, VoicePayload::text_only("Aloe didn't return a response.", false));
        }
        Err(error) => {
            log("dispatch_failed", &error);
            let _ = shared.app.emit(VOICE_ERROR_EVENT, VoicePayload::text_only(error, false));
        }
    }
}

/// Continues voice mode by switching back to capturing a follow-up question, but only if
/// nothing else has happened to the listener since we left it in `WakeWord` mode right after
/// the previous capture finalized — e.g. if the user already interrupted with a fresh wake
/// word, that newer capture is left alone instead of being clobbered by this older turn.
fn rearm_capturing_if_idle(shared: &ListenerShared) {
    let mut guard = match shared.mode.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    if !matches!(&*guard, ListenerMode::WakeWord(_)) {
        return;
    }
    match wakeword::new_open_recognizer(&shared.model, shared.sample_rate) {
        Ok(recognizer) => {
            *guard = ListenerMode::Capturing { recognizer, buffer: Vec::new(), started_at: Instant::now() };
            log("auto_relisten", "continuing voice mode");
        }
        Err(error) => {
            log("auto_relisten_failed", &error);
            return;
        }
    }
    drop(guard);
    renew_orb_watchdog(shared);
}

pub(super) fn reset_to_wake_word_only(shared: &ListenerShared) {
    let mut guard = match shared.mode.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    match wakeword::new_recognizer(&shared.model, shared.sample_rate) {
        Ok(recognizer) => {
            *guard = ListenerMode::WakeWord(recognizer);
            shared.activity.fetch_add(1, Ordering::SeqCst);
            log("wake_word_only", "reset listener");
        }
        Err(error) => log("wake_recognizer_failed", &error),
    }
}

#[derive(serde::Deserialize)]
struct ErrorBody {
    error: String,
}

/// Backend error responses are `{"error": "..."}` with an already-cleaned message (see
/// `extractGeminiErrorMessage` server-side). Anything that doesn't parse falls back to a
/// short, generic message instead of dumping a raw response body — this text can end up
/// read aloud by the orb's TTS, so it needs to stay speakable.
fn clean_http_error(status: reqwest::StatusCode, body: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<ErrorBody>(body) {
        return parsed.error;
    }
    format!("the request failed with status {status}")
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpeakResponse {
    audio_base64: String,
}

/// Synthesizes Aloe's reply with Gemini TTS and returns it as base64-encoded WAV, ready to
/// hand straight to the orb for playback.
async fn synthesize_speech(client: &reqwest::Client, api_url: &str, user_token: &str, text: &str) -> Result<String, String> {
    let started_at = Instant::now();
    log("speak_request", format!("text_chars={} api={api_url}", text.len()));
    let response = client
        .post(format!("{api_url}/api/chat/speak"))
        .bearer_auth(user_token)
        .json(&serde_json::json!({ "text": text }))
        .timeout(TRANSCRIBE_TIMEOUT)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = response.status();
    let body = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        log("speak_response", format!("status={status} elapsed_ms={} body_chars={}", started_at.elapsed().as_millis(), body.len()));
        return Err(clean_http_error(status, &body));
    }

    let parsed: SpeakResponse = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    log("speak_response", format!("status={status} elapsed_ms={} body_chars={} audio_base64={}", started_at.elapsed().as_millis(), body.len(), parsed.audio_base64.len()));
    Ok(parsed.audio_base64)
}

struct ChatResult {
    text: String,
    /// True once Aloe has called the `end_voice_mode` tool — the desktop should stop
    /// auto-relistening after speaking this response and go back to wake-word-only mode.
    ended: bool,
}

/// Sends the transcribed question through the same `/api/chat` endpoint the regular chat UI
/// uses, so voice turns get full tool access, memory, and conversation history for free.
/// `conversationId` is a client-generated id reused across turns in this listening session.
///
/// Reads the SSE response incrementally (rather than buffering the whole body first) and
/// emits each tool-call status the instant it arrives, so the orb can show what Aloe is
/// actually doing — searching the web, checking memory, etc. — instead of sitting on a
/// generic "Thinking…" for the whole turn.
async fn send_chat_message(
    shared: &ListenerShared,
    client: &reqwest::Client,
    api_url: &str,
    user_token: &str,
    wav_bytes: &[u8],
) -> Result<ChatResult, String> {
    let started_at = Instant::now();
    log("chat_request", format!("audio_bytes={} api={api_url}", wav_bytes.len()));
    let response = client
        .post(format!("{api_url}/api/chat"))
        .bearer_auth(user_token)
        .json(&serde_json::json!({
            "message": "[Voice message]",
            "conversationId": &shared.conversation_id,
            "voiceMode": true,
            "audioBase64": BASE64.encode(wav_bytes),
            "audioMimeType": "audio/wav",
        }))
        .timeout(CHAT_TIMEOUT)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.map_err(|e| e.to_string())?;
        log("chat_response", format!("status={status} elapsed_ms={} body_chars={}", started_at.elapsed().as_millis(), body.len()));
        return Err(clean_http_error(status, &body));
    }

    let mut assistant_text = String::new();
    let mut ended = false;
    let mut buffer = String::new();
    let mut byte_stream = response.bytes_stream();
    let mut text_chunks = 0usize;
    let mut status_events = 0usize;

    while let Some(chunk) = byte_stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        log("chat_stream_chunk", format!("bytes={}", chunk.len()));
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(event_end) = buffer.find("\n\n") {
            let event_block: String = buffer.drain(..event_end + 2).collect();
            for line in event_block.lines() {
                let Some(json_str) = line.strip_prefix("data: ") else { continue };
                let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) else { continue };
                match value.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(content) = value.get("content").and_then(|c| c.as_str()) {
                            text_chunks += 1;
                            assistant_text.push_str(content);
                            log("chat_text", format!("chunk_chars={} total_chars={}", content.len(), assistant_text.len()));
                        }
                    }
                    Some("status") => {
                        if let Some(content) = value.get("content").and_then(|c| c.as_str()) {
                            status_events += 1;
                            log("status", content);
                            let _ = shared.app.emit(VOICE_STATUS_EVENT, content);
                        }
                    }
                    Some("voice_mode_ended") => {
                        ended = true;
                        log("voice_mode_ended", format!("assistant_chars={} elapsed_ms={}", assistant_text.len(), started_at.elapsed().as_millis()));
                    }
                    _ => {}
                }
            }
        }
    }
    log("chat_response", format!("status={status} elapsed_ms={} assistant_chars={} text_chunks={} status_events={} ended={ended}", started_at.elapsed().as_millis(), assistant_text.len(), text_chunks, status_events));
    Ok(ChatResult { text: assistant_text, ended })
}
