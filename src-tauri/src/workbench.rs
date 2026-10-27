//! Aloe Code's Files pane: the project tree and the files the user opens from it.
//!
//! The webview calls these directly (they are Tauri commands, not agent jobs), so each one checks
//! the path against the granted folders itself, and writes follow the same sensitive-path rules as
//! Aloe's own writes.

use serde_json::{json, Value};
use std::{fs, path::Path};

use crate::changes::MAX_TRACKED_FILE_BYTES;
use crate::fs::{assert_granted, assert_safe_write};
use crate::models::AgentConfig;

/// Folders the tree never lists: dependencies, build output and VCS internals.
const HIDDEN_DIRS: [&str; 7] = [".git", "node_modules", "target", "dist", ".next", "__pycache__", ".venv"];
/// Entries listed per folder.
const MAX_DIR_ENTRIES: usize = 1_000;

/// One level of the tree under `path`, folders first, each group by name.
pub fn list_dir(config: &AgentConfig, path: &str) -> Result<Value, String> {
    let dir = assert_granted(config, path)?;
    if !dir.is_dir() {
        return Err(format!("{} is not a folder.", dir.display()));
    }
    let mut entries: Vec<(bool, String, String)> = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().ok()?.is_dir();
            if is_dir && HIDDEN_DIRS.contains(&name.as_str()) {
                return None;
            }
            Some((is_dir, name, entry.path().to_string_lossy().to_string()))
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let truncated = entries.len() > MAX_DIR_ENTRIES;
    entries.truncate(MAX_DIR_ENTRIES);
    let entries: Vec<Value> = entries
        .into_iter()
        .map(|(is_dir, name, path)| json!({ "name": name, "path": path, "isDir": is_dir }))
        .collect();
    Ok(json!({ "entries": entries, "truncated": truncated }))
}

/// A text file's content, for the editor. Binary and very large files are refused.
pub fn read_text(config: &AgentConfig, path: &str) -> Result<String, String> {
    let file = assert_granted(config, path)?;
    let meta = fs::metadata(&file).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("{} is not a file.", file.display()));
    }
    if meta.len() > MAX_TRACKED_FILE_BYTES {
        return Err("This file is over 1 MB, too large to open here.".to_string());
    }
    String::from_utf8(fs::read(&file).map_err(|e| e.to_string())?).map_err(|_| "This file isn't text, so it can't be opened here.".to_string())
}

/// Saves the user's edit to an existing file.
pub fn write_text(config: &AgentConfig, path: &str, content: &str) -> Result<(), String> {
    let file = assert_granted(config, path)?;
    if !file.is_file() {
        return Err(format!("{} is not an existing file.", file.display()));
    }
    assert_safe_write(&file, &json!({}))?;
    write_atomically(&file, content)
}

/// Written beside the file and renamed over it, so a crash mid-save never leaves half a file.
fn write_atomically(file: &Path, content: &str) -> Result<(), String> {
    let name = file.file_name().ok_or("Path has no file name.")?.to_string_lossy();
    let temp = file.with_file_name(format!(".{name}.aloe-save"));
    fs::write(&temp, content).map_err(|e| e.to_string())?;
    fs::rename(&temp, file).map_err(|e| {
        let _ = fs::remove_file(&temp);
        e.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::GrantedFolder;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aloe_workbench_test_{name}_{}", std::process::id()));
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

    #[test]
    fn lists_folders_first_and_hides_dependencies() {
        let dir = temp_dir("list");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        fs::write(dir.join("b.txt"), "").unwrap();
        fs::write(dir.join("A.md"), "").unwrap();

        let listed = list_dir(&granting(&dir), &dir.to_string_lossy()).unwrap();
        let names: Vec<&str> = listed["entries"].as_array().unwrap().iter().map(|entry| entry["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["src", "A.md", "b.txt"]);
    }

    #[test]
    fn refuses_paths_outside_the_grants() {
        let (project, outside) = (temp_dir("grant_project"), temp_dir("grant_outside"));
        fs::write(outside.join("a.txt"), "x").unwrap();
        let config = granting(&project);
        assert!(list_dir(&config, &outside.to_string_lossy()).is_err());
        assert!(read_text(&config, &outside.join("a.txt").to_string_lossy()).is_err());
        assert!(write_text(&config, &outside.join("a.txt").to_string_lossy(), "y").is_err());
    }

    #[test]
    fn saves_existing_text_files_but_not_sensitive_or_binary_ones() {
        let dir = temp_dir("save");
        let config = granting(&dir);
        let file = dir.join("a.txt");
        fs::write(&file, "old").unwrap();
        write_text(&config, &file.to_string_lossy(), "new").unwrap();
        assert_eq!(read_text(&config, &file.to_string_lossy()).unwrap(), "new");

        fs::write(dir.join(".env"), "A=1").unwrap();
        assert!(write_text(&config, &dir.join(".env").to_string_lossy(), "A=2").is_err());
        assert!(write_text(&config, &dir.join("missing.txt").to_string_lossy(), "x").is_err());

        fs::write(dir.join("image.bin"), [0xff, 0xfe, 0x00]).unwrap();
        assert!(read_text(&config, &dir.join("image.bin").to_string_lossy()).is_err());
    }
}
