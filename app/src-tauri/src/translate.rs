//! Anthropic Messages <-> OpenAI Chat Completions translation (pure functions).
//!
//! Used by the local proxy so that sources which only speak Chat Completions can
//! still be Claude Code's brain. Written fresh for wickrun-brain.
//!
//! SPDX-License-Identifier: MIT

use serde_json::{json, Map, Value};

fn text_of_blocks(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(|b| match b.get("type").and_then(|t| t.as_str()) {
                Some("text") => b.get("text").and_then(|t| t.as_str()).map(String::from),
                Some("image") => Some("[image omitted]".into()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

fn image_part(b: &Value) -> Option<Value> {
    let src = b.get("source")?;
    match src.get("type").and_then(|t| t.as_str()) {
        Some("base64") => {
            let mt = src.get("media_type").and_then(|t| t.as_str()).unwrap_or("image/png");
            let data = src.get("data").and_then(|t| t.as_str())?;
            Some(json!({"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", mt, data)}}))
        }
        Some("url") => Some(json!({"type": "image_url", "image_url": {"url": src.get("url")?}})),
        _ => None,
    }
}

/// Pick the upstream model: Claude Code's own default names (claude-*) go to the chosen route model.
pub fn pick_model(requested: Option<&str>, route_model: &str) -> String {
    match requested {
        Some(m) if !m.is_empty() && !m.starts_with("claude-") && !m.starts_with("anthropic/") => m.to_string(),
        _ => route_model.to_string(),
    }
}

/// Build a Chat Completions request body from an Anthropic Messages request body.
pub fn request_to_openai(req: &Value, model: &str) -> Result<Value, String> {
    let obj = req.as_object().ok_or("request body must be a JSON object")?;
    let mut messages: Vec<Value> = Vec::new();
    if let Some(sys) = obj.get("system") {
        let s = text_of_blocks(sys);
        if !s.is_empty() {
            messages.push(json!({"role": "system", "content": s}));
        }
    }
    let msgs = obj.get("messages").and_then(|m| m.as_array()).ok_or("\"messages\" is required")?;
    for m in msgs {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let content = m.get("content").cloned().unwrap_or(Value::Null);
        if let Value::String(s) = &content {
            messages.push(json!({"role": role, "content": s}));
            continue;
        }
        let blocks = content.as_array().cloned().unwrap_or_default();
        if role == "assistant" {
            let mut text = Vec::new();
            let mut calls = Vec::new();
            for b in &blocks {
                match b.get("type").and_then(|t| t.as_str()) {
                    Some("text") => text.push(b.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string()),
                    Some("tool_use") => calls.push(json!({
                        "id": b.get("id").cloned().unwrap_or(json!("")),
                        "type": "function",
                        "function": {
                            "name": b.get("name").cloned().unwrap_or(json!("")),
                            "arguments": serde_json::to_string(b.get("input").unwrap_or(&json!({}))).unwrap_or_else(|_| "{}".into()),
                        }
                    })),
                    // thinking / redacted_thinking: dropped (signatures cannot be reproduced upstream)
                    _ => {}
                }
            }
            let joined = text.join("");
            let mut out = Map::new();
            out.insert("role".into(), json!("assistant"));
            out.insert("content".into(), if joined.is_empty() && !calls.is_empty() { Value::Null } else { json!(joined) });
            if !calls.is_empty() {
                out.insert("tool_calls".into(), Value::Array(calls));
            }
            messages.push(Value::Object(out));
        } else {
            let mut parts: Vec<Value> = Vec::new();
            for b in &blocks {
                match b.get("type").and_then(|t| t.as_str()) {
                    Some("tool_result") => {
                        let mut t = text_of_blocks(b.get("content").unwrap_or(&Value::Null));
                        if b.get("is_error").and_then(|e| e.as_bool()) == Some(true) {
                            t = format!("Error: {}", t);
                        }
                        messages.push(json!({"role": "tool", "tool_call_id": b.get("tool_use_id").cloned().unwrap_or(json!("")), "content": t}));
                    }
                    Some("text") => parts.push(json!({"type": "text", "text": b.get("text").cloned().unwrap_or(json!(""))})),
                    Some("image") => {
                        if let Some(p) = image_part(b) {
                            parts.push(p)
                        }
                    }
                    Some("document") => parts.push(json!({"type": "text", "text": "[document omitted]"})),
                    _ => {}
                }
            }
            if !parts.is_empty() {
                let all_text = parts.iter().all(|p| p["type"] == "text");
                let c = if all_text {
                    json!(parts.iter().map(|p| p["text"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>().join("\n\n"))
                } else {
                    Value::Array(parts)
                };
                messages.push(json!({"role": "user", "content": c}));
            }
        }
    }
    let mut out = Map::new();
    out.insert("model".into(), json!(model));
    out.insert("messages".into(), Value::Array(messages));
    if let Some(v) = obj.get("max_tokens") {
        out.insert("max_tokens".into(), v.clone());
    }
    for k in ["temperature", "top_p"] {
        if let Some(v) = obj.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    if let Some(v) = obj.get("stop_sequences") {
        out.insert("stop".into(), v.clone());
    }
    if let Some(tools) = obj.get("tools").and_then(|t| t.as_array()) {
        let fns: Vec<Value> = tools
            .iter()
            // server tools (web_search_..., bash_..., text_editor_...) have a "type" and no input_schema
            .filter(|t| t.get("input_schema").is_some())
            .map(|t| {
                json!({"type": "function", "function": {
                    "name": t.get("name").cloned().unwrap_or(json!("")),
                    "description": t.get("description").cloned().unwrap_or(json!("")),
                    "parameters": t.get("input_schema").cloned().unwrap_or(json!({"type": "object"})),
                }})
            })
            .collect();
        if !fns.is_empty() {
            out.insert("tools".into(), Value::Array(fns));
            if let Some(tc) = obj.get("tool_choice") {
                match tc.get("type").and_then(|t| t.as_str()) {
                    Some("auto") => { out.insert("tool_choice".into(), json!("auto")); }
                    Some("any") => { out.insert("tool_choice".into(), json!("required")); }
                    Some("none") => { out.insert("tool_choice".into(), json!("none")); }
                    Some("tool") => { out.insert("tool_choice".into(), json!({"type": "function", "function": {"name": tc.get("name").cloned().unwrap_or(json!(""))}})); }
                    _ => {}
                }
                if tc.get("disable_parallel_tool_use").and_then(|b| b.as_bool()) == Some(true) {
                    out.insert("parallel_tool_calls".into(), json!(false));
                }
            }
        }
    }
    if obj.get("stream").and_then(|s| s.as_bool()) == Some(true) {
        out.insert("stream".into(), json!(true));
        out.insert("stream_options".into(), json!({"include_usage": true}));
    }
    Ok(Value::Object(out))
}

pub fn map_finish_reason(r: Option<&str>, has_tool_calls: bool) -> &'static str {
    match r {
        Some("length") => "max_tokens",
        Some("tool_calls") | Some("function_call") => "tool_use",
        _ if has_tool_calls => "tool_use",
        _ => "end_turn",
    }
}

fn usage_of(u: Option<&Value>) -> (u64, u64, u64) {
    let g = |k: &str| u.and_then(|u| u.get(k)).and_then(|v| v.as_u64()).unwrap_or(0);
    let cached = u.and_then(|u| u.get("prompt_tokens_details")).and_then(|d| d.get("cached_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);
    (g("prompt_tokens"), g("completion_tokens"), cached)
}

fn parse_args(s: &str) -> Value {
    if s.trim().is_empty() {
        return json!({});
    }
    match serde_json::from_str::<Value>(s) {
        Ok(v @ Value::Object(_)) => v,
        _ => json!({}),
    }
}

pub fn new_message_id() -> String {
    format!("msg_wb{:016x}", rand::random::<u64>())
}

/// Translate a non-streaming Chat Completions response into an Anthropic message.
pub fn response_from_openai(resp: &Value, model: &str) -> Value {
    let choice = resp.get("choices").and_then(|c| c.get(0)).cloned().unwrap_or(json!({}));
    let msg = choice.get("message").cloned().unwrap_or(json!({}));
    let mut content = Vec::new();
    if let Some(t) = msg.get("content").and_then(|c| c.as_str()) {
        if !t.is_empty() {
            content.push(json!({"type": "text", "text": t}));
        }
    }
    let calls = msg.get("tool_calls").and_then(|c| c.as_array()).cloned().unwrap_or_default();
    for c in &calls {
        let f = c.get("function").cloned().unwrap_or(json!({}));
        content.push(json!({
            "type": "tool_use",
            "id": c.get("id").and_then(|i| i.as_str()).map(String::from).unwrap_or_else(|| format!("toolu_wb{:012x}", rand::random::<u64>() >> 16)),
            "name": f.get("name").cloned().unwrap_or(json!("")),
            "input": parse_args(f.get("arguments").and_then(|a| a.as_str()).unwrap_or("")),
        }));
    }
    let (i, o, cached) = usage_of(resp.get("usage"));
    let mut usage = json!({"input_tokens": i.saturating_sub(cached), "output_tokens": o});
    if cached > 0 {
        usage["cache_read_input_tokens"] = json!(cached);
    }
    json!({
        "id": new_message_id(),
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content,
        "stop_reason": map_finish_reason(choice.get("finish_reason").and_then(|r| r.as_str()), !calls.is_empty()),
        "stop_sequence": null,
        "usage": usage,
    })
}

/// Map an upstream HTTP error to Anthropic's error shape. Returns (status, body).
pub fn map_error(status: u16, body: &[u8]) -> (u16, Value) {
    let parsed: Option<Value> = serde_json::from_slice(body).ok();
    let msg = parsed
        .as_ref()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message").and_then(|m| m.as_str()).map(String::from).or_else(|| e.as_str().map(String::from)))
                .or_else(|| v.get("message").and_then(|m| m.as_str()).map(String::from))
        })
        .unwrap_or_else(|| String::from_utf8_lossy(&body[..body.len().min(500)]).trim().to_string());
    let kind = match status {
        400 | 422 => "invalid_request_error",
        401 => "authentication_error",
        402 => "billing_error",
        403 => "permission_error",
        404 => "not_found_error",
        413 => "request_too_large",
        429 => "rate_limit_error",
        503 | 529 => "overloaded_error",
        _ => "api_error",
    };
    let out_status = match status {
        422 => 400,
        s if (400..600).contains(&s) => s,
        _ => 502,
    };
    let message = if msg.is_empty() { format!("upstream returned HTTP {}", status) } else { format!("upstream: {}", msg) };
    (out_status, json!({"type": "error", "error": {"type": kind, "message": message}}))
}

pub fn error_body(kind: &str, message: &str) -> Value {
    json!({"type": "error", "error": {"type": kind, "message": message}})
}

/// Rough token estimate for /v1/messages/count_tokens (Chat Completions has no equivalent).
pub fn estimate_tokens(req: &Value) -> u64 {
    let s = req.to_string();
    (s.chars().count() as u64 / 4).max(1)
}

/* ------------------------------------------------------------ streaming */

#[derive(Default)]
struct ToolBuf {
    id: String,
    name: String,
    args: String,
}

/// Turns Chat Completions stream chunks into Anthropic SSE events.
///
/// Text streams live. Tool calls are buffered and emitted whole at the end of the
/// turn, because Chat Completions servers may interleave argument fragments of
/// several calls, while Anthropic content blocks must be sequential.
pub struct StreamTranslator {
    model: String,
    started: bool,
    finished: bool,
    next_index: usize,
    text_open: Option<usize>,
    tools: Vec<(u64, ToolBuf)>,
    finish_reason: Option<String>,
    usage: (u64, u64, u64),
}

pub type Event = (&'static str, Value);

impl StreamTranslator {
    pub fn new(model: &str) -> Self {
        StreamTranslator { model: model.into(), started: false, finished: false, next_index: 0, text_open: None, tools: vec![], finish_reason: None, usage: (0, 0, 0) }
    }

    fn start(&mut self, out: &mut Vec<Event>) {
        if !self.started {
            self.started = true;
            out.push(("message_start", json!({"type": "message_start", "message": {
                "id": new_message_id(), "type": "message", "role": "assistant", "model": self.model,
                "content": [], "stop_reason": null, "stop_sequence": null,
                "usage": {"input_tokens": 0, "output_tokens": 0}
            }})));
        }
    }

    fn close_text(&mut self, out: &mut Vec<Event>) {
        if let Some(i) = self.text_open.take() {
            out.push(("content_block_stop", json!({"type": "content_block_stop", "index": i})));
        }
    }

    /// Feed one parsed `data:` JSON chunk.
    pub fn feed(&mut self, chunk: &Value) -> Vec<Event> {
        let mut out = Vec::new();
        if self.finished {
            return out;
        }
        if let Some(err) = chunk.get("error") {
            self.start(&mut out);
            let m = err.get("message").and_then(|m| m.as_str()).unwrap_or("upstream error").to_string();
            out.push(("error", error_body("api_error", &format!("upstream: {}", m))));
            self.finished = true;
            return out;
        }
        self.start(&mut out);
        if let Some(u) = chunk.get("usage").filter(|u| !u.is_null()) {
            self.usage = usage_of(Some(u));
        }
        let Some(choice) = chunk.get("choices").and_then(|c| c.get(0)) else { return out };
        let delta = choice.get("delta").cloned().unwrap_or(json!({}));
        if let Some(t) = delta.get("content").and_then(|c| c.as_str()) {
            if !t.is_empty() {
                if self.text_open.is_none() {
                    let i = self.next_index;
                    self.next_index += 1;
                    self.text_open = Some(i);
                    out.push(("content_block_start", json!({"type": "content_block_start", "index": i, "content_block": {"type": "text", "text": ""}})));
                }
                out.push(("content_block_delta", json!({"type": "content_block_delta", "index": self.text_open.unwrap(), "delta": {"type": "text_delta", "text": t}})));
            }
        }
        // reasoning_content / reasoning (DeepSeek, Qwen, vLLM ...) is dropped on purpose.
        if let Some(calls) = delta.get("tool_calls").and_then(|c| c.as_array()) {
            for c in calls {
                let idx = c.get("index").and_then(|i| i.as_u64()).unwrap_or(self.tools.len() as u64);
                let pos = match self.tools.iter().position(|(i, _)| *i == idx) {
                    Some(p) => p,
                    None => {
                        self.tools.push((idx, ToolBuf::default()));
                        self.tools.len() - 1
                    }
                };
                let tb = &mut self.tools[pos].1;
                if let Some(id) = c.get("id").and_then(|v| v.as_str()) {
                    if !id.is_empty() {
                        tb.id = id.into();
                    }
                }
                if let Some(f) = c.get("function") {
                    if let Some(n) = f.get("name").and_then(|v| v.as_str()) {
                        tb.name.push_str(n);
                    }
                    if let Some(a) = f.get("arguments").and_then(|v| v.as_str()) {
                        tb.args.push_str(a);
                    }
                }
            }
        }
        if let Some(r) = choice.get("finish_reason").and_then(|r| r.as_str()) {
            self.finish_reason = Some(r.into());
        }
        out
    }

    /// Call at `[DONE]` or end of the upstream stream.
    pub fn finish(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        if self.finished {
            return out;
        }
        self.finished = true;
        self.start(&mut out);
        self.close_text(&mut out);
        let tools = std::mem::take(&mut self.tools);
        let has_tools = !tools.is_empty();
        for (_, tb) in tools {
            let i = self.next_index;
            self.next_index += 1;
            let id = if tb.id.is_empty() { format!("toolu_wb{:012x}", rand::random::<u64>() >> 16) } else { tb.id };
            out.push(("content_block_start", json!({"type": "content_block_start", "index": i, "content_block": {"type": "tool_use", "id": id, "name": tb.name, "input": {}}})));
            let args = parse_args(&tb.args);
            out.push(("content_block_delta", json!({"type": "content_block_delta", "index": i, "delta": {"type": "input_json_delta", "partial_json": args.to_string()}})));
            out.push(("content_block_stop", json!({"type": "content_block_stop", "index": i})));
        }
        let (inp, outp, cached) = self.usage;
        let mut usage = json!({"input_tokens": inp.saturating_sub(cached), "output_tokens": outp});
        if cached > 0 {
            usage["cache_read_input_tokens"] = json!(cached);
        }
        out.push(("message_delta", json!({"type": "message_delta", "delta": {"stop_reason": map_finish_reason(self.finish_reason.as_deref(), has_tools), "stop_sequence": null}, "usage": usage})));
        out.push(("message_stop", json!({"type": "message_stop"})));
        out
    }
}

/// Incremental SSE parser: push bytes, get back complete `data:` payloads.
#[derive(Default)]
pub struct SseParser {
    buf: String,
}
impl SseParser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.push_str(&String::from_utf8_lossy(bytes));
        if self.buf.contains('\r') {
            self.buf = self.buf.replace("\r\n", "\n");
        }
        let mut out = Vec::new();
        while let Some(pos) = self.buf.find("\n\n") {
            let ev: String = self.buf[..pos].to_string();
            self.buf.drain(..pos + 2);
            let data: Vec<&str> = ev.lines().filter_map(|l| l.strip_prefix("data:")).map(|l| l.strip_prefix(' ').unwrap_or(l)).collect();
            if !data.is_empty() {
                out.push(data.join("\n"));
            }
        }
        out
    }
    pub fn flush(&mut self) -> Vec<String> {
        let rest = std::mem::take(&mut self.buf);
        rest.lines().filter_map(|l| l.strip_prefix("data:")).map(|l| l.trim().to_string()).filter(|s| !s.is_empty()).collect()
    }
}

pub fn format_sse(ev: &Event) -> String {
    format!("event: {}\ndata: {}\n\n", ev.0, ev.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/translate").join(name);
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap()
    }

    #[test]
    fn plain_chat_request() {
        let req = fixture("plain_request.json");
        let out = request_to_openai(&req, "gpt-test").unwrap();
        assert_eq!(out, fixture("plain_request.expected.json"));
    }

    #[test]
    fn tool_round_trip_request() {
        let req = fixture("tools_request.json");
        let out = request_to_openai(&req, "gpt-test").unwrap();
        assert_eq!(out, fixture("tools_request.expected.json"));
    }

    #[test]
    fn plain_response() {
        let r = response_from_openai(&fixture("plain_response.openai.json"), "gpt-test");
        assert_eq!(r["content"], json!([{"type": "text", "text": "Hello there"}]));
        assert_eq!(r["stop_reason"], "end_turn");
        assert_eq!(r["usage"], json!({"input_tokens": 12, "output_tokens": 3}));
        assert_eq!(r["type"], "message");
    }

    #[test]
    fn tool_response() {
        let r = response_from_openai(&fixture("tool_response.openai.json"), "gpt-test");
        assert_eq!(r["stop_reason"], "tool_use");
        assert_eq!(r["content"][0], json!({"type": "text", "text": "Let me look."}));
        assert_eq!(r["content"][1]["type"], "tool_use");
        assert_eq!(r["content"][1]["id"], "call_1");
        assert_eq!(r["content"][1]["name"], "Read");
        assert_eq!(r["content"][1]["input"], json!({"file_path": "/tmp/a.txt"}));
        assert_eq!(r["usage"], json!({"input_tokens": 80, "output_tokens": 20, "cache_read_input_tokens": 20}));
    }

    #[test]
    fn finish_reasons() {
        assert_eq!(map_finish_reason(Some("stop"), false), "end_turn");
        assert_eq!(map_finish_reason(Some("length"), false), "max_tokens");
        assert_eq!(map_finish_reason(Some("tool_calls"), true), "tool_use");
        assert_eq!(map_finish_reason(Some("stop"), true), "tool_use");
        assert_eq!(map_finish_reason(None, false), "end_turn");
    }

    #[test]
    fn model_choice() {
        assert_eq!(pick_model(Some("claude-haiku-4-5"), "qwen"), "qwen");
        assert_eq!(pick_model(Some("my-model"), "qwen"), "my-model");
        assert_eq!(pick_model(None, "qwen"), "qwen");
    }

    fn run_stream(file: &str) -> Vec<Event> {
        let raw = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/translate").join(file)).unwrap();
        let mut p = SseParser::default();
        let mut t = StreamTranslator::new("gpt-test");
        let mut evs = Vec::new();
        // feed in awkward 7-byte pieces to exercise the parser
        for piece in raw.chunks(7) {
            for d in p.push(piece) {
                if d == "[DONE]" {
                    evs.extend(t.finish());
                } else {
                    evs.extend(t.feed(&serde_json::from_str(&d).unwrap()));
                }
            }
        }
        evs.extend(t.finish());
        evs
    }

    #[test]
    fn streaming_text() {
        let evs = run_stream("stream_text.sse");
        let names: Vec<&str> = evs.iter().map(|e| e.0).collect();
        assert_eq!(names, vec!["message_start", "content_block_start", "content_block_delta", "content_block_delta", "content_block_stop", "message_delta", "message_stop"]);
        let text: String = evs.iter().filter(|e| e.0 == "content_block_delta").map(|e| e.1["delta"]["text"].as_str().unwrap().to_string()).collect();
        assert_eq!(text, "Hello world");
        let md = &evs[5].1;
        assert_eq!(md["delta"]["stop_reason"], "end_turn");
        assert_eq!(md["usage"]["output_tokens"], 2);
        assert_eq!(md["usage"]["input_tokens"], 9);
    }

    #[test]
    fn streaming_tool_calls_with_interleaved_fragments_and_reasoning_dropped() {
        let evs = run_stream("stream_tools.sse");
        let names: Vec<&str> = evs.iter().map(|e| e.0).collect();
        assert_eq!(names, vec![
            "message_start",
            "content_block_start", "content_block_delta", "content_block_stop",
            "content_block_start", "content_block_delta", "content_block_stop",
            "content_block_start", "content_block_delta", "content_block_stop",
            "message_delta", "message_stop",
        ]);
        assert_eq!(evs[1].1["content_block"]["type"], "text");
        assert_eq!(evs[2].1["delta"]["text"], "Checking");
        assert_eq!(evs[4].1["content_block"]["name"], "Read");
        assert_eq!(evs[4].1["content_block"]["id"], "call_a");
        let a: Value = serde_json::from_str(evs[5].1["delta"]["partial_json"].as_str().unwrap()).unwrap();
        assert_eq!(a, json!({"file_path": "a.txt"}));
        assert_eq!(evs[7].1["content_block"]["name"], "Bash");
        let b: Value = serde_json::from_str(evs[8].1["delta"]["partial_json"].as_str().unwrap()).unwrap();
        assert_eq!(b, json!({"command": "ls -la"}));
        let idx: Vec<u64> = evs.iter().filter(|e| e.0 == "content_block_start").map(|e| e.1["index"].as_u64().unwrap()).collect();
        assert_eq!(idx, vec![0, 1, 2]);
        assert_eq!(evs[10].1["delta"]["stop_reason"], "tool_use");
        assert!(!evs.iter().any(|e| e.1.to_string().contains("secret reasoning")));
    }

    #[test]
    fn streaming_error_chunk() {
        let mut t = StreamTranslator::new("m");
        let evs = t.feed(&json!({"error": {"message": "boom"}}));
        assert_eq!(evs.last().unwrap().0, "error");
        assert!(t.finish().is_empty());
    }

    #[test]
    fn empty_stream_still_well_formed() {
        let mut t = StreamTranslator::new("m");
        let names: Vec<&str> = t.finish().iter().map(|e| e.0).collect();
        assert_eq!(names, vec!["message_start", "message_delta", "message_stop"]);
    }

    #[test]
    fn error_mapping() {
        let (s, b) = map_error(429, br#"{"error":{"message":"Rate limit reached","type":"requests"}}"#);
        assert_eq!(s, 429);
        assert_eq!(b, json!({"type": "error", "error": {"type": "rate_limit_error", "message": "upstream: Rate limit reached"}}));
        let (s, b) = map_error(401, br#"{"error":{"message":"Incorrect API key"}}"#);
        assert_eq!((s, b["error"]["type"].as_str().unwrap()), (401, "authentication_error"));
        let (s, b) = map_error(500, b"Internal Server Error");
        assert_eq!((s, b["error"]["type"].as_str().unwrap()), (500, "api_error"));
        assert_eq!(b["error"]["message"], "upstream: Internal Server Error");
        let (s, b) = map_error(503, b"");
        assert_eq!((s, b["error"]["type"].as_str().unwrap()), (503, "overloaded_error"));
        let (s, b) = map_error(422, br#"{"message":"bad field"}"#);
        assert_eq!((s, b["error"]["type"].as_str().unwrap()), (400, "invalid_request_error"));
        assert_eq!(map_error(404, b"{}").1["error"]["type"], "not_found_error");
        assert_eq!(map_error(403, b"{}").1["error"]["type"], "permission_error");
    }

    #[test]
    fn sse_parser_handles_crlf_and_split_lines() {
        let mut p = SseParser::default();
        assert!(p.push(b"data: {\"a\"").is_empty());
        assert_eq!(p.push(b":1}\r\n\r\ndata: [DONE]\r\n\r\n"), vec!["{\"a\":1}".to_string(), "[DONE]".to_string()]);
        assert!(p.push(b": keep-alive comment\n\n").is_empty());
    }
}
