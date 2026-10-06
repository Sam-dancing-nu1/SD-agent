//! hub 主页状态：历史会话列表、Token 统计缓存、任务提交。
//!
//! hub 是观察面：不跑 run（run 归 worker 新终端），只读事实源
//! （会话库 + 轨迹统计）；输入回车 = 新开会话并弹终端执行。

use sd_agent::session::{SessionMeta, SessionStore};

use super::editor::Editor;
use crate::stats::Stats;

/// hub 状态。
pub struct HubState {
    pub editor: Editor,
    /// 历史会话（事实源：会话库）。
    pub sessions: Vec<SessionMeta>,
    /// 选中项。
    pub selected: usize,
    /// 列表滚动。
    pub scroll: u16,
    /// 统计缓存（/stats 刷新或收尾刷新）。
    pub stats: Stats,
    /// 本轮是否已发起过任务（WORK 态判定之一）。
    pub launched: bool,
    /// 状态栏注记。
    pub status: String,
}

impl HubState {
    pub fn new(root: &std::path::Path) -> Self {
        let store = SessionStore::new(root);
        let sessions = store.list();
        let stats = Stats::scan(root);
        Self {
            editor: Editor::new(),
            sessions,
            selected: 0,
            scroll: 0,
            stats,
            launched: false,
            status: String::new(),
        }
    }

    /// 两态判定：初始态（大 Logo + 空白对话框）/ WORK 态（顶栏 + 历史 + 统计）。
    /// 有历史会话或已发起过任务即 WORK 态。
    pub fn is_work(&self) -> bool {
        self.launched || !self.sessions.is_empty()
    }

    /// 刷新事实源（会话 + 统计）。
    pub fn refresh(&mut self, root: &std::path::Path) {
        let store = SessionStore::new(root);
        self.sessions = store.list();
        self.stats = Stats::scan(root);
        if self.selected >= self.sessions.len() {
            self.selected = self.sessions.len().saturating_sub(1);
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.sessions.len() {
            self.selected += 1;
        }
    }

    /// 选中会话 id。
    pub fn selected_id(&self) -> Option<&str> {
        self.sessions.get(self.selected).map(|s| s.id.as_str())
    }
}
