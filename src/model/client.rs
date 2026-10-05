use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::config::{Secret, Settings};

use super::stream::{SseParser, StreamAssembler};
use super::wire::{
    WireChatRequest, WireChatResponse, WireStreamOptions, WireThinking, WireTool, WireToolFunction,
    map_http_error, map_openai_error, to_wire_message, wire_response_to_chat,
};
use super::{ChatRequest, ChatResponse, ModelClient, ModelError, StreamObserver};

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
                Some(WireStreamOptions {
                    include_usage: true,
                })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ChatMessage;
    use crate::model::wire::map_http_error;
    use std::sync::Mutex;

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
