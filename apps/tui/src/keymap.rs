//! 热键文件：全部键位的唯一定义出处。
//!
//! 改键位只改这里；帮助浮层（? / F1）与 `--help` 的键位文案同源于 `HELP` 表，
//! 禁止在其他文件硬编码按键分支。鼠标事件不在此表（属交互映射，见 app 层）。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// UI 动作：键位 → 动作 → app 层执行。新增交互先加这里再接线。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // ── 全局 ──
    /// 退出（q，输入框为空时）。
    Quit,
    /// 确认退出（Ctrl+C：连按两次 1.2s 窗，防误触）。
    QuitConfirm,
    /// 帮助浮层（键位表）。
    Help,
    /// 重绘一帧。
    Redraw,
    /// 焦点轮转。
    FocusNext,
    // ── 输入框编辑（键位唯一出处，主循环不硬编码按键分支） ──
    /// 插入字符。
    Insert(char),
    Backspace,
    Delete,
    MoveLeft,
    MoveRight,
    // ── hub ──
    /// 输入框提交：新开会话并弹新终端跑 worker。
    Launch,
    /// 历史列表上/下选择。
    SelectPrev,
    SelectNext,
    /// 打开选中的历史会话（查看/接管）。
    OpenSelected,
    /// 新建会话（当前会话先存档，再重置）。
    NewSession,
    // ── worker ──
    /// 发送输入框内容（开 run）。
    Send,
    /// 输入框换行。
    Newline,
    /// 中断当前 run（Esc，运行中）。
    Interrupt,
    /// 重试上一条任务（失败收尾后）。
    Retry,
    /// 滚动对话区（看更早内容 = 距底部偏移增大）。
    ScrollUp,
    ScrollDown,
    /// 输入框历史输入回翻（worker）。
    InputPrev,
    /// 斜杠命令列表（输入框以 / 开头时自动弹出，此键为显式呼出）。
    Slash,
    // ── 审批弹窗 ──
    ApproveOnce,
    ApproveAlways,
    Deny,
}

/// 键位表：（按键, 说明）。帮助浮层与 --help 同源渲染。
pub const HELP: &[(&str, &str)] = &[
    ("Enter", "hub：新开会话并弹终端执行 / worker：发送"),
    ("Alt+Enter", "输入框换行"),
    ("Esc", "运行中：中断当前任务 / 空闲：关闭浮层"),
    ("q", "退出（需输入框为空）"),
    ("Ctrl+C ×2", "确认退出（1.2 秒内连按两次）"),
    ("?", "本帮助"),
    ("/", "斜杠命令列表"),
    ("↑ ↓", "hub：列表选择 / worker：输入历史回翻与滚动"),
    ("PgUp / PgDn", "对话区滚动（滚轮同效）"),
    ("Tab", "焦点轮转"),
    ("R", "失败后重试上一条任务"),
    ("审批弹窗", "Y 放行 · A 本次会话全放行 · N 拒绝"),
];

/// 全局键位（任何模式先生效）。返回 None 表示交回模式层。
pub fn map_global(key: KeyEvent) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(Action::QuitConfirm),
        (KeyCode::Char('?') | KeyCode::F(1), _) => Some(Action::Help),
        (KeyCode::Char('l'), KeyModifiers::CONTROL) => Some(Action::Redraw),
        (KeyCode::Tab, _) => Some(Action::FocusNext),
        _ => None,
    }
}

/// 输入框编辑键位（文本编辑分支的唯一出处；主循环不得硬编码按键分支）。
pub fn map_edit(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char(ch)
            if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
        {
            Some(Action::Insert(ch))
        }
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Delete => Some(Action::Delete),
        KeyCode::Left => Some(Action::MoveLeft),
        KeyCode::Right => Some(Action::MoveRight),
        _ => None,
    }
}

/// 审批弹窗键位（弹窗期间只认这些）。
pub fn map_approval(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => Some(Action::ApproveOnce),
        KeyCode::Char('a') | KeyCode::Char('A') => Some(Action::ApproveAlways),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(Action::Deny),
        _ => None,
    }
}

/// worker 模式键位（输入框聚焦时的非文本键）。
///
/// `input_empty`：输入框是否为空——q/Esc 等"离开"键只在空输入时生效，
/// 避免打字打到一半被截胡（可退出性与可输入性分开保证）。
pub fn map_worker(key: KeyEvent, input_empty: bool) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::Enter, KeyModifiers::ALT) => Some(Action::Newline),
        (KeyCode::Enter, _) => Some(Action::Send),
        (KeyCode::Esc, _) => Some(Action::Interrupt),
        (KeyCode::Char('r') | KeyCode::Char('R'), _) if input_empty => Some(Action::Retry),
        (KeyCode::Char('/'), _) if input_empty => Some(Action::Slash),
        (KeyCode::Up, _) if input_empty => Some(Action::InputPrev),
        (KeyCode::Up, _) => None,
        (KeyCode::Down, _) if input_empty => Some(Action::ScrollDown),
        (KeyCode::PageUp, _) => Some(Action::ScrollUp),
        (KeyCode::PageDown, _) => Some(Action::ScrollDown),
        (KeyCode::Char('q'), _) if input_empty => Some(Action::Quit),
        _ => None,
    }
}

/// 键位帮助文案（/help 的键位部分与 ? 浮层同源）。
pub fn help_text() -> String {
    let mut out = String::new();
    for (k, d) in HELP {
        out.push_str(&format!("  {:<14} {d}\n", k));
    }
    out
}

/// hub 模式键位。
pub fn map_hub(key: KeyEvent, input_empty: bool) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::Enter, _) => Some(Action::Launch),
        (KeyCode::Up, _) if input_empty => Some(Action::SelectPrev),
        (KeyCode::Down, _) if input_empty => Some(Action::SelectNext),
        (KeyCode::PageUp, _) => Some(Action::ScrollUp),
        (KeyCode::PageDown, _) => Some(Action::ScrollDown),
        (KeyCode::Char('o') | KeyCode::Char('O'), _) if input_empty => Some(Action::OpenSelected),
        (KeyCode::Char('n') | KeyCode::Char('N'), _) if input_empty => Some(Action::NewSession),
        (KeyCode::Char('q'), _) if input_empty => Some(Action::Quit),
        _ => None,
    }
}
