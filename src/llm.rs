//! DeepSeek（OpenAI 兼容）Chat Completions 客户端，使用 SSE 流式输出。
//!
//! - 请求地址：`{base_url}/chat/completions`
//! - 思考模式：`thinking: {"type": "enabled"|"disabled"}` + `reasoning_effort`
//! - 流式响应中 `delta.reasoning_content` 为思考过程，`delta.content` 为正文
//! - 连接失败、429、5xx 会在开始接收数据前自动重试

use crate::config::{Config, Thinking};
use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, HeaderMap, RETRY_AFTER};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RETRIES: u32 = 2;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Serialize for Message {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("role", &self.role)?;
        map.serialize_entry("content", &self.content)?;
        if !self.tool_calls.is_empty() {
            map.serialize_entry("tool_calls", &self.tool_calls)?;
        }
        if let Some(id) = &self.tool_call_id {
            map.serialize_entry("tool_call_id", id)?;
        }
        map.end()
    }
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// assistant 请求一批工具调用；content 为空串（DeepSeek 要求字段存在）
    pub fn tool_calls(calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: String::new(),
            tool_calls: calls,
            tool_call_id: None,
        }
    }

    /// 一次工具调用的结果，回传给模型
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
        }
    }
}

/// 一次工具调用：id 用于把结果和它对应起来
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub function: FunctionCall,
}

impl Serialize for ToolCall {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("id", &self.id)?;
        map.serialize_entry("type", "function")?;
        map.serialize_entry("function", &self.function)?;
        map.end()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// JSON 编码的参数；格式由各工具自行解析和校验
    pub arguments: String,
}

/// 提供给模型的一个工具定义
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl Tool {
    fn to_json(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters,
            },
        })
    }
}

/// 单次请求的可选参数
#[derive(Debug, Clone, Default)]
pub struct ChatOptions {
    /// 要求返回 JSON 对象（`response_format = json_object`）。
    /// DeepSeek 要求此时提示词中必须出现 "json" 字样。
    pub json: bool,
    /// 覆盖配置中的 max_tokens
    pub max_tokens: Option<u32>,
    /// 提供给模型的工具；为空则不发送 `tools` 字段
    pub tools: Vec<Tool>,
}

/// 流式过程中的事件，供 UI 实时渲染
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent<'a> {
    Reasoning(&'a str),
    Content(&'a str),
    Retrying {
        attempt: u32,
        delay: Duration,
        reason: &'a str,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub prompt_cache_hit_tokens: Option<u64>,
    #[serde(default)]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct CompletionTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
}

impl Usage {
    pub fn reasoning_tokens(&self) -> Option<u64> {
        self.completion_tokens_details.as_ref()?.reasoning_tokens
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatResponse {
    pub content: String,
    pub reasoning: String,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
    /// 服务端实际使用的模型名
    pub model: Option<String>,
    /// 模型请求的工具调用；非空时 `finish_reason` 通常是 "tool_calls"
    pub tool_calls: Vec<ToolCall>,
}

impl ChatResponse {
    /// 输出因达到 max_tokens 被截断
    pub fn is_truncated(&self) -> bool {
        self.finish_reason.as_deref() == Some("length")
    }

    pub fn wants_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// 接口返回的非 2xx 错误
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    fn from_body(status: StatusCode, body: &str) -> Self {
        let message = serde_json::from_str::<ErrorEnvelope>(body)
            .ok()
            .and_then(|e| e.error)
            .map(|e| e.message)
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| preview(body.trim(), 300));
        Self { status, message }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self.status.as_u16(), 429 | 500 | 502 | 503 | 504)
    }

    fn hint(&self) -> &'static str {
        match self.status.as_u16() {
            400 => "请求格式错误",
            401 => {
                "API Key 无效或已过期，请输入 /model 重新设置（命令行：vibeguard config set api_key）"
            }
            402 => "账户余额不足，请到服务商的控制台充值",
            403 => "无权访问该接口",
            404 => "接口地址不存在，请输入 /model 检查 API 地址和模型名",
            422 => "请求参数错误，请输入 /model 检查模型和思考力度",
            429 => "请求过于频繁，已触发限流",
            500 => "服务端内部错误",
            502..=504 => "服务暂时不可用",
            _ => "请求失败",
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}（HTTP {}）", self.hint(), self.status.as_u16())?;
        if !self.message.is_empty() {
            write!(f, "：{}", self.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: Option<ErrorBody>,
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    error: Option<ErrorBody>,
}

#[derive(Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: Option<Delta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallDelta>,
}

/// 流式的工具调用分片：`index` 标识是第几个调用，字段在后续分片里逐步补全
#[derive(Deserialize)]
struct ToolCallDelta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FunctionDelta>,
}

#[derive(Deserialize, Default)]
struct FunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

pub struct LlmClient {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
    thinking: Thinking,
    temperature: f64,
    max_tokens: Option<u32>,
    retry_base: Duration,
}

impl LlmClient {
    pub fn new(cfg: &Config) -> Result<Self> {
        Self::build(cfg, reqwest::Client::builder())
    }

    fn build(cfg: &Config, builder: reqwest::ClientBuilder) -> Result<Self> {
        cfg.validate()?;
        let api_key = cfg
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| {
                anyhow!("未配置 API Key：运行 `vibeguard config` 进行设置，或导出环境变量 DEEPSEEK_API_KEY")
            })?
            .to_string();
        let http = builder
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(Duration::from_secs(cfg.timeout_secs))
            .user_agent(concat!("vibeguard/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("初始化 HTTP 客户端失败")?;
        Ok(Self {
            http,
            endpoint: format!("{}/chat/completions", cfg.api_base()),
            api_key,
            model: cfg.model.trim().to_string(),
            thinking: cfg.thinking,
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
            retry_base: Duration::from_secs(1),
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// 非交互调用：内部仍走流式接口，避免长时间无数据导致读超时
    #[allow(dead_code)] // 供后续 scanner / fixer 模块使用
    pub async fn chat(&self, messages: &[Message], opts: &ChatOptions) -> Result<ChatResponse> {
        self.chat_stream(messages, opts, |_| {}).await
    }

    pub async fn chat_stream(
        &self,
        messages: &[Message],
        opts: &ChatOptions,
        mut on_event: impl FnMut(StreamEvent<'_>),
    ) -> Result<ChatResponse> {
        let body = build_body(
            &self.model,
            self.thinking,
            self.temperature,
            self.max_tokens,
            messages,
            opts,
        )?;
        let resp = self.send_with_retry(&body, &mut on_event).await?;
        read_stream(resp, &mut on_event).await
    }

    async fn send_with_retry(
        &self,
        body: &Value,
        on_event: &mut impl FnMut(StreamEvent<'_>),
    ) -> Result<reqwest::Response> {
        let mut attempt = 0;
        loop {
            let result = self
                .http
                .post(&self.endpoint)
                .bearer_auth(&self.api_key)
                .header(ACCEPT, "text/event-stream")
                .json(body)
                .send()
                .await;

            let (err, retry_after) = match result {
                Ok(resp) if resp.status().is_success() => return Ok(resp),
                Ok(resp) => {
                    let status = resp.status();
                    let retry_after = parse_retry_after(resp.headers());
                    let text = resp.text().await.unwrap_or_default();
                    let api_err = ApiError::from_body(status, &text);
                    if !api_err.is_retryable() {
                        return Err(api_err.into());
                    }
                    (anyhow::Error::new(api_err), retry_after)
                }
                // 只重试“确定没发出去”的连接错误；超时的请求可能已在服务端计费，不自动重试
                Err(e) if e.is_connect() => (
                    anyhow::Error::new(e).context(format!("无法连接 {}", self.endpoint)),
                    None,
                ),
                Err(e) => {
                    let msg = if e.is_timeout() {
                        format!("请求 {} 超时", self.endpoint)
                    } else {
                        format!("请求 {} 失败", self.endpoint)
                    };
                    return Err(anyhow::Error::new(e).context(msg));
                }
            };

            if attempt >= MAX_RETRIES {
                return Err(err);
            }
            attempt += 1;
            let delay = retry_after
                .unwrap_or(self.retry_base * 2u32.pow(attempt - 1))
                .min(MAX_RETRY_DELAY);
            let reason = format!("{err:#}");
            on_event(StreamEvent::Retrying {
                attempt,
                delay,
                reason: &reason,
            });
            tokio::time::sleep(delay).await;
        }
    }
}

/// 构造请求体。拆成纯函数便于测试。
fn build_body(
    model: &str,
    thinking: Thinking,
    temperature: f64,
    max_tokens: Option<u32>,
    messages: &[Message],
    opts: &ChatOptions,
) -> Result<Value> {
    if messages.is_empty() {
        bail!("消息列表不能为空");
    }
    if opts.json
        && !messages
            .iter()
            .any(|m| m.content.to_ascii_lowercase().contains("json"))
    {
        bail!("JSON 模式要求提示词中包含 \"json\" 字样（DeepSeek API 限制）");
    }

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    let map = body.as_object_mut().expect("body is an object");
    if !opts.tools.is_empty() {
        let tools: Vec<Value> = opts.tools.iter().map(Tool::to_json).collect();
        map.insert("tools".into(), json!(tools));
    }

    match thinking {
        // 不发送，交给服务端默认值；也兼容不支持该参数的其他服务
        Thinking::Auto => {}
        Thinking::Off => {
            map.insert("thinking".into(), json!({ "type": "disabled" }));
        }
        level => {
            map.insert("thinking".into(), json!({ "type": "enabled" }));
            map.insert("reasoning_effort".into(), json!(level.as_str()));
        }
    }
    // 显式开启思考时 temperature 无效，不发送
    if matches!(thinking, Thinking::Auto | Thinking::Off) {
        map.insert("temperature".into(), json!(temperature));
    }
    if let Some(n) = opts.max_tokens.or(max_tokens) {
        map.insert("max_tokens".into(), json!(n));
    }
    if opts.json {
        map.insert("response_format".into(), json!({ "type": "json_object" }));
    }
    Ok(body)
}

/// 按 `index` 累积的工具调用分片；完成后转成 `Vec<ToolCall>`，按下标排序
#[derive(Default)]
struct ToolCallBuf {
    id: String,
    name: String,
    arguments: String,
}

async fn read_stream(
    resp: reqwest::Response,
    on_event: &mut impl FnMut(StreamEvent<'_>),
) -> Result<ChatResponse> {
    let mut out = ChatResponse::default();
    let mut tool_calls: std::collections::BTreeMap<usize, ToolCallBuf> = Default::default();
    let mut parser = SseParser::default();
    let mut stream = resp.bytes_stream();
    let mut done = false;

    'outer: while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("读取响应流失败（连接中断或超时）")?;
        for data in parser.feed(&chunk) {
            if handle_data(&data, &mut out, &mut tool_calls, on_event)? {
                done = true;
                break 'outer;
            }
        }
    }
    if !done {
        for data in parser.finish() {
            if handle_data(&data, &mut out, &mut tool_calls, on_event)? {
                done = true;
                break;
            }
        }
    }
    if !done && out.finish_reason.is_none() {
        bail!("响应流意外结束，结果可能不完整");
    }
    out.tool_calls = tool_calls
        .into_values()
        .map(|c| ToolCall {
            id: c.id,
            function: FunctionCall {
                name: c.name,
                arguments: c.arguments,
            },
        })
        .collect();
    Ok(out)
}

/// 处理一条 SSE data，收到 `[DONE]` 时返回 true
fn handle_data(
    data: &str,
    out: &mut ChatResponse,
    tool_calls: &mut std::collections::BTreeMap<usize, ToolCallBuf>,
    on_event: &mut impl FnMut(StreamEvent<'_>),
) -> Result<bool> {
    let data = data.trim();
    if data == "[DONE]" {
        return Ok(true);
    }
    if data.is_empty() {
        return Ok(false);
    }
    let chunk: Chunk = serde_json::from_str(data)
        .with_context(|| format!("无法解析响应数据：{}", preview(data, 200)))?;
    if let Some(err) = chunk.error {
        bail!("服务端返回错误：{}", err.message);
    }
    if out.model.is_none() {
        out.model = chunk.model;
    }
    if chunk.usage.is_some() {
        out.usage = chunk.usage;
    }
    for choice in chunk.choices {
        let delta = choice.delta.unwrap_or_default();
        if let Some(text) = delta.reasoning_content.filter(|s| !s.is_empty()) {
            on_event(StreamEvent::Reasoning(&text));
            out.reasoning.push_str(&text);
        }
        if let Some(text) = delta.content.filter(|s| !s.is_empty()) {
            on_event(StreamEvent::Content(&text));
            out.content.push_str(&text);
        }
        for tc in delta.tool_calls {
            let buf = tool_calls.entry(tc.index).or_default();
            if let Some(id) = tc.id {
                buf.id = id;
            }
            if let Some(f) = tc.function {
                if let Some(name) = f.name {
                    buf.name = name;
                }
                if let Some(args) = f.arguments {
                    buf.arguments.push_str(&args);
                }
            }
        }
        if choice.finish_reason.is_some() {
            out.finish_reason = choice.finish_reason;
        }
    }
    Ok(false)
}

/// 最小 SSE 解析器：按字节缓冲，保证跨数据块的多字节 UTF-8 字符不被截断。
/// 忽略注释行（`: keep-alive`）以及 `event:` / `id:` / `retry:` 字段。
#[derive(Default)]
struct SseParser {
    buf: Vec<u8>,
    data: String,
}

impl SseParser {
    fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        let mut start = 0;
        while let Some(pos) = self.buf[start..].iter().position(|&b| b == b'\n') {
            let end = start + pos;
            let line = String::from_utf8_lossy(&self.buf[start..end]).into_owned();
            self.line(line.trim_end_matches('\r'), &mut events);
            start = end + 1;
        }
        self.buf.drain(..start);
        events
    }

    /// 流结束：处理没有换行结尾的最后一行，并输出未结束的事件
    fn finish(&mut self) -> Vec<String> {
        let mut events = Vec::new();
        if !self.buf.is_empty() {
            let line = String::from_utf8_lossy(&std::mem::take(&mut self.buf)).into_owned();
            self.line(line.trim_end_matches('\r'), &mut events);
        }
        if !self.data.is_empty() {
            events.push(std::mem::take(&mut self.data));
        }
        events
    }

    fn line(&mut self, line: &str, events: &mut Vec<String>) {
        if line.is_empty() {
            if !self.data.is_empty() {
                events.push(std::mem::take(&mut self.data));
            }
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !self.data.is_empty() {
                self.data.push('\n');
            }
            self.data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
}

fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// 按字符截断，避免切断多字节字符
fn preview(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    // ---------- SSE 解析 ----------

    #[test]
    fn sse_handles_split_chunks_crlf_and_comments() {
        let mut p = SseParser::default();
        let raw = ": keep-alive\r\n\r\ndata: {\"a\":\"你好\"}\r\n\r\ndata: [DONE]\n\n".as_bytes();
        // 在“你”字的 UTF-8 字节中间切开
        let split = raw.iter().position(|&b| b == 0xE4).unwrap() + 1;
        let mut events = p.feed(&raw[..split]);
        assert!(events.is_empty());
        events.extend(p.feed(&raw[split..]));
        assert_eq!(
            events,
            vec!["{\"a\":\"你好\"}".to_string(), "[DONE]".to_string()]
        );
    }

    #[test]
    fn sse_joins_multiline_data_and_flushes_on_finish() {
        let mut p = SseParser::default();
        let events = p.feed(b"event: message\ndata: line1\ndata:line2\n\ndata: tail");
        assert_eq!(events, vec!["line1\nline2".to_string()]);
        assert_eq!(p.finish(), vec!["tail".to_string()]);
        assert!(p.finish().is_empty());
    }

    // ---------- 请求体 ----------

    fn msgs() -> Vec<Message> {
        vec![Message::system("you are a json bot"), Message::user("hi")]
    }

    #[test]
    fn body_respects_thinking_mode() {
        let o = ChatOptions::default();

        let auto = build_body("m", Thinking::Auto, 0.2, None, &msgs(), &o).unwrap();
        assert!(auto.get("thinking").is_none());
        assert!(auto.get("reasoning_effort").is_none());
        assert_eq!(auto["temperature"], json!(0.2));

        let off = build_body("m", Thinking::Off, 0.0, Some(100), &msgs(), &o).unwrap();
        assert_eq!(off["thinking"], json!({"type": "disabled"}));
        assert_eq!(off["max_tokens"], json!(100));

        let high = build_body("m", Thinking::High, 0.0, None, &msgs(), &o).unwrap();
        assert_eq!(high["thinking"], json!({"type": "enabled"}));
        assert_eq!(high["reasoning_effort"], json!("high"));
        assert!(high.get("temperature").is_none());

        assert_eq!(high["stream"], json!(true));
        assert_eq!(high["stream_options"]["include_usage"], json!(true));
        assert_eq!(
            high["messages"][1],
            json!({"role": "user", "content": "hi"})
        );
    }

    #[test]
    fn body_json_mode() {
        let o = ChatOptions {
            json: true,
            max_tokens: Some(8),
            tools: Vec::new(),
        };
        let body = build_body("m", Thinking::Auto, 0.0, Some(100), &msgs(), &o).unwrap();
        assert_eq!(body["response_format"], json!({"type": "json_object"}));
        assert_eq!(body["max_tokens"], json!(8), "单次参数优先于配置");

        let no_json_word = [Message::user("hello")];
        assert!(build_body("m", Thinking::Auto, 0.0, None, &no_json_word, &o).is_err());
        assert!(build_body("m", Thinking::Auto, 0.0, None, &[], &ChatOptions::default()).is_err());
    }

    // ---------- 错误信息 ----------

    #[test]
    fn api_error_messages() {
        let e = ApiError::from_body(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"message":"Authentication Fails","type":"authentication_error"}}"#,
        );
        let s = e.to_string();
        assert!(
            s.contains("API Key") && s.contains("Authentication Fails") && s.contains("401"),
            "{s}"
        );
        assert!(!e.is_retryable());

        let e = ApiError::from_body(StatusCode::SERVICE_UNAVAILABLE, "<html>busy</html>");
        assert!(e.is_retryable());
        assert!(e.to_string().contains("<html>busy</html>"));
    }

    // ---------- 端到端：本地假服务器 ----------

    const SSE_OK: &str = concat!(
        ": keep-alive\n\n",
        "data: {\"id\":\"1\",\"model\":\"deepseek-flash\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"想一想\"},\"finish_reason\":null}],\"usage\":null}\n\n",
        "data: {\"id\":\"1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"你好\"},\"finish_reason\":null}],\"usage\":null}\n\n",
        "data: {\"id\":\"1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"，世界\"},\"finish_reason\":\"stop\"}],",
        "\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15,\"prompt_cache_hit_tokens\":8,",
        "\"completion_tokens_details\":{\"reasoning_tokens\":2}}}\n\n",
        "data: [DONE]\n\n",
    );

    fn sse_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}"
        )
    }

    fn json_response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// 依次用给定的原始 HTTP 响应应答每个连接，返回收到的原始请求
    fn serve(responses: Vec<String>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            responses
                .into_iter()
                .map(|resp| {
                    let (mut sock, _) = listener.accept().unwrap();
                    let req = read_request(&mut sock);
                    sock.write_all(resp.as_bytes()).unwrap();
                    req
                })
                .collect()
        });
        (format!("http://{addr}"), handle)
    }

    fn read_request(sock: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            let n = sock.read(&mut tmp).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
                let len = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if buf.len() >= end + 4 + len {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    fn client_for(base: &str) -> LlmClient {
        let cfg = Config {
            api_key: Some("sk-test-key-123456".into()),
            base_url: base.into(),
            thinking: Thinking::Off,
            ..Config::default()
        };
        let mut client = LlmClient::build(&cfg, reqwest::Client::builder().no_proxy()).unwrap();
        client.retry_base = Duration::from_millis(10);
        client
    }

    #[tokio::test]
    async fn streams_reasoning_content_and_usage() {
        let (base, server) = serve(vec![sse_response(SSE_OK)]);
        let client = client_for(&base);

        let mut events = Vec::new();
        let res = client
            .chat_stream(&[Message::user("hi")], &ChatOptions::default(), |ev| {
                events.push(format!("{ev:?}"))
            })
            .await
            .unwrap();

        assert_eq!(res.reasoning, "想一想");
        assert_eq!(res.content, "你好，世界");
        assert_eq!(res.finish_reason.as_deref(), Some("stop"));
        assert_eq!(res.model.as_deref(), Some("deepseek-flash"));
        let usage = res.usage.unwrap();
        assert_eq!(usage.total_tokens, 15);
        assert_eq!(usage.prompt_cache_hit_tokens, Some(8));
        assert_eq!(usage.reasoning_tokens(), Some(2));
        assert_eq!(events.len(), 3);

        let req = server.join().unwrap().remove(0);
        assert!(req.starts_with("POST /chat/completions HTTP/1.1"), "{req}");
        assert!(
            req.to_ascii_lowercase()
                .contains("authorization: bearer sk-test-key-123456")
        );
        assert!(req.contains(r#""thinking":{"type":"disabled"}"#), "{req}");
        assert!(req.contains(r#""model":"deepseek-flash""#), "{req}");
    }

    #[tokio::test]
    async fn retries_on_503_then_succeeds() {
        let (base, server) = serve(vec![
            json_response("503 Service Unavailable", r#"{"error":{"message":"busy"}}"#),
            sse_response(SSE_OK),
        ]);
        let client = client_for(&base);

        let mut retries = 0;
        let res = client
            .chat_stream(&[Message::user("hi")], &ChatOptions::default(), |ev| {
                if let StreamEvent::Retrying {
                    attempt, reason, ..
                } = ev
                {
                    assert_eq!(attempt, 1);
                    assert!(reason.contains("busy"));
                    retries += 1;
                }
            })
            .await
            .unwrap();
        assert_eq!(retries, 1);
        assert_eq!(res.content, "你好，世界");
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn auth_error_is_not_retried() {
        let (base, server) = serve(vec![json_response(
            "401 Unauthorized",
            r#"{"error":{"message":"Authentication Fails"}}"#,
        )]);
        let client = client_for(&base);
        let err = client
            .chat(&[Message::user("hi")], &ChatOptions::default())
            .await
            .unwrap_err();
        let api = err.downcast_ref::<ApiError>().expect("ApiError");
        assert_eq!(api.status, StatusCode::UNAUTHORIZED);
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn truncated_stream_is_an_error() {
        let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"半截\"},\"finish_reason\":null}]}\n\n";
        let (base, server) = serve(vec![sse_response(body)]);
        let client = client_for(&base);
        let err = client
            .chat(&[Message::user("hi")], &ChatOptions::default())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("意外结束"), "{err:#}");
        server.join().unwrap();
    }

    #[tokio::test]
    async fn mid_stream_error_is_reported() {
        let body = "data: {\"error\":{\"message\":\"content risk\"}}\n\n";
        let (base, server) = serve(vec![sse_response(body)]);
        let client = client_for(&base);
        let err = client
            .chat(&[Message::user("hi")], &ChatOptions::default())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("content risk"), "{err:#}");
        server.join().unwrap();
    }

    #[test]
    fn missing_api_key_is_explained() {
        let err = LlmClient::new(&Config::default()).err().unwrap();
        assert!(err.to_string().contains("DEEPSEEK_API_KEY"));
    }
}
