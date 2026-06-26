use futures_util::{SinkExt, StreamExt};
use tauri::{AppHandle, Emitter};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};

use crate::config::normalize_api_url;
use super::capture::{
    log, ListenerShared, VoicePayload,
    VOICE_RESPONSE_EVENT, VOICE_STATUS_EVENT, VOICE_THINKING_EVENT, VOICE_ERROR_EVENT,
    reset_to_wake_word_only,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READY_TIMEOUT: Duration = Duration::from_secs(10);

pub struct LiveSession {
    pub pcm_sender: tokio::sync::mpsc::Sender<Vec<i16>>,
    pub task: tauri::async_runtime::JoinHandle<()>,
}

fn live_ws_url(api_url: &str, conversation_id: &str) -> String {
    let base = normalize_api_url(api_url);
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base
    };
    format!("{ws_base}/api/voice/live?conversationId={conversation_id}")
}

pub async fn connect_live(
    app: AppHandle,
    shared: ListenerShared,
    conversation_id: &str,
    api_url: &str,
    user_token: &str,
) -> Result<LiveSession, String> {
    let url = live_ws_url(api_url, conversation_id);
    log("live_connect", &url);

    let mut request = url.into_client_request().map_err(|e| e.to_string())?;
    let auth_value = HeaderValue::from_str(&format!("Bearer {user_token}"))
        .map_err(|e| e.to_string())?;
    request.headers_mut().insert("Authorization", auth_value);

    let (ws_stream, response) = timeout(CONNECT_TIMEOUT, connect_async(request))
        .await
        .map_err(|_| "Timed out connecting to voice live endpoint.".to_string())?
        .map_err(|e| e.to_string())?;

    log("live_ws_connected", format!("status={}", response.status()));

    let (mut write, mut read) = ws_stream.split();

    // Wait for { "type": "ready" } before starting to send audio
    log("live_waiting_ready", "waiting for ready signal from backend");
    let ready = timeout(READY_TIMEOUT, async {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    log("live_pre_ready_rx", &text);
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                        if v.get("type").and_then(|t| t.as_str()) == Some("ready") {
                            return true;
                        }
                    }
                }
                Ok(other) => log("live_pre_ready_rx_other", format!("{other:?}")),
                Err(e) => {
                    log("live_pre_ready_rx_error", e.to_string());
                    return false;
                }
            }
        }
        false
    })
    .await;

    match ready {
        Ok(true) => log("live_ready", "received, starting audio stream"),
        Ok(false) => return Err("Live session closed before sending ready.".to_string()),
        Err(_) => return Err("Timed out waiting for ready from live endpoint.".to_string()),
    }

    let (pcm_tx, mut pcm_rx) = tokio::sync::mpsc::channel::<Vec<i16>>(256);

    let task = tauri::async_runtime::spawn(async move {
        let mut pcm_chunks_sent: u64 = 0;
        let mut pcm_bytes_sent: u64 = 0;

        loop {
            tokio::select! {
                biased;
                // Prefer server messages — ensures queued events (e.g. "audio" after
                // "voice_mode_ended") are processed before we notice the pcm channel closing.
                msg_opt = read.next() => {
                    match msg_opt {
                        Some(Ok(Message::Text(text))) => {
                            log("live_rx", &text);
                            handle_server_message(&app, &shared, &text);
                        }
                        Some(Ok(Message::Binary(b))) => {
                            log("live_rx_binary", format!("{} bytes", b.len()));
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            log("live_rx_error", e.to_string());
                            break;
                        }
                        None => {
                            log("live_rx", "stream closed");
                            break;
                        }
                    }
                }
                pcm_opt = pcm_rx.recv() => {
                    match pcm_opt {
                        Some(samples) => {
                            let bytes: Vec<u8> = samples.iter().flat_map(|&s| s.to_le_bytes()).collect();
                            pcm_chunks_sent += 1;
                            pcm_bytes_sent += bytes.len() as u64;
                            if pcm_chunks_sent % 50 == 1 {
                                log("live_tx_pcm", format!("chunk={pcm_chunks_sent} total_bytes={pcm_bytes_sent} chunk_bytes={}", bytes.len()));
                            }
                            if write.send(Message::Binary(bytes.into())).await.is_err() {
                                log("live_tx", "write error, closing");
                                break;
                            }
                        }
                        None => {
                            // PCM channel closed (mode switched away from Live) — drain any
                            // remaining server messages (e.g. the final "audio" frame) with
                            // a short deadline, then exit.
                            log("live_tx_pcm", format!("channel closed after {pcm_chunks_sent} chunks ({pcm_bytes_sent} bytes), draining server messages"));
                            let _ = timeout(Duration::from_secs(3), async {
                                while let Some(Ok(Message::Text(text))) = read.next().await {
                                    log("live_rx_drain", &text);
                                    handle_server_message(&app, &shared, &text);
                                }
                            })
                            .await;
                            break;
                        }
                    }
                }
            }
        }
        let _ = write
            .send(Message::Text("{\"type\":\"end\"}".to_string().into()))
            .await;
        log("live_session", format!("task ended — sent {pcm_chunks_sent} pcm chunks ({pcm_bytes_sent} bytes)"));
    });

    Ok(LiveSession { pcm_sender: pcm_tx, task })
}

fn handle_server_message(app: &AppHandle, shared: &ListenerShared, text: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else { return };
    match value.get("type").and_then(|t| t.as_str()) {
        Some("transcript") => {
            if let Some(t) = value.get("text").and_then(|v| v.as_str()) {
                let _ = app.emit("voice-live-transcript", t.to_string());
            }
            if value.get("isFinal").and_then(|v| v.as_bool()).unwrap_or(false) {
                let _ = app.emit(VOICE_THINKING_EVENT, ());
            }
        }
        Some("audio") => {
            let audio = value.get("data").and_then(|v| v.as_str()).map(String::from);
            let continue_listening = value
                .get("continueListening")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let text_str = value
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let _ = app.emit(VOICE_RESPONSE_EVENT, VoicePayload {
                text: text_str,
                continue_listening,
                audio_base64: audio,
            });
        }
        Some("tool_call") => {
            if let Some(status) = value.get("status").and_then(|v| v.as_str()) {
                let _ = app.emit(VOICE_STATUS_EVENT, status.to_string());
            }
        }
        Some("voice_mode_ended") => {
            reset_to_wake_word_only(shared);
        }
        Some("error") => {
            let msg = value
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("A voice error occurred.")
                .to_string();
            let _ = app.emit(VOICE_ERROR_EVENT, VoicePayload::text_only(msg, false));
        }
        _ => {}
    }
}

/// Linear-interpolation downsampler from arbitrary device rate to 16 kHz.
/// Runs in the cpal audio callback, so it must never block or allocate excessively.
pub fn resample_to_16k(samples: &[i16], from_rate: f32) -> Vec<i16> {
    if (from_rate - 16000.0).abs() < 1.0 {
        return samples.to_vec();
    }
    let ratio = from_rate / 16000.0;
    let out_len = (samples.len() as f32 / ratio).ceil() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f32 * ratio;
        let src_idx = src_pos as usize;
        let frac = src_pos - src_idx as f32;
        let a = samples.get(src_idx).copied().unwrap_or(0) as f32;
        let b = samples.get(src_idx + 1).copied().unwrap_or(0) as f32;
        out.push((a + (b - a) * frac) as i16);
    }
    out
}
