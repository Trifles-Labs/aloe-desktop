use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::Stdio,
};
use tokio::process::Command;

use crate::config::MAX_TEXT_BYTES;
use crate::models::AgentConfig;

// ── Path helpers ──────────────────────────────────────────────────────────────

fn canonicalize_existing_or_parent(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return fs::canonicalize(path).map_err(|e| e.to_string());
    }
    let parent = path.parent().ok_or("Path has no parent directory.")?;
    let canonical_parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    let filename = path.file_name().ok_or("Path has no filename.")?;
    Ok(canonical_parent.join(filename))
}

pub fn assert_granted(config: &AgentConfig, raw_path: &str) -> Result<PathBuf, String> {
    let target = canonicalize_existing_or_parent(Path::new(raw_path))?;
    for folder in &config.folders {
        let root = fs::canonicalize(&folder.path).map_err(|e| e.to_string())?;
        if target == root || target.starts_with(&root) {
            return Ok(target);
        }
    }
    Err(format!("Path is outside Aloe granted folders: {raw_path}"))
}

pub fn assert_safe_write(path: &Path) -> Result<(), String> {
    let normalized = path.to_string_lossy().replace('\\', "/").to_lowercase();
    let filename = path.file_name().map(|v| v.to_string_lossy().to_lowercase()).unwrap_or_default();
    let blocked = normalized.contains("/.git/")
        || normalized.contains("/node_modules/")
        || normalized.contains("/dist/")
        || normalized.contains("/build/")
        || normalized.contains("/.next/")
        || filename == ".env"
        || filename.starts_with(".env.")
        || filename.ends_with(".pem")
        || filename.ends_with(".key")
        || filename.ends_with(".p12")
        || filename.ends_with(".pfx")
        || filename.contains("secret");
    if blocked {
        Err(format!("Aloe blocks writes to sensitive paths: {}", path.display()))
    } else {
        Ok(())
    }
}

// ── Input helpers ─────────────────────────────────────────────────────────────

pub fn input_string(input: &Value, key: &str) -> Result<String, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("{key} is required"))
}

// ── File operations ───────────────────────────────────────────────────────────

pub fn list_files(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let path = assert_granted(config, &input_string(input, "path")?)?;
    let entries = fs::read_dir(&path)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .take(200)
        .map(|entry| {
            let meta = entry.metadata().ok();
            json!({
                "path": entry.path().to_string_lossy(),
                "name": entry.file_name().to_string_lossy(),
                "kind": if meta.as_ref().is_some_and(|m| m.is_dir()) { "directory" } else { "file" },
                "bytes": meta.filter(|m| m.is_file()).map(|m| m.len()),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "path": path.to_string_lossy(), "entries": entries }))
}

pub fn read_file(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let path = assert_granted(config, &input_string(input, "path")?)?;
    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let lines: Vec<&str> = content.lines().collect();
    let start = input.get("startLine").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
    let end = input.get("endLine").and_then(Value::as_u64).unwrap_or(lines.len() as u64).max(start as u64) as usize;
    let selected = lines.iter().skip(start - 1).take(end - start + 1).copied().collect::<Vec<_>>().join("\n");
    Ok(json!({ "path": path.to_string_lossy(), "file": truncate_text(selected), "startLine": start, "endLine": end.min(lines.len()), "totalLines": lines.len() }))
}

fn mime_type_from_path(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("heic") => "image/heic",
        Some("heif") => "image/heif",
        Some("pdf") => "application/pdf",
        Some("mp3") => "audio/mp3",
        Some("wav") => "audio/wav",
        Some("aac") => "audio/aac",
        Some("ogg") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("aiff") | Some("aif") => "audio/aiff",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/mov",
        Some("mpeg") | Some("mpg") => "video/mpeg",
        Some("webm") => "video/webm",
        Some("avi") => "video/avi",
        Some("3gp") | Some("3gpp") => "video/3gpp",
        _ => "application/octet-stream",
    }
}

const MAX_ATTACH_BYTES: u64 = 15_000_000; // ~15 MB raw

pub fn attach_file(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let path = assert_granted(config, &input_string(input, "path")?)?;
    let meta = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("Path is not a file: {}", path.display()));
    }
    let size = meta.len();
    if size > MAX_ATTACH_BYTES {
        return Err(format!(
            "File is too large to attach ({} bytes). Maximum is {} bytes.",
            size, MAX_ATTACH_BYTES
        ));
    }
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    let mime_type = mime_type_from_path(&path);
    let base64 = BASE64.encode(&bytes);
    Ok(json!({
        "path": path.to_string_lossy(),
        "mimeType": mime_type,
        "base64": base64,
        "sizeBytes": size,
    }))
}

pub fn create_file(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let path = assert_granted(config, &input_string(input, "path")?)?;
    assert_safe_write(&path)?;
    if path.exists() {
        return Err(format!("File already exists: {}", path.display()));
    }
    fs::write(&path, input_string(input, "content")?).map_err(|e| e.to_string())?;
    Ok(json!({ "path": path.to_string_lossy(), "created": true }))
}

pub fn update_file(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let path = assert_granted(config, &input_string(input, "path")?)?;
    assert_safe_write(&path)?;
    fs::write(&path, input_string(input, "content")?).map_err(|e| e.to_string())?;
    Ok(json!({ "path": path.to_string_lossy(), "written": true }))
}

pub fn delete_file(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let path = assert_granted(config, &input_string(input, "path")?)?;
    assert_safe_write(&path)?;
    if !path.is_file() {
        return Err(format!("Path is not a file: {}", path.display()));
    }
    fs::remove_file(&path).map_err(|e| e.to_string())?;
    Ok(json!({ "path": path.to_string_lossy(), "deleted": true }))
}

// ── Folder operations ─────────────────────────────────────────────────────────

pub fn create_folder(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let path = assert_granted(config, &input_string(input, "path")?)?;
    assert_safe_write(&path)?;
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(json!({ "path": path.to_string_lossy(), "created": true }))
}

pub fn update_folder(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let path = assert_granted(config, &input_string(input, "path")?)?;
    let new_path = assert_granted(config, &input_string(input, "newPath")?)?;
    assert_safe_write(&path)?;
    assert_safe_write(&new_path)?;
    if !path.is_dir() {
        return Err(format!("Path is not a folder: {}", path.display()));
    }
    fs::rename(&path, &new_path).map_err(|e| e.to_string())?;
    Ok(json!({ "path": path.to_string_lossy(), "newPath": new_path.to_string_lossy(), "updated": true }))
}

pub fn delete_folder(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let path = assert_granted(config, &input_string(input, "path")?)?;
    assert_safe_write(&path)?;
    if !path.is_dir() {
        return Err(format!("Path is not a folder: {}", path.display()));
    }
    if input.get("recursive").and_then(Value::as_bool).unwrap_or(false) {
        fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
    } else {
        fs::remove_dir(&path).map_err(|e| e.to_string())?;
    }
    Ok(json!({ "path": path.to_string_lossy(), "deleted": true }))
}

// ── Patch ─────────────────────────────────────────────────────────────────────

fn patch_target_paths(patch: &str) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for line in patch.lines() {
        if let Some(raw) = line.strip_prefix("+++ ") {
            if raw == "/dev/null" {
                continue;
            }
            let clean = raw.trim().strip_prefix("b/").unwrap_or(raw.trim());
            let path = PathBuf::from(clean);
            if path.is_absolute() || path.components().any(|p| matches!(p, Component::ParentDir)) {
                return Err("Patch contains an unsafe target path.".to_string());
            }
            paths.push(path);
        }
    }
    Ok(paths)
}

pub async fn apply_patch(config: AgentConfig, input: Value) -> Result<Value, String> {
    let root = assert_granted(&config, &input_string(&input, "path")?)?;
    let patch = input_string(&input, "patch")?;
    for relative in patch_target_paths(&patch)? {
        assert_safe_write(&root.join(relative))?;
    }
    let mut child = Command::new("git")
        .args(["apply", "--whitespace=nowarn"])
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(patch.as_bytes()).await.map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().await.map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok(json!({
        "path": root.to_string_lossy(),
        "applied": true,
        "stdout": String::from_utf8_lossy(&output.stdout),
    }))
}

// kept in config.rs but re-exported here for use in read_file / update_file
pub fn truncate_text(value: String) -> Value {
    if value.len() <= MAX_TEXT_BYTES {
        return json!({ "content": value, "truncated": false });
    }
    let mut end = MAX_TEXT_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    json!({ "content": &value[..end], "truncated": true })
}
