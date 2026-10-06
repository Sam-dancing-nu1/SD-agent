//! worker 会话状态：对话流（消息/工具卡/干预灰字）、运行管理、审批弹窗。
//!
//! worker 是执行面：一条任务一个 run（独立线程），对话流只进不改（流式
//! 追加除外）；会话存档与轨迹是事实源，本状态只是它们的观察面。

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use sd_agent::doctor::DoctorReport;
use sd_agent::policy::{ApprovalDecision, ApprovalRequest};

use super::editor::Editor;
use crate::bridge::UiMsg;

/// 工具卡状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStatus {
    /// 参数流入中。
    Streaming,
    /// 等待审批。
    Pending,
    /// 执行中。
    Running,
    /// 完成（含结果摘要）。
    Done(String),
    /// 被拒/失败（含原因摘要：审批拒绝 / 危险规则 / 白名单 / 执行失败）。
    Failed(String),
}

/// 对话流条目（渲染层按此枚举取样式；干预条目灰字透明）。
#[derive(Debug, Clone)]
pub enum FeedItem {
    /// 用户消息。
    User(String),
    /// 助手正文（含思维链正文，存档续跑回传用——缺失会触发端点 400，
    /// 见核心 model 模块对 reasoning_content 的硬要求）。
    Assistant {
        text: String,
        reasoning: String,
        streaming: bool,
    },
    /// 思维链块（渲染态，可折叠）。
    Reasoning {
        text: String,
        streaming: bool,
        open: bool,
    },
    /// 工具调用卡（tool_call_id 归属；流式阶段 id 暂空，事件到达时补齐）。
    Tool {
        tool_call_id: String,
        name: String,
        args: String,
        status: ToolStatus,
    },
    /// 干预透明条目（系统提示注入/纠偏/记忆召回等）：对话内灰字弱化。
    Intervention(String),
    /// doctor 体检卡片。
    Doctor(DoctorReport),
    /// 错误（修复指引随附）。
    Error { message: String, hint: String },
    /// 状态注记（灰字）。
    Note(String),
}

/// 待裁决审批。
pub struct PendingApproval {
    pub request: ApprovalRequest,
    pub reply: std::sync::mpsc::Sender<ApprovalDecision>,
}

/// worker 状态。
pub struct WorkerState {
    pub feed: Vec<FeedItem>,
    pub editor: Editor,
    /// 当前运行中的 run_id（None=空闲）。
    pub running: Option<u64>,
    next_run_id: u64,
    /// 滚动：距底部偏移（0=贴底自动跟随）。渲染层按尾部窗口取行
    /// （ui::chat::render_feed_window），注释与实现一致。
    pub scroll: u16,
    pub approval: Option<PendingApproval>,
    /// 上一条任务（R 重试用）。
    pub last_task: Option<String>,
    /// 已请求取消的 run（UI 级止损：丢弃其后续增量、后续审批直拒）。
    pub cancelled: bool,
    /// 会话 id（存档用；None=未命名新会话）。
    pub session_id: Option<String>,
    /// 会话级"本次会话全放行"位（跨 run；/new 与切换会话时复位）。
    /// 核心侧另有 run 级 AlwaysAllow 放行位与拒绝熔断（TuiApproval::run_state）。
    pub allow_all: Arc<AtomicBool>,
    /// run 刚收尾标志（App 取走即清，触发会话存档）。
    pub run_just_finished: bool,
    /// 已归档到会话库的消息数（feed 前缀长度）：存档只追加新增，
    /// 防全量替换吃掉旧消息（--session 冷启动 / /clear 场景）。
    pub saved_count: usize,
}

impl WorkerState {
    pub fn new(session_id: Option<String>) -> Self {
        Self {
            feed: Vec::new(),
            editor: Editor::new(),
            running: None,
            next_run_id: 1,
            scroll: 0,
            approval: None,
            last_task: None,
            cancelled: false,
            session_id,
            allow_all: Arc::new(AtomicBool::new(false)),
            run_just_finished: false,
            saved_count: 0,
        }
    }

    /// 新 run 的唯一 id（进程内单调）。
    pub fn alloc_run_id(&mut self) -> u64 {
        let id = self.next_run_id;
        self.next_run_id += 1;
        id
    }

    pub fn push(&mut self, item: FeedItem) {
        self.feed.push(item);
        self.scroll = 0; // 新条目自动贴底（尾部窗口语义）
    }

    /// 是否该丢弃某 run 的增量（已取消）。
    fn drop_run(&self, run_id: u64) -> bool {
        self.cancelled && self.running == Some(run_id)
    }

    /// 按 tool_call_id 找工具卡（空 id 不可匹配）。
    fn tool_mut(&mut self, id: &str) -> Option<&mut FeedItem> {
        if id.is_empty() {
            return None;
        }
        self.feed
            .iter_mut()
            .rev()
            .find(|i| matches!(i, FeedItem::Tool { tool_call_id, .. } if tool_call_id == id))
    }

    /// 按名字找参数流入中的空 id 卡（流式先到、事件补 id 的合并路径）。
    fn streaming_tool_mut(&mut self, name: &str) -> Option<&mut FeedItem> {
        self.feed.iter_mut().rev().find(|i| {
            matches!(
                i,
                FeedItem::Tool {
                    tool_call_id,
                    name: n,
                    status: ToolStatus::Streaming,
                    ..
                } if n == name && tool_call_id.is_empty()
            )
        })
    }

    /// 处理后台消息（追加/原地更新对话流）。返回是否需要重绘。
    pub fn on_msg(&mut self, msg: UiMsg) -> bool {
        match msg {
            UiMsg::ReasoningDelta { run_id, delta, .. } => {
                if self.drop_run(run_id) {
                    return false;
                }
                match self.feed.last_mut() {
                    Some(FeedItem::Reasoning {
                        text,
                        streaming: true,
                        ..
                    }) => text.push_str(&delta),
                    _ => self.push(FeedItem::Reasoning {
                        text: delta,
                        streaming: true,
                        open: true,
                    }),
                }
                true
            }
            UiMsg::TextDelta { run_id, delta, .. } => {
                if self.drop_run(run_id) {
                    return false;
                }
                match self.feed.last_mut() {
                    Some(FeedItem::Assistant {
                        text,
                        streaming: true,
                        ..
                    }) => text.push_str(&delta),
                    _ => self.push(FeedItem::Assistant {
                        text: delta,
                        reasoning: String::new(),
                        streaming: true,
                    }),
                }
                true
            }
            UiMsg::ToolCallDelta {
                run_id,
                name,
                args_so_far,
                ..
            } => {
                if self.drop_run(run_id) {
                    return false;
                }
                // 流式先到：找同名空 id 流入卡更新；无则新建（id 待事件补齐）。
                match self.streaming_tool_mut(&name) {
                    Some(FeedItem::Tool { args, .. }) => *args = args_so_far,
                    _ => self.push(FeedItem::Tool {
                        tool_call_id: String::new(),
                        name,
                        args: args_so_far,
                        status: ToolStatus::Streaming,
                    }),
                }
                true
            }
            UiMsg::TurnDone { run_id, .. } => {
                if self.drop_run(run_id) {
                    return false;
                }
                // 关闭流式态。
                match self.feed.last_mut() {
                    Some(FeedItem::Assistant { streaming, .. })
                    | Some(FeedItem::Reasoning { streaming, .. }) => *streaming = false,
                    _ => {}
                }
                true
            }
            UiMsg::Event { run_id, event } => self.on_event(run_id, event),
            UiMsg::RunDone { status, rounds, .. } => {
                self.running = None;
                self.cancelled = false;
                self.run_just_finished = true;
                self.push(FeedItem::Note(format!(
                    "任务收尾：{status} · 共 {rounds} 轮"
                )));
                true
            }
            UiMsg::RunFailed { error, .. } => {
                self.running = None;
                self.cancelled = false;
                self.run_just_finished = true;
                self.push(FeedItem::Error {
                    message: error,
                    hint: "按 R 重试上一条任务，或 /doctor 体检环境".into(),
                });
                true
            }
            UiMsg::Doctor(report) => {
                self.push(FeedItem::Doctor(report));
                true
            }
            UiMsg::Approval { request, reply, .. } => {
                if self.cancelled {
                    // 已取消的 run 不再放行工具（安全侧直拒，不弹窗）。
                    let _ = reply.send(ApprovalDecision::Denied);
                    return false;
                }
                self.approval = Some(PendingApproval { request, reply });
                true
            }
            UiMsg::SessionSaved { .. } => false,
        }
    }

    /// 轨迹事件 → 对话流（工具卡按 tool_call_id 状态机、干预灰字）。
    fn on_event(&mut self, run_id: u64, event: sd_agent::event::Event) -> bool {
        use sd_agent::event::EventPayload;
        if self.drop_run(run_id) {
            return false;
        }
        match event.payload {
            EventPayload::ToolCallRequested(r) => {
                // 流式卡已先到 → 补 id 并回填权威参数；否则新建（不重复建卡）。
                match self.streaming_tool_mut(&r.tool) {
                    Some(FeedItem::Tool {
                        tool_call_id, args, ..
                    }) => {
                        *tool_call_id = r.tool_call_id;
                        *args = r.args_json;
                    }
                    _ => self.push(FeedItem::Tool {
                        tool_call_id: r.tool_call_id,
                        name: r.tool,
                        args: r.args_json,
                        status: ToolStatus::Streaming,
                    }),
                }
                true
            }
            EventPayload::ToolApprovalRequested(r) => {
                if let Some(FeedItem::Tool { status, .. }) = self.tool_mut(&r.tool_call_id) {
                    *status = ToolStatus::Pending;
                }
                true
            }
            EventPayload::ToolApprovalResolved(r) => {
                let s = if r.approved {
                    ToolStatus::Running
                } else {
                    ToolStatus::Failed("审批拒绝".into())
                };
                if let Some(FeedItem::Tool { status, .. }) = self.tool_mut(&r.tool_call_id) {
                    *status = s;
                }
                true
            }
            EventPayload::ToolCallStarted(r) => {
                if let Some(FeedItem::Tool { status, .. }) = self.tool_mut(&r.tool_call_id) {
                    *status = ToolStatus::Running;
                }
                true
            }
            EventPayload::ToolCallFinished(r) => {
                let s = if r.ok {
                    ToolStatus::Done(r.result_digest)
                } else {
                    ToolStatus::Failed(r.result_digest)
                };
                if let Some(FeedItem::Tool { status, .. }) = self.tool_mut(&r.tool_call_id) {
                    *status = s;
                }
                true
            }
            EventPayload::ToolCallDenied(r) => {
                if let Some(FeedItem::Tool { status, .. }) = self.tool_mut(&r.tool_call_id) {
                    *status = ToolStatus::Failed(r.reason);
                }
                true
            }
            EventPayload::Hook(h) => {
                // 干预透明：系统钩子/注入类事件灰字直接显示在消息流里
                // （工具卡按 id 更新，插队条目不破坏状态机）。
                self.push(FeedItem::Intervention(format!(
                    "[系统] {} · {}",
                    h.hook, h.note
                )));
                true
            }
            EventPayload::VerifyResult(v) => {
                self.push(FeedItem::Intervention(format!(
                    "[验证] {} · {}",
                    if v.ok { "通过" } else { "未通过" },
                    v.detail
                )));
                true
            }
            _ => false,
        }
    }

    /// 裁决审批。
    pub fn resolve_approval(&mut self, decision: ApprovalDecision) {
        if let Some(p) = self.approval.take() {
            let _ = p.reply.send(decision);
        }
    }
}
