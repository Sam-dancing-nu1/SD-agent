//! policy：dispatch 裁决——**唯一模型驱动执行入口**（决策 3 / 审查 #2）。
//!
//! 模型驱动的工具执行只许经过 dispatch()；tools 执行面经 `tools::run_tool()`
//! 单接口收口（机制形态磨合见 decisions.md"决策 3 机制形态实现磨合记录"：
//! pub(in crate::policy) 受 E0742 限制不可行，落为 execute=pub(super) +
//! run_tool=pub(crate)；收口强度=单接口+审查约定，非编译期绝对闭合）。
//! 注意：这是"模型驱动执行"的唯一入口，不是"进程内唯一执行通道"——
//! verify / doctor 的确定性执行（git、探针）是 Harness 侧独立窄通道，
//! 显式声明、不经模型路径（doctor 的工具探针例外：走 dispatch 实测真通道）。
//!
//! 裁决流（硬约束 11/12/13：路由、风控、放行由探针贴标签、查表、代码裁决，
//! 模型不参与对自身行为的裁决）：
//! 白名单查表 → 参数强类型解析 → 危险命令规则表 → ApprovalPort 往返 → 执行。
//! 全程事件留痕（ToolCall* 事件）+ 生命周期钩子经 hooks::emit_hook。

pub mod approval;
pub mod rules;

use std::path::Path;

use crate::config::{DisciplineTables, EnvProfile, resource::ResourceLedger};
use crate::event::{
    EventPayload, LifecycleHook, SinkError, ToolApprovalRequested, ToolApprovalResolved,
    ToolCallDenied, ToolCallFinished, ToolCallRequested, ToolCallStarted, TraceRecorder, emit_hook,
};

pub use approval::{
    ApprovalDecision, ApprovalPort, ApprovalRequest, AutoApproval, CliApproval, DenyAll,
    RunApprovalState,
};
pub use rules::{CombinedPattern, CommandRules, DANGEROUS_PATTERNS, matched_dangerous_pattern};

/// 一次模型工具调用（模型输出 → 强类型）。
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// 参数 JSON 原文（保留原文供审计；解析失败按拒绝处置）。
    pub args_json: String,
}

/// dispatch 的执行结果（喂回模型的 tool message 依据）。
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub ok: bool,
    pub denied: bool,
    pub exit_code: Option<i32>,
    pub text: String,
}

/// 裁决上下文（显式依赖，不引全局状态）。
pub struct PolicyContext<'a> {
    pub env: &'a EnvProfile,
    pub root: &'a Path,
    pub recorder: &'a TraceRecorder,
    pub approval: &'a dyn ApprovalPort,
    pub ledger: &'a ResourceLedger,
    pub tables: &'a DisciplineTables,
}

/// 拒绝熔断阈值（问题③）：同一 run 内连续被拒达该值即收尾，防模型无限重试
/// 被拒工具直到轮数耗尽。口径=连续拒绝（成功执行清零）。
/// 数据位挂在 DisciplineTables 阈值表（config 区被占用期间以关联常量挂载，
/// 归位合入 config/mod.rs 阈值表待接）。
impl crate::config::DisciplineTables {
    pub const DENIAL_STREAK_LIMIT: usize = 5;
}

/// 熔断触发后的统一文案（ToolResult 与事件 reason 都带，含任务口径原话）。
const DENIAL_BREAKER_TEXT: &str = "连续被拒已达上限，停止重试";

/// 唯一模型驱动执行入口。所有生命周期事件与工具事件在此收敛发射。
pub async fn dispatch(ctx: &PolicyContext<'_>, call: &ToolCall) -> Result<ToolResult, SinkError> {
    // 事实事件：模型请求了什么（含参数原文，审计留痕）。
    ctx.recorder
        .record(EventPayload::ToolCallRequested(ToolCallRequested {
            tool_call_id: call.id.clone(),
            tool: call.name.clone(),
            args_json: call.args_json.clone(),
        }))?;

    // 0. 连续拒绝熔断（问题③）：达阈值后本 run 提前收尾——后续调用一律短路
    //（不再执行、不再询问），防止模型无限重试被拒工具直到轮数耗尽。
    if let Some(state) = ctx.approval.run_state() {
        if state.denial_streak() >= DisciplineTables::DENIAL_STREAK_LIMIT {
            ctx.recorder
                .record(EventPayload::ToolCallDenied(ToolCallDenied {
                    tool_call_id: call.id.clone(),
                    tool: call.name.clone(),
                    reason: DENIAL_BREAKER_TEXT.to_string(),
                }))?;
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                ok: false,
                denied: true,
                exit_code: None,
                text: DENIAL_BREAKER_TEXT.to_string(),
            });
        }
    }

    // 1. 白名单查表（硬约束 11：查表裁决，零模型成本）。
    if !ctx.tables.allows(&call.name) {
        return deny(ctx, call, format!("tool not in whitelist: {}", call.name)).await;
    }

    // 2. 参数强类型解析。
    let args: serde_json::Value = match serde_json::from_str(&call.args_json) {
        Ok(v) => v,
        Err(e) => return deny(ctx, call, format!("invalid args json: {e}")).await,
    };

    // 3. 危险命令规则表（纯数据、按平台分行）。
    if call.name == "bash" {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if let Some(pattern) = matched_dangerous_pattern(ctx.env.os_family, command) {
            return deny(
                ctx,
                call,
                format!("dangerous command pattern matched: `{pattern}`"),
            )
            .await;
        }
    }

    // 4. 审批往返（read 免审；其余一律过 ApprovalPort——裁决由人/壳作出）。
    // AlwaysAllow 消费（问题③）：端口置过放行位后，本 run 内后续非只读工具
    // 免询问；事件仍照发（by 如实记 "always"，审计不丢）。
    if call.name != "read" {
        let summary = approval_summary(&call.name, &args);
        ctx.recorder
            .record(EventPayload::ToolApprovalRequested(ToolApprovalRequested {
                tool_call_id: call.id.clone(),
                tool: call.name.clone(),
                summary: summary.clone(),
            }))?;
        let bypass = ctx
            .approval
            .run_state()
            .map(|s| s.allow_all())
            .unwrap_or(false);
        let decision = if bypass {
            ApprovalDecision::AlwaysAllow
        } else {
            ctx.approval.approve(ApprovalRequest {
                tool_call_id: call.id.clone(),
                tool: call.name.clone(),
                summary,
                detail: call.args_json.clone(),
            })
        };
        // AlwaysAllow 消费：置 run 级放行位（后续非只读工具不再询问）。
        if decision == ApprovalDecision::AlwaysAllow {
            if let Some(state) = ctx.approval.run_state() {
                state.engage_allow_all();
            }
        }
        // by 如实记：AlwaysAllow（含放行位直通）= "always"，否则记端口来源。
        let by = if decision == ApprovalDecision::AlwaysAllow {
            decision.source_tag()
        } else {
            ctx.approval.source()
        };
        ctx.recorder
            .record(EventPayload::ToolApprovalResolved(ToolApprovalResolved {
                tool_call_id: call.id.clone(),
                approved: decision.is_approved(),
                by: by.to_string(),
            }))?;
        if !decision.is_approved() {
            return deny(ctx, call, "user denied the tool call".to_string()).await;
        }
    }

    // 5. 执行（唯一通道：tools::run_tool，收口机制见 tools/mod.rs 文件头）。
    ctx.recorder
        .record(EventPayload::ToolCallStarted(ToolCallStarted {
            tool_call_id: call.id.clone(),
            tool: call.name.clone(),
        }))?;
    emit_hook(ctx.recorder, LifecycleHook::ToolBefore, &call.name)?;

    let started = std::time::Instant::now();
    let outcome = crate::tools::run_tool(ctx.root, ctx.env, &call.name, &args).await;
    let duration_ms = started.elapsed().as_millis() as u64;

    emit_hook(ctx.recorder, LifecycleHook::ToolAfter, &call.name)?;

    let (ok, exit_code, text) = match outcome {
        Ok(out) => (out.ok, out.exit_code, out.text),
        Err(e) => (false, None, format!("tool execution error: {e}")),
    };
    // 非文件类副作用登记点（硬约束 20）：bash 起子进程即登记。
    if call.name == "bash" {
        ctx.ledger.register(
            "subprocess",
            &format!("bash tool_call_id={}", call.id),
            crate::event::now_unix_ms(),
        );
    }

    // 非拒绝路径：连续拒绝计数清零（熔断只针对连续被拒，问题③）。
    if let Some(state) = ctx.approval.run_state() {
        state.note_success();
    }

    // 结果摘要阈值真的用起来（问题⑥）：按 DisciplineTables 数据位截断。
    let digest = digest_of(&text, ctx.tables.output_digest_threshold_bytes);
    ctx.recorder
        .record(EventPayload::ToolCallFinished(ToolCallFinished {
            tool_call_id: call.id.clone(),
            tool: call.name.clone(),
            ok,
            exit_code,
            duration_ms,
            result_digest: digest,
        }))?;

    Ok(ToolResult {
        tool_call_id: call.id.clone(),
        ok,
        denied: false,
        exit_code,
        text,
    })
}

/// 拒绝路径（白名单 / 参数 / 规则 / 审批），统一事件形态。
/// 连续拒绝计数（问题③）：同 run 内连续被拒达阈值后文案带熔断提示
///（调用方下一次进入 dispatch 即被短路收尾）。
async fn deny(
    ctx: &PolicyContext<'_>,
    call: &ToolCall,
    reason: String,
) -> Result<ToolResult, SinkError> {
    let streak = ctx
        .approval
        .run_state()
        .map(|s| s.note_denial())
        .unwrap_or(0);
    let tripped = streak >= DisciplineTables::DENIAL_STREAK_LIMIT;
    let reason = if tripped {
        format!("{reason}; {DENIAL_BREAKER_TEXT}")
    } else {
        reason
    };
    ctx.recorder
        .record(EventPayload::ToolCallDenied(ToolCallDenied {
            tool_call_id: call.id.clone(),
            tool: call.name.clone(),
            reason: reason.clone(),
        }))?;
    Ok(ToolResult {
        tool_call_id: call.id.clone(),
        ok: false,
        denied: true,
        exit_code: None,
        text: format!("DENIED: {reason}"),
    })
}

/// 审批摘要（UI 展示一行；detail 另给）。
fn approval_summary(name: &str, args: &serde_json::Value) -> String {
    match name {
        "bash" => format!(
            "bash: {}",
            truncate(
                args.get("command").and_then(|v| v.as_str()).unwrap_or(""),
                120
            )
        ),
        "write" => format!(
            "write: {} ({} bytes)",
            args.get("path").and_then(|v| v.as_str()).unwrap_or("?"),
            args.get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.len())
                .unwrap_or(0)
        ),
        "edit" => format!(
            "edit: {} (replace `{}`)",
            args.get("path").and_then(|v| v.as_str()).unwrap_or("?"),
            truncate(
                args.get("old_string")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
                60
            )
        ),
        other => format!("{other}: (no summary)"),
    }
}

/// 结果摘要：按阈值截断 + 长度记账（阈值来自
/// `DisciplineTables::output_digest_threshold_bytes`——问题⑥：数据位真的生效）。
fn digest_of(text: &str, threshold_bytes: usize) -> String {
    if text.len() <= threshold_bytes {
        return text.to_string();
    }
    format!(
        "{}…[digest: {} bytes total]",
        truncate(text, threshold_bytes),
        text.len()
    )
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    // char-boundary 安全截断复用 sys 单点（多字节字符不 panic）。
    format!("{}…", crate::sys::truncate_char_boundary(s, max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DisciplineTables;
    use crate::config::resource::ResourceLedger;
    use crate::event::{JsonlSink, TraceRecorder};
    use std::sync::Arc;

    fn ctx<'a>(
        env: &'a EnvProfile,
        root: &'a Path,
        rec: &'a TraceRecorder,
        ledger: &'a ResourceLedger,
        tables: &'a DisciplineTables,
        approval: &'a dyn ApprovalPort,
    ) -> PolicyContext<'a> {
        PolicyContext {
            env,
            root,
            recorder: rec,
            approval,
            ledger,
            tables,
        }
    }

    #[tokio::test]
    async fn dispatch_denies_dangerous_bash() {
        let dir = std::env::temp_dir().join(format!("sd-policy-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".sd-agent/traces")).unwrap();
        let sink = Arc::new(JsonlSink::open(dir.join(".sd-agent/traces/t.jsonl")).unwrap());
        let rec = TraceRecorder::new("t", sink);
        let env = EnvProfile::detect();
        let ledger = ResourceLedger::new();
        let tables = DisciplineTables::p0();
        let c = ctx(&env, &dir, &rec, &ledger, &tables, &AutoApproval);
        let res = dispatch(
            &c,
            &ToolCall {
                id: "c1".into(),
                name: "bash".into(),
                args_json: r#"{"command":"rm -rf /"}"#.into(),
            },
        )
        .await
        .unwrap();
        assert!(res.denied);
        assert!(res.text.contains("DENIED"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dispatch_runs_read_through_real_channel() {
        let dir = std::env::temp_dir().join(format!("sd-policy-test2-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".sd-agent/traces")).unwrap();
        std::fs::write(dir.join("probe.txt"), "probe-ok").unwrap();
        let sink = Arc::new(JsonlSink::open(dir.join(".sd-agent/traces/t.jsonl")).unwrap());
        let rec = TraceRecorder::new("t", sink);
        let env = EnvProfile::detect();
        let ledger = ResourceLedger::new();
        let tables = DisciplineTables::p0();
        let c = ctx(&env, &dir, &rec, &ledger, &tables, &AutoApproval);
        let res = dispatch(
            &c,
            &ToolCall {
                id: "c2".into(),
                name: "read".into(),
                args_json: r#"{"path":"probe.txt"}"#.into(),
            },
        )
        .await
        .unwrap();
        assert!(res.ok);
        assert!(res.text.contains("probe-ok"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 带 run 状态与调用计数的测试裁决口。
    struct CountingPort {
        decision: ApprovalDecision,
        state: crate::policy::approval::RunApprovalState,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl CountingPort {
        fn new(decision: ApprovalDecision) -> Self {
            Self {
                decision,
                state: crate::policy::approval::RunApprovalState::new(),
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl ApprovalPort for CountingPort {
        fn approve(&self, _request: ApprovalRequest) -> ApprovalDecision {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.decision
        }

        fn source(&self) -> &'static str {
            "test_port"
        }

        fn run_state(&self) -> Option<&crate::policy::approval::RunApprovalState> {
            Some(&self.state)
        }
    }

    fn write_call(id: &str, path: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "write".into(),
            args_json: format!(r#"{{"path":"{path}","content":"x"}}"#),
        }
    }

    #[tokio::test]
    async fn always_allow_is_consumed_within_run() {
        // 问题③：AlwaysAllow 不再是假承诺——本次 run 内后续非只读工具免询问，
        // 但事件照发且 by 如实记 "always"（审计不丢）。
        let dir = std::env::temp_dir().join(format!("sd-policy-test3-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".sd-agent/traces")).unwrap();
        let sink = Arc::new(JsonlSink::open(dir.join(".sd-agent/traces/t.jsonl")).unwrap());
        let rec = TraceRecorder::new("t3", sink);
        let env = EnvProfile::detect();
        let ledger = ResourceLedger::new();
        let tables = DisciplineTables::p0();
        let port = CountingPort::new(ApprovalDecision::AlwaysAllow);
        let c = ctx(&env, &dir, &rec, &ledger, &tables, &port);
        let r1 = dispatch(&c, &write_call("c1", "a.txt")).await.unwrap();
        let r2 = dispatch(&c, &write_call("c2", "b.txt")).await.unwrap();
        assert!(r1.ok, "{}", r1.text);
        assert!(r2.ok, "{}", r2.text);
        // 消费语义：第二次免询问（approve 只被调用一次）。
        assert_eq!(port.calls(), 1);
        // 审计不丢：两次都记录 ToolApprovalResolved 且 by=always。
        let trace = std::fs::read_to_string(dir.join(".sd-agent/traces/t.jsonl")).unwrap();
        assert_eq!(trace.matches("\"by\":\"always\"").count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn denial_streak_breaker_halts_run() {
        // 问题③：同一 run 内连续被拒达阈值即收尾（特殊文案 + 事件留痕 +
        // 后续调用短路），防模型无限重试被拒工具。
        let dir = std::env::temp_dir().join(format!("sd-policy-test4-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".sd-agent/traces")).unwrap();
        let sink = Arc::new(JsonlSink::open(dir.join(".sd-agent/traces/t.jsonl")).unwrap());
        let rec = TraceRecorder::new("t4", sink);
        let env = EnvProfile::detect();
        let ledger = ResourceLedger::new();
        let tables = DisciplineTables::p0();
        let port = CountingPort::new(ApprovalDecision::Denied);
        let c = ctx(&env, &dir, &rec, &ledger, &tables, &port);
        let limit = DisciplineTables::DENIAL_STREAK_LIMIT;
        let mut last = None;
        for i in 0..limit {
            last = Some(
                dispatch(&c, &write_call(&format!("c{i}"), "a.txt"))
                    .await
                    .unwrap(),
            );
        }
        // 第 limit 次拒绝触发熔断文案（任务口径原话）。
        let tripped = last.unwrap();
        assert!(tripped.denied);
        assert!(
            tripped.text.contains("连续被拒已达上限，停止重试"),
            "{}",
            tripped.text
        );
        // 熔断后短路：approve 不再被调用，文案照给。
        let after = dispatch(&c, &write_call("c-after", "a.txt")).await.unwrap();
        assert!(after.denied);
        assert!(after.text.contains("连续被拒已达上限，停止重试"));
        assert_eq!(port.calls(), limit);
        // 事件留痕：熔断文案进 ToolCallDenied reason。
        let trace = std::fs::read_to_string(dir.join(".sd-agent/traces/t.jsonl")).unwrap();
        assert!(trace.contains("连续被拒已达上限"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn digest_of_uses_tables_threshold() {
        // 问题⑥：digest 阈值数据位真的生效（不再是"仅被展示"）。
        let tables = DisciplineTables::p0();
        let threshold = tables.output_digest_threshold_bytes;
        let long = "x".repeat(threshold + 100);
        let d = digest_of(&long, threshold);
        assert!(d.contains("[digest:"), "{d}");
        assert!(d.len() < long.len());
        // 阈值内不截断。
        assert_eq!(digest_of("short", threshold), "short");
    }
}
