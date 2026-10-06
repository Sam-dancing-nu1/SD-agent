//! run 装配：把核心依赖（模型 / 轨迹 / 审批 / 环境 / 资源账本 / 纪律表）
//! 组装后 spawn 一次 agent run。
//!
//! 执行形态：一次 run 一条独立 OS 线程 + current_thread tokio runtime——
//! TuiApproval::approve 是同步阻塞语义，阻塞在 run 自己的线程上不影响 UI
//! 事件循环。模型装配走核心 OpenAiCompatClient::from_settings；会话历史经
//! AgentConfig.history 传入（恢复会话后接续上下文）。业务逻辑全在 sd-agent
//! 核心，本文件只做装配与消息转发。

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender as StdSender;

use sd_agent::agent::{AgentConfig, AgentDeps, run_task};
use sd_agent::config::settings::Settings;
use sd_agent::config::{DisciplineTables, EnvProfile, ResourceLedger};
use sd_agent::event::{JsonlSink, TraceRecorder, now_unix_ms};
use sd_agent::model::{
    ChatMessage, ChatRequest, ChatResponse, ModelClient, ModelError, OpenAiCompatClient,
    StreamObserver,
};

use crate::bridge::{ChannelSink, StreamRelay, TeeSink, TuiApproval, UiMsg};

/// ModelClient 装饰器：流式桥（delta 经 relay 实时上抛）。
///
/// 注：run_task 在挂载流式观察口（stream_observer=Some）时直接走
/// chat_stream，chat() 不会被调用；chat() 仅作 trait 对称实现直通同一
/// 流式入口，不发任何整轮回填消息（UiMsg 无 ModelTurn，流式 delta 是
/// 唯一正文来源）。
struct NotifyingModel {
    inner: OpenAiCompatClient,
    round: AtomicU32,
    relay: StreamRelay,
}

impl ModelClient for NotifyingModel {
    fn chat<'a>(
        &'a self,
        request: ChatRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ChatResponse, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            // trait 对称实现：与 chat_stream 同路（delta 经 relay 上抛）。
            self.round.fetch_add(1, Ordering::SeqCst);
            self.inner.chat_stream(request, &self.relay).await
        })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest,
        obs: &'a dyn StreamObserver,
    ) -> Pin<Box<dyn Future<Output = Result<ChatResponse, ModelError>> + Send + 'a>> {
        Box::pin(async move { self.inner.chat_stream(request, obs).await })
    }
}

/// 轨迹目录：<root>/.sd-agent/traces（壳层与核心落盘约定一致）。
pub fn traces_dir(root: &Path) -> PathBuf {
    root.join(".sd-agent").join("traces")
}

/// 轨迹文件路径。
pub fn trace_path(root: &Path, trace_id: &str) -> PathBuf {
    traces_dir(root).join(format!("{trace_id}.jsonl"))
}

/// spawn 一次 run：独立线程 + current_thread tokio runtime。
///
/// 每个 run 独立装配：独立 trace_id、独立轨迹文件（TeeSink 双写 UI）、
/// 独立审批口（共享同一 allow_all 开关）。history = 当前会话已存消息。
#[allow(clippy::too_many_arguments)]
pub fn spawn_run(
    root: PathBuf,
    run_id: u64,
    task: String,
    history: Vec<ChatMessage>,
    max_rounds: u32,
    tx: StdSender<UiMsg>,
    allow_all: Arc<AtomicBool>,
) {
    let tx_on_spawn_fail = tx.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("sd-run-{run_id}"))
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(UiMsg::RunFailed {
                        run_id,
                        error: format!("tokio runtime 创建失败: {e}"),
                    });
                    return;
                }
            };
            rt.block_on(async move {
                let trace_id = format!("tui-{run_id}-{}", now_unix_ms());
                let jsonl = match JsonlSink::open(trace_path(&root, &trace_id)) {
                    Ok(sink) => sink,
                    Err(e) => {
                        let _ = tx.send(UiMsg::RunFailed {
                            run_id,
                            error: format!("轨迹文件打开失败: {e}"),
                        });
                        return;
                    }
                };
                let tee = TeeSink::new(vec![
                    Arc::new(jsonl),
                    Arc::new(ChannelSink::new(run_id, tx.clone())),
                ]);
                let recorder = TraceRecorder::new(trace_id, Arc::new(tee));

                let settings = Settings::load();
                let model = match OpenAiCompatClient::from_settings(&settings) {
                    Ok(client) => NotifyingModel {
                        inner: client,
                        round: AtomicU32::new(0),
                        relay: StreamRelay::new(run_id, tx.clone()),
                    },
                    Err(e) => {
                        let _ = tx.send(UiMsg::RunFailed {
                            run_id,
                            error: format!("模型客户端装配失败: {e}"),
                        });
                        return;
                    }
                };
                let approval = TuiApproval::new(run_id, tx.clone(), allow_all);
                let env = EnvProfile::detect();
                let ledger = ResourceLedger::new();
                let tables = DisciplineTables::p0();

                let deps = AgentDeps {
                    model: &model,
                    recorder: &recorder,
                    approval: &approval,
                    env: &env,
                    root: &root,
                    ledger: &ledger,
                    tables: &tables,
                    stream_observer: Some(&model.relay as &dyn StreamObserver),
                };
                let cfg = AgentConfig {
                    task,
                    max_rounds,
                    history,
                };
                match run_task(&deps, &cfg).await {
                    Ok(out) => {
                        let _ = tx.send(UiMsg::RunDone {
                            run_id,
                            status: out.status,
                            rounds: out.rounds,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::RunFailed {
                            run_id,
                            error: e.to_string(),
                        });
                    }
                }
            });
        });
    if let Err(e) = spawned {
        let _ = tx_on_spawn_fail.send(UiMsg::RunFailed {
            run_id,
            error: format!("run 线程启动失败: {e}"),
        });
    }
}

/// 在指定 runtime handle 上跑一次 doctor 体检（结果经 UiMsg::Doctor 回 UI）。
pub fn spawn_doctor(handle: &tokio::runtime::Handle, root: PathBuf, tx: StdSender<UiMsg>) {
    handle.spawn(async move {
        let report = sd_agent::doctor::run_all(&root).await;
        let _ = tx.send(UiMsg::Doctor(report));
    });
}
