//! 事件契约（决策 4：事件契约首日定死）。
//!
//! 每行 JSONL 一个事件，字段固定：
//! `{"v":1,"trace_id":"…","seq":…,"ts_unix_ms":…,"kind":"…","payload":{…}}`
//!
//! 演化规则（冻结，任何改动=破坏契约）：
//! 1. 字段只增、不改名、不改语义；旧 kind 的语义永久冻结；
//! 2. `v` 只在破坏性演进时递增，读端对高版本行的处置见 source.rs；
//! 3. payload 按 kind 强类型（Rust 侧 enum 变体），JSON 侧为该 kind 的扁平对象；
//! 4. 事件不得承载凭据（Secret 不可序列化，编译期保证）。
//!
//! 分级处置标：kind 的处置级别（审计留痕 / 需人工关注 / 失败终止）
//! 以 `EventKind::disposition()` 显式给出，供观察面与 P1 状态机取数。

use serde::{Deserialize, Serialize};

/// 契约版本号。
pub const CONTRACT_VERSION: u32 = 1;

/// 事件类别（序列化为 snake_case 字符串）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    RunStarted,
    ModelTurnStarted,
    ModelTurnFinished,
    ToolCallRequested,
    ToolApprovalRequested,
    ToolApprovalResolved,
    ToolCallStarted,
    ToolCallFinished,
    ToolCallDenied,
    RunFinished,
    RunFailed,
    VerifyResult,
    Hook,
}

/// 分级处置标（契约字段之一）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// 事实留痕，仅审计。
    Audit,
    /// 需人工/观察面关注。
    Attention,
    /// 终止性事件。
    Terminal,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::RunStarted => "run_started",
            EventKind::ModelTurnStarted => "model_turn_started",
            EventKind::ModelTurnFinished => "model_turn_finished",
            EventKind::ToolCallRequested => "tool_call_requested",
            EventKind::ToolApprovalRequested => "tool_approval_requested",
            EventKind::ToolApprovalResolved => "tool_approval_resolved",
            EventKind::ToolCallStarted => "tool_call_started",
            EventKind::ToolCallFinished => "tool_call_finished",
            EventKind::ToolCallDenied => "tool_call_denied",
            EventKind::RunFinished => "run_finished",
            EventKind::RunFailed => "run_failed",
            EventKind::VerifyResult => "verify_result",
            EventKind::Hook => "hook",
        }
    }

    pub fn disposition(&self) -> Disposition {
        match self {
            EventKind::ToolApprovalRequested | EventKind::ToolCallDenied | EventKind::RunFailed => {
                Disposition::Attention
            }
            EventKind::RunFinished => Disposition::Terminal,
            _ => Disposition::Audit,
        }
    }

    /// 从 wire 字符串解析（source.rs 反序列化用）。
    pub fn from_str_lossy(s: &str) -> Option<Self> {
        match s {
            "run_started" => Some(EventKind::RunStarted),
            "model_turn_started" => Some(EventKind::ModelTurnStarted),
            "model_turn_finished" => Some(EventKind::ModelTurnFinished),
            "tool_call_requested" => Some(EventKind::ToolCallRequested),
            "tool_approval_requested" => Some(EventKind::ToolApprovalRequested),
            "tool_approval_resolved" => Some(EventKind::ToolApprovalResolved),
            "tool_call_started" => Some(EventKind::ToolCallStarted),
            "tool_call_finished" => Some(EventKind::ToolCallFinished),
            "tool_call_denied" => Some(EventKind::ToolCallDenied),
            "run_finished" => Some(EventKind::RunFinished),
            "run_failed" => Some(EventKind::RunFailed),
            "verify_result" => Some(EventKind::VerifyResult),
            "hook" => Some(EventKind::Hook),
            _ => None,
        }
    }
}

/// 强类型 payload：每个 kind 对应一个变体，JSON 侧为该 kind 的扁平对象。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EventPayload {
    // 未加 tag 的 untagged 不用于 wire（wire 由 Event 手写序列化保证 kind/payload 平级）；
    // 此处 untagged 仅为 derive Deserialize 的兜底不可用性，实际解析走 payload_for_kind()。
    RunStarted(RunStarted),
    ModelTurnStarted(ModelTurnStarted),
    ModelTurnFinished(ModelTurnFinished),
    ToolCallRequested(ToolCallRequested),
    ToolApprovalRequested(ToolApprovalRequested),
    ToolApprovalResolved(ToolApprovalResolved),
    ToolCallStarted(ToolCallStarted),
    ToolCallFinished(ToolCallFinished),
    ToolCallDenied(ToolCallDenied),
    RunFinished(RunFinished),
    RunFailed(RunFailed),
    VerifyResult(VerifyResult),
    Hook(Hook),
}

// ---- payload 强类型结构（字段即契约，只增不改名）----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStarted {
    pub task: String,
    pub max_rounds: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelTurnStarted {
    pub round: u32,
    pub history_len: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelTurnFinished {
    pub round: u32,
    pub text_chars: usize,
    pub tool_call_count: usize,
    /// 本轮思维链正文字符数（reasoning_content；成本账地基，0=无思考）。
    /// 只增字段：旧轨迹行缺字段时按 0 解析（serde default）。
    #[serde(default)]
    pub reasoning_chars: usize,
    /// 本轮 prompt tokens（usage；无 usage 为 None）。
    #[serde(default)]
    pub usage_prompt_tokens: Option<u64>,
    /// 本轮 completion tokens（usage；无 usage 为 None）。
    #[serde(default)]
    pub usage_completion_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRequested {
    pub tool_call_id: String,
    pub tool: String,
    /// 参数 JSON 原文（审计留痕；轨迹文件不入库）。
    pub args_json: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolApprovalRequested {
    pub tool_call_id: String,
    pub tool: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolApprovalResolved {
    pub tool_call_id: String,
    pub approved: bool,
    /// 裁决来源（cli / tui / desktop / auto）。
    pub by: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallStarted {
    pub tool_call_id: String,
    pub tool: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallFinished {
    pub tool_call_id: String,
    pub tool: String,
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    /// 结果摘要（工具原文输出的摘要/截断形态，入事件不炸轨迹）。
    pub result_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallDenied {
    pub tool_call_id: String,
    pub tool: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunFinished {
    pub rounds: u32,
    /// 收尾状态：completed / max_rounds_reached。
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunFailed {
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyResult {
    /// 探针名。
    pub name: String,
    pub ok: bool,
    pub exit_code: Option<i32>,
    /// 证据文件路径（相对工作区）。
    pub evidence_path: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hook {
    pub hook: String,
    pub note: String,
}

impl EventPayload {
    pub fn kind(&self) -> EventKind {
        match self {
            EventPayload::RunStarted(_) => EventKind::RunStarted,
            EventPayload::ModelTurnStarted(_) => EventKind::ModelTurnStarted,
            EventPayload::ModelTurnFinished(_) => EventKind::ModelTurnFinished,
            EventPayload::ToolCallRequested(_) => EventKind::ToolCallRequested,
            EventPayload::ToolApprovalRequested(_) => EventKind::ToolApprovalRequested,
            EventPayload::ToolApprovalResolved(_) => EventKind::ToolApprovalResolved,
            EventPayload::ToolCallStarted(_) => EventKind::ToolCallStarted,
            EventPayload::ToolCallFinished(_) => EventKind::ToolCallFinished,
            EventPayload::ToolCallDenied(_) => EventKind::ToolCallDenied,
            EventPayload::RunFinished(_) => EventKind::RunFinished,
            EventPayload::RunFailed(_) => EventKind::RunFailed,
            EventPayload::VerifyResult(_) => EventKind::VerifyResult,
            EventPayload::Hook(_) => EventKind::Hook,
        }
    }
}

/// 契约事件：六个固定字段。序列化手写，保证 kind 与 payload 平级且
/// 字段名与顺序 = 契约定义（serde derive 的 tag/content 布局不能满足
/// 该形态，这里按契约实现，字节稳定由测试断言）。
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub v: u32,
    pub trace_id: String,
    pub seq: u64,
    pub ts_unix_ms: u64,
    pub kind: EventKind,
    pub payload: EventPayload,
}

impl Event {
    /// 构造：kind 由 payload 推导，杜绝 kind/payload 不一致。
    pub fn new(
        trace_id: impl Into<String>,
        seq: u64,
        ts_unix_ms: u64,
        payload: EventPayload,
    ) -> Self {
        let kind = payload.kind();
        Self {
            v: CONTRACT_VERSION,
            trace_id: trace_id.into(),
            seq,
            ts_unix_ms,
            kind,
            payload,
        }
    }

    /// 单行 JSON 形态（不含换行）。
    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self).expect("event serializable")
    }
}

impl Serialize for Event {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(6))?;
        map.serialize_entry("v", &self.v)?;
        map.serialize_entry("trace_id", &self.trace_id)?;
        map.serialize_entry("seq", &self.seq)?;
        map.serialize_entry("ts_unix_ms", &self.ts_unix_ms)?;
        map.serialize_entry("kind", self.kind.as_str())?;
        map.serialize_entry("payload", &self.payload)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        #[derive(Deserialize)]
        struct WireEvent {
            v: u32,
            trace_id: String,
            seq: u64,
            ts_unix_ms: u64,
            kind: String,
            payload: serde_json::Value,
        }
        let wire = WireEvent::deserialize(deserializer)?;
        let kind = EventKind::from_str_lossy(&wire.kind)
            .ok_or_else(|| D::Error::custom(format!("unknown event kind: {}", wire.kind)))?;
        let payload = payload_from_wire(kind, wire.payload).map_err(D::Error::custom)?;
        Ok(Event {
            v: wire.v,
            trace_id: wire.trace_id,
            seq: wire.seq,
            ts_unix_ms: wire.ts_unix_ms,
            kind,
            payload,
        })
    }
}

/// kind → payload 强类型解析（wire 兼容层；旧 kind 语义冻结，只增不改）。
fn payload_from_wire(kind: EventKind, value: serde_json::Value) -> Result<EventPayload, String> {
    macro_rules! parse_as {
        ($t:ty, $ctor:path) => {
            serde_json::from_value::<$t>(value)
                .map($ctor)
                .map_err(|e| format!("payload mismatch for {}: {e}", kind.as_str()))?
        };
    }
    Ok(match kind {
        EventKind::RunStarted => parse_as!(RunStarted, EventPayload::RunStarted),
        EventKind::ModelTurnStarted => parse_as!(ModelTurnStarted, EventPayload::ModelTurnStarted),
        EventKind::ModelTurnFinished => {
            parse_as!(ModelTurnFinished, EventPayload::ModelTurnFinished)
        }
        EventKind::ToolCallRequested => {
            parse_as!(ToolCallRequested, EventPayload::ToolCallRequested)
        }
        EventKind::ToolApprovalRequested => {
            parse_as!(ToolApprovalRequested, EventPayload::ToolApprovalRequested)
        }
        EventKind::ToolApprovalResolved => {
            parse_as!(ToolApprovalResolved, EventPayload::ToolApprovalResolved)
        }
        EventKind::ToolCallStarted => parse_as!(ToolCallStarted, EventPayload::ToolCallStarted),
        EventKind::ToolCallFinished => parse_as!(ToolCallFinished, EventPayload::ToolCallFinished),
        EventKind::ToolCallDenied => parse_as!(ToolCallDenied, EventPayload::ToolCallDenied),
        EventKind::RunFinished => parse_as!(RunFinished, EventPayload::RunFinished),
        EventKind::RunFailed => parse_as!(RunFailed, EventPayload::RunFailed),
        EventKind::VerifyResult => parse_as!(VerifyResult, EventPayload::VerifyResult),
        EventKind::Hook => parse_as!(Hook, EventPayload::Hook),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Event {
        Event::new(
            "trace-x",
            7,
            1728000000000,
            EventPayload::RunStarted(RunStarted {
                task: "demo".into(),
                max_rounds: 20,
            }),
        )
    }

    #[test]
    fn wire_shape_matches_contract() {
        let line = sample().to_json_line();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["trace_id"], "trace-x");
        assert_eq!(v["seq"], 7);
        assert_eq!(v["ts_unix_ms"], 1728000000000u64);
        assert_eq!(v["kind"], "run_started");
        assert_eq!(v["payload"]["task"], "demo");
        // 字段序固定（契约字节稳定）
        assert!(line.starts_with(r#"{"v":1,"trace_id":"trace-x","seq":7,"ts_unix_ms":1728000000000,"kind":"run_started","payload":{"#));
    }

    #[test]
    fn roundtrip_preserves_event() {
        let ev = sample();
        let line = ev.to_json_line();
        let back: Event = serde_json::from_str(&line).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn two_serializations_byte_identical() {
        let ev = sample();
        assert_eq!(ev.to_json_line(), ev.to_json_line());
    }

    #[test]
    fn unknown_kind_rejected() {
        let line =
            r#"{"v":1,"trace_id":"t","seq":0,"ts_unix_ms":0,"kind":"future_kind","payload":{}}"#;
        assert!(serde_json::from_str::<Event>(line).is_err());
    }

    #[test]
    fn model_turn_finished_new_fields_are_additive() {
        // 新字段（reasoning_chars / usage_*）随行序列化，roundtrip 不丢。
        let ev = Event::new(
            "trace-y",
            1,
            0,
            EventPayload::ModelTurnFinished(ModelTurnFinished {
                round: 2,
                text_chars: 10,
                tool_call_count: 1,
                reasoning_chars: 321,
                usage_prompt_tokens: Some(11),
                usage_completion_tokens: Some(7),
            }),
        );
        let back: Event = serde_json::from_str(&ev.to_json_line()).unwrap();
        assert_eq!(ev, back);
        if let EventPayload::ModelTurnFinished(p) = &back.payload {
            assert_eq!(p.reasoning_chars, 321);
            assert_eq!(p.usage_prompt_tokens, Some(11));
            assert_eq!(p.usage_completion_tokens, Some(7));
        } else {
            panic!("kind 应为 model_turn_finished");
        }
    }

    #[test]
    fn old_model_turn_finished_rows_still_parse() {
        // 兼容冻结：旧轨迹行没有新字段 → reasoning_chars=0、usage=None。
        let line = r#"{"v":1,"trace_id":"t","seq":0,"ts_unix_ms":0,"kind":"model_turn_finished","payload":{"round":1,"text_chars":5,"tool_call_count":0}}"#;
        let ev: Event = serde_json::from_str(line).unwrap();
        if let EventPayload::ModelTurnFinished(p) = &ev.payload {
            assert_eq!(p.reasoning_chars, 0);
            assert_eq!(p.usage_prompt_tokens, None);
            assert_eq!(p.usage_completion_tokens, None);
        } else {
            panic!("kind 应为 model_turn_finished");
        }
    }
}
