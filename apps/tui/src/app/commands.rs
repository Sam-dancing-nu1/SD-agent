//! 执行落点（App 侧）：斜杠命令、任务提交/开会话、配置落盘。
//!
//! 从 app/mod.rs 拆出（≤500 行/文件纪律）：状态在 mod.rs，动作落盘在本文件；
//! 键位在 keymap，鼠标在 mouse.rs。落盘统一走核心 Settings
//!（upsert_profile + set_active + save），禁旁路写配置文件。

use sd_agent::config::settings::{ModelProfileCfg, Settings};
use sd_agent::model::ChatMessage;
use sd_agent::session::SessionStore;

use super::worker::FeedItem;
use super::{App, Mode};
use crate::slash;

impl App {
    /// 斜杠命令执行（输入以 / 开头的整行）。
    pub fn exec_command(&mut self, line: &str) {
        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("").trim().to_string();
        let arg = parts.next().unwrap_or("").trim().to_string();
        match cmd.as_str() {
            "/help" => {
                self.note(slash::help_text());
                self.note(format!("键位：\n{}", crate::keymap::help_text()));
            }
            "/version" => {
                self.note(format!(
                    "sd-tui {} · 运行时底座（0.x 迭代期）",
                    self.version
                ));
            }
            "/clear" => {
                if let Mode::Worker(w) = &mut self.mode {
                    w.feed.clear();
                    w.saved_count = 0;
                }
            }
            "/retry" => self.retry(),
            "/new" => {
                // 声明即事实：当前会话先存档，再重建（feed/session 全新）。
                self.archive_session();
                self.dispatch(crate::keymap::Action::NewSession);
            }
            "/doctor" => {
                let handle = self.rt.handle().clone();
                crate::run::spawn_doctor(&handle, self.root.clone(), self.tx.clone());
            }
            "/stats" => {
                let stats = crate::stats::Stats::scan(&self.root);
                self.note(format!(
                    "累计 token：{}（prompt {} / completion {}）· run {} 次 · 轨迹 {} 份\n缓存命中：端点未返回缓存字段（如实显示）",
                    stats.total_tokens(),
                    stats.total_prompt(),
                    stats.total_completion(),
                    stats.total_runs,
                    stats.trace_files,
                ));
            }
            "/effort" => {
                if let Some(idx) = slash::EFFORTS.iter().position(|e| *e == arg) {
                    // 与滑条/←→ 同一落盘路径（upsert + save）。
                    self.set_effort_idx(idx);
                } else {
                    self.note(format!("用法：/effort <{}>", slash::EFFORTS.join("|")));
                }
            }
            "/model" | "/settings" | "/conversations" | "/trace" => {
                self.note(format!(
                    "{cmd} 面板在本 demo 为文本态：会话/配置请用 hub 历史列表与 /effort、/doctor（图形面板 [待磨合]）"
                ));
            }
            _ => self.note(format!("未知命令 {cmd}（/help 看清单）")),
        }
    }

    /// 追加一条灰字注记到当前模式（worker 对话流 / hub 状态栏）。
    pub(crate) fn note(&mut self, text: String) {
        match &mut self.mode {
            Mode::Worker(w) => w.push(FeedItem::Note(text)),
            Mode::Hub(h) => h.status = text,
        }
    }

    // ── 任务提交 / 会话打开（自 mod.rs 迁入，行为不变） ──

    /// hub 提交：任务落盘到 inbox，弹新终端跑 worker；弹失败 = 状态栏报错
    /// 提示，任务文件保留在 .sd-agent/inbox/ 可重试（不内嵌降级）。
    pub(crate) fn launch_from_hub(&mut self) {
        let task = match &mut self.mode {
            Mode::Hub(h) => {
                let t = h.editor.submit();
                if t.trim().is_empty() {
                    return;
                }
                h.launched = true;
                t
            }
            Mode::Worker(_) => return,
        };
        // 任务交接走文件（跨进程，免转义）：<root>/.sd-agent/inbox/<日期>-<pid>-<毫秒>.txt
        // （毫秒+pid 双因子防同进程同日覆盖串号）。
        let inbox = self.root.join(".sd-agent").join("inbox");
        let _ = std::fs::create_dir_all(&inbox);
        let ts = crate::stats::today_string();
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let path = inbox.join(format!("{ts}-{}-{ms}.txt", std::process::id()));
        if let Err(e) = std::fs::write(&path, &task) {
            self.note(format!("任务文件写入失败: {e}"));
            return;
        }
        let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("sd-tui"));
        match crate::term::spawn_worker(&exe, &path) {
            Ok(()) => {
                // 动画触发点（拍板）：launch 成功 = anim.start(ToWork)。
                if let Mode::Hub(h) = &mut self.mode {
                    h.anim.start(super::anim::AnimKind::ToWork);
                }
                self.note(format!(
                    "已请求弹出新终端执行（任务：{}…）；若窗口未出现，稍后自动可重试",
                    clip_chars(&task, 24)
                ));
                if let Mode::Hub(h) = &mut self.mode {
                    h.refresh(&self.root);
                }
            }
            Err(e) => {
                // 进程模型铁律：hub 绝不原地变身 worker。任务文件保留可重试。
                self.note(format!(
                    "新终端拉起失败（{e}），任务文件保留在 .sd-agent/inbox/，可重试"
                ));
            }
        }
    }

    /// worker 发送：开 run。
    pub(crate) fn send_from_worker(&mut self) {
        let task = match &mut self.mode {
            Mode::Worker(w) => {
                if w.running.is_some() {
                    w.push(FeedItem::Note("上一条任务还在跑（Esc 取消渲染）".into()));
                    return;
                }
                let t = w.editor.submit();
                if t.trim().is_empty() {
                    return;
                }
                t
            }
            Mode::Hub(_) => return,
        };
        self.start_run(task);
    }

    /// 开一次 run（worker 专用）。
    pub(crate) fn start_run(&mut self, task: String) {
        let Mode::Worker(w) = &mut self.mode else {
            return;
        };
        let run_id = w.alloc_run_id();
        w.last_task = Some(task.clone());
        w.push(FeedItem::User(task.clone()));
        // 历史（会话存档 → ChatMessage）：未恢复会话则空。
        let history = w
            .session_id
            .as_ref()
            .and_then(|id| SessionStore::new(&self.root).load(id).ok().flatten())
            .map(|s| {
                s.messages
                    .iter()
                    .filter_map(|m| {
                        // 只回放 user/assistant 对话正文；tool 消息不回放
                        //（tool_result 需与 assistant.tool_calls 配对，缺失即协议
                        // 非法；工具留痕在轨迹文件，续跑上下文不含工具中间态）。
                        let mut msg = match m.role.as_str() {
                            "user" => ChatMessage::user(m.text.clone()),
                            "assistant" => ChatMessage::assistant(m.text.clone(), vec![]),
                            _ => return None,
                        };
                        if let Some(r) = &m.reasoning_content {
                            msg = msg.with_reasoning_content(r.clone());
                        }
                        Some(msg)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let max_rounds = std::env::var("SD_AGENT_MAX_ROUNDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);
        w.running = Some(run_id);
        w.cancelled = false;
        crate::run::spawn_run(
            self.root.clone(),
            run_id,
            task,
            history,
            max_rounds,
            self.tx.clone(),
            w.allow_all.clone(),
        );
    }

    /// R：重试上一条任务（失败恢复能力）。
    pub(crate) fn retry(&mut self) {
        let task = match &mut self.mode {
            Mode::Worker(w) => {
                if w.running.is_some() {
                    w.push(FeedItem::Note("还在跑，等收尾再重试".into()));
                    return;
                }
                match w.last_task.clone() {
                    Some(t) => t,
                    None => {
                        w.push(FeedItem::Note("没有可重试的任务".into()));
                        return;
                    }
                }
            }
            Mode::Hub(_) => return,
        };
        self.start_run(task);
    }

    /// hub 打开选中历史会话：弹新终端跑 `sd-tui --worker --session <id>`
    ///（一个 worker 窗口单 WORK），hub 原地不动只刷新历史。弹失败 = 状态栏
    /// 报错提示，会话库保留可重试——绝不原地切 Mode::Worker。
    pub(crate) fn open_selected_session(&mut self) {
        let (id, title) = match &self.mode {
            Mode::Hub(h) => match h.selected_id() {
                Some(id) => {
                    let t = h
                        .sessions
                        .get(h.selected)
                        .map(|s| s.title.clone())
                        .unwrap_or_default();
                    (id.to_string(), t)
                }
                None => return,
            },
            Mode::Worker(_) => return,
        };
        let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("sd-tui"));
        match crate::term::spawn_worker_session(&exe, &id) {
            Ok(()) => {
                self.note(format!(
                    "已在新终端打开会话：{title}（sd-tui --worker --session {id}）"
                ));
                if let Mode::Hub(h) = &mut self.mode {
                    h.refresh(&self.root);
                }
            }
            Err(e) => {
                self.note(format!(
                    "新终端拉起失败（{e}），会话 {id} 保留在会话库，可重试"
                ));
            }
        }
    }

    // ── 配置状态机落盘（滑条 / 模型浮层 / 表单共用一条 Settings 路径） ──

    /// 思考强度改档 + 同步落盘（当前激活 profile 的 reasoning_effort）。
    /// hub 同步写 HubState.effort_idx（滑条显示）；档位没变不重复落盘。
    pub fn set_effort_idx(&mut self, idx: usize) {
        let idx = idx.min(slash::EFFORTS.len().saturating_sub(1));
        if let Mode::Hub(h) = &mut self.mode {
            if !h.set_effort(idx) {
                return;
            }
        }
        self.persist_effort(idx);
    }

    /// 落盘思考强度（upsert_profile + set_active + save；与 /effort 同路径）。
    fn persist_effort(&mut self, idx: usize) {
        let name = slash::EFFORTS.get(idx).copied().unwrap_or("medium");
        let mut settings = Settings::load();
        match settings.active() {
            Some(active) => {
                let mut p = active.clone();
                p.reasoning_effort = name.to_string();
                let label = p.label.clone();
                settings.upsert_profile(p);
                settings.set_active(&label);
                match settings.save() {
                    Ok(path) => self.note(format!("思考强度已改 {name} → {}", path.display())),
                    Err(e) => self.note(format!("保存失败: {e}")),
                }
            }
            None => self.note("没有激活的模型配置（M 或 + 新增）".into()),
        }
    }

    /// 模型浮层确认：set_active + save 落盘，再刷新选择器快照。
    pub fn confirm_model_popup(&mut self) {
        let label = match &self.mode {
            Mode::Hub(h) => h.selected_label().map(|s| s.to_string()),
            Mode::Worker(_) => return,
        };
        if let Mode::Hub(h) = &mut self.mode {
            h.model_open = false;
        }
        let Some(label) = label else {
            self.note("占位行不可启用（先 + 添加配置）".into());
            return;
        };
        let mut settings = Settings::load();
        settings.set_active(&label);
        match settings.save() {
            Ok(path) => self.note(format!("已切换模型配置：{label} → {}", path.display())),
            Err(e) => self.note(format!("保存失败: {e}")),
        }
        if let Mode::Hub(h) = &mut self.mode {
            h.reload_profiles();
        }
    }

    /// 表单保存：校验 → upsert_profile + set_active + save → reload_profiles。
    /// 校验失败留在表单上显示 error；api_key 留空 = 不改既有值。
    pub fn submit_form(&mut self) {
        let snap = {
            let Some(f) = self.form_mut() else {
                return;
            };
            if let Err(e) = f.validate() {
                f.error = Some(e);
                return;
            }
            f.error = None;
            (
                f.label.trim().to_string(),
                f.base_url.trim().to_string(),
                f.model.trim().to_string(),
                f.api_key.trim().to_string(),
                f.effort_name().to_string(),
                f.edit_label.clone(),
            )
        };
        let (label, base_url, model, api_key, effort_name, edit_label) = snap;
        let mut settings = Settings::load();
        // 密钥：填了用填的；留空沿用原配置（编辑改名也按原 label 找）。
        let key = if api_key.is_empty() {
            edit_label
                .as_deref()
                .or(Some(label.as_str()))
                .and_then(|l| settings.profiles.iter().find(|p| p.label == l))
                .and_then(|p| p.api_key.clone())
        } else {
            Some(api_key)
        };
        let p = ModelProfileCfg {
            label: label.clone(),
            base_url,
            model,
            api_key: key,
            reasoning_effort: effort_name,
        };
        // 编辑改名：旧 label 行先移除（label 唯一性由 upsert 保证，改名=换行）。
        if let Some(old) = &edit_label {
            if *old != label {
                settings.remove_profile(old);
            }
        }
        settings.upsert_profile(p);
        settings.set_active(&label);
        match settings.save() {
            Ok(path) => {
                self.note(format!("配置 {label} 已保存并启用 → {}", path.display()));
                if let Mode::Hub(h) = &mut self.mode {
                    h.form = None;
                    h.reload_profiles();
                }
            }
            Err(e) => {
                let msg = format!("保存失败: {e}");
                if let Some(f) = self.form_mut() {
                    f.error = Some(msg);
                }
            }
        }
    }
}

/// 截短展示（按字符，防长任务刷屏状态栏）。
fn clip_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
