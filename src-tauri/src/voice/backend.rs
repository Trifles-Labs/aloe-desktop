//! The Aloe backend calls push-to-talk makes, as the signed-in user from this device.
//!
//! The OpenAI key lives on the backend. All this side ever holds is a one-minute ephemeral key for
//! opening the transcription socket (`live_session`); speech is fetched through the backend too.

use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde_json::{json, Value};

use crate::models::AgentConfig;

#[derive(Clone)]
pub struct Backend {
    client: Client,
    api_url: String,
    user_token: String,
    /// The device credential. Sent as `X-Aloe-Agent` on chat turns, exactly as the main window's
    /// requests do (platform.tsx), so a voice turn can use this computer's local tools.
    credential: String,
}

impl Backend {
    /// `None` when the app isn't signed in and paired.
    pub fn from_config(client: Client, config: &AgentConfig) -> Option<Backend> {
        Some(Backend {
            client,
            api_url: config.api_url.trim_end_matches('/').to_string(),
            user_token: config.user_token.clone().filter(|t| !t.is_empty())?,
            credential: config.credential.clone().filter(|c| !c.is_empty())?,
        })
    }

    fn post(&self, path: &str) -> RequestBuilder {
        self.client.post(format!("{}{}", self.api_url, path)).bearer_auth(&self.user_token)
    }

    /// An ephemeral OpenAI key for one transcription socket.
    pub async fn live_session(&self) -> Result<String, String> {
        let response = self.post("/api/voice/live-session").send().await.map_err(network)?;
        let body: Value = ok_json(response, "Couldn't start listening").await?;
        body.get("key").and_then(Value::as_str).map(str::to_string).ok_or_else(|| "Couldn't start listening: no key in the reply.".to_string())
    }

    /// One sentence of Aloe's reply as 24 kHz mono PCM16, streamed. `pace` is "slow", "normal" or
    /// "fast"; a backend that doesn't know it yet ignores it.
    pub async fn speak(&self, text: &str, pace: &str) -> Result<Response, String> {
        let response = self.post("/api/voice/speak").json(&json!({ "text": text, "pace": pace })).send().await.map_err(network)?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(error_of(response, "Couldn't speak the reply").await)
        }
    }

    /// Uploads one user attachment, returning the id to send with the next chat turn.
    pub async fn upload_attachment(&self, bytes: Vec<u8>, filename: &str, mime_type: &str) -> Result<String, String> {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename.to_string())
            .mime_str(mime_type)
            .map_err(|e| format!("Couldn't prepare the attachment: {e}"))?;
        let form = reqwest::multipart::Form::new().part("file", part);
        let response = self.post("/api/chat/attachments").multipart(form).send().await.map_err(network)?;
        let body: Value = ok_json(response, "Couldn't attach the screenshot").await?;
        body.get("id").and_then(Value::as_str).map(str::to_string).ok_or_else(|| "Couldn't attach the screenshot: no id in the reply.".to_string())
    }

    /// Sends a chat turn. The caller reads the event stream off the response.
    pub async fn chat(&self, conversation_id: &str, message: &str, attachment_ids: &[String]) -> Result<Response, String> {
        let mut body = json!({ "message": message, "conversationId": conversation_id, "origin": "desktop_voice" });
        if !attachment_ids.is_empty() {
            body["attachmentIds"] = json!(attachment_ids);
        }
        let response = self
            .post("/api/chat")
            .header("X-Aloe-Agent", &self.credential)
            .json(&body)
            .send()
            .await
            .map_err(network)?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(error_of(response, "Couldn't send that to Aloe").await)
        }
    }

    /// Answers an `ask_question` card the turn is waiting on.
    pub async fn answer_question(&self, id: &str, answer: &str) -> Result<(), String> {
        let response = self.post(&format!("/api/questions/{id}/answer")).json(&json!({ "answer": answer })).send().await.map_err(network)?;
        ok_json(response, "Couldn't send your answer").await.map(drop)
    }

    /// Declines an `ask_question` card, which the model reads as "the user cancelled".
    pub async fn cancel_question(&self, id: &str) -> Result<(), String> {
        let response = self.post(&format!("/api/questions/{id}/cancel")).send().await.map_err(network)?;
        ok_json(response, "Couldn't cancel the question").await.map(drop)
    }

    /// Approves or rejects a proposed email, calendar or GitHub action.
    pub async fn decide_approval(&self, id: &str, approve: bool) -> Result<(), String> {
        let decision = if approve { "approve" } else { "reject" };
        let response = self.post(&format!("/api/approvals/{id}/{decision}")).send().await.map_err(network)?;
        ok_json(response, "Couldn't record your decision").await.map(drop)
    }

    /// Stops a turn still running server-side, so the next voice turn isn't refused as busy. Turns
    /// are tracked by the conversation's database id, which the client id is resolved to first.
    pub async fn stop_turn(&self, conversation_id: &str) {
        let Ok(response) = self.client.get(format!("{}/api/conversations/{}", self.api_url, conversation_id)).bearer_auth(&self.user_token).send().await else {
            return;
        };
        let Ok(body) = response.json::<Value>().await else { return };
        let Some(id) = body.get("id").and_then(Value::as_str) else { return };
        let _ = self.post(&format!("/api/chat/{id}/stop")).send().await;
    }
}

fn network(error: reqwest::Error) -> String {
    format!("Couldn't reach Aloe: {error}")
}

async fn ok_json(response: Response, failure: &str) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(error_of(response, failure).await);
    }
    response.json().await.map_err(|e| format!("{failure}: {e}"))
}

/// The backend's own message where it gave one; these are written for the user.
async fn error_of(response: Response, failure: &str) -> String {
    let status = response.status();
    let message = response.json::<Value>().await.ok().and_then(|body| body.get("error").and_then(Value::as_str).map(str::to_string));
    match (status, message) {
        (StatusCode::UNAUTHORIZED, _) => "Your Aloe session was not accepted. Sign out and sign in again.".to_string(),
        (StatusCode::NOT_FOUND, _) => "This Aloe server doesn't support voice yet.".to_string(),
        (_, Some(message)) => message,
        (status, None) => format!("{failure} ({status})."),
    }
}
