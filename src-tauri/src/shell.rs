use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use tokio::process::Command;

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
/// what corrupts its quoting (see `extract_powershell_script`). Everything else keeps the
/// existing `cmd.exe /C` / `sh -lc` behavior unchanged.
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
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
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
}
