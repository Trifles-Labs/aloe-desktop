//! The jobs behind Aloe Code's Claude Code-style tools: glob, grep and whole-file write.
//!
//! Chat's `search_local_codebase` mixes name and content matching and stops at the first matching
//! line of each file, which is fine for finding a document but not for working on code. These walk
//! the project the way ripgrep does (the `ignore` crate honours .gitignore, .ignore and hidden-file
//! rules), so dependency and build trees drop out without a hard-coded skip list.

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use regex::RegexBuilder;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

use crate::fs::{assert_granted, assert_safe_write, input_string};
use crate::models::AgentConfig;

const MAX_GLOB_RESULTS: usize = 200;
const DEFAULT_GREP_LIMIT: usize = 200;
const MAX_GREP_LIMIT: usize = 1_000;
/// Files above this are skipped by grep: minified bundles and data dumps, never what was meant.
const MAX_GREP_FILE_BYTES: u64 = 2_000_000;
const MAX_LINE_CHARS: usize = 300;

fn walker(root: &Path) -> ignore::Walk {
    WalkBuilder::new(root)
        // Dotfiles such as .github/ and .eslintrc are real project files; .gitignore still applies.
        .hidden(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build()
}

fn build_globset(patterns: &str) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    // "*.{ts,tsx}" is one pattern; a bare comma list ("*.ts,*.tsx") is accepted too.
    let split: Vec<&str> = if patterns.contains('{') { vec![patterns] } else { patterns.split(',').collect() };
    for pattern in split.into_iter().map(str::trim).filter(|p| !p.is_empty()) {
        // A pattern without a slash matches at any depth, like ripgrep's --glob.
        let pattern = if pattern.contains('/') { pattern.to_string() } else { format!("**/{pattern}") };
        builder.add(Glob::new(&pattern).map_err(|e| format!("Invalid glob \"{pattern}\": {e}"))?);
    }
    builder.build().map_err(|e| e.to_string())
}

/// Credential files whose contents must never leave this machine through a search. The backend
/// filters these too (SECRET_PATH_PATTERNS in local_agent.ts); this keeps the bytes off the wire.
fn is_secret_file(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    name == ".env"
        || name.starts_with(".env.")
        || [".pem", ".key", ".pfx", ".p12", ".keystore", ".jks", ".ppk"].iter().any(|ext| name.ends_with(ext))
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.contains("secret")
        || matches!(name.as_str(), ".npmrc" | ".netrc" | ".pgpass")
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

fn clip_line(line: &str) -> String {
    let trimmed = line.trim_end_matches('\r');
    if trimmed.chars().count() <= MAX_LINE_CHARS {
        return trimmed.to_string();
    }
    format!("{}…", trimmed.chars().take(MAX_LINE_CHARS).collect::<String>())
}

/// `{ path, pattern }` → files under `path` matching the glob, newest first.
pub fn glob_files(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let root = assert_granted(config, &input_string(input, "path")?)?;
    let pattern = input_string(input, "pattern")?;
    let set = build_globset(&pattern)?;

    let mut matches: Vec<(SystemTime, String)> = walker(&root)
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|entry| {
            let rel = relative(&root, entry.path());
            if !set.is_match(&rel) {
                return None;
            }
            let modified = entry.metadata().ok().and_then(|m| m.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH);
            Some((modified, rel))
        })
        .collect();
    matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    let total = matches.len();
    let files: Vec<String> = matches.into_iter().take(MAX_GLOB_RESULTS).map(|(_, rel)| rel).collect();
    Ok(json!({ "root": root.to_string_lossy(), "files": files, "total": total, "truncated": total > MAX_GLOB_RESULTS }))
}

/// `{ path, pattern, glob?, caseInsensitive?, outputMode?, context?, headLimit? }` → ripgrep-style matches.
/// `path` may be a single file. outputMode is "files_with_matches" (default), "content" or "count".
pub fn grep_files(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let root = assert_granted(config, &input_string(input, "path")?)?;
    let pattern = input_string(input, "pattern")?;
    let case_insensitive = input.get("caseInsensitive").and_then(Value::as_bool).unwrap_or(false);
    let regex = RegexBuilder::new(&pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|e| format!("Invalid regex: {e}"))?;
    let filter = match input.get("glob").and_then(Value::as_str).filter(|g| !g.trim().is_empty()) {
        Some(glob) => Some(build_globset(glob)?),
        None => None,
    };
    let mode = input.get("outputMode").and_then(Value::as_str).unwrap_or("files_with_matches");
    let context = input.get("context").and_then(Value::as_u64).unwrap_or(0).min(10) as usize;
    let limit = input.get("headLimit").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(DEFAULT_GREP_LIMIT).clamp(1, MAX_GREP_LIMIT);

    let base = if root.is_file() { root.parent().map(Path::to_path_buf).unwrap_or_else(|| root.clone()) } else { root.clone() };
    let mut lines_out: Vec<String> = Vec::new();
    let mut files_matched = 0usize;
    let mut truncated = false;

    'files: for entry in walker(&root).filter_map(Result::ok) {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) || is_secret_file(entry.path()) {
            continue;
        }
        let rel = relative(&base, entry.path());
        if filter.as_ref().map(|set| !set.is_match(&rel)).unwrap_or(false) {
            continue;
        }
        if entry.metadata().map(|m| m.len() > MAX_GREP_FILE_BYTES).unwrap_or(true) {
            continue;
        }
        let Ok(content) = fs::read_to_string(entry.path()) else { continue }; // binary or not UTF-8
        let lines: Vec<&str> = content.lines().collect();
        let hits: Vec<usize> = lines.iter().enumerate().filter(|(_, line)| regex.is_match(line)).map(|(i, _)| i).collect();
        if hits.is_empty() {
            continue;
        }
        files_matched += 1;

        match mode {
            "count" => lines_out.push(format!("{rel}:{}", hits.len())),
            "content" => {
                let mut last_printed: Option<usize> = None;
                for &hit in &hits {
                    let from = hit.saturating_sub(context);
                    let to = (hit + context).min(lines.len() - 1);
                    if let Some(last) = last_printed {
                        if context > 0 && from > last + 1 {
                            lines_out.push("--".to_string());
                        }
                    }
                    let start = last_printed.map(|last| (last + 1).max(from)).unwrap_or(from);
                    for i in start..=to {
                        let sep = if i == hit || hits.binary_search(&i).is_ok() { ':' } else { '-' };
                        lines_out.push(format!("{rel}{sep}{}{sep}{}", i + 1, clip_line(lines[i])));
                        if lines_out.len() >= limit {
                            truncated = true;
                            break 'files;
                        }
                    }
                    last_printed = Some(to);
                }
            }
            _ => lines_out.push(rel),
        }
        if lines_out.len() >= limit {
            truncated = true;
            break;
        }
    }

    Ok(json!({
        "root": base.to_string_lossy(),
        "mode": mode,
        "output": lines_out.join("\n"),
        "filesMatched": files_matched,
        "truncated": truncated,
    }))
}

/// Resolves a path that may not exist yet, nor its parents, and checks it against the grants.
/// `assert_granted` canonicalizes the parent, which fails when a whole new directory is being
/// written; here the nearest existing ancestor is canonicalized and the rest re-attached, with `..`
/// rejected so the remainder cannot climb back out of the grant.
fn granted_new_path(config: &AgentConfig, raw: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("Paths with \"..\" are not allowed: {raw}"));
    }
    let mut existing = path.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        let name = existing.file_name().ok_or_else(|| format!("Invalid path: {raw}"))?.to_os_string();
        rest.push(name);
        existing = existing.parent().ok_or_else(|| format!("Invalid path: {raw}"))?.to_path_buf();
    }
    let mut target = assert_granted(config, &existing.to_string_lossy())?;
    for name in rest.into_iter().rev() {
        target.push(name);
    }
    Ok(target)
}

/// `{ path, content }` → writes the whole file, creating it and any missing parent folders.
pub fn write_text_file(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let path = granted_new_path(config, &input_string(input, "path")?)?;
    assert_safe_write(&path)?;
    let content = input.get("content").and_then(Value::as_str).ok_or("content is required.")?;
    if path.is_dir() {
        return Err(format!("{} is a directory.", path.display()));
    }
    let existed = path.exists();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Keep a CRLF file CRLF: the model writes LF, and flipping every line ending turns a small
    // change into a whole-file diff.
    let crlf = existed && fs::read_to_string(&path).map(|old| old.contains("\r\n")).unwrap_or(false);
    let body = if crlf && !content.contains('\r') { content.replace('\n', "\r\n") } else { content.to_string() };
    fs::write(&path, body.as_bytes()).map_err(|e| e.to_string())?;
    Ok(json!({ "path": path.to_string_lossy(), "created": !existed, "bytes": body.len() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_globs_match_at_any_depth() {
        let set = build_globset("*.ts").unwrap();
        assert!(set.is_match("a.ts"));
        assert!(set.is_match("src/lib/a.ts"));
        assert!(!set.is_match("src/a.tsx"));
    }

    #[test]
    fn brace_and_comma_lists() {
        assert!(build_globset("*.{ts,tsx}").unwrap().is_match("src/a.tsx"));
        let set = build_globset("*.rs, *.toml").unwrap();
        assert!(set.is_match("Cargo.toml") && set.is_match("src/main.rs"));
    }

    #[test]
    fn rooted_globs_respect_their_directory() {
        let set = build_globset("src/**/*.rs").unwrap();
        assert!(set.is_match("src/a/b.rs"));
        assert!(!set.is_match("tests/b.rs"));
    }
}
