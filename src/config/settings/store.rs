//! 持久化配置链 Settings：载入（新旧格式迁移 + 环境变量回落）/ 保存 / 安全视图。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::profile::{DEFAULT_PROFILE_LABEL, ENV_PROFILE_LABEL, ModelProfileCfg, ModelProfileView};

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
