//! 斜杠命令系统：命令表、前缀过滤、精确匹配、帮助文案。
//!
//! 形态参考对话式 TUI（OpenCode / Pi）：输入框敲 `/` 弹出命令列表，
//! 继续输入按前缀过滤，回车执行选中命令。本文件只定义命令与文案，
//! 执行动作在 crate::app::App::exec_command（需要应用状态）。

/// 一条斜杠命令（名称 + 中文说明）。
pub struct SlashCommand {
    pub name: &'static str,
    pub desc: &'static str,
}

/// 命令表（顺序即弹出列表顺序）。
pub const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "/help",
        desc: "显示命令清单",
    },
    SlashCommand {
        name: "/settings",
        desc: "模型配置：列表 / 新增 / 切换 / 改思考强度 / 删除",
    },
    SlashCommand {
        name: "/model",
        desc: "查看当前模型配置并快速切换",
    },
    SlashCommand {
        name: "/effort",
        desc: "改当前模型的思考强度（/effort high 等）",
    },
    SlashCommand {
        name: "/new",
        desc: "新建会话（当前会话自动存档）",
    },
    SlashCommand {
        name: "/conversations",
        desc: "历史会话列表，选择序号切换",
    },
    SlashCommand {
        name: "/runs",
        desc: "任务列表与状态",
    },
    SlashCommand {
        name: "/trace",
        desc: "最近轨迹事件流",
    },
    SlashCommand {
        name: "/doctor",
        desc: "运行六项体检，结果以卡片显示在对话区",
    },
    SlashCommand {
        name: "/clear",
        desc: "清空对话区",
    },
];

/// 思考强度取值（固定档位）。
pub const EFFORTS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

/// 思考强度行为的如实标注（MiMo 官方文档实锤）：
/// 无 = 关思考；其余档位 = 开思考（当前模型未实现分档调节）。
/// 下拉、卡片、帮助文案统一引用本常量，避免各处口径不一。
pub const EFFORT_NOTE: &str = "无=关思考；其余档位=开思考（当前模型未实现分档调节）";

/// 思考强度中文说明（如实标注 MiMo 实际行为，不做夸大）。
pub fn effort_desc(effort: &str) -> &'static str {
    match effort {
        "none" => "关思考",
        e if EFFORTS.contains(&e) => "开思考（未实现分档调节）",
        _ => "未知档位",
    }
}

/// 输入是否处于“斜杠命令输入态”：以 / 开头且还没敲空格/换行进入参数位。
/// 满足时弹出命令列表；Esc 关闭（app 层记录 dismissed）。
pub fn is_slash_typing(input: &str) -> bool {
    input.starts_with('/') && !input.contains(' ') && !input.contains('\n')
}

/// 按前缀过滤命令（弹出列表内容；"/" 列全部）。
pub fn filter(input: &str) -> Vec<&'static SlashCommand> {
    COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(input))
        .collect()
}

/// 精确匹配命令名（执行用；"/help x" 取首个 token）。
pub fn lookup(name: &str) -> Option<&'static SlashCommand> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// 取输入的首个 token（斜杠命令名）。
pub fn first_token(input: &str) -> &str {
    input.split_whitespace().next().unwrap_or("")
}

/// /help 输出（命令清单 + 通用操作提示）。
pub fn help_lines() -> Vec<String> {
    let mut lines: Vec<String> = vec!["命令清单（输入 / 呼出命令列表，回车执行）：".to_string()];
    for cmd in COMMANDS {
        lines.push(format!("  {:<14} {}", cmd.name, cmd.desc));
    }
    lines.push("  /history        /conversations 的别名".to_string());
    lines.push(String::new());
    lines.push("通用操作：".to_string());
    lines.push("  Enter 发送任务 · Shift+Enter 换行 · ↑↓ 历史/选择".to_string());
    lines.push("  滚轮/PgUp PgDn 翻页 · End 回到底部 · Ctrl+T 展开/收起思考块".to_string());
    lines.push("  Esc 关闭列表/取消引导 · q（输入框为空时）退出 · Ctrl+C 连按两次退出".to_string());
    lines.push(String::new());
    lines.push(format!("思考强度说明：{EFFORT_NOTE}"));
    lines
}
