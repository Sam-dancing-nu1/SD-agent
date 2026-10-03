//! CLI 子命令面（手写解析，不引 CLI 框架——p0-brief 第五节）。
//!
//! `sd-agent doctor` —— 六项体检（中文报告）；
//! `sd-agent run [--yes] "<task>"` —— 跑一条任务，收尾自动收集证据。
//!
//! 模型配置走 config::Settings 配置链（用户目录 settings.json + 环境变量
//! 回落，多模型配置取当前启用项）：GUI 配过的 CLI 也能直接用。
//! 帮助与报错全中文，报错必带修法提示（面向不懂技术的用户）。
//! main.rs 只做装配调用；逻辑全在本模块（决策 1）。

use std::path::PathBuf;

use crate::agent::{AgentConfig, AgentDeps, AgentError};
use crate::config::{DisciplineTables, EnvProfile, Settings, resource::ResourceLedger};
use crate::event::TraceRecorder;
use crate::model::OpenAiCompatClient;
use crate::policy::{ApprovalPort, AutoApproval, CliApproval};
use crate::verify;

const USAGE: &str = "sd-agent —— 工作区智能体\n\
用法：\n\
  sd-agent doctor                 环境体检（六项检查，中文报告）\n\
  sd-agent run [--yes] \"<任务>\"   跑一条任务，收尾自动收集证据\n\
选项：\n\
  --yes, -y    自动批准工具调用（无人值守演示模式）\n\
  --help, -h   显示本帮助\n";

/// 入口：返回进程退出码（0=成功）。
pub async fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, rest) = match args.split_first() {
        Some((c, rest)) => (c.as_str(), rest),
        None => {
            eprint!("{USAGE}");
            return 2;
        }
    };
    match command {
        "doctor" => cmd_doctor().await,
        "run" => cmd_run(rest).await,
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            0
        }
        other => {
            eprintln!("未知命令：{other}\n{USAGE}");
            2
        }
    }
}

async fn cmd_doctor() -> i32 {
    let root = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("无法确定工作区目录：{e}。修法：换一个可访问的目录再运行。");
            return 2;
        }
    };
    let report = crate::doctor::run_all(&root).await;
    print!("{}", report.render());
    if report.all_green() { 0 } else { 1 }
}

async fn cmd_run(rest: &[String]) -> i32 {
    let mut auto_approve = false;
    let mut task_parts: Vec<String> = Vec::new();
    for arg in rest {
        match arg.as_str() {
            "--yes" | "-y" => auto_approve = true,
            other => task_parts.push(other.to_string()),
        }
    }
    if task_parts.is_empty() {
        eprintln!("缺少任务描述。\n{USAGE}");
        return 2;
    }
    let task = task_parts.join(" ");

    // 装配（显式依赖，逐项失败可诊断）。
    let root = match std::env::current_dir() {
        Ok(p) => PathBuf::from(p),
        Err(e) => {
            eprintln!("无法确定工作区目录：{e}。修法：换一个可访问的目录再运行。");
            return 2;
        }
    };
    // 模型配置走配置链（GUI 配的 CLI 也能用）：取当前启用配置。
    let settings = Settings::load();
    let missing = settings.missing_fields();
    if !missing.is_empty() {
        let names: Vec<&str> = missing.iter().map(|n| field_cn(n)).collect();
        eprintln!("模型配置缺失：{}。", names.join("、"));
        eprintln!("修法：运行 sd-agent doctor 查看，或在桌面端“设置”面板填写。");
        return 2;
    }
    let model = match OpenAiCompatClient::from_settings(&settings) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("模型客户端初始化失败：{e}");
            return 2;
        }
    };

    let trace_id = format!("run-{}-{}", crate::event::now_unix_ms(), std::process::id());
    let recorder = match TraceRecorder::jsonl(&root, &trace_id) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("轨迹文件创建失败：{e}。修法：确认工作区 .sd-agent 目录可写、磁盘未满。");
            return 2;
        }
    };
    let env = EnvProfile::detect();
    let ledger = ResourceLedger::new();
    let tables = DisciplineTables::p0();

    let outcome = {
        let approval: Box<dyn ApprovalPort> = if auto_approve {
            Box::new(AutoApproval)
        } else {
            Box::new(CliApproval)
        };
        let deps = AgentDeps {
            model: &model,
            recorder: &recorder,
            approval: approval.as_ref(),
            env: &env,
            root: &root,
            ledger: &ledger,
            tables: &tables,
            // CLI 单发任务不接流式观察者（流式 delta 由 TUI/桌面壳接线）。
            stream_observer: None,
        };
        let cfg = AgentConfig {
            task: task.clone(),
            max_rounds: settings.max_rounds,
            // CLI 单发任务：不带历史（“恢复会话继续对话”由壳层走 history）。
            history: vec![],
        };
        crate::agent::run_task(&deps, &cfg).await
    };

    let outcome = match outcome {
        Ok(o) => o,
        Err(AgentError::Model(e)) => {
            eprintln!("任务失败（模型）：{e}");
            eprintln!("轨迹：.sd-agent/traces/{trace_id}.jsonl");
            return 1;
        }
        Err(AgentError::Sink(e)) => {
            eprintln!("任务失败（事件记录）：{e}");
            return 1;
        }
    };

    // 收尾证据（硬约束 7：口头完工无效）。
    let evidence = match verify::collect_evidence(&root, &recorder) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("证据收集失败：{e}。修法：确认工作区 .sd-agent 目录可写后重试。");
            return 1;
        }
    };
    println!("=== sd-agent 任务结束 ===");
    println!("任务：      {task}");
    println!("状态：      {}", status_cn(&outcome.status));
    println!("轮次：      {}", outcome.rounds);
    if !outcome.final_text.trim().is_empty() {
        println!("最终回答：\n{}", outcome.final_text.trim());
    }
    println!("轨迹：      .sd-agent/traces/{trace_id}.jsonl");
    println!(
        "证据：      {}",
        crate::sys::normalize_display(&evidence.evidence_file)
    );
    for p in &evidence.probes {
        println!(
            "  探针 {:<16} 通过={} 退出码={:?}",
            p.name, p.ok, p.exit_code
        );
    }
    0
}

/// 配置项英文键 → 中文名（报错用；doctor 侧同口径）。
fn field_cn(name: &str) -> &'static str {
    match name {
        "base_url" => "模型端点（模型服务的网址）",
        "model" => "模型名",
        "api_key" => "API 密钥",
        "profiles" => "模型配置",
        "active_profile" => "当前启用的模型配置",
        _ => "配置项",
    }
}

/// 任务状态英文键 → 中文（展示用）。
fn status_cn(status: &str) -> &'static str {
    match status {
        "completed" => "已完成",
        "max_rounds_reached" => "已达最大轮数上限（熔断）",
        _ => "未知状态",
    }
}
