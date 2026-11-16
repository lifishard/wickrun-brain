//! wickrun-brain desktop app: find the models this machine can offer and give Claude Code a new brain.
//!
//! SPDX-License-Identifier: MIT

pub mod discover;
pub mod keystore;
pub mod manual;
pub mod proxy;
pub mod settings;
pub mod translate;

use discover::{AuthKind, KeyRef, Route, Source};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use manual::{ManualEntry, ManualInput};
use settings::{AppError, Level};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

const DOWNLOAD_URL: &str = "https://wickrunai.com";

/* ------------------------------------------------------------ app state */

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Active {
    pub source_id: String,
    pub name: String,
    pub model: String,
    pub route: Route,
    /// For translated routes: OpenAI-style upstream base
    pub upstream_base: String,
    pub key: KeyRef,
    /// The thinking level chosen with this switch
    #[serde(default)]
    pub thinking: Level,
    /// What the translator adds as `reasoning_effort` (translated routes only)
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

/// The app's own small state file (0600). Never holds a key except ones the user pasted.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct GuiState {
    pub version: u32,
    pub proxy_port: Option<u16>,
    pub proxy_token: Option<String>,
    pub active: Option<Active>,
    #[serde(default)]
    pub pasted_keys: BTreeMap<String, String>,
    /// Entries added by hand (keys are in the keychain or manual-keys.json, not here)
    #[serde(default)]
    pub manual: Vec<ManualEntry>,
    /// Thinking level per source id (absent = Default)
    #[serde(default)]
    pub thinking: BTreeMap<String, Level>,
}

pub fn config_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        dirs::config_dir().map(|d| d.join("wickrun-brain"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| dirs::home_dir().map(|h| h.join(".config"))).map(|d| d.join("wickrun-brain"))
    }
}

fn key_store() -> Option<keystore::KeyStore> {
    config_dir().map(|d| keystore::KeyStore::system(&d))
}

fn stored_key(st: &GuiState, id: &str) -> Option<String> {
    let e = st.manual.iter().find(|e| e.id == id)?;
    key_store()?.load(&e.id, e.key_store)
}

fn manual_sources(st: &GuiState) -> Vec<Source> {
    st.manual.iter().map(|e| manual::to_source(e, stored_key(st, &e.id).is_some())).collect()
}

fn state_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("gui-state.json"))
}

pub fn load_state(file: &Path) -> GuiState {
    std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_state(file: &Path, st: &GuiState) -> Result<(), AppError> {
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d).map_err(|e| AppError::new("io", e.to_string()))?;
    }
    let mut st = st.clone();
    st.version = 1;
    settings::write_atomic(file, (serde_json::to_string_pretty(&st).unwrap() + "\n").as_bytes())
}

pub struct App {
    pub sources: Vec<Source>,
    pub state: GuiState,
    pub proxy: Option<proxy::Proxy>,
    pub proxy_error: Option<String>,
    pub wickrunai_path: Option<PathBuf>,
}

type Shared = Arc<Mutex<App>>;

fn env_get(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

fn settings_file() -> Result<PathBuf, AppError> {
    settings::settings_path(&env_get, dirs::home_dir())
}

/// Look a key up again at the moment it is needed. Returns None if it is gone.
pub fn resolve_key(k: &KeyRef, source_id: &str, st: &GuiState) -> Option<String> {
    let v = match k {
        KeyRef::Env(n) => env_get(n),
        KeyRef::File { path, name } => discover::read_file_key(path, name),
        KeyRef::CcSwitch { id } => {
            let home = dirs::home_dir()?;
            discover::ccswitch_dirs(&home, dirs::data_dir().as_deref()).iter().find_map(|d| discover::read_ccswitch(d)).and_then(|l| l.into_iter().find(|p| &p.id == id)).and_then(|p| p.key)
        }
        KeyRef::Placeholder(s) => Some(s.clone()),
        KeyRef::Stored { id } => stored_key(st, id),
        KeyRef::Pasted | KeyRef::None => None,
    };
    v.filter(|s| !s.is_empty()).or_else(|| st.pasted_keys.get(source_id).cloned())
}

/* ----------------------------------------------------------- discovery */

async fn run_discovery(st: &GuiState, took_over: bool) -> (Vec<Source>, Option<PathBuf>) {
    let home = dirs::home_dir();
    let cli_profiles = config_dir().map(|d| d.join("profiles.json"));
    let mut out = Vec::new();
    // 1 + 5: local servers and gateways, probed in parallel
    let probes = futures_util::future::join_all(discover::PROBES.iter().map(|p| async move { (p, discover::probe_port(p).await) })).await;
    for (p, seen) in probes {
        if let Some(s) = discover::judge(p, &seen, discover::gateway_key(p.kind, &env_get, home.as_deref())) {
            out.push(s);
        }
    }
    // 2: API keys
    out.extend(discover::find_keys(&env_get, home.as_deref(), cli_profiles.as_deref()));
    // 3: CC Switch
    if let Some(h) = &home {
        for d in discover::ccswitch_dirs(h, dirs::data_dir().as_deref()) {
            if let Some(list) = discover::read_ccswitch(&d) {
                out.extend(discover::ccswitch_sources(&d, &list));
                break;
            }
        }
    }
    // 4: wickrunAI
    let mut wpath = None;
    if let Some(h) = &home {
        wpath = discover::wickrunai_install_paths(h, &env_get).into_iter().find(|p| p.exists());
        let data = discover::wickrunai_data_dirs(h, &env_get).iter().any(|p| p.exists());
        if let Some(s) = discover::wickrunai_source(wpath.as_deref(), data, took_over) {
            out.push(s);
        }
    }
    // Show paths under the home folder as ~/... (shorter, and does not reveal the user name)
    if let Some(h) = &home {
        let hs = h.display().to_string();
        for s in out.iter_mut() {
            if let Some(rest) = s.origin.strip_prefix(&format!("file:{}", hs)) {
                s.origin = format!("file:~{}", rest);
            }
        }
    }
    // 6: entries added by hand
    out.extend(manual_sources(st));
    // Keys the user pasted turn "needs key" into usable
    for s in out.iter_mut() {
        if s.status == "needs_key" {
            if let Some(k) = st.pasted_keys.get(&s.id) {
                s.status = "reachable".into();
                s.key_masked = Some(settings::mask_secret(k));
                if s.key == KeyRef::None {
                    s.key = KeyRef::Pasted;
                }
            }
        }
    }
    (out, wpath)
}

/* -------------------------------------------------------- what's active */

fn tilde(p: &str) -> String {
    match dirs::home_dir().map(|h| h.display().to_string()) {
        Some(h) if p.starts_with(&h) => format!("~{}", &p[h.len()..]),
        _ => p.to_string(),
    }
}

#[derive(Serialize)]
pub struct Current {
    /// official | custom | proxy | proxy_down | wickrunai | broken
    pub kind: String,
    pub label: Option<String>,
    pub model: Option<String>,
    pub backup_exists: bool,
    pub settings_file: String,
    pub base_url: Option<String>,
    pub credential_var: Option<String>,
    pub credential_masked: Option<String>,
}

fn current(app: &App) -> Current {
    let file = settings_file();
    let st = match &file {
        Ok(f) => settings::status(f),
        Err(_) => return Current { kind: "broken".into(), label: None, model: None, backup_exists: false, settings_file: String::new(), base_url: None, credential_var: None, credential_masked: None },
    };
    let mut kind = match st.mode.as_str() {
        "wickrunai-desktop" => "wickrunai",
        m => m,
    }
    .to_string();
    let mut label = None;
    if kind == "custom" {
        let base = st.base_url.clone().unwrap_or_default();
        let ours = app.state.proxy_port.map(|p| format!("http://127.0.0.1:{}", p));
        if ours.as_deref() == Some(base.trim_end_matches('/')) {
            label = app.state.active.as_ref().map(|a| a.name.clone());
            let up = app.proxy.is_some() && app.state.active.as_ref().map(|a| a.route == Route::Translate).unwrap_or(false);
            kind = if up { "proxy".into() } else { "proxy_down".into() };
        } else if let Some(a) = app.state.active.as_ref().filter(|a| a.route == Route::Direct) {
            if app.sources.iter().any(|s| s.id == a.source_id && s.base_url.trim_end_matches('/') == base.trim_end_matches('/')) {
                label = Some(a.name.clone());
            }
        }
        if label.is_none() {
            label = app.sources.iter().find(|s| s.route == Route::Direct && s.base_url.trim_end_matches('/') == base.trim_end_matches('/')).map(|s| s.name.clone());
        }
        if label.is_none() {
            label = discover::provider_for_url(&base).map(|p| p.name.to_string()).or_else(|| url::Url::parse(&base).ok().and_then(|u| u.host_str().map(|h| match u.port() { Some(p) => format!("{}:{}", h, p), None => h.to_string() })));
        }
    }
    Current {
        kind,
        label,
        model: st.model,
        backup_exists: st.backup_exists,
        settings_file: tilde(&st.settings_file),
        base_url: st.base_url,
        credential_var: st.credential_var,
        credential_masked: st.credential_masked,
    }
}

#[derive(Serialize)]
pub struct View {
    pub current: Current,
    /// Which listed source Claude Code is using, when the app can tell
    pub active_id: Option<String>,
    pub sources: Vec<Source>,
    pub proxy_running: bool,
    pub proxy_port: Option<u16>,
    pub proxy_error: Option<String>,
    pub wickrunai_installed: bool,
    pub download_url: &'static str,
    pub version: &'static str,
    /// Per source id: claude_code | reasoning_effort | none
    pub thinking_support: BTreeMap<String, &'static str>,
    /// Saved thinking level per source id
    pub thinking: BTreeMap<String, Level>,
    /// Entries added by hand, for the edit form (no keys)
    pub manual: Vec<ManualEntry>,
    /// The thinking level of the current choice, when the app made it
    pub active_thinking: Option<Level>,
}

fn active_id_thinking(app: &App) -> Option<Level> {
    app.state.active.as_ref().map(|a| a.thinking)
}

/// The level to use for a source: the one asked for, else the saved one; Default where nothing can honour it.
fn level_for(app: &App, s: &Source, asked: Option<Level>) -> Level {
    let l = asked.or_else(|| app.state.thinking.get(&s.id).copied()).unwrap_or_default();
    if manual::thinking_support(s) == "none" { Level::Default } else { l }
}

fn reasoning_for(s: &Source, level: Level) -> Option<String> {
    translate::reasoning_effort_for(manual::reasoning_provider(s), level).map(String::from)
}

fn view(app: &App) -> View {
    let cur = current(app);
    let active_id = app.state.active.as_ref().and_then(|a| {
        let base = cur.base_url.as_deref().unwrap_or("").trim_end_matches('/');
        let ok = match cur.kind.as_str() {
            "proxy" | "proxy_down" => true,
            "custom" => a.route == Route::Direct && a.upstream_base.trim_end_matches('/') == base && cur.model.as_deref() == Some(a.model.as_str()),
            _ => false,
        };
        ok.then(|| a.source_id.clone())
    });
    View {
        current: cur,
        active_id,
        sources: app.sources.clone(),
        proxy_running: app.proxy.is_some(),
        proxy_port: app.proxy.as_ref().map(|p| p.port).or(app.state.proxy_port),
        proxy_error: app.proxy_error.clone(),
        wickrunai_installed: app.wickrunai_path.is_some(),
        download_url: DOWNLOAD_URL,
        version: env!("CARGO_PKG_VERSION"),
        thinking_support: app.sources.iter().map(|s| (s.id.clone(), manual::thinking_support(s))).collect(),
        thinking: app.state.thinking.clone(),
        manual: app.state.manual.clone(),
        active_thinking: active_id_thinking(app),
    }
}

/* -------------------------------------------------------------- actions */

fn find<'a>(app: &'a App, id: &str) -> Result<&'a Source, AppError> {
    app.sources.iter().find(|s| s.id == id).ok_or_else(|| AppError::new("not_found", "this source is no longer there; refresh"))
}

fn pick_base(s: &Source, endpoint: Option<&str>) -> Result<String, AppError> {
    match endpoint {
        Some(e) if !e.is_empty() && e != s.base_url => {
            if s.endpoints.iter().any(|(_, u)| u == e) {
                Ok(e.to_string())
            } else {
                Err(AppError::new("bad_url", "unknown endpoint"))
            }
        }
        _ => Ok(s.base_url.clone()),
    }
}

async fn ensure_proxy(app: &mut App) -> Result<(), AppError> {
    if app.proxy.is_some() {
        return Ok(());
    }
    let token = app.state.proxy_token.clone().unwrap_or_else(proxy::new_token);
    let p = match proxy::start(app.state.proxy_port.unwrap_or(0), token.clone()).await {
        Ok(p) => p,
        Err(_) => proxy::start(0, token.clone()).await.map_err(|e| AppError::new("proxy_start", e.to_string()))?,
    };
    app.state.proxy_port = Some(p.port);
    app.state.proxy_token = Some(token);
    app.proxy = Some(p);
    app.proxy_error = None;
    Ok(())
}

#[derive(Serialize)]
pub struct TestResult {
    pub ok: bool,
    pub status: Option<u16>,
    pub ms: u128,
    /// error code for the UI: auth | rate | not_found | model | network | server | other
    pub code: Option<String>,
    pub message: Option<String>,
}

fn classify(status: u16) -> &'static str {
    match status {
        401 | 403 => "auth",
        429 => "rate",
        404 => "not_found",
        400 | 422 => "model",
        500..=599 => "server",
        _ => "other",
    }
}

async fn test_call(route: Route, base: &str, auth: AuthKind, key: &str, model: &str, reasoning: Option<&str>) -> TestResult {
    let r = test_once(route, base, auth, key, model, reasoning).await;
    // Same rule as the translator: a model that refuses reasoning_effort is retried without it.
    if let (false, Some(_), Some(true)) = (r.0.ok, reasoning, r.1) {
        let mut again = test_once(route, base, auth, key, model, None).await.0;
        if again.ok {
            again.code = Some("reasoning_dropped".into());
        }
        return again;
    }
    r.0
}

async fn test_once(route: Route, base: &str, auth: AuthKind, key: &str, model: &str, reasoning: Option<&str>) -> (TestResult, Option<bool>) {
    let c = reqwest::Client::builder().timeout(std::time::Duration::from_secs(45)).build().unwrap();
    let t0 = std::time::Instant::now();
    let req = match route {
        Route::Direct => {
            let r = c.post(format!("{}/v1/messages", base.trim_end_matches('/'))).header("anthropic-version", "2023-06-01");
            let r = match auth { AuthKind::ApiKey => r.header("x-api-key", key), AuthKind::Bearer => r.bearer_auth(key) };
            r.json(&json!({"model": model, "max_tokens": 16, "messages": [{"role": "user", "content": "Reply with the word OK"}]}))
        }
        _ => {
            let mut body = json!({"model": model, "max_tokens": 16, "messages": [{"role": "user", "content": "Reply with the word OK"}]});
            translate::apply_reasoning(&mut body, reasoning);
            c.post(format!("{}/chat/completions", base.trim_end_matches('/'))).bearer_auth(key).json(&body)
        }
    };
    match req.send().await {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.bytes().await.unwrap_or_default();
            let ms = t0.elapsed().as_millis();
            if (200..300).contains(&status) {
                (TestResult { ok: true, status: Some(status), ms, code: None, message: None }, None)
            } else {
                let (_, e) = translate::map_error(status, &body);
                let msg: String = e["error"]["message"].as_str().unwrap_or("").replace(key, "…").chars().take(300).collect();
                (TestResult { ok: false, status: Some(status), ms, code: Some(classify(status).into()), message: Some(msg) }, Some(translate::rejected_reasoning(status, &body)))
            }
        }
        Err(e) => (TestResult { ok: false, status: None, ms: t0.elapsed().as_millis(), code: Some("network".into()), message: Some(if e.is_timeout() { "timed out".into() } else { "could not connect".into() }) }, None),
    }
}

/* ------------------------------------------------------------ commands */

mod commands {
    use super::*;
    use tauri::State;

    type R<T> = Result<T, AppError>;

    #[tauri::command]
    pub async fn get_view(app: State<'_, Shared>) -> R<View> {
        Ok(view(&*app.lock().await))
    }

    #[tauri::command]
    pub async fn refresh(app: State<'_, Shared>) -> R<View> {
        let (st, took_over) = {
            let a = app.lock().await;
            (a.state.clone(), current(&a).kind == "wickrunai")
        };
        let (sources, wpath) = run_discovery(&st, took_over).await;
        let mut a = app.lock().await;
        a.sources = sources;
        a.wickrunai_path = wpath;
        Ok(view(&a))
    }

    #[tauri::command]
    pub async fn list_models(app: State<'_, Shared>, id: String) -> R<Vec<String>> {
        let (s, key) = {
            let a = app.lock().await;
            let s = find(&a, &id)?.clone();
            let key = resolve_key(&s.key, &s.id, &a.state);
            (s, key)
        };
        let c = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().unwrap();
        let fetched = if let Some(lb) = &s.list_base {
            let mut r = c.get(format!("{}/models", lb.trim_end_matches('/')));
            if let Some(k) = &key {
                r = r.bearer_auth(k);
                if s.group == "manual" && s.route == Route::Direct {
                    r = r.header("x-api-key", k).header("anthropic-version", "2023-06-01");
                }
            }
            r.send().await.ok().filter(|r| r.status().is_success())
        } else if s.base_url.starts_with("https://api.anthropic.com") {
            match &key {
                Some(k) => c.get("https://api.anthropic.com/v1/models?limit=100").header("x-api-key", k).header("anthropic-version", "2023-06-01").send().await.ok().filter(|r| r.status().is_success()),
                None => None,
            }
        } else {
            None
        };
        let mut models = match fetched {
            Some(r) => discover::parse_models(&r.json::<Value>().await.unwrap_or(Value::Null)),
            None => vec![],
        };
        if models.is_empty() {
            models = s.models.clone();
        }
        Ok(models)
    }

    #[tauri::command]
    pub async fn test_source(app: State<'_, Shared>, id: String, model: String, endpoint: Option<String>, thinking: Option<Level>) -> R<TestResult> {
        settings::validate_model(&model)?;
        let (s, key, base, reasoning) = {
            let a = app.lock().await;
            let s = find(&a, &id)?.clone();
            let key = resolve_key(&s.key, &s.id, &a.state).ok_or_else(|| AppError::new("no_key", "no key"))?;
            let base = pick_base(&s, endpoint.as_deref())?;
            let reasoning = reasoning_for(&s, level_for(&a, &s, thinking));
            (s, key, base, reasoning)
        };
        if s.route == Route::None {
            return Err(AppError::new("not_compatible", "not compatible"));
        }
        Ok(test_call(s.route, &base, s.auth, &key, &model, reasoning.as_deref()).await)
    }

    #[tauri::command]
    pub async fn use_source(app: State<'_, Shared>, id: String, model: String, endpoint: Option<String>, thinking: Option<Level>) -> R<View> {
        settings::validate_model(&model)?;
        let file = settings_file()?;
        let mut a = app.lock().await;
        let s = find(&a, &id)?.clone();
        let key = resolve_key(&s.key, &s.id, &a.state).ok_or_else(|| AppError::new("no_key", "no key"))?;
        let base = pick_base(&s, endpoint.as_deref())?;
        let level = level_for(&a, &s, thinking);
        match s.route {
            Route::None => return Err(AppError::new("not_compatible", "not compatible")),
            Route::Direct => {
                let auth = match s.auth { AuthKind::ApiKey => settings::Auth::ApiKey, AuthKind::Bearer => settings::Auth::Bearer };
                let thinking = manual::direct_thinking_env(&s, &base, level);
                settings::apply(&file, &settings::Target { base_url: &base, thinking, auth, key: &key, model: &model })?;
                if let Some(p) = &a.proxy {
                    *p.inner.route.write().await = None;
                }
                a.state.active = Some(Active { source_id: s.id.clone(), name: s.name.clone(), model: model.clone(), route: Route::Direct, upstream_base: base, key: s.key.clone(), thinking: level, reasoning_effort: None });
            }
            Route::Translate => {
                // Check first that settings.json can be changed at all (wickrunAI takeover, broken JSON).
                let cur = settings::read_settings(&file)?;
                if settings::detect_desktop(&cur.data, &file).detected {
                    return Err(AppError::new("desktop_takeover", "managed by wickrunAI"));
                }
                // A fresh token for each switch; the upstream key stays in this process.
                a.state.proxy_token = Some(proxy::new_token());
                if let Some(p) = &a.proxy {
                    *p.inner.token.write().await = a.state.proxy_token.clone().unwrap();
                }
                ensure_proxy(&mut a).await?;
                let p = a.proxy.clone().unwrap();
                let reasoning = reasoning_for(&s, level);
                *p.inner.route.write().await = Some(proxy::Route { upstream_base: base.clone(), upstream_key: key.clone(), model: model.clone(), reasoning_effort: reasoning.clone() });
                let token = a.state.proxy_token.clone().unwrap();
                let local = format!("http://127.0.0.1:{}", p.port);
                // The level goes to the upstream as reasoning_effort; settings.json keeps the original thinking values.
                settings::apply(&file, &settings::Target { base_url: &local, thinking: vec![], auth: settings::Auth::Bearer, key: &token, model: &model })?;
                a.state.active = Some(Active { source_id: s.id.clone(), name: s.name.clone(), model: model.clone(), route: Route::Translate, upstream_base: base, key: s.key.clone(), thinking: level, reasoning_effort: reasoning });
            }
        }
        if level == Level::Default {
            a.state.thinking.remove(&s.id);
        } else {
            a.state.thinking.insert(s.id.clone(), level);
        }
        if let Some(f) = state_file() {
            save_state(&f, &a.state)?;
        }
        Ok(view(&a))
    }

    #[tauri::command]
    pub async fn set_key(app: State<'_, Shared>, id: String, key: String) -> R<View> {
        let key = key.trim().to_string();
        settings::validate_key(&key)?;
        let mut a = app.lock().await;
        find(&a, &id)?;
        a.state.pasted_keys.insert(id.clone(), key.clone());
        if let Some(s) = a.sources.iter_mut().find(|s| s.id == id) {
            if s.status == "needs_key" {
                s.status = "reachable".into();
            }
            s.key_masked = Some(settings::mask_secret(&key));
            if s.key == KeyRef::None {
                s.key = KeyRef::Pasted;
            }
        }
        if let Some(f) = state_file() {
            save_state(&f, &a.state)?;
        }
        Ok(view(&a))
    }

    /// Remember a thinking level for a source without switching.
    #[tauri::command]
    pub async fn set_thinking(app: State<'_, Shared>, id: String, level: Level) -> R<View> {
        let mut a = app.lock().await;
        let s = find(&a, &id)?.clone();
        let level = level_for(&a, &s, Some(level));
        if level == Level::Default {
            a.state.thinking.remove(&id);
        } else {
            a.state.thinking.insert(id, level);
        }
        if let Some(f) = state_file() {
            save_state(&f, &a.state)?;
        }
        Ok(view(&a))
    }

    fn refresh_manual(a: &mut App) {
        a.sources.retain(|s| s.group != "manual");
        let m = manual_sources(&a.state);
        a.sources.extend(m);
    }

    #[tauri::command]
    pub async fn add_manual(app: State<'_, Shared>, input: ManualInput) -> R<(String, View)> {
        let (name, base_url, model) = manual::validate(&input, true)?;
        let ks = key_store().ok_or_else(|| AppError::new("no_home", "no config folder"))?;
        let key = input.key.trim().to_string();
        let mut a = app.lock().await;
        let id = manual::new_id();
        let key_store = ks.save(&id, &key)?;
        a.state.manual.push(ManualEntry { id: id.clone(), name, base_url, format: input.format, model, key_store, key_masked: settings::mask_secret(&key) });
        if let Some(f) = state_file() {
            if let Err(e) = save_state(&f, &a.state) {
                a.state.manual.retain(|e| e.id != id);
                let _ = ks.delete(&id);
                return Err(e);
            }
        }
        refresh_manual(&mut a);
        Ok((id, view(&a)))
    }

    #[tauri::command]
    pub async fn update_manual(app: State<'_, Shared>, id: String, input: ManualInput) -> R<View> {
        let (name, base_url, model) = manual::validate(&input, false)?;
        let ks = key_store().ok_or_else(|| AppError::new("no_home", "no config folder"))?;
        let mut a = app.lock().await;
        let idx = a.state.manual.iter().position(|e| e.id == id).ok_or_else(|| AppError::new("not_found", "no such entry"))?;
        let key = input.key.trim().to_string();
        let mut e = a.state.manual[idx].clone();
        if !key.is_empty() {
            e.key_store = ks.save(&id, &key)?;
            e.key_masked = settings::mask_secret(&key);
        }
        e.name = name;
        e.base_url = base_url;
        e.format = input.format;
        e.model = model;
        a.state.manual[idx] = e;
        if let Some(f) = state_file() {
            save_state(&f, &a.state)?;
        }
        refresh_manual(&mut a);
        Ok(view(&a))
    }

    #[tauri::command]
    pub async fn delete_manual(app: State<'_, Shared>, id: String) -> R<View> {
        let mut a = app.lock().await;
        if view(&a).active_id.as_deref() == Some(id.as_str()) {
            return Err(AppError::new("in_use", "Claude Code is using this entry"));
        }
        if !a.state.manual.iter().any(|e| e.id == id) {
            return Err(AppError::new("not_found", "no such entry"));
        }
        if let Some(ks) = key_store() {
            ks.delete(&id)?;
        }
        a.state.manual.retain(|e| e.id != id);
        a.state.thinking.remove(&id);
        if let Some(f) = state_file() {
            save_state(&f, &a.state)?;
        }
        refresh_manual(&mut a);
        Ok(view(&a))
    }

    /// Probe an address from the form: suggests the format and lists models when the source offers them.
    /// `id` lets the edit form use the saved key when the key field is left empty.
    #[tauri::command]
    pub async fn detect_manual(app: State<'_, Shared>, base_url: String, key: Option<String>, id: Option<String>) -> R<manual::Detected> {
        let key = match key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
            Some(k) => {
                settings::validate_key(&k)?;
                Some(k)
            }
            None => match id {
                Some(id) => stored_key(&app.lock().await.state, &id),
                None => None,
            },
        };
        let o = manual::probe(&base_url, key.as_deref()).await?;
        Ok(manual::judge_format(&base_url, &o))
    }

    #[tauri::command]
    pub async fn restore(app: State<'_, Shared>) -> R<(String, View)> {
        let file = settings_file()?;
        let r = settings::restore(&file)?;
        let mut a = app.lock().await;
        a.state.active = None;
        if let Some(p) = &a.proxy {
            *p.inner.route.write().await = None;
        }
        if let Some(f) = state_file() {
            save_state(&f, &a.state)?;
        }
        Ok((format!("{:?}", r), view(&a)))
    }

    #[tauri::command]
    pub async fn use_official(app: State<'_, Shared>) -> R<View> {
        let file = settings_file()?;
        settings::official(&file)?;
        let mut a = app.lock().await;
        a.state.active = None;
        if let Some(p) = &a.proxy {
            *p.inner.route.write().await = None;
        }
        if let Some(f) = state_file() {
            save_state(&f, &a.state)?;
        }
        Ok(view(&a))
    }

    #[tauri::command]
    pub async fn open_wickrunai(app: State<'_, Shared>, handle: tauri::AppHandle) -> R<()> {
        use tauri_plugin_opener::OpenerExt;
        let p = app.lock().await.wickrunai_path.clone();
        let r = match p {
            Some(p) => handle.opener().open_path(p.display().to_string(), None::<&str>),
            None => handle.opener().open_url(DOWNLOAD_URL, None::<&str>),
        };
        r.map_err(|e| AppError::new("open", e.to_string()))
    }

    #[tauri::command]
    pub async fn open_link(handle: tauri::AppHandle, url: String) -> R<()> {
        use tauri_plugin_opener::OpenerExt;
        if !(url.starts_with("https://")) {
            return Err(AppError::new("bad_url", "only https links"));
        }
        handle.opener().open_url(url, None::<&str>).map_err(|e| AppError::new("open", e.to_string()))
    }

    #[tauri::command]
    pub async fn get_autostart(handle: tauri::AppHandle) -> R<bool> {
        use tauri_plugin_autostart::ManagerExt;
        Ok(handle.autolaunch().is_enabled().unwrap_or(false))
    }

    #[tauri::command]
    pub async fn set_autostart(handle: tauri::AppHandle, on: bool) -> R<bool> {
        use tauri_plugin_autostart::ManagerExt;
        let m = handle.autolaunch();
        let r = if on { m.enable() } else { m.disable() };
        r.map_err(|e| AppError::new("autostart", e.to_string()))?;
        Ok(m.is_enabled().unwrap_or(on))
    }
}

/* ------------------------------------------------------------------ run */

async fn startup() -> App {
    let state = state_file().map(|f| load_state(&f)).unwrap_or_default();
    let mut app = App { sources: vec![], state, proxy: None, proxy_error: None, wickrunai_path: None };
    // If the last choice went through the translator, bring it back on the same port and token.
    if let Some(act) = app.state.active.clone().filter(|a| a.route == Route::Translate) {
        if let (Some(port), Some(token)) = (app.state.proxy_port, app.state.proxy_token.clone()) {
            match proxy::start(port, token).await {
                Ok(p) => {
                    match resolve_key(&act.key, &act.source_id, &app.state) {
                        Some(k) => *p.inner.route.write().await = Some(proxy::Route { upstream_base: act.upstream_base.clone(), upstream_key: k, model: act.model.clone(), reasoning_effort: act.reasoning_effort.clone() }),
                        None => app.proxy_error = Some("key_gone".into()),
                    }
                    app.proxy = Some(p);
                }
                Err(_) => app.proxy_error = Some("port_taken".into()),
            }
        }
    }
    app
}

fn wants_background(app: &App) -> bool {
    app.proxy.is_some() && app.state.active.as_ref().map(|a| a.route == Route::Translate).unwrap_or(false)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;
    use tauri::Manager;

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    tauri::async_runtime::set(rt.handle().clone());
    let shared: Shared = Arc::new(Mutex::new(rt.block_on(startup())));
    let for_close = shared.clone();
    let hidden = std::env::args().any(|a| a == "--hidden");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            commands::get_view,
            commands::refresh,
            commands::list_models,
            commands::test_source,
            commands::use_source,
            commands::set_key,
            commands::set_thinking,
            commands::add_manual,
            commands::update_manual,
            commands::delete_manual,
            commands::detect_manual,
            commands::restore,
            commands::use_official,
            commands::open_wickrunai,
            commands::open_link,
            commands::get_autostart,
            commands::set_autostart,
        ])
        .setup(move |app| {
            let show = MenuItem::with_id(app, "show", "Open wickrun-brain", true, None::<&str>)?;
            let restore = MenuItem::with_id(app, "restore", "Restore original settings", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &restore, &quit])?;
            let st = shared.clone();
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().cloned().expect("icon"))
                .tooltip("wickrun-brain")
                .menu(&menu)
                .on_menu_event(move |app, ev| match ev.id().as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "restore" => {
                        if let Ok(f) = settings_file() {
                            let _ = settings::restore(&f);
                        }
                        let st = st.clone();
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let mut a = st.lock().await;
                            a.state.active = None;
                            if let Some(f) = state_file() {
                                let _ = save_state(&f, &a.state);
                            }
                            drop(a);
                            use tauri::Emitter;
                            let _ = app.emit("changed", ());
                        });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            if hidden {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            Ok(())
        })
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Keep the translator running in the tray while Claude Code depends on it.
                let bg = tauri::async_runtime::block_on(async { wants_background(&*for_close.lock().await) });
                if bg {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running wickrun-brain");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trip_is_private_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x").join("gui-state.json");
        let mut st = GuiState::default();
        st.proxy_port = Some(41000);
        st.active = Some(Active { source_id: "keys:openai".into(), name: "OpenAI".into(), model: "gpt".into(), route: Route::Translate, upstream_base: "https://api.openai.com/v1".into(), key: KeyRef::Env("OPENAI_API_KEY".into()), thinking: Level::High, reasoning_effort: Some("high".into()) });
        save_state(&f, &st).unwrap();
        let back = load_state(&f);
        assert_eq!(back.proxy_port, Some(41000));
        let act = back.active.unwrap();
        assert_eq!(act.key, KeyRef::Env("OPENAI_API_KEY".into()));
        assert_eq!((act.thinking, act.reasoning_effort.as_deref()), (Level::High, Some("high")));
        // a 0.2.0 state file (no thinking, no manual entries) still loads
        let old: GuiState = serde_json::from_str(r#"{"version":1,"proxy_port":1,"active":{"source_id":"a","name":"A","model":"m","route":"direct","upstream_base":"https://x","key":"None"}}"#).unwrap();
        assert_eq!(old.active.unwrap().thinking, Level::Default);
        assert!(old.manual.is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // the env var's value is not copied into the state file
        assert!(!std::fs::read_to_string(&f).unwrap().contains("sk-"));
    }

    #[test]
    fn pasted_key_is_fallback() {
        let mut st = GuiState::default();
        st.pasted_keys.insert("gateway:litellm:4000".into(), "sk-pasted".into());
        assert_eq!(resolve_key(&KeyRef::None, "gateway:litellm:4000", &st).as_deref(), Some("sk-pasted"));
        assert_eq!(resolve_key(&KeyRef::Placeholder("ollama".into()), "x", &st).as_deref(), Some("ollama"));
        assert_eq!(resolve_key(&KeyRef::Env("WB_DEFINITELY_UNSET_VAR".into()), "x", &st), None);
    }

    #[test]
    fn classify_codes() {
        assert_eq!(classify(401), "auth");
        assert_eq!(classify(429), "rate");
        assert_eq!(classify(503), "server");
    }
}
