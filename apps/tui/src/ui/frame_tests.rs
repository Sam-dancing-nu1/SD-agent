//! 帧 dump 回归断言（Bug A/B/C）：审批弹窗几何（标题入框 + 居中）、思考长
//! 文本折行不越界、生命周期系统行不入对话流。
//!
//! 与 ui/mod.rs frame_dump 同口径（TestBackend 全帧文本快照）；
//! `cargo test -p sd-tui frame_tests -- --nocapture` 输出帧 dump 供肉眼比对。

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::app::worker::{FeedItem, PendingApproval, WorkerState};
use crate::app::{App, Mode};
use crate::ui::chat;

const W: u16 = 124;
const H: u16 = 40;

/// 渲染一帧：返回（文本快照, 终端）——几何断言直接读 buffer。
fn render(app: &mut App, w: u16, h: u16) -> (String, Terminal<TestBackend>) {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| {
            let _hits = crate::ui::draw(app, f);
        })
        .expect("draw");
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..h {
        for x in 0..w {
            if let Some(cell) = buf.cell((x, y)) {
                out.push_str(cell.symbol());
            }
        }
        out.push('\n');
    }
    (out, terminal)
}

fn dump(app: &mut App, w: u16, h: u16) -> String {
    render(app, w, h).0
}

/// 存在性断言（口径同 ui/mod.rs frame_dump：CJK 宽字符 padding 先去空格）。
fn has(buf: &str, needle: &str) -> bool {
    buf.contains(needle) || buf.replace(' ', "").contains(&needle.replace(' ', ""))
}

fn worker_app(name: &str) -> App {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio");
    App::new_worker(
        std::env::temp_dir().join(format!("sd-tui-frame-tests-{name}")),
        "0.1.0-alpha.1",
        rt,
        None,
    )
}

fn approval(name: &str, summary: &str) -> PendingApproval {
    let (reply_tx, _reply_rx) = std::sync::mpsc::channel();
    PendingApproval {
        request: sd_agent::policy::ApprovalRequest {
            tool_call_id: name.into(),
            tool: "bash".into(),
            summary: summary.into(),
            detail: String::new(),
        },
        reply: reply_tx,
    }
}

fn long_reasoning() -> String {
    "Task anchor:现在是几点。User says \"现在是几点\". The goal anchor asks whether to \
simply answer with the final summary and do not call tools… The task is simply to \
answer what time it is. I could answer using the system date… but I don't have a live \
clock in context, so I will state the observed time from the task anchor and stop. \
这是一段很长的思考文本，验证折行后任意宽度下都不越界：混排中文、English、省略号…、\
\u{201c}引号\u{201d}、破折号\u{2014}等歧义宽度字符也一并覆盖。"
        .to_string()
}

/// Bug A：审批弹窗居中（约 50%×40%）、标题画在框顶边内、内容在框内。
#[test]
fn approval_popup_centered_with_title_inside() {
    let mut app = worker_app("approval");
    if let Mode::Worker(w) = &mut app.mode {
        w.push(FeedItem::User("现在是几点".into()));
        w.approval = Some(approval("c2", "bash: date \"+%Y-%m-%d %H:%M:%S %Z (%A)\""));
    }
    let (dump, term) = render(&mut app, W, H);
    println!("── 审批弹窗帧（{W}x{H}）──\n{dump}");
    let buf = term.backend().buffer();

    // 标题所在行（⚠ 所在行 = 弹窗顶边行）。
    let mut title_row = None;
    let mut warn_col = None;
    for y in 0..H {
        for x in 0..W {
            if buf.cell((x, y)).map(|c| c.symbol()) == Some("⚠") {
                title_row = Some(y);
                warn_col = Some(x);
            }
        }
    }
    let ty = title_row.expect("审批弹窗应有 ⚠ 标题");
    let warn_x = warn_col.expect("⚠ 列");

    // 弹窗顶边行的左上角 ┌ / 右上角 ┐。
    let sym = |x: u16, y: u16| -> String {
        buf.cell((x, y))
            .map(|c| c.symbol().to_string())
            .unwrap_or_default()
    };
    let l = (0..W).find(|&x| sym(x, ty) == "┌").expect("弹窗左上角 ┌");
    let r = (0..W)
        .rev()
        .find(|&x| sym(x, ty) == "┐")
        .expect("弹窗右上角 ┐");

    // 标题在框顶边内（⚠ 与「工具审批」都落在 ┌ ┐ 之间）。
    assert!(
        l < warn_x && warn_x < r,
        "⚠ 标题应在弹窗框内：l={l} warn={warn_x} r={r}"
    );
    let top_row_text: String = (l + 1..r).map(|x| sym(x, ty)).collect();
    assert!(
        has(&top_row_text, "工具审批"),
        "标题「工具审批」应画在框顶边内: {top_row_text:?}"
    );

    // 弹窗底边：同列向下找 └ / ┘。
    let by = ((ty + 1)..H)
        .find(|&y| sym(l, y) == "└" && sym(r, y) == "┘")
        .expect("弹窗底边 └┘");

    // 居中 + 约 50%×40%。
    let (pw, ph) = (r - l + 1, by - ty + 1);
    let (ml, mr) = (l as i32, (W - 1 - r) as i32);
    let (mt, mb) = (ty as i32, (H - 1 - by) as i32);
    assert!((ml - mr).abs() <= 2, "弹窗应水平居中：左空 {ml} 右空 {mr}");
    assert!((mt - mb).abs() <= 2, "弹窗应垂直居中：上空 {mt} 下空 {mb}");
    assert!(
        (pw as u32 * 100) / u32::from(W) >= 45 && (pw as u32 * 100) / u32::from(W) <= 55,
        "弹窗宽度约占 50%：实际 {}%",
        (pw as u32 * 100) / u32::from(W)
    );
    assert!(
        (ph as u32 * 100) / u32::from(H) >= 33 && (ph as u32 * 100) / u32::from(H) <= 47,
        "弹窗高度约占 40%：实际 {}%",
        (ph as u32 * 100) / u32::from(H)
    );

    // 内容（工具/摘要/按键提示）整体在框内。
    let mut inner = String::new();
    for y in ty + 1..by {
        for x in l + 1..r {
            inner.push_str(&sym(x, y));
        }
    }
    assert!(has(&inner, "工具"), "「工具」应在弹窗框内");
    assert!(has(&inner, "bash"), "工具名应在弹窗框内");
    assert!(has(&inner, "摘要"), "「摘要」应在弹窗框内");
    assert!(has(&inner, "Y 放行"), "按键提示应在弹窗框内");
}

/// Bug C：思考长文本折行后任意宽度不越界（行宽 ≤ 内宽），且内容不丢。
#[test]
fn reasoning_long_text_never_overflows() {
    let text = long_reasoning();
    let feed = vec![FeedItem::Reasoning {
        text: text.clone(),
        streaming: false,
        open: true,
    }];
    for width in [37u16, 60, 80, 124, 200] {
        let inner_w = usize::from(width.saturating_sub(2));
        // 行级（真实断言口径）：每条折行结果的显示宽度 ≤ 内宽。
        let wrapped = chat::render_feed_window(&feed, 0, 10_000, inner_w);
        let mut joined = String::new();
        for line in &wrapped.lines {
            let line_text: String = line
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>();
            assert!(
                chat::line_cols(line) <= inner_w,
                "折行越界（宽 {width}）：{:?}",
                line_text
            );
            joined.push_str(&line_text);
        }
        // 内容不丢：折行只断行不删字。
        assert!(
            joined.replace(' ', "").contains(&text.replace(' ', "")),
            "折行后内容应完整（宽 {width}）"
        );

        // 帧级：对话框边框列不被文本侵入。
        let mut app = worker_app(&format!("wrap{width}"));
        if let Mode::Worker(w) = &mut app.mode {
            w.push(FeedItem::Reasoning {
                text: text.clone(),
                streaming: false,
                open: true,
            });
        }
        let frame = dump(&mut app, width, H);
        for row in frame.lines() {
            if row.starts_with('│') {
                // 对话框行：右边界列必须仍是边框/滚动条字符。
                let last = row.chars().rev().find(|c| !c.is_whitespace());
                assert!(
                    matches!(last, Some('│') | Some('█') | Some('┘') | Some('┐')),
                    "帧 {width} 文本侵入对话框右边界列：{row:?}"
                );
            }
        }
    }
}

/// Bug B：生命周期系统行（run_start/round_start/turn 等）不入对话流
/// （渲染过滤 + 进料过滤）；非生命周期干预行照常显示。
#[test]
fn feed_hides_lifecycle_system_lines() {
    let mut app = worker_app("filter");
    if let Mode::Worker(w) = &mut app.mode {
        w.push(FeedItem::User("现在是几点".into()));
        for line in [
            "[系统] run_start · run started",
            "[系统] round_start · round 1",
            "[系统] round_end · round 1 done",
            "[系统] run_end · run end",
            "[系统] tool_before · bash",
            "[系统] model_turn_started · turn 1",
        ] {
            w.push(FeedItem::Intervention(line.into()));
        }
        w.push(FeedItem::Tool {
            tool_call_id: "c1".into(),
            name: "bash".into(),
            args: "{\"cmd\":\"date\"}".into(),
            status: crate::app::worker::ToolStatus::Done("ok".into()),
        });
    }
    let frame = dump(&mut app, W, H);
    println!("── 过滤生命周期系统行帧（{W}x{H}）──\n{frame}");
    assert!(has(&frame, "现在是几点"), "用户消息应保留");
    assert!(has(&frame, "bash"), "工具卡应保留");
    assert!(
        !frame.contains("[系统]"),
        "feed 无 [系统] 生命周期行（轨迹文件留痕，不入对话流）"
    );
    for hidden in [
        "run_start",
        "round_start",
        "round_end",
        "run_end",
        "tool_before",
    ] {
        assert!(!frame.contains(hidden), "生命周期事件 {hidden} 不应渲染");
    }

    // 边界：非生命周期干预行照常显示。
    let mut app = worker_app("filter-keep");
    if let Mode::Worker(w) = &mut app.mode {
        w.push(FeedItem::Intervention(
            "[系统] round 2 · 上下文已组装".into(),
        ));
        w.push(FeedItem::Intervention(
            "[系统] 已请求取消：执行线程将自然收尾".into(),
        ));
    }
    let frame = dump(&mut app, W, H);
    assert!(
        has(&frame, "[系统] round 2 · 上下文已组装"),
        "非生命周期干预行应保留"
    );
    assert!(has(&frame, "[系统] 已请求取消"), "取消提示应保留");

    // 进料侧（worker::on_event）同样拦截：生命周期 Hook 事件不进 feed。
    let mut w = WorkerState::new(None);
    let ev = sd_agent::event::Event {
        v: 1,
        trace_id: "t".into(),
        seq: 1,
        ts_unix_ms: 0,
        kind: sd_agent::event::EventKind::Hook,
        payload: sd_agent::event::EventPayload::Hook(sd_agent::event::Hook {
            hook: "run_start".into(),
            note: "run started".into(),
        }),
    };
    let redraw = w.on_msg(crate::bridge::UiMsg::Event {
        run_id: 1,
        event: ev,
    });
    assert!(!redraw, "生命周期事件不应触发重绘");
    assert!(w.feed.is_empty(), "生命周期事件不应进 feed");
}
