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

// ── Folder context preload ──────────────────────────────────────────────────

/// Cap per file (MEMORY.md, AGENTS.md) and, doubled, for their combined content — keeps a
/// folder's contribution to the system prompt bounded no matter how large the file on disk is.
pub const MAX_FOLDER_CONTEXT_BYTES: usize = 4_000;

fn truncate_str(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &value[..end])
}

/// Reads a granted folder's own MEMORY.md and AGENTS.md, if present, so their content can ride
/// along the existing folder-sync path and be preloaded into the model's context without an
/// explicit read_file call. Returns None when neither file exists (or both are empty), so a
/// folder with no convention files contributes nothing.
pub fn scan_folder_context(root: &Path) -> Option<String> {
    let sections: Vec<String> = ["MEMORY.md", "AGENTS.md"]
        .into_iter()
        .filter_map(|name| {
            let content = fs::read_to_string(root.join(name)).ok()?;
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return None;
            }
            Some(format!("## {name}\n{}", truncate_str(trimmed, MAX_FOLDER_CONTEXT_BYTES)))
        })
        .collect();
    if sections.is_empty() {
        return None;
    }
    Some(truncate_str(&sections.join("\n\n"), MAX_FOLDER_CONTEXT_BYTES * 2))
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

use crate::config::MAX_ATTACH_BYTES;

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

// Generated binaries (.docx/.xlsx/.pptx/.pdf, screenshots) arrive base64-encoded because the job
// envelope is JSON. Unlike create_file this overwrites, since "save the report to my Documents
// folder" run twice should update the report rather than fail — the caller names the exact path,
// so an overwrite is never a surprise.
//
// The target folder must already exist: assert_granted canonicalises the parent to prove the path
// is inside a granted root, which cannot be done for a directory that isn't there yet. Callers
// name an existing folder, so this is a clearer failure than silently creating a tree.
pub fn write_binary_file(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let path = assert_granted(config, &input_string(input, "path")?)?;
    assert_safe_write(&path)?;

    let bytes = BASE64
        .decode(input_string(input, "base64")?)
        .map_err(|e| format!("base64 payload could not be decoded: {e}"))?;

    let existed = path.exists();
    fs::write(&path, &bytes).map_err(|e| e.to_string())?;

    Ok(json!({
        "path": path.to_string_lossy(),
        "sizeBytes": bytes.len(),
        "overwrote": existed,
    }))
}

// Exact-match find/replace, the same contract as an editor's find/replace: oldString must match
// the file's current text byte-for-byte and be unique unless replaceAll is set. This intentionally
// does not fall back to a full-content overwrite — a missing/ambiguous oldString is a real error
// the caller (the model) needs to see and correct by re-reading the file, not something to guess
// past silently.
pub fn update_file(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let path = assert_granted(config, &input_string(input, "path")?)?;
    assert_safe_write(&path)?;

    let old_string = input_string(input, "oldString")?;
    let new_string = input_string(input, "newString")?;
    let replace_all = input.get("replaceAll").and_then(Value::as_bool).unwrap_or(false);

    if old_string.is_empty() {
        return Err("oldString cannot be empty.".to_string());
    }
    if old_string == new_string {
        return Err("oldString and newString are identical — nothing to change.".to_string());
    }

    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;

    // read_file hands the model LF-only text (it rebuilds each file with `.lines().join("\n")`), so
    // on a CRLF file every multi-line oldString taken from a read is guaranteed to miss — the exact
    // failure that reads as "oldString was not found" on content the model just looked at. Retry
    // with CRLF line endings so the comparison happens in the file's own encoding, which also keeps
    // newString's endings consistent with the rest of the file instead of splicing LF into it.
    let (old_string, new_string) = if content.matches(old_string.as_str()).count() == 0
        && content.contains("\r\n")
        && old_string.contains('\n')
        && !old_string.contains('\r')
    {
        (old_string.replace('\n', "\r\n"), new_string.replace('\n', "\r\n"))
    } else {
        (old_string, new_string)
    };

    let occurrences = content.matches(old_string.as_str()).count();

    if occurrences == 0 {
        return Err(format!(
            "oldString was not found in {} — it must match the file's current text exactly, including whitespace and indentation. Read the file again to get an accurate snippet.",
            path.display()
        ));
    }
    if occurrences > 1 && !replace_all {
        return Err(format!(
            "oldString matches {occurrences} places in {} — include more surrounding context to make it unique, or set replaceAll to true to change every occurrence.",
            path.display()
        ));
    }

    let updated = if replace_all {
        content.replace(old_string.as_str(), new_string.as_str())
    } else {
        content.replacen(old_string.as_str(), new_string.as_str(), 1)
    };

    fs::write(&path, &updated).map_err(|e| e.to_string())?;
    Ok(json!({
        "path": path.to_string_lossy(),
        "written": true,
        "replacements": if replace_all { occurrences } else { 1 },
    }))
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

// `git apply` matches context lines byte-for-byte, so a stray CR is a mismatch, not whitespace it
// will forgive. The model never sees real line endings — read_file rebuilds every file as LF (see
// `read_file`) — yet it still emits CRLF into the patch payload often enough to matter, usually by
// mirroring Windows content it saw somewhere else, and a patch with mixed endings fails outright.
// Normalising to LF is safe in both directions: git happily applies an LF patch to a CRLF working
// file and preserves that file's endings, which is verified in `apply_patch_handles_crlf_payload`.
//
// The trailing newline matters just as much. A patch whose last line has no "\n" is rejected as
// "corrupt patch at line N", and a model emitting JSON strings drops it constantly.
fn normalize_patch(patch: &str) -> String {
    let mut normalized = String::with_capacity(patch.len() + 1);
    for line in patch.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        normalized.push_str(body.strip_suffix('\r').unwrap_or(body));
        normalized.push('\n');
    }
    normalized
}

// Header paths are validated on BOTH sides. Checking only "+++" left deletions unguarded: a hunk
// reading "--- a/.env" / "+++ /dev/null" has no target line to check, so it skipped
// assert_safe_write entirely and could delete the very files every other operation blocks.
fn patch_header_path(raw: &str, strip: &str) -> Result<Option<PathBuf>, String> {
    // git may append a tab and a timestamp to a header path.
    let raw = raw.split('\t').next().unwrap_or(raw).trim();
    if raw == "/dev/null" {
        return Ok(None);
    }
    let path = PathBuf::from(raw.strip_prefix(strip).unwrap_or(raw));
    if path.is_absolute() || path.components().any(|p| matches!(p, Component::ParentDir)) {
        return Err("Patch contains an unsafe target path.".to_string());
    }
    Ok(Some(path))
}

fn patch_target_paths(patch: &str) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for line in patch.lines() {
        let parsed = if let Some(raw) = line.strip_prefix("+++ ") {
            patch_header_path(raw, "b/")?
        } else if let Some(raw) = line.strip_prefix("--- ") {
            patch_header_path(raw, "a/")?
        } else {
            None
        };
        if let Some(path) = parsed {
            paths.push(path);
        }
    }
    Ok(paths)
}

async fn run_git_apply(root: &Path, patch: &str, extra: &[&str]) -> Result<(bool, String), String> {
    let mut args = vec!["apply", "--whitespace=nowarn", "--recount"];
    args.extend_from_slice(extra);
    let mut child = Command::new("git")
        .args(&args)
        .current_dir(root)
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
    Ok((output.status.success(), String::from_utf8_lossy(&output.stderr).trim().to_string()))
}

pub async fn apply_patch(config: AgentConfig, input: Value) -> Result<Value, String> {
    let root = assert_granted(&config, &input_string(&input, "path")?)?;
    let patch = normalize_patch(&input_string(&input, "patch")?);
    for relative in patch_target_paths(&patch)? {
        assert_safe_write(&root.join(relative))?;
    }

    // `--recount` makes the hunk header's line counts advisory, which removes the other routine
    // "corrupt patch" cause: a model that miscounts "@@ -1,9 +1,9 @@" for a hunk it wrote correctly.
    let (applied, strict_error) = run_git_apply(&root, &patch, &[]).await?;
    if applied {
        return Ok(json!({ "path": root.to_string_lossy(), "applied": true, "fuzzy": false }));
    }

    // Last resort. --ignore-whitespace can match a hunk whose indentation drifted, so it is never
    // the first attempt and the result says it was used — in an indentation-sensitive file the
    // caller needs to read the result back rather than trust a bare "applied".
    let (applied, loose_error) = run_git_apply(&root, &patch, &["--ignore-whitespace"]).await?;
    if applied {
        return Ok(json!({
            "path": root.to_string_lossy(),
            "applied": true,
            "fuzzy": true,
            "note": "Applied with --ignore-whitespace after an exact match failed; read the changed files back to confirm indentation is correct.",
        }));
    }

    let detail = if loose_error.is_empty() { strict_error.clone() } else { loose_error };
    Err(format!(
        "{detail}\n\nThe patch did not apply. Context lines must reproduce the file's current text exactly, including indentation. Re-read the affected file and rebuild the patch from what it actually contains; use update_file instead if the change is a single scoped replacement."
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::GrantedFolder;

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aloe_fs_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp project dir");
        fs::canonicalize(&dir).expect("canonicalize temp project dir")
    }

    fn config_granting(dir: &Path) -> AgentConfig {
        AgentConfig {
            folders: vec![GrantedFolder { path: dir.to_string_lossy().to_string(), label: None, indexed_at: None, context: None, context_updated_at: None }],
            ..Default::default()
        }
    }

    #[test]
    fn update_file_replaces_a_unique_match() {
        let dir = temp_project("unique");
        let file = dir.join("a.txt");
        fs::write(&file, "hello world").unwrap();
        let config = config_granting(&dir);

        let result = update_file(&config, &json!({
            "path": file.to_string_lossy(),
            "oldString": "world",
            "newString": "there",
        })).expect("update_file should succeed");

        assert_eq!(fs::read_to_string(&file).unwrap(), "hello there");
        assert_eq!(result["replacements"], 1);
    }

    #[test]
    fn update_file_errors_when_old_string_is_missing() {
        let dir = temp_project("missing");
        let file = dir.join("a.txt");
        fs::write(&file, "hello world").unwrap();
        let config = config_granting(&dir);

        let err = update_file(&config, &json!({
            "path": file.to_string_lossy(),
            "oldString": "goodbye",
            "newString": "hi",
        })).unwrap_err();

        assert!(err.contains("was not found"), "unexpected error: {err}");
        // A failed match must never touch the file.
        assert_eq!(fs::read_to_string(&file).unwrap(), "hello world");
    }

    #[test]
    fn update_file_errors_when_old_string_is_ambiguous_without_replace_all() {
        let dir = temp_project("ambiguous");
        let file = dir.join("a.txt");
        fs::write(&file, "foo foo foo").unwrap();
        let config = config_granting(&dir);

        let err = update_file(&config, &json!({
            "path": file.to_string_lossy(),
            "oldString": "foo",
            "newString": "bar",
        })).unwrap_err();

        assert!(err.contains("matches 3 places"), "unexpected error: {err}");
        assert_eq!(fs::read_to_string(&file).unwrap(), "foo foo foo");
    }

    #[test]
    fn update_file_replace_all_changes_every_occurrence() {
        let dir = temp_project("replace_all");
        let file = dir.join("a.txt");
        fs::write(&file, "foo foo foo").unwrap();
        let config = config_granting(&dir);

        let result = update_file(&config, &json!({
            "path": file.to_string_lossy(),
            "oldString": "foo",
            "newString": "bar",
            "replaceAll": true,
        })).expect("update_file should succeed");

        assert_eq!(fs::read_to_string(&file).unwrap(), "bar bar bar");
        assert_eq!(result["replacements"], 3);
    }

    #[test]
    fn update_file_rejects_identical_old_and_new_strings() {
        let dir = temp_project("identical");
        let file = dir.join("a.txt");
        fs::write(&file, "hello world").unwrap();
        let config = config_granting(&dir);

        let err = update_file(&config, &json!({
            "path": file.to_string_lossy(),
            "oldString": "world",
            "newString": "world",
        })).unwrap_err();

        assert!(err.contains("identical"), "unexpected error: {err}");
    }

    #[test]
    fn update_file_rejects_writes_outside_granted_folders() {
        let granted = temp_project("granted");
        let outside = temp_project("outside");
        let file = outside.join("a.txt");
        fs::write(&file, "hello world").unwrap();
        let config = config_granting(&granted);

        let err = update_file(&config, &json!({
            "path": file.to_string_lossy(),
            "oldString": "world",
            "newString": "there",
        })).unwrap_err();

        assert!(err.contains("outside"), "unexpected error: {err}");
        assert_eq!(fs::read_to_string(&file).unwrap(), "hello world");
    }

    // read_file shows the model LF, so this is the oldString it can actually produce for a CRLF file.
    #[test]
    fn update_file_matches_an_lf_snippet_against_a_crlf_file() {
        let dir = temp_project("crlf_update");
        let file = dir.join("a.txt");
        fs::write(&file, "one\r\ntwo\r\nthree\r\n").unwrap();
        let config = config_granting(&dir);

        update_file(&config, &json!({
            "path": file.to_string_lossy(),
            "oldString": "one\ntwo",
            "newString": "one\nTWO",
        })).expect("an LF snippet should match a CRLF file");

        // The file keeps its CRLF endings — the edit must not splice LF into a CRLF file.
        assert_eq!(fs::read_to_string(&file).unwrap(), "one\r\nTWO\r\nthree\r\n");
    }

    #[test]
    fn normalize_patch_strips_cr_and_adds_a_trailing_newline() {
        assert_eq!(normalize_patch("--- a/f\r\n+++ b/f\r\n"), "--- a/f\n+++ b/f\n");
        assert_eq!(normalize_patch(" context\n-old\n+new"), " context\n-old\n+new\n");
        // A mixed-ending payload is the worst case: git rejects it outright.
        assert_eq!(normalize_patch("a\r\nb\nc"), "a\nb\nc\n");
    }

    #[test]
    fn patch_target_paths_reads_both_header_sides() {
        let paths = patch_target_paths("--- a/src/x.ts\n+++ b/src/y.ts\n").unwrap();
        assert_eq!(paths, vec![PathBuf::from("src/x.ts"), PathBuf::from("src/y.ts")]);
    }

    // A deletion has no "+++" target, which previously meant no safe-write check ran at all.
    #[test]
    fn patch_target_paths_reports_the_source_of_a_deletion() {
        let paths = patch_target_paths("--- a/.env\n+++ /dev/null\n").unwrap();
        assert_eq!(paths, vec![PathBuf::from(".env")]);
    }

    #[test]
    fn patch_target_paths_rejects_an_escaping_source_path() {
        let err = patch_target_paths("--- a/../../secrets.txt\n+++ /dev/null\n").unwrap_err();
        assert!(err.contains("unsafe"), "unexpected error: {err}");
    }

    fn git_repo(name: &str) -> PathBuf {
        let dir = temp_project(name);
        for args in [vec!["init", "-q", "."], vec!["config", "user.email", "t@t"], vec!["config", "user.name", "t"]] {
            std::process::Command::new("git").args(&args).current_dir(&dir).output().expect("git");
        }
        dir
    }

    // Each of these payloads fails against the pre-fix implementation.
    #[tokio::test]
    async fn apply_patch_handles_crlf_payload_and_miscounted_hunks() {
        let dir = git_repo("apply_crlf");
        let file = dir.join("f.txt");
        fs::write(&file, "a\r\nb\r\nc\r\n").unwrap();
        let config = config_granting(&dir);

        // CRLF headers/context, deliberately wrong hunk counts, and no trailing newline.
        let patch = "--- a/f.txt\r\n+++ b/f.txt\r\n@@ -1,9 +1,9 @@\r\n a\r\n-b\r\n+B\r\n c";

        let result = apply_patch(config, json!({ "path": dir.to_string_lossy(), "patch": patch }))
            .await
            .expect("apply_patch should succeed");

        assert_eq!(result["applied"], true);
        assert_eq!(result["fuzzy"], false, "this should apply exactly, not via --ignore-whitespace");
        assert_eq!(fs::read_to_string(&file).unwrap(), "a\r\nB\r\nc\r\n");
    }

    #[tokio::test]
    async fn apply_patch_blocks_a_patch_that_deletes_a_sensitive_file() {
        let dir = git_repo("apply_delete_env");
        let file = dir.join(".env");
        fs::write(&file, "SECRET=1\n").unwrap();
        let config = config_granting(&dir);

        let err = apply_patch(config, json!({
            "path": dir.to_string_lossy(),
            "patch": "--- a/.env\n+++ /dev/null\n@@ -1 +0,0 @@\n-SECRET=1\n",
        })).await.unwrap_err();

        assert!(err.contains("sensitive"), "unexpected error: {err}");
        assert!(file.exists(), ".env must survive a deletion patch");
    }

    #[tokio::test]
    async fn apply_patch_explains_a_genuine_context_mismatch() {
        let dir = git_repo("apply_mismatch");
        fs::write(dir.join("f.txt"), "a\nb\nc\n").unwrap();
        let config = config_granting(&dir);

        let err = apply_patch(config, json!({
            "path": dir.to_string_lossy(),
            "patch": "--- a/f.txt\n+++ b/f.txt\n@@ -1,3 +1,3 @@\n a\n-nothing like this\n+z\n c\n",
        })).await.unwrap_err();

        assert!(err.contains("Re-read the affected file"), "unexpected error: {err}");
    }

    #[test]
    fn scan_folder_context_returns_none_when_neither_file_exists() {
        let dir = temp_project("context_none");
        assert_eq!(scan_folder_context(&dir), None);
    }

    #[test]
    fn scan_folder_context_reads_memory_md_alone() {
        let dir = temp_project("context_memory_only");
        fs::write(dir.join("MEMORY.md"), "The build command is `bun run build`.").unwrap();

        let context = scan_folder_context(&dir).expect("MEMORY.md should be picked up");
        assert!(context.contains("## MEMORY.md"));
        assert!(context.contains("bun run build"));
        assert!(!context.contains("AGENTS.md"));
    }

    #[test]
    fn scan_folder_context_reads_both_files_memory_first() {
        let dir = temp_project("context_both");
        fs::write(dir.join("MEMORY.md"), "memory fact").unwrap();
        fs::write(dir.join("AGENTS.md"), "agent instruction").unwrap();

        let context = scan_folder_context(&dir).expect("both files should be picked up");
        let memory_at = context.find("memory fact").expect("memory fact present");
        let agents_at = context.find("agent instruction").expect("agent instruction present");
        assert!(memory_at < agents_at, "MEMORY.md content should come before AGENTS.md content");
    }

    #[test]
    fn scan_folder_context_ignores_a_blank_file() {
        let dir = temp_project("context_blank");
        fs::write(dir.join("MEMORY.md"), "   \n\n  ").unwrap();
        assert_eq!(scan_folder_context(&dir), None);
    }

    #[test]
    fn scan_folder_context_truncates_oversized_content() {
        let dir = temp_project("context_oversized");
        fs::write(dir.join("MEMORY.md"), "x".repeat(MAX_FOLDER_CONTEXT_BYTES * 3)).unwrap();

        let context = scan_folder_context(&dir).expect("oversized file should still be picked up");
        assert!(context.len() < MAX_FOLDER_CONTEXT_BYTES * 3);
        assert!(context.contains("[truncated]"));
    }
}
