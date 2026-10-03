//! ModelClient（四个接口之一，P5 双模型实测位）+ 现成 OpenAI 兼容客户端库
//! 适配器（p0-brief 第三节：客户端库不自写）。
//!
//! MiMo 官方 API 适配口径（mimo.mi.com 官方文档 chat/openai-api 与 deep-thinking）：
//! - 思维链开关：请求体非标字段 `thinking: {"type":"enabled"|"disabled"}`。
//!   Settings.reasoning_effort="none" → disabled，其余档位 → enabled
//!   （官方明言：当前暂不支持自定义调节推理投入档位，只分关/开）。
//!   reasoning_effort 原字段同时照传（官方无害声明，以 thinking.type 为准）。
//! - reasoning_content 必回传（硬约束）：多轮工具调用时 assistant 历史消息
//!   必须原样携带 reasoning_content，缺失 API 返 400，且指令遵循下降、
//!   幻觉增多。非流式在 choices.message.reasoning_content，
//!   流式在 choices.delta.reasoning_content，两路都必须完整捕获并回传。
//! - 流式：stream:true，先 delta.reasoning_content 逐段流思考过程，思考完后
//!   delta.content 逐段流最终回答；tool_calls 以增量 chunk 出现；
//!   stream_options.include_usage 带上末尾 usage。
//! - temperature/top_p 禁传（mimo-2.6 系列被强制覆盖，传了也白传）。
//! - 工具轮思考开关：Settings.thinking_on_tools=false 时，带工具的请求强制
//!   thinking disabled（官方 FAQ：思考开 + 调工具时 tool_calls 可能混进
//!   reasoning_content，不稳定；官方建议调工具场景关思考）。
//! - 重试（官方 FAQ：指数退避）：传输级错误（网络/5xx/429）重试 2 次，
//!   间隔 500ms / 2s；400/401 等 4xx 不重试，直接报中文错误。
//!
//! 实现路线（实测定案）：非流式走 async-openai 的 byot 泛型入口（自带 wire
//! 类型，保住 reasoning_content）；流式走同一依赖树内 reqwest 的 chunk()
//! 自解析 SSE（async-openai 标准类型会丢弃 reasoning_content）。两路共用
//! 本文件的 wire 类型与解析层（StreamAssembler / SseParser）。
//!
//! 凭据纪律：API key 只经 config::Secret 流转，禁打印、禁进事件；
//! 明文来源只允许 config::Settings 的启用配置（用户目录 settings.json，
//! 在仓库外）或环境变量回落链。端点 / 模型名 / 思考强度从启用配置（active()）取。

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{Secret, Settings};
use crate::tools::ToolSpec;

/// 模型无关消息形态（核心不绑任何第三方 wire 类型）。
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    /// 模型给出的参数 JSON 原文。
    pub arguments_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// assistant 消息携带的工具调用请求。
    pub tool_calls: Vec<ToolCallRequest>,
    /// tool 消息对应哪个 tool_call_id。
    pub tool_call_id: Option<String>,
    /// 思维链正文（MiMo reasoning_content）：assistant 消息回传历史时必须
    /// 原样带上（官方硬要求，缺失 400）。空串归一为 None。
    pub reasoning_content: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
        }
    }

    pub fn assistant(content: impl Into<String>, tool_calls: Vec<ToolCallRequest>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
            reasoning_content: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: Some(tool_call_id.into()),
            reasoning_content: None,
        }
    }

    /// 追加思维链正文（历史回传用；空串归一为 None）。
    pub fn with_reasoning_content(mut self, reasoning: impl Into<String>) -> Self {
        let r = reasoning.into();
        self.reasoning_content = if r.is_empty() { None } else { Some(r) };
        self
    }
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    /// 模型轮次（1 起）：StreamObserver 回调的 round 参数取自这里；
    /// 非流式路径不使用（探针等一次性请求填 0 即可）。
    pub round: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub usage: Option<Usage>,
    /// 本轮思维链正文（reasoning_content 全量；无思考为空串）。
    /// 调用方须存入历史并在后续请求回传（见 ChatMessage.reasoning_content）。
    pub reasoning_content: String,
}

#[derive(Debug)]
pub enum ModelError {
    MissingConfig(Vec<&'static str>),
    MissingCredential(&'static str),
    Transport(String),
    BadResponse(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::MissingConfig(names) => {
                write!(
                    f,
                    "模型配置缺失：{}。修法：运行 sd-agent doctor 查看，或在桌面端“设置”面板填写",
                    names.join("、")
                )
            }
            ModelError::MissingCredential(name) => write!(
                f,
                "模型凭据缺失（{name}）。修法：在桌面端“设置”面板填写 API 密钥，或设置同名环境变量"
            ),
            ModelError::Transport(m) => write!(
                f,
                "模型请求失败（连不上或网络中断）：{m}。修法：检查“端点”网址是否正确、网络是否可用"
            ),
            ModelError::BadResponse(m) => write!(
                f,
                "模型返回内容不合法：{m}。修法：检查模型名是否正确、端点是否为 OpenAI 兼容接口"
            ),
        }
    }
}

impl std::error::Error for ModelError {}

/// 流式回调接口（给壳用，契约写死）：delta 逐段上抛，壳层拼显示；
/// 落盘事件契约不收 delta（防炸轨迹），只有 turn 级汇总进事件。
pub trait StreamObserver: Send + Sync {
    /// 思考过程增量（choices.delta.reasoning_content 逐段）。
    fn on_reasoning_delta(&self, round: u32, delta: &str);
    /// 最终回答增量（choices.delta.content 逐段）。
    fn on_text_delta(&self, round: u32, delta: &str);
    /// 工具调用增量：name 为工具名，args_so_far 为该调用参数 JSON 的累计值。
    fn on_tool_call_delta(&self, round: u32, name: &str, args_so_far: &str);
    /// 一轮模型输出收尾（无论有无工具调用，每轮恰一次）。
    fn on_turn_done(&self, round: u32);
}

/// 模型客户端接口（手写 boxed future，不引 futures 库）。
pub trait ModelClient: Send + Sync {
    fn chat<'a>(
        &'a self,
        request: ChatRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ChatResponse, ModelError>> + Send + 'a>>;

    /// 流式请求：reasoning / 正文 / 工具调用三路 delta 经 `obs` 实时上抛
    /// （round 取自 request.round），返回值与 chat() 同形态
    /// （含 reasoning_content 全量与 usage）。
    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest,
        obs: &'a dyn StreamObserver,
    ) -> Pin<Box<dyn Future<Output = Result<ChatResponse, ModelError>> + Send + 'a>>;
}

/// 重试节奏（官方 FAQ：指数退避；传输级错误重试 2 次）。
const RETRY_DELAYS_MS: [u64; 2] = [500, 2000];

/// 传输级错误可重试（网络 / 5xx / 429）；4xx 请求性错误不重试。
fn is_retryable(err: &ModelError) -> bool {
    matches!(err, ModelError::Transport(_))
}

/// OpenAI 兼容 chat_completions 适配器（现成客户端库）。
pub struct OpenAiCompatClient {
    /// 非流式请求走现成客户端库（byot 泛型入口，自带 wire 类型）。
    client: async_openai::Client<async_openai::config::OpenAIConfig>,
    /// 流式请求走同一依赖树内的 HTTP 客户端（SSE 自解析，见 SseParser）。
    http: reqwest::Client,
    /// 凭据本体（结构不存明文副本，取值只在构造请求头时 expose()）。
    api_key: Secret,
    model: String,
    /// 端点（含 /v1，已去尾部斜杠）。
    base_url: String,
    /// 思考强度原文（"none|minimal|low|medium|high|xhigh|max"）。
    reasoning_effort: String,
    /// 工具轮思考开关：false 时带工具的请求强制 thinking disabled。
    thinking_on_tools: bool,
}

impl OpenAiCompatClient {
    /// 从配置链装配：取 Settings 的启用配置（active()），端点 / 模型名 /
    /// 密钥 / 思考强度 / 工具轮思考开关全套带上；配置缺失报 MissingConfig
    /// （含缺失项名单）。凭据包成 Secret 后持有，本结构不保留明文副本。
    pub fn from_settings(settings: &Settings) -> Result<Self, ModelError> {
        let missing = settings.missing_fields();
        if !missing.is_empty() {
            return Err(ModelError::MissingConfig(missing));
        }
        // missing_fields 为空即 active() 必然命中（防御性兜底走同一错误形态）。
        let profile = settings
            .active()
            .ok_or_else(|| ModelError::MissingConfig(vec!["active_profile"]))?;
        let key = Secret::new(
            profile
                .api_key
                .clone()
                .filter(|k| !k.trim().is_empty())
                .ok_or(ModelError::MissingCredential("api_key"))?,
        );
        let base_url = profile.base_url.trim_end_matches('/').to_string();
        let config = async_openai::config::OpenAIConfig::new()
            .with_api_key(key.expose())
            .with_api_base(base_url.clone());
        Ok(Self {
            client: async_openai::Client::with_config(config),
            http: reqwest::Client::new(),
            api_key: key,
            model: profile.model.clone(),
            base_url,
            reasoning_effort: profile.reasoning_effort.clone(),
            thinking_on_tools: settings.thinking_on_tools,
        })
    }

    /// 兼容入口：只认环境变量（SD_AGENT_BASE_URL / SD_AGENT_MODEL /
    /// SD_AGENT_API_KEY / SD_AGENT_MAX_ROUNDS），内部转 Settings 再走
    /// from_settings；错误形态与旧版一致（先配置后凭据）。
    pub fn from_env() -> Result<Self, ModelError> {
        let settings = Settings::from_env();
        let missing: Vec<&'static str> = settings
            .missing_fields()
            .into_iter()
            .filter(|name| *name != "api_key")
            .collect();
        if !missing.is_empty() {
            return Err(ModelError::MissingConfig(missing));
        }
        if settings.active().map(|p| !p.has_api_key()).unwrap_or(true) {
            return Err(ModelError::MissingCredential("SD_AGENT_API_KEY"));
        }
        Self::from_settings(&settings)
    }

    pub fn model_name(&self) -> &str {
        &self.model
    }

    /// thinking.type 映射（官方口径）：reasoning_effort="none" → disabled，
    /// 其余 → enabled；Settings.thinking_on_tools=false 且本轮带工具 → 强制
    /// disabled（不做智能开关，纯设置项裁决）。
    fn thinking_type(&self, request: &ChatRequest) -> &'static str {
        let effort_on = !self.reasoning_effort.trim().eq_ignore_ascii_case("none");
        let tools_force_off = !request.tools.is_empty() && !self.thinking_on_tools;
        if effort_on && !tools_force_off {
            "enabled"
        } else {
            "disabled"
        }
    }

    /// 组装 wire 请求（唯一 wire 转换点，流式/非流式共用）。
    fn build_wire_request(&self, request: &ChatRequest, stream: bool) -> WireChatRequest {
        WireChatRequest {
            model: self.model.clone(),
            messages: request.messages.iter().map(to_wire_message).collect(),
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(
                    request
                        .tools
                        .iter()
                        .map(|spec| WireTool {
                            tool_type: "function".to_string(),
                            function: WireToolFunction {
                                name: spec.name.to_string(),
                                description: Some(spec.description.to_string()),
                                parameters: Some(spec.parameters.clone()),
                            },
                        })
                        .collect(),
                )
            },
            // 上限沿用旧口径（4096）；temperature/top_p 禁传（mimo-2.6 强制覆盖）。
            max_tokens: 4096,
            stream,
            stream_options: if stream {
                Some(WireStreamOptions { include_usage: true })
            } else {
                None
            },
            thinking: WireThinking {
                kind: self.thinking_type(request),
            },
            reasoning_effort: self.reasoning_effort.clone(),
        }
    }

    /// 非流式单次尝试（不含重试）。
    async fn chat_once(&self, wire: &WireChatRequest) -> Result<ChatResponse, ModelError> {
        let response: WireChatResponse = self
            .client
            .chat()
            .create_byot::<WireChatRequest, WireChatResponse>(wire.clone())
            .await
            .map_err(map_openai_error)?;
        wire_response_to_chat(response)
    }

    /// 流式单次尝试（不含重试）：SSE 自解析 → StreamAssembler → 三路回调。
    /// 失败时 assembler.emitted 告知上层是否已上抛过 delta（已上抛禁止重试，
    /// 防输出重放）。
    async fn stream_once(
        &self,
        wire: &WireChatRequest,
        assembler: &mut StreamAssembler<'_>,
    ) -> Result<(), ModelError> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut resp = self
            .http
            .post(&url)
            .bearer_auth(self.api_key.expose())
            .json(wire)
            .send()
            .await
            .map_err(|e| ModelError::Transport(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(map_http_error(status.as_u16(), &body));
        }
        let mut parser = SseParser::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| ModelError::Transport(e.to_string()))?
        {
            for payload in parser.feed(&chunk) {
                if payload.trim() == "[DONE]" {
                    return Ok(());
                }
                assembler.feed_chunk_json(&payload)?;
            }
        }
        // 流结束没等到 [DONE] 也接受（服务端直接断流的兜底）。
        Ok(())
    }
}

impl ModelClient for OpenAiCompatClient {
    fn chat<'a>(
        &'a self,
        request: ChatRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ChatResponse, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            let wire = self.build_wire_request(&request, false);
            let mut attempt = 0usize;
            loop {
                match self.chat_once(&wire).await {
                    Ok(r) => return Ok(r),
                    Err(e) if attempt < RETRY_DELAYS_MS.len() && is_retryable(&e) => {
                        tokio::time::sleep(Duration::from_millis(RETRY_DELAYS_MS[attempt])).await;
                        attempt += 1;
                    }
                    Err(e) => return Err(e),
                }
            }
        })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest,
        obs: &'a dyn StreamObserver,
    ) -> Pin<Box<dyn Future<Output = Result<ChatResponse, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            let wire = self.build_wire_request(&request, true);
            let mut assembler = StreamAssembler::new(request.round, Some(obs));
            let mut attempt = 0usize;
            loop {
                match self.stream_once(&wire, &mut assembler).await {
                    Ok(()) => return Ok(assembler.finish()),
                    // 已上抛过 delta 的失败不重试（重放会重复输出）。
                    Err(e)
                        if attempt < RETRY_DELAYS_MS.len()
                            && is_retryable(&e)
                            && !assembler.emitted =>
                    {
                        assembler.reset();
                        tokio::time::sleep(Duration::from_millis(RETRY_DELAYS_MS[attempt])).await;
                        attempt += 1;
                    }
                    Err(e) => return Err(e),
                }
            }
        })
    }
}

// ---- wire 形态（OpenAI 兼容协议 + MiMo reasoning_content / thinking 扩展）----

#[derive(Debug, Clone, Serialize)]
struct WireChatRequest {
    model: String,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<WireTool>>,
    max_tokens: u32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<WireStreamOptions>,
    /// MiMo 非标字段（官方 deep-thinking 页）：{"type":"enabled"|"disabled"}。
    thinking: WireThinking,
    /// 原思考强度字段照传（官方声明无害；以 thinking.type 为准）。
    reasoning_effort: String,
}

#[derive(Debug, Clone, Serialize)]
struct WireThinking {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct WireStreamOptions {
    include_usage: bool,
}

#[derive(Debug, Clone, Serialize)]
struct WireTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: WireToolFunction,
}

#[derive(Debug, Clone, Serialize)]
struct WireToolFunction {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<serde_json::Value>,
}

/// 消息 wire 形态（请求序列化 + 响应反序列化共用）。
/// assistant 消息恒带 reasoning_content（官方硬要求：历史 reasoning_content
/// 必须回传，缺失 400；MiMo 优先适配）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<WireToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireToolCall {
    id: String,
    function: WireFnCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireFnCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct WireChatResponse {
    #[serde(default)]
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Debug, Deserialize)]
struct WireChoice {
    #[serde(default)]
    message: WireResponseMessage,
}

#[derive(Debug, Default, Deserialize)]
struct WireResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<WireToolCall>>,
}

/// 流式 chunk（choices 可为空——usage 收尾 chunk 只带 usage）。
#[derive(Debug, Deserialize)]
struct WireChunk {
    #[serde(default)]
    choices: Vec<WireChunkChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Debug, Deserialize)]
struct WireChunkChoice {
    #[serde(default)]
    delta: WireDelta,
}

#[derive(Debug, Default, Deserialize)]
struct WireDelta {
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<WireToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct WireToolCallDelta {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<WireFnDelta>,
}

#[derive(Debug, Deserialize)]
struct WireFnDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

/// 核心消息形态 → wire 形态（唯一转换点；wire 细节不出 model.rs）。
/// assistant 消息恒带 reasoning_content（历史回传硬要求；无思考时空串）。
fn to_wire_message(msg: &ChatMessage) -> WireMessage {
    let (role, reasoning) = match msg.role {
        Role::System => ("system", None),
        Role::User => ("user", None),
        Role::Assistant => (
            "assistant",
            Some(msg.reasoning_content.clone().unwrap_or_default()),
        ),
        Role::Tool => ("tool", None),
    };
    WireMessage {
        role: role.to_string(),
        content: Some(msg.content.clone()),
        reasoning_content: reasoning,
        tool_calls: if msg.tool_calls.is_empty() {
            None
        } else {
            Some(
                msg.tool_calls
                    .iter()
                    .map(|tc| WireToolCall {
                        id: tc.id.clone(),
                        function: WireFnCall {
                            name: tc.name.clone(),
                            arguments: tc.arguments_json.clone(),
                        },
                    })
                    .collect(),
            )
        },
        tool_call_id: msg.tool_call_id.clone(),
    }
}

/// wire 消息 → 核心消息形态（roundtrip 测试与解析层共用）。
fn from_wire_message(msg: WireMessage) -> ChatMessage {
    let mut out = match msg.role.as_str() {
        "assistant" => ChatMessage::assistant(
            msg.content.unwrap_or_default(),
            msg.tool_calls
                .unwrap_or_default()
                .into_iter()
                .map(|tc| ToolCallRequest {
                    id: tc.id,
                    name: tc.function.name,
                    arguments_json: tc.function.arguments,
                })
                .collect(),
        ),
        "tool" => ChatMessage::tool_result(
            msg.tool_call_id.unwrap_or_default(),
            msg.content.unwrap_or_default(),
        ),
        "system" => ChatMessage::system(msg.content.unwrap_or_default()),
        _ => ChatMessage::user(msg.content.unwrap_or_default()),
    };
    out.reasoning_content = msg.reasoning_content.filter(|r| !r.is_empty());
    out
}

/// 非流式 wire 响应 → ChatResponse。
fn wire_response_to_chat(response: WireChatResponse) -> Result<ChatResponse, ModelError> {
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| ModelError::BadResponse("empty choices".into()))?;
    let message = choice.message;
    // 消息映射复用 wire → 核心转换点（含 reasoning_content 提取）。
    let msg = from_wire_message(WireMessage {
        role: "assistant".to_string(),
        content: message.content,
        reasoning_content: message.reasoning_content,
        tool_calls: message.tool_calls,
        tool_call_id: None,
    });
    Ok(ChatResponse {
        text: msg.content,
        reasoning_content: msg.reasoning_content.unwrap_or_default(),
        tool_calls: msg.tool_calls,
        usage: response.usage.map(|u| Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
        }),
    })
}

/// 现成客户端库错误 → ModelError（4xx 不可重试，429/5xx/网络可重试）。
fn map_openai_error(err: async_openai::error::OpenAIError) -> ModelError {
    use async_openai::error::OpenAIError as E;
    match err {
        E::ApiError(resp) => map_http_error(resp.status_code.as_u16(), &resp.api_error.message),
        E::Reqwest(e) => ModelError::Transport(e.to_string()),
        other => ModelError::BadResponse(other.to_string()),
    }
}

/// HTTP 状态 + 错误体 → ModelError（429/5xx → Transport 可重试；4xx → BadResponse）。
fn map_http_error(status: u16, body: &str) -> ModelError {
    let msg = extract_api_error_message(body).unwrap_or_else(|| body.chars().take(300).collect());
    if status == 429 || (500..600).contains(&status) {
        ModelError::Transport(format!("HTTP {status}：{msg}"))
    } else {
        ModelError::BadResponse(format!("HTTP {status}：{msg}"))
    }
}

/// 从错误体抽 {"error":{"message":...}}（抽不出就用原文截断）。
fn extract_api_error_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
}

/// SSE 帧解析器（生产路径：流式响应字节流 → data 载荷序列）。
/// 只取 data 行（多行以 \n 连接），忽略注释行与其他字段；\r\n / \n 均收；
/// 字节级跨包切分不丢不重。
struct SseParser {
    buf: Vec<u8>,
    /// 当前事件已收集的 data 行（空行 = 事件边界时拼接吐出）。
    event_data: Vec<String>,
}

impl SseParser {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            event_data: Vec::new(),
        }
    }

    /// 喂入一段原始字节，吐出本次新完成事件的 data 载荷（按序）。
    fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let Some(pos) = self.buf.iter().position(|b| *b == b'\n') else {
                break;
            };
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop(); // 去 \n
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            if line.is_empty() {
                if !self.event_data.is_empty() {
                    out.push(self.event_data.join("\n"));
                    self.event_data.clear();
                }
                continue;
            }
            if line.starts_with(':') {
                continue; // 注释行（含 keepalive）
            }
            if let Some(value) = line.strip_prefix("data:") {
                let value = value.strip_prefix(' ').unwrap_or(value);
                self.event_data.push(value.to_string());
            }
            // 其他字段（event: / id: / retry:）忽略
        }
        out
    }
}

/// 流式累加器：chunk 解析 → 三路 delta 回调 → ChatResponse 收尾。
/// mock SSE 测试直接驱动本层（与生产路径同一解析代码）。
struct StreamAssembler<'a> {
    round: u32,
    obs: Option<&'a dyn StreamObserver>,
    reasoning: String,
    text: String,
    calls: BTreeMap<usize, ToolCallRequest>,
    usage: Option<Usage>,
    /// 是否已上抛过任何 delta（决定失败后能否重试）。
    emitted: bool,
}

impl<'a> StreamAssembler<'a> {
    fn new(round: u32, obs: Option<&'a dyn StreamObserver>) -> Self {
        Self {
            round,
            obs,
            reasoning: String::new(),
            text: String::new(),
            calls: BTreeMap::new(),
            usage: None,
            emitted: false,
        }
    }

    /// 重试前清空半截状态（只在未上抛过 delta 时调用）。
    fn reset(&mut self) {
        self.reasoning.clear();
        self.text.clear();
        self.calls.clear();
        self.usage = None;
    }

    /// 消费一个 SSE data 载荷（JSON chunk 原文）。
    fn feed_chunk_json(&mut self, json: &str) -> Result<(), ModelError> {
        let chunk: WireChunk = serde_json::from_str(json)
            .map_err(|e| ModelError::BadResponse(format!("流式 chunk 解析失败：{e}")))?;
        self.feed_chunk(chunk);
        Ok(())
    }

    fn feed_chunk(&mut self, chunk: WireChunk) {
        if let Some(u) = chunk.usage {
            self.usage = Some(Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
            });
        }
        for choice in chunk.choices {
            let delta = choice.delta;
            if let Some(r) = delta.reasoning_content {
                if !r.is_empty() {
                    self.reasoning.push_str(&r);
                    self.emitted = true;
                    if let Some(obs) = self.obs {
                        obs.on_reasoning_delta(self.round, &r);
                    }
                }
            }
            if let Some(t) = delta.content {
                if !t.is_empty() {
                    self.text.push_str(&t);
                    self.emitted = true;
                    if let Some(obs) = self.obs {
                        obs.on_text_delta(self.round, &t);
                    }
                }
            }
            for tc in delta.tool_calls.unwrap_or_default() {
                let slot = self.calls.entry(tc.index).or_insert_with(|| ToolCallRequest {
                    id: String::new(),
                    name: String::new(),
                    arguments_json: String::new(),
                });
                if let Some(id) = tc.id {
                    slot.id = id;
                }
                if let Some(f) = tc.function {
                    if let Some(n) = f.name {
                        slot.name.push_str(&n);
                    }
                    if let Some(a) = f.arguments {
                        slot.arguments_json.push_str(&a);
                    }
                }
                self.emitted = true;
                if let Some(obs) = self.obs {
                    obs.on_tool_call_delta(self.round, &slot.name, &slot.arguments_json);
                }
            }
        }
    }

    /// 一轮收尾：回调整轮完成（每轮恰一次），产出与 chat() 同形态的响应。
    fn finish(&mut self) -> ChatResponse {
        let tool_calls: Vec<ToolCallRequest> = self.calls.values().cloned().collect();
        if let Some(obs) = self.obs {
            obs.on_turn_done(self.round);
        }
        ChatResponse {
            text: std::mem::take(&mut self.text),
            tool_calls,
            usage: self.usage.clone(),
            reasoning_content: std::mem::take(&mut self.reasoning),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 录制型观察者：记录三路 delta 与收尾（mock 测试断言用）。
    #[derive(Default)]
    struct Rec {
        reasoning: Mutex<String>,
        text: Mutex<String>,
        tools: Mutex<Vec<(String, String)>>,
        done_rounds: Mutex<Vec<u32>>,
        rounds: Mutex<Vec<u32>>,
    }

    impl StreamObserver for Rec {
        fn on_reasoning_delta(&self, round: u32, delta: &str) {
            self.rounds.lock().unwrap().push(round);
            self.reasoning.lock().unwrap().push_str(delta);
        }
        fn on_text_delta(&self, round: u32, delta: &str) {
            self.rounds.lock().unwrap().push(round);
            self.text.lock().unwrap().push_str(delta);
        }
        fn on_tool_call_delta(&self, round: u32, name: &str, args_so_far: &str) {
            self.rounds.lock().unwrap().push(round);
            self.tools
                .lock()
                .unwrap()
                .push((name.to_string(), args_so_far.to_string()));
        }
        fn on_turn_done(&self, round: u32) {
            self.done_rounds.lock().unwrap().push(round);
        }
    }

    fn req_with(messages: Vec<ChatMessage>) -> ChatRequest {
        ChatRequest {
            messages,
            tools: vec![],
            round: 1,
        }
    }

    fn client_with(effort: &str, thinking_on_tools: bool) -> OpenAiCompatClient {
        OpenAiCompatClient {
            client: async_openai::Client::with_config(async_openai::config::OpenAIConfig::new()),
            http: reqwest::Client::new(),
            api_key: Secret::new("test-key".to_string()),
            model: "mimo-test".to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            reasoning_effort: effort.to_string(),
            thinking_on_tools,
        }
    }

    #[test]
    fn thinking_type_mapping_follows_official_rule() {
        // none → disabled，其余档位 → enabled。
        for effort in ["minimal", "low", "medium", "high", "xhigh", "max"] {
            let c = client_with(effort, true);
            assert_eq!(
                c.thinking_type(&req_with(vec![])),
                "enabled",
                "{effort} 应开思考"
            );
        }
        let off = client_with("none", true);
        assert_eq!(off.thinking_type(&req_with(vec![])), "disabled");

        // 工具轮开关：false 且带工具 → 强制 disabled；true → 按用户设置。
        let mut req = req_with(vec![]);
        req.tools = crate::tools::catalog();
        assert!(!req.tools.is_empty());
        let strict = client_with("high", false);
        assert_eq!(
            strict.thinking_type(&req),
            "disabled",
            "带工具且开关关 → 关思考"
        );
        assert_eq!(
            strict.thinking_type(&req_with(vec![])),
            "enabled",
            "无工具不受开关影响"
        );
        let loose = client_with("high", true);
        assert_eq!(loose.thinking_type(&req), "enabled");
        let none = client_with("none", true);
        assert_eq!(none.thinking_type(&req), "disabled", "none 恒关");
    }

    #[test]
    fn wire_request_carries_thinking_type_and_stream_options() {
        let c = client_with("medium", true);
        let wire = c.build_wire_request(&req_with(vec![ChatMessage::user("hi")]), true);
        let v = serde_json::to_value(&wire).unwrap();
        assert_eq!(v["thinking"]["type"], "enabled");
        assert_eq!(v["reasoning_effort"], "medium");
        assert_eq!(v["stream"], true);
        assert_eq!(v["stream_options"]["include_usage"], true);
        // temperature/top_p 禁传（mimo-2.6 强制覆盖）。
        assert!(v.get("temperature").is_none());
        assert!(v.get("top_p").is_none());

        let off = client_with("none", true);
        let v = serde_json::to_value(&off.build_wire_request(&req_with(vec![]), false)).unwrap();
        assert_eq!(v["thinking"]["type"], "disabled");
        assert!(
            v.get("stream_options").is_none(),
            "非流式不带 stream_options"
        );
    }

    #[test]
    fn reasoning_content_roundtrip_in_history_wire() {
        // 硬要求：assistant 历史消息的 reasoning_content 必须原样回传（缺失 400）。
        let assistant =
            ChatMessage::assistant("看完了", vec![]).with_reasoning_content("先想三步……");
        let mut call = ChatMessage::assistant(
            "",
            vec![ToolCallRequest {
                id: "call_1".to_string(),
                name: "read".to_string(),
                arguments_json: "{\"path\":\"a.txt\"}".to_string(),
            }],
        );
        call.reasoning_content = Some("带工具的思考".to_string());

        let wire_msgs: Vec<WireMessage> = vec![&assistant, &call]
            .into_iter()
            .map(to_wire_message)
            .collect();
        let json = serde_json::to_value(&wire_msgs).unwrap();
        assert_eq!(json[0]["reasoning_content"], "先想三步……");
        assert_eq!(json[1]["reasoning_content"], "带工具的思考");
        assert_eq!(json[1]["tool_calls"][0]["function"]["name"], "read");

        // roundtrip：wire → 核心形态，reasoning_content 不丢。
        for (wire, want) in [
            (wire_msgs[0].clone(), "先想三步……"),
            (wire_msgs[1].clone(), "带工具的思考"),
        ] {
            let back = from_wire_message(wire);
            assert_eq!(back.reasoning_content.as_deref(), Some(want));
        }
        // 序列化 roundtrip：核心 → wire JSON → wire → 核心，内容不丢。
        let text = serde_json::to_string(&to_wire_message(&assistant)).unwrap();
        let back = from_wire_message(serde_json::from_str(&text).unwrap());
        assert_eq!(back, assistant);

        // 无思考的普通消息不带 reasoning_content 字段。
        let plain = to_wire_message(&ChatMessage::user("hi"));
        let v = serde_json::to_value(&plain).unwrap();
        assert!(v.get("reasoning_content").is_none());
    }

    /// 按字节粒度喂 SSE（模拟网络分片），收集 data 载荷。
    fn feed_sse_by_bytes(parser: &mut SseParser, text: &str, step: usize) -> Vec<String> {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let end = (i + step).min(bytes.len());
            out.extend(parser.feed(&bytes[i..end]));
            i = end;
        }
        out
    }

    const MOCK_SSE: &str = "\
data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先想\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"三步\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"答案是\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"42\"}}]}\n\n\
data: {\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7},\"choices\":[]}\n\n\
data: [DONE]\n\n";

    #[test]
    fn mock_sse_two_path_stream_and_usage() {
        let rec = Rec::default();
        let mut parser = SseParser::new();
        let mut asm = StreamAssembler::new(3, Some(&rec));
        for payload in feed_sse_by_bytes(&mut parser, MOCK_SSE, 7) {
            if payload.trim() == "[DONE]" {
                continue;
            }
            asm.feed_chunk_json(&payload).unwrap();
        }
        let resp = asm.finish();
        assert_eq!(resp.reasoning_content, "先想三步");
        assert_eq!(resp.text, "答案是42");
        assert_eq!(resp.usage.as_ref().unwrap().prompt_tokens, 11);
        assert_eq!(resp.usage.as_ref().unwrap().completion_tokens, 7);
        // 两路 delta 都以 round=3 上抛，turn 恰收尾一次。
        assert_eq!(*rec.reasoning.lock().unwrap(), "先想三步");
        assert_eq!(*rec.text.lock().unwrap(), "答案是42");
        assert!(rec.rounds.lock().unwrap().iter().all(|r| *r == 3));
        assert_eq!(*rec.done_rounds.lock().unwrap(), vec![3]);
    }

    const MOCK_SSE_TOOLS: &str = "\
data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"要读文件\",\"tool_calls\":[{\"index\":0,\"id\":\"call_9\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"pa\"}}]}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":\\\"a.txt\"}}]}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"}\"}}]}}]}\n\n\
data: {\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":6},\"choices\":[]}\n\n\
data: [DONE]\n\n";

    #[test]
    fn mock_sse_tool_call_delta_accumulates_args() {
        let rec = Rec::default();
        let mut parser = SseParser::new();
        let mut asm = StreamAssembler::new(1, Some(&rec));
        for payload in feed_sse_by_bytes(&mut parser, MOCK_SSE_TOOLS, 11) {
            if payload.trim() == "[DONE]" {
                continue;
            }
            asm.feed_chunk_json(&payload).unwrap();
        }
        let resp = asm.finish();
        assert_eq!(resp.reasoning_content, "要读文件");
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_9");
        assert_eq!(resp.tool_calls[0].name, "read");
        assert_eq!(resp.tool_calls[0].arguments_json, "{\"path\":\"a.txt\"}");
        // args_so_far 累计值逐段上抛（3 段）。
        let tools = rec.tools.lock().unwrap().clone();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].1, "{\"pa");
        assert_eq!(tools[1].1, "{\"path\":\"a.txt");
        assert_eq!(tools[2].1, "{\"path\":\"a.txt\"}");
        assert!(tools.iter().all(|(n, _)| n == "read"));
        assert_eq!(*rec.done_rounds.lock().unwrap(), vec![1]);
    }

    #[test]
    fn sse_parser_handles_comments_crlf_and_split_frames() {
        let mut p = SseParser::new();
        let text = ": keepalive\r\ndata: {\"a\":1}\r\n\r\ndata: {\"b\":2}\n\n";
        // 逐字节喂：跨包切分不丢不重。
        let payloads = feed_sse_by_bytes(&mut p, text, 1);
        assert_eq!(
            payloads,
            vec!["{\"a\":1}".to_string(), "{\"b\":2}".to_string()]
        );
    }

    #[test]
    fn non_stream_response_parses_reasoning_content() {
        let json = r#"{"choices":[{"message":{"role":"assistant","content":"答","reasoning_content":"思","tool_calls":[{"id":"c1","function":{"name":"bash","arguments":"{}"}}]}}],"usage":{"prompt_tokens":3,"completion_tokens":4}}"#;
        let resp: WireChatResponse = serde_json::from_str(json).unwrap();
        let chat = wire_response_to_chat(resp).unwrap();
        assert_eq!(chat.reasoning_content, "思");
        assert_eq!(chat.text, "答");
        assert_eq!(chat.tool_calls[0].name, "bash");
        assert_eq!(chat.usage.unwrap().completion_tokens, 4);
    }

    #[test]
    fn retry_class_and_error_mapping() {
        // 传输级（网络/5xx/429）可重试，4xx 不可。
        assert!(is_retryable(&ModelError::Transport("x".into())));
        assert!(is_retryable(&map_http_error(429, "slow down")));
        assert!(is_retryable(&map_http_error(503, "upstream")));
        assert!(!is_retryable(&map_http_error(
            400,
            r#"{"error":{"message":"bad tool call"}}"#
        )));
        assert!(!is_retryable(&map_http_error(401, "unauthorized")));
        match map_http_error(400, r#"{"error":{"message":"参数缺失 reasoning_content"}}"#) {
            ModelError::BadResponse(m) => assert!(m.contains("参数缺失 reasoning_content")),
            other => panic!("400 应为 BadResponse，得到 {other:?}"),
        }
    }

    /// 真实端点流式探针（手动跑：
    /// cargo test -p sd-agent real_endpoint_stream_probe -- --ignored --nocapture）。
    /// 只报字段出现/字节数，不打印任何正文与凭据；不落盘任何东西。
    #[tokio::test]
    #[ignore = "真实端点探针：手动执行"]
    async fn real_endpoint_stream_probe() {
        use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

        struct Count {
            reasoning_bytes: AtomicUsize,
            text_bytes: AtomicUsize,
            reasoning_deltas: AtomicU64,
            text_deltas: AtomicU64,
            turns: AtomicU64,
            first_field: Mutex<String>,
        }
        impl StreamObserver for Count {
            fn on_reasoning_delta(&self, _r: u32, d: &str) {
                self.reasoning_bytes.fetch_add(d.len(), Ordering::SeqCst);
                self.reasoning_deltas.fetch_add(1, Ordering::SeqCst);
                let mut f = self.first_field.lock().unwrap();
                if f.is_empty() {
                    *f = "reasoning_content".to_string();
                }
            }
            fn on_text_delta(&self, _r: u32, d: &str) {
                self.text_bytes.fetch_add(d.len(), Ordering::SeqCst);
                self.text_deltas.fetch_add(1, Ordering::SeqCst);
                let mut f = self.first_field.lock().unwrap();
                if f.is_empty() {
                    *f = "content".to_string();
                }
            }
            fn on_tool_call_delta(&self, _r: u32, _n: &str, _a: &str) {}
            fn on_turn_done(&self, _r: u32) {
                self.turns.fetch_add(1, Ordering::SeqCst);
            }
        }

        let settings = Settings::load();
        let client = OpenAiCompatClient::from_settings(&settings).expect("配置齐全");
        let obs = Count {
            reasoning_bytes: AtomicUsize::new(0),
            text_bytes: AtomicUsize::new(0),
            reasoning_deltas: AtomicU64::new(0),
            text_deltas: AtomicU64::new(0),
            turns: AtomicU64::new(0),
            first_field: Mutex::new(String::new()),
        };

        let resp = client
            .chat_stream(
                ChatRequest {
                    messages: vec![ChatMessage::user("用一句话说明 1+1 为什么等于 2")],
                    tools: vec![],
                    round: 1,
                },
                &obs,
            )
            .await
            .expect("真实流式请求成功");

        println!(
            "探针①流式：reasoning_content 出现={} 字节数={}（delta {} 段）",
            !resp.reasoning_content.is_empty(),
            resp.reasoning_content.len(),
            obs.reasoning_deltas.load(Ordering::SeqCst)
        );
        println!(
            "探针①流式：content 字节数={}（delta {} 段），首个流出字段={}，on_turn_done={}，usage={:?}",
            resp.text.len(),
            obs.text_deltas.load(Ordering::SeqCst),
            obs.first_field.lock().unwrap(),
            obs.turns.load(Ordering::SeqCst),
            resp.usage
                .as_ref()
                .map(|u| (u.prompt_tokens, u.completion_tokens))
        );
        assert!(
            !resp.reasoning_content.is_empty(),
            "思考开启时 reasoning_content 必须流出"
        );

        // 探针②：reasoning_content 原样回传再问一轮（验证历史回传不 400）。
        let follow = ChatRequest {
            messages: vec![
                ChatMessage::user("用一句话说明 1+1 为什么等于 2"),
                ChatMessage::assistant(resp.text.clone(), vec![])
                    .with_reasoning_content(resp.reasoning_content.clone()),
                ChatMessage::user("谢谢，就回一个字：好"),
            ],
            tools: vec![],
            round: 2,
        };
        let resp2 = client
            .chat_stream(follow, &obs)
            .await
            .expect("reasoning_content 历史回传必须成功（否则即官方 400 场景）");
        println!(
            "探针②回传：成功，reasoning_content 字节数={}，content 字节数={}",
            resp2.reasoning_content.len(),
            resp2.text.len()
        );
    }
}
