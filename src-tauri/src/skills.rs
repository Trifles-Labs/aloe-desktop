//! Scripts from skill bundles (the backend's skills/skill_bundle.ts and
//! tools/skills/run_skill_script.ts). A `run_skill_script` job carries the bundle's whole file list
//! with hashes. The files are downloaded into `skills/<skill id>-<bundle hash>/` under the app's
//! config directory, each checked against its hash, and the script runs from there with the
//! interpreter its extension names, in a granted folder.
//!
//! A new hash means a new folder, so a bundle that changes never alters a script mid-run. The first
//! run of each version of a bundle always asks the user, whatever the trust mode (see `execute_job`);
//! approving it records the hash in `AgentConfig::trusted_skill_bundles`, and later runs of the same
//! version follow the normal command rules.
//!
//! A job with `local: true` runs a script from a skill in ~/.agents/skills instead (local_skills.rs):
//! nothing is downloaded, and the trust key is the skill's folder, pinned to a hash of its files.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::{process::Stdio, time::Duration};
use tokio::process::Command;

use crate::config::{config_path, COMMAND_TIMEOUT_SECONDS};
use crate::fs::{assert_granted, truncate_text};
use crate::models::AgentConfig;
use crate::shell::hide_command_window;

/// The backend's limits on a bundle (SKILL_BUNDLE_MAX_FILES, SKILL_BUNDLE_MAX_UNPACKED_BYTES).
const MAX_BUNDLE_FILES: usize = 200;
const MAX_BUNDLE_BYTES: u64 = 50_000_000;
const MAX_ARGS: usize = 50;
/// Written last into a finished download, so a half-downloaded folder is never used.
const BUNDLE_MARKER: &str = ".aloe-bundle";

struct BundleFile {
    path: String,
    size: u64,
    sha256: String,
}

struct SkillRun {
    skill_id: String,
    skill_name: String,
    bundle_hash: String,
    files: Vec<BundleFile>,
    script: String,
    args: Vec<String>,
    cwd: String,
}

fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// A bundle path as a relative path of plain names only: no root, no drive, no `.` or `..`.
pub(crate) fn safe_relative_path(raw: &str) -> Option<PathBuf> {
    if raw.is_empty() || raw.contains('\\') || raw.contains(':') {
        return None;
    }
    let path = Path::new(raw);
    if path.components().all(|component| matches!(component, Component::Normal(_))) {
        Some(path.to_path_buf())
    } else {
        None
    }
}

/// The same hash the backend's computeBundleHash makes: every `path\0sha256\n`, sorted by path.
fn bundle_hash(files: &[BundleFile]) -> String {
    let mut lines: Vec<(&str, &str)> = files.iter().map(|file| (file.path.as_str(), file.sha256.as_str())).collect();
    lines.sort_by(|a, b| a.0.cmp(b.0));
    let mut hasher = Sha256::new();
    for (path, sha) in lines {
        hasher.update(path.as_bytes());
        hasher.update([0u8]);
        hasher.update(sha.as_bytes());
        hasher.update(b"\n");
    }
    hex(&hasher.finalize())
}

fn text(input: &Value, key: &str) -> String {
    input.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn parse_run(input: &Value) -> Result<SkillRun, String> {
    let skill_id = text(input, "skillId");
    let bundle_hash_value = text(input, "bundleHash").to_ascii_lowercase();
    if !is_hex(&skill_id, 24) || !is_hex(&bundle_hash_value, 64) {
        return Err("The skill job is missing its skill id or bundle hash.".to_string());
    }

    let raw_files = input.get("files").and_then(Value::as_array).ok_or("The skill job has no file list.")?;
    if raw_files.is_empty() || raw_files.len() > MAX_BUNDLE_FILES {
        return Err("The skill's file list is empty or too long.".to_string());
    }
    let mut files = Vec::with_capacity(raw_files.len());
    let mut total: u64 = 0;
    for raw in raw_files {
        let path = text(raw, "path");
        let sha256 = text(raw, "sha256").to_ascii_lowercase();
        let size = raw.get("size").and_then(Value::as_u64).ok_or("A skill file has no size.")?;
        if safe_relative_path(&path).is_none() || !is_hex(&sha256, 64) {
            return Err(format!("The skill file \"{path}\" has an unsafe path or a bad hash."));
        }
        total += size;
        files.push(BundleFile { path, size, sha256 });
    }
    if total > MAX_BUNDLE_BYTES {
        return Err("The skill's files are too large.".to_string());
    }
    // The hash the trust check keys on must describe exactly these files.
    if bundle_hash(&files) != bundle_hash_value {
        return Err("The skill's file list does not match its bundle hash.".to_string());
    }

    let script = text(input, "script");
    if !files.iter().any(|file| file.path == script) {
        return Err(format!("\"{script}\" is not one of the skill's files."));
    }
    let args: Vec<String> = match input.get("args") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.iter().map(|item| item.as_str().map(str::to_string).ok_or("Script arguments must be strings.")).collect::<Result<_, _>>()?,
        Some(_) => return Err("Script arguments must be a list of strings.".to_string()),
    };
    if args.len() > MAX_ARGS {
        return Err("Too many script arguments.".to_string());
    }

    Ok(SkillRun {
        skill_id,
        skill_name: text(input, "skillName"),
        bundle_hash: bundle_hash_value,
        files,
        script,
        args,
        cwd: text(input, "cwd"),
    })
}

fn is_local(input: &Value) -> bool {
    input.get("local").and_then(Value::as_bool).unwrap_or(false)
}

/// The trust entry a run would need: a key for the skill and the hash of its current files.
fn trust_entry(input: &Value) -> Option<(String, String)> {
    if is_local(input) {
        let skill = crate::local_skills::find(&text(input, "skillName"))?;
        let hash = crate::local_skills::content_hash(&skill.dir)?;
        return Some((format!("local:{}", skill.dir.to_string_lossy().to_lowercase()), hash));
    }
    let skill_id = text(input, "skillId");
    let hash = text(input, "bundleHash").to_ascii_lowercase();
    (is_hex(&skill_id, 24) && is_hex(&hash, 64)).then_some((skill_id, hash))
}

pub fn is_trusted(config: &AgentConfig, input: &Value) -> bool {
    trust_entry(input).is_some_and(|(key, hash)| config.trusted_skill_bundles.get(&key) == Some(&hash))
}

/// Called when the user approves a run: later runs of this exact version need no first-run ask.
/// One entry per skill, so approving a new version replaces the old one.
pub fn trust(config: &mut AgentConfig, input: &Value) {
    if let Some((key, hash)) = trust_entry(input) {
        config.trusted_skill_bundles.insert(key, hash);
    }
}

/// A local skill script's source, for the approval card in the approvals queue (a bundled
/// script's source travels on the job from the backend instead).
pub fn local_script_preview(input: &Value) -> Option<String> {
    if !is_local(input) {
        return None;
    }
    let skill = crate::local_skills::find(&text(input, "skillName"))?;
    let source = std::fs::read_to_string(skill.dir.join(safe_relative_path(&text(input, "script"))?)).ok()?;
    let cut: String = source.chars().take(4_000).collect();
    Some(if cut.len() < source.len() { format!("{cut}
…") } else { cut })
}

/// What the approval prompt shows and voice reads out. The script's source is on the approval card
/// in the approvals queue (the backend's approval_queue.ts), not here: this text is spoken.
pub fn approval_reason(input: &Value, first_run: bool) -> String {
    let script = text(input, "script");
    let name = text(input, "skillName");
    if first_run {
        format!(
            "Aloe wants to run {script} from your skill \"{name}\". This version of the skill hasn't run on \
             this computer before, so it needs your OK whatever your command setting is. Read the script \
             on the approval card first."
        )
    } else {
        format!("Aloe wants to run {script} from your skill \"{name}\".")
    }
}

/// The script and its arguments as one line, for the approval prompt. Not run as such: the script
/// is started directly, without a shell.
pub fn command_line(input: &Value) -> String {
    let mut parts = vec![text(input, "script")];
    if let Some(args) = input.get("args").and_then(Value::as_array) {
        for arg in args.iter().filter_map(Value::as_str) {
            let plain = !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:=@,+".contains(c));
            parts.push(if plain { arg.to_string() } else { format!("{arg:?}") });
        }
    }
    parts.join(" ")
}

fn skills_root() -> Result<PathBuf, String> {
    Ok(config_path()?.parent().ok_or("Config path has no parent.")?.join("skills"))
}

/// Downloads the bundle unless a finished copy of this exact version is already here.
async fn ensure_bundle(client: &reqwest::Client, config: &AgentConfig, job_id: &str, run: &SkillRun) -> Result<PathBuf, String> {
    let root = skills_root()?;
    let name = format!("{}-{}", run.skill_id, &run.bundle_hash[..16]);
    let dir = root.join(&name);
    if std::fs::read_to_string(dir.join(BUNDLE_MARKER)).map(|hash| hash.trim() == run.bundle_hash).unwrap_or(false) {
        return Ok(dir);
    }

    let credential = config.credential.as_ref().ok_or("Aloe Desktop is not signed in.")?;
    let partial = root.join(format!(".{name}.{}.partial", uuid::Uuid::new_v4().simple()));
    let result = async {
        for file in &run.files {
            let response = client
                .get(format!("{}/api/agent/jobs/{job_id}/skill-files/{}", config.api_url, file.sha256))
                .bearer_auth(credential)
                .timeout(Duration::from_secs(120))
                .send()
                .await
                .map_err(|e| format!("Could not download {}: {e}", file.path))?;
            if !response.status().is_success() {
                return Err(format!("Could not download {} (HTTP {}).", file.path, response.status()));
            }
            let bytes = response.bytes().await.map_err(|e| format!("Could not download {}: {e}", file.path))?;
            if bytes.len() as u64 != file.size || sha256_hex(&bytes) != file.sha256 {
                return Err(format!("{} did not match its hash, so it was not saved.", file.path));
            }
            let target = partial.join(safe_relative_path(&file.path).ok_or("Unsafe skill file path.")?);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&target, &bytes).map_err(|e| e.to_string())?;
        }
        std::fs::write(partial.join(BUNDLE_MARKER), &run.bundle_hash).map_err(|e| e.to_string())?;
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        std::fs::rename(&partial, &dir).map_err(|e| e.to_string())
    }
    .await;

    if result.is_err() {
        let _ = std::fs::remove_dir_all(&partial);
    }
    match result {
        Ok(()) => Ok(dir),
        // Another run of the same bundle finished the download first.
        Err(_) if std::fs::read_to_string(dir.join(BUNDLE_MARKER)).map(|hash| hash.trim() == run.bundle_hash).unwrap_or(false) => Ok(dir),
        Err(e) => Err(e),
    }
}

/// The program that runs a script, and the arguments that go before the script's path. Mirrors
/// SKILL_SCRIPT_EXTENSIONS in the backend's run_skill_script.ts.
fn interpreter_for(script: &str) -> Result<(&'static str, Vec<&'static str>), String> {
    let extension = Path::new(script).extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let windows = cfg!(target_os = "windows");
    Ok(match extension.as_str() {
        "py" => (if windows { "python" } else { "python3" }, vec![]),
        "js" | "mjs" | "cjs" => ("node", vec![]),
        "ts" => ("bun", vec![]),
        "sh" => ("bash", vec![]),
        "ps1" => (if windows { "powershell" } else { "pwsh" }, vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]),
        _ => return Err(format!("Aloe Desktop doesn't know how to run {script}.")),
    })
}

pub async fn run_skill_script(client: &reqwest::Client, config: &AgentConfig, job_id: &str, input: &Value) -> Result<Value, String> {
    if is_local(input) {
        return run_local_skill_script(config, input).await;
    }
    let run = parse_run(input)?;
    let cwd = assert_granted(config, &run.cwd)?;
    interpreter_for(&run.script)?;
    let dir = ensure_bundle(client, config, job_id, &run).await?;

    // Checked again right before running: the copy on disk is what the user approved.
    let script_path = dir.join(safe_relative_path(&run.script).ok_or("Unsafe script path.")?);
    let expected = run.files.iter().find(|file| file.path == run.script).map(|file| file.sha256.as_str()).unwrap_or_default();
    let on_disk = std::fs::read(&script_path).map_err(|e| format!("Could not read {}: {e}", run.script))?;
    if sha256_hex(&on_disk) != expected {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!("{} changed on disk since it was downloaded, so it did not run. Try again to download it fresh.", run.script));
    }

    run_script(&run.skill_name, &run.script, &script_path, &dir, &run.args, &cwd).await
}

/// A script from a skill in ~/.agents/skills, run in place from the skill's own folder.
async fn run_local_skill_script(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let name = text(input, "skillName");
    let skill = crate::local_skills::find(&name).ok_or_else(|| format!("No skill named \"{name}\" on this computer."))?;
    let script = text(input, "script").replace('\\', "/");
    let script_path = skill.dir.join(safe_relative_path(&script).ok_or("That is not a path inside the skill.")?);
    if !script_path.is_file() {
        return Err(format!("\"{script}\" is not a file in the skill \"{}\".", skill.name));
    }
    let cwd = assert_granted(config, &text(input, "cwd"))?;
    let args: Vec<String> = match input.get("args") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.iter().map(|item| item.as_str().map(str::to_string).ok_or("Script arguments must be strings.")).collect::<Result<_, _>>()?,
        Some(_) => return Err("Script arguments must be a list of strings.".to_string()),
    };
    if args.len() > MAX_ARGS {
        return Err("Too many script arguments.".to_string());
    }
    run_script(&skill.name, &script, &script_path, &skill.dir, &args, &cwd).await
}

async fn run_script(skill_name: &str, script: &str, script_path: &Path, skill_dir: &Path, args: &[String], cwd: &Path) -> Result<Value, String> {
    let (program, leading_args) = interpreter_for(script)?;
    let mut process = Command::new(program);
    process
        .args(&leading_args)
        .arg(script_path)
        .args(args)
        .current_dir(cwd)
        .env("ALOE_SKILL_DIR", skill_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    hide_command_window(&mut process);

    let output = tokio::time::timeout(Duration::from_secs(COMMAND_TIMEOUT_SECONDS), process.output())
        .await
        .map_err(|_| format!("{script} timed out after {COMMAND_TIMEOUT_SECONDS} seconds."))?
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("{program} is not installed or not on PATH, so {script} can't run. Tell the user what to install.")
            } else {
                e.to_string()
            }
        })?;

    Ok(json!({
        "skill": skill_name,
        "script": script,
        "cwd": cwd.to_string_lossy(),
        "exitCode": output.status.code(),
        "stdout": truncate_text(String::from_utf8_lossy(&output.stdout).to_string()),
        "stderr": truncate_text(String::from_utf8_lossy(&output.stderr).to_string()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, sha: char) -> BundleFile {
        BundleFile { path: path.to_string(), size: 1, sha256: sha.to_string().repeat(64) }
    }

    #[test]
    fn bundle_hash_matches_the_backend() {
        // computeBundleHash([{ path: "a.py", sha256: "1"x64 }, { path: "b.py", sha256: "2"x64 }]) in
        // the backend's skill_bundle.ts. Order in the list must not matter.
        let forward = bundle_hash(&[file("a.py", '1'), file("b.py", '2')]);
        let backward = bundle_hash(&[file("b.py", '2'), file("a.py", '1')]);
        assert_eq!(forward, backward);
        let expected = sha256_hex(format!("a.py\0{}\nb.py\0{}\n", "1".repeat(64), "2".repeat(64)).as_bytes());
        assert_eq!(forward, expected);
    }

    #[test]
    fn only_plain_relative_paths_are_safe() {
        assert!(safe_relative_path("scripts/fill.py").is_some());
        for bad in ["", "/etc/passwd", "../x.py", "scripts/../../x.py", "C:/x.py", "scripts\\x.py", "./x.py"] {
            assert!(safe_relative_path(bad).is_none(), "should refuse {bad}");
        }
    }

    #[test]
    fn the_job_must_match_its_hash_and_name_a_bundled_script() {
        let files = vec![file("scripts/a.py", '1')];
        let hash = bundle_hash(&files);
        let input = json!({
            "skillId": "0123456789abcdef01234567",
            "bundleHash": hash,
            "files": [{ "path": "scripts/a.py", "size": 1, "sha256": "1".repeat(64) }],
            "script": "scripts/a.py",
            "args": ["--x"],
            "cwd": "C:/work",
        });
        assert!(parse_run(&input).is_ok());

        let mut wrong_hash = input.clone();
        wrong_hash["bundleHash"] = json!("f".repeat(64));
        assert!(parse_run(&wrong_hash).is_err());

        let mut other_script = input.clone();
        other_script["script"] = json!("scripts/b.py");
        assert!(parse_run(&other_script).is_err());
    }

    #[test]
    fn trust_is_per_version() {
        let mut config = AgentConfig::default();
        let input = json!({ "skillId": "0123456789abcdef01234567", "bundleHash": "a".repeat(64) });
        assert!(!is_trusted(&config, &input));
        trust(&mut config, &input);
        assert!(is_trusted(&config, &input));
        let changed = json!({ "skillId": "0123456789abcdef01234567", "bundleHash": "b".repeat(64) });
        assert!(!is_trusted(&config, &changed));
    }
}
