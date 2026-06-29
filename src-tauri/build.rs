fn main() {
    println!("cargo:rerun-if-env-changed=ALOE_BACKEND_URL");
    println!("cargo:rerun-if-changed=../.env");
    if let Some(api_url) = desktop_env_value("ALOE_BACKEND_URL") {
        println!("cargo:rustc-env=ALOE_BACKEND_URL={api_url}");
    }
    tauri_build::build();
}

fn desktop_env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| read_desktop_dotenv(key))
}

fn read_desktop_dotenv(key: &str) -> Option<String> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(".env");
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        if name.trim() == key {
            return Some(unquote_env_value(value.trim()).to_string());
        }
    }
    None
}

fn unquote_env_value(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|inner| inner.strip_suffix('\'')))
        .unwrap_or(value)
}
