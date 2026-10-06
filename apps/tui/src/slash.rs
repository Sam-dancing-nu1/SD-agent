//! 斜杠命令系统：命令表、前缀过滤、帮助文案。
//!
//! 形态参考对话式 TUI：输入框敲 `/` 弹出命令列表，继续输入按前缀过滤，
//! 回车执行选中命令。本文件只定义命令与文案，执行动作在 app 层
//! （需要应用状态），键位在 keymap.rs。

/// 一条斜杠命令（名称 + 中文说明）。
pub struct SlashCommand {
    pub name: &'static str,
    pub desc: &'static str,
}

/// 命令表（顺序即弹出列表顺序）。
pub const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "/help",
        desc: "显示命令与键位清单",
    },
    SlashCommand {
        name: "/doctor",
        desc: "六项体检：环境 / 配置 / 端点，失败项给修法",
    },
    SlashCommand {
        name: "/model",
        desc: "查看当前模型配置并切换",
    },
    SlashCommand {
        name: "/settings",
        desc: "模型配置：列表 / 新增 / 切换 / 改思考强度 / 删除",
    },
    SlashCommand {
        name: "/effort",
        desc: "改思考强度（/effort high 等，7 档）",
    },
    SlashCommand {
        name: "/new",
        desc: "新建会话（当前会话自动存档）",
    },
    SlashCommand {
        name: "/conversations",
        desc: "历史会话列表，选序号切换",
    },
    SlashCommand {
        name: "/stats",
        desc: "Token 消耗统计与热力图",
    },
    SlashCommand {
        name: "/trace",
        desc: "最近轨迹事件流（诊断）",
    },
    SlashCommand {
        name: "/retry",
        desc: "重试上一条任务",
    },
    SlashCommand {
        name: "/clear",
        desc: "清空对话区",
    },
    SlashCommand {
        name: "/version",
        desc: "版本与构建信息",
    },
];

/// 思考强度固定档位（7 档，核心 reasoning_effort 透传同档）。
pub const EFFORTS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

/// 按前缀过滤命令（`typed` 含开头的 `/`）。
pub fn filter(typed: &str) -> Vec<&'static SlashCommand> {
    COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(typed))
        .collect()
}

/// 帮助文案（/help 与 ? 键位帮助的命令部分）。
pub fn help_text() -> String {
    let mut out = String::from("命令：\n");
    for c in COMMANDS {
        out.push_str(&format!("  {:<14} {}\n", c.name, c.desc));
    }
    out
}
