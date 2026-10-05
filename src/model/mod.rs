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

use std::future::Future;
use std::pin::Pin;

use crate::tools::ToolSpec;

mod client;
mod stream;
mod wire;

pub use client::OpenAiCompatClient;

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
