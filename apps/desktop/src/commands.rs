//! Tauri command 层：前端 invoke 的唯一入口（薄壳）。
//!
//! 业务逻辑全部在核心 lib：run_task / doctor::run_all / JsonlSource /
//! Settings（配置链）/ SessionStore（会话持久化）；本层只做 DTO 转换、
//! 文件名校验与共享状态转发。
//!
//! 前端 wire 契约（字段名严格固定，密钥明文永不回传）：
//! - get_settings() -> {profiles:[{label,base_url,model,has_api_key,
//!                      reasoning_effort}], active_profile, max_rounds, settings_path}
//! - save_profiles(profiles, active_profile, max_rounds) -> {ok, path, error?}
//! - switch_profile(label) -> {ok, error?}
//! - list_sessions() -> [{id,title,updated_at_ms,message_count}]
//! - create_session(title) -> {id,title}
//! - load_session(id) -> {id,title,messages:[{role,text,ts_unix_ms}]}
//! - delete_session(id) -> {ok}
//! - start_task(task, session_id) -> {ok, run_id?, error?}
//! - test_connection() -> {ok, model, message}
//!
//! 流式 wire 契约（Tauri 事件，run_id 关联）：
//! - stream_delta {run_id, round, kind: "reasoning"|"text", delta}
//! - stream_tool_call {run_id, round, name, args_so_far}
//! - stream_turn_done {run_id, round}
//! - model_message {run_id, round, text, tool_calls}（一轮收尾的完整形态）

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use sd_agent::config::settings::{ModelProfileCfg, Settings};
use sd_agent::event::{EventSource, JsonlSource};
use sd_agent::model::{ChatMessage, ChatRequest, ModelClient, OpenAiCompatClient};
use sd_agent::session::{SessionMeta, SessionStore};

use crate::runner;
use crate::state::{self, AppState, RunInfo};

/// 轨迹文件列表条目。
#[derive(Debug, Clone, Serialize)]
pub struct TraceFileDto {
    pub name: String,
    pub size_bytes: u64,
    pub mtime_ms: u64,
}

/// 一份轨迹回放产物（事件 wire 形态 + 读侧警告）。
#[derive(Debug, Clone, Serialize)]
pub struct TraceDto {
    pub name: String,
    pub events: Vec<serde_json::Value>,
    pub warnings: Vec<String>,
}

/// 单套模型配置（前端契约固定字段；只回 has_api_key 布尔位，无密钥内容）。
#[derive(Debug, Clone, Serialize)]
pub struct ProfileDto {
    pub label: String,
    pub base_url: String,
    pub model: String,
    pub has_api_key: bool,
    pub reasoning_effort: String,
}

/// get_settings 返回（前端契约固定字段）。
#[derive(Debug, Clone, Serialize)]
pub struct SettingsDto {
    pub profiles: Vec<ProfileDto>,
    pub active_profile: String,
    pub max_rounds: u32,
    pub settings_path: String,
}

/// save_profiles 的单条配置入参。api_key=None 表示该配置密钥不变。
#[derive(Debug, Clone, Deserialize)]
pub struct ProfilePayload {
    pub label: String,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub reasoning_effort: String,
}

/// save_profiles 返回（{ok, path, error?}；path 恒为配置文件目标路径）。
#[derive(Debug, Clone, Serialize)]
pub struct SaveOutcome {
    pub ok: bool,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SaveOutcome {
    fn err(msg: &str) -> Self {
        Self {
            ok: false,
            path: sd_agent::sys::normalize_display(&Settings::settings_path()),
            error: Some(msg.to_string()),
        }
    }
}

/// 纯确认返回（{ok, error?}）。
#[derive(Debug, Clone, Serialize)]
pub struct OkOutcome {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl OkOutcome {
    fn err(msg: &str) -> Self {
        Self {
            ok: false,
            error: Some(msg.to_string()),
        }
    }
}

/// 会话列表条目（前端契约固定字段）。
#[derive(Debug, Clone, Serialize)]
pub struct SessionDto {
    pub id: String,
    pub title: String,
    pub updated_at_ms: u64,
    pub message_count: u32,
}

/// 会话消息（前端契约固定字段）。
#[derive(Debug, Clone, Serialize)]
pub struct SessionMessageDto {
    pub role: String,
    pub text: String,
    pub ts_unix_ms: u64,
}

/// 会话详情（含全部消息）。
#[derive(Debug, Clone, Serialize)]
pub struct SessionDetailDto {
    pub id: String,
    pub title: String,
    pub messages: Vec<SessionMessageDto>,
}

/// 新建会话返回。
#[derive(Debug, Clone, Serialize)]
pub struct SessionCreatedDto {
    pub id: String,
    pub title: String,
}

/// doctor 体检项（全中文字段）。
#[derive(Debug, Clone, Serialize)]
pub struct DoctorItemDto {
    pub name: String,
    pub title: String,
    pub detail: String,
    pub hint: String,
    pub fix: String,
    pub ok: bool,
}

/// 测试连接返回（前端契约固定字段）：ok + 中文 message + 成功时的 model。
#[derive(Debug, Clone, Serialize)]
pub struct TestConnectionDto {
    pub ok: bool,
    pub model: String,
    pub message: String,
}

impl TestConnectionDto {
    fn fail(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            model: String::new(),
            message: msg.into(),
        }
    }
}

/// start_task 返回（{ok, run_id?, error?}）：run_id 只在成功时给出，
/// 前端用它把 run_update / agent_event / stream_* 精确路由回发起会话
///（缺 run_id 只能猜当前会话，切走会话就会串台）。
#[derive(Debug, Clone, Serialize)]
pub struct TaskOutcome {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl TaskOutcome {
    fn err(msg: &str) -> Self {
        Self {
            ok: false,
            run_id: None,
            error: Some(msg.to_string()),
        }
    }
}

// —— 模型配置链 commands ——

/// 读取模型配置链（多配置 + 激活项 + 轮数上限）。
/// 密钥只回显 has_api_key 布尔位，明文永不回传。
#[tauri::command(rename_all = "snake_case")]
pub fn get_settings() -> SettingsDto {
    let settings = Settings::load();
    SettingsDto {
        profiles: settings
            .profiles
            .iter()
            .map(|p| ProfileDto {
                label: p.label.clone(),
                base_url: p.base_url.clone(),
                model: p.model.clone(),
                has_api_key: p.has_api_key(),
                reasoning_effort: p.reasoning_effort.clone(),
            })
            .collect(),
        active_profile: settings.active_profile.clone(),
        max_rounds: settings.max_rounds,
        settings_path: sd_agent::sys::normalize_display(&Settings::settings_path()),
    }
}

/// 整体保存模型配置（多配置 + 激活项 + 轮数上限），一次落盘 Settings::save。
/// 每条配置 api_key=None 或空白串表示密钥不变（沿用旧值，防误清），
/// Some(非空) 才覆盖。
#[tauri::command(rename_all = "snake_case")]
pub fn save_profiles(
    profiles: Vec<ProfilePayload>,
    active_profile: String,
    max_rounds: u32,
) -> SaveOutcome {
    match build_settings(&Settings::load(), &profiles, &active_profile, max_rounds) {
        Ok(next) => match next.save() {
            Ok(path) => SaveOutcome {
                ok: true,
                path: sd_agent::sys::normalize_display(&path),
                error: None,
            },
            Err(e) => SaveOutcome::err(&format!("配置写入失败：{e}")),
        },
        Err(msg) => SaveOutcome::err(&msg),
    }
}

/// 入参 → 持久化形态（纯函数，测试直击）：校验 + 密钥合并 + 归一。
/// 校验失败返回中文错误（点名到具体配置）。
fn build_settings(
    current: &Settings,
    profiles: &[ProfilePayload],
    active_profile: &str,
    max_rounds: u32,
) -> Result<Settings, String> {
    if profiles.is_empty() {
        return Err("至少需要一个模型配置".to_string());
    }
    let active_profile = active_profile.trim();
    if active_profile.is_empty() {
        return Err("请选择启用的模型配置".to_string());
    }
    let mut next = Settings {
        profiles: Vec::new(),
        active_profile: active_profile.to_string(),
        max_rounds: max_rounds.clamp(1, Settings::MAX_ROUNDS_CAP),
        // 工具轮思考开关不在本命令入参里，沿用当前值（防保存时误重置）。
        thinking_on_tools: current.thinking_on_tools,
    };
    for p in profiles {
        let label = p.label.trim().to_string();
        if label.is_empty() {
            return Err("配置名不能为空".to_string());
        }
        if next.profiles.iter().any(|x| x.label == label) {
            return Err(format!("配置名重复：{label}"));
        }
        if p.base_url.trim().is_empty() {
            return Err(format!("配置「{label}」的服务地址不能为空"));
        }
        if p.model.trim().is_empty() {
            return Err(format!("配置「{label}」的模型名不能为空"));
        }
        // 密钥合并：None / 空白串 = 不变（沿用旧值），Some(非空) = 覆盖。
        let api_key = match p.api_key.as_deref().map(str::trim) {
            Some(k) if !k.is_empty() => Some(k.to_string()),
            _ => current
                .profiles
                .iter()
                .find(|x| x.label == label)
                .and_then(|x| x.api_key.clone()),
        };
        next.upsert_profile(ModelProfileCfg {
            label,
            base_url: p.base_url.trim().to_string(),
            model: p.model.trim().to_string(),
            api_key,
            reasoning_effort: p.reasoning_effort.trim().to_string(),
        });
    }
    if !next.profiles.iter().any(|x| x.label == next.active_profile) {
        return Err(format!(
            "启用配置「{}」不存在，请选择已有配置",
            next.active_profile
        ));
    }
    Ok(next)
}

/// 切换当前启用的模型配置（label 必须已存在）。
#[tauri::command(rename_all = "snake_case")]
pub fn switch_profile(label: String) -> OkOutcome {
    let label = label.trim();
    let mut settings = Settings::load();
    if !settings.profiles.iter().any(|p| p.label == label) {
        return OkOutcome::err(&format!("模型配置「{label}」不存在"));
    }
    settings.set_active(label);
    match settings.save() {
        Ok(_) => OkOutcome {
            ok: true,
            error: None,
        },
        Err(e) => OkOutcome::err(&format!("配置写入失败：{e}")),
    }
}

/// 起跑前的模型配置就绪检查（start_task 的唯一判定口，测试共用）。
/// 不就绪返回中文错误引导设置向导；错误里不出现任何环境变量名。
pub fn ensure_runnable(settings: &Settings) -> Result<(), String> {
    let missing = settings.missing_fields();
    if missing.is_empty() {
        return Ok(());
    }
    let names: Vec<&str> = missing.iter().map(|f| missing_label(f)).collect();
    Err(format!(
        "模型配置不完整，缺少：{}。请在「设置向导」中补全后再启动任务",
        names.join("、")
    ))
}

/// 缺失字段标识 → 中文展示名。
fn missing_label(field: &str) -> &str {
    match field {
        "profiles" => "模型配置（一套都没有）",
        "active_profile" => "启用配置（当前指向的配置不存在）",
        "base_url" => "服务地址",
        "model" => "模型名",
        "api_key" => "API 密钥",
        other => other,
    }
}

// —— 会话 commands ——

/// 会话列表（updated_at 倒序；坏文件由核心层跳过）。
#[tauri::command(rename_all = "snake_case")]
pub fn list_sessions(state: State<'_, AppState>) -> Vec<SessionDto> {
    SessionStore::new(&state.root)
        .list()
        .into_iter()
        .map(session_dto)
        .collect()
}

fn session_dto(meta: SessionMeta) -> SessionDto {
    SessionDto {
        id: meta.id,
        title: meta.title,
        updated_at_ms: meta.updated_at_ms,
        message_count: meta.message_count as u32,
    }
}

/// 新建会话（标题为空时用默认标题）。
#[tauri::command(rename_all = "snake_case")]
pub fn create_session(
    state: State<'_, AppState>,
    title: String,
) -> Result<SessionCreatedDto, String> {
    let trimmed = title.trim();
    let title = if trimmed.is_empty() { "新会话" } else { trimmed };
    let session = SessionStore::new(&state.root)
        .create(title)
        .map_err(|e| format!("会话创建失败：{e}"))?;
    Ok(SessionCreatedDto {
        id: session.id,
        title: session.title,
    })
}

/// 读一个会话（含全部消息）。
#[tauri::command(rename_all = "snake_case")]
pub fn load_session(state: State<'_, AppState>, id: String) -> Result<SessionDetailDto, String> {
    let session = SessionStore::new(&state.root)
        .load(id.trim())
        .map_err(|e| format!("会话读取失败：{e}"))?
        .ok_or_else(|| "会话不存在或已删除".to_string())?;
    Ok(SessionDetailDto {
        id: session.id,
        title: session.title,
        messages: session
            .messages
            .into_iter()
            .map(|m| SessionMessageDto {
                role: m.role,
                text: m.text,
                ts_unix_ms: m.ts_unix_ms,
            })
            .collect(),
    })
}

/// 删除会话（幂等：不存在也算成功）。
#[tauri::command(rename_all = "snake_case")]
pub fn delete_session(state: State<'_, AppState>, id: String) -> Result<OkOutcome, String> {
    SessionStore::new(&state.root)
        .delete(id.trim())
        .map_err(|e| format!("会话删除失败：{e}"))?;
    Ok(OkOutcome {
        ok: true,
        error: None,
    })
}

// —— 任务 / 体检 / 轨迹 commands ——

/// 启动任务并持续写入会话（任务=用户消息，模型回复=助手消息）。
/// 前置校验（任务非空 / 会话存在 / 模型配置完整）不通过时返回中文错误
/// 引导设置向导，不留僵尸 run。同步 command（无 await，装配即返回），
/// 回 {ok, run_id?, error?}；run_id 供前端把流式事件路由回发起会话。
#[tauri::command(rename_all = "snake_case")]
pub fn start_task(
    app: AppHandle,
    state: State<'_, AppState>,
    task: String,
    session_id: String,
) -> TaskOutcome {
    let task = task.trim().to_string();
    if task.is_empty() {
        return TaskOutcome::err("任务内容不能为空");
    }
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return TaskOutcome::err("会话 id 不能为空，请先创建会话");
    }
    // 会话必须已存在（append 语义是往既有会话追加）。
    match SessionStore::new(&state.root).load(&session_id) {
        Ok(Some(_)) => {}
        Ok(None) => return TaskOutcome::err("会话不存在或已删除，请重新创建会话"),
        Err(e) => return TaskOutcome::err(&format!("会话读取失败：{e}")),
    }
    // 模型配置链闭环：起跑前快速失败（中文错误引导设置向导，不是 missing env）。
    let settings = Settings::load();
    if let Err(msg) = ensure_runnable(&settings) {
        return TaskOutcome::err(&msg);
    }
    let name = state::derive_name(&task, 32);
    match runner::spawn_run(app, &state, name, task, session_id) {
        Ok(info) => TaskOutcome {
            ok: true,
            run_id: Some(info.id),
            error: None,
        },
        Err(e) => TaskOutcome::err(&e),
    }
}

/// 任务列表（侧栏轮询/刷新用）。
#[tauri::command(rename_all = "snake_case")]
pub fn list_tasks(state: State<'_, AppState>) -> Vec<RunInfo> {
    state.snapshot()
}

/// 回传一次工具审批裁决（弹窗 → DesktopApproval 桥）。
#[tauri::command(rename_all = "snake_case")]
pub fn resolve_approval(
    state: State<'_, AppState>,
    tool_call_id: String,
    decision: String,
) -> Result<(), String> {
    state.resolve(&tool_call_id, &decision)
}

/// doctor 体检（含一次最小模型调用，可能耗时数秒）。全中文字段。
#[tauri::command(rename_all = "snake_case")]
pub async fn run_doctor(state: State<'_, AppState>) -> Result<Vec<DoctorItemDto>, String> {
    let root = state.root.clone();
    let report = sd_agent::doctor::run_all(&root).await;
    Ok(report
        .items
        .into_iter()
        .map(|i| DoctorItemDto {
            name: i.name.to_string(),
            title: i.title,
            detail: i.detail,
            hint: i.hint,
            fix: i.fix,
            ok: i.ok,
        })
        .collect())
}

/// 测试连接：对当前启用配置发一次最小模型调用，回答“连不连得上”。
///
/// 与 doctor 体检分离的两个理由：
/// 1. 不闪黑色控制台窗——本命令零子进程（doctor 的 rustc 等探针会 spawn
///    控制台子进程，GUI 进程里会闪 conhost 窗，那是核心 doctor 的待修项）；
/// 2. 结果直白——只答“连接成功，模型：xxx”或具体失败原因，不吐体检清单。
///
/// 计费敏感：每次测试都真实调用一次模型，壳层强制 30 秒冷却
///（state::AppState::try_begin_test），冷却中直接拒绝，防连打烧额度。
#[tauri::command(rename_all = "snake_case")]
pub async fn test_connection(state: State<'_, AppState>) -> Result<TestConnectionDto, String> {
    if let Err(wait_s) = state.try_begin_test() {
        return Ok(TestConnectionDto::fail(format!(
            "测试太频繁：请 {wait_s} 秒后再试（每次测试都会真实调用一次模型，防止误触烧额度）"
        )));
    }
    let settings = Settings::load();
    if let Err(msg) = ensure_runnable(&settings) {
        return Ok(TestConnectionDto::fail(msg));
    }
    let client = match OpenAiCompatClient::from_settings(&settings) {
        Ok(c) => c,
        Err(e) => return Ok(TestConnectionDto::fail(e.to_string())),
    };
    let request = ChatRequest {
        messages: vec![
            ChatMessage::system("你是连通性探针，只回答一个词。"),
            ChatMessage::user("请只回复一个词：pong"),
        ],
        tools: vec![],
        // 探针请求不进流式轮次（非流式路径不使用 round）。
        round: 0,
    };
    match client.chat(request).await {
        Ok(_) => {
            let model = client.model_name().to_string();
            Ok(TestConnectionDto {
                ok: true,
                model: model.clone(),
                message: format!("连接成功，模型：{model}"),
            })
        }
        Err(e) => Ok(TestConnectionDto::fail(e.to_string())),
    }
}

/// 轨迹文件列表（.sd-agent/traces/*.jsonl，按修改时间倒序）。
#[tauri::command(rename_all = "snake_case")]
pub fn list_traces(state: State<'_, AppState>) -> Vec<TraceFileDto> {
    let dir = state.traces_dir();
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let meta = entry.metadata().ok();
            let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime_ms = meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            out.push(TraceFileDto {
                name,
                size_bytes,
                mtime_ms,
            });
        }
    }
    out.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms));
    out
}

/// 轨迹文件名白名单：只收纯文件名，防目录穿越与盘符相对路径绕过。
///
/// 三层校验（缺一不可）：
/// 1. 字符集白名单 [A-Za-z0-9._-]，并显式拒 ':'（盘符 / NTFS 流）；
/// 2. 逐段检查路径组件：必须恰为单个普通组件（拒 `..`、`.`、盘符前缀、
///    分隔符等一切非常规组件）——不用 `contains("..")`，避免误杀 `a..b.jsonl`；
/// 3. 必须以 `.jsonl` 结尾。
fn is_safe_trace_name(file: &str) -> bool {
    if file.is_empty() || !file.ends_with(".jsonl") {
        return false;
    }
    if file.contains(':') {
        return false;
    }
    if !file
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return false;
    }
    let mut comps = std::path::Path::new(file).components();
    matches!(
        (comps.next(), comps.next()),
        (Some(std::path::Component::Normal(_)), None)
    )
}

/// 轨迹文件名 → traces 目录内路径。双保险：白名单 + join 后父目录断言
/// （防 `PathBuf::join` 遇盘符前缀整体替换路径）。
fn resolve_trace_path(dir: &std::path::Path, file: &str) -> Result<std::path::PathBuf, String> {
    if !is_safe_trace_name(file) {
        return Err("非法轨迹文件名".to_string());
    }
    let path = dir.join(file);
    if path.parent() != Some(dir) {
        return Err("非法轨迹文件名".to_string());
    }
    Ok(path)
}

/// 读一份轨迹（JsonlSource 回放，容忍末尾半行）。
/// 文件名只收纯名（白名单 + 路径组件逐段校验），防目录穿越。
#[tauri::command(rename_all = "snake_case")]
pub fn get_trace_events(state: State<'_, AppState>, file: String) -> Result<TraceDto, String> {
    let dir = state.traces_dir();
    let path = resolve_trace_path(&dir, &file)?;
    let outcome = JsonlSource::new(&path)
        .replay_from(0)
        .map_err(|e| e.to_string())?;
    let events = outcome
        .events
        .iter()
        .map(|e| serde_json::to_value(e).unwrap_or(serde_json::Value::Null))
        .collect();
    Ok(TraceDto {
        name: file,
        events,
        warnings: outcome.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ProfilePayload, Settings, ensure_runnable, get_settings, is_safe_trace_name,
        resolve_trace_path, save_profiles, switch_profile,
    };
    use sd_agent::config::settings::ModelProfileCfg;

    const DIR: &str = "C:/ws/.sd-agent/traces";

    /// Windows 盘符相对路径 / 冒号名：join 会整体替换路径，必须拒。
    #[test]
    fn rejects_drive_prefix_and_colon_names() {
        for bad in ["C:evil.jsonl", "x:y.jsonl"] {
            assert!(!is_safe_trace_name(bad), "{bad} 应拒绝");
            assert!(
                resolve_trace_path(std::path::Path::new(DIR), bad).is_err(),
                "{bad} 应拒绝"
            );
        }
    }

    /// 相对路径穿越：斜杠/反斜杠形式都必须拒。
    #[test]
    fn rejects_traversal_names() {
        for bad in ["../x.jsonl", "..\\x.jsonl"] {
            assert!(!is_safe_trace_name(bad), "{bad} 应拒绝");
            assert!(
                resolve_trace_path(std::path::Path::new(DIR), bad).is_err(),
                "{bad} 应拒绝"
            );
        }
    }

    /// 名字里带点点但不是穿越（旧实现 contains("..") 误杀），必须放行。
    #[test]
    fn accepts_names_with_inner_dots() {
        for ok in ["a..b.jsonl", "run-1-0.jsonl", "trace_2.jsonl"] {
            assert!(is_safe_trace_name(ok), "{ok} 应放行");
            assert!(
                resolve_trace_path(std::path::Path::new(DIR), ok).is_ok(),
                "{ok} 应放行"
            );
        }
    }

    /// 空名 / 错扩展名 / 含分隔符 / 多组件：一律拒。
    #[test]
    fn rejects_empty_wrong_ext_and_multi_component() {
        for bad in ["", "x.txt", "a/b.jsonl", "a\\b.jsonl", "..", "C:"] {
            assert!(!is_safe_trace_name(bad), "{bad} 应拒绝");
            assert!(
                resolve_trace_path(std::path::Path::new(DIR), bad).is_err(),
                "{bad} 应拒绝"
            );
        }
    }

    // —— 配置链命令测试（隔离用户目录：USERPROFILE 指向临时目录）——

    /// 进程内环境变量互斥（USERPROFILE 是进程全局，测试串行进入临界区）。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 测试用户目录沙箱：持锁 + 隔离 USERPROFILE，Drop 时还原并清理。
    struct HomeSandbox {
        _guard: std::sync::MutexGuard<'static, ()>,
        home: std::path::PathBuf,
        saved: Option<String>,
    }

    impl HomeSandbox {
        fn new(tag: &str) -> Self {
            let guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let home = std::env::temp_dir()
                .join(format!("sd-desktop-settings-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&home);
            std::fs::create_dir_all(&home).expect("temp home");
            let saved = std::env::var("USERPROFILE").ok();
            unsafe {
                std::env::set_var("USERPROFILE", &home);
            }
            Self {
                _guard: guard,
                home,
                saved,
            }
        }
    }

    impl Drop for HomeSandbox {
        fn drop(&mut self) {
            unsafe {
                match self.saved.take() {
                    Some(v) => std::env::set_var("USERPROFILE", v),
                    None => std::env::remove_var("USERPROFILE"),
                }
            }
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    fn payload(label: &str, api_key: Option<&str>) -> ProfilePayload {
        ProfilePayload {
            label: label.to_string(),
            base_url: format!("https://{label}.invalid/v1"),
            model: format!("model-{label}"),
            api_key: api_key.map(|k| k.to_string()),
            reasoning_effort: "medium".to_string(),
        }
    }

    /// profiles save/load roundtrip：整体落盘 → 读回一致；
    /// api_key 明文永不回显；api_key=None / 空白串 = 密钥不变。
    #[test]
    fn profiles_save_load_roundtrip_and_key_never_echoed() {
        let _sb = HomeSandbox::new("roundtrip");
        let out = save_profiles(
            vec![
                payload("alpha", Some("sk-alpha-secret")),
                payload("beta", Some("sk-beta-secret")),
            ],
            "beta".to_string(),
            33,
        );
        assert!(out.ok, "{:?}", out.error);
        assert!(out.path.ends_with("settings.json"), "{}", out.path);

        let dto = get_settings();
        assert_eq!(dto.profiles.len(), 2);
        assert_eq!(dto.profiles[0].label, "alpha");
        assert_eq!(dto.profiles[0].model, "model-alpha");
        assert!(dto.profiles.iter().all(|p| p.has_api_key));
        assert_eq!(dto.active_profile, "beta");
        assert_eq!(dto.max_rounds, 33);
        assert!(dto.settings_path.ends_with("settings.json"));
        // 密钥明文永不回显（wire 形态整体检查）。
        let dump = serde_json::to_string(&dto).expect("serialize");
        assert!(!dump.contains("sk-"), "api_key 明文禁止回显：{dump}");

        // 落盘真实读回（roundtrip 的 load 一侧直接看核心读回）。
        let loaded = Settings::load();
        assert_eq!(loaded.profiles.len(), 2);
        assert_eq!(loaded.profiles[0].api_key.as_deref(), Some("sk-alpha-secret"));
        assert_eq!(loaded.profiles[1].api_key.as_deref(), Some("sk-beta-secret"));
        assert_eq!(loaded.active_profile, "beta");
        assert_eq!(loaded.max_rounds, 33);

        // api_key=None / 空白串 = 不变；其余字段照常更新。
        let mut keep_alpha = payload("alpha", None);
        keep_alpha.model = "model-a2".to_string();
        let mut keep_beta = payload("beta", Some("   "));
        keep_beta.model = "model-b2".to_string();
        let out2 = save_profiles(vec![keep_alpha, keep_beta], "alpha".to_string(), 20);
        assert!(out2.ok, "{:?}", out2.error);
        let loaded = Settings::load();
        assert_eq!(
            loaded.profiles[0].api_key.as_deref(),
            Some("sk-alpha-secret"),
            "api_key=None 不改密钥"
        );
        assert_eq!(
            loaded.profiles[1].api_key.as_deref(),
            Some("sk-beta-secret"),
            "空白串不改密钥"
        );
        assert_eq!(loaded.profiles[0].model, "model-a2");
        assert_eq!(loaded.profiles[1].model, "model-b2");
        assert_eq!(loaded.active_profile, "alpha");
    }

    /// save_profiles 校验：空列表 / 重名 / 空字段 / 悬空启用项 → 中文错误。
    #[test]
    fn save_profiles_validates_with_chinese_errors() {
        let _sb = HomeSandbox::new("validate");
        let err = save_profiles(vec![], "a".to_string(), 20)
            .error
            .expect("空列表必须拦下");
        assert!(err.contains("至少需要一个模型配置"), "{err}");

        let err = save_profiles(
            vec![payload("a", None), payload("a", None)],
            "a".to_string(),
            20,
        )
        .error
        .expect("重名必须拦下");
        assert!(err.contains("重复"), "{err}");

        let mut empty_url = payload("a", None);
        empty_url.base_url = "   ".to_string();
        let err = save_profiles(vec![empty_url], "a".to_string(), 20)
            .error
            .expect("空服务地址必须拦下");
        assert!(err.contains("服务地址"), "{err}");

        let err = save_profiles(vec![payload("a", Some("sk-x"))], "不存在".to_string(), 20)
            .error
            .expect("悬空启用项必须拦下");
        assert!(err.contains("启用配置"), "{err}");
    }

    /// switch_profile：切换落盘；不存在的配置名报中文错误。
    #[test]
    fn switch_profile_marks_active_and_rejects_unknown() {
        let _sb = HomeSandbox::new("switch");
        let out = save_profiles(
            vec![payload("a", Some("sk-a")), payload("b", Some("sk-b"))],
            "a".to_string(),
            20,
        );
        assert!(out.ok, "{:?}", out.error);

        let out = switch_profile("b".to_string());
        assert!(out.ok, "{:?}", out.error);
        assert_eq!(get_settings().active_profile, "b");
        assert_eq!(Settings::load().active_profile, "b", "切换必须落盘");

        let out = switch_profile("不存在的配置".to_string());
        assert!(!out.ok);
        let err = out.error.expect("必须报错");
        assert!(err.contains("不存在"), "{err}");
    }

    /// start_task 缺配置时的中文错误（命令唯一判定口 ensure_runnable）：
    /// 引导设置向导，且不出现任何环境变量名。
    #[test]
    fn start_task_missing_config_error_is_chinese() {
        // 一套配置都没有。
        let err = ensure_runnable(&Settings::default()).expect_err("空配置必须拦下");
        assert!(err.contains("设置向导"), "{err}");
        assert!(err.contains("模型配置"), "{err}");
        assert!(!err.contains("SD_AGENT_"), "{err}");

        // 有配置但缺服务地址 / 密钥。
        let mut s = Settings::default();
        s.upsert_profile(ModelProfileCfg {
            label: "wip".to_string(),
            base_url: String::new(),
            model: "m".to_string(),
            api_key: None,
            reasoning_effort: "medium".to_string(),
        });
        s.set_active("wip");
        let err = ensure_runnable(&s).expect_err("缺字段必须拦下");
        assert!(err.contains("服务地址"), "{err}");
        assert!(err.contains("API 密钥"), "{err}");
        assert!(err.contains("设置向导"), "{err}");
        assert!(!err.contains("SD_AGENT_"), "{err}");

        // 启用项悬空。
        s.set_active("不存在的配置");
        let err = ensure_runnable(&s).expect_err("悬空启用项必须拦下");
        assert!(err.contains("启用配置"), "{err}");

        // 补全后放行。
        let mut ok = Settings::default();
        ok.upsert_profile(ModelProfileCfg {
            label: "ready".to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            model: "m".to_string(),
            api_key: Some("sk-test".to_string()),
            reasoning_effort: "medium".to_string(),
        });
        ok.set_active("ready");
        assert!(ensure_runnable(&ok).is_ok());
    }
}
