//! 无头自检：TestBackend 渲染多帧断言（--selfcheck）。
//!
//! 目的：渲染层回归护航 + 无 TTY 环境可验证（CI/夜间任务）。断言只查
//! "渲染不 panic + 关键内容出现"，不复刻交互逻辑（交互靠用户旅程验收）。

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::app::worker::{FeedItem, ToolStatus};
use crate::app::{App, Mode};

/// 跑自检：返回退出码（0 全过）。
pub fn run() -> i32 {
    let mut failures: Vec<String> = Vec::new();

    // 帧 1：hub 初始态（大 Logo）。
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio");
        let root = std::env::temp_dir().join("sd-tui-selfcheck-empty");
        let mut app = App::new_hub(root, "0.1.0-alpha.1", rt);
        let frame_ok = render_frame(&mut app, 120, 40);
        match frame_ok {
            Ok(buf) => {
                expect(&mut failures, "帧1 hub 初始态", &buf, "SD");
                expect(&mut failures, "帧1 hub 输入提示", &buf, "输入任务");
            }
            Err(e) => failures.push(format!("帧1 渲染失败: {e}")),
        }
    }

    // 帧 2：worker 对话流（用户/助手/工具卡）。
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio");
        let root = std::env::temp_dir().join("sd-tui-selfcheck-worker");
        let mut app = App::new_worker(root, "0.1.0-alpha.1", rt, None);
        if let Mode::Worker(w) = &mut app.mode {
            w.push(FeedItem::User("帮我看看这个文件".into()));
            w.push(FeedItem::Assistant {
                text: "好的，我先读取。".into(),
                reasoning: String::new(),
                streaming: true,
            });
            w.push(FeedItem::Tool {
                tool_call_id: "c1".into(),
                name: "read".into(),
                args: "{\"path\":\"src/main.rs\"}".into(),
                status: ToolStatus::Done("120 行".into()),
            });
            w.push(FeedItem::Intervention(
                "[系统] round 2 · 上下文已组装".into(),
            ));
        }
        match render_frame(&mut app, 120, 40) {
            Ok(buf) => {
                expect(&mut failures, "帧2 用户消息", &buf, "帮我看看这个文件");
                expect(&mut failures, "帧2 工具卡", &buf, "read");
                expect(&mut failures, "帧2 干预灰字", &buf, "[系统]");
                expect(&mut failures, "帧2 流式光标", &buf, "▍");
            }
            Err(e) => failures.push(format!("帧2 渲染失败: {e}")),
        }
    }

    // 帧 3：审批弹窗（真 PendingApproval）+ 错误修复提示。
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio");
        let root = std::env::temp_dir().join("sd-tui-selfcheck-approval");
        let mut app = App::new_worker(root, "0.1.0-alpha.1", rt, None);
        if let Mode::Worker(w) = &mut app.mode {
            w.push(FeedItem::Tool {
                tool_call_id: "c2".into(),
                name: "bash".into(),
                args: "{\"cmd\":\"cargo test\"}".into(),
                status: ToolStatus::Pending,
            });
            // 真审批弹窗（覆盖 draw_approval 渲染路径）。
            let (reply_tx, _reply_rx) = std::sync::mpsc::channel();
            w.approval = Some(crate::app::worker::PendingApproval {
                request: sd_agent::policy::ApprovalRequest {
                    tool_call_id: "c2".into(),
                    tool: "bash".into(),
                    summary: "执行 cargo test".into(),
                    detail: "cmd: cargo test".into(),
                },
                reply: reply_tx,
            });
            w.push(FeedItem::Error {
                message: "端点不可达".into(),
                hint: "按 R 重试".into(),
            });
        }
        match render_frame(&mut app, 120, 40) {
            Ok(buf) => {
                expect(&mut failures, "帧3 审批弹窗", &buf, "工具审批");
                expect(&mut failures, "帧3 错误修复提示", &buf, "按 R 重试");
            }
            Err(e) => failures.push(format!("帧3 渲染失败: {e}")),
        }
    }

    // 帧 4：帮助浮层。
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio");
        let root = std::env::temp_dir().join("sd-tui-selfcheck-help");
        let mut app = App::new_hub(root, "0.1.0-alpha.1", rt);
        app.help_open = true;
        match render_frame(&mut app, 120, 40) {
            Ok(buf) => {
                expect(&mut failures, "帧4 帮助浮层", &buf, "键位");
            }
            Err(e) => failures.push(format!("帧4 渲染失败: {e}")),
        }
    }

    // 帧 5：统计热力图（hub WORK 态）。
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio");
        let root = std::env::temp_dir().join("sd-tui-selfcheck-stats");
        let mut app = App::new_hub(root, "0.1.0-alpha.1", rt);
        if let Mode::Hub(h) = &mut app.mode {
            h.launched = true;
        }
        match render_frame(&mut app, 120, 40) {
            Ok(buf) => {
                expect(&mut failures, "帧5 Token 总览", &buf, "Token 消耗");
                expect(&mut failures, "帧5 热力图", &buf, "消耗热力图");
            }
            Err(e) => failures.push(format!("帧5 渲染失败: {e}")),
        }
    }

    // 帧 6：窄终端不 panic（80x24）。
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio");
        let root = std::env::temp_dir().join("sd-tui-selfcheck-small");
        let mut app = App::new_hub(root, "0.1.0-alpha.1", rt);
        match render_frame(&mut app, 80, 24) {
            Ok(buf) => expect(&mut failures, "帧6 窄屏", &buf, "SD"),
            Err(e) => failures.push(format!("帧6 渲染失败: {e}")),
        }
    }

    // 帧 7：配置表单弹窗（读 FormState，五字段 + 按钮位）。
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio");
        let root = std::env::temp_dir().join("sd-tui-selfcheck-form");
        let mut app = App::new_hub(root, "0.1.0-alpha.1", rt);
        if let Mode::Hub(h) = &mut app.mode {
            h.form = Some(crate::app::form::FormState::add());
        }
        match render_frame(&mut app, 120, 40) {
            Ok(buf) => {
                expect(&mut failures, "帧7 表单字段", &buf, "配置名");
                expect(&mut failures, "帧7 表单按钮", &buf, "保存");
            }
            Err(e) => failures.push(format!("帧7 渲染失败: {e}")),
        }
    }

    if failures.is_empty() {
        println!("selfcheck: OK（7 帧全过）");
        0
    } else {
        println!("selfcheck: FAIL（{} 项）", failures.len());
        for f in &failures {
            println!("  ✗ {f}");
        }
        1
    }
}

/// 渲染一帧，返回 TestBackend 终端缓冲的文本快照。
fn render_frame(app: &mut App, w: u16, h: u16) -> Result<String, String> {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;
    terminal
        .draw(|f| {
            // ui::draw 现返回 HitRects（交互区登记）；自检只渲染，丢弃返回值。
            crate::ui::draw(app, f);
        })
        .map_err(|e| e.to_string())?;
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
    Ok(out)
}

fn expect(failures: &mut Vec<String>, name: &str, buf: &str, needle: &str) {
    // TestBackend 对 CJK 宽字符会写 padding 格（" "），精确比较先试，
    // 失败再做去空格规范化比较（存在性断言，规范化足够且不改渲染）。
    let hit = buf.contains(needle) || buf.replace(' ', "").contains(&needle.replace(' ', ""));
    if !hit {
        failures.push(format!("{name}: 未找到「{needle}」"));
        // 诊断留痕：失败帧全文落盘（scratch），供排障复查。
        let dir = std::env::temp_dir().join("sd-tui-selfcheck-dump");
        let _ = std::fs::create_dir_all(&dir);
        let safe: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let _ = std::fs::write(dir.join(format!("{safe}.txt")), buf);
    }
}
