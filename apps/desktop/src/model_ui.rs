//! DesktopModelClient：ModelClient 装饰器（四个接口之一的壳侧实现）。
//!
//! 只做三件事：把 chat / chat_stream 原样转给核心 OpenAiCompatClient
//!（启用配置 + reasoning_effort 全链路透传，流式 delta 由核心经
//! StreamObserver 上抛，壳侧实现见 stream_ui.rs）；一轮收尾把完整形态
//! 发前端（model_message，round 取自 ChatRequest.round，与流式增量同源，
//! 前端按 run_id + round 对账定稿）；把模型回复持续 append 进会话
//! （SessionTap）。本文件不重实现任何模型/wire 逻辑。

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};

use sd_agent::config::settings::Settings;
use sd_agent::model::{
    ChatRequest, ChatResponse, ModelClient, ModelError, OpenAiCompatClient, StreamObserver,
};
use tauri::Emitter;

use crate::session_tap::SessionTap;

pub struct DesktopModelClient {
    inner: OpenAiCompatClient,
    app: tauri::AppHandle,
    run_id: String,
    /// 轮次兜底计数（1 起）：request.round 为 0（探针等）时才用它补号。
    fallback_round: AtomicU32,
    /// 会话写入通道（None = 本 run 不挂会话）。
    session: Option<SessionTap>,
}

impl DesktopModelClient {
    /// 从配置链装配：Settings 的启用配置（active）决定端点 / 模型名 /
    /// reasoning_effort；配置不全报 MissingConfig（由上层译成中文引导）。
    pub fn from_settings(
        settings: &Settings,
        session: Option<SessionTap>,
        app: tauri::AppHandle,
        run_id: impl Into<String>,
    ) -> Result<Self, ModelError> {
        Ok(Self {
            inner: OpenAiCompatClient::from_settings(settings)?,
            app,
            run_id: run_id.into(),
            fallback_round: AtomicU32::new(0),
            session,
        })
    }

    /// 一轮收尾的统一处理（chat / chat_stream 共用）：会话追加 + 前端定稿。
    fn finish_turn(&self, round: u32, response: &ChatResponse) {
        // 模型回复持续 append 进会话（纯工具调用的空文本回合不落会话）。
        if let Some(tap) = &self.session {
            if !response.text.is_empty() {
                tap.append_assistant(&response.text);
            }
        }
        let tool_calls: Vec<serde_json::Value> = response
            .tool_calls
            .iter()
            .map(|tc| {
                serde_json::json!({
                    "id": tc.id,
                    "name": tc.name,
                    "arguments_json": tc.arguments_json,
                })
            })
            .collect();
        let _ = self.app.emit(
            "model_message",
            serde_json::json!({
                "run_id": self.run_id,
                "round": round,
                "text": response.text,
                "tool_calls": tool_calls,
            }),
        );
    }

    /// 轮次编号：request.round 优先（run_task 挂载的权威轮次，与流式
    /// 增量回调同源对账）；为 0 时用进程内计数补号。
    fn round_of(&self, request: &ChatRequest) -> u32 {
        if request.round > 0 {
            request.round
        } else {
            self.fallback_round.fetch_add(1, Ordering::SeqCst) + 1
        }
    }
}

impl ModelClient for DesktopModelClient {
    fn chat<'a>(
        &'a self,
        request: ChatRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ChatResponse, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            let round = self.round_of(&request);
            let response = self.inner.chat(request).await?;
            self.finish_turn(round, &response);
            Ok(response)
        })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest,
        obs: &'a dyn StreamObserver,
    ) -> Pin<Box<dyn Future<Output = Result<ChatResponse, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            let round = self.round_of(&request);
            let response = self.inner.chat_stream(request, obs).await?;
            self.finish_turn(round, &response);
            Ok(response)
        })
    }
}
