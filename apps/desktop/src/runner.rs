//! run 执行线程：一次 run 一个 OS 线程 + current_thread tokio runtime。
//!
//! 为什么不用全局 tokio 多线程 runtime：ApprovalPort::approve 是同步阻塞
//! 语义，阻塞在 run 自己的线程上不影响 UI 事件循环、也不占全局 worker。
//! run_task 本体、工具执行、模型调用全部走核心 lib，本文件只做壳层装配。
//!
//! 会话接入：历史先读、任务后写（本次任务不进自己的上下文），模型回复由
//! DesktopModelClient 持续 append；恢复会话续跑走 AgentConfig.history。

use std::sync::Arc;

use sd_agent::agent::{AgentConfig, AgentDeps, run_task};
use sd_agent::config::settings::Settings;
use sd_agent::config::{DisciplineTables, EnvProfile, ResourceLedger};
use sd_agent::event::{EventSink, TraceRecorder};
use tauri::Emitter;

use crate::approval::DesktopApproval;
use crate::model_ui::DesktopModelClient;
use crate::session_tap::SessionTap;
use crate::sinks::TeeSink;
use crate::state::{self, AppState, RunInfo};
use crate::stream_ui::DesktopStreamObserver;

/// 启动一次 run：登记 → 起线程 → 立即返回 RunInfo（前端拿到列表即时反馈）。
pub fn spawn_run(
    app: tauri::AppHandle,
    state: &AppState,
    name: String,
    task: String,
    session_id: String,
) -> Result<RunInfo, String> {
    let run_id = state.next_run_id();
    let trace_id = run_id.clone();

    // 模型装配走配置链（Settings::load → from_settings，启用配置决定
    // 端点/模型名/reasoning_effort），配置不全在起跑前报中文错误
    // （快速失败，不留僵尸 run）。
    let settings = Settings::load();
    if let Err(msg) = crate::commands::ensure_runnable(&settings) {
        return Err(msg);
    }
    let max_rounds = settings.max_rounds;

    // 历史先读、任务后 append：本次任务不进自己的上下文。
    let tap = SessionTap::new(&state.root, &session_id);
    let history = tap.history();
    tap.append_user(&task);

    let model = DesktopModelClient::from_settings(
        &settings,
        Some(SessionTap::new(&state.root, &session_id)),
        app.clone(),
        run_id.clone(),
    )
    .map_err(|e| format!("模型装配失败: {e}"))?;

    // 轨迹命名走壳内唯一入口（state.trace_path，与 list/get_trace 同源）。
    let trace_path = state.trace_path(&trace_id);
    let sink = TeeSink::open(app.clone(), run_id.clone(), trace_path)
        .map_err(|e| format!("轨迹文件打开失败: {e}"))?;

    let info = RunInfo {
        id: run_id.clone(),
        name,
        task: task.clone(),
        status: "running".to_string(),
        rounds: 0,
        trace_id: trace_id.clone(),
        final_text: String::new(),
        error: None,
    };
    state.push_run(info.clone());
    let _ = app.emit("run_update", &info);

    let runs = Arc::clone(&state.runs);
    let pending = Arc::clone(&state.pending);
    let root = state.root.clone();

    std::thread::Builder::new()
        .name(format!("sd-run-{run_id}"))
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                let sink: Arc<dyn EventSink> = Arc::new(sink);
                let recorder = TraceRecorder::new(trace_id, sink);
                let approval = DesktopApproval::new(app.clone(), run_id.clone(), pending);
                // 流式观察者：reasoning / 正文 / 工具参数三路增量实时转发前端。
                let observer = DesktopStreamObserver::new(app.clone(), run_id.clone());
                let env = EnvProfile::detect();
                let ledger = ResourceLedger::new();
                let tables = DisciplineTables::p0();

                let cfg = AgentConfig {
                    task,
                    max_rounds,
                    history,
                };
                let deps = AgentDeps {
                    model: &model,
                    recorder: &recorder,
                    approval: &approval,
                    env: &env,
                    root: &root,
                    ledger: &ledger,
                    tables: &tables,
                    stream_observer: Some(&observer),
                };

                match run_task(&deps, &cfg).await {
                    Ok(out) => {
                        state::finish_run(
                            &runs,
                            &run_id,
                            &out.status,
                            out.rounds,
                            &out.final_text,
                            None,
                        );
                    }
                    Err(e) => {
                        // 失败 run 的轮数不可知，如实记 0（禁止编数）。
                        state::finish_run(&runs, &run_id, "failed", 0, "", Some(e.to_string()));
                    }
                }
                // 收尾后同步一次列表（前端以此刷新状态徽标）。
                if let Some(latest) = runs
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .iter()
                    .find(|r| r.id == run_id)
                    .cloned()
                {
                    let _ = app.emit("run_update", &latest);
                }
            });
        })
        .map_err(|e| format!("run 线程启动失败: {e}"))?;

    Ok(info)
}
