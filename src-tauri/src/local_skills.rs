//! Skills that live on this computer, in the folder other AI agents share (agentskills.io's
//! client guide): `~/.agents/skills/<name>/SKILL.md`. They belong to the whole app (chat, voice,
//! Aloe Code), not to any granted folder. Claude Code's (`~/.claude/skills`) and Codex's
//! (`~/.codex/skills`) personal skills use the same format and are read too, but never written.
//!
//! Only each skill's name and description leave the computer: they go to the backend over the
//! agent socket (`local_skills` message) so chat turns from this desktop can list them, and the
//! backend keeps them in memory only. The instructions and files are read here when Aloe loads a
//! skill (`read_local_skill`), and scripts run from the skill's own folder (skills.rs).
//!
//! Aloe can save and remove skills in ~/.agents/skills only. Other agents on this computer read
//! that folder too, so both always go through the approval queue (see `execute_job`).

use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use tokio_tungstenite::tungstenite::Message;

use crate::config::AppState;
use crate::skills::{safe_relative_path, sha256_hex};

const MAX_SKILLS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 1_024;
const MAX_LISTED_FILES: usize = 200;
const MAX_FILE_READ_BYTES: u64 = 1_000_000;
const FILE_TEXT_CHAR_CAP: usize = 60_000;
const MAX_WRITE_FILES: usize = 50;
const MAX_WRITE_BYTES: usize = 2_000_000;

#[derive(Debug, Clone)]
pub struct LocalSkill {
    pub name: String,
    pub description: String,
    pub dir: PathBuf,
    /// Whose folder it is in: "shared" (~/.agents/skills), "Claude Code" or "Codex".
    pub source: &'static str,
}

/// `~/.agents/skills`: the shared, user-level skills folder, and the only one Aloe writes to.
pub fn user_root() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".agents").join("skills"))
}

/// The folders skills are read from, in precedence order: the shared one first.
fn roots() -> Vec<(PathBuf, &'static str)> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    vec![
        (home.join(".agents").join("skills"), "shared"),
        (home.join(".claude").join("skills"), "Claude Code"),
        (home.join(".codex").join("skills"), "Codex"),
    ]
}

/// The text between the opening `---` and the closing `---`, and the body after it.
fn split_frontmatter(source: &str) -> Option<(String, String)> {
    let normalized = source.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let rest = normalized.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let front = rest[..end].to_string();
    let after = &rest[end + 4..];
    let body = after.split_once('\n').map(|(_, body)| body).unwrap_or("");
    Some((front, body.trim().to_string()))
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return value[1..value.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\").replace("\\n", "\n");
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].replace("''", "'");
    }
    value.to_string()
}

/// One top-level frontmatter field. Enough YAML for SKILL.md: plain, quoted, folded (`>`) and
/// literal (`|`) values, and plain values continued on indented lines.
pub fn frontmatter_field(front: &str, key: &str) -> Option<String> {
    let lines: Vec<&str> = front.lines().collect();
    let prefix = format!("{key}:");
    let start = lines.iter().position(|line| line.starts_with(&prefix))?;
    let first = lines[start][prefix.len()..].trim();
    let continuation: Vec<&str> = lines[start + 1..]
        .iter()
        .take_while(|line| line.trim().is_empty() || line.starts_with(' ') || line.starts_with('\t'))
        .map(|line| line.trim())
        .collect();
    let value = if first.starts_with('>') || first.starts_with('|') {
        let joiner = if first.starts_with('|') { "\n" } else { " " };
        continuation.join(joiner)
    } else if continuation.iter().any(|line| !line.is_empty()) && !first.starts_with('"') && !first.starts_with('\'') {
        std::iter::once(first).chain(continuation.iter().copied()).filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" ")
    } else {
        unquote(first)
    };
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn read_skill(dir: &Path, source: &'static str) -> Option<LocalSkill> {
    let contents = std::fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let (front, _) = split_frontmatter(&contents)?;
    let name = frontmatter_field(&front, "name").or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string()))?;
    let description: String = frontmatter_field(&front, "description")?.split_whitespace().collect::<Vec<_>>().join(" ");
    let description = description.chars().take(MAX_DESCRIPTION_CHARS).collect();
    Some(LocalSkill { name, description, dir: dir.to_path_buf(), source })
}

/// Every skill in the skill folders, by folder name; a skill whose name an earlier folder already
/// has is skipped.
pub fn scan() -> Vec<LocalSkill> {
    let mut skills: Vec<LocalSkill> = Vec::new();
    for (root, source) in roots() {
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        let mut dirs: Vec<PathBuf> = entries.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.is_dir()).collect();
        dirs.sort();
        for dir in dirs {
            if skills.len() >= MAX_SKILLS {
                return skills;
            }
            if let Some(skill) = read_skill(&dir, source) {
                if !skills.iter().any(|existing| existing.name.eq_ignore_ascii_case(&skill.name)) {
                    skills.push(skill);
                }
            }
        }
    }
    skills
}

/// What goes to the backend: names, descriptions and whose folder, nothing else.
pub fn index_message(skills: &[LocalSkill]) -> Value {
    json!({
        "type": "local_skills",
        "skills": skills.iter().map(|skill| json!({ "name": skill.name, "description": skill.description, "source": skill.source })).collect::<Vec<_>>(),
    })
}

/// Sends the current index to the backend now, rather than at the socket's next rescan.
pub fn push_index(state: &AppState, skills: &[LocalSkill]) {
    if let Some(sender) = state.outbound.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
        let _ = sender.send(Message::Text(index_message(skills).to_string().into()));
    }
}

/// One skill as the Skills settings pane lists it (aloe-ui's LocalSkillSummary).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSkillSummary {
    name: String,
    description: String,
    path: String,
    source: &'static str,
    files: usize,
    /// Aloe can remove it (it is in ~/.agents/skills).
    removable: bool,
}

fn summaries(skills: &[LocalSkill]) -> Vec<LocalSkillSummary> {
    let root = user_root();
    skills
        .iter()
        .map(|skill| LocalSkillSummary {
            name: skill.name.clone(),
            description: skill.description.clone(),
            path: skill.dir.to_string_lossy().to_string(),
            source: skill.source,
            files: list_files(&skill.dir).len() + 1,
            removable: root.as_ref().is_some_and(|root| skill.dir.starts_with(root)),
        })
        .collect()
}

#[tauri::command]
pub fn local_skills_list() -> Vec<LocalSkillSummary> {
    summaries(&scan())
}

/// Rescans the skill folders and sends the index to the backend straight away.
#[tauri::command]
pub fn local_skills_refresh(state: State<'_, AppState>) -> Vec<LocalSkillSummary> {
    let skills = scan();
    push_index(&state, &skills);
    summaries(&skills)
}

/// Opens ~/.agents/skills in the file manager, making it first if needed.
#[tauri::command]
pub fn local_skills_open_folder(app: AppHandle) -> Result<(), String> {
    let root = user_root().ok_or("Could not find the home folder.")?;
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    app.opener().open_path(root.to_string_lossy().to_string(), None::<String>).map_err(|e| e.to_string())
}

pub fn find(name: &str) -> Option<LocalSkill> {
    scan().into_iter().find(|skill| skill.name.eq_ignore_ascii_case(name.trim()))
}

/// Every file in a skill's folder except SKILL.md, as forward-slash relative paths.
pub fn list_files(dir: &Path) -> Vec<String> {
    let mut files: Vec<String> = walkdir::WalkDir::new(dir)
        .max_depth(6)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.path().strip_prefix(dir).ok().map(|rel| rel.to_string_lossy().replace('\\', "/")))
        .filter(|rel| rel != "SKILL.md" && !rel.split('/').any(|part| part.starts_with('.')))
        .collect();
    files.sort();
    files.truncate(MAX_LISTED_FILES);
    files
}

/// A hash of every file in a local skill's folder, so a changed skill asks again before its
/// scripts run (the local counterpart of a bundle hash).
pub fn content_hash(dir: &Path) -> Option<String> {
    let mut lines = String::new();
    for rel in std::iter::once("SKILL.md".to_string()).chain(list_files(dir)) {
        let bytes = std::fs::read(dir.join(&rel)).ok()?;
        lines.push_str(&format!("{rel}\0{}\n", sha256_hex(&bytes)));
    }
    Some(sha256_hex(lines.as_bytes()))
}

fn text(input: &Value, key: &str) -> String {
    input.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn not_found(name: &str) -> String {
    let names: Vec<String> = scan().into_iter().map(|skill| skill.name).collect();
    format!("No skill named \"{name}\" on this computer. Its skills: {}.", if names.is_empty() { "none".to_string() } else { names.join(", ") })
}

/// `read_local_skill`: a skill's instructions and file list, or one of its files.
pub fn read_local_skill(input: &Value) -> Result<Value, String> {
    let name = text(input, "name");
    let skill = find(&name).ok_or_else(|| not_found(&name))?;
    let file = text(input, "file").trim().replace('\\', "/").trim_start_matches("./").to_string();

    if file.is_empty() {
        let source = std::fs::read_to_string(skill.dir.join("SKILL.md")).map_err(|e| e.to_string())?;
        let instructions = split_frontmatter(&source).map(|(_, body)| body).unwrap_or(source);
        return Ok(json!({
            "name": skill.name,
            "instructions": instructions,
            "files": list_files(&skill.dir),
            "location": skill.dir.to_string_lossy(),
        }));
    }

    let rel = safe_relative_path(&file).ok_or("That is not a path inside the skill.")?;
    let path = skill.dir.join(rel);
    let size = std::fs::metadata(&path).map_err(|_| format!("\"{file}\" is not a file in the skill \"{}\".", skill.name))?.len();
    if size > MAX_FILE_READ_BYTES {
        return Ok(json!({ "name": skill.name, "file": file, "sizeBytes": size, "note": "Too large to read as text." }));
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    match String::from_utf8(bytes) {
        Ok(content) if !content.contains('\0') => {
            let total = content.chars().count();
            if total > FILE_TEXT_CHAR_CAP {
                let cut: String = content.chars().take(FILE_TEXT_CHAR_CAP).collect();
                Ok(json!({ "name": skill.name, "file": file, "content": cut, "truncated": format!("Cut at {FILE_TEXT_CHAR_CAP} of {total} characters.") }))
            } else {
                Ok(json!({ "name": skill.name, "file": file, "content": content }))
            }
        }
        _ => Ok(json!({ "name": skill.name, "file": file, "sizeBytes": size, "binary": true, "note": "A binary file; a script from this skill can use it." })),
    }
}

/// The Agent Skills name rule: lowercase letters, digits and hyphens, at most 64 characters.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed: String = out.trim_matches('-').chars().take(64).collect();
    let trimmed = trimmed.trim_end_matches('-').to_string();
    if trimmed.is_empty() { "skill".to_string() } else { trimmed }
}

fn yaml_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " "))
}

pub fn skill_md(name: &str, description: &str, instructions: &str) -> String {
    format!("---\nname: {name}\ndescription: {}\n---\n\n{}\n", yaml_quote(description.trim()), instructions.trim())
}

/// `write_local_skill`: saves a skill folder under ~/.agents/skills. Runs only after the user
/// approved it in the approval queue.
pub fn write_local_skill(input: &Value) -> Result<Value, String> {
    let root = user_root().ok_or("Could not find the home folder.")?;
    let name = slug(&text(input, "name"));
    let description = text(input, "description");
    let instructions = text(input, "instructions");
    if description.trim().is_empty() || instructions.trim().is_empty() {
        return Err("A skill needs a description and instructions.".to_string());
    }
    let replace = input.get("replace").and_then(Value::as_bool).unwrap_or(false);
    let dir = root.join(&name);
    if dir.exists() && !replace {
        return Err(format!("A skill named \"{name}\" already exists in {}. Ask the user before replacing it, then save again with replace.", root.display()));
    }

    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut total = 0usize;
    for file in input.get("files").and_then(Value::as_array).cloned().unwrap_or_default() {
        let path = text(&file, "path").replace('\\', "/");
        let rel = safe_relative_path(&path).ok_or_else(|| format!("\"{path}\" is not a path a skill file can have."))?;
        if path.eq_ignore_ascii_case("SKILL.md") {
            return Err("SKILL.md is written from name, description and instructions; don't pass it as a file.".to_string());
        }
        let content = text(&file, "content");
        total += content.len();
        files.push((rel, content));
    }
    if files.len() > MAX_WRITE_FILES || total > MAX_WRITE_BYTES {
        return Err(format!("A saved skill can have at most {MAX_WRITE_FILES} files and {} MB.", MAX_WRITE_BYTES / 1_000_000));
    }

    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let partial = root.join(format!(".{name}.{}.partial", uuid::Uuid::new_v4().simple()));
    let result = (|| -> Result<(), String> {
        std::fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
        std::fs::write(partial.join("SKILL.md"), skill_md(&name, &description, &instructions)).map_err(|e| e.to_string())?;
        for (rel, content) in &files {
            let target = partial.join(rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(target, content).map_err(|e| e.to_string())?;
        }
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&partial, &dir).map_err(|e| e.to_string())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(e);
    }
    Ok(json!({ "saved": true, "name": name, "location": dir.to_string_lossy(), "files": files.len() + 1 }))
}

/// `remove_local_skill`: deletes a skill folder from ~/.agents/skills, after approval. Skills in
/// Claude Code's or Codex's folders belong to those tools and are never removed from here.
pub fn remove_local_skill(input: &Value) -> Result<Value, String> {
    let name = text(input, "name");
    let skill = find(&name).ok_or_else(|| not_found(&name))?;
    let root = user_root().ok_or("Could not find the home folder.")?;
    if !skill.dir.starts_with(&root) {
        return Err(format!("\"{}\" is a {} skill ({}). Aloe only removes skills in {}.", skill.name, skill.source, skill.dir.display(), root.display()));
    }
    let dir = skill.dir;
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(json!({ "removed": true, "name": name, "location": dir.to_string_lossy() }))
}

/// One line for the approval prompt and voice: what is about to be saved or removed, and where.
pub fn approval_summary(kind: &str, input: &Value) -> (String, String, String) {
    let root = user_root().map(|root| root.to_string_lossy().to_string()).unwrap_or_default();
    if kind == "remove_local_skill" {
        let name = text(input, "name");
        return (
            format!("Remove the skill {name}"),
            root,
            format!("Aloe wants to delete the skill \"{name}\" from your shared skills folder. Other AI tools on this computer use that folder too."),
        );
    }
    let name = slug(&text(input, "name"));
    let files = input.get("files").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
    let replacing = input.get("replace").and_then(Value::as_bool).unwrap_or(false);
    (
        format!("{} the skill {name}{}", if replacing { "Replace" } else { "Save" }, if files > 0 { format!(" with {files} extra file(s)") } else { String::new() }),
        root,
        format!("Aloe wants to {} the skill \"{name}\" in your shared skills folder, where other AI tools on this computer can use it too. Check its instructions on the approval card first.", if replacing { "replace" } else { "save" }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_plain_quoted_and_folded_fields() {
        let front = "name: pdf-tools\ndescription: >\n  Use when the user\n  works with PDFs.\nother: x";
        assert_eq!(frontmatter_field(front, "name").as_deref(), Some("pdf-tools"));
        assert_eq!(frontmatter_field(front, "description").as_deref(), Some("Use when the user works with PDFs."));
        assert_eq!(frontmatter_field("description: \"Say \\\"hi\\\"\"", "description").as_deref(), Some("Say \"hi\""));
        assert_eq!(frontmatter_field("description: 'It''s fine'", "description").as_deref(), Some("It's fine"));
        assert_eq!(frontmatter_field("description: one\n  two", "description").as_deref(), Some("one two"));
        assert_eq!(frontmatter_field("name: x", "description"), None);
    }

    #[test]
    fn written_skill_md_reads_back() {
        let source = skill_md("expense-report", "Use when filing \"expenses\".", "Step one.");
        let (front, body) = split_frontmatter(&source).unwrap();
        assert_eq!(frontmatter_field(&front, "name").as_deref(), Some("expense-report"));
        assert_eq!(frontmatter_field(&front, "description").as_deref(), Some("Use when filing \"expenses\"."));
        assert_eq!(body, "Step one.");
    }

    #[test]
    fn slugs_follow_the_spec() {
        assert_eq!(slug("Expense Report!"), "expense-report");
        assert_eq!(slug("  --  "), "skill");
        assert_eq!(slug(&"a".repeat(80)).len(), 64);
    }
}
