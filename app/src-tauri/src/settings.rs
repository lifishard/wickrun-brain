//! Reading and changing Claude Code's settings.json.
//!
//! A port of the tested logic in `scripts/wickrun-brain.mjs` (same backup file
//! names, same managed keys, same wickrunAI takeover detection), so the CLI and
//! the app can be used on the same machine and each can undo the other's change.
//!
//! SPDX-License-Identifier: MIT

use serde::Serialize;
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const BACKUP_SUFFIX: &str = ".before-wickrun-brain";
pub const ABSENT_SUFFIX: &str = ".before-wickrun-brain.absent";
/// Backup the wickrunAI desktop app keeps next to settings.json. Read-only for us.
const DESKTOP_BACKUP_SUFFIX: &str = ".before-wickrun";

pub const MANAGED_KEYS: [&str; 9] = [
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
];
const MODEL_KEYS: [&str; 6] = [
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
];

/// Error with a stable code the UI translates; `detail` never contains a key.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AppError {
    pub code: String,
    pub detail: String,
}
impl AppError {
    pub fn new(code: &str, detail: impl Into<String>) -> Self {
        AppError { code: code.into(), detail: detail.into() }
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}
pub type Result<T> = std::result::Result<T, AppError>;

/* ------------------------------------------------------------------ paths */

/// Where settings.json lives, following Claude Code's rules (CLAUDE_CONFIG_DIR, else ~/.claude).
pub fn settings_path(get: &dyn Fn(&str) -> Option<String>, home: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = get("CLAUDE_CONFIG_DIR").filter(|s| !s.is_empty()) {
        let p = PathBuf::from(&dir);
        if !p.is_absolute() {
            return Err(AppError::new("config_dir_relative", "CLAUDE_CONFIG_DIR must be an absolute path"));
        }
        return Ok(p.join("settings.json"));
    }
    let home = home.ok_or_else(|| AppError::new("no_home", "cannot find the home folder"))?;
    Ok(home.join(".claude").join("settings.json"))
}

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/* --------------------------------------------------------------- file io */

fn temp_path_for(file: &Path) -> PathBuf {
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let rnd: u64 = rand::random();
    file.with_file_name(format!(".{}.{}.{:012x}.tmp", name, std::process::id(), rnd & 0xffff_ffff_ffff))
}

fn write_temp(file: &Path, data: &[u8]) -> std::io::Result<PathBuf> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = temp_path_for(file);
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(tmp)
}

fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    // Windows can briefly lock a file that an editor or antivirus has open.
    let mut i = 0;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if i < 8 && matches!(e.kind(), std::io::ErrorKind::PermissionDenied) => {
                std::thread::sleep(std::time::Duration::from_millis(25 * (i + 1)));
                i += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Atomic replace: temp file in the same folder, then rename over the target.
pub fn write_atomic(file: &Path, data: &[u8]) -> Result<()> {
    let tmp = write_temp(file, data).map_err(|e| io_err(file, e))?;
    if let Err(e) = rename_with_retry(&tmp, file) {
        let _ = fs::remove_file(&tmp);
        return Err(io_err(file, e));
    }
    Ok(())
}

/// Create a file that must not exist yet. Returns false (writing nothing) if it exists.
pub fn create_exclusive(file: &Path, data: &[u8]) -> Result<bool> {
    if file.exists() {
        return Ok(false);
    }
    let tmp = write_temp(file, data).map_err(|e| io_err(file, e))?;
    let r = match fs::hard_link(&tmp, file) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(_) => {
            if file.exists() {
                Ok(false)
            } else {
                rename_with_retry(&tmp, file).map(|_| true).map_err(|e| io_err(file, e))
            }
        }
    };
    let _ = fs::remove_file(&tmp);
    r
}

fn io_err(file: &Path, e: std::io::Error) -> AppError {
    AppError::new("io", format!("{}: {}", file.display(), e.kind()))
}

pub struct Settings {
    pub file: PathBuf,
    pub exists: bool,
    pub raw: Vec<u8>,
    pub data: Map<String, Value>,
    crlf: bool,
    bom: bool,
    trailing_newline: bool,
}

pub fn read_settings(file: &Path) -> Result<Settings> {
    let raw = match fs::read(file) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings { file: file.into(), exists: false, raw: vec![], data: Map::new(), crlf: false, bom: false, trailing_newline: true })
        }
        Err(e) => return Err(io_err(file, e)),
    };
    let mut text = String::from_utf8(raw.clone()).map_err(|_| AppError::new("settings_invalid", "settings.json is not UTF-8"))?;
    let bom = text.starts_with('\u{feff}');
    if bom {
        text = text['\u{feff}'.len_utf8()..].to_string();
    }
    let v: Value = serde_json::from_str(&text).map_err(|_| AppError::new("settings_invalid", "settings.json is not valid JSON"))?;
    let data = match v {
        Value::Object(m) => m,
        _ => return Err(AppError::new("settings_invalid", "settings.json must contain a JSON object")),
    };
    if let Some(env) = data.get("env") {
        if !env.is_object() {
            return Err(AppError::new("settings_invalid", "\"env\" in settings.json is not an object"));
        }
    }
    Ok(Settings { file: file.into(), exists: true, crlf: text.contains("\r\n"), trailing_newline: text.ends_with('\n'), raw, data, bom })
}

pub fn serialize_settings(s: &Settings, data: &Map<String, Value>) -> Vec<u8> {
    let mut out = serde_json::to_string_pretty(&Value::Object(data.clone())).unwrap_or_default();
    if s.trailing_newline {
        out.push('\n');
    }
    if s.crlf {
        out = out.replace('\n', "\r\n");
    }
    let mut bytes = Vec::new();
    if s.bom {
        bytes.extend_from_slice("\u{feff}".as_bytes());
    }
    bytes.extend_from_slice(out.as_bytes());
    bytes
}

/* ---------------------------------------------------- state & detection */

/// Never show a secret: at most the last 4 characters, and only for long keys.
pub fn mask_secret(value: &str) -> String {
    let n = value.chars().count();
    if n == 0 {
        return String::new();
    }
    if n >= 20 {
        let tail: String = value.chars().skip(n - 4).collect();
        format!("…{}", tail)
    } else {
        "set".into()
    }
}

pub fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]") || host.starts_with("127.")
}

fn env_obj(data: &Map<String, Value>) -> Map<String, Value> {
    data.get("env").and_then(|v| v.as_object()).cloned().unwrap_or_default()
}
fn env_str<'a>(env: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    env.get(k).and_then(|v| v.as_str())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Desktop {
    pub detected: bool,
    pub kind: Option<String>,
}

/// Same rules as the CLI: the wickrunAI desktop app leaves no marker, so recognise its shape.
pub fn detect_desktop(data: &Map<String, Value>, file: &Path) -> Desktop {
    let env = env_obj(data);
    let host = env_str(&env, "ANTHROPIC_BASE_URL").and_then(|u| url::Url::parse(u).ok()).and_then(|u| u.host_str().map(|h| h.to_string()));
    if let Some(h) = host {
        let model = env_str(&env, "ANTHROPIC_MODEL");
        if is_loopback(&h)
            && env_str(&env, "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC") == Some("1")
            && model.is_some()
            && env_str(&env, "CLAUDE_CODE_SUBAGENT_MODEL") == model
        {
            return Desktop { detected: true, kind: Some("route".into()) };
        }
    }
    if data.get("forceLoginMethod").and_then(|v| v.as_str()) == Some("claudeai") && with_suffix(file, DESKTOP_BACKUP_SUFFIX).exists() {
        return Desktop { detected: true, kind: Some("subscription".into()) };
    }
    Desktop { detected: false, kind: None }
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub settings_file: String,
    pub settings_exists: bool,
    /// "official" | "custom" | "wickrunai-desktop" | "broken"
    pub mode: String,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub credential_var: Option<String>,
    pub credential_masked: Option<String>,
    pub keys_set: Vec<String>,
    pub backup_exists: bool,
    pub desktop: Desktop,
}

pub fn status(file: &Path) -> Status {
    let backup_exists = with_suffix(file, BACKUP_SUFFIX).exists() || with_suffix(file, ABSENT_SUFFIX).exists();
    let s = match read_settings(file) {
        Ok(s) => s,
        Err(_) => {
            return Status {
                settings_file: file.display().to_string(),
                settings_exists: true,
                mode: "broken".into(),
                base_url: None,
                model: None,
                credential_var: None,
                credential_masked: None,
                keys_set: vec![],
                backup_exists,
                desktop: Desktop { detected: false, kind: None },
            }
        }
    };
    let env = env_obj(&s.data);
    let keys_set: Vec<String> = MANAGED_KEYS.iter().filter(|k| env.contains_key(**k)).map(|k| k.to_string()).collect();
    let desktop = detect_desktop(&s.data, file);
    let mode = if desktop.detected { "wickrunai-desktop" } else if keys_set.is_empty() { "official" } else { "custom" };
    let cred = ["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"].iter().find(|k| env.contains_key(**k)).map(|k| k.to_string());
    let masked = cred.as_ref().map(|k| {
        let v = env.get(k).map(|v| v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string())).unwrap_or_default();
        mask_secret(&v)
    });
    Status {
        settings_file: file.display().to_string(),
        settings_exists: s.exists,
        mode: mode.into(),
        base_url: env_str(&env, "ANTHROPIC_BASE_URL").map(String::from),
        model: env_str(&env, "ANTHROPIC_MODEL").map(String::from),
        credential_var: cred,
        credential_masked: masked,
        keys_set,
        backup_exists,
        desktop,
    }
}

/* ------------------------------------------------------------ validation */

pub fn validate_base_url(v: &str) -> Result<()> {
    let u = url::Url::parse(v).map_err(|_| AppError::new("bad_url", "not a valid URL"))?;
    if u.scheme() != "https" && u.scheme() != "http" {
        return Err(AppError::new("bad_url", "must start with https:// or http://"));
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err(AppError::new("bad_url", "must not contain a user name or password"));
    }
    if v.contains('?') || v.contains('#') || u.host_str().is_none() {
        return Err(AppError::new("bad_url", "must not contain ? or #, and needs a host"));
    }
    Ok(())
}

pub fn validate_model(v: &str) -> Result<()> {
    let ok = !v.is_empty() && v.len() <= 200 && v.chars().all(|c| c.is_ascii_alphanumeric() || "_./:@+-[]".contains(c));
    if ok { Ok(()) } else { Err(AppError::new("bad_model", "model names use letters, digits and . / : @ + - _ [ ]")) }
}

pub fn validate_key(v: &str) -> Result<()> {
    if v.is_empty() {
        return Err(AppError::new("no_key", "the key is empty"));
    }
    if v.len() > 4096 || v.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(AppError::new("bad_key", "the key is too long or contains spaces or control characters"));
    }
    Ok(())
}

/* -------------------------------------------------------------- changes */

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Auth {
    /// ANTHROPIC_AUTH_TOKEN (Authorization: Bearer)
    Bearer,
    /// ANTHROPIC_API_KEY (x-api-key)
    ApiKey,
}

pub struct Target<'a> {
    pub base_url: &'a str,
    pub auth: Auth,
    /// Always required: never point Claude Code at another address without its own key,
    /// or it would send the user's claude.ai login there.
    pub key: &'a str,
    pub model: &'a str,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Changed {
    pub changed: bool,
    pub created_backup: bool,
}

fn ensure_backup(s: &Settings) -> Result<bool> {
    let b = with_suffix(&s.file, BACKUP_SUFFIX);
    let a = with_suffix(&s.file, ABSENT_SUFFIX);
    if b.exists() || a.exists() {
        return Ok(false);
    }
    if s.exists { create_exclusive(&b, &s.raw) } else { create_exclusive(&a, &[]) }
}

fn without_managed(data: &Map<String, Value>) -> Map<String, Value> {
    let mut next = data.clone();
    if let Some(Value::Object(env)) = data.get("env") {
        let mut env = env.clone();
        for k in MANAGED_KEYS {
            env.remove(k);
        }
        if env.is_empty() {
            next.remove("env");
        } else {
            next.insert("env".into(), Value::Object(env));
        }
    }
    next
}

fn apply_data(s: &Settings, next: &Map<String, Value>) -> Result<Changed> {
    let bytes = serialize_settings(s, next);
    if s.exists && bytes == s.raw {
        return Ok(Changed { changed: false, created_backup: false });
    }
    let created_backup = ensure_backup(s)?;
    write_atomic(&s.file, &bytes)?;
    Ok(Changed { changed: true, created_backup })
}

fn refuse_if_desktop(s: &Settings) -> Result<()> {
    if detect_desktop(&s.data, &s.file).detected {
        return Err(AppError::new("desktop_takeover", "Claude Code is managed by the wickrunAI desktop app"));
    }
    Ok(())
}

pub fn apply(file: &Path, t: &Target) -> Result<Changed> {
    validate_base_url(t.base_url)?;
    validate_model(t.model)?;
    validate_key(t.key)?;
    let s = read_settings(file)?;
    refuse_if_desktop(&s)?;
    let mut next = without_managed(&s.data);
    let mut env = env_obj(&next);
    env.insert("ANTHROPIC_BASE_URL".into(), Value::String(t.base_url.into()));
    let var = match t.auth { Auth::Bearer => "ANTHROPIC_AUTH_TOKEN", Auth::ApiKey => "ANTHROPIC_API_KEY" };
    env.insert(var.into(), Value::String(t.key.into()));
    for k in MODEL_KEYS {
        env.insert(k.into(), Value::String(t.model.into()));
    }
    next.insert("env".into(), Value::Object(env));
    apply_data(&s, &next)
}

/// Remove every override: back to the official login. Keeps the backup.
pub fn official(file: &Path) -> Result<Changed> {
    let s = read_settings(file)?;
    refuse_if_desktop(&s)?;
    let env = env_obj(&s.data);
    if !MANAGED_KEYS.iter().any(|k| env.contains_key(*k)) {
        return Ok(Changed { changed: false, created_backup: false });
    }
    apply_data(&s, &without_managed(&s.data))
}

#[derive(Debug, Serialize, PartialEq)]
pub enum Restored {
    NothingToRestore,
    Identical,
    RemovedAgain,
}

/// Put settings.json back byte-for-byte as it was before the first change, then delete the backup.
pub fn restore(file: &Path) -> Result<Restored> {
    let b = with_suffix(file, BACKUP_SUFFIX);
    let a = with_suffix(file, ABSENT_SUFFIX);
    match (b.exists(), a.exists()) {
        (false, false) => return Ok(Restored::NothingToRestore),
        (true, true) => return Err(AppError::new("two_backups", "both backup markers exist; remove the wrong one")),
        _ => {}
    }
    if let Ok(cur) = read_settings(file) {
        if cur.exists {
            refuse_if_desktop(&cur)?;
        }
    }
    if b.exists() {
        let original = fs::read(&b).map_err(|e| io_err(&b, e))?;
        write_atomic(file, &original)?;
        fs::remove_file(&b).map_err(|e| io_err(&b, e))?;
        Ok(Restored::Identical)
    } else {
        match fs::remove_file(file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(file, e)),
        }
        fs::remove_file(&a).map_err(|e| io_err(&a, e))?;
        Ok(Restored::RemovedAgain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    const SECRET: &str = "sk-wbtest-Q7xR2mN9pL4vK8sJ3hG6fD1aZ5cB0eYw-tail";
    const ORIGINAL: &str = "{\n    \"theme\": \"dark\",\n    \"env\": { \"MY_OWN_VAR\": \"keep me\", \"ANTHROPIC_MODEL\": \"user-model\" },\n    \"unicode\": \"中文 é\"\n}";

    fn sha(b: &[u8]) -> Vec<u8> { Sha256::digest(b).to_vec() }
    fn tgt() -> Target<'static> { Target { base_url: "https://llm.example.com", auth: Auth::Bearer, key: SECRET, model: "gw-model-1" } }
    fn setup(content: Option<&str>) -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::Builder::new().prefix("wb gui ").tempdir().unwrap();
        let f = d.path().join("claude config").join("settings.json");
        if let Some(c) = content {
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(&f, c).unwrap();
        }
        (d, f)
    }

    #[test]
    fn apply_then_restore_is_byte_identical() {
        let (_d, f) = setup(Some(ORIGINAL));
        let r = apply(&f, &tgt()).unwrap();
        assert!(r.changed && r.created_backup);
        assert_eq!(sha(&fs::read(with_suffix(&f, BACKUP_SUFFIX)).unwrap()), sha(ORIGINAL.as_bytes()));
        let v: Value = serde_json::from_slice(&fs::read(&f).unwrap()).unwrap();
        assert_eq!(v["env"]["MY_OWN_VAR"], "keep me");
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], SECRET);
        assert_eq!(v["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "gw-model-1");
        assert_eq!(v["theme"], "dark");
        assert_eq!(restore(&f).unwrap(), Restored::Identical);
        assert_eq!(fs::read(&f).unwrap(), ORIGINAL.as_bytes());
        assert!(!with_suffix(&f, BACKUP_SUFFIX).exists());
    }

    #[test]
    fn second_change_keeps_true_original_and_existing_backup_is_never_overwritten() {
        let (_d, f) = setup(Some(ORIGINAL));
        apply(&f, &tgt()).unwrap();
        apply(&f, &Target { model: "other", ..tgt() }).unwrap();
        assert_eq!(fs::read(with_suffix(&f, BACKUP_SUFFIX)).unwrap(), ORIGINAL.as_bytes());
        let (_d2, f2) = setup(Some("{}"));
        fs::write(with_suffix(&f2, BACKUP_SUFFIX), "EARLIER").unwrap();
        apply(&f2, &tgt()).unwrap();
        assert_eq!(fs::read(with_suffix(&f2, BACKUP_SUFFIX)).unwrap(), b"EARLIER");
    }

    #[test]
    fn missing_settings_is_removed_again_on_restore() {
        let (_d, f) = setup(None);
        apply(&f, &tgt()).unwrap();
        assert!(f.exists() && with_suffix(&f, ABSENT_SUFFIX).exists());
        assert_eq!(restore(&f).unwrap(), Restored::RemovedAgain);
        assert!(!f.exists() && !with_suffix(&f, ABSENT_SUFFIX).exists());
        assert_eq!(restore(&f).unwrap(), Restored::NothingToRestore);
    }

    #[test]
    fn noop_does_not_create_backup_and_keeps_bytes() {
        let (_d, f) = setup(Some("{\"theme\":\"dark\"}\n"));
        assert!(!official(&f).unwrap().changed);
        assert!(!with_suffix(&f, BACKUP_SUFFIX).exists());
    }

    #[test]
    fn keeps_crlf_bom_and_no_trailing_newline() {
        let (_d, f) = setup(None);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, "\u{feff}{\r\n  \"a\": 1\r\n}").unwrap();
        apply(&f, &tgt()).unwrap();
        let out = String::from_utf8(fs::read(&f).unwrap()).unwrap();
        assert!(out.starts_with('\u{feff}') && out.contains("\r\n") && !out.ends_with('\n'));
        official(&f).unwrap();
        let out = String::from_utf8(fs::read(&f).unwrap()).unwrap();
        assert_eq!(out, "\u{feff}{\r\n  \"a\": 1\r\n}");
    }

    #[test]
    fn invalid_json_changes_nothing() {
        let (_d, f) = setup(Some("{ not json"));
        assert_eq!(apply(&f, &tgt()).unwrap_err().code, "settings_invalid");
        assert_eq!(fs::read(&f).unwrap(), b"{ not json");
        assert!(!with_suffix(&f, BACKUP_SUFFIX).exists());
        let (_d, f) = setup(Some("{\"env\": []}"));
        assert_eq!(apply(&f, &tgt()).unwrap_err().code, "settings_invalid");
    }

    #[test]
    fn a_key_is_always_required() {
        let (_d, f) = setup(Some("{}"));
        assert_eq!(apply(&f, &Target { key: "", ..tgt() }).unwrap_err().code, "no_key");
        assert_eq!(fs::read(&f).unwrap(), b"{}");
        assert_eq!(apply(&f, &Target { key: "a b", ..tgt() }).unwrap_err().code, "bad_key");
        assert_eq!(apply(&f, &Target { base_url: "https://u:p@x.com", ..tgt() }).unwrap_err().code, "bad_url");
        assert_eq!(apply(&f, &Target { base_url: "file:///x", ..tgt() }).unwrap_err().code, "bad_url");
    }

    #[test]
    fn api_key_auth_uses_x_api_key_var_and_clears_the_other() {
        let (_d, f) = setup(Some("{\"env\":{\"ANTHROPIC_AUTH_TOKEN\":\"old\"}}"));
        apply(&f, &Target { auth: Auth::ApiKey, ..tgt() }).unwrap();
        let v: Value = serde_json::from_slice(&fs::read(&f).unwrap()).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_API_KEY"], SECRET);
        assert!(v["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    }

    #[test]
    fn refuses_when_wickrunai_desktop_took_over() {
        let route = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:45123","CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC":"1","ANTHROPIC_MODEL":"m","CLAUDE_CODE_SUBAGENT_MODEL":"m","ANTHROPIC_AUTH_TOKEN":"t"}}"#;
        let (_d, f) = setup(Some(route));
        assert_eq!(status(&f).mode, "wickrunai-desktop");
        assert_eq!(apply(&f, &tgt()).unwrap_err().code, "desktop_takeover");
        assert_eq!(official(&f).unwrap_err().code, "desktop_takeover");
        fs::write(with_suffix(&f, BACKUP_SUFFIX), "{}").unwrap();
        assert_eq!(restore(&f).unwrap_err().code, "desktop_takeover");
        assert_eq!(fs::read(&f).unwrap(), route.as_bytes());
        let (_d, f) = setup(Some(r#"{"forceLoginMethod":"claudeai"}"#));
        assert_eq!(status(&f).mode, "official");
        fs::write(with_suffix(&f, DESKTOP_BACKUP_SUFFIX), "{}").unwrap();
        assert_eq!(status(&f).mode, "wickrunai-desktop");
    }

    #[test]
    fn our_own_proxy_shape_is_not_mistaken_for_wickrunai() {
        let (_d, f) = setup(Some("{}"));
        apply(&f, &Target { base_url: "http://127.0.0.1:41234", ..tgt() }).unwrap();
        assert_eq!(status(&f).mode, "custom");
    }

    #[test]
    fn status_masks_the_key() {
        let (_d, f) = setup(Some("{}"));
        apply(&f, &tgt()).unwrap();
        let st = status(&f);
        let json = serde_json::to_string(&st).unwrap();
        assert!(!json.contains(SECRET) && !json.contains(&SECRET[..12]));
        assert_eq!(st.credential_masked.as_deref(), Some("…tail"));
    }

    #[test]
    fn masking() {
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("short-key"), "set");
        assert_eq!(mask_secret(SECRET), "…tail");
        assert_eq!(mask_secret("中文中文中文中文中文中文中文中文中文中文"), "…中文中文");
    }

    #[test]
    fn restore_works_when_current_file_is_broken() {
        let (_d, f) = setup(Some(ORIGINAL));
        apply(&f, &tgt()).unwrap();
        fs::write(&f, "garbage").unwrap();
        assert_eq!(restore(&f).unwrap(), Restored::Identical);
        assert_eq!(fs::read(&f).unwrap(), ORIGINAL.as_bytes());
    }

    #[test]
    fn interoperates_with_cli_backup_names() {
        assert_eq!(BACKUP_SUFFIX, ".before-wickrun-brain");
        assert_eq!(ABSENT_SUFFIX, ".before-wickrun-brain.absent");
    }

    #[test]
    fn settings_path_rules() {
        let none = |_: &str| None;
        assert_eq!(settings_path(&none, Some("/h".into())).unwrap(), PathBuf::from("/h/.claude/settings.json"));
        let rel = |k: &str| if k == "CLAUDE_CONFIG_DIR" { Some("rel".to_string()) } else { None };
        assert!(settings_path(&rel, Some("/h".into())).is_err());
    }
}
