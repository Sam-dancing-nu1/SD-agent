//! 模型配置链（多模型配置 + GUI 配置闭环）：端点 / 模型名 / API 密钥 /
//! 思考强度 / 最大轮数，持久化于用户目录，支持多套配置与一键切换。
//!
//! 存储口径（2026-10-03 拍板）：
//! - 配置文件位于【用户目录】`<用户目录>/.sd-agent/settings.json`，
//!   **不在本仓库内，本仓库永不包含该文件**。
//! - API 密钥在该用户目录文件中明文保存（文件权限保持系统默认），
//!   供桌面端 / TUI / CLI 三路复用。**凭据入库禁令仍然有效**：
//!   凭据禁止写入本仓库任何文件（含示例代码与文档）；配置文件在仓库外，
//!   "存盘到用户目录"不等于"入库"。
//! - 任何展示处必须打码：对外只给 SettingsView / ModelProfileView
//!   （has_api_key 布尔位），明文唯一消费链是 Settings → model.rs 请求头。
//!
//! 结构（模型切换是硬需求）：
//! - `profiles`：多套模型配置，label 唯一（upsert_profile 按 label 覆盖）；
//! - `active_profile`：当前启用配置的 label；
//! - `max_rounds`：最大工具调用轮数熔断，钳制 [1, 200]，默认 20。
//!
//! 兼容迁移：旧单套格式（base_url / model / api_key / reasoning_effort
//! 在顶层）载入时自动转为 profiles[0]（label="default"）。
//! 环境变量回落：profiles 为空时，用 SD_AGENT_BASE_URL / SD_AGENT_MODEL /
//! SD_AGENT_API_KEY / SD_AGENT_MAX_ROUNDS 临时合成一个 label="env" 的配置
//! （max_rounds 全程支持 SD_AGENT_MAX_ROUNDS 回落）。
//! reasoning_effort 取值："none|minimal|low|medium|high|xhigh|max"，
//! 默认 "medium"，非法值归一为 "medium"。

mod profile;
mod store;

pub use profile::{
    DEFAULT_PROFILE_LABEL, ENV_PROFILE_LABEL, ModelProfileCfg, ModelProfileView, REASONING_EFFORTS,
    THINKING_HINT, THINKING_ON_TOOLS_HINT,
};
pub use store::{Settings, SettingsView};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::env_lock;
    use std::path::PathBuf;

    /// 测试用临时"用户目录"（USERPROFILE 指向它即隔离真实配置文件）。
    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sd-settings-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp home");
        dir
    }

    fn profile(label: &str, key: Option<&str>) -> ModelProfileCfg {
        ModelProfileCfg {
            label: label.to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            model: format!("model-{label}"),
            api_key: key.map(|k| k.to_string()),
            reasoning_effort: "high".to_string(),
        }
    }

    #[test]
    fn multi_profile_roundtrip_and_view_hides_key() {
        let _guard = env_lock();
        let home = temp_home("roundtrip");
        unsafe {
            std::env::set_var("USERPROFILE", &home);
        }

        let mut s = Settings {
            profiles: vec![
                profile("default", Some("sk-secret-a")),
                profile("mimo", Some("sk-secret-b")),
            ],
            active_profile: "mimo".to_string(),
            max_rounds: 33,
            thinking_on_tools: false,
        };
        s.upsert_profile(profile("mimo", Some("sk-secret-b"))); // 按 label 覆盖，不重复
        assert_eq!(s.profiles.len(), 2);

        let path = s.save().expect("save");
        assert!(path.to_string_lossy().contains(".sd-agent"));
        let loaded = Settings::load();
        assert_eq!(loaded, s);

        // 安全视图：有/无密钥位在，内容永不出现。
        let view = loaded.view();
        assert_eq!(view.active_profile, "mimo");
        assert_eq!(view.profiles.len(), 2);
        assert!(view.profiles.iter().all(|p| p.has_api_key));
        assert!(view.has_api_key);
        assert_eq!(view.api_key_source, "settings");
        assert_eq!(view.max_rounds, 33);
        let dump = format!("{view:?}");
        assert!(
            !dump.contains("sk-secret"),
            "view must never carry key material"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn legacy_single_profile_migrates_to_default() {
        let _guard = env_lock();
        let home = temp_home("legacy");
        unsafe {
            std::env::set_var("USERPROFILE", &home);
        }
        let path = Settings::settings_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &path,
            r#"{"base_url":"https://old.invalid/v1","model":"old-model","api_key":"sk-old","reasoning_effort":"xhigh","max_rounds":7}"#,
        )
        .expect("write legacy");

        let s = Settings::load();
        assert_eq!(s.profiles.len(), 1, "旧单套格式应迁移为一套配置");
        assert_eq!(s.profiles[0].label, DEFAULT_PROFILE_LABEL);
        assert_eq!(s.profiles[0].base_url, "https://old.invalid/v1");
        assert_eq!(s.profiles[0].model, "old-model");
        assert_eq!(s.profiles[0].api_key.as_deref(), Some("sk-old"));
        assert_eq!(s.profiles[0].reasoning_effort, "xhigh");
        assert_eq!(s.active_profile, DEFAULT_PROFILE_LABEL);
        assert_eq!(s.max_rounds, 7);

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn active_switch_upsert_remove() {
        let mut s = Settings::default();
        s.upsert_profile(profile("a", Some("k-a")));
        s.upsert_profile(profile("b", Some("k-b")));
        assert_eq!(s.active_profile, "a", "首套应自动成为启用项");

        s.set_active("b");
        assert_eq!(s.active().map(|p| p.label.as_str()), Some("b"));

        s.upsert_profile(ModelProfileCfg {
            model: "model-b2".to_string(),
            ..profile("b", Some("k-b2"))
        });
        assert_eq!(s.profiles.len(), 2, "upsert 按 label 覆盖");
        assert_eq!(s.active().map(|p| p.model.as_str()), Some("model-b2"));

        s.remove_profile("b");
        assert_eq!(s.active_profile, "a", "删掉启用项应自动切到剩余第一套");
        s.remove_profile("a");
        assert!(s.active().is_none());
        assert_eq!(s.missing_fields(), vec!["profiles"]);
    }

    #[test]
    fn env_fallback_synthesizes_env_profile() {
        let _guard = env_lock();
        let home = temp_home("envfallback");
        let saved = (
            std::env::var("USERPROFILE").ok(),
            std::env::var("SD_AGENT_BASE_URL").ok(),
            std::env::var("SD_AGENT_MODEL").ok(),
            std::env::var("SD_AGENT_API_KEY").ok(),
            std::env::var("SD_AGENT_MAX_ROUNDS").ok(),
        );
        unsafe {
            std::env::set_var("USERPROFILE", &home);
            std::env::set_var("SD_AGENT_BASE_URL", "https://env.invalid/v1");
            std::env::set_var("SD_AGENT_MODEL", "env-model");
            std::env::set_var("SD_AGENT_API_KEY", "sk-env");
            std::env::set_var("SD_AGENT_MAX_ROUNDS", "9");
        }

        let s = Settings::load();
        assert_eq!(s.profiles.len(), 1);
        assert_eq!(s.profiles[0].label, ENV_PROFILE_LABEL);
        assert_eq!(s.profiles[0].base_url, "https://env.invalid/v1");
        assert_eq!(s.profiles[0].model, "env-model");
        assert_eq!(s.active_profile, ENV_PROFILE_LABEL);
        assert_eq!(s.max_rounds, 9);
        let view = s.view();
        assert!(view.has_api_key);
        assert_eq!(view.api_key_source, "env");

        // max_rounds 钳制：越界值一律进 [1,200]。
        unsafe {
            std::env::set_var("SD_AGENT_MAX_ROUNDS", "99999");
        }
        assert_eq!(Settings::load().max_rounds, Settings::MAX_ROUNDS_CAP);

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
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn reasoning_effort_mapping() {
        let cases = [
            ("none", async_openai::types::chat::ReasoningEffort::None),
            (
                "minimal",
                async_openai::types::chat::ReasoningEffort::Minimal,
            ),
            ("low", async_openai::types::chat::ReasoningEffort::Low),
            ("medium", async_openai::types::chat::ReasoningEffort::Medium),
            ("high", async_openai::types::chat::ReasoningEffort::High),
            ("xhigh", async_openai::types::chat::ReasoningEffort::Xhigh),
            ("max", async_openai::types::chat::ReasoningEffort::Max),
        ];
        for (text, want) in cases {
            let p = ModelProfileCfg {
                reasoning_effort: text.to_string(),
                ..ModelProfileCfg::default()
            };
            assert_eq!(p.reasoning_effort_enum(), want, "mapping for {text}");
        }
        // 非法值 → Medium。
        let p = ModelProfileCfg {
            reasoning_effort: "超纲".to_string(),
            ..ModelProfileCfg::default()
        };
        assert_eq!(
            p.reasoning_effort_enum(),
            async_openai::types::chat::ReasoningEffort::Medium
        );
        assert_eq!(
            p.reasoning_effort_enum(),
            Settings::default().reasoning_effort_enum(),
            "无启用配置时 Settings 侧也回落 Medium"
        );
    }

    #[test]
    fn missing_fields_reports_active_profile() {
        let mut s = Settings::default();
        assert_eq!(s.missing_fields(), vec!["profiles"]);
        s.upsert_profile(ModelProfileCfg {
            base_url: String::new(),
            model: String::new(),
            api_key: None,
            ..profile("empty", None)
        });
        assert_eq!(s.missing_fields(), vec!["base_url", "model", "api_key"]);
        s.set_active("不存在的配置");
        assert_eq!(s.missing_fields(), vec!["active_profile"]);
    }

    #[test]
    fn thinking_mapping_and_tools_switch() {
        // 官方口径：none → 思维链关，其余档位 → 开。
        for effort in ["minimal", "low", "medium", "high", "xhigh", "max"] {
            let p = ModelProfileCfg {
                reasoning_effort: effort.to_string(),
                ..ModelProfileCfg::default()
            };
            assert!(p.thinking_enabled(), "{effort} 应开思维链");
        }
        let off = ModelProfileCfg {
            reasoning_effort: "none".to_string(),
            ..ModelProfileCfg::default()
        };
        assert!(!off.thinking_enabled());
        // 非法值归一 medium → 开。
        let weird = ModelProfileCfg {
            reasoning_effort: "超纲".to_string(),
            ..ModelProfileCfg::default()
        };
        assert!(weird.thinking_enabled());

        // 工具轮思考开关：false 时带工具的请求强制关，无工具不受影响。
        let mut s = Settings::default();
        s.upsert_profile(profile("a", Some("k")));
        assert!(s.thinking_enabled());
        assert!(s.thinking_enabled_for(true), "默认开关 true：按用户设置");
        s.thinking_on_tools = false;
        assert!(s.thinking_enabled_for(false), "无工具不受开关影响");
        assert!(!s.thinking_enabled_for(true), "带工具且开关关 → 强制关");
    }

    #[test]
    fn thinking_on_tools_defaults_true_and_roundtrips() {
        let _guard = env_lock();
        let home = temp_home("thinkingtools");
        unsafe {
            std::env::set_var("USERPROFILE", &home);
        }
        // 旧格式（无 thinking_on_tools 字段）→ true，行为不变。
        let path = Settings::settings_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &path,
            r#"{"base_url":"https://old.invalid/v1","model":"m","api_key":"sk-x"}"#,
        )
        .expect("write legacy");
        assert!(Settings::load().thinking_on_tools);

        // 显式 false 载入生效并可回写 roundtrip。
        std::fs::write(
            &path,
            r#"{"profiles":[{"label":"p","base_url":"https://x.invalid/v1","model":"m","api_key":"sk-x"}],"active_profile":"p","thinking_on_tools":false}"#,
        )
        .expect("write new");
        let s = Settings::load();
        assert!(!s.thinking_on_tools);
        assert_eq!(s.view().thinking_on_tools, false);
        s.save().expect("save");
        assert!(!Settings::load().thinking_on_tools);

        let _ = std::fs::remove_dir_all(&home);
    }
}
