//! Live speech-to-text through the Aloe backend's relay, used when the backend runs voice on
//! Sarvam. Sarvam has no short-lived keys, so the backend holds the key and passes the audio on.
//!
//! The protocol is the backend's own (lib/voice/live_relay.ts), not Sarvam's: raw s16le PCM at
//! `VOICE_RATE` goes up as binary frames and `{"type":"commit"}` ends the hold; back come
//! `{"type":"transcript","text"}` with the whole text so far, then `done` or `error`.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::{sync::mpsc::UnboundedReceiver, time::Instant};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};

use super::realtime::{save_debug_wav, send, sleep_until, APPEND_SAMPLES, MIN_SAMPLES};
use crate::config::{debug_log, VOICE_FINAL_TIMEOUT_MS};

/// Same contract as `realtime::transcribe`: `on_text` gets the whole transcript each time it
/// grows, and the result is the final text, empty when nothing was said.
pub async fn transcribe(url: &str, ticket: &str, mut audio: UnboundedReceiver<Vec<i16>>, mut on_text: impl FnMut(&str)) -> Result<String, String> {
    let mut request = url.into_client_request().map_err(|e| e.to_string())?;
    let auth = HeaderValue::from_str(&format!("Bearer {ticket}")).map_err(|e| e.to_string())?;
    request.headers_mut().insert("Authorization", auth);
    let (socket, _) = connect_async(request).await.map_err(|e| format!("Couldn't reach the transcriber: {e}"))?;
    let (mut sink, mut stream) = socket.split();

    let mut text = String::new();
    let mut pending: Vec<i16> = Vec::with_capacity(APPEND_SAMPLES * 2);
    let mut total_samples = 0usize;
    let mut sent: Vec<i16> = Vec::new();
    let mut audio_open = true;
    let mut done = false;
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
                        let _ = sink.send(Message::Close(None)).await;
                        return Ok(String::new());
                    }
                    if !pending.is_empty() {
                        append(&mut sink, &mut pending).await?;
                    }
                    send(&mut sink, json!({ "type": "commit" })).await?;
                    deadline = Some(Instant::now() + Duration::from_millis(VOICE_FINAL_TIMEOUT_MS));
                }
            },
            message = stream.next() => {
                let raw = match message {
                    Some(Ok(Message::Text(raw))) => raw,
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => continue,
                    Some(Err(error)) => return Err(format!("The transcriber disconnected: {error}")),
                };
                let Ok(event) = serde_json::from_str::<Value>(raw.as_str()) else { continue };
                let event_text = event.get("text").and_then(Value::as_str);
                match event.get("type").and_then(Value::as_str).unwrap_or_default() {
                    "transcript" => {
                        if let Some(latest) = event_text {
                            text = latest.to_string();
                            on_text(&text);
                        }
                    }
                    "done" => {
                        if let Some(latest) = event_text {
                            text = latest.to_string();
                            on_text(&text);
                        }
                        done = true;
                        break;
                    }
                    "error" => {
                        let message = event.get("message").and_then(Value::as_str).unwrap_or("The transcriber failed.");
                        debug_log("voice", "relay_error", message);
                        return Err(message.to_string());
                    }
                    _ => {}
                }
            },
            _ = sleep_until(deadline) => {
                debug_log("voice", "transcript_timeout", "no done event before the deadline; using the live text");
                break;
            }
        }
    }

    let _ = sink.send(Message::Close(None)).await;
    debug_log("voice", "transcript", format!("via=relay final={} text={text:?}", if done { "done" } else { "no_done" }));
    if cfg!(debug_assertions) {
        save_debug_wav(&sent);
    }
    Ok(text)
}

async fn append<S>(sink: &mut S, pending: &mut Vec<i16>) -> Result<(), String>
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let bytes: Vec<u8> = pending.drain(..).flat_map(i16::to_le_bytes).collect();
    sink.send(Message::Binary(bytes.into())).await.map_err(|e| format!("The transcriber disconnected: {e}"))
}
