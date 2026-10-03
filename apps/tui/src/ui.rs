//! 渲染层：ratatui 绘制对话式主界面（状态栏 / 对话流 / 快捷提示 / 输入框）
//! + 斜杠命令弹出列表 + 思考强度选择列表 + 工具审批弹窗。
//!
//! 纯函数式渲染：只读 App 状态画帧，不改状态、不做业务计算。
//! 深色技术风：主色金棕 rgb(216,162,90)，深底、细边框、等宽节奏。

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{App, ConvoItem, ToolState};
use crate::slash;

/// 主色：金棕（接近 #d8a25a）。
const GOLD: Color = Color::Rgb(216, 162, 90);
const DIM: Color = Color::DarkGray;
const OK_GREEN: Color = Color::Green;
const FAIL_RED: Color = Color::LightRed;
const WARN_YELLOW: Color = Color::Yellow;

// ---------------- 展示辅助 ----------------

/// Debug 文本化展示值：兼容 String / PathBuf / 枚举 / 数字 / Option<T>。
/// 处理 Some(...) 包裹、引号、转义；None → 空串。密钥内容绝不走本函数展示。
pub fn show_val(v: &impl std::fmt::Debug) -> String {
    let raw = format!("{v:?}");
    let inner = raw
        .strip_prefix("Some(")
        .and_then(|x| x.strip_suffix(')'))
        .unwrap_or(&raw);
    if inner == "None" {
        return String::new();
    }
    if inner.len() >= 2 && inner.starts_with('"') && inner.ends_with('"') {
        unescape(&inner[1..inner.len() - 1])
    } else {
        inner.to_string()
    }
}

/// 反转义 Debug 字符串体（\\ \" \n \t \r）。
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 按字符截断（展示用；char 边界安全）。
pub fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// 宽字符（CJK 等）按 2 格估。
fn char_width(c: char) -> usize {
    let cp = c as u32;
    let wide = (0x1100..=0x115F).contains(&cp)
        || (0x2E80..=0xA4CF).contains(&cp)
        || (0xAC00..=0xD7A3).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFE30..=0xFE4F).contains(&cp)
        || (0xFF00..=0xFF60).contains(&cp)
        || (0xFFE0..=0xFFE6).contains(&cp)
        || (0x20000..=0x3FFFD).contains(&cp);
    if wide { 2 } else { 1 }
}

/// 按显示宽度折行（CJK 宽字符占 2 格）。
pub fn wrap_text(s: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for raw in s.split('\n') {
        let mut cur = String::new();
        let mut w = 0usize;
        for c in raw.chars() {
            let cw = char_width(c);
            if w + cw > width && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                w = 0;
            }
            cur.push(c);
            w += cw;
        }
        lines.push(cur);
    }
    lines
}

// ---------------- 整帧 ----------------

/// 画一整帧：状态栏 + 对话流 + 快捷提示 + 输入框 +（必要时）弹出层。
pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let input_h = (app.input_lines() + 2).clamp(3, 7) as u16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
            Constraint::Length(input_h),
        ])
        .split(area);

    draw_status(f, app, chunks[0]);
    draw_convo(f, app, chunks[1]);
    draw_hints(f, app, chunks[2]);
    draw_input(f, app, chunks[3]);

    // 弹出层（互斥：引导选择列表 / 斜杠命令列表）。
    if app.pick_open() {
        draw_pick_popup(f, app, chunks[2]);
    } else if app.slash_open() {
        draw_slash_popup(f, app, chunks[2]);
    }

    if !app.approval_queue.is_empty() {
        draw_approval_popup(f, app, area);
    }
}

/// 顶部细状态栏：会话标题 / 模型 / 思考强度 / 工作目录 / 运行状态。
fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let cwd = crate::ui::trunc(&app.root.display().to_string(), 28);
    let left = format!(
        " sd-agent · 会话: {} · 模型: {} · 思考: {} · 目录: {} · 状态: {} ",
        app.session_title(),
        app.model_label(),
        app.effort_label(),
        cwd,
        app.run_state(),
    );
    let allow = if app.allow_all.load(std::sync::atomic::Ordering::SeqCst) {
        Span::styled(" 全放行 ", Style::default().fg(WARN_YELLOW))
    } else {
        Span::raw("")
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(left, Style::default().fg(GOLD)),
            allow,
        ]))
        .style(Style::default().bg(Color::Rgb(38, 30, 18))),
        area,
    );
}

/// 对话流（滚动视图；scroll_follow = 跟底，上翻后固定阅读位置）。
fn draw_convo(f: &mut Frame, app: &App, area: Rect) {
    let width = area.width.saturating_sub(1) as usize;
    let lines = convo_lines(app, width);
    let visible = area.height as usize;
    let total = lines.len();
    let bottom_start = total.saturating_sub(visible);
    let start = if app.scroll_follow {
        bottom_start
    } else {
        app.view_start.get().min(bottom_start)
    };
    // 回填视口度量（按键/滚轮翻页换算行数用；draw 保持只读语义）。
    app.view_start.set(start);
    app.view_total.set(total);
    app.view_lines.set(visible);
    let end = (start + visible).min(total);
    let window: Vec<Line> = lines[start..end].to_vec();
    f.render_widget(Paragraph::new(window), area);
}

/// 思考块渲染（暗色可折叠）：展开=标头+暗色正文，收起=单行摘要。
/// 复用于流式条目与固化条目，形态一致（主流 agent 终端体验）。
fn push_thinking_block(
    lines: &mut Vec<Line<'static>>,
    reasoning: &str,
    expanded: bool,
    thinking: bool,
    body_w: usize,
) {
    if reasoning.is_empty() && !thinking {
        return;
    }
    if expanded || thinking {
        let header = if thinking {
            "✦ 思考中…"
        } else {
            "✦ 思考（Ctrl+T 收起）"
        };
        lines.push(Line::from(Span::styled(
            header,
            Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
        )));
        for l in wrap_text(reasoning, body_w) {
            lines.push(Line::from(Span::styled(
                format!("│ {l}"),
                // 暗色/灰色块：思考正文统一 DIM。
                Style::default().fg(DIM),
            )));
        }
    } else {
        let chars = reasoning.chars().count();
        lines.push(Line::from(Span::styled(
            format!("▸ 思考（{chars} 字 · Ctrl+T 展开）"),
            Style::default().fg(DIM),
        )));
    }
}

/// 助手正文（流式中带 ▌ 光标尾巴；已收尾无）。
fn push_assistant_text(lines: &mut Vec<Line<'static>>, text: &str, body_w: usize, streaming: bool) {
    let mut wrapped = wrap_text(text, body_w);
    if streaming {
        if let Some(last) = wrapped.last_mut() {
            last.push('▌');
        }
    }
    for l in wrapped {
        lines.push(Line::from(Span::styled(format!("  {l}"), Style::default())));
    }
}

/// 对话条目 → 渲染行。
fn convo_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let body_w = width.saturating_sub(3).max(8);

    if app.convo.is_empty() {
        lines.push(Line::from(Span::styled(
            "（对话为空：输入任务开始，或 /help 查看命令）",
            Style::default().fg(DIM),
        )));
        return lines;
    }

    for item in &app.convo {
        match item {
            ConvoItem::User { text } => {
                lines.push(Line::from(Span::styled(
                    "❯ 你",
                    Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
                )));
                push_body(&mut lines, text, body_w);
                lines.push(Line::from(""));
            }
            ConvoItem::Assistant {
                text,
                reasoning,
                expanded,
            } => {
                // 思考块（暗色可折叠）在前，正文在后。
                push_thinking_block(&mut lines, reasoning, *expanded, false, body_w);
                lines.push(Line::from(Span::styled(
                    "✦ 助手",
                    Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
                )));
                push_assistant_text(&mut lines, text, body_w, false);
                lines.push(Line::from(""));
            }
            ConvoItem::Streaming {
                reasoning,
                text,
                thinking,
                tool_preview,
                expanded,
                ..
            } => {
                // 流式条目：思考流（暗色）→ 工具参数预览（原地更新）→ 正文流。
                push_thinking_block(&mut lines, reasoning, *expanded, *thinking, body_w);
                if let Some((name, args)) = tool_preview {
                    lines.push(Line::from(Span::styled(
                        format!("◆ 工具 {name} {}", trunc(args, 120)),
                        Style::default().fg(WARN_YELLOW),
                    )));
                }
                if !text.is_empty() || !*thinking {
                    lines.push(Line::from(Span::styled(
                        "✦ 助手",
                        Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
                    )));
                    // 流式中正文带 ▌ 光标，逐字流出感。
                    push_assistant_text(&mut lines, text, body_w, true);
                }
                lines.push(Line::from(""));
            }
            ConvoItem::ToolCard {
                id,
                tool,
                args,
                state,
            } => {
                lines.push(Line::from(vec![
                    Span::styled("◆ 工具调用 ", Style::default().fg(WARN_YELLOW)),
                    Span::styled(
                        tool.clone(),
                        Style::default()
                            .fg(WARN_YELLOW)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!(" [{}]", trunc(id, 8)), Style::default().fg(DIM)),
                ]));
                for l in wrap_text(&format!("参数 {args}"), body_w) {
                    lines.push(Line::from(Span::styled(
                        format!("  {l}"),
                        Style::default().fg(DIM),
                    )));
                }
                match state {
                    ToolState::Pending => {
                        lines.push(Line::from(Span::styled(
                            "  ⌛ 等待结果…",
                            Style::default().fg(WARN_YELLOW),
                        )));
                    }
                    ToolState::Done { ok, result } => {
                        let style = if *ok {
                            Style::default().fg(OK_GREEN)
                        } else {
                            Style::default().fg(FAIL_RED)
                        };
                        lines.push(Line::from(Span::styled(
                            format!("  {} {}", if *ok { "✅ 完成" } else { "❌ 失败" }, result),
                            style,
                        )));
                    }
                    ToolState::Denied { reason } => {
                        lines.push(Line::from(Span::styled(
                            format!("  ⛔ 被拒：{reason}"),
                            Style::default().fg(FAIL_RED),
                        )));
                    }
                }
                lines.push(Line::from(""));
            }
            ConvoItem::Note(text) => {
                push_body(&mut lines, &format!("· {text}"), body_w);
            }
            ConvoItem::Error(text) => {
                for l in wrap_text(&format!("✕ {text}"), body_w) {
                    lines.push(Line::from(Span::styled(l, Style::default().fg(FAIL_RED))));
                }
            }
            ConvoItem::Card { title, lines: body } => {
                lines.push(Line::from(Span::styled(
                    format!("┌─ {title}"),
                    Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
                )));
                for l in body {
                    for w in wrap_text(l, body_w) {
                        lines.push(Line::from(Span::styled(
                            format!("│ {w}"),
                            Style::default().fg(Color::White),
                        )));
                    }
                }
                lines.push(Line::from(Span::styled("└─", Style::default().fg(GOLD))));
                lines.push(Line::from(""));
            }
            ConvoItem::Doctor(report) => {
                lines.push(Line::from(Span::styled(
                    "┌─ 🩺 doctor 体检报告",
                    Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
                )));
                for item in &report.items {
                    let mark = if item.ok { "✅" } else { "❌" };
                    let style = if item.ok {
                        Style::default().fg(OK_GREEN)
                    } else {
                        Style::default().fg(FAIL_RED)
                    };
                    lines.push(Line::from(Span::styled(
                        format!("│ {mark} {}", show_val(&item.title)),
                        style,
                    )));
                    for w in wrap_text(&format!("状态：{}", show_val(&item.detail)), body_w) {
                        lines.push(Line::from(Span::styled(
                            format!("│   {w}"),
                            Style::default().fg(DIM),
                        )));
                    }
                    let hint = show_val(&item.hint);
                    if !hint.is_empty() {
                        for w in wrap_text(&format!("说明：{hint}"), body_w) {
                            lines.push(Line::from(Span::styled(
                                format!("│   {w}"),
                                Style::default().fg(DIM),
                            )));
                        }
                    }
                    if !item.ok {
                        let fix = show_val(&item.fix);
                        if !fix.is_empty() {
                            for w in wrap_text(&format!("修法：{fix}"), body_w) {
                                lines.push(Line::from(Span::styled(
                                    format!("│   {w}"),
                                    Style::default().fg(WARN_YELLOW),
                                )));
                            }
                        }
                    }
                }
                let (sum, style) = if report.all_green() {
                    ("✅ 全部通过", Style::default().fg(OK_GREEN))
                } else {
                    ("❌ 有未通过项（见上方修法）", Style::default().fg(FAIL_RED))
                };
                lines.push(Line::from(Span::styled(format!("│ {sum}"), style)));
                lines.push(Line::from(Span::styled("└─", Style::default().fg(GOLD))));
                lines.push(Line::from(""));
            }
        }
    }
    lines
}

/// 正文折行进渲染行。
fn push_body(lines: &mut Vec<Line<'static>>, text: &str, width: usize) {
    for l in wrap_text(text, width) {
        lines.push(Line::from(Span::styled(format!("  {l}"), Style::default())));
    }
}

/// 底部快捷提示行（右侧带瞬时提示；键位与实际行为一一对应）。
fn draw_hints(f: &mut Frame, app: &App, area: Rect) {
    let hints = if !matches!(app.flow, crate::app::Flow::None) {
        "Esc=取消引导 · Enter=确认"
    } else {
        "Enter=发送 · Shift+Enter=换行 · /=命令 · 滚轮/PgUp PgDn 翻页 · End=回到底部 · ↑↓=历史 · Ctrl+T=展开/收起思考 · q=退出（输入为空） · Ctrl+C×2=退出"
    };
    let status = app.status.clone();
    let spans = if status.is_empty() {
        vec![Span::styled(format!(" {hints} "), Style::default().fg(DIM))]
    } else {
        vec![
            Span::styled(format!(" {hints} "), Style::default().fg(DIM)),
            Span::styled(
                format!("  ⚙ {} ", trunc(&status, 60)),
                Style::default().fg(GOLD),
            ),
        ]
    };
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// 底部输入框（多行；引导态标题随流程变化；密钥步打码）。
fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(GOLD))
        .title(Line::from(Span::styled(
            app.input_title(),
            Style::default().fg(GOLD),
        )));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let display = if app.input_masked() {
        "●".repeat(app.input.chars().count())
    } else {
        app.input.clone()
    };
    let mut rows: Vec<String> = display.split('\n').map(|s| s.to_string()).collect();
    let visible = inner.height as usize;
    if rows.len() > visible {
        let start = rows.len() - visible;
        rows.drain(..start);
    }
    if let Some(last) = rows.last_mut() {
        last.push_str("▌");
    }
    let lines: Vec<Line> = rows.into_iter().map(Line::from).collect();
    f.render_widget(Paragraph::new(lines), inner);
}

/// 斜杠命令弹出列表（输入框上方；输 / 弹出，前缀过滤）。
fn draw_slash_popup(f: &mut Frame, app: &App, hints_area: Rect) {
    let items = slash::filter(&app.input);
    let h = (items.len() + 2).min(12).max(3) as u16;
    let w = (hints_area.width as f32 * 0.62) as u16;
    let x = hints_area.x + 2;
    let y = hints_area.y.saturating_sub(h);
    let rect = Rect::new(x, y, w.max(30).min(hints_area.width), h);
    f.render_widget(Clear, rect);

    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(GOLD))
        .title(Line::from(Span::styled(
            " 斜杠命令（↑↓ 选择 · Enter 执行 · Tab 补全 · Esc 关闭） ",
            Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
        )));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let sel = app.slash_sel.min(items.len().saturating_sub(1));
    let rows: Vec<Line> = items
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let style = if i == sel {
                Style::default()
                    .fg(GOLD)
                    .bg(Color::Rgb(64, 48, 26))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::from(Span::styled(format!(" {} — {}", cmd.name, cmd.desc), style))
        })
        .collect();
    f.render_widget(Paragraph::new(rows), inner);
}

/// 思考强度选择列表（引导态弹出；↑↓ 选择，Enter 确认）。
fn draw_pick_popup(f: &mut Frame, app: &App, hints_area: Rect) {
    let Some((title, items, sel)) = app.pick_items() else {
        return;
    };
    let h = (items.len() + 2).min(12).max(3) as u16;
    let w = (hints_area.width as f32 * 0.5) as u16;
    let x = hints_area.x + 2;
    let y = hints_area.y.saturating_sub(h);
    let rect = Rect::new(x, y, w.max(30).min(hints_area.width), h);
    f.render_widget(Clear, rect);

    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(WARN_YELLOW))
        .title(Line::from(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(WARN_YELLOW)
                .add_modifier(Modifier::BOLD),
        )));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let rows: Vec<Line> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let style = if i == sel {
                Style::default()
                    .fg(GOLD)
                    .bg(Color::Rgb(64, 48, 26))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::from(Span::styled(
                format!(" {} {item}", if i == sel { "▶" } else { " " }),
                style,
            ))
        })
        .collect();
    f.render_widget(Paragraph::new(rows), inner);
}

/// 工具审批弹窗（覆盖渲染；y/n/a 裁决；提示行固定占底行）。
fn draw_approval_popup(f: &mut Frame, app: &App, area: Rect) {
    let Some(pending) = app.approval_queue.front() else {
        return;
    };
    let popup = centered_rect(66, 62, area);
    f.render_widget(Clear, popup);
    let req = &pending.request;
    let rows_in = vec![
        Line::from(vec![
            Span::styled("来源任务: ", Style::default().fg(DIM)),
            Span::styled(
                format!("#{}", pending.run_id),
                Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("工具: ", Style::default().fg(DIM)),
            Span::styled(req.tool.clone(), Style::default().fg(WARN_YELLOW)),
            Span::styled("   调用: ", Style::default().fg(DIM)),
            Span::raw(trunc(&req.tool_call_id, 16)),
        ]),
        Line::from(vec![
            Span::styled("摘要: ", Style::default().fg(DIM)),
            Span::raw(req.summary.clone()),
        ]),
        Line::from(""),
        Line::from(Span::styled("详情:", Style::default().fg(DIM))),
        // 超长 detail 截断（提示行独立占弹窗底部一行，任何长度都顶不掉）。
        Line::from(Span::styled(
            trunc(&req.detail, 400),
            Style::default().fg(Color::White),
        )),
    ];
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(WARN_YELLOW))
        .title(Line::from(Span::styled(
            " ⚠ 工具审批待裁决 ",
            Style::default()
                .fg(WARN_YELLOW)
                .add_modifier(Modifier::BOLD),
        )));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    f.render_widget(Paragraph::new(rows_in), rows[0]);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "y=批准   n=拒绝   a=本会话全放行（新建/切换会话后复位）   Esc=拒绝",
            Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
        ))),
        rows[1],
    );
}

/// 居中弹窗矩形（相对百分比）。
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
