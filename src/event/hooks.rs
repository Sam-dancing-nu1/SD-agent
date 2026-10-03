//! 生命周期钩子唯一收敛点（硬约束 14：全部生命周期钩子必须经单一调度入口；
//! 审查 #17：不得把"单一调度入口"偷换成 dispatch 点——dispatch 是执行入口，
//! 本模块是钩子调度入口，两者正交）。
//!
//! P0 口径：emit_hook() 只落盘（转为 hook 事件）；P1 起观察者异步扇出，
//! 同步只允许存在于决策链（硬约束 14）。

use super::contract::EventPayload;
use super::sink::{SinkError, TraceRecorder};

/// 生命周期钩子点位（枚举只增不删）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleHook {
    RunStart,
    RoundStart,
    RoundEnd,
    ToolBefore,
    ToolAfter,
    VerifyBefore,
    VerifyAfter,
    RunEnd,
}

impl LifecycleHook {
    pub fn as_str(&self) -> &'static str {
        match self {
            LifecycleHook::RunStart => "run_start",
            LifecycleHook::RoundStart => "round_start",
            LifecycleHook::RoundEnd => "round_end",
            LifecycleHook::ToolBefore => "tool_before",
            LifecycleHook::ToolAfter => "tool_after",
            LifecycleHook::VerifyBefore => "verify_before",
            LifecycleHook::VerifyAfter => "verify_after",
            LifecycleHook::RunEnd => "run_end",
        }
    }
}

/// 钩子唯一收敛点：任何生命周期通知必须经此函数，禁止旁路发事件。
///（P0 观察者=落盘 sink；扇出挂接点就位后仍从这里分发。）
pub fn emit_hook(
    recorder: &TraceRecorder,
    hook: LifecycleHook,
    note: &str,
) -> Result<(), SinkError> {
    recorder.record(EventPayload::Hook(super::contract::Hook {
        hook: hook.as_str().to_string(),
        note: note.to_string(),
    }))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::sink::NullSink;
    use std::sync::Arc;

    #[test]
    fn hook_records_through_single_point() {
        let rec = TraceRecorder::new("t", Arc::new(NullSink));
        emit_hook(&rec, LifecycleHook::RunStart, "hello").unwrap();
        assert_eq!(rec.next_seq(), 1);
    }
}
