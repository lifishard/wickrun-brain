//! Finding every "brain" this machine can offer Claude Code.
//!
//! Everything here is read-only. Keys are kept in `KeyRef`s inside the app and only
//! a masked form ever leaves this module.
//!
//! SPDX-License-Identifier: MIT

use crate::settings::mask_secret;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How Claude Code would reach a source.
#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// Speaks the Anthropic Messages API: settings.json points straight at it.
    Direct,
    /// Only speaks Chat Completions: goes through the built-in translator.
    Translate,
    /// Can't be used from here.
    None,
}

#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Bearer,
    ApiKey,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum KeyRef {
    /// Process environment variable
    Env(String),
    /// A value parsed from a file (shell profile, auth.json, CLI profiles); re-read at use time
    File { path: PathBuf, name: String },
    /// A CC Switch provider (re-read from its database at use time)
    CcSwitch { id: String },
    /// Placeholder for servers that don't check keys (Ollama, LM Studio, open gateways)
    Placeholder(String),
    /// Pasted by the user into the app
    Pasted,
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub id: String,
    /// local | keys | ccswitch | gateway | wickrunai
    pub group: String,
    pub name: String,
    /// Where it came from, for "details": an env var name, a file path, a URL
    pub origin: String,
    /// reachable | needs_key | not_compatible | unreachable | managed_elsewhere
    pub status: String,
    pub route: Route,
    pub base_url: String,
    pub auth: AuthKind,
    pub key_masked: Option<String>,
    pub models: Vec<String>,
    pub default_model: Option<String>,
    /// Alternative endpoints (label, url), e.g. China / international
    pub endpoints: Vec<(String, String)>,
    /// i18n code for an extra note
    pub note: Option<String>,
    pub docs: Option<String>,
    #[serde(skip)]
    pub key: KeyRef,
    /// Base for /models listing (OpenAI-style), if any
    #[serde(skip)]
    pub list_base: Option<String>,
}

/* ---------------------------------------------------------- the catalog */

pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub env: &'static [&'static str],
    pub route: Route,
    pub base: &'static str,
    pub endpoints: &'static [(&'static str, &'static str)],
    pub auth: AuthKind,
    pub models: &'static [&'static str],
    /// OpenAI-style base for listing models, if the provider offers one
    pub list_base: Option<&'static str>,
    pub docs: &'static str,
}

/// Cloud providers and how Claude Code can reach them. Sources are in the README.
pub const PROVIDERS: &[Provider] = &[
    Provider { id: "anthropic", name: "Anthropic", env: &["ANTHROPIC_API_KEY"], route: Route::Direct, base: "https://api.anthropic.com", endpoints: &[], auth: AuthKind::ApiKey, models: &[], list_base: None, docs: "https://code.claude.com/docs/en/settings" },
    Provider { id: "deepseek", name: "DeepSeek", env: &["DEEPSEEK_API_KEY"], route: Route::Direct, base: "https://api.deepseek.com/anthropic", endpoints: &[], auth: AuthKind::ApiKey, models: &["deepseek-v4-pro", "deepseek-flash"], list_base: None, docs: "https://api-docs.deepseek.com/guides/anthropic_api" },
    Provider { id: "moonshot", name: "Kimi (Moonshot)", env: &["MOONSHOT_API_KEY", "KIMI_API_KEY"], route: Route::Direct, base: "https://api.moonshot.cn/anthropic", endpoints: &[("china", "https://api.moonshot.cn/anthropic"), ("global", "https://api.moonshot.ai/anthropic")], auth: AuthKind::Bearer, models: &["kimi-k3.1-pro", "kimi-k3"], list_base: None, docs: "https://platform.kimi.com/docs/guide/claude-code-kimi" },
    Provider { id: "zhipu", name: "Zhipu GLM", env: &["ZHIPUAI_API_KEY", "ZHIPU_API_KEY", "GLM_API_KEY", "BIGMODEL_API_KEY"], route: Route::Direct, base: "https://open.bigmodel.cn/api/anthropic", endpoints: &[], auth: AuthKind::Bearer, models: &["glm-5.3", "glm-5.2"], list_base: None, docs: "https://docs.bigmodel.cn/cn/guide/develop/claude/introduction" },
    Provider { id: "zai", name: "Z.ai (GLM)", env: &["ZAI_API_KEY", "Z_AI_API_KEY"], route: Route::Direct, base: "https://api.z.ai/api/anthropic", endpoints: &[], auth: AuthKind::Bearer, models: &["glm-5.3"], list_base: None, docs: "https://docs.z.ai/devpack/tool/claude" },
    Provider { id: "qwen", name: "Qwen (DashScope)", env: &["DASHSCOPE_API_KEY", "QWEN_API_KEY"], route: Route::Direct, base: "https://dashscope-intl.aliyuncs.com/apps/anthropic", endpoints: &[("international", "https://dashscope-intl.aliyuncs.com/apps/anthropic"), ("china", "https://dashscope.aliyuncs.com/apps/anthropic")], auth: AuthKind::Bearer, models: &["qwen3.7-max", "qwen3.6-flash"], list_base: None, docs: "https://www.alibabacloud.com/help/en/model-studio/claude-code" },
    Provider { id: "minimax", name: "MiniMax", env: &["MINIMAX_API_KEY"], route: Route::Direct, base: "https://api.minimax.io/anthropic", endpoints: &[("global", "https://api.minimax.io/anthropic"), ("china", "https://api.minimaxi.com/anthropic")], auth: AuthKind::Bearer, models: &["MiniMax-M3.1-Flash"], list_base: None, docs: "https://platform.minimax.io/docs/token-plan/claude-code" },
    Provider { id: "openrouter", name: "OpenRouter", env: &["OPENROUTER_API_KEY"], route: Route::Direct, base: "https://openrouter.ai/api", endpoints: &[], auth: AuthKind::Bearer, models: &[], list_base: Some("https://openrouter.ai/api/v1"), docs: "https://openrouter.ai/docs/guides/coding-agents/claude-code-integration" },
    Provider { id: "openai", name: "OpenAI", env: &["OPENAI_API_KEY"], route: Route::Translate, base: "https://api.openai.com/v1", endpoints: &[], auth: AuthKind::Bearer, models: &[], list_base: Some("https://api.openai.com/v1"), docs: "https://platform.openai.com/docs/api-reference/chat" },
    Provider { id: "gemini", name: "Google Gemini", env: &["GEMINI_API_KEY", "GOOGLE_API_KEY"], route: Route::Translate, base: "https://generativelanguage.googleapis.com/v1beta/openai", endpoints: &[], auth: AuthKind::Bearer, models: &[], list_base: Some("https://generativelanguage.googleapis.com/v1beta/openai"), docs: "https://ai.google.dev/gemini-api/docs/openai" },
    Provider { id: "groq", name: "Groq", env: &["GROQ_API_KEY"], route: Route::Translate, base: "https://api.groq.com/openai/v1", endpoints: &[], auth: AuthKind::Bearer, models: &[], list_base: Some("https://api.groq.com/openai/v1"), docs: "https://console.groq.com/docs/openai" },
];

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// Which provider a base URL belongs to (used to name CC Switch entries and the current brain).
pub fn provider_for_url(url: &str) -> Option<&'static Provider> {
    let host = url::Url::parse(url).ok()?.host_str()?.to_lowercase();
    PROVIDERS.iter().find(|p| {
        std::iter::once(p.base).chain(p.endpoints.iter().map(|e| e.1)).any(|b| url::Url::parse(b).ok().and_then(|u| u.host_str().map(|h| h == host)).unwrap_or(false))
    })
}

/* ------------------------------------------------------ key file parsing */

/// Parse `export NAME=value` / `NAME=value` / fish `set -x NAME value` lines.
/// Values that reference other variables or command output are skipped.
pub fn parse_shell_exports(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('#') {
            continue;
        }
        let (name, raw) = if let Some(rest) = l.strip_prefix("set ").filter(|_| l.contains(" -x ") || l.contains(" -gx ") || l.contains(" -Ux ")) {
            let parts: Vec<&str> = rest.split_whitespace().filter(|p| !p.starts_with('-')).collect();
            if parts.len() < 2 {
                continue;
            }
            (parts[0].to_string(), parts[1..].join(" "))
        } else {
            let l = l.strip_prefix("export ").unwrap_or(l);
            let l = l.strip_prefix("$env:").unwrap_or(l); // PowerShell profile
            let Some((n, v)) = l.split_once('=') else { continue };
            (n.trim().to_string(), v.trim().to_string())
        };
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let v = raw.split(" #").next().unwrap_or("").trim();
        let v = if (v.starts_with('"') && v.ends_with('"') && v.len() >= 2) || (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2) { &v[1..v.len() - 1] } else { v };
        if v.is_empty() || v.contains('$') || v.contains('`') || v.contains(char::is_whitespace) {
            continue;
        }
        out.push((name, v.to_string()));
    }
    out
}

/// Read one named value from a key file again at use time.
pub fn read_file_key(path: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    if path.extension().and_then(|e| e.to_str()) == Some("json") {
        let v: Value = serde_json::from_str(&text).ok()?;
        // wickrun-brain CLI profiles: "profiles.<name>.key"
        if let Some(p) = name.strip_prefix("profile:") {
            return v.get("profiles")?.get(p)?.get("key")?.as_str().map(String::from);
        }
        return v.get(name)?.as_str().map(String::from);
    }
    parse_shell_exports(&text).into_iter().rev().find(|(n, _)| n == name).map(|(_, v)| v)
}

pub fn shell_profile_files(home: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = [".zshrc", ".zprofile", ".zshenv", ".bashrc", ".bash_profile", ".profile", ".config/fish/config.fish"].iter().map(|f| home.join(f)).collect();
    v.push(home.join("Documents").join("PowerShell").join("Microsoft.PowerShell_profile.ps1"));
    v.push(home.join("Documents").join("WindowsPowerShell").join("Microsoft.PowerShell_profile.ps1"));
    v
}

fn make_key_source(p: &Provider, origin: String, key: KeyRef, value: &str) -> Source {
    let compatible = p.route != Route::None;
    Source {
        id: format!("key:{}:{}", p.id, origin),
        group: "keys".into(),
        name: p.name.into(),
        origin,
        status: if compatible { "reachable".into() } else { "not_compatible".into() },
        route: p.route,
        base_url: p.base.into(),
        auth: p.auth,
        key_masked: Some(mask_secret(value)),
        models: p.models.iter().map(|s| s.to_string()).collect(),
        default_model: p.models.first().map(|s| s.to_string()),
        endpoints: p.endpoints.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        note: if p.route == Route::Translate { Some("via_translator".into()) } else { None },
        docs: Some(p.docs.into()),
        key,
        list_base: p.list_base.map(String::from),
    }
}

/// API keys from the environment and common files. Each (provider, place) is listed once.
pub fn find_keys(get_env: &dyn Fn(&str) -> Option<String>, home: Option<&Path>, cli_profiles: Option<&Path>) -> Vec<Source> {
    let mut out = Vec::new();
    let mut seen_values: Vec<(String, String)> = Vec::new();
    let mut push = |out: &mut Vec<Source>, p: &Provider, origin: String, key: KeyRef, value: String| {
        if seen_values.iter().any(|(pid, v)| pid == p.id && *v == value) {
            return; // same key found in two places: list it once (env wins)
        }
        seen_values.push((p.id.into(), value.clone()));
        out.push(make_key_source(p, origin, key, &value));
    };
    for p in PROVIDERS {
        for var in p.env {
            if let Some(v) = get_env(var).filter(|v| !v.trim().is_empty()) {
                push(&mut out, p, format!("env:{}", var), KeyRef::Env(var.to_string()), v);
            }
        }
    }
    if let Some(home) = home {
        for f in shell_profile_files(home) {
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            for (name, value) in parse_shell_exports(&text) {
                if let Some(p) = PROVIDERS.iter().find(|p| p.env.contains(&name.as_str())) {
                    push(&mut out, p, format!("file:{}#{}", f.display(), name), KeyRef::File { path: f.clone(), name: name.clone() }, value);
                }
            }
        }
        let codex = home.join(".codex").join("auth.json");
        if let Some(v) = read_file_key(&codex, "OPENAI_API_KEY").filter(|v| !v.is_empty()) {
            push(&mut out, provider("openai").unwrap(), format!("file:{}#OPENAI_API_KEY", codex.display()), KeyRef::File { path: codex.clone(), name: "OPENAI_API_KEY".into() }, v);
        }
    }
    if let Some(pf) = cli_profiles {
        if let Ok(text) = std::fs::read_to_string(pf) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                if let Some(profiles) = v.get("profiles").and_then(|p| p.as_object()) {
                    for (name, prof) in profiles {
                        let (Some(base), Some(model)) = (prof.get("baseUrl").and_then(|b| b.as_str()), prof.get("model").and_then(|m| m.as_str())) else { continue };
                        let (key, value) = if let Some(env) = prof.get("keyEnv").and_then(|k| k.as_str()) {
                            (KeyRef::Env(env.into()), get_env(env).unwrap_or_default())
                        } else if let Some(k) = prof.get("key").and_then(|k| k.as_str()) {
                            (KeyRef::File { path: pf.into(), name: format!("profile:{}", name) }, k.to_string())
                        } else {
                            continue;
                        };
                        let auth = if prof.get("auth").and_then(|a| a.as_str()) == Some("x-api-key") { AuthKind::ApiKey } else { AuthKind::Bearer };
                        out.push(Source {
                            id: format!("cli:{}", name),
                            group: "keys".into(),
                            name: format!("{} ({})", name, provider_for_url(base).map(|p| p.name).unwrap_or("custom")),
                            origin: format!("file:{}#{}", pf.display(), name),
                            status: if value.is_empty() { "needs_key".into() } else { "reachable".into() },
                            route: Route::Direct,
                            base_url: base.into(),
                            auth,
                            key_masked: if value.is_empty() { None } else { Some(mask_secret(&value)) },
                            models: vec![model.into()],
                            default_model: Some(model.into()),
                            endpoints: vec![],
                            note: Some("from_cli".into()),
                            docs: None,
                            key,
                            list_base: None,
                        });
                    }
                }
            }
        }
    }
    out
}

/* ---------------------------------------------------------- CC Switch */

/// Where CC Switch keeps its data. Default `~/.cc-switch`; a user override lives in the
/// Tauri store `app_paths.json` (key `app_config_dir_override`) in CC Switch's app data dir.
pub fn ccswitch_dirs(home: &Path, data_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = data_dir {
        let store = d.join("com.ccswitch.desktop").join("app_paths.json");
        if let Ok(t) = std::fs::read_to_string(store) {
            if let Ok(j) = serde_json::from_str::<Value>(&t) {
                if let Some(p) = j.get("app_config_dir_override").and_then(|p| p.as_str()).map(str::trim).filter(|p| !p.is_empty()) {
                    let p = if let Some(rest) = p.strip_prefix("~/") { home.join(rest) } else { PathBuf::from(p) };
                    v.push(p);
                }
            }
        }
    }
    v.push(home.join(".cc-switch"));
    v
}

#[derive(Debug, Clone, PartialEq)]
pub struct CcProvider {
    pub id: String,
    pub name: String,
    pub base_url: Option<String>,
    pub key: Option<String>,
    pub auth: AuthKind,
    pub model: Option<String>,
    pub api_format: Option<String>,
    pub is_current: bool,
}

/// Parse CC Switch's `settings_config` (the Claude Code `settings.json` fragment it would write).
pub fn parse_cc_settings(id: &str, name: &str, settings_config: &Value, meta: &Value, is_current: bool) -> CcProvider {
    let env = settings_config.get("env").cloned().unwrap_or(Value::Null);
    let s = |k: &str| env.get(k).and_then(|v| v.as_str()).map(String::from).filter(|v| !v.is_empty());
    let (key, auth) = match (s("ANTHROPIC_AUTH_TOKEN"), s("ANTHROPIC_API_KEY")) {
        (Some(k), _) => (Some(k), AuthKind::Bearer),
        (None, Some(k)) => (Some(k), AuthKind::ApiKey),
        _ => (None, AuthKind::Bearer),
    };
    CcProvider {
        id: id.into(),
        name: name.into(),
        base_url: s("ANTHROPIC_BASE_URL"),
        key,
        auth,
        model: s("ANTHROPIC_MODEL").or_else(|| s("ANTHROPIC_DEFAULT_SONNET_MODEL")),
        api_format: meta.get("apiFormat").and_then(|v| v.as_str()).map(String::from),
        is_current,
    }
}

/// Read CC Switch providers for Claude Code. The database is copied to a temp folder
/// first and opened read-only there, so CC Switch's own file is never locked or changed.
pub fn read_ccswitch(dir: &Path) -> Option<Vec<CcProvider>> {
    let db = dir.join("cc-switch.db");
    if db.exists() {
        let tmp = tempdir_for_copy()?;
        let copy = tmp.join("cc.db");
        std::fs::copy(&db, &copy).ok()?;
        let wal = dir.join("cc-switch.db-wal");
        if wal.exists() {
            let _ = std::fs::copy(&wal, tmp.join("cc.db-wal"));
        }
        let res = (|| {
            let conn = rusqlite::Connection::open_with_flags(&copy, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
            let mut stmt = conn.prepare("SELECT id, name, settings_config, COALESCE(meta,'{}'), COALESCE(is_current,0) FROM providers WHERE app_type = 'claude' ORDER BY COALESCE(sort_index, 999999), name").ok()?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, bool>(4)?)))
                .ok()?;
            let mut out = Vec::new();
            for row in rows.flatten() {
                let sc: Value = serde_json::from_str(&row.2).unwrap_or(Value::Null);
                let meta: Value = serde_json::from_str(&row.3).unwrap_or(Value::Null);
                out.push(parse_cc_settings(&row.0, &row.1, &sc, &meta, row.4));
            }
            Some(out)
        })();
        let _ = std::fs::remove_dir_all(&tmp);
        return res;
    }
    // Older CC Switch (v3.x before SQLite): config.json { "claude": { "providers": {...}, "current": id } }
    let cfg = dir.join("config.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(cfg).ok()?).ok()?;
    let claude = v.get("claude")?;
    let current = claude.get("current").and_then(|c| c.as_str()).unwrap_or("");
    let mut out = Vec::new();
    for (id, p) in claude.get("providers")?.as_object()? {
        let name = p.get("name").and_then(|n| n.as_str()).unwrap_or(id);
        let sc = p.get("settingsConfig").cloned().unwrap_or(Value::Null);
        let meta = p.get("meta").cloned().unwrap_or(Value::Null);
        out.push(parse_cc_settings(id, name, &sc, &meta, id == current));
    }
    Some(out)
}

fn tempdir_for_copy() -> Option<PathBuf> {
    let d = std::env::temp_dir().join(format!("wickrun-brain-cc-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&d).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
    }
    Some(d)
}

pub fn ccswitch_sources(dir: &Path, list: &[CcProvider]) -> Vec<Source> {
    list.iter()
        .map(|p| {
            let openai = matches!(p.api_format.as_deref(), Some("openai_chat"));
            let (route, status) = match (&p.base_url, &p.key, p.api_format.as_deref()) {
                (_, _, Some(f)) if f != "anthropic" && f != "openai_chat" => (Route::None, "not_compatible"),
                (None, _, _) => (Route::None, "not_compatible"), // official login entry: nothing to switch to
                (Some(_), None, _) => (if openai { Route::Translate } else { Route::Direct }, "needs_key"),
                _ => (if openai { Route::Translate } else { Route::Direct }, "reachable"),
            };
            let base = p.base_url.clone().unwrap_or_default();
            let named = provider_for_url(&base).map(|pr| pr.name);
            Source {
                id: format!("ccswitch:{}", p.id),
                group: "ccswitch".into(),
                name: p.name.clone(),
                origin: format!("file:{}", dir.join("cc-switch.db").display()),
                status: status.into(),
                route,
                base_url: base,
                auth: p.auth,
                key_masked: p.key.as_deref().map(mask_secret),
                models: p.model.iter().cloned().collect(),
                default_model: p.model.clone(),
                endpoints: vec![],
                note: if p.base_url.is_none() { Some("cc_official".into()) } else if openai { Some("via_translator".into()) } else if p.is_current { Some("cc_current".into()) } else { named.map(|_| "cc_named".into()) },
                docs: None,
                key: KeyRef::CcSwitch { id: p.id.clone() },
                list_base: None,
            }
        })
        .collect()
}

/* ------------------------------------------------ local servers & gateways */

pub struct Probe {
    pub port: u16,
    pub kind: &'static str,
    pub name: &'static str,
    pub group: &'static str,
}

/// Default localhost ports. Gateways front cloud routes; local servers run models here.
pub const PROBES: &[Probe] = &[
    Probe { port: 11434, kind: "ollama", name: "Ollama", group: "local" },
    Probe { port: 1234, kind: "lmstudio", name: "LM Studio", group: "local" },
    Probe { port: 8080, kind: "llamacpp", name: "llama.cpp server", group: "local" },
    Probe { port: 8000, kind: "vllm", name: "vLLM", group: "local" },
    Probe { port: 1337, kind: "jan", name: "Jan", group: "local" },
    Probe { port: 20128, kind: "omniroute", name: "OmniRoute", group: "gateway" },
    Probe { port: 3456, kind: "ccr", name: "Claude Code Router", group: "gateway" },
    Probe { port: 4000, kind: "litellm", name: "LiteLLM", group: "gateway" },
    Probe { port: 3000, kind: "oneapi", name: "one-api / new-api", group: "gateway" },
];

/// Model ids from an OpenAI `/v1/models`, Anthropic `/v1/models` or Ollama `/api/tags` body.
pub fn parse_models(body: &Value) -> Vec<String> {
    let arr = body.get("data").or_else(|| body.get("models")).and_then(|d| d.as_array());
    let mut v: Vec<String> = arr
        .map(|a| a.iter().filter_map(|m| m.get("id").or_else(|| m.get("name")).or_else(|| m.get("model")).and_then(|i| i.as_str()).map(String::from)).collect())
        .unwrap_or_default();
    v.dedup();
    v
}

/// Compare dotted versions: is `v` at least `min`?
pub fn version_at_least(v: &str, min: &str) -> bool {
    let parse = |s: &str| s.trim_start_matches('v').split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).take(3).map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (a, b) = (parse(v), parse(min));
    for i in 0..3 {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    true
}

/// What a probe saw, gathered by `probe_port` (network) and judged by `judge` (pure).
#[derive(Debug, Default, Clone)]
pub struct Seen {
    pub alive: bool,
    /// GET /v1/models (or /api/tags for Ollama) status and body
    pub models_status: Option<u16>,
    pub models_body: Option<Value>,
    /// Status of POST /v1/messages with an empty body: 404/405 means no Anthropic endpoint
    pub messages_status: Option<u16>,
    /// Identification text (version / health / status endpoint)
    pub ident: Option<String>,
}

pub fn judge(p: &Probe, seen: &Seen, gateway_key: Option<(KeyRef, String)>) -> Option<Source> {
    if !seen.alive {
        return None;
    }
    let base = format!("http://127.0.0.1:{}", p.port);
    let models = seen.models_body.as_ref().map(parse_models).unwrap_or_default();
    let has_messages = matches!(seen.messages_status, Some(s) if s != 404 && s != 405);
    let mut name = p.name.to_string();
    // Identify the port's owner where it can be told apart.
    match p.kind {
        "ollama" if seen.ident.is_none() => return None,
        "litellm" if !seen.ident.as_deref().unwrap_or("").to_lowercase().contains("alive") && models.is_empty() => return None,
        "oneapi" => {
            let id = seen.ident.as_deref().unwrap_or("");
            if id.is_empty() {
                return None; // not one-api/new-api: port 3000 is often a web dev server
            }
            if id.to_lowercase().contains("new-api") || id.to_lowercase().contains("new api") {
                name = "new-api".into();
            }
        }
        "llamacpp" | "vllm" | "jan" | "lmstudio" | "omniroute" if seen.models_status.is_none() => return None,
        _ => {}
    }
    let needs_key = matches!(seen.models_status, Some(401) | Some(403)) || matches!(seen.messages_status, Some(401) | Some(403));
    let route = if has_messages { Route::Direct } else if seen.models_status.is_some() { Route::Translate } else { Route::None };
    let (key, masked) = match (&gateway_key, needs_key) {
        (Some((k, v)), _) => (k.clone(), Some(mask_secret(v))),
        (None, true) => (KeyRef::None, None),
        (None, false) => (KeyRef::Placeholder(match p.kind { "ollama" => "ollama", "lmstudio" => "lmstudio", _ => "wickrun-brain" }.into()), None),
    };
    let status = if route == Route::None { "not_compatible" } else if key == KeyRef::None { "needs_key" } else { "reachable" };
    let mut note = match (p.kind, route) {
        (_, Route::Translate) => Some("via_translator".to_string()),
        _ => None,
    };
    if p.kind == "ollama" {
        if let Some(v) = &seen.ident {
            if !version_at_least(v, "0.14.0") {
                note = Some("ollama_old".into());
            }
        }
    }
    let translate_base = format!("{}/v1", base);
    Some(Source {
        id: format!("{}:{}:{}", p.group, p.kind, p.port),
        group: p.group.into(),
        name,
        origin: format!("url:{}", base),
        status: status.into(),
        route,
        base_url: if route == Route::Translate { translate_base.clone() } else { base.clone() },
        auth: AuthKind::Bearer,
        key_masked: masked,
        default_model: models.first().cloned(),
        models,
        endpoints: vec![],
        note,
        docs: None,
        key,
        list_base: Some(translate_base),
    })
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_millis(1500)).connect_timeout(Duration::from_millis(400)).no_proxy().build().unwrap()
}

pub async fn probe_port(p: &Probe) -> Seen {
    let c = http();
    let base = format!("http://127.0.0.1:{}", p.port);
    let mut seen = Seen::default();
    let models_path = if p.kind == "ollama" { "/api/tags" } else { "/v1/models" };
    match c.get(format!("{}{}", base, models_path)).send().await {
        Ok(r) => {
            seen.alive = true;
            let st = r.status().as_u16();
            if st == 200 || st == 401 || st == 403 {
                seen.models_status = Some(st);
                seen.models_body = r.json::<Value>().await.ok();
            }
        }
        Err(e) if e.is_connect() => return seen,
        Err(_) => seen.alive = true,
    }
    let ident_path = match p.kind {
        "ollama" => Some("/api/version"),
        "litellm" => Some("/health/liveliness"),
        "oneapi" => Some("/api/status"),
        "ccr" => Some("/api/config"),
        "vllm" => Some("/version"),
        "llamacpp" => Some("/health"),
        _ => None,
    };
    if let Some(path) = ident_path {
        if let Ok(r) = c.get(format!("{}{}", base, path)).send().await {
            if r.status().is_success() || p.kind == "ccr" {
                let t = r.text().await.unwrap_or_default();
                seen.ident = match p.kind {
                    "ollama" => serde_json::from_str::<Value>(&t).ok().and_then(|v| v.get("version").and_then(|v| v.as_str()).map(String::from)),
                    "oneapi" => serde_json::from_str::<Value>(&t).ok().filter(|v| v.get("data").is_some() && v.get("success").is_some()).map(|v| {
                        let d = &v["data"];
                        format!("{} {}", d.get("system_name").and_then(|s| s.as_str()).unwrap_or("one-api"), d.get("version").and_then(|s| s.as_str()).unwrap_or(""))
                    }),
                    _ => Some(t.chars().take(200).collect()),
                };
            }
        }
    }
    if let Ok(r) = c.post(format!("{}/v1/messages", base)).header("content-type", "application/json").header("anthropic-version", "2023-06-01").body("{}").send().await {
        seen.messages_status = Some(r.status().as_u16());
    }
    seen
}

/// Keys that local gateways commonly keep in a known place.
pub fn gateway_key(kind: &str, get_env: &dyn Fn(&str) -> Option<String>, home: Option<&Path>) -> Option<(KeyRef, String)> {
    match kind {
        "litellm" => get_env("LITELLM_MASTER_KEY").map(|v| (KeyRef::Env("LITELLM_MASTER_KEY".into()), v)),
        "omniroute" => get_env("OMNIROUTE_API_KEY").map(|v| (KeyRef::Env("OMNIROUTE_API_KEY".into()), v)),
        "ccr" => {
            let f = home?.join(".claude-code-router").join("config.json");
            let v = read_file_key(&f, "APIKEY").filter(|v| !v.is_empty())?;
            Some((KeyRef::File { path: f, name: "APIKEY".into() }, v))
        }
        _ => None,
    }
}

/* ------------------------------------------------------------ wickrunAI */

/// Places the wickrunAI desktop app (Electron) is usually installed. Only existence is
/// checked; nothing inside its data folder is read.
pub fn wickrunai_install_paths(home: &Path, get_env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut v = vec![
        PathBuf::from("/Applications/wickrunAI.app"),
        home.join("Applications").join("wickrunAI.app"),
        PathBuf::from("/opt/wickrunAI/wickrunai"),
        PathBuf::from("/usr/bin/wickrunai"),
        home.join(".local").join("bin").join("wickrunAI.AppImage"),
        home.join("Applications").join("wickrunAI.AppImage"),
    ];
    if let Some(l) = get_env("LOCALAPPDATA") {
        v.push(PathBuf::from(&l).join("Programs").join("wickrunAI").join("wickrunAI.exe"));
        v.push(PathBuf::from(&l).join("Programs").join("wickrunai").join("wickrunAI.exe"));
    }
    if let Some(p) = get_env("ProgramFiles") {
        v.push(PathBuf::from(p).join("wickrunAI").join("wickrunAI.exe"));
    }
    v
}

pub fn wickrunai_data_dirs(home: &Path, get_env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut v = vec![home.join("Library").join("Application Support").join("wickrunAI"), home.join(".config").join("wickrunAI")];
    if let Some(a) = get_env("APPDATA") {
        v.push(PathBuf::from(a).join("wickrunAI"));
    }
    v
}

pub fn wickrunai_source(installed: Option<&Path>, data_dir_found: bool, took_over: bool) -> Option<Source> {
    if installed.is_none() && !data_dir_found && !took_over {
        return None;
    }
    Some(Source {
        id: "wickrunai".into(),
        group: "wickrunai".into(),
        name: "wickrunAI".into(),
        origin: installed.map(|p| format!("file:{}", p.display())).unwrap_or_else(|| "settings".into()),
        status: "managed_elsewhere".into(),
        route: Route::None,
        base_url: String::new(),
        auth: AuthKind::Bearer,
        key_masked: None,
        models: vec![],
        default_model: None,
        endpoints: vec![],
        note: Some(if took_over { "wickrunai_active".into() } else { "wickrunai_detected".into() }),
        docs: Some("https://wickrunai.com".into()),
        key: KeyRef::None,
        list_base: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const K1: &str = "sk-deepseek-0123456789abcdefWXYZ";
    const K2: &str = "sk-or-v1-0123456789abcdef0123ABCD";

    fn fx(p: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/discover").join(p)
    }

    #[test]
    fn shell_exports() {
        let text = std::fs::read_to_string(fx("zshrc")).unwrap();
        let got = parse_shell_exports(&text);
        assert!(got.contains(&("DEEPSEEK_API_KEY".into(), K1.into())));
        assert!(got.contains(&("OPENROUTER_API_KEY".into(), K2.into())));
        assert!(got.contains(&("MOONSHOT_API_KEY".into(), "sk-moon-singlequoted-1234567890".into())));
        assert!(got.contains(&("GROQ_API_KEY".into(), "gsk_fish_1234567890abcdef".into())));
        assert!(got.contains(&("GEMINI_API_KEY".into(), "AIza-powershell-1234567890".into())));
        assert!(!got.iter().any(|(n, _)| n == "OPENAI_API_KEY"), "references to other vars are skipped");
        assert!(!got.iter().any(|(n, _)| n == "ZHIPUAI_API_KEY"), "commented lines are skipped");
        assert!(!got.iter().any(|(n, _)| n == "DASHSCOPE_API_KEY"), "command output is skipped");
    }

    #[test]
    fn keys_from_env_and_files_are_masked_and_deduplicated() {
        let home = tempfile::tempdir().unwrap();
        std::fs::copy(fx("zshrc"), home.path().join(".zshrc")).unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::write(home.path().join(".codex/auth.json"), r#"{"OPENAI_API_KEY":"sk-proj-codexcodexcodex1234"}"#).unwrap();
        let env = |k: &str| match k { "DEEPSEEK_API_KEY" => Some(K1.to_string()), "MINIMAX_API_KEY" => Some("short".to_string()), _ => None };
        let got = find_keys(&env, Some(home.path()), Some(&fx("cli-profiles.json")));
        let ds: Vec<_> = got.iter().filter(|s| s.name == "DeepSeek").collect();
        assert_eq!(ds.len(), 1, "same key in env and .zshrc is listed once");
        assert_eq!(ds[0].origin, "env:DEEPSEEK_API_KEY");
        assert_eq!(ds[0].key_masked.as_deref(), Some("…WXYZ"));
        assert_eq!(ds[0].route, Route::Direct);
        let mm = got.iter().find(|s| s.name == "MiniMax").unwrap();
        assert_eq!(mm.key_masked.as_deref(), Some("set"));
        let or = got.iter().find(|s| s.name == "OpenRouter").unwrap();
        assert!(or.origin.contains(".zshrc#OPENROUTER_API_KEY"));
        let oa = got.iter().find(|s| s.name == "OpenAI").unwrap();
        assert_eq!(oa.route, Route::Translate);
        assert!(oa.origin.contains("auth.json"));
        let cli = got.iter().find(|s| s.id == "cli:work").unwrap();
        assert_eq!(cli.base_url, "https://llm.example.com");
        assert_eq!(cli.default_model.as_deref(), Some("gw-model-1"));
        let json = serde_json::to_string(&got).unwrap();
        for secret in [K1, K2, "sk-proj-codexcodexcodex1234", "sk-cli-stored-key-0123456789"] {
            assert!(!json.contains(secret), "no full key in what the UI sees");
        }
        assert_eq!(read_file_key(&fx("cli-profiles.json"), "profile:work").as_deref(), Some("sk-cli-stored-key-0123456789"));
        assert_eq!(read_file_key(&home.path().join(".zshrc"), "OPENROUTER_API_KEY").as_deref(), Some(K2));
    }

    fn make_ccswitch_db(dir: &Path) {
        let c = rusqlite::Connection::open(dir.join("cc-switch.db")).unwrap();
        c.execute_batch(&std::fs::read_to_string(fx("ccswitch.sql")).unwrap()).unwrap();
    }

    #[test]
    fn ccswitch_database() {
        let d = tempfile::tempdir().unwrap();
        make_ccswitch_db(d.path());
        let before = std::fs::read(d.path().join("cc-switch.db")).unwrap();
        let list = read_ccswitch(d.path()).unwrap();
        assert_eq!(std::fs::read(d.path().join("cc-switch.db")).unwrap(), before, "CC Switch's file is untouched");
        assert_eq!(list.len(), 4, "only app_type = claude");
        let ds = list.iter().find(|p| p.id == "deepseek-1").unwrap();
        assert_eq!(ds.base_url.as_deref(), Some("https://api.deepseek.com/anthropic"));
        assert_eq!(ds.auth, AuthKind::Bearer);
        assert!(ds.is_current);
        let srcs = ccswitch_sources(d.path(), &list);
        let s = |id: &str| srcs.iter().find(|s| s.id == format!("ccswitch:{}", id)).unwrap();
        assert_eq!(s("deepseek-1").status, "reachable");
        assert_eq!(s("deepseek-1").default_model.as_deref(), Some("deepseek-v4-pro"));
        assert_eq!(s("deepseek-1").key_masked.as_deref(), Some("…cc01"));
        assert_eq!(s("official").status, "not_compatible");
        assert_eq!(s("openai-chat").route, Route::Translate);
        assert_eq!(s("nokey").status, "needs_key");
        assert_eq!(s("nokey").auth, AuthKind::Bearer);
        assert!(!serde_json::to_string(&srcs).unwrap().contains("sk-ccswitch-deepseek-key-0000cc01"));
    }

    #[test]
    fn ccswitch_legacy_json_and_override_dir() {
        let d = tempfile::tempdir().unwrap();
        std::fs::copy(fx("ccswitch-config.json"), d.path().join("config.json")).unwrap();
        let list = read_ccswitch(d.path()).unwrap();
        assert_eq!(list.len(), 2);
        let k = list.iter().find(|p| p.id == "kimi").unwrap();
        assert!(k.is_current);
        assert_eq!(k.auth, AuthKind::ApiKey);
        let home = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(data.path().join("com.ccswitch.desktop")).unwrap();
        std::fs::write(data.path().join("com.ccswitch.desktop/app_paths.json"), r#"{"app_config_dir_override":"~/Dropbox/ccs"}"#).unwrap();
        let dirs = ccswitch_dirs(home.path(), Some(data.path()));
        assert_eq!(dirs, vec![home.path().join("Dropbox/ccs"), home.path().join(".cc-switch")]);
        assert!(read_ccswitch(home.path()).is_none());
    }

    fn body(name: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(fx(name)).unwrap()).unwrap()
    }
    fn probe(kind: &str) -> &'static Probe {
        PROBES.iter().find(|p| p.kind == kind).unwrap()
    }

    #[test]
    fn gateway_and_server_responses() {
        // Ollama new enough: direct, placeholder key, models from /api/tags
        let s = judge(probe("ollama"), &Seen { alive: true, models_status: Some(200), models_body: Some(body("ollama-tags.json")), messages_status: Some(400), ident: Some("0.15.2".into()) }, None).unwrap();
        assert_eq!((s.route, s.status.as_str(), s.base_url.as_str()), (Route::Direct, "reachable", "http://127.0.0.1:11434"));
        assert_eq!(s.models, vec!["qwen3-coder:30b", "gpt-oss:20b"]);
        assert_eq!(s.key, KeyRef::Placeholder("ollama".into()));
        // Old Ollama without /v1/messages: via translator, with a note
        let s = judge(probe("ollama"), &Seen { alive: true, models_status: Some(200), models_body: Some(body("ollama-tags.json")), messages_status: Some(404), ident: Some("0.9.0".into()) }, None).unwrap();
        assert_eq!((s.route, s.base_url.as_str(), s.note.as_deref()), (Route::Translate, "http://127.0.0.1:11434/v1", Some("ollama_old")));
        // LM Studio
        let s = judge(probe("lmstudio"), &Seen { alive: true, models_status: Some(200), models_body: Some(body("openai-models.json")), messages_status: Some(400), ident: None }, None).unwrap();
        assert_eq!(s.models, vec!["qwen/qwen3-coder-30b", "openai/gpt-oss-20b"]);
        assert_eq!(s.key, KeyRef::Placeholder("lmstudio".into()));
        // LiteLLM with auth on: needs key; with master key in env: reachable, masked
        let seen = Seen { alive: true, models_status: Some(401), models_body: None, messages_status: Some(401), ident: Some("\"I'm alive!\"".into()) };
        assert_eq!(judge(probe("litellm"), &seen, None).unwrap().status, "needs_key");
        let s = judge(probe("litellm"), &seen, Some((KeyRef::Env("LITELLM_MASTER_KEY".into()), "sk-litellm-master-0123456789".into()))).unwrap();
        assert_eq!((s.status.as_str(), s.key_masked.as_deref()), ("reachable", Some("…6789")));
        // one-api vs new-api vs a random dev server on :3000
        let ident = |v: &Value| format!("{} {}", v["data"]["system_name"].as_str().unwrap(), v["data"]["version"].as_str().unwrap());
        let s = judge(probe("oneapi"), &Seen { alive: true, models_status: Some(401), models_body: None, messages_status: Some(401), ident: Some(ident(&body("newapi-status.json"))) }, None).unwrap();
        assert_eq!((s.name.as_str(), s.route, s.status.as_str()), ("new-api", Route::Direct, "needs_key"));
        let s = judge(probe("oneapi"), &Seen { alive: true, models_status: Some(401), models_body: None, messages_status: Some(404), ident: Some(ident(&body("oneapi-status.json"))) }, None).unwrap();
        assert_eq!((s.name.as_str(), s.route), ("one-api / new-api", Route::Translate));
        assert!(judge(probe("oneapi"), &Seen { alive: true, ident: None, ..Default::default() }, None).is_none());
        // OmniRoute: open gateway, direct
        let s = judge(probe("omniroute"), &Seen { alive: true, models_status: Some(200), models_body: Some(body("openai-models.json")), messages_status: Some(400), ident: None }, None).unwrap();
        assert_eq!((s.group.as_str(), s.route, s.status.as_str()), ("gateway", Route::Direct, "reachable"));
        // Nothing listening
        assert!(judge(probe("ccr"), &Seen::default(), None).is_none());
    }

    #[test]
    fn versions() {
        assert!(version_at_least("0.14.0", "0.14.0"));
        assert!(version_at_least("v0.15.1-rc2", "0.14.0"));
        assert!(!version_at_least("0.13.9", "0.14.0"));
    }

    #[test]
    fn provider_lookup() {
        assert_eq!(provider_for_url("https://api.deepseek.com/anthropic").unwrap().id, "deepseek");
        assert_eq!(provider_for_url("https://api.minimaxi.com/anthropic").unwrap().id, "minimax");
        assert!(provider_for_url("https://example.com").is_none());
        assert!(PROVIDERS.iter().all(|p| crate::settings::validate_base_url(p.base).is_ok()));
    }

    #[test]
    fn wickrunai() {
        assert!(wickrunai_source(None, false, false).is_none());
        let s = wickrunai_source(Some(Path::new("/Applications/wickrunAI.app")), true, false).unwrap();
        assert_eq!((s.status.as_str(), s.note.as_deref()), ("managed_elsewhere", Some("wickrunai_detected")));
        assert_eq!(wickrunai_source(None, false, true).unwrap().note.as_deref(), Some("wickrunai_active"));
        let _ = json!({});
    }
}
