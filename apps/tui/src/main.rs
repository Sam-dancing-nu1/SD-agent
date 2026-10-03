//! sd-tui 入口：参数解析、终端生命周期、UI 主事件循环、无头自检。
//!
//! 界面形态：对话式单屏（Pi / OpenCode 风格）——顶部细状态栏（会话标题 /
//! 模型 / 思考强度 / 工作目录 / 运行状态），中间对话流滚动区（用户消息、
//! 助手回复、工具调用卡片、命令卡片），底部输入框（Enter 发送、Shift+Enter
//! 换行、/ 呼出斜杠命令列表）。无多 tab。
//!
//! 架构：UI 主线程跑 ratatui 渲染 + crossterm 按键循环（try_recv 收后台消息
//! 更新状态）；每个 agent run 一条独立 OS 线程 + current_thread tokio runtime
//! （阻塞式审批只卡 run 自己的线程，见 crate::run）。业务逻辑全在 sd-agent
//! 核心，本 crate 只是壳。
//!
//! 退出：q（输入框为空）或 Ctrl+C 连按两次；panic-free 验证用 --selfcheck
//! （TestBackend 渲染六帧断言后退出）。

mod app;
mod bridge;
mod run;
mod slash;
mod ui;

use std::io::{self, IsTerminal, Stdout};
use std::time::Duration;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as TermEvent, KeyEventKind, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{CrosstermBackend, TestBackend};

use app::{App, Flow};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }
    if args.iter().any(|a| a == "--selfcheck") {
        std::process::exit(selfcheck());
    }
    if let Err(e) = run_tui() {
        eprintln!("sd-tui 启动失败: {e}");
        std::process::exit(1);
    }
}

fn print_help() {
    println!(
        "sd-tui · sd-agent 对话式终端界面\n\
         \n\
         用法: sd-tui [--help] [--selfcheck]\n\
         \n\
         界面：顶部状态栏（会话/模型/思考强度/目录/状态）· 中间对话流 ·\n\
              底部输入框（Enter 发送 · Shift+Enter 换行 · / 呼出命令）\n\
         \n\
         斜杠命令：\n\
           /help          命令清单\n\
           /settings      模型配置：列表 / 新增（交互向导） / 切换 / 改思考强度 / 删除\n\
           /model         查看当前模型配置并快速切换\n\
           /effort        改当前模型思考强度（/effort high 等）\n\
           /new           新建会话（当前会话自动存档）\n\
           /conversations 历史会话列表，选择序号切换（/history 别名）\n\
           /runs          任务列表与状态\n\
           /trace         最近轨迹事件流\n\
           /doctor        六项体检，结果卡片流入对话区\n\
           /clear         清空对话区\n\
         \n\
         交互：Esc=关闭列表/取消引导 · ↑↓=历史/选择 · 滚轮/PgUp PgDn=翻页对话区\n\
             End=回到底部 · Ctrl+T=展开/收起最近思考块\n\
         审批弹窗（任务运行中触发）: y=批准 n=拒绝 a=本会话全放行（新建/切换会话后复位） Esc=拒绝\n\
         退出: q（输入框为空时）或 Ctrl+C 连按两次\n\
         \n\
         首次启动未配置模型时，对话区显示欢迎引导：输入 /settings 即可\n\
         在对话内逐步完成配置（端点 → 模型名 → API 密钥（打码）→ 思考强度）。\n\
         \n\
         --selfcheck  用 TestBackend 渲染全部关键帧后退出（无 TTY 自检）"
    );
}

/// 还原终端（raw mode + 备用屏 + 光标 + 鼠标捕获）。幂等，任何退出路径可安全调用。
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        Show
    );
}

/// RAII 终端守卫：drop 即还原（覆盖提前 return 与 `?` 传播路径）。
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// 真终端 TUI 主流程（raw mode + 备用屏；任何退出路径无条件还原终端）。
fn run_tui() -> Result<(), String> {
    // 非 TTY 直接拒绝：管道/重定向下 Windows 的 enable_raw_mode 仍会“成功”，
    // UI 随后在无输入源的 poll→draw 空转里永不退出（实测挂死）。
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!(
            "sd-tui 需要真实终端（TTY）：stdin/stdout 须为终端，管道或重定向下无法运行交互界面"
        );
        std::process::exit(2);
    }
    enable_raw_mode().map_err(|e| format!("无法进入 raw mode（需要真实终端 TTY）: {e}"))?;
    // 鼠标捕获：滚轮翻页对话区（退出时 DisableMouseCapture 还原）。
    if let Err(e) = execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture, Hide) {
        let _ = disable_raw_mode();
        return Err(format!("进入备用屏失败: {e}"));
    }
    // panic 钩子：先还原终端，再走默认钩子把 panic 信息打回正常屏
    //（否则 panic 信息留在备用屏里，进程退出后用户看不到）。
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).map_err(|e| format!("终端初始化失败: {e}"))?;
    event_loop(&mut terminal).map_err(|e| format!("{e}"))
    // _guard drop：无条件还原终端。
}

/// UI 主循环：每帧先排空后台消息，再画帧，再以短超时收按键。
fn event_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    let root = std::env::current_dir()?;
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| io::Error::other(format!("tokio runtime 创建失败: {e}")))?;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(root, tx, runtime.handle().clone());

    loop {
        // 排空后台消息（每帧上限，防止高频事件饿死渲染）。
        for _ in 0..500 {
            match rx.try_recv() {
                Ok(msg) => app.on_msg(msg),
                Err(_) => break,
            }
        }

        terminal.draw(|f| ui::draw(f, &app))?;

        // 事件读取失败（EOF / 终端消失）：退出主循环，不留在循环里空转。
        let ev = match event::poll(Duration::from_millis(120)) {
            Ok(true) => match event::read() {
                Ok(ev) => Some(ev),
                Err(e) => return Err(e),
            },
            Ok(false) => None,
            Err(e) => return Err(e),
        };
        if let Some(TermEvent::Key(key)) = ev {
            // 只取按下/连发；Release（增强键盘模式）忽略。
            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                app.on_key(key);
            }
        } else if let Some(TermEvent::Mouse(mouse)) = ev {
            // 鼠标只管滚动：滚轮上滚=对话区上翻（历史），下滚=回到底部。
            match mouse.kind {
                MouseEventKind::ScrollUp => app.scroll_by(3),
                MouseEventKind::ScrollDown => app.scroll_by(-3),
                _ => {} // 拖选/点击不处理
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

/// --selfcheck：TestBackend 渲染全部关键帧（对话区 / 输入框 / 斜杠列表 / 审批弹窗 /
/// 引导卡片 / 会话列表 / 思考强度 / 流式思考块 / 思考收起与展开 / 流式正文 /
/// 滚轮提示行），逐帧断言后退出。不需要 TTY，证明 UI 帧可渲染、panic-free。
fn selfcheck() -> i32 {
    println!("sd-tui selfcheck");
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            println!("  [FAIL] tokio runtime 创建失败: {e}");
            return 1;
        }
    };
    let (tx, _rx) = std::sync::mpsc::channel();
    let root = std::env::current_dir().unwrap_or_default();
    let mut ok = true;

    // 帧 1：对话区（demo 对话流内容为面板独有文案）。
    let mut app = App::demo(root.clone(), tx.clone(), runtime.handle().clone());
    ok &= render_frame("对话区", &mut app, 120, 40, &["我先查看工作区结构"]);

    // 帧 2：输入框（输入内容为输入框独有文案）。
    app.input = "输入框标记文本".to_string();
    ok &= render_frame("输入框", &mut app, 120, 40, &["输入框标记文本"]);

    // 帧 3：斜杠命令列表（输 / 弹出；弹窗标题为弹窗独有文案）。
    app.input = "/".to_string();
    app.slash_dismissed = false;
    app.slash_sel = 0;
    ok &= render_frame("斜杠列表", &mut app, 120, 40, &["斜杠命令", "/doctor"]);

    // 帧 4：工具审批弹窗（标题 + 底部固定提示行）。
    app.input.clear();
    let (reply_tx, _reply_rx) = std::sync::mpsc::channel();
    app.approval_queue.push_back(app::PendingApproval {
        run_id: 1,
        request: sd_agent::policy::ApprovalRequest {
            tool_call_id: "call-demo".to_string(),
            tool: "bash".to_string(),
            summary: "bash: echo hi".to_string(),
            detail: r#"{"command":"echo hi"}"#.to_string(),
        },
        reply: reply_tx,
    });
    ok &= render_frame("审批弹窗", &mut app, 120, 40, &["工具审批待裁决", "y=批准"]);

    // 帧 5：首启欢迎引导卡片（模型未配置时的对话区卡片）。
    let mut app2 = App::demo(root.clone(), tx.clone(), runtime.handle().clone());
    app2.convo.clear();
    app2.show_guide();
    ok &= render_frame("引导卡片", &mut app2, 120, 40, &["当前未配置模型"]);

    // 帧 6：历史会话列表卡片（/conversations 输出 + 菜单态）。
    let mut app3 = App::demo(root.clone(), tx.clone(), runtime.handle().clone());
    app3.exec_command("/conversations", "");
    ok &= render_frame("会话列表", &mut app3, 120, 40, &["历史会话"]);

    // 帧 7：思考强度选择列表（引导态弹出层）。
    let mut app4 = App::demo(root.clone(), tx.clone(), runtime.handle().clone());
    app4.flow = Flow::EffortPick {
        label: "demo".to_string(),
        sel: 3,
    };
    ok &= render_frame("思考强度选择", &mut app4, 120, 40, &["选择思考强度"]);

    // 帧 8：流式思考块（思考中：暗色标头 + 已流出思考文本逐字追加）。
    let mut app5 = App::demo(root.clone(), tx.clone(), runtime.handle().clone());
    app5.convo.clear();
    app5.convo.push(app::ConvoItem::Streaming {
        run_id: 9,
        round: 1,
        reasoning: "先看目录结构再决定读哪个文件流式标记".to_string(),
        text: String::new(),
        thinking: true,
        tool_preview: None,
        expanded: true,
    });
    ok &= render_frame(
        "流式思考块",
        &mut app5,
        120,
        40,
        &["✦ 思考中", "先看目录结构再决定读哪个文件流式标记"],
    );

    // 帧 9：思考收起态 + 正文流出（固化条目：▸ 摘要行 + 白色正文）。
    let mut app6 = App::demo(root.clone(), tx.clone(), runtime.handle().clone());
    app6.convo.clear();
    app6.convo.push(app::ConvoItem::Assistant {
        text: "正文流式内容标记已固化".to_string(),
        reasoning: "这是一段收起的思考内容标记".to_string(),
        expanded: false,
    });
    ok &= render_frame(
        "思考收起态",
        &mut app6,
        120,
        40,
        &["▸ 思考", "正文流式内容标记已固化"],
    );

    // 帧 10：思考展开态（Ctrl+T 展开：暗色思考正文可见）。
    let mut app7 = App::demo(root.clone(), tx.clone(), runtime.handle().clone());
    app7.convo.clear();
    app7.convo.push(app::ConvoItem::Assistant {
        text: "正文内容".to_string(),
        reasoning: "展开的思考内容标记可见".to_string(),
        expanded: true,
    });
    ok &= render_frame(
        "思考展开态",
        &mut app7,
        120,
        40,
        &["展开的思考内容标记可见"],
    );

    // 帧 11：流式正文帧（转正文后思考收起、正文带 ▌ 光标逐字流出）。
    let mut app8 = App::demo(root.clone(), tx.clone(), runtime.handle().clone());
    app8.convo.clear();
    app8.convo.push(app::ConvoItem::Streaming {
        run_id: 9,
        round: 2,
        reasoning: "思考已结束标记".to_string(),
        text: "流式正文逐字流出标记".to_string(),
        thinking: false,
        tool_preview: Some(("bash".to_string(), r#"{"command":"ls"}"#.to_string())),
        expanded: false,
    });
    ok &= render_frame(
        "流式正文帧",
        &mut app8,
        120,
        40,
        &["流式正文逐字流出标记", "▸ 思考", "◆ 工具 bash"],
    );

    // 帧 12：滚轮翻页提示行（快捷提示含「滚轮」，键位与实际一致）。
    let mut app9 = App::demo(root.clone(), tx.clone(), runtime.handle().clone());
    app9.input.clear();
    ok &= render_frame("滚轮提示行", &mut app9, 120, 40, &["滚轮/PgUp PgDn 翻页"]);

    println!("selfcheck: {}", if ok { "OK" } else { "FAILED" });
    if ok { 0 } else { 1 }
}

/// 渲染一帧并断言标记（宽字符去空格后匹配；标记须为该帧独有文案）。
fn render_frame(name: &str, app: &mut App, width: u16, height: u16, markers: &[&str]) -> bool {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    match terminal.draw(|f| ui::draw(f, app)) {
        Ok(_) => {
            let text: String = buffer_text(terminal.backend().buffer(), width)
                .join("\n")
                .replace(' ', "");
            let mut frame_ok = true;
            for marker in markers {
                if text.contains(&marker.replace(' ', "")) {
                    println!("  [PASS] {name} 帧 ok（含标记「{marker}」）");
                } else {
                    println!("  [FAIL] {name} 帧缺少标记「{marker}」");
                    frame_ok = false;
                }
            }
            frame_ok
        }
        Err(e) => {
            println!("  [FAIL] {name} 帧渲染异常: {e}");
            false
        }
    }
}

/// TestBackend 缓冲 → 逐行文本（selfcheck 断言用）。
fn buffer_text(buffer: &ratatui::buffer::Buffer, width: u16) -> Vec<String> {
    let width = width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|cells| cells.iter().map(|c| c.symbol()).collect::<String>())
        .collect()
}
