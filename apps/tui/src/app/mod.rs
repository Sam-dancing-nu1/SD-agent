//! 应用状态机：模式（hub / worker）、动作执行、后台消息分发。
//!
//! 只做状态与动作，不碰渲染（ui/）与键位（keymap.rs）。会话存档与轨迹
//! 是事实源，本状态是观察面 + 控制面。执行落点（提交/开会话/落盘）在
//! commands.rs，鼠标交互在 mouse.rs，选区与剪贴板在 clip.rs。

pub mod anim;
pub mod archive;
pub mod clip;
pub mod commands;
pub mod editor;
pub mod form;
pub mod hub;
pub mod mouse;
pub mod slash_ui;
pub mod worker;

use std::path::PathBuf;

use sd_agent::policy::ApprovalDecision;

use crate::bridge::UiMsg;
use crate::keymap::Action;
use hub::HubState;
use worker::{FeedItem, WorkerState};

/// 运行模式。
pub enum Mode {
    /// 主页（观察面）。
    Hub(HubState),
    /// 会话执行（工作面）。
    Worker(WorkerState),
}

/// 焦点区（Tab 轮转）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Input,
    List,
}

/// 应用总状态。
pub struct App {
    pub mode: Mode,
    pub focus: Focus,
    pub should_quit: bool,
    pub help_open: bool,
    pub version: &'static str,
    pub root: PathBuf,
    /// 后台消息通道（run/doctor → UI）。
    pub rx: std::sync::mpsc::Receiver<UiMsg>,
    pub tx: std::sync::mpsc::Sender<UiMsg>,
    /// doctor 所需的 tokio runtime（main 建 multi_thread，共享给后台体检）。
    pub rt: tokio::runtime::Runtime,
    /// 斜杠命令弹出列表（输入以 / 开头）。
    pub slash_open: bool,
    /// 斜杠浮层选中索引（↑↓/滚轮/点击切换，Enter 执行选中；契约冻结，
    /// 渲染层读取高亮、交互层写入）。
    pub slash_sel: usize,
    /// 浮层抑制态（浮层外点击关闭后不自动重开，直到 '/' 前缀消失复位）。
    slash_dismissed: bool,
    /// 每帧渲染登记的可交互区（ui::draw 返回值写入；鼠标命中消费）。
    pub hit: crate::ui::layout::HitRects,
    /// 拖选区：屏幕格坐标 (x,y) 起止（含端点，行主序归一），渲染层高亮消费。
    /// 松开后经 OSC 52 复制并清空（见 mouse.rs / clip.rs）。
    pub selection: Option<clip::Sel>,
    /// 鼠标交互状态（双击检测 / 拖动模式 / 拖选锚点）。
    mouse: mouse::MouseState,
    /// Ctrl+C 连按检测（时间窗）。
    last_ctrl_c: Option<std::time::Instant>,
}

impl App {
    /// hub 模式启动。
    pub fn new_hub(root: PathBuf, version: &'static str, rt: tokio::runtime::Runtime) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let hub = HubState::new(&root);
        Self {
            mode: Mode::Hub(hub),
            focus: Focus::Input,
            should_quit: false,
            help_open: false,
            version,
            root,
            rx,
            tx,
            rt,
            slash_open: false,
            slash_sel: 0,
            slash_dismissed: false,
            hit: crate::ui::layout::HitRects::default(),
            selection: None,
            mouse: mouse::MouseState::default(),
            last_ctrl_c: None,
        }
    }

    /// worker 模式启动（可带初始任务文本）。
    pub fn new_worker(
        root: PathBuf,
        version: &'static str,
        rt: tokio::runtime::Runtime,
        session_id: Option<String>,
    ) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut worker = WorkerState::new(session_id);
        // `--worker --session <id>` 冷启动：载入会话历史继续追问
        //（feed 回放 + 记账起点；run 上下文另由 start_run 组装）。
        worker.replay_session(&root);
        Self {
            mode: Mode::Worker(worker),
            focus: Focus::Input,
            should_quit: false,
            help_open: false,
            version,
            root,
            rx,
            tx,
            rt,
            slash_open: false,
            slash_sel: 0,
            slash_dismissed: false,
            hit: crate::ui::layout::HitRects::default(),
            selection: None,
            mouse: mouse::MouseState::default(),
            last_ctrl_c: None,
        }
    }

    pub fn is_worker(&self) -> bool {
        matches!(self.mode, Mode::Worker(_))
    }

    /// 表单是否打开（main 键位路由用：表单期间只认表单键位）。
    pub fn form_open(&self) -> bool {
        matches!(&self.mode, Mode::Hub(h) if h.form.is_some())
    }

    /// 模型选择浮层是否打开（main 键位路由用）。
    pub fn model_popup_open(&self) -> bool {
        matches!(&self.mode, Mode::Hub(h) if h.model_open)
    }

    /// 表单可编辑态（仅 hub）。
    fn form_mut(&mut self) -> Option<&mut form::FormState> {
        match &mut self.mode {
            Mode::Hub(h) => h.form.as_mut(),
            Mode::Worker(_) => None,
        }
    }

    /// 每帧渲染登记可交互区。`Into` 兜底：渲染路未改 ui::draw 返回值时
    /// 接住 `()`（空表），改成返回 HitRects 后原样接入，调用处零改动。
    pub fn set_hits<H: Into<crate::ui::layout::HitRects>>(&mut self, h: H) {
        self.hit = h.into();
    }

    /// 后台消息：排空通道，逐条分发（返回是否需要重绘）。
    pub fn pump_msgs(&mut self) -> bool {
        let mut dirty = false;
        while let Ok(msg) = self.rx.try_recv() {
            match &mut self.mode {
                Mode::Worker(w) => dirty |= w.on_msg(msg),
                Mode::Hub(h) => {
                    // hub 消费：会话存档通知 → 刷新事实源；doctor → 状态栏摘要。
                    match msg {
                        UiMsg::SessionSaved { .. } => {
                            h.refresh(&self.root);
                            dirty = true;
                        }
                        UiMsg::Doctor(report) => {
                            let total = report.items.len();
                            let green = report.items.iter().filter(|i| i.ok).count();
                            h.status = format!(
                                "体检：{green}/{total} 通过{}（详情在会话里 /doctor）",
                                if green == total { " ✅" } else { " ⚠" }
                            );
                            dirty = true;
                        }
                        _ => {}
                    }
                }
            }
        }
        self.sync_hub_anim();
        dirty
    }

    /// 执行动作（键位/鼠标分发后的落点）。
    pub fn dispatch(&mut self, action: Action) {
        match action {
            Action::Quit => self.should_quit = true,
            Action::QuitConfirm => self.request_quit(),
            Action::Help => self.help_open = !self.help_open,
            Action::Redraw => {}
            Action::FocusNext => {
                self.focus = match self.focus {
                    Focus::Input => Focus::List,
                    Focus::List => Focus::Input,
                }
            }
            Action::Launch => self.submit_input(),
            Action::SelectPrev => {
                if let Mode::Hub(h) = &mut self.mode {
                    h.select_prev()
                }
            }
            Action::SelectNext => {
                if let Mode::Hub(h) = &mut self.mode {
                    h.select_next()
                }
            }
            Action::OpenSelected => self.open_selected_session(),
            Action::NewSession => match &mut self.mode {
                Mode::Hub(h) => h.editor.clear(),
                Mode::Worker(w) => {
                    // 全新会话：feed/session/记账/放行位全部重置（/new 已先存档）。
                    w.editor.clear();
                    w.feed.clear();
                    w.session_id = None;
                    w.saved_count = 0;
                    w.last_task = None;
                    w.scroll = 0;
                    w.allow_all
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                }
            },
            Action::Send => self.submit_input(),
            Action::Newline => {
                let ed = self.editor_mut();
                ed.insert('\n');
            }
            Action::Insert(ch) => self.editor_mut().insert(ch),
            Action::Backspace => self.editor_mut().backspace(),
            Action::Delete => self.editor_mut().delete(),
            Action::MoveLeft => self.editor_mut().move_left(),
            Action::MoveRight => self.editor_mut().move_right(),
            Action::Interrupt => self.interrupt(),
            Action::Retry => self.retry(),
            Action::ScrollUp => self.scroll_by(3),
            Action::ScrollDown => self.scroll_by(-3),
            Action::InputPrev => {
                if let Some(ed) = self.editor_opt() {
                    ed.prev();
                }
            }
            Action::Slash => {
                // 显式呼出：插入 '/' 前缀并开浮层（与直接键入 '/' 同路）。
                self.editor_mut().insert('/');
                self.slash_open = true;
                self.slash_sel = 0;
                self.slash_dismissed = false;
            }
            Action::SlashPrev => self.slash_move(true),
            Action::SlashNext => self.slash_move(false),
            Action::SlashConfirm => self.submit_input(),
            Action::SlashCancel => self.slash_cancel(),
            Action::ApproveOnce => self.resolve_approval(ApprovalDecision::Approved),
            Action::ApproveAlways => self.resolve_approval(ApprovalDecision::AlwaysAllow),
            Action::Deny => self.resolve_approval(ApprovalDecision::Denied),
            // ── 模型选择浮层 ──
            Action::ModelPopupOpen => {
                if let Mode::Hub(h) = &mut self.mode {
                    h.form = None; // 浮层与表单互斥
                    h.model_open = true;
                }
            }
            Action::ModelPopupCancel => {
                if let Mode::Hub(h) = &mut self.mode {
                    h.model_open = false;
                    h.reload_profiles(); // 高亮复位到激活项
                }
            }
            Action::ModelPopupPrev => {
                if let Mode::Hub(h) = &mut self.mode {
                    h.model_prev()
                }
            }
            Action::ModelPopupNext => {
                if let Mode::Hub(h) = &mut self.mode {
                    h.model_next()
                }
            }
            Action::ModelPopupConfirm => self.confirm_model_popup(),
            // ── 思考强度滑条（键盘入口；鼠标走 mouse.rs） ──
            Action::EffortDec => {
                let idx = match &self.mode {
                    Mode::Hub(h) => h.effort_idx.saturating_sub(1),
                    Mode::Worker(_) => 0,
                };
                self.set_effort_idx(idx);
            }
            Action::EffortInc => {
                let idx = match &self.mode {
                    Mode::Hub(h) => h.effort_idx.saturating_add(1),
                    Mode::Worker(_) => 0,
                };
                self.set_effort_idx(idx);
            }
            // ── 配置表单 ──
            Action::FormOpen => {
                if let Mode::Hub(h) = &mut self.mode {
                    h.model_open = false;
                    h.form = Some(form::FormState::add());
                }
            }
            Action::FormCancel => {
                if let Mode::Hub(h) = &mut self.mode {
                    h.form = None;
                }
            }
            Action::FormSubmit => self.submit_form(),
            Action::FormFieldNext => {
                if let Some(f) = self.form_mut() {
                    f.focus_next();
                }
            }
            Action::FormFieldPrev => {
                if let Some(f) = self.form_mut() {
                    f.focus_prev();
                }
            }
            Action::FormEffortLeft => {
                if let Some(f) = self.form_mut() {
                    f.effort_dec();
                }
            }
            Action::FormEffortRight => {
                if let Some(f) = self.form_mut() {
                    f.effort_inc();
                }
            }
            Action::FormInsert(ch) => {
                if let Some(f) = self.form_mut() {
                    f.insert(ch);
                }
            }
            Action::FormBackspace => {
                if let Some(f) = self.form_mut() {
                    f.backspace();
                }
            }
            Action::FormDelete => {
                if let Some(f) = self.form_mut() {
                    f.delete();
                }
            }
        }
        self.sync_slash();
        self.sync_hub_anim();
    }

    /// 两态动画触发点（ToIdle 选择：自动）——hub 回到初始态（无历史且未
    /// launched）且此前动画落点是 WORK 时补一发 ToIdle；ToWork 只在
    /// launch_from_hub 成功时触发（见 commands.rs）。
    fn sync_hub_anim(&mut self) {
        if let Mode::Hub(h) = &mut self.mode {
            if !h.is_work() && h.anim.kind() == Some(anim::AnimKind::ToWork) {
                h.anim.start(anim::AnimKind::ToIdle);
            }
        }
    }

    /// worker 启动后接的 inbox 任务（hub 跨进程交接）：直接开跑。
    pub fn start_inbox_task(&mut self, task: String) {
        self.start_run(task);
    }

    /// run 刚收尾（Some→None）检测：置位由 worker::on_msg 完成，取走即清。
    pub fn take_run_just_finished(&mut self) -> bool {
        match &mut self.mode {
            Mode::Worker(w) => std::mem::take(&mut w.run_just_finished),
            Mode::Hub(_) => false,
        }
    }

    fn editor_mut(&mut self) -> &mut editor::Editor {
        match &mut self.mode {
            Mode::Hub(h) => &mut h.editor,
            Mode::Worker(w) => &mut w.editor,
        }
    }

    fn editor_opt(&mut self) -> Option<&mut editor::Editor> {
        match &mut self.mode {
            Mode::Worker(w) => Some(&mut w.editor),
            Mode::Hub(_) => None,
        }
    }

    /// 公开访问编辑器（main 的文本编辑分支用；hub 与 worker 都可编辑）。
    pub fn editor_opt_pub(&mut self) -> Option<&mut editor::Editor> {
        match &mut self.mode {
            Mode::Hub(h) => Some(&mut h.editor),
            Mode::Worker(w) => Some(&mut w.editor),
        }
    }

    /// Ctrl+C 连按两次（1.2s 窗）退出；单按给提示。
    pub fn request_quit(&mut self) {
        let now = std::time::Instant::now();
        if self
            .last_ctrl_c
            .map(|t| now.duration_since(t) < std::time::Duration::from_millis(1200))
            .unwrap_or(false)
        {
            self.should_quit = true;
        } else {
            self.last_ctrl_c = Some(now);
            self.note("再按一次 Ctrl+C 退出（或输入框为空时按 q）".into());
        }
    }

    /// Enter 提交：输入以 / 开头 → 斜杠命令（浮层开着且是命令前缀时执行
    /// slash_sel 选中项，否则整行）；否则按模式发送/启动（斜杠路由是动作
    /// 语义不是键位语义，键位判定全在 keymap）。
    pub fn submit_input(&mut self) {
        let text = self.input_text().to_string();
        if text.trim_start().starts_with('/') {
            let line = text.trim().to_string();
            // 浮层开着 = 整行是命令前缀（过滤非空）→ Enter/点击执行选中项；
            // 过滤为空（如 /effort high 带参命令）浮层已关，整行照走。
            let exec_line = if self.slash_open {
                match slash_ui::exec_target(&line, self.slash_sel) {
                    slash_ui::ExecTarget::Selected(name) => name.to_string(),
                    slash_ui::ExecTarget::WholeLine => line,
                }
            } else {
                line
            };
            if let Some(ed) = self.editor_opt_pub() {
                ed.submit();
            }
            self.slash_open = false;
            self.slash_sel = 0;
            self.slash_dismissed = false;
            self.exec_command(&exec_line);
            self.sync_slash();
            return;
        }
        if self.is_worker() {
            self.send_from_worker();
        } else {
            self.launch_from_hub();
        }
        self.sync_slash();
    }

    /// Esc：运行中 = UI 级取消（丢弃后续增量，run 线程自然收尾）。
    fn interrupt(&mut self) {
        let Mode::Worker(w) = &mut self.mode else {
            self.help_open = false;
            return;
        };
        if w.approval.is_some() {
            w.resolve_approval(ApprovalDecision::Denied);
            return;
        }
        if w.running.is_some() && !w.cancelled {
            w.cancelled = true;
            w.push(FeedItem::Intervention(
                "[系统] 已请求取消：停止显示后续输出，执行线程将自然收尾（真中断需核心取消通道，[待磨合]）".into(),
            ));
        }
    }

    fn resolve_approval(&mut self, decision: ApprovalDecision) {
        if let Mode::Worker(w) = &mut self.mode {
            w.resolve_approval(decision);
        }
    }

    fn scroll_by(&mut self, delta: i32) {
        match &mut self.mode {
            Mode::Worker(w) => {
                let cur = w.scroll as i32;
                w.scroll = (cur + delta).clamp(0, 10_000) as u16;
            }
            Mode::Hub(h) => {
                let cur = h.scroll as i32;
                h.scroll = (cur + delta).clamp(0, 10_000) as u16;
            }
        }
    }

    /// 输入框当前文本（斜杠弹出过滤用）。
    pub fn input_text(&self) -> &str {
        match &self.mode {
            Mode::Hub(h) => &h.editor.text,
            Mode::Worker(w) => &w.editor.text,
        }
    }
}
