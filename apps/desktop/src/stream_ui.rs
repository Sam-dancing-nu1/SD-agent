//! DesktopStreamObserver：StreamObserver 的壳侧实现（流式转发前端）。
//!
//! 契约（sd_agent::model::StreamObserver）：核心流式解析逐段回调
//! on_reasoning_delta（思考，先流）→ on_text_delta（正文，后流）→
//! on_tool_call_delta（工具参数边生成边报）→ on_turn_done（一轮收完）。
//! 本文件只做转发：把增量包成 Tauri 事件发给前端（流式 wire 契约见
//! commands.rs 模块注释），不做任何缓冲/拼接——拼接归前端会话流状态，
//! 完整文本归核心 ChatResponse（model_message 定稿）。
//!
//! 转发失败静默降级：显示通道自身不允许制造执行故障（与 TeeSink 同纪律）。

use sd_agent::model::StreamObserver;
use tauri::Emitter;

pub struct DesktopStreamObserver {
    app: tauri::AppHandle,
    run_id: String,
}

impl DesktopStreamObserver {
    pub fn new(app: tauri::AppHandle, run_id: impl Into<String>) -> Self {
        Self {
            app,
            run_id: run_id.into(),
        }
    }

    /// 一条流式增量事件（kind: "reasoning" | "text"）。
    fn emit_delta(&self, round: u32, kind: &str, delta: &str) {
        let _ = self.app.emit(
            "stream_delta",
            serde_json::json!({
                "run_id": self.run_id,
                "round": round,
                "kind": kind,
                "delta": delta,
            }),
        );
    }
}

impl StreamObserver for DesktopStreamObserver {
    fn on_reasoning_delta(&self, round: u32, delta: &str) {
        self.emit_delta(round, "reasoning", delta);
    }

    fn on_text_delta(&self, round: u32, delta: &str) {
        self.emit_delta(round, "text", delta);
    }

    fn on_tool_call_delta(&self, round: u32, name: &str, args_so_far: &str) {
        let _ = self.app.emit(
            "stream_tool_call",
            serde_json::json!({
                "run_id": self.run_id,
                "round": round,
                "name": name,
                "args_so_far": args_so_far,
            }),
        );
    }

    fn on_turn_done(&self, round: u32) {
        let _ = self.app.emit(
            "stream_turn_done",
            serde_json::json!({
                "run_id": self.run_id,
                "round": round,
            }),
        );
    }
}
