//! Aloe Code sessions kept on this computer.
//!
//! The server stores none of an Aloe Code chat: each session lives here, one JSON file per session
//! under the app's config directory (`code-chats/`), and the app sends its history with every turn
//! (aloe-ui's lib/local_code_chats.ts holds the shape). An `index.json` beside them keeps the
//! sidebar's listing cheap, since a session file carries every tool result and can be large; it is
//! rebuilt from the files whenever it is missing or unreadable.
//!
//! Nothing here is pruned: these files are the only copy of the user's sessions.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::config::config_path;
use crate::workbench::write_atomically;

/// A single session larger than this is refused rather than written: something has gone wrong.
const MAX_CHAT_BYTES: usize = 50 * 1024 * 1024;
const INDEX_FILE: &str = "index.json";
/// What the listing keeps of each session.
const SUMMARY_FIELDS: [&str; 5] = ["id", "title", "projectPath", "createdAt", "updatedAt"];

/// One writer at a time, so a save and a delete can't interleave their index updates.
static LOCK: Mutex<()> = Mutex::new(());

fn store_dir() -> Result<PathBuf, String> {
    let dir = config_path()?.parent().ok_or("Config path has no parent.")?.join("code-chats");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Session ids are the app's own (`local-<uuid>`), but hashing keeps the file name safe whatever they hold.
fn chat_file(dir: &Path, id: &str) -> PathBuf {
    let digest = Sha256::digest(id.as_bytes());
    let name: String = digest.iter().take(16).map(|byte| format!("{byte:02x}")).collect();
    dir.join(format!("{name}.json"))
}

fn valid_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 64 {
        return Err("A session id is 1 to 64 characters.".into());
    }
    Ok(())
}

fn summary_of(chat: &Value) -> Option<Value> {
    let object = chat.as_object()?;
    object.get("id")?.as_str()?;
    let summary: Map<String, Value> = SUMMARY_FIELDS
        .iter()
        .filter_map(|field| object.get(*field).map(|value| (field.to_string(), value.clone())))
        .collect();
    Some(Value::Object(summary))
}

fn read_index(dir: &Path) -> Option<Map<String, Value>> {
    let text = fs::read_to_string(dir.join(INDEX_FILE)).ok()?;
    serde_json::from_str::<Value>(&text).ok()?.as_object().cloned()
}

/// The listing, rebuilt from the session files themselves.
fn rebuild_index(dir: &Path) -> Map<String, Value> {
    let mut index = Map::new();
    let Ok(entries) = fs::read_dir(dir) else { return index };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some(INDEX_FILE) || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(summary) = fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()).as_ref().and_then(summary_of) else {
            continue;
        };
        if let Some(id) = summary.get("id").and_then(Value::as_str) {
            index.insert(id.to_string(), summary.clone());
        }
    }
    index
}

fn write_index(dir: &Path, index: &Map<String, Value>) -> Result<(), String> {
    let text = serde_json::to_string(index).map_err(|e| e.to_string())?;
    write_atomically(&dir.join(INDEX_FILE), &text)
}

fn load_index(dir: &Path) -> Map<String, Value> {
    match read_index(dir) {
        Some(index) => index,
        None => {
            let index = rebuild_index(dir);
            let _ = write_index(dir, &index);
            index
        }
    }
}

// ── Against a directory (tested directly) ─────────────────────────────────────

fn list_in(dir: &Path) -> Vec<Value> {
    let _guard = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut summaries: Vec<Value> = load_index(dir).into_values().collect();
    // Newest first, as the sidebar shows them.
    summaries.sort_by(|a, b| {
        let key = |value: &Value| value.get("createdAt").and_then(Value::as_str).unwrap_or("").to_string();
        key(b).cmp(&key(a))
    });
    summaries
}

fn get_in(dir: &Path, id: &str) -> Result<Option<Value>, String> {
    valid_id(id)?;
    let Ok(text) = fs::read_to_string(chat_file(dir, id)) else { return Ok(None) };
    let chat: Value = serde_json::from_str(&text).map_err(|e| format!("This session's file is damaged: {e}"))?;
    // The hash could in principle collide; the id inside is the real check.
    Ok((chat.get("id").and_then(Value::as_str) == Some(id)).then_some(chat))
}

fn save_in(dir: &Path, chat: &Value) -> Result<(), String> {
    let id = chat.get("id").and_then(Value::as_str).ok_or("A session needs an id.")?;
    valid_id(id)?;
    let text = serde_json::to_string(chat).map_err(|e| e.to_string())?;
    if text.len() > MAX_CHAT_BYTES {
        return Err("This session is too large to save.".into());
    }
    let summary = summary_of(chat).ok_or("A session needs an id.")?;

    let _guard = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    write_atomically(&chat_file(dir, id), &text)?;
    let mut index = load_index(dir);
    index.insert(id.to_string(), summary);
    write_index(dir, &index)
}

fn remove_in(dir: &Path, id: &str) -> Result<(), String> {
    valid_id(id)?;
    let _guard = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match fs::remove_file(chat_file(dir, id)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let mut index = load_index(dir);
    if index.remove(id).is_some() {
        write_index(dir, &index)?;
    }
    Ok(())
}

// ── Commands ─────────────────────────────────────────────────────────────────

pub fn list() -> Result<Vec<Value>, String> {
    Ok(list_in(&store_dir()?))
}

pub fn get(id: &str) -> Result<Option<Value>, String> {
    get_in(&store_dir()?, id)
}

pub fn save(chat: &Value) -> Result<(), String> {
    save_in(&store_dir()?, chat)
}

pub fn remove(id: &str) -> Result<(), String> {
    remove_in(&store_dir()?, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aloe_code_chats_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn chat(id: &str, title: &str, created: &str) -> Value {
        json!({
            "version": 1,
            "id": id,
            "title": title,
            "projectPath": "C:/repo",
            "createdAt": created,
            "updatedAt": created,
            "messages": [
                { "id": "u1", "role": "user", "content": "hi" },
                { "id": "a1", "role": "assistant", "content": "hello", "metadata": { "turnContents": [{ "role": "model", "parts": [{ "text": "hello" }] }] } }
            ]
        })
    }

    #[test]
    fn saves_lists_and_reads_back() {
        let dir = temp_dir("roundtrip");
        save_in(&dir, &chat("local-a", "First", "2026-11-01T00:00:00Z")).unwrap();
        save_in(&dir, &chat("local-b", "Second", "2026-11-02T00:00:00Z")).unwrap();

        let listed = list_in(&dir);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["id"], "local-b", "newest first");
        assert!(listed[0].get("messages").is_none(), "the listing leaves the messages out");

        let read = get_in(&dir, "local-a").unwrap().unwrap();
        assert_eq!(read["messages"][1]["metadata"]["turnContents"][0]["parts"][0]["text"], "hello");
        assert!(get_in(&dir, "local-missing").unwrap().is_none());
    }

    #[test]
    fn saving_again_replaces_and_updates_the_listing() {
        let dir = temp_dir("replace");
        save_in(&dir, &chat("local-a", "Old", "2026-11-01T00:00:00Z")).unwrap();
        save_in(&dir, &chat("local-a", "New", "2026-11-01T00:00:00Z")).unwrap();
        let listed = list_in(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["title"], "New");
    }

    #[test]
    fn removing_drops_the_file_and_the_listing() {
        let dir = temp_dir("remove");
        save_in(&dir, &chat("local-a", "A", "2026-11-01T00:00:00Z")).unwrap();
        remove_in(&dir, "local-a").unwrap();
        remove_in(&dir, "local-a").unwrap();
        assert!(list_in(&dir).is_empty());
        assert!(get_in(&dir, "local-a").unwrap().is_none());
    }

    #[test]
    fn a_lost_index_is_rebuilt_from_the_files() {
        let dir = temp_dir("rebuild");
        save_in(&dir, &chat("local-a", "A", "2026-11-01T00:00:00Z")).unwrap();
        fs::write(dir.join(INDEX_FILE), "not json").unwrap();
        let listed = list_in(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["title"], "A");
    }

    #[test]
    fn rejects_bad_ids() {
        let dir = temp_dir("ids");
        assert!(save_in(&dir, &json!({ "title": "no id" })).is_err());
        assert!(save_in(&dir, &json!({ "id": "" })).is_err());
        assert!(get_in(&dir, &"x".repeat(65)).is_err());
    }
}
