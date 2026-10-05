//! doctor：六项体检（p0-brief 第四节 6），全中文、面向不懂技术的用户。
//!
//! 口径：
//! - 模型相关两项按**多模型配置**口径报：只看当前启用配置（active()）的
//!   状态，凭据只验在位不显内容，端点项发一次最小模型调用回答"连不连得上"；
//! - 数据源是 config::Settings 配置链（用户目录 settings.json + 环境变量
//!   回落）：GUI 配了就算配，不再只认环境变量；
//! - 工具执行探针走 policy::dispatch 实测真通道（防"体检全绿但通道坏"）；
//! - 每项产出：中文短标题 + 一句话状态 + 一句大白话说明 + 失败时的具体修法；
//! - 铁律 5（口头完工无效）：体检结果同时落盘 `.sd-agent/evidence/doctor-<ts>.txt`。

mod checks;
mod report;

pub use report::{DoctorItem, DoctorReport};

use std::path::Path;

use crate::config::Settings;

use checks::{
    check_command_env, check_credentials, check_endpoint, check_tool_dispatch, check_versions,
    check_workspace_writable,
};

/// 六项体检（顺序固定：模型凭据 / 模型端点与模型名 / 工作区可写 /
/// 命令环境 / 工具执行通道 / 程序与版本）。
pub async fn run_all(root: &Path) -> DoctorReport {
    // 体检数据源 = 完整配置链（文件 + 环境变量回落），GUI 配了就算配。
    let settings = Settings::load();
    let mut items = Vec::new();
    items.push(check_credentials(&settings));
    items.push(check_endpoint(&settings).await);
    items.push(check_workspace_writable(root));
    items.push(check_command_env());
    items.push(check_tool_dispatch(root).await);
    items.push(check_versions());
    let report = DoctorReport { items };
    // 落盘证据（与终端 render 同文；写失败不掩盖体检结果，但 stderr 留痕）。
    let ts = crate::event::now_unix_ms();
    let evidence_dir = root.join(".sd-agent").join("evidence");
    let written = std::fs::create_dir_all(&evidence_dir).and_then(|_| {
        std::fs::write(
            evidence_dir.join(format!("doctor-{ts}.txt")),
            report.render(),
        )
    });
    if let Err(e) = written {
        eprintln!("[体检] 证据文件写入失败：{e}");
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::env_lock;

    /// 网络隔离（M7）+ 配置隔离：USERPROFILE 指向临时目录、摘掉全部
    /// SD_AGENT_* 环境变量，体检走本地失败路径，绝不真实打模型端点
    ///（不耗钱、不留访问痕迹）。edition 2024 下环境变量操作是 unsafe，
    /// 测试内短临界区可接受（env_lock 串行化）。
    async fn run_scrubbed(tag: &str) -> (DoctorReport, std::path::PathBuf) {
        let _guard = env_lock();
        let dir = std::env::temp_dir().join(format!("sd-doctor-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let saved = (
            std::env::var("USERPROFILE").ok(),
            std::env::var("SD_AGENT_BASE_URL").ok(),
            std::env::var("SD_AGENT_MODEL").ok(),
            std::env::var("SD_AGENT_API_KEY").ok(),
            std::env::var("SD_AGENT_MAX_ROUNDS").ok(),
        );
        unsafe {
            std::env::set_var("USERPROFILE", &dir);
            std::env::remove_var("SD_AGENT_BASE_URL");
            std::env::remove_var("SD_AGENT_MODEL");
            std::env::remove_var("SD_AGENT_API_KEY");
            std::env::remove_var("SD_AGENT_MAX_ROUNDS");
        }
        let report = run_all(&dir).await;
        unsafe {
            if let Some(v) = saved.0 {
                std::env::set_var("USERPROFILE", v);
            } else {
                std::env::remove_var("USERPROFILE");
            }
            for (name, val) in [
                ("SD_AGENT_BASE_URL", saved.1),
                ("SD_AGENT_MODEL", saved.2),
                ("SD_AGENT_API_KEY", saved.3),
                ("SD_AGENT_MAX_ROUNDS", saved.4),
            ] {
                if let Some(v) = val {
                    std::env::set_var(name, v);
                } else {
                    std::env::remove_var(name);
                }
            }
        }
        (report, dir)
    }

    #[tokio::test]
    async fn doctor_reports_six_items() {
        let (report, dir) = run_scrubbed("six").await;
        assert_eq!(report.items.len(), 6);
        let names: Vec<&str> = report.items.iter().map(|i| i.name).collect();
        assert_eq!(
            names,
            vec![
                "model_credentials",
                "model_endpoint",
                "workspace_writable",
                "command_env",
                "tool_dispatch",
                "versions"
            ]
        );
        // 隔离环境下模型两项应为红（没配置），工作区/命令/工具三项应为绿。
        assert!(!report.items[0].ok, "凭据未配置应报红");
        assert!(!report.items[1].ok, "端点未配置应报红");
        assert!(report.items[2].ok, "临时目录应可写");
        assert!(report.items[3].ok);
        assert!(report.items[4].ok, "工具通道应实测通过");
        // 体检结果落盘证据（铁律 5）应真实生成。
        let evidence = std::fs::read_dir(dir.join(".sd-agent").join("evidence"))
            .map(|d| d.filter_map(|e| e.ok()).count())
            .unwrap_or(0);
        assert!(evidence >= 1, "doctor evidence file must be written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn doctor_items_have_chinese_fields() {
        let (report, dir) = run_scrubbed("fields").await;
        for item in &report.items {
            assert!(!item.title.is_empty(), "{}: title 必须非空", item.name);
            assert!(!item.detail.is_empty(), "{}: detail 必须非空", item.name);
            assert!(!item.hint.is_empty(), "{}: hint 必须非空", item.name);
            if item.ok {
                assert!(
                    item.fix.is_empty(),
                    "{}: 通过时 fix 应为空（不吓唬用户）",
                    item.name
                );
            } else {
                assert!(
                    !item.fix.is_empty(),
                    "{}: 不通过时 fix 必须给修法",
                    item.name
                );
            }
        }
        let text = report.render();
        assert!(text.contains("状态："), "render 必须有状态行");
        assert!(text.contains("说明："), "render 必须有大白话说明");
        assert!(text.contains("修法："), "失败项必须渲染修法行");
        assert!(text.contains("总体结论："), "render 必须有总体结论");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
