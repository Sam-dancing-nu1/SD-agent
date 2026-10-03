//! ApprovalPort：裁决往返口（决策 7 / 审查 #19）。
//!
//! 语义：核心 → UI → 用户 → 回传。P0 = CLI stdin / 自动批准；
//! P4 = TUI 弹窗；P5 = 远程审批回传。接口同步语义（P0），
//! 异步壳层用阻塞 channel 桥接实现，不改本接口。
//!
//! run 级裁决状态（问题③修复）：AlwaysAllow 消费语义 + 连续拒绝熔断，
//! 由 [`RunApprovalState`] 自持、经 [`ApprovalPort::run_state`] 槽暴露给核心
//! dispatch——状态归裁决实现方所有（不改 PolicyContext 结构、不引全局状态），
//! 核心只读写本结构。默认 `None` = 无状态端口（行为同旧版；外部壳层
//! 如 TUI 的会话放行位待接，UI 侧 allow_all 复位亦属壳层职责）。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// 一次裁决请求（summary 给 UI 展示，detail 给展开审计）。
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub tool_call_id: String,
    pub tool: String,
    pub summary: String,
    pub detail: String,
}

/// 裁决结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved,
    Denied,
    /// 本 run 内后续非只读工具免询问（消费语义：置 RunApprovalState 放行位，
    /// 事件仍照发、by 如实记 "always"——不是假承诺）。
    AlwaysAllow,
}

impl ApprovalDecision {
    pub fn is_approved(&self) -> bool {
        matches!(
            self,
            ApprovalDecision::Approved | ApprovalDecision::AlwaysAllow
        )
    }

    /// 裁决来源标识（进事件 payload 的 by 字段；AlwaysAllow 如实记 "always"）。
    pub fn source_tag(&self) -> &'static str {
        match self {
            ApprovalDecision::Approved => "approved",
            ApprovalDecision::AlwaysAllow => "always",
            ApprovalDecision::Denied => "denied",
        }
    }
}

/// run 级裁决状态（AlwaysAllow 放行位 + 连续拒绝熔断计数）。
/// 原子位实现（dispatch 只拿共享引用）；会话边界 = 端口实例/进程
///（CLI 一进程一会话）。跨 run 复位属壳层职责。
#[derive(Debug)]
pub struct RunApprovalState {
    /// AlwaysAllow 放行位置位后：本 run 内非只读工具免询问。
    allow_all: AtomicBool,
    /// 连续被拒计数（成功执行清零；只增于连续拒绝）。
    denial_streak: AtomicUsize,
}

impl RunApprovalState {
    pub const fn new() -> Self {
        Self {
            allow_all: AtomicBool::new(false),
            denial_streak: AtomicUsize::new(0),
        }
    }

    /// AlwaysAllow 消费：置放行位。
    pub fn engage_allow_all(&self) {
        self.allow_all.store(true, Ordering::SeqCst);
    }

    /// 放行位是否已置。
    pub fn allow_all(&self) -> bool {
        self.allow_all.load(Ordering::SeqCst)
    }

    /// 记一次拒绝，返回连续拒绝计数（调用方与熔断阈值比较）。
    pub fn note_denial(&self) -> usize {
        self.denial_streak.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// 成功执行清零连续拒绝计数（熔断只针对连续被拒）。
    pub fn note_success(&self) {
        self.denial_streak.store(0, Ordering::SeqCst);
    }

    /// 当前连续拒绝计数。
    pub fn denial_streak(&self) -> usize {
        self.denial_streak.load(Ordering::SeqCst)
    }
}

impl Default for RunApprovalState {
    fn default() -> Self {
        Self::new()
    }
}

/// 裁决往返口（四个接口之一）。
pub trait ApprovalPort: Send + Sync {
    /// 阻塞式裁决往返（异步壳层在自己的线程里实现）。
    fn approve(&self, request: ApprovalRequest) -> ApprovalDecision;

    /// 裁决来源标签（cli / tui / desktop / auto）。
    fn source(&self) -> &'static str;

    /// run 级状态槽（默认 None = 无状态端口：不消费 AlwaysAllow、不做拒绝
    /// 熔断，行为同旧版）。持有状态的实现应覆盖本方法。
    fn run_state(&self) -> Option<&RunApprovalState> {
        None
    }
}

/// 自动批准（doctor 探针 / `run --yes` 无人值守；证据链仍全量留痕）。
pub struct AutoApproval;

impl ApprovalPort for AutoApproval {
    fn approve(&self, _request: ApprovalRequest) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn source(&self) -> &'static str {
        "auto"
    }

    fn run_state(&self) -> Option<&RunApprovalState> {
        // 进程级单例承载 run 状态（单位结构体无字段位可放；CLI 一进程
        // 一会话语义）。无人值守路径同样要拒绝熔断（防模型无限重试）。
        static STATE: RunApprovalState = RunApprovalState::new();
        Some(&STATE)
    }
}

/// 全拒（测试用；无 run 状态槽——避免测试间共享计数互相污染）。
pub struct DenyAll;

impl ApprovalPort for DenyAll {
    fn approve(&self, _request: ApprovalRequest) -> ApprovalDecision {
        ApprovalDecision::Denied
    }

    fn source(&self) -> &'static str {
        "deny_all"
    }
}

/// CLI stdin 裁决（P0 默认交互形态）。
pub struct CliApproval;

impl ApprovalPort for CliApproval {
    fn approve(&self, request: ApprovalRequest) -> ApprovalDecision {
        use std::io::Write;
        eprintln!(
            "\n[approval] tool={} summary={}\n  detail={}\n  allow? [y/N/always] ",
            request.tool, request.summary, request.detail
        );
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return ApprovalDecision::Denied;
        }
        match line.trim().to_lowercase().as_str() {
            "y" | "yes" => ApprovalDecision::Approved,
            "always" => ApprovalDecision::AlwaysAllow,
            _ => ApprovalDecision::Denied,
        }
    }

    fn source(&self) -> &'static str {
        "cli"
    }

    fn run_state(&self) -> Option<&RunApprovalState> {
        // 进程级单例承载 run 状态（CLI 一进程一会话）。
        static STATE: RunApprovalState = RunApprovalState::new();
        Some(&STATE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> ApprovalRequest {
        ApprovalRequest {
            tool_call_id: "c1".into(),
            tool: "bash".into(),
            summary: "echo".into(),
            detail: "echo hi".into(),
        }
    }

    #[test]
    fn auto_allows_deny_all_denies() {
        assert!(AutoApproval.approve(req()).is_approved());
        assert!(!DenyAll.approve(req()).is_approved());
        // DenyAll 测试助手无 run 状态槽（防跨测试共享计数）。
        assert!(DenyAll.run_state().is_none());
    }

    #[test]
    fn always_allow_source_tag_is_honest() {
        // 问题③：AlwaysAllow 的 by 如实记 "always"（不再混记 "approved"）。
        assert_eq!(ApprovalDecision::AlwaysAllow.source_tag(), "always");
        assert_eq!(ApprovalDecision::Approved.source_tag(), "approved");
        assert_eq!(ApprovalDecision::Denied.source_tag(), "denied");
    }

    #[test]
    fn run_state_semantics() {
        let state = RunApprovalState::new();
        assert!(!state.allow_all());
        state.engage_allow_all();
        assert!(state.allow_all());
        // 连续拒绝计数：只增于拒绝、成功清零。
        assert_eq!(state.note_denial(), 1);
        assert_eq!(state.note_denial(), 2);
        assert_eq!(state.denial_streak(), 2);
        state.note_success();
        assert_eq!(state.denial_streak(), 0);
    }
}
