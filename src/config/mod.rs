//! 三类档案门面（环境 / 模型 / 任务）+ 认知纪律域数据位（白名单表 / 阈值表，
//! 纯数据，P3 取数）。config 位于依赖图最底，零业务依赖。

pub mod env;
pub mod resource;
pub mod secret;
pub mod settings;

pub use env::{EnvProfile, ShellKind};
pub use resource::{ResourceAllocation, ResourceLedger, SideEffectRecord};
pub use secret::Secret;
pub use settings::{
    ModelProfileCfg, ModelProfileView, Settings, SettingsView, THINKING_HINT,
    THINKING_ON_TOOLS_HINT,
};

/// 认知纪律域数据位：工具白名单 + 风险阈值（纯数据，决策零成本优先，
/// 硬约束 10/11——路由与放行一律查表裁决，不引入模型推理）。
pub struct DisciplineTables {
    /// 允许模型驱动的工具名白名单。
    pub tool_whitelist: &'static [&'static str],
    /// 风险阈值：超过该字节数的工具输出进摘要通道（P0 只登记，不实施截断策略）。
    pub output_digest_threshold_bytes: usize,
}

impl DisciplineTables {
    /// P0 白名单 = 四件工具；阈值登记为 4 KiB（P2 起按实测调整）。
    pub const fn p0() -> Self {
        Self {
            tool_whitelist: &["read", "write", "edit", "bash"],
            output_digest_threshold_bytes: 4096,
        }
    }

    pub fn allows(&self, tool: &str) -> bool {
        self.tool_whitelist.contains(&tool)
    }
}

/// 模型档案（环境变量取值的元数据；凭据本体只在 model.rs 消费）。
#[derive(Debug, Clone)]
pub struct ModelProfile {
    /// 端点地址（非凭据）。
    pub base_url: String,
    /// 模型名（非凭据）。
    pub model: String,
    /// 最大工具调用轮数熔断（SD_AGENT_MAX_ROUNDS，默认 20）。
    /// 钳制到 [1, 200]：0 轮=静默空转，超大值=烧钱熔断形同虚设。
    pub max_rounds: u32,
}

impl ModelProfile {
    pub const DEFAULT_MAX_ROUNDS: u32 = 20;
    pub const MAX_ROUNDS_CAP: u32 = 200;

    /// 从环境变量组装；缺项返回 None 并由调用方（doctor）报缺失名单。
    pub fn from_env() -> Result<Self, Vec<&'static str>> {
        let mut missing = Vec::new();
        let base_url = std::env::var("SD_AGENT_BASE_URL").unwrap_or_default();
        if base_url.is_empty() {
            missing.push("SD_AGENT_BASE_URL");
        }
        let model = std::env::var("SD_AGENT_MODEL").unwrap_or_default();
        if model.is_empty() {
            missing.push("SD_AGENT_MODEL");
        }
        let max_rounds = std::env::var("SD_AGENT_MAX_ROUNDS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(Self::DEFAULT_MAX_ROUNDS)
            .clamp(1, Self::MAX_ROUNDS_CAP);
        if missing.is_empty() {
            Ok(Self {
                base_url,
                model,
                max_rounds,
            })
        } else {
            Err(missing)
        }
    }
}

/// 任务档案：一次 run 的元数据（P0 最小面）。
#[derive(Debug, Clone)]
pub struct TaskProfile {
    pub task: String,
    /// 是否免交互自动批准（CLI `--yes` / 壳层无人值守模式）。
    pub auto_approve: bool,
}
