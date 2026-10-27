//! Aloe Code's Changes pane: what Aloe's own file writes did to a project, per session.
//!
//! Before the first write a job makes to a path in a conversation, that file's original content is
//! saved under the app's config directory; the pane diffs it against what is on disk now. This is
//! deliberately not git: running `git status` on a repo runs that repo's config (see project.rs),
//! and it would also mix in the user's own uncommitted work. The cost is that files changed by a
//! command (a formatter, codegen) are not tracked, which the pane says.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Emitter};

use crate::config::{config_path, debug_log};
use crate::fs::{assert_granted, assert_safe_write};
use crate::models::AgentConfig;

/// Larger files are recorded as changed, without content: no diff, no revert.
pub const MAX_TRACKED_FILE_BYTES: u64 = 1_000_000;
/// Files recorded per session; writes beyond this still happen, they just aren't tracked.
const MAX_TRACKED_FILES: usize = 500;
/// Session logs kept on disk; the oldest go first.
const MAX_TRACKED_SESSIONS: usize = 200;

/// Job kinds that write the file at `input.path`.
fn is_tracked_write(kind: &str) -> bool {
    matches!(kind, "write_text_file" | "update_file" | "write_local_file" | "create_file" | "delete_file" | "write_binary_file")
}

/// A file's content at one moment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// `None` with `untracked: false`: the file did not exist.
    pub content: Option<String>,
    /// The file existed but was too large or not UTF-8 text, so its content was not kept.
    #[serde(default)]
    pub untracked: bool,
    #[serde(default)]
    pub recorded_at: String,
}

impl Snapshot {
    fn existed(&self) -> bool {
        self.content.is_some() || self.untracked
    }
}

/// Canonical path → the file before Aloe's first write to it in the session.
type Log = BTreeMap<String, Snapshot>;

pub fn read_snapshot(path: &Path) -> Snapshot {
    let recorded_at = Utc::now().to_rfc3339();
    let untracked = |recorded_at| Snapshot { content: None, untracked: true, recorded_at };
    match fs::metadata(path) {
        Err(_) => Snapshot { content: None, untracked: false, recorded_at },
        Ok(meta) if !meta.is_file() || meta.len() > MAX_TRACKED_FILE_BYTES => untracked(recorded_at),
        Ok(_) => match fs::read(path).ok().and_then(|bytes| String::from_utf8(bytes).ok()) {
            Some(text) => Snapshot { content: Some(text), untracked: false, recorded_at },
            None => untracked(recorded_at),
        },
    }
}

/// The key a path is logged under: canonical when it exists, else its canonical parent plus the
/// name, so the same file maps to the same key before and after it is created or deleted.
fn path_key(path: &Path) -> String {
    let canonical = fs::canonicalize(path).ok().or_else(|| {
        let parent = fs::canonicalize(path.parent()?).ok()?;
        Some(parent.join(path.file_name()?))
    });
    canonical.unwrap_or_else(|| path.to_path_buf()).to_string_lossy().to_string()
}

/// A write that is about to run, with the file as it was before it.
pub struct PendingWrite {
    conversation_id: String,
    path: PathBuf,
    before: Snapshot,
}

/// Called before a job runs. Only reads files inside the job's granted folders: a write outside
/// them fails anyway, and this must never copy an ungranted file into Aloe's own storage.
pub fn before_job(config: &AgentConfig, conversation_id: Option<&str>, kind: &str, input: &Value) -> Option<PendingWrite> {
    let conversation_id = conversation_id?;
    if !is_tracked_write(kind) {
        return None;
    }
    let path = PathBuf::from(input.get("path")?.as_str()?);
    let before = if assert_granted(config, &path.to_string_lossy()).is_ok() {
        read_snapshot(&path)
    } else if !path.exists() {
        // A new file under folders that don't exist yet (write_text_file creates them).
        Snapshot { content: None, untracked: false, recorded_at: Utc::now().to_rfc3339() }
    } else {
        return None;
    };
    Some(PendingWrite { conversation_id: conversation_id.to_string(), path, before })
}

/// Called once the job succeeded: keeps the original if this is the session's first write to the
/// file, and tells the Code window either way, since the file's current content changed.
pub fn after_job(app: &AppHandle, pending: PendingWrite) {
    let key = path_key(&pending.path);
    if let Err(error) = record(&pending.conversation_id, &key, pending.before) {
        debug_log("changes", "record_failed", error);
    }
    let _ = app.emit("agent://code-changes", json!({ "conversationId": pending.conversation_id, "path": key }));
}

fn record(conversation_id: &str, key: &str, before: Snapshot) -> Result<(), String> {
    let dir = store_dir()?;
    record_in(&dir, conversation_id, key, before)
}

fn record_in(dir: &Path, conversation_id: &str, key: &str, before: Snapshot) -> Result<(), String> {
    let mut log = load_from(dir, conversation_id);
    if log.contains_key(key) || log.len() >= MAX_TRACKED_FILES {
        return Ok(());
    }
    log.insert(key.to_string(), before);
    save_to(dir, conversation_id, &log)?;
    prune(dir);
    Ok(())
}

// ── Storage ──────────────────────────────────────────────────────────────────

fn store_dir() -> Result<PathBuf, String> {
    let dir = config_path()?.parent().ok_or("Config path has no parent.")?.join("code-changes");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Conversation ids come from the backend; hashing them keeps the file name safe whatever they hold.
fn log_file(dir: &Path, conversation_id: &str) -> PathBuf {
    let digest = Sha256::digest(conversation_id.as_bytes());
    let name: String = digest.iter().take(16).map(|byte| format!("{byte:02x}")).collect();
    dir.join(format!("{name}.json"))
}

fn load_from(dir: &Path, conversation_id: &str) -> Log {
    fs::read_to_string(log_file(dir, conversation_id))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_to(dir: &Path, conversation_id: &str, log: &Log) -> Result<(), String> {
    let text = serde_json::to_string(log).map_err(|e| e.to_string())?;
    fs::write(log_file(dir, conversation_id), text).map_err(|e| e.to_string())
}

fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    if files.len() <= MAX_TRACKED_SESSIONS {
        return;
    }
    files.sort();
    for (_, path) in files.iter().take(files.len() - MAX_TRACKED_SESSIONS) {
        let _ = fs::remove_file(path);
    }
}

// ── What the pane asks for ───────────────────────────────────────────────────

/// Every file Aloe changed in this session under `project`, with its original and current text.
/// Files back to how they started (or created and then deleted again) are left out.
pub fn session_changes(config: &AgentConfig, conversation_id: &str, project: &str) -> Result<Value, String> {
    let root = assert_granted(config, project)?;
    session_changes_in(&store_dir()?, &root, conversation_id)
}

fn session_changes_in(dir: &Path, root: &Path, conversation_id: &str) -> Result<Value, String> {
    let mut files = Vec::new();
    for (key, before) in load_from(dir, conversation_id) {
        let path = PathBuf::from(&key);
        if !path.starts_with(root) {
            continue;
        }
        let now = read_snapshot(&path);
        let status = match (before.existed(), now.existed()) {
            (false, false) => continue,
            (false, true) => "added",
            (true, false) => "deleted",
            (true, true) if !before.untracked && !now.untracked && before.content == now.content => continue,
            (true, true) => "modified",
        };
        let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        files.push(json!({
            "path": key,
            "relativePath": relative,
            "status": status,
            "original": before.content,
            "current": now.content,
            "binary": before.untracked || now.untracked,
        }));
    }
    Ok(json!({ "files": files }))
}

/// Puts a file back the way it was before Aloe's first write to it in this session.
pub fn revert_file(config: &AgentConfig, conversation_id: &str, path: &str) -> Result<(), String> {
    revert_in(&store_dir()?, config, conversation_id, path)
}

fn revert_in(dir: &Path, config: &AgentConfig, conversation_id: &str, path: &str) -> Result<(), String> {
    let log = load_from(dir, conversation_id);
    let before = log.get(path).ok_or("Aloe has no record of this file before the session changed it.")?;
    if before.untracked {
        return Err("This file was too large or not text, so its original wasn't kept and it can't be reverted here.".to_string());
    }
    let target = assert_granted(config, path)?;
    assert_safe_write(&target, &json!({}))?;
    match &before.content {
        Some(text) => fs::write(&target, text).map_err(|e| e.to_string()),
        None if target.exists() => fs::remove_file(&target).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::GrantedFolder;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aloe_changes_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::canonicalize(&dir).unwrap()
    }

    fn granting(dir: &Path) -> AgentConfig {
        AgentConfig {
            folders: vec![GrantedFolder { path: dir.to_string_lossy().to_string(), label: None, indexed_at: None, context: None, context_updated_at: None }],
            ..Default::default()
        }
    }

    fn write_job(config: &AgentConfig, path: &Path) -> PendingWrite {
        before_job(config, Some("conv-1"), "write_text_file", &json!({ "path": path.to_string_lossy() })).expect("tracked")
    }

    #[test]
    fn keeps_only_the_first_original_of_each_file() {
        let (store, project) = (temp_dir("first_store"), temp_dir("first_project"));
        let config = granting(&project);
        let file = project.join("a.txt");
        fs::write(&file, "one").unwrap();

        let first = write_job(&config, &file);
        fs::write(&file, "two").unwrap();
        record_in(&store, "conv-1", &path_key(&first.path), first.before).unwrap();
        let second = write_job(&config, &file);
        fs::write(&file, "three").unwrap();
        record_in(&store, "conv-1", &path_key(&second.path), second.before).unwrap();

        let changes = session_changes_in(&store, &project, "conv-1").unwrap();
        assert_eq!(changes["files"][0]["original"], "one");
        assert_eq!(changes["files"][0]["current"], "three");
        assert_eq!(changes["files"][0]["status"], "modified");
    }

    #[test]
    fn reports_added_and_drops_files_back_to_their_original() {
        let (store, project) = (temp_dir("status_store"), temp_dir("status_project"));
        let config = granting(&project);
        let added = project.join("new.txt");
        let restored = project.join("same.txt");
        fs::write(&restored, "same").unwrap();

        let pending = write_job(&config, &added);
        fs::write(&added, "hi").unwrap();
        record_in(&store, "conv-1", &path_key(&pending.path), pending.before).unwrap();
        let pending = write_job(&config, &restored);
        record_in(&store, "conv-1", &path_key(&pending.path), pending.before).unwrap();

        let files = session_changes_in(&store, &project, "conv-1").unwrap()["files"].as_array().unwrap().clone();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["status"], "added");
        assert_eq!(files[0]["relativePath"], "new.txt");
    }

    #[test]
    fn revert_restores_the_original_bytes_or_removes_a_new_file() {
        let (store, project) = (temp_dir("revert_store"), temp_dir("revert_project"));
        let config = granting(&project);
        let edited = project.join("a.txt");
        let created = project.join("b.txt");
        fs::write(&edited, "line\r\noriginal\r\n").unwrap();

        for file in [&edited, &created] {
            let pending = write_job(&config, file);
            record_in(&store, "conv-1", &path_key(&pending.path), pending.before).unwrap();
            fs::write(file, "changed").unwrap();
        }
        revert_in(&store, &config, "conv-1", &path_key(&edited)).unwrap();
        revert_in(&store, &config, "conv-1", &path_key(&created)).unwrap();

        assert_eq!(fs::read(&edited).unwrap(), b"line\r\noriginal\r\n");
        assert!(!created.exists());
    }

    #[test]
    fn never_reads_files_outside_granted_folders() {
        let (project, outside) = (temp_dir("grant_project"), temp_dir("grant_outside"));
        let file = outside.join("secret.txt");
        fs::write(&file, "nope").unwrap();
        assert!(before_job(&granting(&project), Some("conv-1"), "update_file", &json!({ "path": file.to_string_lossy() })).is_none());
    }

    #[test]
    fn revert_refuses_sensitive_paths_and_other_projects() {
        let (store, project) = (temp_dir("sensitive_store"), temp_dir("sensitive_project"));
        let config = granting(&project);
        let env = project.join(".env");
        fs::write(&env, "A=1").unwrap();
        let pending = write_job(&config, &env);
        record_in(&store, "conv-1", &path_key(&pending.path), pending.before).unwrap();
        fs::write(&env, "A=2").unwrap();

        assert!(revert_in(&store, &config, "conv-1", &path_key(&env)).is_err());
        assert!(revert_in(&store, &granting(&temp_dir("sensitive_other")), "conv-1", &path_key(&env)).is_err());
    }

    #[test]
    fn ignores_jobs_without_a_conversation_or_that_dont_write() {
        let project = temp_dir("ignore_project");
        let config = granting(&project);
        let input = json!({ "path": project.join("a.txt").to_string_lossy() });
        assert!(before_job(&config, None, "write_text_file", &input).is_none());
        assert!(before_job(&config, Some("conv-1"), "read_file", &input).is_none());
    }
}
