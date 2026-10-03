//! 桥接层：核心 sd-agent ↔ UI 主线程的通道与接口适配（壳层唯一跨界点）。
//!
//! 1. TeeSink / ChannelSink：事件双写——同时落盘 JsonlSink 与转发 UI 通道，
//!    两者都实现核心 EventSink，TeeSink 负责组合扇出（落盘优先，磁盘错误先暴露）；
//! 2. StreamRelay：核心 StreamObserver 的 TUI 实现——流式增量（思考 / 正文 /
//!    工具参数）逐条转发为 UiMsg，按 run_id 归属，多个 run 并发互不串；
//! 3. TuiApproval：核心 ApprovalPort 的 TUI 实现——approve() 是同步 fn，
//!    在 run 专属线程上以 std::sync::mpsc 阻塞等待 UI 主线程弹窗裁决
//!    （run 一线程一 current_thread runtime，阻塞不影响 UI 事件循环与其他
//!    run；UI 主线程渲染弹窗并 send 回裁决）；
//! 4. UiMsg：后台（run / doctor）→ UI 主线程的唯一消息枚举。
//!
//! 本文件不含任何业务逻辑，只做通道与接口适配。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender as StdSender;

use sd_agent::doctor::DoctorReport;
use sd_agent::event::{Event, EventSink, SinkError};
use sd_agent::model::StreamObserver;
use sd_agent::policy::{ApprovalDecision, ApprovalPort, ApprovalRequest};

/// 后台 → UI 主线程的消息。
pub enum UiMsg {
    /// 轨迹事件（ChannelSink 转发；run_id 归属到具体 run）。
    Event { run_id: u64, event: Event },
    /// 流式增量：思考文本（StreamRelay 转发）。
    ReasoningDelta {
        run_id: u64,
        round: u32,
        delta: String,
    },
    /// 流式增量：回复正文。
    TextDelta {
        run_id: u64,
        round: u32,
        delta: String,
    },
    /// 流式增量：工具调用参数（原地更新工具预览）。
    ToolCallDelta {
        run_id: u64,
        round: u32,
        name: String,
        args_so_far: String,
    },
    /// 一轮模型输出流结束（思考/正文/工具参数全部到齐）。
    TurnDone { run_id: u64, round: u32 },
    /// 模型一轮完整回复（事件契约只记字数，正文经 ModelClient 装饰器上报；
    /// 流式路径已逐字流入时仅作权威回填/兜底）。
    ModelTurn {
        run_id: u64,
        round: u32,
        text: String,
        reasoning: String,
    },
    /// 一次 run 正常收尾（模型正文已由 ModelTurn 逐轮流入对话区，
    /// 这里只收状态与轮数，不再携带正文副本）。
    RunDone {
        run_id: u64,
        status: String,
        rounds: u32,
    },
    /// run 以 Err 收尾（模型/轨迹故障）。
    RunFailed { run_id: u64, error: String },
    /// doctor 六项体检结果。
    Doctor(DoctorReport),
    /// 审批弹窗请求（reply 通道回传裁决）。
    Approval {
        run_id: u64,
        request: ApprovalRequest,
        reply: StdSender<ApprovalDecision>,
    },
}

/// 流式增量转发器：核心 StreamObserver 的 TUI 实现。
///
/// 每条增量原样转发为 UiMsg（run_id 归属），不做任何缓冲与合并——
/// UI 线程负责增量追加渲染；UI 已退出则丢弃（观察面尽力而为）。
pub struct StreamRelay {
    run_id: u64,
    tx: StdSender<UiMsg>,
}

impl StreamRelay {
    pub fn new(run_id: u64, tx: StdSender<UiMsg>) -> Self {
        Self { run_id, tx }
    }
}

impl StreamObserver for StreamRelay {
    fn on_reasoning_delta(&self, round: u32, delta: &str) {
        let _ = self.tx.send(UiMsg::ReasoningDelta {
            run_id: self.run_id,
            round,
            delta: delta.to_string(),
        });
    }

    fn on_text_delta(&self, round: u32, delta: &str) {
        let _ = self.tx.send(UiMsg::TextDelta {
            run_id: self.run_id,
            round,
            delta: delta.to_string(),
        });
    }

    fn on_tool_call_delta(&self, round: u32, name: &str, args_so_far: &str) {
        let _ = self.tx.send(UiMsg::ToolCallDelta {
            run_id: self.run_id,
            round,
            name: name.to_string(),
            args_so_far: args_so_far.to_string(),
        });
    }

    fn on_turn_done(&self, round: u32) {
        let _ = self.tx.send(UiMsg::TurnDone {
            run_id: self.run_id,
            round,
        });
    }
}

/// 事件转发到 UI 通道的 sink（观察面，尽力而为：UI 已退出则丢弃，不判故障）。
pub struct ChannelSink {
    run_id: u64,
    tx: StdSender<UiMsg>,
}

impl ChannelSink {
    pub fn new(run_id: u64, tx: StdSender<UiMsg>) -> Self {
        Self { run_id, tx }
    }
}

impl EventSink for ChannelSink {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        let _ = self.tx.send(UiMsg::Event {
            run_id: self.run_id,
            event,
        });
        Ok(())
    }
}

/// 组合 sink：按序提交到多个 sink（TUI 里固定为 落盘 + 转发 两路）。
pub struct TeeSink {
    sinks: Vec<Arc<dyn EventSink>>,
}

impl TeeSink {
    pub fn new(sinks: Vec<Arc<dyn EventSink>>) -> Self {
        Self { sinks }
    }
}

impl EventSink for TeeSink {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        for sink in &self.sinks {
            sink.emit(event.clone())?;
        }
        Ok(())
    }
}

/// TUI 审批口：阻塞等 UI 弹窗裁决。用户按 a 全放行后，后续调用直接放行。
pub struct TuiApproval {
    run_id: u64,
    tx: StdSender<UiMsg>,
    allow_all: Arc<AtomicBool>,
}

impl TuiApproval {
    pub fn new(run_id: u64, tx: StdSender<UiMsg>, allow_all: Arc<AtomicBool>) -> Self {
        Self {
            run_id,
            tx,
            allow_all,
        }
    }
}

impl ApprovalPort for TuiApproval {
    fn approve(&self, request: ApprovalRequest) -> ApprovalDecision {
        if self.allow_all.load(Ordering::SeqCst) {
            return ApprovalDecision::AlwaysAllow;
        }
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        if self
            .tx
            .send(UiMsg::Approval {
                run_id: self.run_id,
                request,
                reply: reply_tx,
            })
            .is_err()
        {
            // UI 线程已退出：保守拒绝。
            return ApprovalDecision::Denied;
        }
        // UI 线程渲染弹窗并回传；UI 消失时 recv 失败同样保守拒绝。
        reply_rx.recv().unwrap_or(ApprovalDecision::Denied)
    }

    fn source(&self) -> &'static str {
        "tui"
    }
}
