//! 事件层稳定门面：公共类型 re-export，路径永不变（决策 4 / 审查 #13）。
//! 未来拆文件不改公共类型路径，消费方一律 `use sd_agent::event::*`。

pub mod contract;
pub mod hooks;
pub mod sink;
pub mod source;

pub use contract::{
    CONTRACT_VERSION, Disposition, Event, EventKind, EventPayload, Hook, ModelTurnFinished,
    ModelTurnStarted, RunFailed, RunFinished, RunStarted, ToolApprovalRequested,
    ToolApprovalResolved, ToolCallDenied, ToolCallFinished, ToolCallRequested, ToolCallStarted,
    VerifyResult,
};
pub use hooks::{LifecycleHook, emit_hook};
pub use sink::{EventSink, JsonlSink, NullSink, SinkError, TraceRecorder, now_unix_ms};
pub use source::{EventSource, JsonlSource, ReplayOutcome, SourceError};
