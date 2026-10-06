//! hub 主页渲染：初始态（图1 规格）/ WORK 态（图2），两态动画按
//! anim.progress() 插值布局。每帧产出 HitRects（契约 ui/layout.rs）。
//!
//! 初始态自上而下（设计重心铁律：发起对话前 UI 一切为发起新对话服务）：
//! 1) 上方约 1/3 大留白（克制，不堆元素）；
//! 2) 大 Logo 居中（SVG 渲染链，视觉主角）；
//! 3) 居中输入框（约 60% 宽）：上部输入区（placeholder "输入任务…"）+
//!    框内底部模型信息行（`对话 · 模型 · 思考强度 ▾`，整行点击开模型浮层、
//!    分段档点击改档）；
//! 4) 快捷键提示行（`M 模型　/ 命令　? 键位`）；
//! 5) Tip 提示（● Tip …，实用提示，装饰性）；
//! 6) 底部两栏：左 1/3 历史会话 / 右 2/3 token 记录（初始态即在场）；
//! 7) 右下角版本号（低对比度小字）。
//!
//! WORK 态：顶栏（小 Logo + 输入框）+ 左 1/3 历史 + 右 2/3 统计（见
//! ui/panels.rs，两态组件样式统一）。
//!
//! 交互形态调研（吸收形态不抄实现）：OpenCode TUI 首屏居中大标识 + 输入框
//! 内嵌模型信息行（opencode.ai/docs/tui）、Claude Code /model 单浮层选择器 +
//! effort 调节（developersdigest.tech/guides/model-picker）。

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::hub::HubState;
use crate::app::Focus;
use crate::ui::layout::HitRects;
use crate::ui::theme;
use crate::ui::widgets;

/// 初始态输入框高度（边框 2 + 输入区 3 + 模型信息行 1）。
const INPUT_H0: u16 = 6;
/// WORK 态顶栏高度（小 Logo + 输入框）。
const TOP_H1: u16 = 3;
/// WORK 态顶栏小 Logo 宽度。
const LOGO_W1: u16 = 22;
/// Logo 大小形态切换阈值（t≥此值渲染顶栏小 Logo）。
const SMALL_LOGO_AT: f32 = 0.6;
/// 初始态输入框宽度占比（图1 ≈60%）。
const INPUT_W_PCT: u16 = 60;
/// 底部两栏高度占比（两态同一分割，动画只做伸缩位移）。
const PANELS_PCT: u16 = 30;
/// 上方留白占比（图1 约 1/3，实际受 Logo 尺寸下限约束）。
const TOP_PAD_PCT: u16 = 14;

/// 渲染 hub（占满给定区域，状态栏由 ui::mod 绘制），返回可交互组件登记。
pub fn draw(
    area: Rect,
    hub: &HubState,
    focus: Focus,
    version: &str,
    frame: &mut ratatui::Frame,
) -> HitRects {
    let mut hits = HitRects::default();
    let t = work_amount(hub);
    let sk = skeleton(area, t);

    draw_logo(sk.logo, version, t, frame);
    hits.logo = Some(sk.logo);

    if let Some((model_row, bar)) = draw_input(sk.input, hub, focus, t, frame) {
        hits.model_bar = Some(model_row);
        hits.effort_slider = bar;
    }
    hits.input = Some(sk.input);

    // 底部/内容两栏（初始态短栏精简内容 / WORK 态全高全量；样式统一）。
    super::panels::draw(sk.content, hub, focus, t < 0.5, frame, &mut hits);

    // 初始态专属：快捷键行 / Tip / 右下角版本号（动画中渐隐 = 直接消失）。
    if t < 0.5 {
        draw_key_hints(sk.hints, frame);
        draw_tips(sk.tips, frame);
        if let Some(token_panel) = hits.token_panel {
            draw_version_corner(token_panel, version, frame);
        }
    }
    hits
}

/// 两态混合系数：0=初始态、1=WORK 态。动画中按 progress() 插值（ease-out
/// 已在 Anim 内），无动画时按 is_work() 定格。
fn work_amount(hub: &HubState) -> f32 {
    let anim = &hub.anim;
    if anim.done() {
        // 定格：无动画按 is_work；已触发过动画按落点（含结束帧）。
        return match anim.kind() {
            Some(_) => {
                if anim.heading_to_work() {
                    1.0
                } else {
                    0.0
                }
            }
            None => {
                if hub.is_work() {
                    1.0
                } else {
                    0.0
                }
            }
        };
    }
    let p = anim.progress();
    if anim.heading_to_work() {
        p
    } else {
        1.0 - p
    }
}

/// 布局骨架（Logo / 输入框 / 内容两栏 / 快捷键行 / Tip）。
struct Skeleton {
    logo: Rect,
    input: Rect,
    content: Rect,
    hints: Rect,
    tips: Rect,
}

/// 两态布局插值：Logo+输入框平滑上移到顶栏，底部两栏推入为全高内容区。
fn skeleton(area: Rect, t: f32) -> Skeleton {
    // ── 初始态（图1）：留白 + 大 Logo + 居中输入框 + 快捷键行 + Tip + 底部两栏 ──
    let hints_h = 1u16.min(area.height);
    let gap_h = 1u16.min(area.height);
    let input_h = INPUT_H0.min(area.height);
    let panels_h = ((area.height * PANELS_PCT) / 100)
        .clamp(7, 12)
        .min(area.height.saturating_sub(10));
    let tips_h = 2u16.min(area.height);
    let fixed = input_h + hints_h + gap_h + tips_h + panels_h;
    let leftover = area.height.saturating_sub(fixed);
    let mut top_pad = ((area.height * TOP_PAD_PCT) / 100).min(leftover);
    let mut logo_h = leftover - top_pad;
    if logo_h < 4 {
        // Logo 给足下限 4 行，缺的从留白里挪（留白仍然最大块）。
        let steal = 4u16.saturating_sub(logo_h).min(top_pad);
        top_pad -= steal;
        logo_h += steal;
    }

    let in_w0 = ((area.width * INPUT_W_PCT) / 100)
        .clamp(20, 84)
        .min(area.width);
    let l0 = fit(area, area.x, area.y + top_pad, area.width, logo_h);
    let i0 = fit(
        area,
        area.x + (area.width.saturating_sub(in_w0)) / 2,
        l0.bottom(),
        in_w0,
        input_h,
    );
    let hh0 = fit(area, area.x, i0.bottom(), area.width, hints_h);
    let tp0 = fit(area, area.x, hh0.bottom() + gap_h, area.width, tips_h);
    let c0 = fit(
        area,
        area.x,
        area.bottom().saturating_sub(panels_h),
        area.width,
        panels_h,
    );

    // ── WORK 态（图2）：顶栏（小 Logo + 输入框）+ 全高两栏 ──
    let rows1 = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(TOP_H1), Constraint::Min(6)])
        .split(area);
    let top1 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(LOGO_W1), Constraint::Min(20)])
        .split(rows1[0]);

    Skeleton {
        logo: lerp_rect(l0, top1[0], t),
        input: lerp_rect(i0, top1[1], t),
        content: lerp_rect(c0, rows1[1], t),
        hints: hh0,
        tips: tp0,
    }
}

/// 夹取到 area 内的矩形（窄/矮终端不越界，渲染不 panic）。
fn fit(area: Rect, x: u16, y: u16, w: u16, h: u16) -> Rect {
    let x = x.min(area.right());
    let y = y.min(area.bottom());
    Rect {
        x: area.x + x.saturating_sub(area.x),
        y,
        width: w.min(area.right().saturating_sub(x)),
        height: h.min(area.bottom().saturating_sub(y)),
    }
}

/// 矩形线性插值（t=0/1 精确落回端点）。
fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    let lp = |x: u16, y: u16| -> u16 { (x as f32 + (y as f32 - x as f32) * t).round() as u16 };
    Rect {
        x: lp(a.x, b.x),
        y: lp(a.y, b.y),
        width: lp(a.width, b.width),
        height: lp(a.height, b.height),
    }
}

/// Logo：t<阈值渲染大字形（行数随 t 渐减、垂直居中），t≥阈值渲染顶栏小 Logo。
fn draw_logo(rect: Rect, version: &str, t: f32, frame: &mut ratatui::Frame) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    if t >= SMALL_LOGO_AT {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border());
        frame.render_widget(
            Paragraph::new(super::logo::small(version)).block(block),
            rect,
        );
        return;
    }
    // 大 Logo：SVG 网格字形（半字符 ▀ 双色）。尺寸渐变 = 整体重渲染
    // 缩小（早期 take(k) 截断会把正方形拦腰截成横条，用户实测抓包）。
    let mut lines = Vec::new();
    super::logo::render_into(&mut lines, rect.width as usize, rect.height as usize);
    let full = lines.len() as u16;
    let k = (((full as f32) * (1.0 - t / SMALL_LOGO_AT)).ceil() as u16)
        .max(2)
        .min(full)
        .min(rect.height);
    if k < full {
        // 渐变途中：按当前尺寸重画完整正方形（宽=2k），不截断。
        lines.clear();
        super::logo::render_into(&mut lines, (2 * k) as usize, k as usize);
    }
    let y = rect.y + (rect.height.saturating_sub(k)) / 2;
    let sub = Rect {
        x: rect.x,
        y,
        width: rect.width,
        height: k,
    };
    // 居中对齐（早期左对齐贴左上角，用户实测抓包）。
    frame.render_widget(Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center), sub);
}

/// 输入框（两态共用，随骨架插值移动缩放）。初始态含框内底部模型信息行，
/// 返回 (模型信息行 Rect, 思考强度分段档 Rect) 供 HitRects 登记。
fn draw_input(
    rect: Rect,
    hub: &HubState,
    focus: Focus,
    t: f32,
    frame: &mut ratatui::Frame,
) -> Option<(Rect, Option<Rect>)> {
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    let focused = focus == Focus::Input;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if focused {
            theme::border_focus()
        } else {
            theme::border()
        })
        .title(" 任务 ")
        .title_style(theme::title());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 || inner.width == 0 {
        return None;
    }

    // 模型信息行（初始态）：框内底部行；输入区占其余行。
    let show_model = t < 0.5 && inner.height >= 2;
    let text_h = if show_model {
        inner.height - 1
    } else {
        inner.height
    };
    let text_area = Rect {
        height: text_h,
        ..inner
    };

    let placeholder = if t >= SMALL_LOGO_AT {
        "新任务，回车开新会话"
    } else {
        "输入任务…"
    };
    if hub.editor.text.is_empty() {
        let ph = Paragraph::new(Line::from(Span::styled(placeholder, theme::dim())));
        frame.render_widget(ph, text_area);
    } else {
        let p = Paragraph::new(hub.editor.text.clone())
            .style(theme::text())
            .wrap(ratatui::widgets::Wrap { trim: false });
        frame.render_widget(p, text_area);
        if focused {
            // 光标按显示宽度定位（CJK=2 列近似；多行折行不追踪——已知限制）。
            let col = super::cursor_display_col(&hub.editor.text, hub.editor.cursor());
            let cx = text_area.x + col.min(text_area.width.saturating_sub(1));
            frame.set_cursor_position((cx, text_area.y));
        }
    }

    if !show_model {
        return None;
    }
    let row = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    let model = hub
        .profiles
        .get(hub.profile_sel)
        .map(|p| p.model.as_str())
        .unwrap_or("—");
    let (line, bar_x) = widgets::model_info_line(model, hub.effort_idx, row.width as usize);
    frame.render_widget(Paragraph::new(line), row);
    let bar_w = crate::slash::EFFORTS.len() as u16;
    let bar = bar_x.and_then(|bx| {
        if bx + bar_w <= row.width {
            Some(Rect {
                x: row.x + bx,
                y: row.y,
                width: bar_w,
                height: 1,
            })
        } else {
            None
        }
    });
    Some((row, bar))
}

/// 快捷键提示行（输入框下方一行居中；键位语义同 keymap：M 模型浮层、
/// / 命令浮层、? 键位帮助）。
fn draw_key_hints(rect: Rect, frame: &mut ratatui::Frame) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let key = |k: &str| Span::styled(k.to_string(), theme::muted().add_modifier(Modifier::BOLD));
    let label = |l: &str| Span::styled(format!(" {l}"), theme::dim());
    let line = Line::from(vec![
        key("M"),
        label("模型"),
        Span::styled("　", theme::dim()),
        key("/"),
        label("命令"),
        Span::styled("　", theme::dim()),
        key("?"),
        label("键位"),
    ]);
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), rect);
}

/// Tip 提示（快捷键行下方 1-2 行居中，● Tip 形态；内容为实用提示）。
fn draw_tips(rect: Rect, frame: &mut ratatui::Frame) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let lines = vec![
        Line::from(vec![
            Span::styled("● ", theme::accent()),
            Span::styled("Tip ", theme::accent().add_modifier(Modifier::BOLD)),
            Span::styled("回车发起对话，将在新窗口执行", theme::muted()),
        ]),
        Line::from(Span::styled("按 ? 查看全部键位", theme::dim())),
    ];
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), rect);
}

/// 右下角版本号（token 面板内侧末行右端，低对比度小字；不压边框）。
fn draw_version_corner(panel: Rect, version: &str, frame: &mut ratatui::Frame) {
    if panel.height < 3 || panel.width < 6 {
        return;
    }
    let w = widgets::disp_width(version) as u16;
    let x = panel.right().saturating_sub(w + 2);
    if x <= panel.x {
        return;
    }
    let y = panel.bottom() - 2;
    frame
        .buffer_mut()
        .set_string(x, y, version.to_string(), theme::dim());
}

/// 供 ui::mod 复用的状态栏行（全 owned，免生命周期拼接）。
pub fn status_line(version: &str, mode: &str, status: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(" SD ", theme::accent().add_modifier(Modifier::BOLD)),
        Span::styled(version.to_string(), theme::muted()),
        Span::styled(" │ ", theme::dim()),
        Span::styled(mode.to_string(), theme::info()),
        Span::styled(" │ ", theme::dim()),
        Span::styled(status.to_string(), theme::dim()),
    ])
}
