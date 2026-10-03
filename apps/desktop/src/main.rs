//! sd-desktop：sd-agent 桌面壳（Tauri 2）入口。
//!
//! 职责边界：只做装配——managed state、command 注册、窗口启动；
//! 业务逻辑（工具执行 / 模型调用 / 事件契约 / 裁决）全部走核心 sd-agent lib。
//! 前端为纯静态 HTML/JS（dist/），无框架、无 CDN。

// 双击 exe 不弹黑色控制台窗（纯 GUI）。开发期诊断走两条通道：
// 文件日志 .sd-agent/logs/desktop.log，或 SD_DESKTOP_CONSOLE=1 挂回控制台。
#![windows_subsystem = "windows"]

mod approval;
mod commands;
mod logging;
mod model_ui;
mod runner;
mod session_tap;
mod sinks;
mod state;
mod stream_ui;

use state::AppState;

fn main() {
    logging::init();
    logging::log(format!("sd-desktop 启动（pid={}）", std::process::id()));

    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::start_task,
            commands::list_tasks,
            commands::resolve_approval,
            commands::run_doctor,
            commands::test_connection,
            commands::list_traces,
            commands::get_trace_events,
            commands::get_settings,
            commands::save_profiles,
            commands::switch_profile,
            commands::list_sessions,
            commands::create_session,
            commands::load_session,
            commands::delete_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
