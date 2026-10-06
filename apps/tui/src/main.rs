//! sd-tui 入口：参数解析、终端生命周期、UI 主事件循环、无头自检。
//!
//! 形态：双模式——hub 主页（默认）与 worker 会话执行（--worker）。
//! hub 输入回车 → 弹新终端跑 worker（双终端，观察面/工作面分离）。
//! 键位唯一出处 = keymap.rs；样式唯一出处 = ui/theme.rs。
//!
//! 性能：事件 poll 16ms，后台消息 try_recv 排空后统一重绘一帧
//! （帧合并，避免逐 delta 重绘卡顿）；空闲不重绘。
//!
//! 稳健：panic hook 恢复终端（不留残废终端），非 TTY 快速拒绝。

mod app;
mod bridge;
mod keymap;
mod run;
mod selfcheck;
mod slash;
mod stats;
mod term;
mod ui;

use std::io::{self, IsTerminal, Stdout, Write};
use std::path::PathBuf;
use std::time::Duration;

use crossterm::cursor::Show;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as TermEvent, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use app::{App, Focus, Mode};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("sd-tui {VERSION}");
        return;
    }
    if args.iter().any(|a| a == "--selfcheck") {
        std::process::exit(selfcheck::run());
    }

    let worker_mode = args.iter().any(|a| a == "--worker");
    let inbox: Option<PathBuf> = args
        .windows(2)
        .find(|w| w[0] == "--inbox")
        .map(|w| PathBuf::from(&w[1]));
    let session: Option<String> = args
        .windows(2)
        .find(|w| w[0] == "--session")
        .map(|w| w[1].clone());
    // 参数缺值显式报错（禁静默变 None 让 worker 空跑）。
    for flag in ["--inbox", "--session"] {
        if args.iter().any(|a| a == flag) {
            let has_value = args
                .windows(2)
                .any(|w| w[0] == flag && !w[1].starts_with("--"));
            if !has_value {
                eprintln!("sd-tui: {flag} 缺少参数值（用法见 --help）");
                std::process::exit(2);
            }
        }
    }

    // 非 TTY 快速拒绝（管道输入场景不挂死）。
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!("sd-tui 需要真实终端（TTY）才能运行。自检请用 --selfcheck。");
        std::process::exit(2);
    }

    let code = match run_tui(worker_mode, inbox, session) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("sd-tui 启动失败: {e}");
            1
        }
    };
    std::process::exit(code);
}

fn print_help() {
    println!(
        "sd-tui {VERSION} · sd-agent 双模式终端界面\n\
         \n\
         用法: sd-tui [--worker [--inbox <任务文件>] [--session <id>]] [--selfcheck] [--version]\n\
         \n\
         模式：\n\
           默认        hub 主页（大 Logo + 任务输入 + 历史会话 + Token 统计热力图）\n\
           --worker    会话执行（对话流 + 流式输出 + 工具审批）\n\
         \n\
         键位：\n\
         {}\
         \n\
         双终端：hub 回车/开会话 → 弹系统默认终端跑会话（WT/conhost；失败=状态栏提示，\n\
         任务文件/会话库保留可重试，hub 不内嵌降级）。\n\
         版本号口径：0.x 迭代期（规划见 docs/roadmap.md 版本号规划节）。",
        keymap::help_text()
    );
}

/// 终端生命周期 + 主事件循环。
fn run_tui(worker_mode: bool, inbox: Option<PathBuf>, session: Option<String>) -> io::Result<()> {
    install_panic_hook();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    terminal.clear()?;

    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| io::Error::other(e.to_string()))?;

    let mut app = if worker_mode {
        let mut app = App::new_worker(root, VERSION, rt, session);
        // inbox 任务：读取文件直接开跑（hub 跨进程交接的任务）。
        if let Some(path) = &inbox {
            if let Ok(task) = std::fs::read_to_string(path) {
                let task = task.trim().to_string();
                let _ = std::fs::remove_file(path);
                if !task.is_empty() {
                    app.start_inbox_task(task);
                }
            }
        }
        app
    } else {
        App::new_hub(root, VERSION, rt)
    };

    let result = event_loop(&mut app, &mut terminal);

    // 收尾：worker 会话存档 + 终端恢复。
    if app.is_worker() {
        app.archive_session();
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        Show
    )?;
    terminal.show_cursor()?;
    result
}

/// 主事件循环：poll 事件 + 排空后台消息 + 帧合并重绘。
fn event_loop(app: &mut App, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    let mut dirty = true;
    loop {
        // 1) 输入事件（16ms 窗；有事件就处理，无事件看后台消息）。
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                TermEvent::Key(key) if key.kind != KeyEventKind::Release => {
                    dirty |= handle_key(app, key);
                }
                TermEvent::Mouse(mouse) => {
                    // 鼠标全量语义在 app::mouse（点击/双击/滚轮/拖动/拖选）。
                    dirty |= app.handle_mouse(mouse);
                }
                TermEvent::Resize(..) => dirty = true,
                _ => {}
            }
        }

        // 2) 后台消息排空（帧合并：一批消息一帧）。
        dirty |= app.pump_msgs();

        // 3) 重绘（每帧把可交互区登记进 App，供鼠标命中消费）。
        if dirty {
            terminal.draw(|f| {
                let hits = ui::draw(app, f);
                app.set_hits(hits);
            })?;
            dirty = false;
        }

        if app.should_quit {
            break;
        }

        // worker 收尾检测：run 从 Some→None 时存档（观察面/事实源同步）。
        if app.take_run_just_finished() {
            app.archive_session();
            dirty = true;
        }
    }
    Ok(())
}

/// 键位分发：keymap 是键位唯一出处，这里只做动作执行接线。
fn handle_key(app: &mut App, key: event::KeyEvent) -> bool {
    // 审批弹窗：只认审批键。
    if let Mode::Worker(w) = &app.mode {
        if w.approval.is_some() {
            if let Some(action) = keymap::map_approval(key) {
                app.dispatch(action);
                return true;
            }
            return false;
        }
    }

    // 配置表单（模态）：只认表单键位（Tab 换字段 · Ctrl+S 保存 · Esc 取消）。
    if app.form_open() {
        if let Some(action) = keymap::map_form(key) {
            app.dispatch(action);
            return true;
        }
        return false;
    }

    // 模型选择浮层（模态）：只认浮层键位（↑↓ 选择 · Enter 确认 · Esc 取消）。
    if app.model_popup_open() {
        if let Some(action) = keymap::map_model_popup(key) {
            app.dispatch(action);
            return true;
        }
        return false;
    }

    // 帮助浮层：任意键关闭。
    if app.help_open {
        app.help_open = false;
        return true;
    }

    // 全局键位（含 Ctrl+C 确认退出 / ? / Tab）。
    if let Some(action) = keymap::map_global(key) {
        app.dispatch(action);
        return true;
    }

    let input_empty = app.input_text().is_empty();

    // 斜杠浮层开着：↑↓/PgUp/PgDn/Enter/Esc 归浮层（hub/worker 通用），
    // 其余键交回输入编辑（过滤继续）。
    if app.slash_open {
        if let Some(action) = keymap::map_slash(key) {
            app.dispatch(action);
            return true;
        }
    }

    // 模式键位（Enter→Send/Launch 经 submit_input 做斜杠路由；
    // hub Enter 按焦点分叉：输入框=新开任务 / 历史列表=打开会话）。
    let mode_action = if app.is_worker() {
        keymap::map_worker(key, input_empty)
    } else {
        keymap::map_hub(key, input_empty, app.focus == Focus::List)
    };
    if let Some(action) = mode_action {
        app.dispatch(action);
        return true;
    }

    // 输入框编辑键位（keymap 唯一出处）。
    if let Some(action) = keymap::map_edit(key) {
        app.dispatch(action);
        return true;
    }
    false
}

/// panic hook：panic 也恢复终端（不留残废终端），错误照常打印。
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let mut out = io::stdout();
        let _ = execute!(out, LeaveAlternateScreen, DisableMouseCapture, Show);
        let _ = out.flush();
        original(info);
    }));
}

/// 焦点标签（状态栏微调预留）。
#[allow(dead_code)]
fn focus_label(f: Focus) -> &'static str {
    match f {
        Focus::Input => "输入",
        Focus::List => "列表",
    }
}
