// ---- wire 形态（OpenAI 兼容协议 + MiMo reasoning_content / thinking 扩展）----

use serde::{Deserialize, Serialize};

use super::{ChatMessage, ChatResponse, ModelError, Role, ToolCallRequest, Usage};

#[derive(Debug, Clone, Serialize)]
pub(super) struct WireChatRequest {
    pub(super) model: String,
    pub(super) messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tools: Option<Vec<WireTool>>,
    pub(super) max_tokens: u32,
    pub(super) stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) stream_options: Option<WireStreamOptions>,
    /// MiMo 非标字段（官方 deep-thinking 页）：{"type":"enabled"|"disabled"}。
    pub(super) thinking: WireThinking,
    /// 原思考强度字段照传（官方声明无害；以 thinking.type 为准）。
    pub(super) reasoning_effort: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct WireThinking {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct WireStreamOptions {
    pub(super) include_usage: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct WireTool {
    #[serde(rename = "type")]
    pub(super) tool_type: String,
    pub(super) function: WireToolFunction,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct WireToolFunction {
    pub(super) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) parameters: Option<serde_json::Value>,
}

/// 消息 wire 形态（请求序列化 + 响应反序列化共用）。
/// assistant 消息恒带 reasoning_content（官方硬要求：历史 reasoning_content
/// 必须回传，缺失 400；MiMo 优先适配）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireMessage {
    pub(super) role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) tool_calls: Option<Vec<WireToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireToolCall {
    pub(super) id: String,
    pub(super) function: WireFnCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireFnCall {
    pub(super) name: String,
    pub(super) arguments: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct WireChatResponse {
    #[serde(default)]
    pub(super) choices: Vec<WireChoice>,
    #[serde(default)]
    pub(super) usage: Option<WireUsage>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WireChoice {
    #[serde(default)]
    pub(super) message: WireResponseMessage,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct WireResponseMessage {
    #[serde(default)]
    pub(super) content: Option<String>,
    #[serde(default)]
    pub(super) reasoning_content: Option<String>,
    #[serde(default)]
    pub(super) tool_calls: Option<Vec<WireToolCall>>,
}

/// 流式 chunk（choices 可为空——usage 收尾 chunk 只带 usage）。
#[derive(Debug, Deserialize)]
pub(super) struct WireChunk {
    #[serde(default)]
    pub(super) choices: Vec<WireChunkChoice>,
    #[serde(default)]
    pub(super) usage: Option<WireUsage>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WireChunkChoice {
    #[serde(default)]
    pub(super) delta: WireDelta,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct WireDelta {
    #[serde(default)]
    pub(super) reasoning_content: Option<String>,
    #[serde(default)]
    pub(super) content: Option<String>,
    #[serde(default)]
    pub(super) tool_calls: Option<Vec<WireToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WireToolCallDelta {
    #[serde(default)]
    pub(super) index: usize,
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) function: Option<WireFnDelta>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WireFnDelta {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) arguments: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct WireUsage {
    #[serde(default)]
    pub(super) prompt_tokens: u64,
    #[serde(default)]
    pub(super) completion_tokens: u64,
}

/// 核心消息形态 → wire 形态（唯一转换点；wire 细节不出 model.rs）。
/// assistant 消息恒带 reasoning_content（历史回传硬要求；无思考时空串）。
pub(super) fn to_wire_message(msg: &ChatMessage) -> WireMessage {
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
pub(super) fn wire_response_to_chat(
    response: WireChatResponse,
) -> Result<ChatResponse, ModelError> {
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
pub(super) fn map_openai_error(err: async_openai::error::OpenAIError) -> ModelError {
    use async_openai::error::OpenAIError as E;
    match err {
        E::ApiError(resp) => map_http_error(resp.status_code.as_u16(), &resp.api_error.message),
        E::Reqwest(e) => ModelError::Transport(e.to_string()),
        other => ModelError::BadResponse(other.to_string()),
    }
}

/// HTTP 状态 + 错误体 → ModelError（429/5xx → Transport 可重试；4xx → BadResponse）。
pub(super) fn map_http_error(status: u16, body: &str) -> ModelError {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
