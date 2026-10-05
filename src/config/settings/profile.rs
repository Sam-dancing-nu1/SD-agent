//! 模型配置单元：ModelProfileCfg 及其归一化、安全视图与相关常量。

use serde::{Deserialize, Serialize};

/// 旧单套格式迁移后的配置名。
pub const DEFAULT_PROFILE_LABEL: &str = "default";
/// 环境变量临时合成配置的名字。
pub const ENV_PROFILE_LABEL: &str = "env";
/// reasoning_effort 合法取值表（与 async-openai 枚举一一对应）。
pub const REASONING_EFFORTS: [&str; 7] =
    ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

/// UI 提示文案素材：思维链开关与思考强度的映射说明（thinking.type）。
pub const THINKING_HINT: &str = "思考强度=none 时思维链关闭（thinking.type=disabled），其余档位一律开启（官方暂不支持自定义推理投入档位）；开启时推理过程在回答前逐段流式显示。";

/// UI 提示文案素材：工具轮思考开关的行为说明（thinking_on_tools）。
pub const THINKING_ON_TOOLS_HINT: &str = "关闭后，带工具的请求强制关思维链（thinking.type=disabled）。官方说明：思考开启时调用工具不稳定，tool_calls 可能混入思维链内容；此项为纯设置裁决，逐请求生效，不自动判断。";

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
    pub(super) fn normalized(mut self) -> Self {
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

/// 思考强度归一：trim + 小写；不在合法表内一律归 "medium"。
fn normalize_effort(value: &str) -> String {
    let t = value.trim().to_ascii_lowercase();
    if REASONING_EFFORTS.contains(&t.as_str()) {
        t
    } else {
        ModelProfileCfg::DEFAULT_REASONING_EFFORT.to_string()
    }
}
