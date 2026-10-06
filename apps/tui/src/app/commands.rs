//! 斜杠命令执行（App 侧落点）：命令表在 crate::slash，键位在 keymap。
//!
//! 从 app/mod.rs 拆出（≤500 行/文件纪律）。

use sd_agent::config::settings::Settings;

use super::App;
use super::worker::FeedItem;
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
                if let super::Mode::Worker(w) = &mut self.mode {
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
                if slash::EFFORTS.contains(&arg.as_str()) {
                    let mut settings = Settings::load();
                    match settings.active() {
                        Some(active) => {
                            let mut p = active.clone();
                            p.reasoning_effort = arg.clone();
                            let label = p.label.clone();
                            settings.upsert_profile(p);
                            settings.set_active(&label);
                            match settings.save() {
                                Ok(path) => {
                                    self.note(format!("思考强度已改 {arg} → {}", path.display()))
                                }
                                Err(e) => self.note(format!("保存失败: {e}")),
                            }
                        }
                        None => self.note("没有激活的模型配置（先 /settings）".into()),
                    }
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
            super::Mode::Worker(w) => w.push(FeedItem::Note(text)),
            super::Mode::Hub(h) => h.status = text,
        }
    }
}
