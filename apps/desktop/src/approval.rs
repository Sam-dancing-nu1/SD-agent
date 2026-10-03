//! DesktopApproval：ApprovalPort 的桌面实现（四个接口之一的壳侧实现）。
//!
//! 语义：approve() 同步阻塞——发 approval_request 事件到前端弹窗，
//! 前端 invoke("resolve_approval") 经 std::sync::mpsc 回传，recv() 取裁决。
//! 阻塞只发生在该 run 自己的执行线程（见 runner.rs），不冻结 UI 与其他 run。
//! 会话放行位：裁决 AlwaysAllow 后置位，本次 run 后续 approve() 直接放行
//! （与 TUI 的 a=会话全放行同语义）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sd_agent::policy::{ApprovalDecision, ApprovalPort, ApprovalRequest};
use tauri::Emitter;

use crate::state::PendingMap;

pub struct DesktopApproval {
    app: tauri::AppHandle,
    run_id: String,
    pending: PendingMap,
    /// 会话放行位（本次 run 内全放行）。
    allow_all: AtomicBool,
}

impl DesktopApproval {
    pub fn new(app: tauri::AppHandle, run_id: impl Into<String>, pending: PendingMap) -> Self {
        Self {
            app,
            run_id: run_id.into(),
            pending,
            allow_all: AtomicBool::new(false),
        }
    }
}

impl ApprovalPort for DesktopApproval {
    fn approve(&self, request: ApprovalRequest) -> ApprovalDecision {
        if self.allow_all.load(Ordering::SeqCst) {
            return ApprovalDecision::AlwaysAllow;
        }
        let (tx, rx) = std::sync::mpsc::channel::<ApprovalDecision>();
        let key = request.tool_call_id.clone();
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key.clone(), tx);

        let payload = serde_json::json!({
            "run_id": self.run_id,
            "tool_call_id": request.tool_call_id,
            "tool": request.tool,
            "summary": request.summary,
            "detail": request.detail,
        });
        // UI 通知失败不构成 SinkError（落盘链路不受影响），静默降级为等待。
        let _ = self.app.emit("approval_request", payload);

        let decision = loop {
            match rx.recv_timeout(Duration::from_secs(3600)) {
                Ok(d) => break d,
                // 超时继续等：用户可能长时间不点弹窗。
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    break ApprovalDecision::Denied;
                }
            }
        };
        // AlwaysAllow 置会话放行位：本次 run 后续调用直接放行。
        if decision == ApprovalDecision::AlwaysAllow {
            self.allow_all.store(true, Ordering::SeqCst);
        }
        // 兜底 remove（resolve() 已先 remove，这里只防 Disconnected 等异常残留）。
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&key);
        decision
    }

    fn source(&self) -> &'static str {
        "desktop"
    }
}
