//! sd-agent 核心库（Harness 底座 P0 最小闭环）。
//!
//! 模块布局见 docs/project-structure.md。依赖方向只许向下：
//!   cli → agent → policy → tools；agent → model / event / context；
//!   event、config 位于最底、零业务依赖。核心 lib 严禁依赖任何壳 crate。
//!
//! 四个接口（决策 7）：EventSink / EventSource（event 层）、
//! ApprovalPort（policy/approval）、ModelClient（model）。
//! 唯一模型驱动执行入口：policy::dispatch（决策 3）。

pub mod agent;
pub mod cli;
pub mod config;
pub mod context;
pub mod doctor;
pub mod event;
pub mod model;
pub mod policy;
pub mod session;
pub mod sys;
pub mod tools;
pub mod verify;

/// 测试专用：进程级环境变量互斥锁。edition 2024 下环境变量操作是 unsafe
/// 且进程全局，多线程测试的临界区必须串行化（USERPROFILE / SD_AGENT_* 等）。
#[cfg(test)]
pub(crate) mod test_env {
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 拿锁（中毒时接管，不让一个失败测试连坐全部）。
    pub(crate) fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
}
