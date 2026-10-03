//! 对话式 UI 状态机（Pi / OpenCode 形态）：单一对话流 + 底部输入框 + 顶部状态栏。
//!
//! 流式渲染：模型回复经 StreamObserver 增量流入——思考文本先流出（暗色块，
//! 标头「✦ 思考中…」，转正文时收起、Ctrl+T 展开/收起），正文随后逐字流出；
//! 工具调用参数原地更新预览。增量一律原样追加进对应流式条目（按 run_id +
//! round 归属，多 run 并发互不串），不做整轮重建。
//!
//! 职责边界：只持有壳层视图状态、斜杠命令与交互引导流程；业务执行全部经
//! crate::run 装配核心 sd_agent（agent::run_task / doctor::run_all /
//! model::OpenAiCompatClient::from_settings / session::SessionStore）完成，
//! 本文件不实现任何工具执行、模型调用或事件契约语义。
//!
//! 核心 API 适配集中在文件底部 glue 小节：契约签名变动只改那里。

use std::cell::Cell;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender as StdSender;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sd_agent::doctor::DoctorReport;
use sd_agent::event::{Event, EventPayload};
use sd_agent::model::ChatMessage;
use sd_agent::policy::{ApprovalDecision, ApprovalRequest};
use sd_agent::session::{Session, SessionMessage, SessionMeta, SessionStore};

use crate::bridge::UiMsg;
use crate::slash;
use crate::ui::show_val;

// ---------------- 对话流条目 ----------------

/// 工具调用卡片状态。
#[derive(Debug, Clone)]
pub enum ToolState {
    /// 已请求，等待结果。
    Pending,
    /// 已完成（ok + 结果摘要）。
    Done { ok: bool, result: String },
    /// 被拒。
    Denied { reason: String },
}

/// 对话流一条目（时间顺序追加）。
#[derive(Debug, Clone)]
pub enum ConvoItem {
    /// 用户消息。
    User { text: String },
    /// 助手回复（含思考块：reasoning 非空时可展开/收起）。
    Assistant {
        text: String,
        /// 思考文本（会话恢复带回；空 = 无思考块）。
        reasoning: String,
        /// 思考块展开态（收起为单行摘要）。
        expanded: bool,
    },
    /// 流式输出条目（一次模型轮的实时渲染：思考流 → 正文流 → 收尾固化）。
    Streaming {
        run_id: u64,
        round: u32,
        /// 已流出的思考文本（增量追加）。
        reasoning: String,
        /// 已流出的正文（增量追加）。
        text: String,
        /// 思考阶段进行中（true 时标头「✦ 思考中…」）。
        thinking: bool,
        /// 工具参数流预览（name, args_so_far；原地更新）。
        tool_preview: Option<(String, String)>,
        /// 思考块展开态（转正文时自动收起）。
        expanded: bool,
    },
    /// 工具调用卡片（请求→结果原地更新）。
    ToolCard {
        id: String,
        tool: String,
        args: String,
        state: ToolState,
    },
    /// 说明行（run 收尾 / 验证结果 / 提示）。
    Note(String),
    /// 错误行。
    Error(String),
    /// 信息卡片（斜杠命令输出 / 引导文案；title + 行）。
    Card { title: String, lines: Vec<String> },
    /// doctor 体检卡片。
    Doctor(DoctorReport),
}

// ---------------- run / 事件 / 审批视图 ----------------

/// run 状态（/runs 列表展示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    /// 熔断（max_rounds_reached）。
    Cutoff,
}

impl RunStatus {
    pub fn label(self) -> &'static str {
        match self {
            RunStatus::Pending => "待启动",
            RunStatus::Running => "运行中",
            RunStatus::Completed => "完成",
            RunStatus::Failed => "失败",
            RunStatus::Cutoff => "熔断",
        }
    }
}

/// 一次 run 的壳层视图状态。
#[derive(Debug, Clone)]
pub struct RunEntry {
    pub id: u64,
    pub task: String,
    pub status: RunStatus,
    pub rounds: u32,
}

/// 事件观察行（/trace）。
#[derive(Debug, Clone)]
pub struct EventRow {
    pub seq: u64,
    pub kind: &'static str,
    pub trace_id: String,
    pub summary: String,
}

/// 一条待裁决的审批请求。
pub struct PendingApproval {
    pub run_id: u64,
    pub request: ApprovalRequest,
    pub reply: StdSender<ApprovalDecision>,
}

// ---------------- 模型配置视图 ----------------

/// 一条模型配置的展示快照（密钥只留是否已配，绝不留内容）。
#[derive(Debug, Clone)]
pub struct ProfileRow {
    pub label: String,
    pub model: String,
    pub base_url: String,
    pub effort: String,
    pub has_key: bool,
    pub active: bool,
}

/// 配置总览快照（启动时与每次变更后重建；渲染层不重建）。
#[derive(Debug, Clone)]
pub struct CfgSummary {
    pub rows: Vec<ProfileRow>,
    pub max_rounds: u32,
    pub settings_path: String,
    pub missing: Vec<String>,
}

/// 历史会话展示行。
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id_key: String,
    pub title: String,
    pub time_label: String,
    pub count: usize,
}

/// 向导草稿（新增/编辑模型配置）。
#[derive(Debug, Clone, Default)]
pub struct Draft {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub effort: String,
}

// ---------------- 交互流程 ----------------

/// 输入框之上的交互引导流程（对话内逐步问答，非独立面板）。
#[derive(Debug, Clone)]
pub enum Flow {
    /// 无引导（普通输入 = 发任务/斜杠命令）。
    None,
    /// /settings 菜单（a 新增 · 序号切换 · e序号 改思考强度 · d序号 删除 · 回车返回）。
    SettingsMenu,
    /// /model 菜单（序号切换当前配置 · 回车返回）。
    ModelMenu,
    /// /conversations 菜单（序号切换会话 · n 新建 · 回车返回）。
    SessionMenu,
    /// 新增/编辑模型配置向导（step 0 端点 1 模型名 2 密钥 3 思考强度选择）。
    ProfileWizard {
        edit_label: Option<String>,
        step: u8,
        sel: usize,
        draft: Draft,
    },
    /// 思考强度列表选择（↑↓ 选择 · Enter 确认）。
    EffortPick { label: String, sel: usize },
    /// 删除确认（输入 y 确认）。
    DeleteConfirm { label: String },
}

// ---------------- 应用状态 ----------------

/// 壳层应用状态。
pub struct App {
    pub root: PathBuf,
    /// 对话流（时间顺序）。
    pub convo: Vec<ConvoItem>,
    /// 自动跟底（true = 新内容自动跟随；滚轮/翻页上翻后置 false）。
    pub scroll_follow: bool,
    /// 渲染层回填的视口度量（上一帧窗口顶行 / 总行数 / 可见行数）。
    /// Cell 保持 ui::draw(&App) 只读签名；按键翻页用它换算行数。
    pub view_start: Cell<usize>,
    pub view_total: Cell<usize>,
    pub view_lines: Cell<usize>,
    /// 输入框内容（多行；向导密钥步为真实密钥，渲染层打码）。
    pub input: String,
    /// 斜杠命令列表选中项。
    pub slash_sel: usize,
    /// Esc 关闭过斜杠列表（输入变化前不再弹出）。
    pub slash_dismissed: bool,
    /// 发送历史（↑↓ 召回）。
    pub history: Vec<String>,
    pub hist_idx: Option<usize>,
    /// 当前交互引导。
    pub flow: Flow,
    /// 任务列表（/runs）。
    pub runs: Vec<RunEntry>,
    /// 实时事件流（/trace）。
    pub events: Vec<EventRow>,
    pub doctor_running: bool,
    /// 审批弹窗队列（同一时刻只渲染队首）。
    pub approval_queue: VecDeque<PendingApproval>,
    /// 本次会话全放行开关（审批弹窗 a 键置位）。
    pub allow_all: Arc<AtomicBool>,
    /// 瞬时提示（渲染到快捷提示行右侧）。
    pub status: String,
    pub should_quit: bool,
    /// Ctrl+C 首按时间（两按退出）。
    pub ctrl_c_at: Option<Instant>,
    pub tx: StdSender<UiMsg>,
    pub handle: tokio::runtime::Handle,
    /// 配置快照（变更后重建）。
    pub cfg: CfgSummary,
    /// 会话存储与当前会话。
    pub store: SessionStore,
    pub session: Option<Session>,
    /// 历史会话列表（/conversations 刷新）。
    pub session_rows: Vec<SessionRow>,
    session_metas: Vec<SessionMeta>,
    next_run_id: u64,
    /// 已收尾的模型轮（run_id, round）：流式收尾与整轮兜底上报的去重账。
    done_rounds: HashSet<(u64, u32)>,
}

impl App {
    /// 新建应用状态（root = 工作区根）。
    pub fn new(root: PathBuf, tx: StdSender<UiMsg>, handle: tokio::runtime::Handle) -> Self {
        let cfg = glue::cfg_summary();
        let store = glue::session_store(&root);
        let mut app = Self {
            root,
            convo: Vec::new(),
            scroll_follow: true,
            view_start: Cell::new(0),
            view_total: Cell::new(0),
            view_lines: Cell::new(0),
            input: String::new(),
            slash_sel: 0,
            slash_dismissed: false,
            history: Vec::new(),
            hist_idx: None,
            flow: Flow::None,
            runs: Vec::new(),
            events: Vec::new(),
            doctor_running: false,
            approval_queue: VecDeque::new(),
            allow_all: Arc::new(AtomicBool::new(false)),
            status: String::new(),
            should_quit: false,
            ctrl_c_at: None,
            tx,
            handle,
            cfg,
            store,
            session: None,
            session_rows: Vec::new(),
            session_metas: Vec::new(),
            next_run_id: 1,
            done_rounds: HashSet::new(),
        };
        app.refresh_sessions();
        // 首启引导：模型未配置（配置为空或无激活）→ 欢迎引导卡片。
        if !app.configured() {
            app.show_guide();
        }
        app
    }

    /// 自检用演示状态（--selfcheck 渲染帧用；不触碰任何后台执行）。
    pub fn demo(root: PathBuf, tx: StdSender<UiMsg>, handle: tokio::runtime::Handle) -> Self {
        let mut app = Self::new(root, tx, handle);
        app.convo.clear();
        app.convo.push(ConvoItem::User {
            text: "检查工作区并汇报".to_string(),
        });
        app.convo.push(ConvoItem::Assistant {
            text: "我先查看工作区结构。".to_string(),
            reasoning: "用户要检查工作区，我先读一下目录结构。".to_string(),
            expanded: false,
        });
        app.convo.push(ConvoItem::ToolCard {
            id: "c1".to_string(),
            tool: "read".to_string(),
            args: r#"{"path":"README.md"}"#.to_string(),
            state: ToolState::Done {
                ok: true,
                result: "…".to_string(),
            },
        });
        app.convo
            .push(ConvoItem::Note("✓ 任务完成（run #1，共 2 轮）".to_string()));
        app.runs.push(RunEntry {
            id: 1,
            task: "检查工作区并汇报".to_string(),
            status: RunStatus::Completed,
            rounds: 2,
        });
        app.events.push(EventRow {
            seq: 0,
            kind: "run_started",
            trace_id: "tui-1".to_string(),
            summary: "task=检查工作区并汇报 max_rounds=20".to_string(),
        });
        app.session_rows.push(SessionRow {
            id_key: "demo-1".to_string(),
            title: "检查工作区并汇报".to_string(),
            time_label: "刚刚".to_string(),
            count: 4,
        });
        app.session_rows.push(SessionRow {
            id_key: "demo-2".to_string(),
            title: "整理文档".to_string(),
            time_label: "2 小时前".to_string(),
            count: 12,
        });
        app
    }

    /// 首启欢迎引导卡片（模型未配置时）。
    pub fn show_guide(&mut self) {
        self.convo.push(ConvoItem::Card {
            title: "🎉 欢迎使用 sd-agent".to_string(),
            lines: vec![
                "当前未配置模型。".to_string(),
                "输入 /settings 开始配置：端点 URL → 模型名 → API 密钥 → 思考强度，全程无需改文件。"
                    .to_string(),
                "配好后直接输入任务开始；输入 /help 查看全部命令。".to_string(),
            ],
        });
    }

    /// 当前是否已配置可用模型（有配置且有激活项）。
    pub fn configured(&self) -> bool {
        !self.cfg.rows.is_empty() && self.cfg.rows.iter().any(|r| r.active)
    }

    /// 当前激活配置的展示行。
    fn active_row(&self) -> Option<&ProfileRow> {
        self.cfg.rows.iter().find(|r| r.active)
    }

    // ---------------- 顶部状态栏数据 ----------------

    /// 当前会话标题（未命名时给占位）。
    pub fn session_title(&self) -> String {
        match &self.session {
            Some(s) => show_val(&s.title),
            None => "（未命名会话）".to_string(),
        }
    }

    /// 当前模型名（未配置给占位）。
    pub fn model_label(&self) -> String {
        self.active_row()
            .map(|r| r.model.clone())
            .unwrap_or_else(|| "未配置".to_string())
    }

    /// 当前思考强度（未配置给占位）。
    pub fn effort_label(&self) -> String {
        self.active_row()
            .map(|r| format!("{}（{}）", r.effort, slash::effort_desc(&r.effort)))
            .unwrap_or_else(|| "未配置".to_string())
    }

    /// 运行状态（等待审批 > 体检中 > 运行中 > 就绪）。
    pub fn run_state(&self) -> String {
        if !self.approval_queue.is_empty() {
            format!("等待审批（{}）", self.approval_queue.len())
        } else if self.doctor_running {
            "体检中".to_string()
        } else {
            let running = self
                .runs
                .iter()
                .filter(|r| r.status == RunStatus::Running)
                .count();
            if running > 0 {
                format!("运行中（{running}）")
            } else {
                "就绪".to_string()
            }
        }
    }

    // ---------------- 输入框数据（渲染层读取） ----------------

    /// 输入框标题（随引导流程变化）。
    pub fn input_title(&self) -> String {
        match &self.flow {
            Flow::None => " 输入（Enter 发送任务 · Shift+Enter 换行 · / 呼出命令） ".to_string(),
            Flow::SettingsMenu | Flow::ModelMenu | Flow::SessionMenu => {
                " 输入序号或命令（回车确认 · 直接回车返回） ".to_string()
            }
            Flow::ProfileWizard { step, .. } => match *step {
                0 => " 第 1/4 步：输入端点 URL（回车确认 · Esc 取消） ".to_string(),
                1 => " 第 2/4 步：输入模型名（回车确认） ".to_string(),
                2 => " 第 3/4 步：输入 API 密钥（打码不回显 · 回车确认） ".to_string(),
                _ => " 第 4/4 步：选择思考强度（↑↓ 选择 · Enter 确认） ".to_string(),
            },
            Flow::EffortPick { .. } => {
                " 选择思考强度（↑↓ 选择 / 输入序号 · Enter 确认） ".to_string()
            }
            Flow::DeleteConfirm { label } => {
                format!(" 删除配置「{label}」：输入 y 确认，其他取消 ")
            }
        }
    }

    /// 输入框内容是否打码渲染（向导密钥步）。
    pub fn input_masked(&self) -> bool {
        matches!(&self.flow, Flow::ProfileWizard { step: 2, .. })
    }

    /// 是否弹出选择列表（思考强度档位）。
    pub fn pick_open(&self) -> bool {
        matches!(
            &self.flow,
            Flow::EffortPick { .. } | Flow::ProfileWizard { step: 3, .. }
        )
    }

    /// 选择列表数据（标题、条目、选中下标）。
    pub fn pick_items(&self) -> Option<(String, Vec<String>, usize)> {
        let sel = match &self.flow {
            Flow::EffortPick { sel, .. } => *sel,
            Flow::ProfileWizard { sel, .. } => *sel,
            _ => return None,
        };
        let items: Vec<String> = slash::EFFORTS
            .iter()
            .map(|e| format!("{e} — {}", slash::effort_desc(e)))
            .collect();
        Some((
            format!(
                "选择思考强度（{}）· ↑↓ 选择 · Enter 确认 · Esc 取消",
                slash::EFFORT_NOTE
            ),
            items,
            sel.min(slash::EFFORTS.len() - 1),
        ))
    }

    /// 输入框内容行数（含换行；渲染层算高度用）。
    pub fn input_lines(&self) -> usize {
        self.input.split('\n').count().max(1)
    }

    /// 斜杠命令列表是否弹出。
    pub fn slash_open(&self) -> bool {
        matches!(self.flow, Flow::None)
            && slash::is_slash_typing(&self.input)
            && !self.slash_dismissed
    }

    // ---------------- 按键分发 ----------------

    /// 按键分发（审批弹窗优先，其余按引导状态路由）。
    pub fn on_key(&mut self, key: KeyEvent) {
        // Ctrl+C 两按退出（3 秒窗口）。
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            match self.ctrl_c_at {
                Some(t) if t.elapsed() < Duration::from_secs(3) => self.should_quit = true,
                _ => {
                    self.ctrl_c_at = Some(Instant::now());
                    self.status = "再按一次 Ctrl+C 确认退出（3 秒内）".to_string();
                }
            }
            return;
        }
        // 审批弹窗：任何引导状态下拦截按键。
        if !self.approval_queue.is_empty() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.resolve_approval(ApprovalDecision::Approved)
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.resolve_approval(ApprovalDecision::Denied)
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    self.allow_all.store(true, Ordering::SeqCst);
                    self.resolve_approval(ApprovalDecision::AlwaysAllow);
                }
                _ => {}
            }
            return;
        }

        let pick = self.pick_open();
        match key.code {
            KeyCode::Enter => {
                // Shift/Ctrl/Alt+Enter 与 Ctrl+J = 换行。
                if key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    self.input.push('\n');
                    return;
                }
                if pick {
                    self.confirm_pick();
                } else if self.slash_open() {
                    self.exec_selected_slash();
                } else {
                    self.submit();
                }
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.push('\n');
            }
            // Ctrl+T：最近一条思考块展开/收起（与主流终端 agent 一致）。
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toggle_latest_thinking();
            }
            KeyCode::Esc => {
                if self.slash_open() {
                    self.slash_dismissed = true;
                } else if !matches!(self.flow, Flow::None) {
                    self.flow = Flow::None;
                    self.input.clear();
                    self.status = "已取消引导".to_string();
                } else if !self.input.is_empty() {
                    self.input.clear();
                    self.status = "已清空输入".to_string();
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.slash_dismissed = false;
            }
            KeyCode::Up => {
                if self.slash_open() {
                    let len = slash::filter(&self.input).len();
                    if len > 0 {
                        self.slash_sel = (self.slash_sel + len - 1) % len;
                    }
                } else if pick {
                    self.move_pick(-1);
                } else if matches!(self.flow, Flow::None) {
                    self.history_prev();
                }
            }
            KeyCode::Down => {
                if self.slash_open() {
                    let len = slash::filter(&self.input).len();
                    if len > 0 {
                        self.slash_sel = (self.slash_sel + 1) % len;
                    }
                } else if pick {
                    self.move_pick(1);
                } else if matches!(self.flow, Flow::None) {
                    self.history_next();
                }
            }
            KeyCode::Tab if self.slash_open() => {
                let items = slash::filter(&self.input);
                if let Some(cmd) = items.get(self.slash_sel.min(items.len().saturating_sub(1))) {
                    self.input = format!("{} ", cmd.name);
                    self.slash_dismissed = true;
                }
            }
            // PgUp/PgDn 降为辅助翻页（主翻页是鼠标滚轮，见 ui::draw_hints）。
            KeyCode::PageUp => {
                let page = self.view_lines.get().max(5);
                self.scroll_by(page as i32);
            }
            KeyCode::PageDown => {
                let page = self.view_lines.get().max(5);
                self.scroll_by(-(page as i32));
            }
            KeyCode::End => self.scroll_follow = true,
            KeyCode::Char(ch) => {
                if self.input.is_empty() && ch == 'q' && matches!(self.flow, Flow::None) {
                    self.should_quit = true;
                    return;
                }
                self.input.push(ch);
                self.slash_dismissed = false;
            }
            _ => {}
        }
    }

    /// 选择列表移动（负 = 上）。
    fn move_pick(&mut self, delta: i32) {
        let len = slash::EFFORTS.len() as i32;
        let sel = match &mut self.flow {
            Flow::EffortPick { sel, .. } => sel,
            Flow::ProfileWizard { sel, .. } => sel,
            _ => return,
        };
        *sel = ((*sel as i32 + delta).rem_euclid(len)) as usize;
    }

    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.hist_idx {
            None => self.history.len() - 1,
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.hist_idx = Some(idx);
        self.input = self.history[idx].clone();
    }

    fn history_next(&mut self) {
        match self.hist_idx {
            None => {}
            Some(i) if i + 1 < self.history.len() => {
                self.hist_idx = Some(i + 1);
                self.input = self.history[i + 1].clone();
            }
            Some(_) => {
                self.hist_idx = None;
                self.input.clear();
            }
        }
    }

    // ---------------- 发送 / 引导应答 ----------------

    /// Enter：引导态应答 / 斜杠命令 / 新任务。
    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        self.input.clear();
        self.slash_dismissed = false;
        if !matches!(self.flow, Flow::None) {
            self.answer_flow(&text);
            return;
        }
        if text.is_empty() {
            self.status = "输入为空，未发送".to_string();
            return;
        }
        if text.starts_with('/') {
            let name = slash::first_token(&text).to_string();
            let rest = text[name.len()..].trim().to_string();
            if slash::lookup(&name).is_some() {
                self.exec_command(&name, &rest);
            } else {
                self.push_convo(ConvoItem::Error(format!(
                    "未知命令：{name}（输入 /help 查看命令清单）"
                )));
            }
            return;
        }
        // 非斜杠文本 = 新任务。
        self.start_task(text);
    }

    /// 发送新任务：归属当前会话（惰性创建），实时流式显示进度。
    fn start_task(&mut self, task: String) {
        // 历史上下文 = 当前会话已存消息（不含本次任务）。
        let history = self.session_history();
        // 惰性建会话（首任务即标题）。
        if self.session.is_none() {
            let title = task_title(&task);
            if let Some(s) = glue::session_create(&self.store, &title) {
                self.session = Some(s);
            }
        } else if self
            .session
            .as_ref()
            .map(|s| show_val(&s.title))
            .unwrap_or_default()
            == "新会话"
        {
            // 新建会话收到首个任务 → 以任务命名。
            if let Some(s) = self.session.as_mut() {
                s.title = task_title(&task);
                glue::session_save(&self.store, s);
            }
        }
        self.push_convo(ConvoItem::User { text: task.clone() });
        self.session_push("user", &task, "");
        self.history.push(task.clone());
        self.hist_idx = None;

        let id = self.next_run_id;
        self.next_run_id += 1;
        self.runs.push(RunEntry {
            id,
            task: task.clone(),
            status: RunStatus::Pending,
            rounds: 0,
        });
        crate::run::spawn_run(
            self.root.clone(),
            id,
            task,
            history,
            self.cfg.max_rounds,
            self.tx.clone(),
            self.allow_all.clone(),
        );
        self.status = format!("run #{id} 已启动");
    }

    /// 引导流程应答。
    fn answer_flow(&mut self, text: &str) {
        let flow = std::mem::replace(&mut self.flow, Flow::None);
        match flow {
            Flow::SettingsMenu => self.settings_menu_answer(text),
            Flow::ModelMenu => self.model_menu_answer(text),
            Flow::SessionMenu => self.session_menu_answer(text),
            Flow::DeleteConfirm { label } => {
                if text.trim() == "y" || text.trim() == "Y" {
                    self.remove_profile(&label);
                } else {
                    self.push_note("已取消删除");
                }
            }
            Flow::ProfileWizard {
                edit_label,
                step,
                sel,
                mut draft,
            } => {
                let t = text.trim();
                match step {
                    0 => {
                        if t.is_empty() {
                            self.push_convo(ConvoItem::Error("端点 URL 不能为空".to_string()));
                            self.flow = Flow::ProfileWizard {
                                edit_label,
                                step,
                                sel,
                                draft,
                            };
                        } else if !(t.starts_with("http://") || t.starts_with("https://")) {
                            self.push_convo(ConvoItem::Error(
                                "端点 URL 需以 http:// 或 https:// 开头".to_string(),
                            ));
                            self.flow = Flow::ProfileWizard {
                                edit_label,
                                step,
                                sel,
                                draft,
                            };
                        } else {
                            draft.base_url = t.trim_end_matches('/').to_string();
                            self.flow = Flow::ProfileWizard {
                                edit_label,
                                step: 1,
                                sel,
                                draft,
                            };
                        }
                    }
                    1 => {
                        if t.is_empty() {
                            self.push_convo(ConvoItem::Error("模型名不能为空".to_string()));
                            self.flow = Flow::ProfileWizard {
                                edit_label,
                                step,
                                sel,
                                draft,
                            };
                        } else {
                            draft.model = t.to_string();
                            self.flow = Flow::ProfileWizard {
                                edit_label,
                                step: 2,
                                sel,
                                draft,
                            };
                        }
                    }
                    2 => {
                        // 空输入：编辑时保持原密钥，新增时留空。
                        if !t.is_empty() {
                            draft.api_key = t.to_string();
                        }
                        self.flow = Flow::ProfileWizard {
                            edit_label,
                            step: 3,
                            sel: 0,
                            draft,
                        };
                    }
                    _ => {
                        self.flow = Flow::ProfileWizard {
                            edit_label,
                            step,
                            sel,
                            draft,
                        };
                    }
                }
            }
            Flow::EffortPick { .. } | Flow::None => {}
        }
    }

    /// 选择列表 Enter：输入序号优先，否则取高亮项。
    fn confirm_pick(&mut self) {
        let text = self.input.trim().to_string();
        self.input.clear();
        let idx = text
            .parse::<usize>()
            .ok()
            .filter(|n| *n >= 1)
            .map(|n| n - 1);
        let flow = std::mem::replace(&mut self.flow, Flow::None);
        match flow {
            Flow::EffortPick { label, sel } => {
                let i = idx.unwrap_or(sel).min(slash::EFFORTS.len() - 1);
                self.set_effort(&label, slash::EFFORTS[i]);
            }
            Flow::ProfileWizard {
                edit_label,
                sel,
                mut draft,
                ..
            } => {
                let i = idx.unwrap_or(sel).min(slash::EFFORTS.len() - 1);
                draft.effort = slash::EFFORTS[i].to_string();
                self.wizard_save(edit_label, draft);
            }
            other => self.flow = other,
        }
    }

    /// 向导收尾：写入配置（新增/编辑），无激活配置时自动设为当前。
    fn wizard_save(&mut self, edit_label: Option<String>, draft: Draft) {
        let label = edit_label.clone().unwrap_or_else(|| draft.model.clone());
        let had_active = self.cfg.rows.iter().any(|r| r.active);
        let key_note = if draft.api_key.is_empty() {
            "未配置密钥（模型调用可能失败，可稍后 /settings 重新编辑）"
        } else {
            "密钥已保存（内容不显示）"
        };
        glue::upsert_profile(&label, &draft, !had_active);
        self.cfg = glue::cfg_summary();
        self.push_convo(ConvoItem::Card {
            title: format!(
                "✅ 配置{}完成",
                if edit_label.is_some() {
                    "编辑"
                } else {
                    "新增"
                }
            ),
            lines: vec![
                format!(
                    "配置「{label}」→ 模型 {}（思考强度 {}）",
                    draft.model, draft.effort
                ),
                key_note.to_string(),
                "现在直接输入任务开始，或 /help 看命令、/model 切换模型。".to_string(),
            ],
        });
    }

    /// /settings 菜单应答。
    fn settings_menu_answer(&mut self, text: &str) {
        let t = text.trim();
        if t.is_empty() {
            self.status = "已返回输入".to_string();
            return;
        }
        if t == "a" || t == "A" {
            self.flow = Flow::ProfileWizard {
                edit_label: None,
                step: 0,
                sel: 0,
                draft: Draft {
                    effort: "medium".to_string(),
                    ..Default::default()
                },
            };
            self.push_convo(ConvoItem::Card {
                title: "新增模型配置".to_string(),
                lines: vec![
                    "第 1 步：输入端点 URL（如 https://api.openai.com/v1）".to_string(),
                    "后续：模型名 → API 密钥（打码输入）→ 思考强度（列表选择）。".to_string(),
                    "任何一步 Esc 取消；完成后自动保存并可直接使用。".to_string(),
                ],
            });
            return;
        }
        if let Some(rest) = t.strip_prefix('e').or_else(|| t.strip_prefix('E')) {
            match self
                .profile_by_num(rest)
                .map(|r| (r.label.clone(), r.effort.clone()))
            {
                Some((label, effort)) => {
                    let sel = slash::EFFORTS
                        .iter()
                        .position(|e| *e == effort)
                        .unwrap_or(3);
                    self.flow = Flow::EffortPick {
                        label: label.clone(),
                        sel,
                    };
                    self.push_convo(ConvoItem::Card {
                        title: format!("编辑思考强度 · {label}"),
                        lines: vec![
                            format!("当前：{effort}（{}）", slash::effort_desc(&effort)),
                            "↑↓ 选择新档位，Enter 确认（也可输入序号）。".to_string(),
                        ],
                    });
                }
                None => {
                    self.push_convo(ConvoItem::Error("无效序号".to_string()));
                    self.flow = Flow::SettingsMenu;
                }
            }
            return;
        }
        if let Some(rest) = t.strip_prefix('d').or_else(|| t.strip_prefix('D')) {
            match self.profile_by_num(rest).map(|r| r.label.clone()) {
                Some(label) => {
                    self.flow = Flow::DeleteConfirm {
                        label: label.clone(),
                    };
                    self.push_convo(ConvoItem::Card {
                        title: format!("删除配置 · {label}"),
                        lines: vec!["输入 y 确认删除，其他任意输入取消。".to_string()],
                    });
                }
                None => {
                    self.push_convo(ConvoItem::Error("无效序号".to_string()));
                    self.flow = Flow::SettingsMenu;
                }
            }
            return;
        }
        if let Some(label) = self.profile_by_num(t).map(|r| r.label.clone()) {
            self.activate_profile(&label);
        } else {
            self.push_convo(ConvoItem::Error(format!(
                "无效输入：{t}（a 新增 · 序号 切换当前 · e序号 改思考强度 · d序号 删除）"
            )));
            self.flow = Flow::SettingsMenu;
        }
    }

    /// /model 菜单应答。
    fn model_menu_answer(&mut self, text: &str) {
        let t = text.trim();
        if t.is_empty() {
            self.status = "已返回输入".to_string();
            return;
        }
        if let Some(label) = self.profile_by_num(t).map(|r| r.label.clone()) {
            self.activate_profile(&label);
        } else {
            self.push_convo(ConvoItem::Error(
                "无效序号（输入列表序号切换，回车返回）".to_string(),
            ));
            self.flow = Flow::ModelMenu;
        }
    }

    /// /conversations 菜单应答。
    fn session_menu_answer(&mut self, text: &str) {
        let t = text.trim();
        if t.is_empty() {
            self.status = "已返回输入".to_string();
            return;
        }
        if t == "n" || t == "N" {
            self.new_session();
            return;
        }
        if let Ok(n) = t.parse::<usize>() {
            if n >= 1 && n <= self.session_rows.len() {
                self.switch_session(n - 1);
                return;
            }
        }
        self.push_convo(ConvoItem::Error(
            "无效序号（输入列表序号切换会话，n 新建，回车返回）".to_string(),
        ));
        self.flow = Flow::SessionMenu;
    }

    /// 按菜单序号取配置行。
    fn profile_by_num(&self, s: &str) -> Option<&ProfileRow> {
        let n: usize = s.trim().parse().ok()?;
        if n == 0 {
            return None;
        }
        self.cfg.rows.get(n - 1)
    }

    // ---------------- 斜杠命令执行 ----------------

    /// 斜杠列表弹窗 Enter：执行选中命令。
    fn exec_selected_slash(&mut self) {
        let items = slash::filter(&self.input);
        if items.is_empty() {
            return;
        }
        let name = items[self.slash_sel.min(items.len() - 1)].name;
        self.input.clear();
        self.slash_dismissed = true;
        self.exec_command(name, "");
    }

    /// 执行斜杠命令。
    pub fn exec_command(&mut self, name: &str, arg: &str) {
        match name {
            "/help" => self.push_convo(ConvoItem::Card {
                title: "帮助".to_string(),
                lines: slash::help_lines(),
            }),
            "/settings" => self.show_settings_menu(),
            "/model" => self.show_model_menu(),
            "/effort" => self.cmd_effort(arg),
            "/new" => self.new_session(),
            "/conversations" | "/history" => self.show_sessions(),
            "/runs" => self.show_runs(),
            "/trace" => self.show_trace(),
            "/doctor" => self.run_doctor(),
            "/clear" => {
                self.convo.clear();
                self.scroll_follow = true;
                self.status = "已清空对话区".to_string();
            }
            _ => {}
        }
    }

    /// /settings：配置列表 + 子操作菜单。
    fn show_settings_menu(&mut self) {
        self.cfg = glue::cfg_summary();
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!("配置文件：{}", self.cfg.settings_path));
        lines.push(format!("最大轮数：{}", self.cfg.max_rounds));
        if self.cfg.rows.is_empty() {
            lines.push("（暂无模型配置）".to_string());
        }
        for (i, r) in self.cfg.rows.iter().enumerate() {
            let mark = if r.active { "★" } else { " " };
            lines.push(format!(
                "{}) {mark} {} | 模型 {} | 端点 {} | 思考 {} | 密钥 {}",
                i + 1,
                r.label,
                r.model,
                r.base_url,
                r.effort,
                if r.has_key { "已配置" } else { "未配置" }
            ));
        }
        if !self.cfg.missing.is_empty() {
            lines.push(format!(
                "未配置字段：{}（请新增/编辑配置补齐）",
                self.cfg.missing.join(", ")
            ));
        }
        lines.push(String::new());
        lines.push(
            "操作：a 新增 · <序号> 设为当前 · e<序号> 改思考强度 · d<序号> 删除 · 直接回车返回"
                .to_string(),
        );
        self.push_convo(ConvoItem::Card {
            title: "模型配置".to_string(),
            lines,
        });
        self.flow = Flow::SettingsMenu;
    }

    /// /model：当前配置 + 快速切换。
    fn show_model_menu(&mut self) {
        self.cfg = glue::cfg_summary();
        let mut lines: Vec<String> = Vec::new();
        match self.active_row() {
            Some(r) => lines.push(format!(
                "当前：{} → 模型 {}（思考强度 {}）· 密钥 {}",
                r.label,
                r.model,
                r.effort,
                if r.has_key { "已配置" } else { "未配置" }
            )),
            None => lines.push("当前无激活配置（请 /settings 新增）".to_string()),
        }
        lines.push(String::new());
        for (i, r) in self.cfg.rows.iter().enumerate() {
            let mark = if r.active { "★" } else { " " };
            lines.push(format!(
                "{}) {mark} {} | 模型 {} | 思考 {}",
                i + 1,
                r.label,
                r.model,
                r.effort
            ));
        }
        if self.cfg.rows.is_empty() {
            lines.push("（暂无模型配置：/settings 新增）".to_string());
        }
        lines.push(String::new());
        lines.push("输入序号切换当前模型配置，直接回车返回。".to_string());
        self.push_convo(ConvoItem::Card {
            title: "模型切换".to_string(),
            lines,
        });
        self.flow = Flow::ModelMenu;
    }

    /// /effort [档位]：改当前配置的思考强度。
    fn cmd_effort(&mut self, arg: &str) {
        let arg = arg.trim();
        let Some(row) = self.active_row().cloned() else {
            self.push_convo(ConvoItem::Error(
                "当前没有激活的模型配置（请先 /settings 配置）".to_string(),
            ));
            return;
        };
        if arg.is_empty() {
            let sel = slash::EFFORTS
                .iter()
                .position(|e| *e == row.effort)
                .unwrap_or(3);
            self.flow = Flow::EffortPick {
                label: row.label.clone(),
                sel,
            };
            self.push_convo(ConvoItem::Card {
                title: format!("思考强度 · {}", row.label),
                lines: vec![
                    format!(
                        "当前：{}（{}）",
                        row.effort,
                        slash::effort_desc(&row.effort)
                    ),
                    format!("说明：{}", slash::EFFORT_NOTE),
                    "↑↓ 选择新档位，Enter 确认。".to_string(),
                ],
            });
            return;
        }
        if slash::EFFORTS.contains(&arg) {
            self.set_effort(&row.label, arg);
        } else {
            self.push_convo(ConvoItem::Error(format!(
                "未知思考强度：{arg}（可选 none|minimal|low|medium|high|xhigh|max）"
            )));
        }
    }

    /// 设置某配置的思考强度并保存。
    fn set_effort(&mut self, label: &str, effort: &str) {
        glue::set_profile_effort(label, effort);
        self.cfg = glue::cfg_summary();
        self.push_convo(ConvoItem::Card {
            title: "✅ 思考强度已更新".to_string(),
            lines: vec![format!(
                "配置「{label}」→ {}（{}）",
                effort,
                slash::effort_desc(effort)
            )],
        });
    }

    /// 切换当前模型配置。
    fn activate_profile(&mut self, label: &str) {
        glue::set_active_label(label);
        self.cfg = glue::cfg_summary();
        self.push_note(&format!("已切换当前模型配置：「{label}」"));
        self.flow = Flow::None;
    }

    /// 删除模型配置。
    fn remove_profile(&mut self, label: &str) {
        glue::remove_profile_label(label);
        self.cfg = glue::cfg_summary();
        self.push_note(&format!("已删除模型配置：「{label}」"));
    }

    // ---------------- 会话 ----------------

    /// 刷新历史会话列表。
    pub fn refresh_sessions(&mut self) {
        self.session_metas = glue::session_list(&self.store);
        self.session_rows = glue::session_rows(&self.session_metas);
    }

    /// /conversations：历史会话列表 + 选择。
    fn show_sessions(&mut self) {
        self.refresh_sessions();
        let mut lines: Vec<String> = Vec::new();
        if self.session_rows.is_empty() {
            lines.push("（暂无历史会话：发送任务或 /new 新建后自动存档）".to_string());
        }
        for (i, r) in self.session_rows.iter().enumerate() {
            lines.push(format!(
                "{}) {} · {} · {} 条消息",
                i + 1,
                r.title,
                r.time_label,
                r.count
            ));
        }
        lines.push(String::new());
        lines.push("输入序号切换到该会话（恢复上下文继续），n 新建，直接回车返回。".to_string());
        self.push_convo(ConvoItem::Card {
            title: "历史会话".to_string(),
            lines,
        });
        self.flow = Flow::SessionMenu;
    }

    /// /new：当前会话自动存档，新建空会话。
    fn new_session(&mut self) {
        if let Some(s) = &self.session {
            self.push_note(&format!(
                "已存档会话「{}」（{} 条消息）",
                show_val(&s.title),
                s.messages.len()
            ));
        }
        match glue::session_create(&self.store, "新会话") {
            Some(s) => {
                self.session = Some(s);
                self.convo.clear();
                self.scroll_follow = true;
                // 会话语义：全放行随会话复位（新会话重新逐项审批）。
                self.allow_all.store(false, Ordering::SeqCst);
                self.push_note("已新建会话，输入任务开始。（工具审批恢复逐项确认）");
                self.refresh_sessions();
            }
            None => self.push_convo(ConvoItem::Error("新建会话失败".to_string())),
        }
        self.flow = Flow::None;
    }

    /// 切换到历史会话（恢复上下文）。
    fn switch_session(&mut self, idx: usize) {
        let Some(row) = self.session_rows.get(idx).cloned() else {
            self.push_convo(ConvoItem::Error("无效会话序号".to_string()));
            return;
        };
        let Some(meta_idx) = self
            .session_metas
            .iter()
            .position(|m| show_val(&m.id) == row.id_key)
        else {
            self.push_convo(ConvoItem::Error("会话记录不存在（可能已删除）".to_string()));
            return;
        };
        let meta = self.session_metas.remove(meta_idx);
        match glue::session_load(&self.store, meta) {
            Some(sess) => {
                self.convo = convo_from_messages(&sess.messages);
                // 会话语义：全放行随会话复位（切换会话重新逐项审批）。
                self.allow_all.store(false, Ordering::SeqCst);
                self.push_note(&format!(
                    "已切换到会话「{}」（{} 条消息），继续输入即可接续上下文。（工具审批恢复逐项确认）",
                    show_val(&sess.title),
                    sess.messages.len()
                ));
                self.session = Some(sess);
                self.scroll_follow = true;
            }
            None => self.push_convo(ConvoItem::Error("会话加载失败".to_string())),
        }
        self.flow = Flow::None;
    }

    /// 追加一条会话消息并存档（role: "user" / "assistant"；reasoning 随消息存档，
    /// 会话恢复时经 ChatMessage.reasoning_content 带回）。
    fn session_push(&mut self, role: &str, text: &str, reasoning: &str) {
        if let Some(s) = self.session.as_mut() {
            s.messages.push(
                SessionMessage::new(role, text, sd_agent::event::now_unix_ms())
                    .with_reasoning_content(reasoning),
            );
            glue::session_save(&self.store, s);
        }
    }

    /// 当前会话历史 → 核心 ChatMessage（run 的 AgentConfig.history 上下文；
    /// 思考文本经 reasoning_content 带回）。
    fn session_history(&self) -> Vec<ChatMessage> {
        self.session
            .as_ref()
            .map(|s| {
                s.messages
                    .iter()
                    .filter_map(|m| match m.role.as_str() {
                        "user" => Some(ChatMessage::user(m.text.clone())),
                        "assistant" => Some(
                            ChatMessage::assistant(m.text.clone(), Vec::new())
                                .with_reasoning_content(
                                    m.reasoning_content.clone().unwrap_or_default(),
                                ),
                        ),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    // ---------------- 任务 / doctor ----------------

    /// 运行 doctor 体检（结果以卡片流入对话区）。
    pub fn run_doctor(&mut self) {
        if self.doctor_running {
            self.push_note("体检仍在运行中…");
            return;
        }
        self.doctor_running = true;
        self.push_note("🩺 doctor 体检运行中…（含一次真实端点连通探测，请稍候）");
        crate::run::spawn_doctor(&self.handle, self.root.clone(), self.tx.clone());
    }

    /// /runs：任务列表。
    fn show_runs(&mut self) {
        let mut lines: Vec<String> = Vec::new();
        if self.runs.is_empty() {
            lines.push("（暂无任务：直接输入任务开始）".to_string());
        }
        for r in &self.runs {
            lines.push(format!(
                "#{} {} · {} · {} 轮",
                r.id,
                crate::ui::trunc(&r.task, 40),
                r.status.label(),
                r.rounds
            ));
        }
        self.push_convo(ConvoItem::Card {
            title: "任务列表".to_string(),
            lines,
        });
    }

    /// /trace：最近轨迹事件流。
    fn show_trace(&mut self) {
        let mut lines: Vec<String> = Vec::new();
        if self.events.is_empty() {
            lines.push("（暂无事件：事件随任务运行产生）".to_string());
        }
        let start = self.events.len().saturating_sub(30);
        for row in &self.events[start..] {
            lines.push(format!(
                "{:>4} {:<22} [{}] {}",
                row.seq,
                row.kind,
                crate::ui::trunc(&row.trace_id, 12),
                crate::ui::trunc(&row.summary, 80)
            ));
        }
        self.push_convo(ConvoItem::Card {
            title: "轨迹事件流（最近 30 条）".to_string(),
            lines,
        });
    }

    // ---------------- 后台消息 ----------------

    /// 后台消息入口（UI 主线程每帧调用）。
    pub fn on_msg(&mut self, msg: UiMsg) {
        match msg {
            UiMsg::Event { run_id, event } => {
                self.events.push(EventRow {
                    seq: event.seq,
                    kind: event.kind.as_str(),
                    trace_id: event.trace_id.clone(),
                    summary: summarize_event(&event),
                });
                self.apply_event(run_id, &event);
            }
            // ---- 流式增量：追加进对应流式条目（按 run_id + round 归属，多 run 不串）----
            UiMsg::ReasoningDelta {
                run_id,
                round,
                delta,
            } => {
                let item = self.stream_entry(run_id, round);
                if let ConvoItem::Streaming {
                    reasoning,
                    thinking,
                    ..
                } = item
                {
                    reasoning.push_str(&delta); // 增量追加，不重建
                    *thinking = true;
                }
            }
            UiMsg::TextDelta {
                run_id,
                round,
                delta,
            } => {
                let item = self.stream_entry(run_id, round);
                if let ConvoItem::Streaming {
                    text,
                    thinking,
                    expanded,
                    ..
                } = item
                {
                    text.push_str(&delta); // 增量追加，不重建
                    // 思考结束转正文：思考块自动收起（Ctrl+T 可展开）。
                    *thinking = false;
                    *expanded = false;
                }
            }
            UiMsg::ToolCallDelta {
                run_id,
                round,
                name,
                args_so_far,
            } => {
                let item = self.stream_entry(run_id, round);
                if let ConvoItem::Streaming {
                    tool_preview,
                    thinking,
                    ..
                } = item
                {
                    // 工具参数原地更新预览（不新建条目）。
                    *tool_preview = Some((name, args_so_far));
                    *thinking = false;
                }
            }
            // 一轮流结束：流式条目固化为助手回复（思考块保持收起）。
            UiMsg::TurnDone { run_id, round } => {
                self.finalize_stream(run_id, round);
            }
            UiMsg::ModelTurn {
                run_id,
                round,
                text,
                reasoning,
            } => {
                if let Some(run) = self.find_run(run_id) {
                    run.rounds = run.rounds.max(round);
                }
                // 流式路径已收尾 → 以流式结果为准，不再重复入对话流/会话。
                if self.done_rounds.contains(&(run_id, round)) {
                    return;
                }
                if self.find_stream(run_id, round).is_some() {
                    // 流式条目仍在（收尾消息先到）：用完整回复权威回填后固化。
                    self.finalize_stream_with(run_id, round, &text, &reasoning);
                } else if !text.trim().is_empty() {
                    // 兜底路径（无流式输出时）：整轮一次性显示。
                    self.push_convo(ConvoItem::Assistant {
                        text: text.clone(),
                        reasoning: reasoning.clone(),
                        expanded: false,
                    });
                    self.session_push("assistant", &text, &reasoning);
                }
                self.done_rounds.insert((run_id, round));
            }
            UiMsg::RunDone {
                run_id,
                status,
                rounds,
                ..
            } => {
                // 兜底：该 run 还没收尾的流式条目就地固化（防半截流卡在对话区）。
                let pending: Vec<u32> = self
                    .convo
                    .iter()
                    .filter_map(|i| match i {
                        ConvoItem::Streaming {
                            run_id: r,
                            round: n,
                            ..
                        } if *r == run_id => Some(*n),
                        _ => None,
                    })
                    .collect();
                for round in pending {
                    self.finalize_stream(run_id, round);
                }
                if let Some(run) = self.find_run(run_id) {
                    run.status = match status.as_str() {
                        "completed" => RunStatus::Completed,
                        "max_rounds_reached" => RunStatus::Cutoff,
                        _ => RunStatus::Failed,
                    };
                    run.rounds = rounds;
                }
                let note = match status.as_str() {
                    "completed" => format!("✓ 任务完成（run #{run_id}，共 {rounds} 轮）"),
                    "max_rounds_reached" => {
                        format!("⚠ 任务熔断：达到最大轮数（run #{run_id}，{rounds} 轮）")
                    }
                    other => format!("✕ 任务结束：{other}（run #{run_id}，{rounds} 轮）"),
                };
                self.push_convo(ConvoItem::Note(note));
                self.status = format!("run #{run_id} 结束: {status}");
            }
            UiMsg::RunFailed { run_id, error } => {
                if let Some(run) = self.find_run(run_id) {
                    run.status = RunStatus::Failed;
                }
                self.push_convo(ConvoItem::Error(format!(
                    "任务失败（run #{run_id}）：{error}"
                )));
                self.status = format!("run #{run_id} 失败");
            }
            UiMsg::Doctor(report) => {
                self.doctor_running = false;
                self.status = format!(
                    "doctor 体检完成: {}",
                    if report.all_green() {
                        "ALL GREEN"
                    } else {
                        "FAILED"
                    }
                );
                self.push_convo(ConvoItem::Doctor(report));
            }
            UiMsg::Approval {
                run_id,
                request,
                reply,
            } => {
                self.approval_queue.push_back(PendingApproval {
                    run_id,
                    request,
                    reply,
                });
                self.status = format!("run #{run_id} 等待审批（y/n/a）");
            }
        }
    }

    /// 事件 → 工具卡片 / 说明行。
    fn apply_event(&mut self, run_id: u64, event: &Event) {
        match &event.payload {
            EventPayload::RunStarted(p) => {
                if let Some(run) = self.find_run(run_id) {
                    run.status = RunStatus::Running;
                }
                self.push_convo(ConvoItem::Note(format!(
                    "▶ 任务已启动（run #{run_id}，上限 {} 轮）",
                    p.max_rounds
                )));
            }
            EventPayload::ModelTurnStarted(p) => {
                if let Some(run) = self.find_run(run_id) {
                    run.rounds = run.rounds.max(p.round);
                }
                // 先立流式条目壳：思考标头立即可见，增量随后原地追加。
                self.stream_entry(run_id, p.round);
            }
            EventPayload::ToolCallRequested(p) => {
                self.push_convo(ConvoItem::ToolCard {
                    id: p.tool_call_id.clone(),
                    tool: p.tool.clone(),
                    args: p.args_json.clone(),
                    state: ToolState::Pending,
                });
            }
            EventPayload::ToolCallFinished(p) => {
                if let Some(ConvoItem::ToolCard { state, .. }) =
                    self.find_tool_card(&p.tool_call_id)
                {
                    *state = ToolState::Done {
                        ok: p.ok,
                        result: p.result_digest.clone(),
                    };
                }
            }
            EventPayload::ToolCallDenied(p) => {
                if let Some(ConvoItem::ToolCard { state, .. }) =
                    self.find_tool_card(&p.tool_call_id)
                {
                    *state = ToolState::Denied {
                        reason: p.reason.clone(),
                    };
                }
            }
            EventPayload::VerifyResult(p) => {
                self.push_convo(ConvoItem::Note(format!(
                    "验证 {}：{} {}",
                    p.name,
                    if p.ok { "通过" } else { "失败" },
                    p.detail
                )));
            }
            EventPayload::RunFailed(p) => {
                if let Some(run) = self.find_run(run_id) {
                    run.status = RunStatus::Failed;
                }
                self.push_convo(ConvoItem::Error(format!("任务失败：{}", p.error)));
            }
            _ => {}
        }
    }

    fn find_run(&mut self, id: u64) -> Option<&mut RunEntry> {
        self.runs.iter_mut().find(|r| r.id == id)
    }

    fn find_tool_card(&mut self, id: &str) -> Option<&mut ConvoItem> {
        self.convo
            .iter_mut()
            .find(|item| matches!(item, ConvoItem::ToolCard { id: cid, .. } if cid == id))
    }

    /// 找该 run 该轮的流式条目。
    fn find_stream(&mut self, run_id: u64, round: u32) -> Option<&mut ConvoItem> {
        self.convo.iter_mut().find(|item| {
            matches!(item, ConvoItem::Streaming { run_id: r, round: n, .. } if *r == run_id && *n == round)
        })
    }

    /// 取（或建）该 run 该轮的流式条目——增量到达即建壳，后续原地追加。
    fn stream_entry(&mut self, run_id: u64, round: u32) -> &mut ConvoItem {
        if self.find_stream(run_id, round).is_none() {
            self.convo.push(ConvoItem::Streaming {
                run_id,
                round,
                reasoning: String::new(),
                text: String::new(),
                thinking: true,
                tool_preview: None,
                expanded: true,
            });
        }
        self.find_stream(run_id, round)
            .expect("流式条目刚创建，必然存在")
    }

    /// 一轮流收尾：流式条目固化为助手回复（空内容直接丢弃），思考块收起。
    fn finalize_stream(&mut self, run_id: u64, round: u32) {
        self.finalize_stream_with(run_id, round, "", "");
    }

    /// 一轮流收尾（可带完整回复做权威回填；空串 = 沿用已流出内容）。
    fn finalize_stream_with(
        &mut self,
        run_id: u64,
        round: u32,
        full_text: &str,
        full_reasoning: &str,
    ) {
        let Some(item) = self.find_stream(run_id, round) else {
            self.done_rounds.insert((run_id, round));
            return;
        };
        let (mut text, mut reasoning) = match item {
            ConvoItem::Streaming {
                text, reasoning, ..
            } => (text.clone(), reasoning.clone()),
            _ => (String::new(), String::new()),
        };
        if !full_text.trim().is_empty() {
            text = full_text.to_string();
        }
        if !full_reasoning.trim().is_empty() {
            reasoning = full_reasoning.to_string();
        }
        // 移除流式条目，原位替换为固化条目（保留对话流顺序）。
        if let Some(slot) = self
            .convo
            .iter_mut()
            .find(|item| matches!(item, ConvoItem::Streaming { run_id: r, round: n, .. } if *r == run_id && *n == round))
        {
            if text.trim().is_empty() && reasoning.trim().is_empty() {
                *slot = ConvoItem::Note(String::new());
                // 全空轮（如纯工具调用轮）不留空条目。
                self.convo.retain(|i| !matches!(i, ConvoItem::Note(s) if s.is_empty()));
            } else {
                let has_text = !text.trim().is_empty();
                let sess_text = text.clone();
                *slot = ConvoItem::Assistant {
                    text,
                    reasoning: reasoning.clone(),
                    expanded: false,
                };
                if has_text {
                    self.session_push("assistant", &sess_text, &reasoning);
                }
            }
        }
        self.done_rounds.insert((run_id, round));
    }

    /// Ctrl+T：最近一条带思考内容的条目展开/收起切换。
    fn toggle_latest_thinking(&mut self) {
        for item in self.convo.iter_mut().rev() {
            match item {
                ConvoItem::Assistant {
                    reasoning,
                    expanded,
                    ..
                } if !reasoning.is_empty() => {
                    *expanded = !*expanded;
                    self.status = if *expanded {
                        "思考块已展开".to_string()
                    } else {
                        "思考块已收起".to_string()
                    };
                    return;
                }
                ConvoItem::Streaming {
                    reasoning,
                    expanded,
                    ..
                } if !reasoning.is_empty() => {
                    *expanded = !*expanded;
                    return;
                }
                _ => {}
            }
        }
        self.status = "没有可展开的思考块".to_string();
    }

    /// 对话区滚动：delta > 0 = 向上（历史方向），delta < 0 = 向下（回到底部）。
    /// 到底自动恢复跟随；上翻后新内容不再抢滚动位置（流式追加不打扰阅读）。
    pub fn scroll_by(&mut self, delta: i32) {
        let total = self.view_total.get();
        let visible = self.view_lines.get().max(1);
        let bottom_start = total.saturating_sub(visible);
        let cur = if self.scroll_follow {
            bottom_start
        } else {
            self.view_start.get().min(bottom_start)
        };
        let new_start = if delta >= 0 {
            cur.saturating_sub(delta as usize)
        } else {
            (cur + (-delta) as usize).min(bottom_start)
        };
        self.scroll_follow = new_start >= bottom_start;
    }

    /// 追加对话条目（滚动位置不动：跟随态自动可见，翻阅态不被打扰）。
    fn push_convo(&mut self, item: ConvoItem) {
        self.convo.push(item);
    }

    fn push_note(&mut self, text: &str) {
        self.push_convo(ConvoItem::Note(text.to_string()));
    }

    /// 裁决队首审批请求。
    pub fn resolve_approval(&mut self, decision: ApprovalDecision) {
        if let Some(pending) = self.approval_queue.pop_front() {
            let _ = pending.reply.send(decision);
            self.status = format!(
                "run #{} 审批裁决: {}",
                pending.run_id,
                match decision {
                    ApprovalDecision::Approved => "批准",
                    ApprovalDecision::Denied => "拒绝",
                    ApprovalDecision::AlwaysAllow => "本次会话全放行",
                }
            );
        }
    }
}

/// 任务标题（会话命名用：首行截断）。
fn task_title(task: &str) -> String {
    let first = task.lines().next().unwrap_or(task);
    crate::ui::trunc(first, 30)
}

/// 历史会话消息 → 对话流条目（恢复上下文展示）。
fn convo_from_messages(messages: &[SessionMessage]) -> Vec<ConvoItem> {
    let mut items = Vec::new();
    for msg in messages {
        let text = msg.text.trim();
        if text.is_empty() {
            continue;
        }
        match msg.role.as_str() {
            "user" => items.push(ConvoItem::User {
                text: text.to_string(),
            }),
            "assistant" => items.push(ConvoItem::Assistant {
                text: text.to_string(),
                reasoning: msg
                    .reasoning_content
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                // 恢复的历史思考块默认收起（Ctrl+T 展开）。
                expanded: false,
            }),
            _ => {}
        }
    }
    items
}

/// 事件摘要（/trace 每行摘要列）。
pub fn summarize_event(event: &Event) -> String {
    let text = match &event.payload {
        EventPayload::RunStarted(p) => format!("task={} max_rounds={}", p.task, p.max_rounds),
        EventPayload::ModelTurnStarted(p) => {
            format!("round={} history_len={}", p.round, p.history_len)
        }
        EventPayload::ModelTurnFinished(p) => format!(
            "round={} text_chars={} tool_calls={}",
            p.round, p.text_chars, p.tool_call_count
        ),
        EventPayload::ToolCallRequested(p) => {
            format!("{} {} args={}", p.tool_call_id, p.tool, p.args_json)
        }
        EventPayload::ToolApprovalRequested(p) => {
            format!("{} {} {}", p.tool_call_id, p.tool, p.summary)
        }
        EventPayload::ToolApprovalResolved(p) => {
            format!("{} approved={} by={}", p.tool_call_id, p.approved, p.by)
        }
        EventPayload::ToolCallStarted(p) => format!("{} {}", p.tool_call_id, p.tool),
        EventPayload::ToolCallFinished(p) => format!(
            "{} {} ok={} exit={:?} {}ms {}",
            p.tool_call_id, p.tool, p.ok, p.exit_code, p.duration_ms, p.result_digest
        ),
        EventPayload::ToolCallDenied(p) => format!("{} {} {}", p.tool_call_id, p.tool, p.reason),
        EventPayload::RunFinished(p) => format!("rounds={} status={}", p.rounds, p.status),
        EventPayload::RunFailed(p) => format!("error={}", p.error),
        EventPayload::VerifyResult(p) => format!("{} ok={} {}", p.name, p.ok, p.detail),
        EventPayload::Hook(p) => format!("{} {}", p.hook, p.note),
    };
    crate::ui::trunc(&text, 160)
}

// ---------------- 核心 API 适配（glue） ----------------

mod glue {
    //! 核心 sd_agent API 适配集中区：核心契约签名变动只改本小节。
    //!
    //! 展示值统一走 crate::ui::show_val（Debug 文本化 + 去引号/反转义），
    //! 对 String / PathBuf / 枚举等类型差异免疫；密钥只取"是否已配"，
    //! 内容绝不进入任何展示与消息。

    use super::*;
    use sd_agent::config::settings::{ModelProfileCfg, Settings};

    pub fn load_settings() -> Settings {
        Settings::load()
    }

    pub fn save_settings(s: &Settings) {
        let _ = s.save();
    }

    /// 配置总览快照。
    pub fn cfg_summary() -> CfgSummary {
        let s = load_settings();
        let v = s.view();
        let active_label = show_val(&v.active_profile);
        let rows = s
            .profiles
            .iter()
            .map(|p| {
                let label = show_val(&p.label);
                ProfileRow {
                    active: label == active_label && !active_label.is_empty(),
                    label,
                    model: show_val(&p.model),
                    base_url: show_val(&p.base_url),
                    effort: show_val(&p.reasoning_effort).to_lowercase(),
                    has_key: !show_val(&p.api_key).is_empty(),
                }
            })
            .collect();
        CfgSummary {
            rows,
            max_rounds: v.max_rounds as u32,
            settings_path: show_val(&v.settings_path),
            missing: s.missing_fields().iter().map(|m| m.to_string()).collect(),
        }
    }

    /// 写入一条配置（label 为键）；set_active：无任何激活配置时设为当前。
    /// draft.api_key 为空且该 label 已存在 → 保留原密钥（编辑时密钥步回车=不变）。
    pub fn upsert_profile(label: &str, draft: &Draft, set_active: bool) {
        let mut s = load_settings();
        let keep_key = if draft.api_key.is_empty() {
            s.profiles
                .iter()
                .find(|p| show_val(&p.label) == label)
                .map(|p| p.api_key.clone())
        } else {
            None
        };
        let cfg = ModelProfileCfg {
            label: label.to_string(),
            base_url: draft.base_url.clone(),
            model: draft.model.clone(),
            api_key: if draft.api_key.is_empty() {
                keep_key.unwrap_or(None)
            } else {
                Some(draft.api_key.clone())
            },
            reasoning_effort: draft.effort.clone(),
        };
        s.upsert_profile(cfg);
        if set_active {
            s.set_active(label);
        }
        save_settings(&s);
    }

    pub fn set_active_label(label: &str) {
        let mut s = load_settings();
        s.set_active(label);
        save_settings(&s);
    }

    pub fn remove_profile_label(label: &str) {
        let mut s = load_settings();
        s.remove_profile(label);
        // 删掉当前激活项后自动切到第一条剩余配置。
        if show_val(&s.view().active_profile).is_empty() && !s.profiles.is_empty() {
            let first = show_val(&s.profiles[0].label);
            s.set_active(&first);
        }
        save_settings(&s);
    }

    pub fn set_profile_effort(label: &str, effort: &str) {
        let mut s = load_settings();
        if let Some(p) = s.profiles.iter_mut().find(|p| show_val(&p.label) == label) {
            p.reasoning_effort = effort.to_string();
        }
        save_settings(&s);
    }

    // ---- 会话 ----

    pub fn session_store(root: &Path) -> SessionStore {
        SessionStore::new(root)
    }

    pub fn session_list(store: &SessionStore) -> Vec<SessionMeta> {
        store.list()
    }

    pub fn session_create(store: &SessionStore, title: &str) -> Option<Session> {
        store.create(title).ok()
    }

    pub fn session_load(store: &SessionStore, meta: SessionMeta) -> Option<Session> {
        store.load(&meta.id).ok().flatten()
    }

    pub fn session_save(store: &SessionStore, session: &Session) {
        let _ = store.save(session);
    }

    /// 会话元数据 → 展示行。
    pub fn session_rows(metas: &[SessionMeta]) -> Vec<SessionRow> {
        metas
            .iter()
            .map(|m| SessionRow {
                id_key: show_val(&m.id),
                title: show_val(&m.title),
                time_label: rel_time(m.updated_at_ms as u64),
                count: m.message_count as usize,
            })
            .collect()
    }

    /// 时间戳 → 相对时间文案。
    fn rel_time(updated_ms: u64) -> String {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let diff = now_ms.saturating_sub(updated_ms);
        if diff < 60_000 {
            "刚刚".to_string()
        } else if diff < 3_600_000 {
            format!("{} 分钟前", diff / 60_000)
        } else if diff < 86_400_000 {
            format!("{} 小时前", diff / 3_600_000)
        } else {
            format!("{} 天前", diff / 86_400_000)
        }
    }
}
