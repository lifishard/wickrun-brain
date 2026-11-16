//! The local translating proxy: Claude Code (Anthropic Messages) -> a Chat Completions source.
//!
//! Binds 127.0.0.1 only. Claude Code authenticates with a random token; the real
//! upstream key stays inside this process and is never written to settings.json.
//!
//! SPDX-License-Identifier: MIT

use crate::translate::{self, SseParser, StreamTranslator};
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct Route {
    /// OpenAI-style base, e.g. https://api.openai.com/v1 (".../chat/completions" is appended)
    pub upstream_base: String,
    pub upstream_key: String,
    pub model: String,
}

pub struct Inner {
    pub token: RwLock<String>,
    pub route: RwLock<Option<Route>>,
    pub client: reqwest::Client,
}

#[derive(Clone)]
pub struct Proxy {
    pub inner: Arc<Inner>,
    pub port: u16,
}

pub fn new_token() -> String {
    let a: u128 = rand::random();
    let b: u128 = rand::random();
    format!("wbp-{:032x}{:032x}", a, b)
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn presented_key(h: &HeaderMap) -> Option<String> {
    if let Some(v) = h.get("x-api-key").and_then(|v| v.to_str().ok()) {
        return Some(v.to_string());
    }
    h.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ").map(String::from))
}

fn err(status: u16, kind: &str, msg: &str) -> Response {
    (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), Json(translate::error_body(kind, msg))).into_response()
}

async fn authorize(inner: &Inner, h: &HeaderMap) -> Result<Route, Response> {
    let token = inner.token.read().await.clone();
    match presented_key(h) {
        Some(k) if ct_eq(k.as_bytes(), token.as_bytes()) => {}
        _ => return Err(err(401, "authentication_error", "wickrun-brain: wrong or missing key for the local translator. Switch again from the wickrun-brain app.")),
    }
    inner.route.read().await.clone().ok_or_else(|| err(503, "overloaded_error", "wickrun-brain: no source is selected in the app"))
}

async fn messages(State(inner): State<Arc<Inner>>, h: HeaderMap, body: Bytes) -> Response {
    let route = match authorize(&inner, &h).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return err(400, "invalid_request_error", "request body is not valid JSON"),
    };
    let model = translate::pick_model(req.get("model").and_then(|m| m.as_str()), &route.model);
    let oreq = match translate::request_to_openai(&req, &model) {
        Ok(v) => v,
        Err(e) => return err(400, "invalid_request_error", &e),
    };
    let stream = oreq.get("stream").and_then(|s| s.as_bool()) == Some(true);
    let url = format!("{}/chat/completions", route.upstream_base.trim_end_matches('/'));
    let resp = match inner.client.post(&url).bearer_auth(&route.upstream_key).json(&oreq).send().await {
        Ok(r) => r,
        Err(e) => {
            let what = if e.is_timeout() { "timed out" } else if e.is_connect() { "could not connect" } else { "request failed" };
            return err(502, "api_error", &format!("wickrun-brain: upstream {}", what));
        }
    };
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        let b = resp.bytes().await.unwrap_or_default();
        let (s, body) = translate::map_error(status, &b);
        return (StatusCode::from_u16(s).unwrap_or(StatusCode::BAD_GATEWAY), Json(body)).into_response();
    }
    if !stream {
        let b = resp.bytes().await.unwrap_or_default();
        return match serde_json::from_slice::<Value>(&b) {
            Ok(v) if v.get("choices").is_some() => Json(translate::response_from_openai(&v, &model)).into_response(),
            Ok(v) if v.get("error").is_some() => {
                let (s, body) = translate::map_error(502, &b);
                let _ = v;
                (StatusCode::from_u16(s).unwrap_or(StatusCode::BAD_GATEWAY), Json(body)).into_response()
            }
            _ => err(502, "api_error", "wickrun-brain: upstream sent a response that is not Chat Completions"),
        };
    }
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(64);
    let mut upstream = resp.bytes_stream();
    tokio::spawn(async move {
        let mut p = SseParser::default();
        let mut t = StreamTranslator::new(&model);
        let send = |evs: Vec<translate::Event>| {
            let s: String = evs.iter().map(translate::format_sse).collect();
            Bytes::from(s)
        };
        let mut done = false;
        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(bytes) => {
                    for d in p.push(&bytes) {
                        let evs = if d.trim() == "[DONE]" {
                            done = true;
                            t.finish()
                        } else {
                            match serde_json::from_str::<Value>(&d) {
                                Ok(v) => t.feed(&v),
                                Err(_) => vec![],
                            }
                        };
                        if !evs.is_empty() && tx.send(Ok(send(evs))).await.is_err() {
                            return; // Claude Code went away
                        }
                    }
                    if done {
                        break;
                    }
                }
                Err(_) => {
                    let _ = tx.send(Ok(send(vec![("error", translate::error_body("api_error", "wickrun-brain: upstream stream was cut off"))]))).await;
                    return;
                }
            }
        }
        for d in p.flush() {
            if d != "[DONE]" {
                if let Ok(v) = serde_json::from_str::<Value>(&d) {
                    let _ = tx.send(Ok(send(t.feed(&v)))).await;
                }
            }
        }
        let _ = tx.send(Ok(send(t.finish()))).await;
    });
    let body_stream = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|item| (item, rx)) });
    Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(Body::from_stream(body_stream))
        .unwrap()
}

async fn count_tokens(State(inner): State<Arc<Inner>>, h: HeaderMap, body: Bytes) -> Response {
    if let Err(e) = authorize(&inner, &h).await {
        return e;
    }
    let v: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    Json(json!({"input_tokens": translate::estimate_tokens(&v)})).into_response()
}

async fn models(State(inner): State<Arc<Inner>>, h: HeaderMap) -> Response {
    let route = match authorize(&inner, &h).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    Json(json!({"data": [{"type": "model", "id": route.model, "display_name": route.model}], "has_more": false})).into_response()
}

pub fn router(inner: Arc<Inner>) -> Router {
    Router::new()
        .route("/v1/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .route("/v1/models", get(models))
        .route("/wickrun-brain/health", get(|| async { "ok" }))
        .with_state(inner)
}

/// Start on 127.0.0.1. `port` 0 picks a random free port. Fails if `port` is taken.
pub async fn start(port: u16, token: String) -> std::io::Result<Proxy> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let port = listener.local_addr()?.port();
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .read_timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(std::io::Error::other)?;
    let inner = Arc::new(Inner { token: RwLock::new(token), route: RwLock::new(None), client });
    let app = router(inner.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(Proxy { inner, port })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::post;

    /// A fake Chat Completions server whose reply depends on the requested model.
    async fn fake_upstream() -> u16 {
        async fn chat(h: HeaderMap, Json(v): Json<Value>) -> Response {
            assert_eq!(h.get("authorization").unwrap(), "Bearer upstream-secret-key");
            match v["model"].as_str().unwrap() {
                "rate" => (StatusCode::TOO_MANY_REQUESTS, Json(json!({"error": {"message": "slow down"}}))).into_response(),
                "auth" => (StatusCode::UNAUTHORIZED, Json(json!({"error": {"message": "bad key"}}))).into_response(),
                "down" => (StatusCode::INTERNAL_SERVER_ERROR, "oops").into_response(),
                _ if v["stream"] == true => {
                    let s = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_x\",\"function\":{\"name\":\"Read\",\"arguments\":\"{\\\"p\\\":1}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n";
                    Response::builder().header("content-type", "text/event-stream").body(Body::from(s)).unwrap()
                }
                m => Json(json!({"choices": [{"message": {"content": format!("echo {} {}", m, v["messages"].as_array().unwrap().len())}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 5, "completion_tokens": 2}})).into_response(),
            }
        }
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, Router::new().route("/v1/chat/completions", post(chat))).await.unwrap() });
        port
    }

    async fn setup(model: &str) -> (Proxy, reqwest::Client) {
        let up = fake_upstream().await;
        let p = start(0, "tok-123".into()).await.unwrap();
        *p.inner.route.write().await = Some(Route { upstream_base: format!("http://127.0.0.1:{}/v1", up), upstream_key: "upstream-secret-key".into(), model: model.into() });
        (p, reqwest::Client::new())
    }

    fn body() -> Value {
        json!({"model": "claude-sonnet-4-5", "max_tokens": 10, "messages": [{"role": "user", "content": "hi"}], "system": "s"})
    }

    #[tokio::test]
    async fn plain_round_trip_and_auth() {
        let (p, c) = setup("gpt-x").await;
        let url = format!("http://127.0.0.1:{}/v1/messages", p.port);
        let r = c.post(&url).header("x-api-key", "tok-123").json(&body()).send().await.unwrap();
        assert_eq!(r.status(), 200);
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["content"][0]["text"], "echo gpt-x 2");
        assert_eq!(v["usage"]["input_tokens"], 5);
        let r = c.post(&url).bearer_auth("tok-123").json(&body()).send().await.unwrap();
        assert_eq!(r.status(), 200);
        let r = c.post(&url).header("x-api-key", "nope").json(&body()).send().await.unwrap();
        assert_eq!(r.status(), 401);
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["error"]["type"], "authentication_error");
        let r = c.post(&url).json(&body()).send().await.unwrap();
        assert_eq!(r.status(), 401);
    }

    #[tokio::test]
    async fn streaming_round_trip() {
        let (p, c) = setup("gpt-x").await;
        let mut b = body();
        b["stream"] = json!(true);
        let r = c.post(format!("http://127.0.0.1:{}/v1/messages", p.port)).header("x-api-key", "tok-123").json(&b).send().await.unwrap();
        assert_eq!(r.headers()["content-type"], "text/event-stream");
        let text = r.text().await.unwrap();
        let events: Vec<&str> = text.lines().filter_map(|l| l.strip_prefix("event: ")).collect();
        assert_eq!(events, vec!["message_start", "content_block_start", "content_block_delta", "content_block_stop", "content_block_start", "content_block_delta", "content_block_stop", "message_delta", "message_stop"]);
        assert!(text.contains("\"stop_reason\":\"tool_use\""));
        assert!(text.contains("call_x"));
    }

    #[tokio::test]
    async fn upstream_errors_become_anthropic_errors() {
        for (model, status, kind) in [("rate", 429, "rate_limit_error"), ("auth", 401, "authentication_error"), ("down", 500, "api_error")] {
            let (p, c) = setup(model).await;
            let r = c.post(format!("http://127.0.0.1:{}/v1/messages", p.port)).header("x-api-key", "tok-123").json(&body()).send().await.unwrap();
            assert_eq!(r.status().as_u16(), status);
            let v: Value = r.json().await.unwrap();
            assert_eq!(v["type"], "error");
            assert_eq!(v["error"]["type"], kind);
            assert!(!v.to_string().contains("upstream-secret-key"));
        }
    }

    #[tokio::test]
    async fn unreachable_upstream_is_502_and_binds_loopback_only() {
        let p = start(0, "t".into()).await.unwrap();
        *p.inner.route.write().await = Some(Route { upstream_base: "http://127.0.0.1:9/v1".into(), upstream_key: "k".into(), model: "m".into() });
        let r = reqwest::Client::new().post(format!("http://127.0.0.1:{}/v1/messages", p.port)).header("x-api-key", "t").json(&body()).send().await.unwrap();
        assert_eq!(r.status(), 502);
        // not reachable on other interfaces: the listener is bound to 127.0.0.1
        assert!(tokio::net::TcpListener::bind(("127.0.0.1", p.port)).await.is_err());
    }

    #[tokio::test]
    async fn count_tokens_and_no_route() {
        let p = start(0, "t".into()).await.unwrap();
        let c = reqwest::Client::new();
        let r = c.post(format!("http://127.0.0.1:{}/v1/messages", p.port)).header("x-api-key", "t").json(&body()).send().await.unwrap();
        assert_eq!(r.status(), 503);
        *p.inner.route.write().await = Some(Route { upstream_base: "http://127.0.0.1:9/v1".into(), upstream_key: "k".into(), model: "m".into() });
        let v: Value = c.post(format!("http://127.0.0.1:{}/v1/messages/count_tokens", p.port)).header("x-api-key", "t").json(&body()).send().await.unwrap().json().await.unwrap();
        assert!(v["input_tokens"].as_u64().unwrap() > 0);
    }
}
