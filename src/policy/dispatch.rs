//! dispatch 裁决流执行面（从 policy/mod.rs 拆出）：白名单查表 → 参数强类型
//! 解析 → 危险命令规则表 → ApprovalPort 往返 → 执行；拒绝路径与摘要辅助同居
//! 本文件。唯一模型驱动执行入口 dispatch() 对外由 policy/mod.rs 门面 re-export。

use crate::config::DisciplineTables;
use crate::event::{
    EventPayload, LifecycleHook, SinkError, ToolApprovalRequested, ToolApprovalResolved,
    ToolCallDenied, ToolCallFinished, ToolCallRequested, ToolCallStarted, emit_hook,
};

use super::{
    ApprovalDecision, ApprovalRequest, PolicyContext, ToolCall, ToolResult, is_safe_readonly,
    matched_dangerous_pattern,
};

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

    // 4. 审批往返（read 免审；bash 确定性只读单命令免审——rules::is_safe_readonly
    // 查表口径：组合形态/重定向/git 写子命令一律照走 ApprovalPort，裁决由人/壳
    // 作出。免审路径不发 ToolApprovalRequested（无询问就无申请），但
    // ToolCallStarted/ToolCallFinished 照发（审计不丢）。
    // AlwaysAllow 消费（问题③）：端口置过放行位后，本 run 内后续非只读工具
    // 免询问；事件仍照发（by 如实记 "always"，审计不丢）。
    let readonly_exempt = call.name == "read"
        || (call.name == "bash"
            && is_safe_readonly(
                args.get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default(),
            ));
    if !readonly_exempt {
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
pub(super) fn digest_of(text: &str, threshold_bytes: usize) -> String {
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
