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
/// The user's Liked Songs.
const SPOTIFY_LIKED_SONGS: &str = "spotify:collection:tracks";

/// A `spotify:` link this job built itself: either a checked `spotify:<kind>:<id>` URI, Liked Songs,
/// or a search for the query. Nothing else reaches the OS opener or the AppleScript below.
pub(crate) fn spotify_link(input: &Value) -> Result<String, String> {
    if let Some(uri) = input.get("uri").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        let uri = uri
            .strip_prefix("https://open.spotify.com/")
            .map(|rest| format!("spotify:{}", rest.split('?').next().unwrap_or_default().replace('/', ":")))
            .unwrap_or_else(|| uri.to_string());
        let parts: Vec<&str> = uri.split(':').collect();
        let valid = uri == SPOTIFY_LIKED_SONGS
            || matches!(parts.as_slice(), ["spotify", kind, id]
                if SPOTIFY_URI_KINDS.contains(kind) && (16..=32).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric()));
        return if valid {
            Ok(uri)
        } else {
            Err("uri must look like spotify:track:<id> (track, album, playlist, artist, episode or show) or be spotify:collection:tracks.".to_string())
        };
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

/// Hands a link built by `spotify_link` to the registered `spotify:` handler.
async fn open_link(link: &str) -> Result<(), String> {
    let status = if cfg!(target_os = "windows") {
        let mut cmd = Command::new("rundll32");
        cmd.arg("url.dll,FileProtocolHandler").arg(link).stdout(Stdio::null()).stderr(Stdio::null());
        hide_command_window(&mut cmd);
        cmd.status().await
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(link).status().await
    } else {
        Command::new("xdg-open").arg(link).status().await
    }
    .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Couldn't open Spotify. It may not be installed; open.spotify.com works in the browser.".to_string());
    }
    Ok(())
}

/// What Spotify says it is playing.
#[derive(Clone, Debug, PartialEq)]
struct Playback {
    title: String,
    artist: String,
    playing: bool,
}

impl Playback {
    fn label(&self) -> String {
        if self.artist.is_empty() { self.title.clone() } else { format!("{} — {}", self.title, self.artist) }
    }

    fn differs_from(&self, before: Option<&Playback>) -> bool {
        before.map_or(true, |b| b.title != self.title || b.artist != self.artist)
    }
}

/// Spotify's entry in Windows' media controls (the same one the volume flyout shows).
#[cfg(target_os = "windows")]
mod smtc {
    use super::Playback;
    use windows::{
        Media::Control::{
            GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
            GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
        },
        Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED},
    };

    fn spotify_session() -> Option<Session> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let manager = Manager::RequestAsync().ok()?.get().ok()?;
        manager
            .GetSessions()
            .ok()?
            .into_iter()
            .find(|s| s.SourceAppUserModelId().map(|id| id.to_string().to_lowercase().contains("spotify")).unwrap_or(false))
    }

    pub fn read() -> Option<Playback> {
        let session = spotify_session()?;
        let props = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
        let playing = session.GetPlaybackInfo().ok()?.PlaybackStatus().ok()? == Status::Playing;
        let title = props.Title().ok()?.to_string();
        let artist = props.Artist().map(|a| a.to_string()).unwrap_or_default();
        (!title.is_empty()).then_some(Playback { title, artist, playing })
    }

    pub fn play() -> bool {
        spotify_session().and_then(|s| s.TryPlayAsync().ok()?.get().ok()).unwrap_or(false)
    }
}

#[cfg(target_os = "windows")]
async fn now_playing() -> Option<Playback> {
    tokio::task::spawn_blocking(smtc::read).await.ok().flatten()
}

/// Opens the link, then watches Spotify until the new item is playing. Spotify plays some links and
/// only shows the page for others, and a cold start drops the link's play, so: once Spotify first
/// shows up the link is opened again, and a new item that loaded paused gets a play. `playing` is
/// false when nothing new started, so the caller knows to press Play on screen.
#[cfg(target_os = "windows")]
async fn play_and_confirm(link: &str) -> Result<(Option<bool>, Option<Playback>), String> {
    let before = now_playing().await;
    open_link(link).await?;
    let mut reopened = before.is_some();
    let mut pressed = false;
    let rounds = if before.is_some() { 12 } else { 30 };
    for _ in 0..rounds {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let Some(now) = now_playing().await else { continue };
        if !reopened {
            reopened = true;
            tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
            open_link(link).await?;
            continue;
        }
        if now.differs_from(before.as_ref()) {
            if now.playing {
                return Ok((Some(true), Some(now)));
            }
            if !pressed {
                pressed = true;
                let _ = tokio::task::spawn_blocking(smtc::play).await;
            }
        }
    }
    let last = now_playing().await;
    let playing = last.as_ref().is_some_and(|now| now.playing && now.differs_from(before.as_ref()));
    Ok((Some(playing), last))
}

#[cfg(target_os = "macos")]
async fn applescript(script: &str) -> Option<String> {
    let out = Command::new("osascript").arg("-e").arg(script).stderr(Stdio::null()).output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Spotify's AppleScript plays a link outright, no page in between. Falls back to `open` when the
/// script is refused (the user declined the automation prompt) or Spotify isn't there.
#[cfg(target_os = "macos")]
async fn play_and_confirm(link: &str) -> Result<(Option<bool>, Option<Playback>), String> {
    // `link` is spotify:<kind>:<alphanumeric id> or Liked Songs, so it can't break out of the quotes.
    if applescript(&format!("tell application \"Spotify\" to play track \"{link}\"")).await.is_none() {
        open_link(link).await?;
        return Ok((None, None));
    }
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let state = applescript(
        "tell application \"Spotify\" to return (name of current track) & linefeed & (artist of current track) & linefeed & (player state as string)",
    )
    .await;
    let now = state.and_then(|text| {
        let mut lines = text.lines();
        let title = lines.next()?.to_string();
        let artist = lines.next().unwrap_or_default().to_string();
        let playing = lines.next() == Some("playing");
        Some(Playback { title, artist, playing })
    });
    Ok((now.as_ref().map(|n| n.playing), now))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
async fn play_and_confirm(link: &str) -> Result<(Option<bool>, Option<Playback>), String> {
    open_link(link).await?;
    Ok((None, None))
}

/// Plays a Spotify link in the Spotify app; a search link only opens the search. The link was built
/// by `spotify_link`, never passed through from the model. `playing` is true when Spotify reports a
/// new item playing, false when it doesn't, and null when this computer can't tell.
pub async fn open_spotify(input: &Value) -> Result<Value, String> {
    let link = spotify_link(input)?;
    #[cfg(target_os = "windows")]
    if !spotify_installed().await {
        return Err("The Spotify app isn't installed on this computer. Open open.spotify.com in the browser instead.".to_string());
    }
    if link.starts_with("spotify:search:") {
        open_link(&link).await?;
        return Ok(json!({ "link": link, "opened": true, "playing": false }));
    }
    let (playing, now) = play_and_confirm(&link).await?;
    Ok(json!({ "link": link, "opened": true, "playing": playing, "nowPlaying": now.map(|n| n.label()) }))
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
        assert_eq!(spotify_link(&json!({ "uri": "spotify:collection:tracks" })).unwrap(), "spotify:collection:tracks");
        for uri in [
            "spotify:track:abc/../x",
            "file:///C:/calc.exe",
            "spotify:user:x:playlist:y",
            "ms-msdt:/id x",
            "spotify:track:0VjIjW4GlUZAMYd2vXMi3b&x",
            "spotify:track:0VjIjW4GlUZAMYd2vXMi3b\" & do shell script \"x",
            "spotify:collection:tracks:x",
        ] {
            assert!(spotify_link(&json!({ "uri": uri })).is_err(), "should refuse {uri}");
        }
        assert!(spotify_link(&json!({})).is_err());
    }
}
