//! hub 主页状态：历史会话列表、Token 统计缓存、任务提交。
//!
//! hub 是观察面：不跑 run（run 归 worker 新终端），只读事实源
//! （会话库 + 轨迹统计）；输入回车 = 新开会话并弹终端执行。

use sd_agent::session::{SessionMeta, SessionStore};

use super::anim::Anim;
use super::editor::Editor;
use super::form::FormState;
use crate::stats::Stats;

/// 模型配置行（选择器显示用；完整配置在核心 Settings）。
#[derive(Debug, Clone)]
pub struct ProfileRow {
    pub label: String,
    pub model: String,
    /// 思考强度档位索引（0..7 对齐 crate::slash::EFFORTS）。
    pub effort_idx: usize,
}

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
    // ── 以下为 hub 重做契约字段（结构冻结，交互层写、渲染层读） ──
    /// 模型配置快照（来自核心 Settings；选择器/表单共用）。
    pub profiles: Vec<ProfileRow>,
    /// 模型选择器当前索引。
    pub profile_sel: usize,
    /// 思考强度滑条档位（0..7）。
    pub effort_idx: usize,
    /// "+" 配置表单弹窗（None=关闭）。
    pub form: Option<FormState>,
    /// 两态动画（初始态 ↔ WORK 态）。
    pub anim: Anim,
    /// 模型选择浮层开关（契约外补充字段：浮层态无冻结位，交互层写、渲染层读；
    /// ↑↓ 高亮即 profile_sel，Esc 复位到激活项）。
    pub model_open: bool,
}

impl HubState {
    pub fn new(root: &std::path::Path) -> Self {
        let store = SessionStore::new(root);
        let sessions = store.list();
        let stats = Stats::scan(root);
        let settings = sd_agent::config::settings::Settings::load();
        let mut profiles: Vec<ProfileRow> = settings
            .view()
            .profiles
            .iter()
            .map(|p| ProfileRow {
                label: p.label.clone(),
                model: p.model.clone(),
                effort_idx: effort_index(&p.reasoning_effort),
            })
            .collect();
        if profiles.is_empty() {
            profiles.push(ProfileRow {
                label: "(未配置)".into(),
                model: "—".into(),
                effort_idx: 3,
            });
        }
        let profile_sel = 0;
        let effort_idx = profiles[profile_sel].effort_idx;
        Self {
            editor: Editor::new(),
            sessions,
            selected: 0,
            scroll: 0,
            stats,
            launched: false,
            status: String::new(),
            profiles,
            profile_sel,
            effort_idx,
            form: None,
            anim: Anim::default(),
            model_open: false,
        }
    }

    /// 重新读取 Settings 快照（增删改配置后刷新选择器）。
    pub fn reload_profiles(&mut self) {
        let settings = sd_agent::config::settings::Settings::load();
        let view = settings.view();
        let active = view.active_profile.clone();
        self.profiles = view
            .profiles
            .iter()
            .map(|p| ProfileRow {
                label: p.label.clone(),
                model: p.model.clone(),
                effort_idx: effort_index(&p.reasoning_effort),
            })
            .collect();
        if self.profiles.is_empty() {
            self.profiles.push(ProfileRow {
                label: "(未配置)".into(),
                model: "—".into(),
                effort_idx: 3,
            });
        }
        self.profile_sel = self
            .profiles
            .iter()
            .position(|p| p.label == active)
            .unwrap_or(0)
            .min(self.profiles.len() - 1);
        self.effort_idx = self.profiles[self.profile_sel].effort_idx;
    }

    /// 两态判定：初始态（大 Logo + 空白对话框）/ WORK 态（顶栏 + 历史 + 统计）。
    /// 只认"本轮是否发起过对话"（launched）：打开程序一律初始态（为发起
    /// 新对话服务），发起后才切 WORK 态；历史会话多少不影响两态（用户拍板）。
    pub fn is_work(&self) -> bool {
        self.launched
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

    // ── 以下为交互层方法（新增，不动冻结字段） ──

    /// 模型浮层高亮上移（clamp 到 0）。
    pub fn model_prev(&mut self) {
        self.profile_sel = self.profile_sel.saturating_sub(1);
    }

    /// 模型浮层高亮下移（clamp 到末项）。
    pub fn model_next(&mut self) {
        if self.profile_sel + 1 < self.profiles.len() {
            self.profile_sel += 1;
        }
    }

    /// 当前高亮配置名（None=占位行「(未配置)」，不可确认）。
    pub fn selected_label(&self) -> Option<&str> {
        self.profiles
            .get(self.profile_sel)
            .map(|p| p.label.as_str())
            .filter(|l| *l != "(未配置)")
    }

    /// 滑条/键盘改档（clamp 0..EFFORTS 末档）。返回是否真的变化。
    pub fn set_effort(&mut self, idx: usize) -> bool {
        let idx = idx.min(crate::slash::EFFORTS.len().saturating_sub(1));
        if self.effort_idx == idx {
            return false;
        }
        self.effort_idx = idx;
        true
    }
}

/// 思考强度字符串 → 档位索引（0..7；未知归 medium=3）。
fn effort_index(name: &str) -> usize {
    crate::slash::EFFORTS
        .iter()
        .position(|e| e.eq_ignore_ascii_case(name.trim()))
        .unwrap_or(3)
}
