//! 应用状态机：模式（hub / worker）、动作执行、后台消息分发。
//!
//! 只做状态与动作，不碰渲染（ui/）与键位（keymap.rs）。会话存档与轨迹
//! 是事实源，本状态是观察面 + 控制面。

pub mod archive;
pub mod commands;
pub mod editor;
pub mod hub;
pub mod worker;

use std::path::PathBuf;

use sd_agent::model::ChatMessage;
use sd_agent::policy::ApprovalDecision;
use sd_agent::session::SessionStore;

use crate::bridge::UiMsg;
use crate::keymap::Action;
use crate::run::spawn_run;
use crate::term;
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
        let worker = WorkerState::new(session_id);
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
            last_ctrl_c: None,
        }
    }

    pub fn is_worker(&self) -> bool {
        matches!(self.mode, Mode::Worker(_))
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
        dirty
    }

    /// 执行动作（键位分发后的落点）。
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
            Action::Slash => self.slash_open = true,
            Action::ApproveOnce => self.resolve_approval(ApprovalDecision::Approved),
            Action::ApproveAlways => self.resolve_approval(ApprovalDecision::AlwaysAllow),
            Action::Deny => self.resolve_approval(ApprovalDecision::Denied),
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

    /// Enter 提交：输入以 / 开头 → 斜杠命令；否则按模式发送/启动
    ///（斜杠路由是动作语义不是键位语义，键位判定全在 keymap）。
    pub fn submit_input(&mut self) {
        let text = self.input_text().to_string();
        if text.trim_start().starts_with('/') {
            let line = text.trim().to_string();
            if let Some(ed) = self.editor_opt_pub() {
                ed.submit();
            }
            self.slash_open = false;
            self.exec_command(&line);
            return;
        }
        if self.is_worker() {
            self.send_from_worker();
        } else {
            self.launch_from_hub();
        }
    }

    /// hub 提交：任务落盘到 inbox，弹新终端跑 worker；弹失败降级内嵌执行。
    fn launch_from_hub(&mut self) {
        let task = match &mut self.mode {
            Mode::Hub(h) => {
                let t = h.editor.submit();
                if t.trim().is_empty() {
                    return;
                }
                h.launched = true;
                t
            }
            Mode::Worker(_) => return,
        };
        // 任务交接走文件（跨进程，免转义）：<root>/.sd-agent/inbox/<日期>-<pid>-<毫秒>.txt
        // （毫秒+pid 双因子防同进程同日覆盖串号）。
        let inbox = self.root.join(".sd-agent").join("inbox");
        let _ = std::fs::create_dir_all(&inbox);
        let ts = crate::stats::today_string();
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let path = inbox.join(format!("{ts}-{}-{ms}.txt", std::process::id()));
        if let Err(e) = std::fs::write(&path, &task) {
            self.note(format!("任务文件写入失败: {e}"));
            return;
        }
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("sd-tui"));
        match term::spawn_worker(&exe, &path) {
            Ok(()) => {
                self.note(format!(
                    "已请求弹出新终端执行（任务：{}…）；若窗口未出现，稍后自动可重试",
                    clip(&task, 24)
                ));
                if let Mode::Hub(h) = &mut self.mode {
                    h.refresh(&self.root);
                }
            }
            Err(e) => {
                // 降级：不弹新终端，内嵌直接进 worker（不丢任务）。
                self.note(format!("新终端启动失败（{e}），改内嵌执行"));
                self.embed_worker_with_task(task, path);
            }
        }
    }

    /// 降级路径：本终端转 worker 并直接开跑。
    fn embed_worker_with_task(&mut self, task: String, inbox_path: PathBuf) {
        let _ = std::fs::remove_file(&inbox_path);
        self.mode = Mode::Worker(WorkerState::new(None));
        self.start_run(task);
    }

    /// worker 发送：开 run。
    fn send_from_worker(&mut self) {
        let task = match &mut self.mode {
            Mode::Worker(w) => {
                if w.running.is_some() {
                    w.push(FeedItem::Note("上一条任务还在跑（Esc 取消渲染）".into()));
                    return;
                }
                let t = w.editor.submit();
                if t.trim().is_empty() {
                    return;
                }
                t
            }
            Mode::Hub(_) => return,
        };
        self.start_run(task);
    }

    /// 开一次 run（worker 专用）。
    fn start_run(&mut self, task: String) {
        let Mode::Worker(w) = &mut self.mode else {
            return;
        };
        let run_id = w.alloc_run_id();
        w.last_task = Some(task.clone());
        w.push(FeedItem::User(task.clone()));
        // 历史（会话存档 → ChatMessage）：未恢复会话则空。
        let history = w
            .session_id
            .as_ref()
            .and_then(|id| SessionStore::new(&self.root).load(id).ok().flatten())
            .map(|s| {
                s.messages
                    .iter()
                    .filter_map(|m| {
                        // 只回放 user/assistant 对话正文；tool 消息不回放
                        //（tool_result 需与 assistant.tool_calls 配对，缺失即协议
                        // 非法；工具留痕在轨迹文件，续跑上下文不含工具中间态）。
                        let mut msg = match m.role.as_str() {
                            "user" => ChatMessage::user(m.text.clone()),
                            "assistant" => ChatMessage::assistant(m.text.clone(), vec![]),
                            _ => return None,
                        };
                        if let Some(r) = &m.reasoning_content {
                            msg = msg.with_reasoning_content(r.clone());
                        }
                        Some(msg)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let max_rounds = std::env::var("SD_AGENT_MAX_ROUNDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);
        w.running = Some(run_id);
        w.cancelled = false;
        spawn_run(
            self.root.clone(),
            run_id,
            task,
            history,
            max_rounds,
            self.tx.clone(),
            w.allow_all.clone(),
        );
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

    /// R：重试上一条任务（失败恢复能力）。
    pub(crate) fn retry(&mut self) {
        let task = match &mut self.mode {
            Mode::Worker(w) => {
                if w.running.is_some() {
                    w.push(FeedItem::Note("还在跑，等收尾再重试".into()));
                    return;
                }
                match w.last_task.clone() {
                    Some(t) => t,
                    None => {
                        w.push(FeedItem::Note("没有可重试的任务".into()));
                        return;
                    }
                }
            }
            Mode::Hub(_) => return,
        };
        self.start_run(task);
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

    /// hub 打开选中历史会话（查看摘要进 worker 观察）。
    fn open_selected_session(&mut self) {
        let (id, title) = match &self.mode {
            Mode::Hub(h) => match h.selected_id() {
                Some(id) => {
                    let t = h
                        .sessions
                        .get(h.selected)
                        .map(|s| s.title.clone())
                        .unwrap_or_default();
                    (id.to_string(), t)
                }
                None => return,
            },
            Mode::Worker(_) => return,
        };
        let loaded = SessionStore::new(&self.root).load(&id).ok().flatten();
        let mut w = WorkerState::new(Some(id));
        // 新 worker 的放行位默认关（会话切换即复位）。
        w.allow_all
            .store(false, std::sync::atomic::Ordering::SeqCst);
        w.push(FeedItem::Note(format!("已载入会话：{title}")));
        if let Some(s) = loaded {
            for m in s.messages {
                match m.role.as_str() {
                    "user" => w.push(FeedItem::User(m.text)),
                    "assistant" => w.push(FeedItem::Assistant {
                        text: m.text,
                        reasoning: m.reasoning_content.unwrap_or_default(),
                        streaming: false,
                    }),
                    _ => w.push(FeedItem::Intervention(m.text)),
                }
            }
        }
        // 回放的历史已是库内消息：记账起点=当前 feed 长度（存档只追加新增）。
        w.saved_count = w.feed.len();
        self.mode = Mode::Worker(w);
    }

    /// 输入框当前文本（斜杠弹出过滤用）。
    pub fn input_text(&self) -> &str {
        match &self.mode {
            Mode::Hub(h) => &h.editor.text,
            Mode::Worker(w) => &w.editor.text,
        }
    }
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
