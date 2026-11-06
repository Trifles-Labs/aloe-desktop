use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Grants are stored canonicalized, which on Windows means `\\?\C:\...`. cmd.exe can't use that
/// form as its working directory (os error 267), so processes get the plain form. Grant checks
/// keep comparing canonical paths; this is only for handing a directory to a process.
pub fn shell_cwd(path: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let raw = path.to_string_lossy();
        if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = raw.strip_prefix(r"\\?\") {
            if rest.as_bytes().get(1) == Some(&b':') {
                return PathBuf::from(rest);
            }
        }
    }
    path.to_path_buf()
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(target_os = "windows")]
pub fn hide_command_window(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
pub fn hide_command_window(_command: &mut Command) {}

// Splits on whitespace, but a double-quoted run (quotes stripped) counts as one token — enough
// to reliably pull the script body out of `powershell -Command "...`, without needing a full
// shell-argument parser.
fn split_shell_like(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in input.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

// `powershell -Command "<script>"` run through `cmd.exe /C` gets double-parsed: cmd.exe's own
// quote handling can strip/rearrange the embedded double quotes before PowerShell ever sees the
// string, which makes PowerShell treat `"$content = ..."` as a double-quoted *string literal*
// (interpolating the not-yet-assigned `$content` to empty) instead of literal code to execute —
// the command reports exit code 0 while silently doing nothing. Detect this shape so the caller
// can invoke powershell.exe directly instead of nesting it inside cmd.exe.
#[cfg(target_os = "windows")]
fn extract_powershell_script(command: &str) -> Option<String> {
    let tokens = split_shell_like(command.trim());
    let first = tokens.first()?.to_lowercase();
    if first != "powershell" && first != "powershell.exe" && first != "pwsh" && first != "pwsh.exe" {
        return None;
    }
    let flag_index = tokens.iter().position(|t| {
        let lower = t.to_lowercase();
        lower == "-command" || lower == "-c"
    })?;
    let script = tokens.get(flag_index + 1)?.trim();
    if script.is_empty() {
        None
    } else {
        Some(script.to_string())
    }
}

#[cfg(target_os = "windows")]
fn encoded_powershell_command(script: &str) -> String {
    // -EncodedCommand takes base64 of UTF-16LE — sidesteps quoting entirely since it's opaque
    // bytes, not text PowerShell (or an intermediate cmd.exe layer) has to re-parse.
    let utf16le: Vec<u8> = script.encode_utf16().flat_map(|unit| unit.to_le_bytes()).collect();
    BASE64.encode(utf16le)
}

/// Builds the process used to run an arbitrary user/model-supplied shell command string.
/// On Windows, a `powershell -Command "..."` invocation is routed directly through
/// powershell.exe with `-EncodedCommand` instead of being nested inside `cmd.exe /C`, which is
/// what corrupts its quoting (see `extract_powershell_script`). Everything else runs through
/// `cmd.exe /S /C "<command>"` / `sh -lc`.
pub fn build_shell_command(command: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        if let Some(script) = extract_powershell_script(command) {
            let mut cmd = Command::new("powershell");
            cmd.arg("-NoProfile")
                .arg("-NonInteractive")
                .arg("-EncodedCommand")
                .arg(encoded_powershell_command(&script));
            return cmd;
        }
        // `.arg(command)` would quote it with MSVC rules (`"` becomes `\"`), which cmd.exe doesn't
        // understand: `"%EDGE%"` arrived as `'"%EDGE%"'`. With /S, cmd.exe strips exactly the
        // outer pair of quotes and runs the rest verbatim.
        let mut cmd = Command::new("cmd");
        cmd.raw_arg(format!("/S /C \"{command}\""));
        cmd
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-lc").arg(command);
        cmd
    }
}

#[cfg(test)]
#[cfg(target_os = "windows")]
mod tests {
    use super::*;

    #[test]
    fn extracts_powershell_command_script() {
        let command = r#"powershell -Command "$content = Get-Content 'a.txt' -Raw; Set-Content 'a.txt' $content""#;
        let script = extract_powershell_script(command).expect("should detect powershell -Command");
        assert!(script.starts_with("$content = Get-Content"));
        assert!(script.contains("Set-Content 'a.txt' $content"));
    }

    #[test]
    fn ignores_non_powershell_commands() {
        assert_eq!(extract_powershell_script("npm run build"), None);
        assert_eq!(extract_powershell_script("dir"), None);
    }

    #[test]
    fn round_trips_through_utf16_base64() {
        let script = "$x = 1; Write-Output $x";
        let encoded = encoded_powershell_command(script);
        let decoded_bytes = BASE64.decode(encoded).unwrap();
        let utf16: Vec<u16> = decoded_bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(String::from_utf16(&utf16).unwrap(), script);
    }

    #[test]
    fn shell_cwd_strips_extended_prefix() {
        assert_eq!(shell_cwd(Path::new(r"\\?\C:\Users\x\GitHub")), PathBuf::from(r"C:\Users\x\GitHub"));
        assert_eq!(shell_cwd(Path::new(r"\\?\UNC\server\share\dir")), PathBuf::from(r"\\server\share\dir"));
        assert_eq!(shell_cwd(Path::new(r"C:\plain")), PathBuf::from(r"C:\plain"));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aloe_shell_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    async fn run(command: &str, cwd: &Path) -> (Option<i32>, String, String) {
        let output = build_shell_command(command).current_dir(shell_cwd(cwd)).output().await.unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        )
    }

    #[tokio::test]
    async fn runs_in_canonical_granted_folder() {
        let dir = temp_dir("cwd");
        assert!(dir.to_string_lossy().starts_with(r"\\?\"));
        let (code, stdout, stderr) = run("cd", &dir).await;
        assert_eq!(code, Some(0), "stderr: {stderr}");
        assert_eq!(stdout.trim(), shell_cwd(&dir).to_string_lossy());
    }

    #[tokio::test]
    async fn passes_switches_and_quotes_through_unchanged() {
        let dir = temp_dir("quotes");
        std::fs::write(dir.join("marker file.txt"), "x").unwrap();

        let (code, stdout, stderr) = run("cmd /c dir /b", &dir).await;
        assert_eq!(code, Some(0), "stderr: {stderr}");
        assert_eq!(stdout.trim(), "marker file.txt");

        let (code, stdout, stderr) = run(r#"set "NAME=marker file.txt" && call type "%NAME%""#, &dir).await;
        assert_eq!(code, Some(0), "stderr: {stderr}");
        assert_eq!(stdout.trim(), "x");

        let quoted_exe = r#""C:\Windows\System32\cmd.exe" /c echo ok"#;
        let (code, stdout, stderr) = run(quoted_exe, &dir).await;
        assert_eq!(code, Some(0), "stderr: {stderr}");
        assert_eq!(stdout.trim(), "ok");
    }
}
