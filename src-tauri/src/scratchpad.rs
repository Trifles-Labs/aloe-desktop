//! The scratchpad: a folder of Aloe's own on this computer for temporary work (a throwaway script,
//! a downloaded file, an intermediate result) so it never has to clutter a granted project or ask
//! for a new folder. Each conversation gets `<root>/<conversation id>`, and the backend tells the
//! model that path. A job may use only its own conversation's folder: `executor::dispatch_tool`
//! sets `AgentConfig::scratchpad_session` on the copy it checks paths against, and
//! `fs::assert_granted` accepts paths inside it with the same checks as anywhere else (sensitive
//! writes, command approvals). Session folders untouched for a week are deleted at startup.

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, SystemTime},
};

use crate::config::debug_log;

/// How long a conversation's scratch folder is kept after its last change.
pub const SCRATCHPAD_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// `<local data dir>/Aloe Desktop/scratchpad`, created and canonicalized once. None when the OS has
/// no local data dir or it can't be created, in which case there is simply no scratchpad.
pub fn scratchpad_root() -> Option<&'static Path> {
    static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
    ROOT.get_or_init(|| {
        let dir = dirs::data_local_dir()?.join("Aloe Desktop").join("scratchpad");
        if let Err(error) = fs::create_dir_all(&dir) {
            debug_log("scratchpad", "create_failed", error.to_string());
            return None;
        }
        fs::canonicalize(&dir).ok()
    })
    .as_deref()
}

/// The root as the backend and the model see it: forward slashes, no `\\?\` prefix.
pub fn display_root() -> Option<String> {
    scratchpad_root().map(display_path)
}

fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\").unwrap_or(&text).replace('\\', "/")
}

/// A conversation id as a folder name: only `[A-Za-z0-9_-]`, at most 64 characters. The backend
/// names the folder the same way (`scratchpadPathFor` in lib/tools/local_agent.ts).
fn session_folder_name(conversation_id: &str) -> Option<String> {
    let name: String = conversation_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    (!name.is_empty()).then_some(name)
}

fn mentions(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.replace('\\', "/").to_lowercase().contains(needle),
        Value::Array(items) => items.iter().any(|item| mentions(item, needle)),
        Value::Object(map) => map.values().any(|item| mentions(item, needle)),
        _ => false,
    }
}

/// The conversation's scratch folder for this job, created first when the job refers to it so a
/// command's working directory or a search there works on first use. None for a job with no
/// conversation, or when there is no scratchpad.
pub fn session_for_job(conversation_id: Option<&str>, input: &Value) -> Option<PathBuf> {
    session_in(scratchpad_root()?, conversation_id, input)
}

fn session_in(root: &Path, conversation_id: Option<&str>, input: &Value) -> Option<PathBuf> {
    let session = root.join(session_folder_name(conversation_id?)?);
    if !session.exists() && mentions(input, &display_path(&session).to_lowercase()) {
        if let Err(error) = fs::create_dir_all(&session) {
            debug_log("scratchpad", "session_create_failed", error.to_string());
        }
    }
    Some(session)
}

/// The newest modification time of anything under `path`, the folder itself included.
fn newest_change(path: &Path) -> Option<SystemTime> {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok()?.modified().ok())
        .max()
}

/// Deletes session folders with nothing changed in `max_age`. Best-effort: a folder that can't be
/// read or removed (a file still open on Windows) is left for the next start.
pub fn cleanup(root: &Path, max_age: Duration, now: SystemTime) -> usize {
    let Ok(entries) = fs::read_dir(root) else { return 0 };
    let mut removed = 0;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let stale = newest_change(&path)
            .and_then(|changed| now.duration_since(changed).ok())
            .is_some_and(|age| age > max_age);
        if !stale {
            continue;
        }
        match fs::remove_dir_all(&path) {
            Ok(()) => removed += 1,
            Err(error) => debug_log("scratchpad", "cleanup_failed", format!("{}: {error}", path.display())),
        }
    }
    removed
}

/// Run once at startup, off the main thread.
pub fn cleanup_in_background() {
    std::thread::spawn(|| {
        if let Some(root) = scratchpad_root() {
            let removed = cleanup(root, SCRATCHPAD_MAX_AGE, SystemTime::now());
            if removed > 0 {
                debug_log("scratchpad", "cleanup", format!("removed={removed}"));
            }
        }
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aloe_scratchpad_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::canonicalize(&dir).unwrap()
    }

    #[test]
    fn conversation_ids_become_safe_folder_names() {
        assert_eq!(session_folder_name("abc-123_x").as_deref(), Some("abc-123_x"));
        assert_eq!(session_folder_name("../../etc").as_deref(), Some("etc"));
        assert_eq!(session_folder_name("/\\.."), None);
    }

    #[test]
    fn creates_the_session_folder_only_for_jobs_that_use_it() {
        let root = temp_root("ensure");
        let inside = format!("{}/conv1/test.py", display_path(&root));
        let other = format!("{}/conv2/test.py", display_path(&root));

        assert_eq!(session_in(&root, Some("conv1"), &json!({ "path": other })), Some(root.join("conv1")));
        assert!(!root.join("conv1").exists());
        assert!(!root.join("conv2").exists());

        session_in(&root, Some("conv1"), &json!({ "path": inside }));
        assert!(root.join("conv1").is_dir());
        assert_eq!(session_in(&root, None, &json!({})), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cleanup_removes_only_stale_session_folders() {
        let root = temp_root("cleanup");
        fs::create_dir_all(root.join("old")).unwrap();
        fs::write(root.join("old/a.txt"), "x").unwrap();
        fs::create_dir_all(root.join("fresh")).unwrap();
        // Seen from now nothing is stale; from eight days on, both are.
        assert_eq!(cleanup(&root, SCRATCHPAD_MAX_AGE, SystemTime::now()), 0);
        let later = SystemTime::now() + Duration::from_secs(8 * 24 * 60 * 60);
        assert_eq!(cleanup(&root, SCRATCHPAD_MAX_AGE, later), 2);
        assert!(!root.join("old").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
