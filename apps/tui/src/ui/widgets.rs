//! 通用浮层组件：居中弹窗矩形、帮助浮层、斜杠命令浮层、审批弹窗。

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::app::form::{FormField, FormState};
use crate::app::worker::WorkerState;
use crate::keymap;
use crate::slash;
use crate::ui::theme;

/// 居中弹窗矩形（按百分比占父区域）。
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vert = Layout::default()
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
        .split(vert[1])[1]
}

/// 帮助浮层（键位表，与 keymap::HELP 同源）。
pub fn draw_help(frame: &mut ratatui::Frame, area: Rect) {
    let rect = centered_rect(64, 70, area);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .title(" 键位（? 关闭） ")
        .title_style(theme::title())
        .borders(Borders::ALL)
        .border_style(theme::border_focus())
        .style(theme::text().bg(theme::POPUP_BG));
    let mut lines = vec![Line::from("")];
    for (k, d) in keymap::HELP {
        lines.push(Line::from(vec![
            Span::styled(format!("  {k:<14}"), theme::accent()),
            Span::styled(d.to_string(), theme::text()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  命令：/help /doctor /model /settings /effort /new",
        theme::muted(),
    )));
    let p = Paragraph::new(lines).block(block);
    frame.render_widget(p, rect);
}

/// 斜杠命令浮层（输入以 / 开头时弹出，按前缀过滤）：选中行按 `sel` 高亮
/// （theme::selected），浮层位置尽量贴输入框上方（放不下改贴输入框下方）。
/// 返回浮层 Rect（HitRects.slash 登记）；无可匹配命令时 None。
/// 行序 = slash::filter 结果顺序，与交互层按 y 定位行的几何约定一致
/// （首行起逐行 = 命令表顺序，行 y = rect.y + 1 + i）。
pub fn draw_slash(
    frame: &mut ratatui::Frame,
    area: Rect,
    typed: &str,
    sel: usize,
    anchor: Option<Rect>,
) -> Option<Rect> {
    let matches = slash::filter(typed);
    if matches.is_empty() {
        return None;
    }
    let height = (matches.len() as u16 + 2).min(14).min(area.height);
    let width = area.width.saturating_sub(4).min(56).max(20.min(area.width));
    // 贴输入框锚定：x 对齐输入框左缘；y 尽量在输入框上方，放不下贴输入框下方。
    let x = anchor
        .map(|a| a.x)
        .unwrap_or(area.x + 2)
        .min(area.right().saturating_sub(width));
    let y = match anchor {
        Some(a) if a.y >= area.y + height => a.y - height,
        Some(a) => a.bottom().min(area.bottom().saturating_sub(height)),
        None => area.bottom().saturating_sub(height + 8),
    };
    let rect = Rect {
        x,
        y,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    let items: Vec<ListItem> = matches
        .iter()
        .map(|c| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<14}", c.name), theme::accent()),
                Span::styled(c.desc, theme::muted()),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border_focus())
                .style(theme::text().bg(theme::POPUP_BG)),
        )
        .highlight_style(theme::selected())
        .highlight_symbol("▶ ");
    let sel = sel.min(matches.len().saturating_sub(1));
    let mut state = ListState::default().with_selected(Some(sel));
    frame.render_stateful_widget(list, rect, &mut state);
    Some(rect)
}

/// 审批弹窗（run 线程阻塞等待裁决）：居中浮层，约占屏幕 50%×40%，
/// 标题即框顶边（Block::title 画在顶边内），内容整体在框内，浮在对话流上层。
pub fn draw_approval(frame: &mut ratatui::Frame, area: Rect, w: &WorkerState) {
    let Some(pending) = &w.approval else {
        return;
    };
    let rect = centered_rect(50, 40, area);
    frame.render_widget(Clear, rect);
    let req = &pending.request;
    let block = Block::default()
        .title(" ⚠ 工具审批")
        .title_style(theme::accent())
        .borders(Borders::ALL)
        .border_style(theme::border_focus())
        .style(theme::text().bg(theme::POPUP_BG));
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  工具：", theme::muted()),
            Span::styled(req.tool.clone(), theme::info()),
        ]),
        Line::from(vec![
            Span::styled("  摘要：", theme::muted()),
            Span::styled(req.summary.clone(), theme::text()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Y 放行 · A 本次会话全放行 · N 拒绝（Esc 同）",
            theme::accent(),
        )),
        Line::from(""),
    ];
    // 长摘要在框内折行，不越框。
    let p = Paragraph::new(lines)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(p, rect);
}

/// worker 顶部状态提示条（非浮层）。
pub fn worker_note<'a>(w: &WorkerState) -> Line<'a> {
    let running = match w.running {
        Some(_) => "● 运行中",
        None => "○ 空闲",
    };
    Line::from(Span::styled(
        format!("{running} · Enter 发送 · Esc 取消 · ? 键位 · / 命令"),
        theme::dim(),
    ))
}

// ── hub 重做通用件（宽度口径 / 滚动条 / 滑条 / 芯片 / 表单弹窗） ──
// 交互形态调研：OpenCode TUI 输入栏内嵌模型下拉（opencode.ai/docs/tui）、
// Claude Code /model 单浮层选择器+effort 调节（developersdigest.tech/guides/
// model-picker）、分段块状滑条（docs.rs/tui-slider）。

/// 按显示宽度截断 s 到 cols 列（CJK 按 2 列近似，与 cursor_display_col 同口径）。
pub fn fit_width(s: &str, cols: usize) -> String {
    let mut out = String::new();
    let mut w = 0usize;
    for c in s.chars() {
        let cw = if super::is_wide(c) { 2 } else { 1 };
        if w + cw > cols {
            break;
        }
        out.push(c);
        w += cw;
    }
    out
}

/// 右侧补空格到 cols 显示列（标签列对齐用）。
pub fn pad_width(s: &str, cols: usize) -> String {
    let w: usize = s
        .chars()
        .map(|c| if super::is_wide(c) { 2 } else { 1 })
        .sum();
    format!("{}{}", s, " ".repeat(cols.saturating_sub(w)))
}

/// 侧边滚动条（自绘 1 列：滑块 '█' + 轨道 '│'），画在面板右边界列上。
/// area=带边框的面板矩形；total=内容总行数、visible=可见行数、offset=顶部偏移。
pub fn draw_scrollbar(
    frame: &mut ratatui::Frame,
    area: Rect,
    total: usize,
    visible: usize,
    offset: usize,
) {
    if area.height < 3 || area.width == 0 || total <= visible.max(1) {
        return;
    }
    let x = area.x + area.width - 1;
    let track_h = area.height.saturating_sub(2) as usize;
    if track_h == 0 {
        return;
    }
    let thumb = ((visible * track_h) / total).clamp(1, track_h);
    let max_off = total - visible;
    let start = if max_off == 0 {
        0
    } else {
        (offset.min(max_off) * (track_h - thumb)) / max_off
    };
    let buf = frame.buffer_mut();
    for i in 0..track_h {
        let (ch, style) = if i >= start && i < start + thumb {
            ('█', theme::scroll_thumb())
        } else {
            ('│', theme::scroll_track())
        };
        buf.set_string(x, area.y + 1 + i as u16, ch.to_string(), style);
    }
}

/// 思考强度 7 档分段滑条字符段（已过=▓、当前=█、未到=░）。
pub fn effort_bar_spans(idx: usize) -> Vec<Span<'static>> {
    (0..slash::EFFORTS.len())
        .map(|i| {
            let (ch, style) = if i < idx {
                ('▓', theme::slider_on())
            } else if i == idx {
                ('█', theme::slider_on())
            } else {
                ('░', theme::slider_off())
            };
            Span::styled(ch.to_string(), style)
        })
        .collect()
}

/// 显示宽度（CJK 按 2 列近似，与 cursor_display_col 同口径）。
pub fn disp_width(s: &str) -> usize {
    s.chars()
        .map(|c| if super::is_wide(c) { 2 } else { 1 })
        .sum()
}

/// 初始态输入框底部的模型信息行（图1 规格）：`对话 · {model} · ████░ {effort} ▾`。
/// 模式标签 · 模型名 · 思考强度分段档 + 档名，右端 ▾ 为可点击切换形态。
/// 整行点击开模型浮层（HitRects.model_bar），分段档按列映射改档
/// （HitRects.effort_slider）。返回 (行, 分段档起始显示列；被截断则 None)。
pub fn model_info_line(
    model: &str,
    effort_idx: usize,
    cols: usize,
) -> (Line<'static>, Option<u16>) {
    let effort = crate::slash::EFFORTS
        .get(effort_idx)
        .copied()
        .unwrap_or("medium");
    let bar_w = crate::slash::EFFORTS.len();
    // 固定列宽：" 对话 "6 + "· "2 + " · "3 + 档位段 + " "1 + 档名 + " ▾"2。
    let fixed = 6 + 2 + 3 + bar_w + 1 + disp_width(effort) + 2;
    let model_fit = fit_width(model, cols.saturating_sub(fixed).max(1));
    let model_w = disp_width(&model_fit);
    let bar_x = 6 + 2 + model_w + 3;
    let total = 6 + 2 + model_w + 3 + bar_w + 1 + disp_width(effort);
    let mut spans = vec![
        Span::styled(" 对话 ", theme::info()),
        Span::styled("· ", theme::dim()),
        Span::styled(
            model_fit,
            theme::text().add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled(" · ", theme::dim()),
    ];
    spans.extend(effort_bar_spans(effort_idx));
    spans.push(Span::styled(
        format!(" {effort}"),
        theme::accent().add_modifier(ratatui::style::Modifier::BOLD),
    ));
    if total + 2 <= cols {
        spans.push(Span::styled(
            format!("{} ▾", " ".repeat(cols - total - 2)),
            theme::dim(),
        ));
    }
    let bar_x = if bar_x + bar_w <= cols {
        Some(bar_x as u16)
    } else {
        None
    };
    (Line::from(spans), bar_x)
}

/// 模型选择浮层（单浮层即选即生效；↑↓ 高亮、Enter 确认、Esc 关闭——键位在
/// 交互层）。行序与交互层几何约定一致：首行起逐行 = profiles 顺序。
pub fn draw_model_popup(
    frame: &mut ratatui::Frame,
    area: Rect,
    profiles: &[crate::app::hub::ProfileRow],
    sel: usize,
) {
    let width = 46u16.min(area.width.saturating_sub(4));
    let height = (profiles.len() as u16 + 2).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + area.height / 3;
    let rect = Rect {
        x,
        y,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    let items: Vec<ListItem> = profiles
        .iter()
        .map(|p| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<14}", p.label), theme::accent()),
                Span::styled(format!(" · {}", p.model), theme::text()),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .title(" 选择模型（Enter 确认 · Esc 取消） ")
                .title_style(theme::title())
                .borders(Borders::ALL)
                .border_style(theme::border_focus())
                .style(theme::text().bg(theme::POPUP_BG)),
        )
        .highlight_style(theme::selected())
        .highlight_symbol("▶ ");
    let mut state =
        ListState::default().with_selected(Some(sel.min(profiles.len().saturating_sub(1))));
    frame.render_stateful_widget(list, rect, &mut state);
}

/// 配置表单弹窗（读 FormState）：五字段 + 错误提示行 + 保存/取消按钮位，
/// 聚焦字段高亮，密钥打码显示。返回弹窗 Rect（HitRects.form 登记）。
/// 单浮层形态（非 prompt/confirm 连弹），参考 Claude Code /model 选择器。
pub fn draw_form(frame: &mut ratatui::Frame, area: Rect, form: &FormState) -> Rect {
    let width = area.width.saturating_sub(8).min(68).max(36.min(area.width));
    let height = area
        .height
        .saturating_sub(2)
        .min(13)
        .max(10.min(area.height));
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    let title = if form.edit_label.is_some() {
        " 编辑模型配置 "
    } else {
        " 添加模型配置 "
    };
    let block = Block::default()
        .title(title)
        .title_style(theme::title())
        .borders(Borders::ALL)
        .border_style(theme::border_focus())
        .style(theme::text().bg(theme::POPUP_BG));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 || inner.width == 0 {
        return rect;
    }

    // 密钥打码：已填显示掩码；编辑时留空=不改。
    let key_val = if form.api_key.is_empty() {
        if form.edit_label.is_some() {
            "（留空不改）".to_string()
        } else {
            "（未填）".to_string()
        }
    } else {
        "••••••••（已填）".to_string()
    };
    let val_w = inner.width.saturating_sub(14) as usize;
    let fields: [(FormField, &str, String); 5] = [
        (FormField::Label, "配置名", form.label.clone()),
        (FormField::BaseUrl, "端点", form.base_url.clone()),
        (FormField::Model, "模型名", form.model.clone()),
        (FormField::ApiKey, "API 密钥", key_val),
        (
            FormField::Effort,
            "思考强度",
            form.effort_name().to_string(),
        ),
    ];

    let mut lines: Vec<Line> = vec![Line::from("")];
    for (field, name, value) in fields {
        let focused = form.focus == field;
        let marker = if focused { "▸" } else { " " };
        let val_style = if focused {
            theme::accent()
        } else {
            theme::field_value()
        };
        let mut val_spans: Vec<Span> = Vec::new();
        if field == FormField::Effort {
            val_spans.extend(effort_bar_spans(form.effort_idx));
            val_spans.push(Span::styled(format!(" {value}"), val_style));
        } else {
            let mut v = fit_width(&value, val_w);
            if focused {
                v.push('▍');
            }
            val_spans.push(Span::styled(v, val_style));
        }
        let mut spans = vec![
            Span::styled(
                format!("{marker} "),
                if focused {
                    theme::accent()
                } else {
                    theme::dim()
                },
            ),
            Span::styled(pad_width(name, 12), theme::field_label()),
        ];
        spans.extend(val_spans);
        lines.push(Line::from(spans));
    }
    // 错误提示行。
    lines.push(Line::from(match &form.error {
        Some(e) => Span::styled(format!("  ⚠ {}", fit_width(e, val_w + 10)), theme::err()),
        None => Span::raw(""),
    }));
    // 保存 / 取消按钮位。
    lines.push(Line::from(vec![
        Span::styled(
            "  [保存]",
            theme::accent().add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled("  [取消]", theme::muted()),
        Span::styled("   Tab 切字段 · Esc 取消", theme::dim()),
    ]));
    frame.render_widget(Paragraph::new(lines), inner);
    rect
}
