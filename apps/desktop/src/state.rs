//! 应用状态：run 注册表 + 审批回传通道注册表。
//!
//! 所有共享结构显式 Arc/Mutex，供 command 层、run 线程、审批端三方共用；
//! 不引全局静态，生命周期随 Tauri managed state。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

/// 测试连接冷却窗口：30 秒。计费敏感——每次测试都真实调用一次模型，
/// 按钮连打会重复扣额度，壳层强制节流（前端禁用按钮只是第一道）。
pub const TEST_COOLDOWN_MS: u64 = 30_000;

/// 冷却余量（纯函数，测试直击）：last 为上次测试时间戳，now 为当前时间戳，
/// 返回剩余毫秒数；0 表示可以再测。时钟回拨（now < last）视为可再测，
/// 不许负数或误锁。
pub fn cooldown_remaining_ms(last_ms: u64, now_ms: u64) -> u64 {
    if now_ms < last_ms {
        return 0;
    }
    let elapsed = now_ms - last_ms;
    if elapsed >= TEST_COOLDOWN_MS {
        0
    } else {
        TEST_COOLDOWN_MS - elapsed
    }
}

use sd_agent::event::now_unix_ms;
use sd_agent::policy::ApprovalDecision;
use serde::Serialize;

/// 一次 run 的元数据（侧栏任务列表直接消费）。
#[derive(Debug, Clone, Serialize)]
pub struct RunInfo {
    pub id: String,
    pub name: String,
    pub task: String,
    /// running / completed / max_rounds_reached / failed。
    pub status: String,
    pub rounds: u32,
    pub trace_id: String,
    pub final_text: String,
    pub error: Option<String>,
}

/// 审批回传通道表：tool_call_id → 回传 Sender（DesktopApproval 与
/// resolve_approval command 之间的唯一桥）。
pub type PendingMap = Arc<Mutex<HashMap<String, mpsc::Sender<ApprovalDecision>>>>;

pub struct AppState {
    /// 工作区根目录 = 启动目录。
    pub root: PathBuf,
    pub runs: Arc<Mutex<Vec<RunInfo>>>,
    pub pending: PendingMap,
    counter: AtomicU64,
    /// 上次“测试连接”时间戳（ms）；0 = 从未测过。冷却判定见 try_begin_test。
    last_test_ms: AtomicU64,
}

impl AppState {
    pub fn new() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            root,
            runs: Arc::new(Mutex::new(Vec::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            counter: AtomicU64::new(0),
            last_test_ms: AtomicU64::new(0),
        }
    }

    /// 尝试占用一次“测试连接”名额：冷却中 Err(剩余秒数)，放行则记时间戳。
    /// 进程内存状态即可（重启后重置属预期：重启本身已是自然节流）。
    /// last=0 表示从未测过，直接放行（不从“纪元 0”起算冷却）。
    pub fn try_begin_test(&self) -> Result<(), u64> {
        let now = now_unix_ms() as u64;
        let last = self.last_test_ms.load(Ordering::SeqCst);
        if last != 0 {
            let remaining = cooldown_remaining_ms(last, now);
            if remaining > 0 {
                return Err(remaining.div_ceil(1000));
            }
        }
        self.last_test_ms.store(now, Ordering::SeqCst);
        Ok(())
    }

    /// run_id 全局单调唯一（时间戳 + 进程内计数）。
    pub fn next_run_id(&self) -> String {
        format!(
            "run-{}-{}",
            now_unix_ms(),
            self.counter.fetch_add(1, Ordering::SeqCst)
        )
    }

    pub fn push_run(&self, info: RunInfo) {
        push_run(&self.runs, info);
    }

    pub fn snapshot(&self) -> Vec<RunInfo> {
        self.runs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    /// 轨迹目录：<root>/.sd-agent/traces（轨迹命名规则的壳内唯一入口）。
    pub fn traces_dir(&self) -> PathBuf {
        self.root.join(".sd-agent").join("traces")
    }

    /// 轨迹文件路径：<root>/.sd-agent/traces/<trace_id>.jsonl。
    pub fn trace_path(&self, trace_id: &str) -> PathBuf {
        self.traces_dir().join(format!("{trace_id}.jsonl"))
    }

    /// 回传一次审批裁决（resolve_approval command 唯一入口）。
    pub fn resolve(&self, tool_call_id: &str, decision: &str) -> Result<(), String> {
        let decision = match decision {
            "approved" => ApprovalDecision::Approved,
            "always" => ApprovalDecision::AlwaysAllow,
            "denied" => ApprovalDecision::Denied,
            other => return Err(format!("未知裁决: {other}")),
        };
        // 先 remove 再 send：重复 resolve 投不进第二票，run 线程已消亡也不留悬挂条目。
        let sender = self
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(tool_call_id);
        match sender {
            Some(tx) => tx.send(decision).map_err(|_| "审批通道已关闭".to_string()),
            None => Err(format!("没有待裁决的调用: {tool_call_id}")),
        }
    }
}

/// 追加一条 run（run 线程侧亦可直接用共享表调用）。
pub fn push_run(runs: &Mutex<Vec<RunInfo>>, info: RunInfo) {
    runs.lock().unwrap_or_else(|p| p.into_inner()).push(info);
}

/// run 收尾：改状态、记轮数与收尾文本/错误。
#[allow(clippy::too_many_arguments)]
pub fn finish_run(
    runs: &Mutex<Vec<RunInfo>>,
    id: &str,
    status: &str,
    rounds: u32,
    final_text: &str,
    error: Option<String>,
) {
    let mut list = runs.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(info) = list.iter_mut().find(|r| r.id == id) {
        info.status = status.to_string();
        info.rounds = rounds;
        info.final_text = final_text.to_string();
        info.error = error;
    }
}

/// 侧栏任务名兜底：取任务文本前若干字符。
pub fn derive_name(task: &str, max_chars: usize) -> String {
    let trimmed = task.trim();
    let mut out: String = trimmed.chars().take(max_chars).collect();
    if trimmed.chars().count() > max_chars {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{cooldown_remaining_ms, TEST_COOLDOWN_MS};

    /// 冷却判定：窗口内返回剩余毫秒，窗口外返回 0（可再测）。
    #[test]
    fn cooldown_window_is_30s() {
        let last = 1_000_000u64;
        // 刚测完：还剩一整个窗口。
        assert_eq!(cooldown_remaining_ms(last, last), TEST_COOLDOWN_MS);
        // 窗口中段：剩余递减。
        assert_eq!(cooldown_remaining_ms(last, last + 10_000), 20_000);
        // 窗口边界：恰好到期。
        assert_eq!(cooldown_remaining_ms(last, last + TEST_COOLDOWN_MS), 0);
        // 过窗：可再测。
        assert_eq!(cooldown_remaining_ms(last, last + TEST_COOLDOWN_MS + 1), 0);
    }

    /// now < last（时钟回拨）：视为可再测，不许负数或误锁。
    #[test]
    fn cooldown_tolerates_clock_skew() {
        assert_eq!(cooldown_remaining_ms(5_000, 4_000), 0);
    }

    /// try_begin_test：放行后窗口内拒绝并报剩余秒数，过窗再放行。
    #[test]
    fn try_begin_test_throttles_repeats() {
        let st = super::AppState::new();
        assert!(st.try_begin_test().is_ok(), "首次必须放行");
        let err = st.try_begin_test().expect_err("冷却窗口内必须拒绝");
        assert!((1..=30).contains(&err), "剩余秒数应落在 1..=30，实际 {err}");
    }
}
