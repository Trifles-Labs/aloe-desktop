use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tauri::{AppHandle, Emitter, Manager};
use vosk::{DecodingState, Model, Recognizer};

use super::wakeword;
use crate::config::AppState;

pub const WAKE_WORD_EVENT: &str = "wake-word-detected";
pub const VOICE_THINKING_EVENT: &str = "voice-thinking";
pub const VOICE_RESPONSE_EVENT: &str = "voice-response";
pub const VOICE_ERROR_EVENT: &str = "voice-error";
const ORB_WINDOW_LABEL: &str = "orb";
/// Safety net only: normally a natural pause in speech finalizes the question via Vosk's
/// own endpointer well before this.
const MAX_CAPTURE_DURATION: Duration = Duration::from_secs(15);
/// Defensive auto-hide in case the frontend never gets a chance to hide the orb itself
/// (e.g. a crashed renderer or an unhandled error before a response/error event fires).
const ORB_FALLBACK_AUTO_HIDE: Duration = Duration::from_secs(20);

fn log(event: &str, detail: impl AsRef<str>) {
    println!("[voice] {event} {}", detail.as_ref());
}

/// Owns the background thread that keeps the microphone stream and Vosk recognizers alive.
/// The thread is kept alive by simply not letting the `cpal::Stream` (which isn't safe to
/// hand around between threads) drop until `stop` is observed.
pub struct VoiceEngineHandle {
    stop: Arc<AtomicBool>,
    muted: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl VoiceEngineHandle {
    pub fn set_muted(&self, muted: bool) {
        log("muted", muted.to_string());
        self.muted.store(muted, Ordering::SeqCst);
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Either listening for the wake word, or actively capturing the question that follows it.
/// While capturing, raw audio is buffered alongside Vosk's recognizer: Vosk's small model is
/// only used to detect *when* the user stops talking (its endpointer is reliable), while the
/// buffered audio itself gets transcribed by Gemini, which is far more accurate.
enum ListenerMode {
    WakeWord(Recognizer),
    Capturing { recognizer: Recognizer, buffer: Vec<i16>, started_at: Instant },
}

/// Below this much audio there's not enough to bother transcribing.
const MIN_CAPTURE_DURATION_SECS: f32 = 0.3;

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
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

    log("spawn_listener", format!("conversation_id={conversation_id}"));

    let thread = std::thread::Builder::new()
        .name("aloe-voice-listener".into())
        .spawn(move || run_listener(app, model_path, conversation_id, stop_for_thread, muted_for_thread, ready_tx))
        .map_err(|e| e.to_string())?;

    ready_rx
        .recv()
        .map_err(|_| "Voice listener thread exited before starting.".to_string())??;

    Ok(VoiceEngineHandle { stop, muted, thread: Some(thread) })
}

fn run_listener(
    app: AppHandle,
    model_path: PathBuf,
    conversation_id: String,
    stop: Arc<AtomicBool>,
    muted: Arc<AtomicBool>,
    ready_tx: mpsc::Sender<Result<(), String>>,
) {
    let setup = (|| -> Result<(cpal::Stream, Arc<Model>), String> {
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
        let mode = Arc::new(Mutex::new(ListenerMode::WakeWord(recognizer)));
        let stream_config: cpal::StreamConfig = config.clone().into();
        let err_fn = |err| eprintln!("[voice] microphone stream error: {err}");

        let stream = match config.sample_format() {
            cpal::SampleFormat::I16 => {
                let mode = mode.clone();
                let muted = muted.clone();
                let model = model.clone();
                let app = app.clone();
                let conversation_id = conversation_id.clone();
                device.build_input_stream(
                    stream_config.clone(),
                    move |data: &[i16], _| {
                        process_samples(&mode, &muted, &model, sample_rate, &conversation_id, data, channels, &app)
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I32 => {
                let mode = mode.clone();
                let muted = muted.clone();
                let model = model.clone();
                let app = app.clone();
                let conversation_id = conversation_id.clone();
                device.build_input_stream(
                    stream_config.clone(),
                    move |data: &[i32], _| {
                        let converted: Vec<i16> = data.iter().map(|&s| (s >> 16) as i16).collect();
                        process_samples(&mode, &muted, &model, sample_rate, &conversation_id, &converted, channels, &app);
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::F32 => {
                let mode = mode.clone();
                let muted = muted.clone();
                let model = model.clone();
                let app = app.clone();
                let conversation_id = conversation_id.clone();
                device.build_input_stream(
                    stream_config.clone(),
                    move |data: &[f32], _| {
                        let converted: Vec<i16> = data
                            .iter()
                            .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                            .collect();
                        process_samples(&mode, &muted, &model, sample_rate, &conversation_id, &converted, channels, &app);
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
        Ok((stream, model))
    })();

    let (stream, _model) = match setup {
        Ok(pair) => {
            let _ = ready_tx.send(Ok(()));
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

#[allow(clippy::too_many_arguments)]
fn process_samples(
    mode: &Arc<Mutex<ListenerMode>>,
    muted: &Arc<AtomicBool>,
    model: &Arc<Model>,
    sample_rate: f32,
    conversation_id: &str,
    data: &[i16],
    channels: cpal::ChannelCount,
    app: &AppHandle,
) {
    if muted.load(Ordering::Relaxed) {
        return;
    }

    let mono = downmix_to_mono(data, channels);
    let mut guard = match mode.lock() {
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

            log("wake_word", "detected, now capturing the question");
            match wakeword::new_open_recognizer(model, sample_rate) {
                Ok(capture_recognizer) => {
                    *guard = ListenerMode::Capturing { recognizer: capture_recognizer, buffer: Vec::new(), started_at: Instant::now() };
                }
                Err(error) => log("capture_recognizer_failed", &error),
            }
            drop(guard);
            on_wake_word(app);
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
            log("captured", format!("{} samples (~{:.1}s)", captured.len(), captured.len() as f32 / sample_rate));

            match wakeword::new_recognizer(model, sample_rate) {
                Ok(wake_recognizer) => *guard = ListenerMode::WakeWord(wake_recognizer),
                Err(error) => log("wake_recognizer_failed", &error),
            }
            drop(guard);

            if (captured.len() as f32) < sample_rate * MIN_CAPTURE_DURATION_SECS {
                let _ = app.emit(VOICE_ERROR_EVENT, "I didn't catch a question.");
                schedule_orb_hide(app, Duration::from_secs(2));
            } else {
                match encode_wav(&captured, sample_rate as u32) {
                    Ok(wav_bytes) => {
                        let app = app.clone();
                        let conversation_id = conversation_id.to_string();
                        tauri::async_runtime::spawn(async move {
                            dispatch_audio(app, conversation_id, wav_bytes).await;
                        });
                    }
                    Err(error) => {
                        log("encode_wav_failed", &error);
                        let _ = app.emit(VOICE_ERROR_EVENT, "Something went wrong capturing that.");
                        schedule_orb_hide(app, Duration::from_secs(2));
                    }
                }
            }
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

fn on_wake_word(app: &AppHandle) {
    let _ = app.emit(WAKE_WORD_EVENT, ());

    let Some(orb) = app.get_webview_window(ORB_WINDOW_LABEL) else { return };
    let _ = orb.show();
    let _ = orb.set_focus();
    schedule_orb_hide(app, ORB_FALLBACK_AUTO_HIDE);
}

fn schedule_orb_hide(app: &AppHandle, after: Duration) {
    let Some(orb) = app.get_webview_window(ORB_WINDOW_LABEL) else { return };
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(after).await;
        let _ = orb.hide();
    });
}

async fn dispatch_audio(app: AppHandle, conversation_id: String, wav_bytes: Vec<u8>) {
    log("dispatch", format!("audio={} bytes", wav_bytes.len()));
    let _ = app.emit(VOICE_THINKING_EVENT, ());

    let (client, api_url, user_token) = {
        let state = app.state::<AppState>();
        let config = state.config.lock().expect("config mutex");
        (state.client.clone(), config.api_url.clone(), config.user_token.clone())
    };

    let Some(user_token) = user_token else {
        log("dispatch_failed", "not signed in");
        let _ = app.emit(VOICE_ERROR_EVENT, "Aloe Desktop isn't signed in.");
        return;
    };

    let question = match transcribe_audio(&client, &api_url, &user_token, &wav_bytes).await {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => {
            log("transcribe", "empty transcript");
            let _ = app.emit(VOICE_ERROR_EVENT, "I didn't catch a question.");
            return;
        }
        Err(error) => {
            log("transcribe_failed", &error);
            let _ = app.emit(VOICE_ERROR_EVENT, format!("Transcription failed: {error}"));
            return;
        }
    };
    log("transcribed", format!("{question:?}"));

    match send_chat_message(&client, &api_url, &user_token, &conversation_id, &question).await {
        Ok(answer) if !answer.trim().is_empty() => {
            log("response", format!("{} chars", answer.len()));
            let _ = app.emit(VOICE_RESPONSE_EVENT, answer);
        }
        Ok(_) => {
            log("response", "empty");
            let _ = app.emit(VOICE_ERROR_EVENT, "Aloe didn't return a response.");
        }
        Err(error) => {
            log("dispatch_failed", &error);
            let _ = app.emit(VOICE_ERROR_EVENT, error);
        }
    }
}

#[derive(serde::Deserialize)]
struct TranscribeResponse {
    text: String,
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

/// Sends the captured audio to Aloe's backend for Gemini-based transcription. Far more
/// accurate than transcribing locally with the small on-device Vosk model, which is tuned
/// for spotting the wake word, not general dictation.
async fn transcribe_audio(client: &reqwest::Client, api_url: &str, user_token: &str, wav_bytes: &[u8]) -> Result<String, String> {
    let response = client
        .post(format!("{api_url}/api/chat/transcribe"))
        .bearer_auth(user_token)
        .json(&serde_json::json!({ "audioBase64": BASE64.encode(wav_bytes), "mimeType": "audio/wav" }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = response.status();
    let body = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(clean_http_error(status, &body));
    }

    let parsed: TranscribeResponse = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    Ok(parsed.text)
}

/// Sends the transcribed question through the same `/api/chat` endpoint the regular chat UI
/// uses, so voice turns get full tool access, memory, and conversation history for free.
/// `conversationId` is a client-generated id reused across turns in this listening session.
async fn send_chat_message(
    client: &reqwest::Client,
    api_url: &str,
    user_token: &str,
    conversation_id: &str,
    message: &str,
) -> Result<String, String> {
    let response = client
        .post(format!("{api_url}/api/chat"))
        .bearer_auth(user_token)
        .json(&serde_json::json!({ "message": message, "conversationId": conversation_id }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = response.status();
    let body = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(clean_http_error(status, &body));
    }

    let mut assistant_text = String::new();
    for line in body.lines() {
        let Some(json_str) = line.strip_prefix("data: ") else { continue };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) else { continue };
        if value.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(content) = value.get("content").and_then(|c| c.as_str()) {
                assistant_text.push_str(content);
            }
        }
    }
    Ok(assistant_text)
}
