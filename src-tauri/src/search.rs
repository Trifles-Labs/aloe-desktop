use regex::RegexBuilder;
use serde_json::{json, Value};
use std::{fs, path::Path};
use walkdir::WalkDir;

use crate::config::MAX_SEARCH_RESULTS;
use crate::fs::{assert_granted, input_string};
use crate::models::{AgentConfig, SearchMatch};

pub fn should_skip_dir(path: &Path) -> bool {
    path.file_name()
        .map(|n| {
            matches!(
                n.to_string_lossy().to_lowercase().as_str(),
                ".git" | "node_modules" | "dist" | "build" | ".next" | "target" | ".cache"
            )
        })
        .unwrap_or(false)
}

fn build_regex(pattern: &str, case_sensitive: bool) -> Result<regex::Regex, String> {
    RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|e| format!("Invalid regex: {e}"))
}

// ── Simple keyword search (used by the AI agent) ──────────────────────────────

pub fn search_codebase(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let root = assert_granted(config, &input_string(input, "path")?)?;
    let query = input_string(input, "query")?;
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for entry in WalkDir::new(&root)
        .into_iter()
        .filter_entry(|e| !should_skip_dir(e.path()))
        .filter_map(Result::ok)
    {
        if results.len() >= MAX_SEARCH_RESULTS {
            break;
        }
        let path = entry.path();
        let path_text = path.to_string_lossy();
        if path_text.to_lowercase().contains(&query_lower) {
            results.push(json!({ "path": path_text, "match": "filename" }));
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        if let Ok(content) = fs::read_to_string(path) {
            if let Some((line_num, line)) = content
                .lines()
                .enumerate()
                .find(|(_, l)| l.to_lowercase().contains(&query_lower))
            {
                results.push(json!({
                    "path": path_text,
                    "match": "content",
                    "line": line_num + 1,
                    "preview": line.trim().chars().take(240).collect::<String>(),
                }));
            }
        }
    }

    Ok(json!({ "path": root.to_string_lossy(), "query": query, "results": results }))
}

// ── Regex content search (used by the desktop UI) ────────────────────────────

pub fn search_content(
    config: &AgentConfig,
    path: &str,
    pattern: &str,
    case_sensitive: bool,
) -> Result<Vec<SearchMatch>, String> {
    let root = assert_granted(config, path)?;
    let re = build_regex(pattern, case_sensitive)?;
    let mut results = Vec::new();

    'file: for entry in WalkDir::new(&root)
        .into_iter()
        .filter_entry(|e| !should_skip_dir(e.path()))
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() || results.len() >= MAX_SEARCH_RESULTS {
            continue;
        }
        let Ok(content) = fs::read_to_string(entry.path()) else {
            continue; // skip binary files
        };
        let path_str = entry.path().to_string_lossy().to_string();
        let mut per_file = 0usize;
        for (idx, line) in content.lines().enumerate() {
            if re.is_match(line) {
                results.push(SearchMatch {
                    path: path_str.clone(),
                    match_type: "content".to_string(),
                    line: Some((idx + 1) as u32),
                    preview: Some(line.trim().chars().take(200).collect()),
                });
                per_file += 1;
                if per_file >= 8 || results.len() >= MAX_SEARCH_RESULTS {
                    continue 'file;
                }
            }
        }
    }

    Ok(results)
}

// ── Regex filename search (used by the desktop UI) ───────────────────────────

pub fn search_files(
    config: &AgentConfig,
    path: &str,
    pattern: &str,
    case_sensitive: bool,
) -> Result<Vec<SearchMatch>, String> {
    let root = assert_granted(config, path)?;
    let re = build_regex(pattern, case_sensitive)?;
    let mut results = Vec::new();

    for entry in WalkDir::new(&root)
        .into_iter()
        .filter_entry(|e| !should_skip_dir(e.path()))
        .filter_map(Result::ok)
    {
        if results.len() >= MAX_SEARCH_RESULTS {
            break;
        }
        if re.is_match(&entry.file_name().to_string_lossy()) {
            results.push(SearchMatch {
                path: entry.path().to_string_lossy().to_string(),
                match_type: if entry.file_type().is_dir() { "directory" } else { "file" }.to_string(),
                line: None,
                preview: None,
            });
        }
    }

    Ok(results)
}
