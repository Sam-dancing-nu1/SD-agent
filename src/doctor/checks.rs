//! 六项体检的具体检查实现（每项产出 DoctorItem）。

use std::path::Path;

use crate::config::{DisciplineTables, EnvProfile, Settings, resource::ResourceLedger};
use crate::event::{NullSink, TraceRecorder};
use crate::model::{ChatMessage, ChatRequest, ModelClient, OpenAiCompatClient};
use crate::policy::{self, ApprovalPort, AutoApproval, ToolCall};
use crate::sys::OsFamily;

use super::report::DoctorItem;

/// 1. 模型凭据：当前启用配置的 API 密钥在位（只验在位，不显内容）。
pub(super) fn check_credentials(settings: &Settings) -> DoctorItem {
    let name = "model_credentials";
    let title = "模型凭据".to_string();
    let hint = "调用 AI 模型要用的“钥匙”（API 密钥），配好才能让模型干活。".to_string();
    if settings.profiles.is_empty() {
        return DoctorItem {
            name,
            title,
            ok: false,
            detail: "还没有任何模型配置".to_string(),
            hint,
            fix: "在桌面端“设置”面板添加一套模型配置并填写 API 密钥；或设置环境变量 SD_AGENT_API_KEY 后重开程序。".to_string(),
        };
    }
    let Some(profile) = settings.active() else {
        return DoctorItem {
            name,
            title,
            ok: false,
            detail: format!(
                "当前启用的配置“{}”不存在（可能已被删除）",
                settings.view().active_profile
            ),
            hint,
            fix: "在桌面端“设置”面板重新选择一套启用的模型配置。".to_string(),
        };
    };
    if profile.has_api_key() {
        let source = if profile.label == crate::config::settings::ENV_PROFILE_LABEL {
            "环境变量"
        } else {
            "设置文件"
        };
        DoctorItem {
            name,
            title,
            ok: true,
            detail: format!(
                "已配置（当前启用配置“{}”，来源：{source}，内容隐藏）",
                profile.label
            ),
            hint,
            fix: String::new(),
        }
    } else {
        DoctorItem {
            name,
            title,
            ok: false,
            detail: format!("当前启用配置“{}”还没有 API 密钥", profile.label),
            hint,
            fix: "在桌面端“设置”面板给这套配置填写 API 密钥并保存。".to_string(),
        }
    }
}

/// 2. 模型端点与模型名：配置在位 + 一次最小模型调用（回答“连不连得上”）。
pub(super) async fn check_endpoint(settings: &Settings) -> DoctorItem {
    let name = "model_endpoint";
    let title = "模型端点与模型名".to_string();
    let hint = "“端点”是模型服务的网址（形如 https://…/v1），模型名是你要用的那一个；这一项会实际连一次，回答“连不连得上”。".to_string();
    let Some(profile) = settings.active() else {
        return DoctorItem {
            name,
            title,
            ok: false,
            detail: "还没有可测试的模型配置".to_string(),
            hint,
            fix: "在桌面端“设置”面板添加一套模型配置（端点、模型名、API 密钥）。".to_string(),
        };
    };
    let missing = profile.missing_fields();
    if !missing.is_empty() {
        let names: Vec<&str> = missing.iter().map(|f| field_cn(f)).collect();
        return DoctorItem {
            name,
            title,
            ok: false,
            detail: format!(
                "当前启用配置“{}”还缺：{}，无法测试连接",
                profile.label,
                names.join("、")
            ),
            hint,
            fix: "在桌面端“设置”面板把缺的项补齐（端点要含 /v1，模型名照服务商文档填）。"
                .to_string(),
        };
    }
    let client = match OpenAiCompatClient::from_settings(settings) {
        Ok(c) => c,
        Err(e) => {
            return DoctorItem {
                name,
                title,
                ok: false,
                detail: format!("客户端装配失败：{e}"),
                hint,
                fix: "运行 sd-agent doctor 查看其余各项，或在桌面端“设置”面板核对配置。"
                    .to_string(),
            };
        }
    };
    let request = ChatRequest {
        messages: vec![
            ChatMessage::system("你是连通性探针，只回答一个词。"),
            ChatMessage::user("请只回复一个词：pong"),
        ],
        tools: vec![],
        // 探针一次性请求，无轮次语义。
        round: 0,
    };
    match client.chat(request).await {
        Ok(resp) => DoctorItem {
            name,
            title,
            ok: true,
            detail: format!(
                "已连通：当前启用配置“{}”，模型 {} 正常回应（回复 {} 字）",
                profile.label,
                client.model_name(),
                resp.text.trim().chars().count()
            ),
            hint,
            fix: String::new(),
        },
        Err(e) => DoctorItem {
            name,
            title,
            ok: false,
            detail: format!("连不上：{e}"),
            hint,
            fix: "核对“端点”网址是否正确（要含 /v1）、模型名是否存在、API 密钥是否有效；网络或代理异常时稍后重试。".to_string(),
        },
    }
}

/// 3. 工作区可写（.sd-agent/ 探针：写→读→删；失败分支也清理）。
pub(super) fn check_workspace_writable(root: &Path) -> DoctorItem {
    let name = "workspace_writable";
    let title = "工作区可写".to_string();
    let hint = "AI 干活的目录，要能保存文件和记录，不然什么都留不下。".to_string();
    let probe_dir = root.join(".sd-agent").join("doctor-probe");
    let probe_file = probe_dir.join("write-probe.txt");
    let result = std::fs::create_dir_all(&probe_dir)
        .and_then(|_| std::fs::write(&probe_file, b"probe"))
        .and_then(|_| std::fs::read_to_string(&probe_file).map(|_| ()))
        .and_then(|_| std::fs::remove_file(&probe_file))
        .and_then(|_| std::fs::remove_dir(&probe_dir));
    // 无论成败都清垃圾（失败分支可能留半成品）。
    let _ = std::fs::remove_file(&probe_file);
    let _ = std::fs::remove_dir(&probe_dir);
    match result {
        Ok(()) => DoctorItem {
            name,
            title,
            ok: true,
            detail: format!(
                "可以正常保存文件（{}）",
                crate::sys::normalize_display(&root.join(".sd-agent"))
            ),
            hint,
            fix: String::new(),
        },
        Err(e) => DoctorItem {
            name,
            title,
            ok: false,
            detail: format!("写不进去：{e}"),
            hint,
            fix: "确认工作区目录存在且当前用户有写权限；检查磁盘是否已满；换个有写权限的目录再试。"
                .to_string(),
        },
    }
}

/// 4. 命令环境（环境档案显式承载，硬约束 9）。
pub(super) fn check_command_env() -> DoctorItem {
    let env = EnvProfile::detect();
    let os = match env.os_family {
        OsFamily::Windows => "Windows",
        OsFamily::Macos => "macOS",
        OsFamily::Linux => "Linux",
    };
    let (program, args) = env.shell_kind.executor();
    DoctorItem {
        name: "command_env",
        title: "命令环境".to_string(),
        ok: true,
        detail: format!(
            "已识别：{os} 系统，命令由 {program} {} 执行",
            args.join(" ")
        ),
        hint: "AI 执行命令时使用的“外壳”（命令行程序）；识别错了命令会跑不起来。".to_string(),
        fix: String::new(),
    }
}

/// 5. 工具执行：走 policy::dispatch 实测真通道（read 探针文件）。
/// 探针文件只落 .sd-agent/doctor-probe/ 下（不留仓库根垃圾）。
pub(super) async fn check_tool_dispatch(root: &Path) -> DoctorItem {
    let name = "tool_dispatch";
    let title = "工具执行通道".to_string();
    let hint = "AI 通过这条通道真正读写文件、执行命令；它坏了 AI 就什么都做不了。".to_string();
    let fail = |detail: String| DoctorItem {
        name,
        title: title.clone(),
        ok: false,
        detail,
        hint: hint.clone(),
        fix: "重启程序再试；若仍失败，检查安全软件是否拦截文件读写，并确认工作区目录可访问。"
            .to_string(),
    };
    let probe_dir = root.join(".sd-agent").join("doctor-probe");
    if let Err(e) = std::fs::create_dir_all(&probe_dir) {
        return fail(format!("探针目录建不了：{e}"));
    }
    let probe_file = probe_dir.join("dispatch-probe.txt");
    if let Err(e) = std::fs::write(&probe_file, "dispatch-probe-ok") {
        return fail(format!("探针文件写不了：{e}"));
    }
    // 确定性装配：NullSink 轨迹（体检不污染真实轨迹）。
    let recorder = TraceRecorder::new("doctor", std::sync::Arc::new(NullSink));
    let env = EnvProfile::detect();
    let ledger = ResourceLedger::new();
    let tables = DisciplineTables::p0();
    let approval = AutoApproval;
    let ctx = policy::PolicyContext {
        env: &env,
        root,
        recorder: &recorder,
        approval: &approval as &dyn ApprovalPort,
        ledger: &ledger,
        tables: &tables,
    };
    let call = ToolCall {
        id: "doctor-probe".into(),
        name: "read".into(),
        args_json: r#"{"path":".sd-agent/doctor-probe/dispatch-probe.txt"}"#.into(),
    };
    // dispatch 是 async；doctor 本体在 tokio 上下文运行，直接 await
    //（禁止 block_on：tokio runtime 内嵌套阻塞会 panic）。
    let outcome = policy::dispatch(&ctx, &call).await;
    let _ = std::fs::remove_file(&probe_file);
    match outcome {
        Ok(res) if res.ok && res.text.contains("dispatch-probe-ok") => DoctorItem {
            name,
            title,
            ok: true,
            detail: format!(
                "正常：实测读文件成功（记录 {} 条事件）",
                recorder.next_seq()
            ),
            hint,
            fix: String::new(),
        },
        Ok(res) => fail(format!("通道回了意外结果：{}", res.text)),
        Err(e) => fail(format!("通道执行失败：{e}")),
    }
}

/// 6. 版本与依赖信息（rustc 缺失即红；字节稳定断言实跑；依赖版本以
/// Cargo.toml/D5② 登记为准，此处只列登记名防漂移误报）。
pub(super) fn check_versions() -> DoctorItem {
    let name = "versions";
    let title = "程序与版本".to_string();
    let hint = "程序本体和它的开发工具版本；版本不对照样能用，但出问题时不好排查。".to_string();
    let mut rustc_cmd = std::process::Command::new("rustc");
    rustc_cmd.arg("--version");
    crate::sys::no_console_window(&mut rustc_cmd); // GUI 进程防黑窗
    let rustc = rustc_cmd
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let stable = crate::tools::assert_bytes_stable();
    match rustc {
        Some(rustc) => DoctorItem {
            name,
            title,
            ok: stable,
            detail: format!(
                "程序版本 {}，编译工具 {rustc}，工具目录校验：{}",
                env!("CARGO_PKG_VERSION"),
                if stable { "通过" } else { "失败" }
            ),
            hint,
            fix: if stable {
                String::new()
            } else {
                "程序文件校验失败，请重新构建或重新安装 sd-agent。".to_string()
            },
        },
        None => DoctorItem {
            name,
            title,
            ok: false,
            detail: format!(
                "程序版本 {}，没找到编译工具 rustc",
                env!("CARGO_PKG_VERSION")
            ),
            hint,
            fix: "日常使用可忽略这一项；开发调试请先安装 Rust 工具链（rustc）。".to_string(),
        },
    }
}

/// 配置项英文键 → 中文名（体检与报错共用口径）。
fn field_cn(name: &str) -> &'static str {
    match name {
        "base_url" => "模型端点",
        "model" => "模型名",
        "api_key" => "API 密钥",
        "profiles" => "模型配置",
        "active_profile" => "当前启用的模型配置",
        _ => "配置项",
    }
}
