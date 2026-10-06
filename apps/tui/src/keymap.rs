//! 热键文件：全部键位的唯一定义出处。
//!
//! 改键位只改这里；帮助浮层（? / F1）与 `--help` 的键位文案同源于 `HELP` 表，
//! 禁止在其他文件硬编码按键分支。鼠标动作登记在 `MOUSE_HELP` 表（HELP 旁），
//! 鼠标命中判定的几何口径注释在各 Action 与 app/mouse.rs。

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
    /// 打开选中的历史会话（弹新终端跑 `--worker --session`，hub 原地不动；
    /// Enter（列表聚焦）/ 双击 / o 同路）。
    OpenSelected,
    /// 新建会话（当前会话先存档，再重置）。
    NewSession,
    // ── 模型选择浮层（模型条点击 / M 呼出） ──
    /// 打开模型选择浮层（列出 profiles）。
    ModelPopupOpen,
    /// 浮层内移动高亮（实时预览 profile_sel）。
    ModelPopupPrev,
    ModelPopupNext,
    /// 确认切换（set_active + save 落盘 + reload_profiles）。
    ModelPopupConfirm,
    /// 取消（关浮层，profile_sel 复位到激活项）。
    ModelPopupCancel,
    // ── 思考强度滑条（←→ 改档；鼠标点击/拖动走 app/mouse.rs） ──
    EffortDec,
    EffortInc,
    // ── 配置表单（"+" 呼出；鼠标点击字段/按钮走 app/mouse.rs） ──
    /// 打开新增配置表单。
    FormOpen,
    /// Tab / Shift+Tab 字段轮转。
    FormFieldNext,
    FormFieldPrev,
    /// 保存（Ctrl+S；校验过则 upsert+set_active+save+reload，错则显示 error）。
    FormSubmit,
    /// 取消（Esc）。
    FormCancel,
    /// Effort 字段 ←→ 改档。
    FormEffortLeft,
    FormEffortRight,
    /// 表单文本编辑（作用于聚焦字段）。
    FormInsert(char),
    FormBackspace,
    FormDelete,
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
    /// 斜杠命令浮层（输入 / 弹出；此键显式呼出并插入 '/' 前缀）。
    Slash,
    // ── 斜杠命令浮层（浮层开着时 ↑↓/PgUp/PgDn/Enter/Esc 归浮层） ──
    /// 选中上/下（clamp 不环绕；PgUp/PgDn 同 ↑↓，滚轮同效）。
    SlashPrev,
    SlashNext,
    /// 执行选中命令（浮层开着时 Enter；非命令前缀走整行提交）。
    SlashConfirm,
    /// 关闭浮层并清空输入（Esc）。
    SlashCancel,
    // ── 审批弹窗 ──
    ApproveOnce,
    ApproveAlways,
    Deny,
}

/// 键位表：（按键, 说明）。帮助浮层与 --help 同源渲染。
pub const HELP: &[(&str, &str)] = &[
    (
        "Enter",
        "斜杠浮层=执行选中命令 · hub：输入框=新开任务 / 历史=打开会话（均弹新终端）· worker：发送",
    ),
    ("Alt+Enter", "输入框换行"),
    (
        "Esc",
        "斜杠浮层=关闭并清空 · worker 运行中=中断 · 空闲=关闭浮层",
    ),
    ("q", "退出（需输入框为空）"),
    ("Ctrl+C ×2", "确认退出（1.2 秒内连按两次）"),
    ("?", "本帮助"),
    (
        "/",
        "斜杠命令浮层（↑↓/PgUp/PgDn 选中 · Enter 执行选中 · Esc 关闭清空 · 点击按行执行）",
    ),
    (
        "↑ ↓",
        "斜杠浮层=切选中 · hub：列表选择 / worker：输入历史回翻与滚动",
    ),
    ("PgUp / PgDn", "对话区滚动（滚轮同效）；斜杠浮层开着=同 ↑↓"),
    ("Tab", "焦点轮转（表单内=字段轮转）"),
    ("R", "失败后重试上一条任务"),
    (
        "M",
        "模型选择浮层（输入框为空时；↑↓ 选择 · Enter 确认 · Esc 取消）",
    ),
    ("← →", "思考强度改档（输入框为空时；表单 Effort 字段同效）"),
    ("表单", "Tab 换字段 · Ctrl+S 保存 · Esc 取消"),
    ("审批弹窗", "Y 放行 · A 本次会话全放行 · N 拒绝"),
];

/// 鼠标动作表（与 HELP 并列登记；判定几何口径见 app/mouse.rs）。
pub const MOUSE_HELP: &[(&str, &str)] = &[
    (
        "单击",
        "命中区聚焦/选中（输入框聚焦 · 历史按行选中 · 模型条开浮层 · 滑条定档 · + 开表单 · 斜杠浮层按行执行）",
    ),
    (
        "双击",
        "历史会话项 = 弹新终端打开会话（--worker --session，hub 原地不动）",
    ),
    (
        "滚轮",
        "按鼠标所在区滚动（历史/对话 = 距底部偏移；斜杠/模型浮层 = 移动选中）",
    ),
    ("拖动", "思考强度滑条按住拖动实时改档"),
    (
        "拖选",
        "对话区/历史按住左键拖选文本，松开经 OSC 52 写剪贴板",
    ),
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

/// 斜杠命令浮层键位（浮层开着时 ↑↓/PgUp/PgDn/Enter/Esc 归浮层；
/// 其余键返回 None 交回输入编辑，过滤继续）。
pub fn map_slash(key: KeyEvent) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::Up | KeyCode::PageUp, _) => Some(Action::SlashPrev),
        (KeyCode::Down | KeyCode::PageDown, _) => Some(Action::SlashNext),
        (KeyCode::Enter, KeyModifiers::ALT) => None, // Alt+Enter 换行交回编辑
        (KeyCode::Enter, _) => Some(Action::SlashConfirm),
        (KeyCode::Esc, _) => Some(Action::SlashCancel),
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

/// 配置表单键位（表单打开期间只认这些；Esc=取消、Ctrl+S/Enter=保存）。
pub fn map_form(key: KeyEvent) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => Some(Action::FormCancel),
        (KeyCode::Char('s'), KeyModifiers::CONTROL)
        | (KeyCode::Char('S'), KeyModifiers::CONTROL) => Some(Action::FormSubmit),
        (KeyCode::Enter, _) => Some(Action::FormSubmit),
        (KeyCode::Tab, _) => Some(Action::FormFieldNext),
        (KeyCode::BackTab, _) => Some(Action::FormFieldPrev),
        (KeyCode::Left, _) => Some(Action::FormEffortLeft),
        (KeyCode::Right, _) => Some(Action::FormEffortRight),
        (KeyCode::Backspace, _) => Some(Action::FormBackspace),
        (KeyCode::Delete, _) => Some(Action::FormDelete),
        (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            Some(Action::FormInsert(ch))
        }
        _ => None,
    }
}

/// 模型选择浮层键位（浮层打开期间只认这些；↑↓ 实时预览，Enter 落盘确认）。
pub fn map_model_popup(key: KeyEvent) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::Up | KeyCode::Left, _) => Some(Action::ModelPopupPrev),
        (KeyCode::Down | KeyCode::Right, _) => Some(Action::ModelPopupNext),
        (KeyCode::Enter, _) => Some(Action::ModelPopupConfirm),
        (KeyCode::Esc, _) => Some(Action::ModelPopupCancel),
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

/// 键位帮助文案（/help 的键位部分与 ? 浮层同源，含鼠标动作表）。
pub fn help_text() -> String {
    let mut out = String::new();
    for (k, d) in HELP {
        out.push_str(&format!("  {:<14} {d}\n", k));
    }
    out.push_str("  ── 鼠标 ──\n");
    for (k, d) in MOUSE_HELP {
        out.push_str(&format!("  {:<14} {d}\n", k));
    }
    out
}

/// hub 模式键位。
///
/// `list_focus`：焦点是否在历史列表——Enter 语义按焦点分叉：
/// 列表聚焦 = 打开选中会话（Action::OpenSelected），输入框聚焦 = 新开任务
///（Action::Launch）。←→/M 是滑条与模型浮层的键盘入口（输入框为空时生效，
/// 避免抢编辑光标）。
pub fn map_hub(key: KeyEvent, input_empty: bool, list_focus: bool) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::Enter, _) if list_focus => Some(Action::OpenSelected),
        (KeyCode::Enter, _) => Some(Action::Launch),
        (KeyCode::Up, _) if input_empty => Some(Action::SelectPrev),
        (KeyCode::Down, _) if input_empty => Some(Action::SelectNext),
        (KeyCode::PageUp, _) => Some(Action::ScrollUp),
        (KeyCode::PageDown, _) => Some(Action::ScrollDown),
        (KeyCode::Char('o') | KeyCode::Char('O'), _) if input_empty => Some(Action::OpenSelected),
        (KeyCode::Char('n') | KeyCode::Char('N'), _) if input_empty => Some(Action::NewSession),
        (KeyCode::Char('m') | KeyCode::Char('M'), _) if input_empty => Some(Action::ModelPopupOpen),
        (KeyCode::Left, _) if input_empty => Some(Action::EffortDec),
        (KeyCode::Right, _) if input_empty => Some(Action::EffortInc),
        (KeyCode::Char('q'), _) if input_empty => Some(Action::Quit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn hub_enter_splits_by_focus() {
        assert_eq!(
            map_hub(key(KeyCode::Enter, KeyModifiers::NONE), true, true),
            Some(Action::OpenSelected)
        );
        assert_eq!(
            map_hub(key(KeyCode::Enter, KeyModifiers::NONE), true, false),
            Some(Action::Launch)
        );
    }

    #[test]
    fn form_keys_take_over() {
        assert_eq!(
            map_form(key(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Some(Action::FormSubmit)
        );
        assert_eq!(
            map_form(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Action::FormCancel)
        );
        assert_eq!(
            map_form(key(KeyCode::Char('x'), KeyModifiers::NONE)),
            Some(Action::FormInsert('x'))
        );
    }

    #[test]
    fn model_popup_keys() {
        assert_eq!(
            map_model_popup(key(KeyCode::Up, KeyModifiers::NONE)),
            Some(Action::ModelPopupPrev)
        );
        assert_eq!(
            map_model_popup(key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Action::ModelPopupConfirm)
        );
    }

    #[test]
    fn slash_popup_keys_take_over_navigation_only() {
        // ↑↓/PgUp/PgDn/Enter/Esc 归浮层。
        assert_eq!(
            map_slash(key(KeyCode::Up, KeyModifiers::NONE)),
            Some(Action::SlashPrev)
        );
        assert_eq!(
            map_slash(key(KeyCode::PageUp, KeyModifiers::NONE)),
            Some(Action::SlashPrev)
        );
        assert_eq!(
            map_slash(key(KeyCode::Down, KeyModifiers::NONE)),
            Some(Action::SlashNext)
        );
        assert_eq!(
            map_slash(key(KeyCode::PageDown, KeyModifiers::NONE)),
            Some(Action::SlashNext)
        );
        assert_eq!(
            map_slash(key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Action::SlashConfirm)
        );
        assert_eq!(
            map_slash(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Action::SlashCancel)
        );
        // Alt+Enter 换行与普通编辑键交回输入编辑（过滤继续）。
        assert_eq!(map_slash(key(KeyCode::Enter, KeyModifiers::ALT)), None);
        assert_eq!(map_slash(key(KeyCode::Char('h'), KeyModifiers::NONE)), None);
        assert_eq!(map_slash(key(KeyCode::Backspace, KeyModifiers::NONE)), None);
    }

    #[test]
    fn help_text_has_mouse_section() {
        let t = help_text();
        assert!(t.contains("鼠标"));
        assert!(t.contains("OSC 52"));
    }
}
