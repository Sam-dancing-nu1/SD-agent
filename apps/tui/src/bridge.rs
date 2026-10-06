//! 桥接层：核心 sd-agent ↔ UI 主线程的通道与接口适配（壳层唯一跨界点）。
//!
//! 1. TeeSink / ChannelSink：事件双写（落盘 JsonlSink + 转发 UI 通道）；
//! 2. StreamRelay：核心 StreamObserver 实现，流式增量逐条转发 UI（不缓冲）；
//! 3. TuiApproval：核心 ApprovalPort 实现，run 线程阻塞等 UI 弹窗裁决；
//! 4. UiMsg：后台（run / doctor / 轨迹回读）→ UI 主线程的唯一消息枚举。
//!
//! 本文件不含业务逻辑，只做通道与接口适配。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender as StdSender;

use sd_agent::doctor::DoctorReport;
use sd_agent::event::{Event, EventSink, SinkError};
use sd_agent::model::StreamObserver;
use sd_agent::policy::{ApprovalDecision, ApprovalPort, ApprovalRequest, RunApprovalState};

/// 后台 → UI 主线程的消息。
///
/// run_id / round / id / title 等归属字段为协议字段：部分仅被多 run 归属与
/// 事实源回读路径消费，允许暂不读取（多 run 并发与 hub 回读是既定扩展位）。
#[allow(dead_code)]
pub enum UiMsg {
    /// 轨迹事件（ChannelSink 转发，run_id 归属）。
    Event { run_id: u64, event: Event },
    /// 流式增量：思考文本。
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
    /// 流式增量：工具调用参数（原地更新预览）。
    ToolCallDelta {
        run_id: u64,
        round: u32,
        name: String,
        args_so_far: String,
    },
    /// 一轮模型输出流结束。
    TurnDone { run_id: u64, round: u32 },
    /// run 正常收尾（正文已逐轮流式流入，只收状态与轮数）。
    RunDone {
        run_id: u64,
        status: String,
        rounds: u32,
    },
    /// run 以错误收尾。
    RunFailed { run_id: u64, error: String },
    /// doctor 体检结果。
    Doctor(DoctorReport),
    /// 审批弹窗请求（reply 回传裁决）。
    Approval {
        run_id: u64,
        request: ApprovalRequest,
        reply: StdSender<ApprovalDecision>,
    },
    /// 会话存档追加（worker 收尾时由 run 侧发起，hub 回读时也用）。
    SessionSaved { id: String, title: String },
}

/// 流式增量转发器：核心 StreamObserver 的壳层实现（零缓冲直转）。
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

/// 事件通道转发器：核心 EventSink → UiMsg::Event。
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
        // UI 已退出则丢弃（观察面尽力而为，不反压 run）。
        let _ = self.tx.send(UiMsg::Event {
            run_id: self.run_id,
            event,
        });
        Ok(())
    }
}

/// 事件双写扇出：落盘优先（磁盘错误先暴露），再转发 UI。
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
        for s in &self.sinks {
            s.emit(event.clone())?;
        }
        Ok(())
    }
}

/// 审批口：核心 ApprovalPort 的壳层实现。
///
/// approve() 是同步 fn，在 run 专属线程上以 std::sync::mpsc 阻塞等待 UI
/// 主线程弹窗裁决（run 一线程一 current_thread runtime，阻塞只卡自己）。
/// `allow_all` 是会话级"本次会话全放行"开关（裁决 AlwaysAllow 时置位）；
/// `state` 是 run 级放行位与连续拒绝熔断账本（核心 dispatch 消费）。
pub struct TuiApproval {
    run_id: u64,
    tx: StdSender<UiMsg>,
    allow_all: Arc<AtomicBool>,
    state: RunApprovalState,
}

impl TuiApproval {
    pub fn new(run_id: u64, tx: StdSender<UiMsg>, allow_all: Arc<AtomicBool>) -> Self {
        Self {
            run_id,
            tx,
            allow_all,
            state: RunApprovalState::new(),
        }
    }
}

impl ApprovalPort for TuiApproval {
    fn source(&self) -> &'static str {
        "tui"
    }

    fn run_state(&self) -> Option<&RunApprovalState> {
        // 每 run 一个账本（拒绝熔断与 run 级 AlwaysAllow 由核心 dispatch 消费）。
        Some(&self.state)
    }

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
            // UI 已退出：默认拒绝（安全侧），run 走拒绝分支收尾。
            return ApprovalDecision::Denied;
        }
        match reply_rx.recv() {
            Ok(d) => {
                if d == ApprovalDecision::AlwaysAllow {
                    self.allow_all.store(true, Ordering::SeqCst);
                }
                d
            }
            // UI 崩溃/断开：同样按拒绝收尾，禁悬死。
            Err(_) => ApprovalDecision::Denied,
        }
    }
}
