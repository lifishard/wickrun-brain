//! Entries the user adds by hand (for example a relay service's address and key),
//! format detection for them, and which thinking control each source can honour.
//!
//! SPDX-License-Identifier: MIT

use crate::discover::{self, AuthKind, KeyRef, Route, Source};
use crate::keystore::Store;
use crate::settings::{self, AppError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// Anthropic Messages API: Claude Code talks to it directly
    Anthropic,
    /// Chat Completions: goes through the built-in translator
    Openai,
}

/// One saved entry. The key itself is in the keychain (or the fallback file), never here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManualEntry {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub format: Format,
    pub model: String,
    pub key_store: Store,
    pub key_masked: String,
}

/// What the form sends. `key` may be empty when editing (keep the saved one).
#[derive(Debug, Clone, Deserialize)]
pub struct ManualInput {
    pub name: String,
    pub base_url: String,
    pub format: Format,
    pub model: String,
    #[serde(default)]
    pub key: String,
}

pub fn new_id() -> String {
    let r: u64 = rand::random();
    format!("manual:{:012x}", r & 0xffff_ffff_ffff)
}

/// Tidy an address for its format. Claude Code appends /v1/messages itself, so an Anthropic
/// address must not end in /v1; a Chat Completions address is used as typed (the app appends
/// /chat/completions) but never with the endpoint path pasted on.
pub fn normalize_base(raw: &str, format: Format) -> String {
    let mut b = raw.trim().trim_end_matches('/').to_string();
    for suffix in ["/chat/completions", "/v1/messages", "/messages"] {
        if b.to_lowercase().ends_with(suffix) {
            b.truncate(b.len() - suffix.len());
            b = b.trim_end_matches('/').to_string();
        }
    }
    if format == Format::Anthropic && b.to_lowercase().ends_with("/v1") {
        b.truncate(b.len() - 3);
    }
    b
}

/// Check the form. Returns the cleaned (name, base_url, model).
pub fn validate(input: &ManualInput, key_required: bool) -> Result<(String, String, String), AppError> {
    let name = input.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 60 || name.chars().any(|c| c.is_control()) {
        return Err(AppError::new("bad_name", "the name must be 1 to 60 characters"));
    }
    let base = normalize_base(&input.base_url, input.format);
    settings::validate_base_url(&base)?;
    let model = input.model.trim().to_string();
    settings::validate_model(&model)?;
    let key = input.key.trim();
    if key_required || !key.is_empty() {
        settings::validate_key(key)?;
    }
    Ok((name, base, model))
}

fn is_anthropic_official(base: &str) -> bool {
    url::Url::parse(base).ok().and_then(|u| u.host_str().map(|h| h.eq_ignore_ascii_case("api.anthropic.com"))).unwrap_or(false)
}

/// The source the rest of the app works with.
pub fn to_source(e: &ManualEntry, key_present: bool) -> Source {
    let route = match e.format { Format::Anthropic => Route::Direct, Format::Openai => Route::Translate };
    Source {
        id: e.id.clone(),
        group: "manual".into(),
        name: e.name.clone(),
        origin: format!("manual:{}", if e.key_store == Store::File { "file" } else { "keychain" }),
        status: if key_present { "reachable".into() } else { "needs_key".into() },
        route,
        base_url: e.base_url.clone(),
        auth: if is_anthropic_official(&e.base_url) { AuthKind::ApiKey } else { AuthKind::Bearer },
        key_masked: if key_present { Some(e.key_masked.clone()) } else { None },
        models: vec![e.model.clone()],
        default_model: Some(e.model.clone()),
        endpoints: vec![],
        note: if route == Route::Translate { Some("via_translator".into()) } else { None },
        docs: None,
        key: KeyRef::Stored { id: e.id.clone() },
        list_base: Some(match e.format { Format::Anthropic => format!("{}/v1", e.base_url), Format::Openai => e.base_url.clone() }),
    }
}

/* ------------------------------------------------------------ thinking */

/// How a source can take a thinking level:
/// - `claude_code`: Claude Code itself sends it (direct Anthropic-format sources)
/// - `reasoning_effort`: the translator maps it onto the provider's documented parameter
/// - `none`: nothing documented to map it onto; the level is not sent
pub fn thinking_support(s: &Source) -> &'static str {
    match s.route {
        Route::Direct => "claude_code",
        Route::Translate if reasoning_provider(s).is_some() => "reasoning_effort",
        _ => "none",
    }
}

/// The catalog provider whose documented reasoning parameter applies to a translated source.
pub fn reasoning_provider(s: &Source) -> Option<&'static str> {
    let p = discover::provider_for_url(&s.base_url)?;
    crate::translate::reasoning_effort_for(Some(p.id), settings::Level::Low).map(|_| p.id)
}

/// What a level writes into settings.json for a direct source.
pub fn direct_thinking_env(s: &Source, base: &str, level: settings::Level) -> Vec<(&'static str, String)> {
    let _ = s;
    settings::thinking_env(level, is_anthropic_official(base))
}

/* ------------------------------------------------------------ detection */

/// What probing an address saw. Filled by `probe` (network), judged by `judge_format` (pure).
#[derive(Debug, Default, Clone, Deserialize)]
pub struct Observed {
    /// Status of POST <root>/v1/messages with an empty JSON body
    pub messages_status: Option<u16>,
    pub messages_body: Option<Value>,
    /// The OpenAI-style base that answered POST <base>/chat/completions, and its status
    pub chat_base: Option<String>,
    pub chat_status: Option<u16>,
    pub chat_body: Option<Value>,
    /// GET .../models status and body (whichever answered first)
    pub models_status: Option<u16>,
    pub models_body: Option<Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Detected {
    pub format: Option<Format>,
    /// The address to save for that format
    pub base_url: Option<String>,
    pub models: Vec<String>,
    /// true when an endpoint refused the key (401/403)
    pub key_refused: bool,
}

fn endpoint_exists(st: Option<u16>) -> bool {
    matches!(st, Some(s) if s != 404 && s != 405 && s < 500)
}

fn looks_anthropic(body: &Option<Value>) -> bool {
    match body {
        Some(v) => v.get("type").and_then(|t| t.as_str()) == Some("error") || v.get("error").and_then(|e| e.get("type")).is_some(),
        None => false,
    }
}

fn looks_chat(body: &Option<Value>) -> bool {
    matches!(body, Some(v) if v.get("error").is_some() || v.get("choices").is_some() || v.get("detail").is_some() || v.get("message").is_some())
}

/// Anthropic format wins when both answer: Claude Code then needs no translator.
pub fn judge_format(input: &str, o: &Observed) -> Detected {
    let models = o.models_body.as_ref().map(discover::parse_models).unwrap_or_default();
    let key_refused = [o.messages_status, o.chat_status, o.models_status].iter().any(|s| matches!(s, Some(401) | Some(403)));
    if endpoint_exists(o.messages_status) && looks_anthropic(&o.messages_body) {
        return Detected { format: Some(Format::Anthropic), base_url: Some(normalize_base(input, Format::Anthropic)), models, key_refused };
    }
    if endpoint_exists(o.chat_status) && looks_chat(&o.chat_body) {
        return Detected { format: Some(Format::Openai), base_url: o.chat_base.clone().or_else(|| Some(normalize_base(input, Format::Openai))), models, key_refused };
    }
    Detected { format: None, base_url: None, models, key_refused }
}

/// The OpenAI-style bases worth trying for what the user typed.
pub fn chat_candidates(input: &str) -> Vec<String> {
    let b = normalize_base(input, Format::Openai);
    if b.to_lowercase().ends_with("/v1") { vec![b] } else { vec![b.clone(), format!("{}/v1", b)] }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(8)).connect_timeout(Duration::from_secs(5)).build().unwrap()
}

fn with_key(r: reqwest::RequestBuilder, key: Option<&str>) -> reqwest::RequestBuilder {
    match key {
        Some(k) if !k.is_empty() => r.header("x-api-key", k).bearer_auth(k),
        _ => r,
    }
}

/// Probe an address the user typed. Sends the key only to that address.
pub async fn probe(input: &str, key: Option<&str>) -> Result<Observed, AppError> {
    let root = normalize_base(input, Format::Anthropic);
    settings::validate_base_url(&root)?;
    let c = client();
    let mut o = Observed::default();
    let r = with_key(c.post(format!("{}/v1/messages", root)), key).header("anthropic-version", "2023-06-01").header("content-type", "application/json").body("{}").send().await;
    if let Ok(r) = r {
        o.messages_status = Some(r.status().as_u16());
        o.messages_body = r.json::<Value>().await.ok();
    }
    for b in chat_candidates(input) {
        if let Ok(r) = with_key(c.post(format!("{}/chat/completions", b)), key).header("content-type", "application/json").body("{}").send().await {
            let st = r.status().as_u16();
            let body = r.json::<Value>().await.ok();
            if endpoint_exists(Some(st)) && looks_chat(&body) {
                o.chat_base = Some(b);
                o.chat_status = Some(st);
                o.chat_body = body;
                break;
            }
            o.chat_status.get_or_insert(st);
        }
    }
    let mut model_urls = vec![format!("{}/v1/models", root)];
    if let Some(b) = &o.chat_base {
        model_urls.insert(0, format!("{}/models", b));
    }
    for u in model_urls {
        if let Ok(r) = with_key(c.get(&u), key).header("anthropic-version", "2023-06-01").send().await {
            let st = r.status().as_u16();
            o.models_status.get_or_insert(st);
            if r.status().is_success() {
                o.models_status = Some(st);
                o.models_body = r.json::<Value>().await.ok();
                break;
            }
        }
    }
    if o.messages_status.is_none() && o.chat_status.is_none() && o.models_status.is_none() {
        return Err(AppError::new("network", "could not connect"));
    }
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(base: &str, format: Format) -> ManualInput {
        ManualInput { name: "My relay".into(), base_url: base.into(), format, model: "claude-sonnet-5".into(), key: "sk-relay-0123456789abcdef".into() }
    }

    #[test]
    fn manual_entry_validation() {
        let ok = validate(&input("https://relay.example.com/v1/", Format::Anthropic), true).unwrap();
        assert_eq!(ok.1, "https://relay.example.com");
        assert_eq!(validate(&input("https://relay.example.com/v1/chat/completions", Format::Openai), true).unwrap().1, "https://relay.example.com/v1");
        assert_eq!(validate(&input("https://relay.example.com/v1/messages", Format::Anthropic), true).unwrap().1, "https://relay.example.com");
        for bad in ["relay.example.com", "ftp://x.com", "https://u:p@x.com", "https://x.com/?a=1", "https://x.com/#f", ""] {
            assert_eq!(validate(&input(bad, Format::Anthropic), true).unwrap_err().code, "bad_url", "{}", bad);
        }
        let mut i = input("https://x.com", Format::Anthropic);
        i.name = "  ".into();
        assert_eq!(validate(&i, true).unwrap_err().code, "bad_name");
        i.name = "x".repeat(61);
        assert_eq!(validate(&i, true).unwrap_err().code, "bad_name");
        let mut i = input("https://x.com", Format::Anthropic);
        i.model = "bad model".into();
        assert_eq!(validate(&i, true).unwrap_err().code, "bad_model");
        let mut i = input("https://x.com", Format::Anthropic);
        i.key = "".into();
        assert_eq!(validate(&i, true).unwrap_err().code, "no_key");
        assert!(validate(&i, false).is_ok()); // editing: keep the saved key
        i.key = "has space".into();
        assert_eq!(validate(&i, false).unwrap_err().code, "bad_key");
    }

    #[test]
    fn manual_source_shape() {
        let e = ManualEntry { id: "manual:1".into(), name: "R".into(), base_url: "https://relay.example.com".into(), format: Format::Anthropic, model: "m".into(), key_store: Store::File, key_masked: "…cdef".into() };
        let s = to_source(&e, true);
        assert_eq!((s.group.as_str(), s.route, s.auth, s.status.as_str()), ("manual", Route::Direct, AuthKind::Bearer, "reachable"));
        assert_eq!(s.origin, "manual:file");
        assert_eq!(thinking_support(&s), "claude_code");
        let o = to_source(&ManualEntry { format: Format::Openai, base_url: "https://api.openai.com/v1".into(), ..e.clone() }, false);
        assert_eq!((o.route, o.status.as_str()), (Route::Translate, "needs_key"));
        assert_eq!(thinking_support(&o), "reasoning_effort");
        let unknown = to_source(&ManualEntry { format: Format::Openai, base_url: "https://relay.example.com/v1".into(), ..e.clone() }, true);
        assert_eq!(thinking_support(&unknown), "none");
        let official = to_source(&ManualEntry { base_url: "https://api.anthropic.com".into(), ..e }, true);
        assert_eq!(official.auth, AuthKind::ApiKey);
    }

    fn fixture(name: &str) -> Observed {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/detect").join(name);
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn format_detection_with_fixtures() {
        let d = judge_format("https://relay.example.com/v1", &fixture("both.json"));
        assert_eq!(d.format, Some(Format::Anthropic));
        assert_eq!(d.base_url.as_deref(), Some("https://relay.example.com"));
        assert_eq!(d.models, vec!["claude-sonnet-5", "gpt-5.2"]);
        let d = judge_format("https://api.example.cn/compatible-mode", &fixture("openai_only.json"));
        assert_eq!(d.format, Some(Format::Openai));
        assert_eq!(d.base_url.as_deref(), Some("https://api.example.cn/compatible-mode/v1"));
        assert_eq!(d.models, vec!["qwen-max"]);
        let d = judge_format("https://relay.example.com", &fixture("key_refused.json"));
        assert_eq!(d.format, Some(Format::Anthropic));
        assert!(d.key_refused);
        let d = judge_format("https://example.com", &fixture("neither.json"));
        assert_eq!(d.format, None);
        // a web page answering 200 HTML to POST /v1/messages is not an Anthropic endpoint
        let d = judge_format("https://example.com", &fixture("html_site.json"));
        assert_eq!(d.format, None);
    }

    #[test]
    fn chat_candidate_bases() {
        assert_eq!(chat_candidates("https://x.com/v1/"), vec!["https://x.com/v1"]);
        assert_eq!(chat_candidates("https://x.com/api/paas/v4"), vec!["https://x.com/api/paas/v4", "https://x.com/api/paas/v4/v1"]);
    }

    #[tokio::test]
    async fn probe_against_a_fake_relay() {
        use axum::{routing::{get, post}, Json, Router, http::StatusCode};
        let app = Router::new()
            .route("/v1/messages", post(|| async { (StatusCode::BAD_REQUEST, Json(serde_json::json!({"type": "error", "error": {"type": "invalid_request_error", "message": "messages: field required"}}))) }))
            .route("/v1/models", get(|| async { Json(serde_json::json!({"data": [{"id": "relay-model"}]})) }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let input = format!("http://127.0.0.1:{}/v1", port);
        let o = probe(&input, Some("sk-x")).await.unwrap();
        let d = judge_format(&input, &o);
        assert_eq!(d.format, Some(Format::Anthropic));
        assert_eq!(d.base_url, Some(format!("http://127.0.0.1:{}", port)));
        assert_eq!(d.models, vec!["relay-model"]);
    }
}
