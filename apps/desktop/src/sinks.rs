//! TeeSink：事件双写——JsonlSink 落盘（契约轨迹）+ 转发前端实时流。
//!
//! 落盘链路失败是真故障（返回 Err，核心会处置）；前端转发失败静默降级，
//! 不影响审计口径。转发 payload 为 `{run_id, event}`（event = 契约 wire 形态）。

use sd_agent::event::{Event, EventSink, JsonlSink, SinkError};
use tauri::Emitter;

pub struct TeeSink {
    disk: JsonlSink,
    app: tauri::AppHandle,
    run_id: String,
}

impl TeeSink {
    pub fn open(
        app: tauri::AppHandle,
        run_id: impl Into<String>,
        path: impl Into<std::path::PathBuf>,
    ) -> Result<Self, SinkError> {
        Ok(Self {
            disk: JsonlSink::open(path)?,
            app,
            run_id: run_id.into(),
        })
    }
}

impl EventSink for TeeSink {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        self.disk.emit(event.clone())?;
        if let Ok(payload) = serde_json::to_value(&event) {
            let _ = self.app.emit(
                "agent_event",
                serde_json::json!({ "run_id": self.run_id, "event": payload }),
            );
        }
        Ok(())
    }
}
