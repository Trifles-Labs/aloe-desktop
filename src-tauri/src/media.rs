//! Media keys and Spotify links: the fast path for "pause", "next song" and "play X on Spotify",
//! instead of driving the screen. Neither job can start an arbitrary program, so neither needs
//! command approval or the desktop-control switch.

use std::process::Stdio;

use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use serde_json::{json, Value};
use tokio::process::Command;

use crate::shell::hide_command_window;

fn media_key_for(action: &str) -> Option<Key> {
    Some(match action {
        "play_pause" => Key::MediaPlayPause,
        "next" => Key::MediaNextTrack,
        "previous" => Key::MediaPrevTrack,
        "volume_up" => Key::VolumeUp,
        "volume_down" => Key::VolumeDown,
        "mute" => Key::VolumeMute,
        _ => return None,
    })
}

/// Presses a system media key, which whatever is playing (Spotify, a browser tab, a desktop player)
/// picks up without being focused.
pub async fn press_media_key(input: &Value) -> Result<Value, String> {
    let action = input.get("action").and_then(Value::as_str).unwrap_or_default().to_string();
    let key = media_key_for(&action)
        .ok_or_else(|| "action must be play_pause, next, previous, volume_up, volume_down or mute.".to_string())?;
    let repeat = input.get("repeat").and_then(Value::as_u64).unwrap_or(1).clamp(1, 10);
    tokio::task::spawn_blocking(move || {
        let mut enigo = Enigo::new(&Settings::default()).map_err(|e| format!("Could not access keyboard input: {e}"))?;
        for _ in 0..repeat {
            enigo.key(key, Direction::Click).map_err(|e| format!("Could not press the media key: {e}"))?;
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(json!({ "action": action, "repeat": repeat, "pressed": true }))
}

const SPOTIFY_URI_KINDS: [&str; 6] = ["track", "album", "playlist", "artist", "episode", "show"];

/// A `spotify:` link this job built itself: either a checked `spotify:<kind>:<id>` URI, or a search
/// for the query. Nothing else reaches the OS opener.
pub(crate) fn spotify_link(input: &Value) -> Result<String, String> {
    if let Some(uri) = input.get("uri").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        let uri = uri
            .strip_prefix("https://open.spotify.com/")
            .map(|rest| format!("spotify:{}", rest.split('?').next().unwrap_or_default().replace('/', ":")))
            .unwrap_or_else(|| uri.to_string());
        let parts: Vec<&str> = uri.split(':').collect();
        let valid = matches!(parts.as_slice(), ["spotify", kind, id]
            if SPOTIFY_URI_KINDS.contains(kind) && (16..=32).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric()));
        return if valid { Ok(uri) } else { Err("uri must look like spotify:track:<id> (track, album, playlist, artist, episode or show).".to_string()) };
    }
    let query = input.get("query").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if query.is_empty() {
        return Err("Give a Spotify uri or a search query.".to_string());
    }
    let encoded: String = url::form_urlencoded::byte_serialize(query.chars().take(200).collect::<String>().as_bytes()).collect();
    Ok(format!("spotify:search:{}", encoded.replace('+', "%20")))
}

#[cfg(target_os = "windows")]
async fn spotify_installed() -> bool {
    let mut cmd = Command::new("reg");
    cmd.args(["query", r"HKCR\spotify", "/ve"]).stdout(Stdio::null()).stderr(Stdio::null());
    hide_command_window(&mut cmd);
    cmd.status().await.map(|s| s.success()).unwrap_or(false)
}

/// Opens a Spotify link in the Spotify app. `rundll32`/`open` hand it to the registered `spotify:`
/// handler; the link itself was built by `spotify_link`, never passed through from the model.
pub async fn open_spotify(input: &Value) -> Result<Value, String> {
    let link = spotify_link(input)?;
    let status = if cfg!(target_os = "windows") {
        #[cfg(target_os = "windows")]
        if !spotify_installed().await {
            return Err("The Spotify app isn't installed on this computer. Open open.spotify.com in the browser instead.".to_string());
        }
        let mut cmd = Command::new("rundll32");
        cmd.arg("url.dll,FileProtocolHandler").arg(&link).stdout(Stdio::null()).stderr(Stdio::null());
        hide_command_window(&mut cmd);
        cmd.status().await
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(&link).status().await
    } else {
        Command::new("xdg-open").arg(&link).status().await
    }
    .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Couldn't open Spotify. It may not be installed; open.spotify.com works in the browser.".to_string());
    }
    Ok(json!({ "link": link, "opened": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_actions_map_to_keys() {
        for action in ["play_pause", "next", "previous", "volume_up", "volume_down", "mute"] {
            assert!(media_key_for(action).is_some(), "{action}");
        }
        assert!(media_key_for("enter").is_none());
    }

    #[test]
    fn spotify_links_are_built_not_passed_through() {
        assert_eq!(spotify_link(&json!({ "uri": "spotify:track:0VjIjW4GlUZAMYd2vXMi3b" })).unwrap(), "spotify:track:0VjIjW4GlUZAMYd2vXMi3b");
        assert_eq!(
            spotify_link(&json!({ "uri": "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc" })).unwrap(),
            "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"
        );
        assert_eq!(spotify_link(&json!({ "query": "blinding lights & the weeknd" })).unwrap(), "spotify:search:blinding%20lights%20%26%20the%20weeknd");
        for uri in ["spotify:track:abc/../x", "file:///C:/calc.exe", "spotify:user:x:playlist:y", "ms-msdt:/id x", "spotify:track:0VjIjW4GlUZAMYd2vXMi3b&x"] {
            assert!(spotify_link(&json!({ "uri": uri })).is_err(), "should refuse {uri}");
        }
        assert!(spotify_link(&json!({})).is_err());
    }
}
