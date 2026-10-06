//! 主题：配色与文本样式的唯一出处（深色简洁技术风）。
//!
//! 改配色只改这里；其余渲染文件一律通过本模块取样式，禁止散落硬编码颜色。
//! 配色取向：深底、柔白正文、琥珀金强调、暗灰弱化（干预透明），不用高饱和色。

use ratatui::style::{Color, Modifier, Style};

// ── 调色板 ────────────────────────────────────────────────
/// 强调色：琥珀金（#d8a25a）——Logo、选中、关键数字。
pub const ACCENT: Color = Color::Rgb(216, 162, 90);
/// 正文：柔白。
pub const TEXT: Color = Color::Rgb(208, 212, 222);
/// 弱化（干预透明 / 灰字注记）：暗灰。
pub const DIM: Color = Color::Rgb(112, 118, 130);
/// 次级文本（标签、说明）。
pub const MUTED: Color = Color::Rgb(150, 158, 172);
/// 成功。
pub const OK: Color = Color::Rgb(126, 186, 120);
/// 失败。
pub const ERR: Color = Color::Rgb(224, 108, 108);
/// 信息（工具调用、链接）。
pub const INFO: Color = Color::Rgb(126, 166, 222);
/// 边框常态。
pub const BORDER: Color = Color::Rgb(62, 68, 82);
/// 边框聚焦。
pub const BORDER_FOCUS: Color = ACCENT;
/// 选中行背景。
pub const SELECT_BG: Color = Color::Rgb(42, 46, 58);
/// 浮层背景。
pub const POPUP_BG: Color = Color::Rgb(26, 29, 38);

// ── 样式快捷取用 ──────────────────────────────────────────
pub fn accent() -> Style {
    Style::default().fg(ACCENT)
}

pub fn text() -> Style {
    Style::default().fg(TEXT)
}

pub fn dim() -> Style {
    Style::default().fg(DIM)
}

pub fn muted() -> Style {
    Style::default().fg(MUTED)
}

pub fn ok() -> Style {
    Style::default().fg(OK)
}

pub fn err() -> Style {
    Style::default().fg(ERR)
}

pub fn info() -> Style {
    Style::default().fg(INFO)
}

/// 标题样式（块标题）。
pub fn title() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

/// 边框常态样式。
pub fn border() -> Style {
    Style::default().fg(BORDER)
}

/// 边框聚焦样式。
pub fn border_focus() -> Style {
    Style::default().fg(BORDER_FOCUS)
}

/// 选中行（列表/历史）。
pub fn selected() -> Style {
    Style::default().fg(ACCENT).bg(SELECT_BG)
}

/// 干预透明样式：系统提示注入/纠偏/记忆召回等干预条目，灰字弱化显示在消息流里。
pub fn intervention() -> Style {
    Style::default().fg(DIM).add_modifier(Modifier::ITALIC)
}

/// 热力图五档字符（空日 → 满），与 heat() 样式配套。
/// 档 0（无消耗日）用暗灰 '░' 占位——不留空格（用户反馈空格看着像乱码）。
pub const HEAT_CHARS: [char; 5] = ['░', '▒', '▓', '█', '█'];

/// 滑条：已选段 / 未选段（分段块状滑条，形态取自 tui-slider 类组件）。
pub fn slider_on() -> Style {
    Style::default().fg(ACCENT)
}

pub fn slider_off() -> Style {
    Style::default().fg(Color::Rgb(72, 78, 92))
}

/// 滚动条：滑块 / 轨道。
pub fn scroll_thumb() -> Style {
    Style::default().fg(Color::Rgb(104, 112, 128))
}

pub fn scroll_track() -> Style {
    Style::default().fg(Color::Rgb(48, 54, 66))
}

/// 表单字段标签 / 输入值。
pub fn field_label() -> Style {
    Style::default().fg(MUTED)
}

pub fn field_value() -> Style {
    Style::default().fg(TEXT)
}

/// 热力图格子样式：按档位 0..4 取色（档越高越亮，统一琥珀系）。
pub fn heat(level: usize) -> Style {
    let fg = match level {
        0 => Color::Rgb(56, 60, 72),
        1 => Color::Rgb(120, 100, 72),
        2 => Color::Rgb(166, 132, 84),
        3 => Color::Rgb(216, 162, 90),
        _ => Color::Rgb(240, 196, 130),
    };
    Style::default().fg(fg)
}
