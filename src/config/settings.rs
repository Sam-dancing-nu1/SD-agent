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

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 旧单套格式迁移后的配置名。
pub const DEFAULT_PROFILE_LABEL: &str = "default";
/// 环境变量临时合成配置的名字。
pub const ENV_PROFILE_LABEL: &str = "env";
/// reasoning_effort 合法取值表（与 async-openai 枚举一一对应）。
pub const REASONING_EFFORTS: [&str; 7] =
    ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

/// UI 提示文案素材：思维链开关与思考强度的映射说明（thinking.type）。
pub const THINKING_HINT: &str =
    "思考强度=none 时思维链关闭（thinking.type=disabled），其余档位一律开启（官方暂不支持自定义推理投入档位）；开启时推理过程在回答前逐段流式显示。";

/// UI 提示文案素材：工具轮思考开关的行为说明（thinking_on_tools）。
pub const THINKING_ON_TOOLS_HINT: &str =
    "关闭后，带工具的请求强制关思维链（thinking.type=disabled）。官方说明：思考开启时调用工具不稳定，tool_calls 可能混入思维链内容；此项为纯设置裁决，逐请求生效，不自动判断。";

/// 一套模型配置（多模型切换的基本单元）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelProfileCfg {
    /// 配置名，如 "MiMo-payg"；profiles 内唯一（upsert 按 label 覆盖）。
    pub label: String,
    /// OpenAI 兼容端点（含 /v1）。
    pub base_url: String,
    /// 模型名。
    pub model: String,
    /// API 密钥：明文存用户目录配置文件，任何展示处打码。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// 思考强度："none|minimal|low|medium|high|xhigh|max"，默认 "medium"。
    pub reasoning_effort: String,
}

impl Default for ModelProfileCfg {
    fn default() -> Self {
        Self {
            label: DEFAULT_PROFILE_LABEL.to_string(),
            base_url: String::new(),
            model: String::new(),
            api_key: None,
            reasoning_effort: ModelProfileCfg::DEFAULT_REASONING_EFFORT.to_string(),
        }
    }
}

impl ModelProfileCfg {
    pub const DEFAULT_REASONING_EFFORT: &'static str = "medium";

    /// 思维链开关映射（MiMo 官方口径）："none" → 关（thinking.type=disabled），
    /// 其余档位 → 开（enabled）。官方暂不支持自定义推理投入档位，只分关/开。
    pub fn thinking_enabled(&self) -> bool {
        !self.reasoning_effort.trim().eq_ignore_ascii_case("none")
    }

    /// 密钥是否在位（只看有无，不看内容）。
    pub fn has_api_key(&self) -> bool {
        self.api_key
            .as_deref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false)
    }

    /// 缺失项清单：空 base_url / 空 model / 缺 key → ["base_url", …]。
    pub fn missing_fields(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.base_url.trim().is_empty() {
            missing.push("base_url");
        }
        if self.model.trim().is_empty() {
            missing.push("model");
        }
        if !self.has_api_key() {
            missing.push("api_key");
        }
        missing
    }

    /// 字符串 → 枚举映射（none→None … max→Max，非法/未知→Medium）。
    pub fn reasoning_effort_enum(&self) -> async_openai::types::chat::ReasoningEffort {
        use async_openai::types::chat::ReasoningEffort as E;
        match self.reasoning_effort.trim().to_ascii_lowercase().as_str() {
            "none" => E::None,
            "minimal" => E::Minimal,
            "low" => E::Low,
            "medium" => E::Medium,
            "high" => E::High,
            "xhigh" => E::Xhigh,
            "max" => E::Max,
            _ => E::Medium,
        }
    }

    /// 载入归一：思考强度小写化 + 非法归 "medium"，空密钥归 None，空 label 归 "default"。
    fn normalized(mut self) -> Self {
        self.label = self.label.trim().to_string();
        if self.label.is_empty() {
            self.label = DEFAULT_PROFILE_LABEL.to_string();
        }
        self.reasoning_effort = normalize_effort(&self.reasoning_effort);
        if !self.has_api_key() {
            self.api_key = None;
        }
        self
    }
}

/// 单套模型配置的安全视图（无密钥内容）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelProfileView {
    pub label: String,
    pub base_url: String,
    pub model: String,
    pub has_api_key: bool,
    pub reasoning_effort: String,
}

/// 模型配置全量安全视图：多配置列表 + 当前启用项 + 当前项镜像字段
/// （镜像字段方便只关心当前配置的展示处，内容与 active() 一致）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SettingsView {
    /// 多套模型配置（无密钥内容）。
    pub profiles: Vec<ModelProfileView>,
    /// 当前启用配置的 label。
    pub active_profile: String,
    // —— 当前启用配置的镜像字段 ——
    pub base_url: String,
    pub model: String,
    pub has_api_key: bool,
    /// "settings" | "env" | "missing"。
    pub api_key_source: String,
    pub reasoning_effort: String,
    pub max_rounds: u32,
    /// 工具轮思考开关（true=按用户设置；false=带工具的请求强制关思维链）。
    pub thinking_on_tools: bool,
    /// 配置文件绝对路径（展示用，不含密钥）。
    pub settings_path: String,
}

/// 模型配置链（持久化形态）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// 多套模型配置。
    pub profiles: Vec<ModelProfileCfg>,
    /// 当前激活的 label。
    pub active_profile: String,
    /// 最大工具调用轮数熔断，钳制 [1, 200]，默认 20。
    pub max_rounds: u32,
    /// 工具轮思考开关（默认 true = 按用户设置的 reasoning_effort 走）。
    /// false 时带工具的请求强制 thinking.type=disabled：MiMo 官方 FAQ
    /// 指出思考开启时调工具不稳定（tool_calls 可能混进 reasoning_content），
    /// 官方建议调工具场景关思考。纯设置项裁决，不做智能开关。
    #[serde(default = "default_thinking_on_tools")]
    pub thinking_on_tools: bool,
}

/// thinking_on_tools 缺省值（旧配置文件无此字段 → true，行为与旧版一致）。
fn default_thinking_on_tools() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            profiles: Vec::new(),
            active_profile: String::new(),
            max_rounds: Settings::DEFAULT_MAX_ROUNDS,
            thinking_on_tools: true,
        }
    }
}

impl Settings {
    pub const DEFAULT_MAX_ROUNDS: u32 = 20;
    pub const MAX_ROUNDS_CAP: u32 = 200;

    /// 配置文件绝对路径：`<用户目录>/.sd-agent/settings.json`。
    /// Windows 用 USERPROFILE，其他平台回落 HOME，两者都缺失时回落系统临时目录。
    pub fn settings_path() -> PathBuf {
        home_dir().join(".sd-agent").join("settings.json")
    }

    /// 读取配置链：文件优先（含旧单套格式迁移），profiles 为空时回落
    /// 环境变量临时合成，再缺留默认。
    pub fn load() -> Settings {
        Self::load_from(&Self::settings_path())
    }

    /// 读取指定配置文件（测试与自定义路径用），回落逻辑与 load 相同。
    pub fn load_from(path: &Path) -> Settings {
        let raw: Option<RawSettings> = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<RawSettings>(&text).ok());

        let mut s = Settings::default();
        let mut file_rounds: Option<u32> = None;
        let mut file_thinking_on_tools: Option<bool> = None;
        if let Some(raw) = raw {
            file_rounds = raw.max_rounds;
            file_thinking_on_tools = raw.thinking_on_tools;
            if let Some(profiles) = raw.profiles {
                // 新格式：多套配置。
                s.profiles = profiles.into_iter().map(|p| p.normalized()).collect();
                s.active_profile = raw.active_profile.unwrap_or_default().trim().to_string();
            } else if raw.base_url.is_some()
                || raw.model.is_some()
                || raw.api_key.is_some()
                || raw.reasoning_effort.is_some()
            {
                // 旧单套格式 → profiles[0]（label="default"）。
                let profile = ModelProfileCfg {
                    label: DEFAULT_PROFILE_LABEL.to_string(),
                    base_url: raw.base_url.unwrap_or_default(),
                    model: raw.model.unwrap_or_default(),
                    api_key: raw.api_key,
                    reasoning_effort: raw
                        .reasoning_effort
                        .unwrap_or_else(|| ModelProfileCfg::DEFAULT_REASONING_EFFORT.to_string()),
                };
                s.profiles = vec![profile.normalized()];
                s.active_profile = DEFAULT_PROFILE_LABEL.to_string();
            }
        }

        // active_profile 校验：指不到任何配置就落到第一套（不留悬空指针）。
        if !s.profiles.iter().any(|p| p.label == s.active_profile) {
            s.active_profile = s
                .profiles
                .first()
                .map(|p| p.label.clone())
                .unwrap_or_default();
        }

        // max_rounds：文件 → 环境变量 → 默认，统一钳制。
        s.max_rounds = file_rounds
            .or_else(|| {
                std::env::var("SD_AGENT_MAX_ROUNDS")
                    .ok()
                    .and_then(|v| v.trim().parse::<u32>().ok())
            })
            .unwrap_or(Settings::DEFAULT_MAX_ROUNDS)
            .clamp(1, Settings::MAX_ROUNDS_CAP);

        // 工具轮思考开关：文件 → 默认 true（旧格式无此字段 = 行为不变）。
        s.thinking_on_tools = file_thinking_on_tools.unwrap_or(true);

        // 环境变量回落：profiles 为空时临时合成 label="env" 的配置。
        if s.profiles.is_empty() {
            let env_profile = ModelProfileCfg {
                label: ENV_PROFILE_LABEL.to_string(),
                base_url: env_str("SD_AGENT_BASE_URL"),
                model: env_str("SD_AGENT_MODEL"),
                api_key: env_opt("SD_AGENT_API_KEY"),
                ..ModelProfileCfg::default()
            };
            if !env_profile.base_url.is_empty()
                || !env_profile.model.is_empty()
                || env_profile.api_key.is_some()
            {
                s.active_profile = ENV_PROFILE_LABEL.to_string();
                s.profiles.push(env_profile);
            }
        }
        s
    }

    /// 仅从环境变量装配（不读文件）：恒有一个 label="env" 的配置，
    /// 供 model.rs 的 from_env 兼容入口使用。
    pub fn from_env() -> Settings {
        Settings {
            profiles: vec![ModelProfileCfg {
                label: ENV_PROFILE_LABEL.to_string(),
                base_url: env_str("SD_AGENT_BASE_URL"),
                model: env_str("SD_AGENT_MODEL"),
                api_key: env_opt("SD_AGENT_API_KEY"),
                ..ModelProfileCfg::default()
            }],
            active_profile: ENV_PROFILE_LABEL.to_string(),
            max_rounds: std::env::var("SD_AGENT_MAX_ROUNDS")
                .ok()
                .and_then(|v| v.trim().parse::<u32>().ok())
                .unwrap_or(Settings::DEFAULT_MAX_ROUNDS)
                .clamp(1, Settings::MAX_ROUNDS_CAP),
            thinking_on_tools: true,
        }
    }

    /// 创建目录并把当前配置写回用户目录配置文件（JSON），返回写入路径。
    pub fn save(&self) -> std::io::Result<PathBuf> {
        let path = Self::settings_path();
        self.save_to(&path)?;
        Ok(path)
    }

    /// 写到指定路径（测试与自定义路径用）。
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)
    }

    /// 当前启用配置（active_profile 指向的那套；指不到返回 None）。
    pub fn active(&self) -> Option<&ModelProfileCfg> {
        self.profiles
            .iter()
            .find(|p| p.label == self.active_profile)
    }

    /// 切换当前启用配置（纯指针赋值；label 不在 profiles 中时 active()
    /// 返回 None，missing_fields() 会报 "active_profile"）。
    pub fn set_active(&mut self, label: &str) {
        self.active_profile = label.to_string();
    }

    /// 按 label 插入或覆盖（label 唯一性由本方法保证）；
    /// 若 active_profile 为空则顺延指向新配置。
    pub fn upsert_profile(&mut self, p: ModelProfileCfg) {
        let p = p.normalized();
        match self.profiles.iter_mut().find(|x| x.label == p.label) {
            Some(slot) => *slot = p,
            None => {
                self.profiles.push(p);
            }
        }
        if self.active_profile.trim().is_empty() {
            self.active_profile = self
                .profiles
                .last()
                .map(|x| x.label.clone())
                .unwrap_or_default();
        }
    }

    /// 删除一套配置；删掉的若是当前启用项，自动切到剩余第一套（没有就清空）。
    pub fn remove_profile(&mut self, label: &str) {
        self.profiles.retain(|p| p.label != label);
        if !self.profiles.iter().any(|p| p.label == self.active_profile) {
            self.active_profile = self
                .profiles
                .first()
                .map(|p| p.label.clone())
                .unwrap_or_default();
        }
    }

    /// 给 UI 的安全视图（无密钥内容，只有 has/source 布尔位）。
    pub fn view(&self) -> SettingsView {
        let profiles = self
            .profiles
            .iter()
            .map(|p| ModelProfileView {
                label: p.label.clone(),
                base_url: p.base_url.clone(),
                model: p.model.clone(),
                has_api_key: p.has_api_key(),
                reasoning_effort: p.reasoning_effort.clone(),
            })
            .collect();
        let active = self.active();
        let has_api_key = active.map(|p| p.has_api_key()).unwrap_or(false);
        // 来源口径：密钥来自环境变量合成的 "env" 配置 → "env"；
        // 否则（文件载入 / GUI 填写）→ "settings"；没有密钥 → "missing"。
        let api_key_source = if !has_api_key {
            "missing"
        } else if self.active_profile == ENV_PROFILE_LABEL {
            "env"
        } else {
            "settings"
        };
        SettingsView {
            profiles,
            active_profile: self.active_profile.clone(),
            base_url: active.map(|p| p.base_url.clone()).unwrap_or_default(),
            model: active.map(|p| p.model.clone()).unwrap_or_default(),
            has_api_key,
            api_key_source: api_key_source.to_string(),
            reasoning_effort: active
                .map(|p| p.reasoning_effort.clone())
                .unwrap_or_else(|| ModelProfileCfg::DEFAULT_REASONING_EFFORT.to_string()),
            max_rounds: self.max_rounds,
            thinking_on_tools: self.thinking_on_tools,
            settings_path: crate::sys::normalize_display(&Self::settings_path()),
        }
    }

    /// 缺失项清单（按当前启用配置）：整链没配置 → ["profiles"]；
    /// 悬空指针 → ["active_profile"]；否则报启用配置缺的具体字段。
    pub fn missing_fields(&self) -> Vec<&'static str> {
        if self.profiles.is_empty() {
            return vec!["profiles"];
        }
        match self.active() {
            Some(p) => p.missing_fields(),
            None => vec!["active_profile"],
        }
    }

    /// 当前启用配置的思考强度映射（无启用配置 → Medium）。
    pub fn reasoning_effort_enum(&self) -> async_openai::types::chat::ReasoningEffort {
        self.active()
            .map(|p| p.reasoning_effort_enum())
            .unwrap_or(async_openai::types::chat::ReasoningEffort::Medium)
    }

    /// 当前启用配置的思维链开关（无启用配置 → true）。
    pub fn thinking_enabled(&self) -> bool {
        self.active().map(|p| p.thinking_enabled()).unwrap_or(true)
    }

    /// 本轮请求的思维链裁决（MiMo thinking.type）：
    /// effort 非 none 且（无工具或工具轮开关开）→ 思考。
    pub fn thinking_enabled_for(&self, has_tools: bool) -> bool {
        self.thinking_enabled() && !(has_tools && !self.thinking_on_tools)
    }
}

/// 载入中间形态：新旧两种格式共用一个宽容解析壳（字段全可缺）。
#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    profiles: Option<Vec<ModelProfileCfg>>,
    active_profile: Option<String>,
    max_rounds: Option<u32>,
    thinking_on_tools: Option<bool>,
    // —— 旧单套格式字段（迁移用）——
    base_url: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
    reasoning_effort: Option<String>,
}

/// 思考强度归一：trim + 小写；不在合法表内一律归 "medium"。
fn normalize_effort(value: &str) -> String {
    let t = value.trim().to_ascii_lowercase();
    if REASONING_EFFORTS.contains(&t.as_str()) {
        t
    } else {
        ModelProfileCfg::DEFAULT_REASONING_EFFORT.to_string()
    }
}

/// 用户目录：Windows 用 USERPROFILE，其他回落 HOME，再缺回落系统临时目录。
fn home_dir() -> PathBuf {
    if let Ok(p) = std::env::var("USERPROFILE") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(p) = std::env::var("HOME") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    std::env::temp_dir()
}

fn env_str(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::env_lock;

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
