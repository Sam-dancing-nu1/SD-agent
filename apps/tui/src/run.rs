//! run 装配：把核心依赖（模型 / 轨迹 / 审批 / 环境 / 资源账本 / 纪律表）
//! 组装后 spawn 一次 agent run。
//!
//! 执行形态：一次 run 一条独立 OS 线程 + current_thread tokio runtime——
//! TuiApproval::approve 是同步阻塞语义（等 UI 弹窗裁决），阻塞在 run 自己的
//! 线程上不影响 UI 事件循环、不占共享 runtime worker（与桌面壳同形态）。
//!
//! 模型装配走核心 OpenAiCompatClient::from_settings（读激活 profile）；
//! 会话历史经 AgentConfig.history 传入（恢复会话后接续上下文）。
//! 业务逻辑全部走 sd_agent 核心，本文件只做装配与消息转发。

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

/// ModelClient 装饰器：流式桥 + 整轮兜底上报。
///
/// 流式增量经 StreamRelay（核心 StreamObserver）逐字转发 UI；chat() 是
/// run_task 的调用面——在这里转调 chat_stream 把流打通（核心 run_task 挂载点
/// 落地后由 chat_stream 直通，两路互斥不重复）。整轮 ModelTurn 上报只作
/// 权威回填与无流式兜底（app::on_msg 有去重账）。
struct NotifyingModel {
    inner: OpenAiCompatClient,
    run_id: u64,
    tx: StdSender<UiMsg>,
    round: AtomicU32,
    relay: StreamRelay,
}

impl ModelClient for NotifyingModel {
    fn chat<'a>(
        &'a self,
        request: ChatRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ChatResponse, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            let round = self.round.fetch_add(1, Ordering::SeqCst) + 1;
            // 转调流式入口：delta 经 relay 上抛，回复形态与 chat() 一致。
            let response = self.inner.chat_stream(request, &self.relay).await?;
            let _ = self.tx.send(UiMsg::ModelTurn {
                run_id: self.run_id,
                round,
                text: response.text.clone(),
                reasoning: response.reasoning_content.clone(),
            });
            Ok(response)
        })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest,
        obs: &'a dyn StreamObserver,
    ) -> Pin<Box<dyn Future<Output = Result<ChatResponse, ModelError>> + Send + 'a>> {
        // 核心 run_task 直接挂观察口时：直通核心流式入口。
        Box::pin(async move { self.inner.chat_stream(request, obs).await })
    }
}

/// 轨迹目录：<root>/.sd-agent/traces（壳层唯一命名副本）。
///
/// 与核心 TraceRecorder::jsonl 的落盘约定一致。核心构造器把 sink 焊死为单个
/// JsonlSink（内部 sink 不可取、核心 API 不许改），而 TUI 需要 TeeSink 双写
/// UI 通道，故路径派生收敛在本组函数（run 落盘与回放扫描共用）。
pub fn traces_dir(root: &Path) -> PathBuf {
    root.join(".sd-agent").join("traces")
}

/// 轨迹文件路径：<root>/.sd-agent/traces/<trace_id>.jsonl。
pub fn trace_path(root: &Path, trace_id: &str) -> PathBuf {
    traces_dir(root).join(format!("{trace_id}.jsonl"))
}

/// spawn 一次 run：独立线程 + current_thread tokio runtime。
///
/// 每个 run 独立装配：独立 trace_id、独立轨迹文件（经 TeeSink 双写到 UI）、
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

                // 模型装配：from_settings 读激活 profile（核心契约）。
                let settings = Settings::load();
                let model = match OpenAiCompatClient::from_settings(&settings) {
                    Ok(client) => NotifyingModel {
                        inner: client,
                        run_id,
                        tx: tx.clone(),
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
                    // 流式观察口：核心每轮走 chat_stream，三路 delta 经 relay 实时转发 UI。
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

/// spawn 一次 doctor 体检（结果经 UiMsg::Doctor 回 UI，卡片流入对话区）。
pub fn spawn_doctor(handle: &tokio::runtime::Handle, root: PathBuf, tx: StdSender<UiMsg>) {
    handle.spawn(async move {
        let report = sd_agent::doctor::run_all(&root).await;
        let _ = tx.send(UiMsg::Doctor(report));
    });
}
