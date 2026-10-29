//! Live speech-to-text over OpenAI's Realtime transcription socket (`gpt-live-transcribe`).
//!
//! The model streams words back while audio is still going in, which is what lets the overlay
//! show the transcript as the user talks. It has no voice-activity detection of its own: the end
//! of the hold is the end of the turn, sent as one `input_audio_buffer.commit`.
//!
//! The microphone starts before the socket is open (minting the key and connecting takes a few
//! hundred milliseconds), so the first words wait in the audio channel and go out as soon as it is.

use std::{collections::HashMap, time::Duration};

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::{sync::mpsc::UnboundedReceiver, time::Instant};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};

use super::{audio::VOICE_RATE, backend::Backend};
use crate::config::{debug_log, VOICE_FINAL_TIMEOUT_MS};

const REALTIME_URL: &str = "wss://api.openai.com/v1/realtime?intent=transcription";
/// Audio is sent in ~100 ms appends rather than one per device callback.
const APPEND_SAMPLES: usize = VOICE_RATE as usize / 10;
/// The API refuses to commit less than 100 ms; anything that short wasn't speech anyway.
const MIN_SAMPLES: usize = VOICE_RATE as usize / 10;

/// Transcribes until `audio` closes (the capture stopped), calling `on_text` with the whole
/// transcript so far each time it grows. Returns the final text, empty when nothing was said.
pub async fn transcribe(backend: &Backend, mut audio: UnboundedReceiver<Vec<i16>>, mut on_text: impl FnMut(&str)) -> Result<String, String> {
    let key = backend.live_session().await?;
    let mut request = REALTIME_URL.into_client_request().map_err(|e| e.to_string())?;
    let auth = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|e| e.to_string())?;
    request.headers_mut().insert("Authorization", auth);
    let (socket, _) = connect_async(request).await.map_err(|e| format!("Couldn't reach the transcriber: {e}"))?;
    let (mut sink, mut stream) = socket.split();

    // The session was configured when the key was minted; this repeats it so the socket's settings
    // never depend on how the backend minted it.
    let session = json!({
        "type": "session.update",
        "session": {
            "type": "transcription",
            "audio": { "input": {
                "format": { "type": "audio/pcm", "rate": VOICE_RATE },
                "noise_reduction": { "type": "near_field" },
                "transcription": {
                    "model": "gpt-live-transcribe",
                    // English only: left to guess, a mumble came back as Chinese and Vietnamese.
                    "language": "en",
                    "prompt": "Someone talking to Aloe, their AI assistant on their computer, asking it a question or to do something.",
                },
                "turn_detection": null,
            } },
        },
    });
    send(&mut sink, session).await?;

    let mut transcript = Transcript::default();
    let mut pending: Vec<i16> = Vec::with_capacity(APPEND_SAMPLES * 2);
    let mut total_samples = 0usize;
    let mut sent: Vec<i16> = Vec::new();
    let mut audio_open = true;
    let mut committed: Option<String> = None;
    let mut deadline: Option<Instant> = None;

    loop {
        tokio::select! {
            // Audio first, so everything captured before the release is sent before the commit.
            biased;
            chunk = audio.recv(), if audio_open => match chunk {
                Some(chunk) => {
                    total_samples += chunk.len();
                    if cfg!(debug_assertions) {
                        sent.extend_from_slice(&chunk);
                    }
                    pending.extend(chunk);
                    if pending.len() >= APPEND_SAMPLES {
                        append(&mut sink, &mut pending).await?;
                    }
                }
                None => {
                    audio_open = false;
                    if total_samples < MIN_SAMPLES {
                        return Ok(String::new());
                    }
                    if !pending.is_empty() {
                        append(&mut sink, &mut pending).await?;
                    }
                    send(&mut sink, json!({ "type": "input_audio_buffer.commit" })).await?;
                    deadline = Some(Instant::now() + Duration::from_millis(VOICE_FINAL_TIMEOUT_MS));
                }
            },
            message = stream.next() => {
                let text = match message {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => continue,
                    Some(Err(error)) => return Err(format!("The transcriber disconnected: {error}")),
                };
                let Ok(event) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                let item = event.get("item_id").and_then(Value::as_str).unwrap_or_default().to_string();
                match event.get("type").and_then(Value::as_str).unwrap_or_default() {
                    "conversation.item.input_audio_transcription.delta" => {
                        if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                            transcript.delta(&item, delta);
                            on_text(&transcript.text());
                        }
                    }
                    "conversation.item.input_audio_transcription.completed" => {
                        transcript.complete(&item, event.get("transcript").and_then(Value::as_str).unwrap_or_default());
                        on_text(&transcript.text());
                        if committed.as_deref() == Some(item.as_str()) || (!audio_open && transcript.all_complete()) {
                            break;
                        }
                    }
                    "input_audio_buffer.committed" => committed = Some(item),
                    "error" => {
                        let message = event.pointer("/error/message").and_then(Value::as_str).unwrap_or("unknown error");
                        debug_log("voice", "transcriber_error", message);
                        return Err(format!("The transcriber failed: {message}"));
                    }
                    _ => {}
                }
            },
            _ = sleep_until(deadline) => {
                debug_log("voice", "transcript_timeout", "no completed event before the deadline; using the live text");
                break;
            }
        }
    }

    let _ = sink.send(Message::Close(None)).await;
    let text = transcript.text();
    debug_log("voice", "transcript", format!("final={} text={text:?}", if committed.is_some() { "committed" } else { "no_commit_ack" }));
    if cfg!(debug_assertions) {
        save_debug_wav(&sent);
    }
    Ok(text)
}

/// Dev builds keep the last recording exactly as it was sent, to listen to when a transcript is off.
fn save_debug_wav(samples: &[i16]) {
    let path = std::env::temp_dir().join("aloe-voice-last.wav");
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&VOICE_RATE.to_le_bytes());
    wav.extend_from_slice(&(VOICE_RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    match std::fs::write(&path, wav) {
        Ok(()) => debug_log("voice", "debug_wav", path.display().to_string()),
        Err(error) => debug_log("voice", "debug_wav_error", error.to_string()),
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

async fn send<S>(sink: &mut S, event: Value) -> Result<(), String>
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    sink.send(Message::Text(event.to_string().into())).await.map_err(|e| format!("The transcriber disconnected: {e}"))
}

async fn append<S>(sink: &mut S, pending: &mut Vec<i16>) -> Result<(), String>
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let bytes: Vec<u8> = pending.drain(..).flat_map(i16::to_le_bytes).collect();
    let audio = base64::engine::general_purpose::STANDARD.encode(bytes);
    send(sink, json!({ "type": "input_audio_buffer.append", "audio": audio })).await
}

/// Text per item, in the order items first appeared. A completed transcript replaces its deltas.
#[derive(Default)]
struct Transcript {
    order: Vec<String>,
    items: HashMap<String, (String, bool)>,
}

impl Transcript {
    fn entry(&mut self, item: &str) -> &mut (String, bool) {
        if !self.items.contains_key(item) {
            self.order.push(item.to_string());
        }
        self.items.entry(item.to_string()).or_default()
    }

    fn delta(&mut self, item: &str, delta: &str) {
        let entry = self.entry(item);
        if !entry.1 {
            entry.0.push_str(delta);
        }
    }

    fn complete(&mut self, item: &str, text: &str) {
        *self.entry(item) = (text.to_string(), true);
    }

    fn all_complete(&self) -> bool {
        !self.items.is_empty() && self.items.values().all(|(_, done)| *done)
    }

    fn text(&self) -> String {
        self.order.iter().filter_map(|id| self.items.get(id)).map(|(text, _)| text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::Transcript;

    #[test]
    fn completed_text_replaces_deltas_in_order() {
        let mut t = Transcript::default();
        t.delta("a", "hello wor");
        t.delta("b", "second");
        t.delta("a", "ld");
        assert_eq!(t.text(), "hello world second");
        assert!(!t.all_complete());
        t.complete("a", "Hello, world.");
        t.delta("a", " ignored");
        t.complete("b", "Second.");
        assert_eq!(t.text(), "Hello, world. Second.");
        assert!(t.all_complete());
    }
}
