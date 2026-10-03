// sd-agent desktop 前端逻辑（纯静态，无框架、无外部资源）。
// 职责：首启向导 / 多模型配置 / 会话历史切换 / 会话流（气泡、工具卡片、diff）/
//       审批 / 体检 / 轨迹。
// 数据源：invoke 命令（拉取） + Tauri 事件（推送）。

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// ---------- 前端状态 ----------
const sessions = [];             // {id, title, updated_at_ms, message_count} 镜像（侧栏）
let activeSessionId = null;
const stream = new Map();        // session_id -> 会话流条目数组
const runToSession = new Map();  // run_id -> session_id（事件路由）
const sessionRun = new Map();    // session_id -> {status, rounds}
const approvalQueue = [];        // 待弹窗的审批请求
let approvalShowing = false;
let settings = null;             // get_settings() 结果

// 流式渲染状态：每轮（round）一个流式条目，思考/正文/工具增量就地更新。
let entrySeq = 0;                // 条目自增 id（DOM 定点更新锚）
const eidToEntry = new Map();    // eid -> 流式条目
const liveTurns = new Map();     // `${run_id}#${round}` -> 未定稿的流式条目

// 设置面板编辑态（整体保存）
let editProfiles = [];           // [{label, base_url, model, has_api_key, api_key, reasoning_effort}]
let editActive = "";
let editSelected = 0;

// 向导态
let wzStep = 1;
const WZ_MAX_STEP = 6;

const EFFORT_LABELS = {
  none: "无", minimal: "极低", low: "低", medium: "中",
  high: "高", xhigh: "极高", max: "最大",
};
const STATUS_LABELS = {
  running: "运行中", completed: "已完成",
  failed: "失败", max_rounds_reached: "已熔断", idle: "空闲",
};

// ---------- 工具函数 ----------
const $ = (id) => document.getElementById(id);

function esc(s) {
  return String(s ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function trunc(s, n) {
  const t = String(s ?? "");
  return t.length > n ? t.slice(0, n) + "…" : t;
}

function fmtTime(ms) {
  if (!ms) return "";
  const d = new Date(Number(ms));
  return d.toLocaleString("zh-CN", { hour12: false });
}

function statusLabel(s) {
  return STATUS_LABELS[s] || String(s ?? "空闲");
}

function hint(text, isError = false) {
  const el = $("hint");
  el.textContent = text;
  el.style.color = isError ? "var(--danger)" : "var(--dim)";
  setTimeout(() => { if (el.textContent === text) el.textContent = ""; }, 6000);
}

function ensureStream(sid) {
  if (!stream.has(sid)) stream.set(sid, []);
  return stream.get(sid);
}

function activeProfile() {
  const s = settings || {};
  return (s.profiles || []).find((p) => p.label === s.active_profile) || (s.profiles || [])[0] || null;
}

// ---------- 渲染：富文本 / diff ----------
function renderRich(text) {
  // 把 ```diff 围栏渲染成着色 diff 块，其余围栏渲染成等宽代码块。
  const s = String(text ?? "");
  const parts = s.split(/```/);
  return parts.map((part, i) => {
    if (i % 2 === 0) {
      return part ? `<div class="bubble-text">${esc(part)}</div>` : "";
    }
    const nl = part.indexOf("\n");
    const lang = (nl >= 0 ? part.slice(0, nl) : part).trim().toLowerCase();
    const body = nl >= 0 ? part.slice(nl + 1) : "";
    if (lang === "diff") return renderDiff(body);
    return `<div class="diff-block"><div class="diff-title">代码</div><div class="diff-body"><div class="diff-line">${esc(body)}</div></div></div>`;
  }).join("");
}

function looksLikeDiff(s) {
  const t = String(s ?? "");
  return /(^|\n)(@@ .*@@|\+\+\+ |--- )/.test(t) ||
    t.split("\n").filter((l) => /^[+-]/.test(l)).length >= 3;
}

function renderDiff(text) {
  const lines = String(text ?? "").replace(/\n$/, "").split("\n");
  const body = lines.map((l) => {
    let cls = "";
    if (/^@@/.test(l)) cls = "hunk";
    else if (/^\+/.test(l)) cls = "add";
    else if (/^-/.test(l)) cls = "del";
    return `<div class="diff-line ${cls}">${esc(l)}</div>`;
  }).join("");
  return `<div class="diff-block"><div class="diff-title">DIFF</div><div class="diff-body">${body}</div></div>`;
}

// ---------- 侧栏：会话列表 ----------
function renderSessionList() {
  const box = $("session-list");
  if (sessions.length === 0) {
    box.innerHTML = '<div class="meta">暂无对话</div>';
    return;
  }
  box.innerHTML = sessions
    .map((s) => {
      const run = sessionRun.get(s.id);
      const status = run ? run.status : "idle";
      return `
      <div class="run-item ${s.id === activeSessionId ? "active" : ""}" data-session="${esc(s.id)}">
        <div class="run-name" title="${esc(s.title)}">${esc(trunc(s.title, 40) || "（无标题对话）")}</div>
        <div class="run-sub">
          <span class="badge ${esc(status)}">${esc(statusLabel(status))}</span>
          <span class="run-rounds">${esc(s.message_count ?? 0)} 条 · ${esc(fmtTime(s.updated_at_ms))}</span>
        </div>
      </div>`;
    })
    .join("");
  box.querySelectorAll(".run-item").forEach((el) => {
    el.addEventListener("click", () => selectSession(el.dataset.session));
  });
}

function upsertSession(info) {
  if (!info || !info.id) return;
  const idx = sessions.findIndex((s) => s.id === info.id);
  if (idx >= 0) sessions[idx] = { ...sessions[idx], ...info };
  else sessions.unshift({ title: "新对话", message_count: 0, updated_at_ms: Date.now(), ...info });
  sessions.sort((a, b) => (Number(b.updated_at_ms) || 0) - (Number(a.updated_at_ms) || 0));
  renderSessionList();
}

// ---------- 主区头部 ----------
function renderHeader() {
  const s = sessions.find((x) => x.id === activeSessionId);
  const run = activeSessionId ? sessionRun.get(activeSessionId) : null;
  const status = run ? run.status : "idle";
  if (s) {
    $("session-title").textContent = trunc(s.title, 60) || "（无标题对话）";
    $("session-meta").textContent =
      `${s.id} · ${s.message_count ?? 0} 条消息 · ${fmtTime(s.updated_at_ms)}`;
    $("dp-run-info").innerHTML =
      `${esc(trunc(s.title, 120))}<br><span class="mono">${esc(s.id)}</span> · ${esc(statusLabel(status))}${run ? " · " + esc(run.rounds) + " 轮" : ""}`;
  } else {
    $("session-title").textContent = "未选择对话";
    $("session-meta").textContent = "点击左侧历史对话，或新建对话开始使用";
    $("dp-run-info").textContent = "未选择对话";
  }
  const st = $("run-status");
  st.textContent = statusLabel(status);
  st.className = "badge " + (status || "idle");
}

async function selectSession(id) {
  activeSessionId = id;
  renderSessionList();
  renderHeader();
  if (!stream.has(id)) {
    // 从服务端恢复历史上下文
    try {
      const dto = await invoke("load_session", { id });
      const entries = ((dto && dto.messages) || []).map((m) => ({
        t: "model",
        role: m.role || "assistant",
        text: m.text || "",
      }));
      stream.set(id, entries);
    } catch (e) {
      stream.set(id, []);
    }
  }
  renderStream();
  renderDetailEvents();
}

// ---------- 主区：会话流 ----------
function renderStream() {
  const box = $("stream");
  if (!activeSessionId) {
    box.innerHTML = '<div class="empty">新建对话开始使用。</div>';
    return;
  }
  const entries = stream.get(activeSessionId) || [];
  if (entries.length === 0) {
    box.innerHTML = '<div class="empty">发送第一条消息开始对话。</div>';
    return;
  }
  box.innerHTML = entries.map(renderEntry).join("");
  box.scrollTop = box.scrollHeight;
}

function renderEntry(en) {
  if (en.t === "user") {
    return `<div class="bubble user"><div class="bubble-tag">用户</div><div class="bubble-text">${esc(en.text)}</div></div>`;
  }
  if (en.t === "stream") {
    return renderStreamEntry(en);
  }
  if (en.t === "model") {
    const isUser = en.role === "user";
    const tag = isUser ? "用户" : "助手";
    const cls = isUser ? "user" : "assistant";
    const body = en.text ? renderRich(en.text) : '<div class="bubble-text dim">（本轮无文本，仅工具调用）</div>';
    return `<div class="bubble ${cls}"><div class="bubble-tag">${tag}</div>${body}</div>`;
  }
  return renderEvent(en.event);
}

// ---------- 流式条目（思考折叠块 + 逐字正文 + 工具卡片） ----------
function renderStreamEntry(en) {
  // 思考块：有思考内容、或思考流还没开始出字时都显示；流式期间展开，
  // 流完自动收起（en.done），点头部可手动展开回看全文。
  const showThink = en.reasoning.length > 0 || !en.done;
  const thinkHtml = showThink
    ? `<div class="think-block ${en.done ? "" : "live"} ${en.thinkOpen ? "open" : ""}">
        <div class="think-head" data-eid="${en.eid}">
          <span class="think-caret">${en.thinkOpen ? "▾" : "▸"}</span>
          <span class="think-label">${en.done ? "思考过程" : "思考中…"}</span>
          <span class="think-meta">${en.reasoning.length} 字</span>
        </div>
        <div class="think-body">${esc(en.reasoning)}</div>
      </div>`
    : "";
  let textHtml;
  if (en.done) {
    // 定稿：走富文本（diff / 代码块着色），与一次性气泡同观感。
    textHtml = en.text
      ? renderRich(en.text)
      : '<div class="bubble-text dim">（本轮无文本，仅工具调用）</div>';
  } else if (en.text) {
    textHtml = `<div class="bubble-text stream-text">${esc(en.text)}<span class="stream-cursor">▍</span></div>`;
  } else {
    textHtml = '<div class="bubble-text dim stream-text"><span class="stream-cursor">▍</span></div>';
  }
  const toolsHtml = en.tools.length
    ? `<div class="stream-tools">${en.tools.map(streamToolCard).join("")}</div>`
    : "";
  return `<div class="bubble assistant stream-entry" data-eid="${en.eid}"><div class="bubble-tag">助手</div>${thinkHtml}${textHtml}${toolsHtml}</div>`;
}

// 流式工具卡片：参数随流原地更新（streamToolCard 整卡重绘）。
function streamToolCard(t) {
  const args = String(t.args ?? "");
  const body = looksLikeDiff(args) ? renderDiff(args) : `<pre>${esc(trunc(args, 800))}</pre>`;
  const id = t.id ? `<span class="tool-id">${esc(t.id)}</span>` : "";
  const phase = t.done ? "工具调用" : "工具调用 · 参数生成中…";
  return `<div class="card tool-live">
    <div class="card-head"><span class="tool-name">⚙ ${esc(t.name)}</span><span>${phase}</span>${id}</div>
    ${body}
  </div>`;
}

// 取（或建）某轮的流式条目：run_id + round 唯一定位一轮模型回合。
function liveEntry(sid, runId, round) {
  const key = `${runId}#${round}`;
  let en = liveTurns.get(key);
  if (!en) {
    en = {
      t: "stream",
      eid: ++entrySeq,
      run_id: runId,
      round,
      reasoning: "",
      text: "",
      tools: [],
      thinkOpen: true,
      done: false,
    };
    liveTurns.set(key, en);
    eidToEntry.set(en.eid, en);
    ensureStream(sid).push(en);
  }
  return en;
}

// 流式增量的 DOM 定点更新（不整段重绘：逐字流出不闪、滚动不跳）。
function patchStreamEntry(en) {
  const box = $("stream");
  const el = box.querySelector(`.stream-entry[data-eid="${en.eid}"]`);
  if (!el) {
    renderStream();
    return;
  }
  const body = el.querySelector(".think-body");
  if (body) body.textContent = en.reasoning;
  const meta = el.querySelector(".think-meta");
  if (meta) meta.textContent = `${en.reasoning.length} 字`;
  const txt = el.querySelector(".stream-text");
  if (txt && !en.done) {
    txt.innerHTML = esc(en.text) + '<span class="stream-cursor">▍</span>';
  }
  const toolsBox = el.querySelector(".stream-tools");
  if (toolsBox) {
    toolsBox.innerHTML = en.tools.map(streamToolCard).join("");
  } else if (en.tools.length) {
    renderStream();
    return;
  }
  box.scrollTop = box.scrollHeight;
}

function renderEvent(ev) {
  const p = ev.payload || {};
  switch (ev.kind) {
    case "run_started":
      return `<div class="divider">任务开始 · 最大轮数 ${esc(p.max_rounds)} · ${fmtTime(ev.ts_unix_ms)}</div>`;
    case "tool_call_requested": {
      // 已被流式工具卡片就地展示（参数同源）时不重复画卡；事件仍进时间线。
      if (ev.claimed) return "";
      const args = String(p.args_json ?? "");
      const body = looksLikeDiff(args) ? renderDiff(args) : `<pre>${esc(trunc(args, 800))}</pre>`;
      return `<div class="card">
        <div class="card-head"><span class="tool-name">⚙ ${esc(p.tool)}</span><span>工具调用请求</span><span class="tool-id">${esc(p.tool_call_id)}</span></div>
        ${body}
      </div>`;
    }
    case "tool_call_finished": {
      const res = String(p.result_digest ?? "");
      const body = looksLikeDiff(res) ? renderDiff(res) : `<pre>${esc(trunc(res, 800))}</pre>`;
      return `<div class="card tool-result ${p.ok ? "" : "err"}">
        <div class="card-head">
          <span class="tool-name">⚙ ${esc(p.tool)}</span>
          <span class="ok-flag ${p.ok ? "ok" : "bad"}">${p.ok ? "成功" : "失败"}${p.exit_code !== null && p.exit_code !== undefined ? " · exit=" + esc(p.exit_code) : ""}</span>
          <span class="dim">${esc(p.duration_ms)} ms</span>
          <span class="tool-id">${esc(p.tool_call_id)}</span>
        </div>
        ${body}
      </div>`;
    }
    case "tool_call_denied":
      return `<div class="card denied">
        <div class="card-head"><span class="tool-name">⛔ ${esc(p.tool)}</span><span class="ok-flag bad">已拒绝</span><span class="tool-id">${esc(p.tool_call_id)}</span></div>
        <pre>${esc(p.reason)}</pre>
      </div>`;
    case "tool_approval_requested":
      return `<div class="stream-note">审批请求 · ${esc(p.tool)} · ${esc(trunc(p.summary, 120))}</div>`;
    case "tool_approval_resolved":
      return `<div class="stream-note">审批结果 · ${p.approved ? "通过" : "拒绝"} · by ${esc(p.by)}</div>`;
    case "run_finished":
      return `<div class="divider">结束 · ${esc(statusLabel(p.status))} · ${esc(p.rounds)} 轮</div>`;
    case "run_failed":
      return `<div class="divider error">任务失败 · ${esc(trunc(p.error, 200))}</div>`;
    default:
      return ""; // 其余 kind 只进右侧事件流 / trace 面板
  }
}

// ---------- 右侧：事件时间线 ----------
function renderDetailEvents() {
  const box = $("dp-events");
  if (!activeSessionId) {
    box.innerHTML = '<div class="meta">暂无事件</div>';
    return;
  }
  const evs = (stream.get(activeSessionId) || []).filter((e) => e.t === "event");
  if (evs.length === 0) {
    box.innerHTML = '<div class="meta">暂无事件</div>';
    return;
  }
  box.innerHTML = evs
    .map((e) => {
      const ev = e.event;
      const brief = trunc(JSON.stringify(ev.payload ?? {}), 120);
      const warn = /failed|denied/.test(ev.kind) ? "warn" : "";
      return `<div class="dp-ev ${warn}"><span class="k">${esc(ev.kind)}</span> ${esc(brief)}</div>`;
    })
    .join("");
  box.scrollTop = box.scrollHeight;
}

function toggleDetail() {
  $("detail-panel").classList.toggle("collapsed");
}

// ---------- 审批弹窗 ----------
function enqueueApproval(req) {
  approvalQueue.push(req);
  maybeShowApproval();
}

function maybeShowApproval() {
  if (approvalShowing || approvalQueue.length === 0) return;
  const req = approvalQueue.shift();
  approvalShowing = true;
  $("ap-tool").textContent = `${req.tool} · ${req.tool_call_id}`;
  $("ap-summary").textContent = req.summary || "";
  $("ap-detail").textContent = req.detail || "";
  $("approval-modal").dataset.toolCallId = req.tool_call_id;
  $("approval-modal").classList.remove("hidden");
}

async function resolveApproval(decision) {
  const modal = $("approval-modal");
  const toolCallId = modal.dataset.toolCallId;
  modal.classList.add("hidden");
  approvalShowing = false;
  try {
    await invoke("resolve_approval", { tool_call_id: toolCallId, decision });
  } catch (e) {
    hint("审批回传失败：" + e, true);
  }
  maybeShowApproval();
}

// ---------- 设置与状态条 ----------
async function loadSettings() {
  try {
    settings = await invoke("get_settings");
  } catch (e) {
    settings = null;
  }
  renderStatusBar();
}

function renderStatusBar() {
  const s = settings || {};
  const profiles = s.profiles || [];
  const act = activeProfile();

  // 模型切换下拉
  const sel = $("quick-profile");
  sel.innerHTML = profiles.length === 0
    ? '<option value="">未配置模型</option>'
    : profiles.map((p) => `<option value="${esc(p.label)}">${esc(p.label)} · ${esc(p.model || "无模型名")}</option>`).join("");
  if (act) sel.value = act.label;

  // 思考强度快捷选择（就地改当前配置）
  $("quick-effort").value = (act && act.reasoning_effort) || "medium";

  $("sb-model").textContent = "模型：" + (act && act.model ? act.model : "未配置");
  $("sb-rounds").textContent = "轮数上限：" + (s.max_rounds ?? "—");
}

function missingFields() {
  const p = activeProfile();
  const missing = [];
  if (!p) {
    missing.push("模型配置");
    return missing;
  }
  if (!String(p.base_url ?? "").trim()) missing.push("模型端点 URL");
  if (!String(p.model ?? "").trim()) missing.push("模型名");
  return missing;
}

// 快捷切换模型配置
async function quickSwitchProfile(label) {
  if (!label) return;
  try {
    await invoke("switch_profile", { label });
  } catch (e) {
    hint("切换配置失败：" + e, true);
    return;
  }
  if (settings) settings.active_profile = label;
  renderStatusBar();
  hint("已切换模型配置：" + label);
}

// 快捷调整思考强度（就地改当前配置并整体保存）
async function quickSetEffort(effort) {
  const s = settings || {};
  const profiles = s.profiles || [];
  const act = activeProfile();
  if (!act) {
    hint("还没有模型配置，请先完成设置", true);
    return;
  }
  const next = profiles.map((p) => ({
    label: p.label,
    base_url: p.base_url,
    model: p.model,
    api_key: null,                    // null = 不修改密钥
    reasoning_effort: p.label === act.label ? effort : p.reasoning_effort,
  }));
  try {
    await invoke("save_profiles", {
      profiles: next,
      active_profile: s.active_profile ?? act.label,
      max_rounds: Number(s.max_rounds) || 12,
    });
    await loadSettings();
    hint("思考强度已改为：" + (EFFORT_LABELS[effort] || effort));
  } catch (e) {
    hint("保存失败：" + e, true);
    renderStatusBar();
  }
}

// ---------- 设置面板（多配置管理） ----------
function openSettings(alertText, isError = false) {
  const s = settings || {};
  editProfiles = (s.profiles || []).map((p) => ({
    label: p.label ?? "",
    base_url: p.base_url ?? "",
    model: p.model ?? "",
    has_api_key: !!p.has_api_key,
    api_key: null,                    // 编辑期密钥槽：null = 不修改
    reasoning_effort: p.reasoning_effort ?? "medium",
  }));
  if (editProfiles.length === 0) {
    editProfiles = [{ label: "默认配置", base_url: "", model: "", has_api_key: false, api_key: null, reasoning_effort: "medium" }];
  }
  editActive = s.active_profile ?? editProfiles[0].label;
  editSelected = Math.max(0, editProfiles.findIndex((p) => p.label === editActive));
  $("st-max-rounds").value = s.max_rounds ?? 12;
  $("st-path").textContent = s.settings_path ?? "—";
  $("test-result").textContent = "";
  $("test-list").innerHTML = "";
  renderProfileList();
  fillProfileForm();
  const alert = $("settings-alert");
  if (alertText) {
    alert.textContent = alertText;
    alert.className = "alert" + (isError ? " error" : "");
  } else {
    alert.className = "alert hidden";
  }
  $("settings-modal").classList.remove("hidden");
}

function renderProfileList() {
  const box = $("profile-list");
  box.innerHTML = editProfiles
    .map((p, i) => `
      <div class="profile-item ${i === editSelected ? "active" : ""}" data-idx="${i}">
        <div class="p-label">
          ${esc(p.label || "未命名")}
          ${p.label === editActive ? '<span class="badge completed">当前</span>' : ""}
          ${p.has_api_key ? '<span class="badge">密钥已存</span>' : ""}
        </div>
        <div class="p-sub">${esc(p.model || "无模型名")} · ${esc(trunc(p.base_url, 28) || "无端点")}</div>
        <div class="p-sub">思考：${esc(EFFORT_LABELS[p.reasoning_effort] || p.reasoning_effort || "—")}</div>
      </div>`)
    .join("");
  box.querySelectorAll(".profile-item").forEach((el) => {
    el.addEventListener("click", () => {
      syncFormToEdit();
      editSelected = Number(el.dataset.idx);
      renderProfileList();
      fillProfileForm();
    });
  });
}

function fillProfileForm() {
  const p = editProfiles[editSelected];
  if (!p) return;
  $("st-label").value = p.label;
  $("st-model").value = p.model;
  $("st-base-url").value = p.base_url;
  $("st-api-key").value = "";                    // 密钥永不回显
  $("st-effort").value = p.reasoning_effort ?? "medium";
  $("st-api-key-hint").textContent = p.has_api_key
    ? "已保存（不回显）。留空 = 保持不变；输入新值 = 覆盖。"
    : "密钥保存后不再回显，界面上只有“是否已保存”。";
}

function syncFormToEdit() {
  const p = editProfiles[editSelected];
  if (!p) return;
  const keyInput = $("st-api-key").value.trim();
  p.label = $("st-label").value.trim();
  p.model = $("st-model").value.trim();
  p.base_url = $("st-base-url").value.trim();
  p.reasoning_effort = $("st-effort").value;
  if (keyInput) p.api_key = keyInput;          // 输入新值才覆盖
}

function addProfile() {
  syncFormToEdit();
  let n = editProfiles.length + 1;
  let label = "配置 " + n;
  while (editProfiles.some((p) => p.label === label)) { n += 1; label = "配置 " + n; }
  editProfiles.push({ label, base_url: "", model: "", has_api_key: false, api_key: null, reasoning_effort: "medium" });
  editSelected = editProfiles.length - 1;
  renderProfileList();
  fillProfileForm();
}

function deleteProfile() {
  if (editProfiles.length <= 1) {
    openSettings("至少保留一份模型配置", true);
    return;
  }
  syncFormToEdit();
  const removed = editProfiles.splice(editSelected, 1)[0];
  if (removed && removed.label === editActive) editActive = editProfiles[0].label;
  editSelected = 0;
  renderProfileList();
  fillProfileForm();
}

function activateProfile() {
  syncFormToEdit();
  const p = editProfiles[editSelected];
  if (!p) return;
  editActive = p.label;
  renderProfileList();
}

async function saveSettings() {
  syncFormToEdit();
  const labels = editProfiles.map((p) => p.label);
  if (labels.some((l) => !l)) {
    openSettings("配置名称不能为空", true);
    return;
  }
  if (new Set(labels).size !== labels.length) {
    openSettings("配置名称不能重复", true);
    return;
  }
  const profiles = editProfiles.map((p) => ({
    label: p.label,
    base_url: p.base_url,
    model: p.model,
    api_key: p.api_key,              // null = 不修改密钥
    reasoning_effort: p.reasoning_effort,
  }));
  const maxRounds = Number($("st-max-rounds").value) || 12;
  try {
    const res = await invoke("save_profiles", {
      profiles,
      active_profile: editActive,
      max_rounds: maxRounds,
    });
    const ok = res === undefined || res === null || res === true || (res && res.ok !== false);
    if (ok) {
      await loadSettings();
      const path = (res && res.path) || (settings && settings.settings_path) || "（未知路径）";
      openSettings("设置已保存，配置文件：" + path, false);
      $("settings-alert").className = "alert ok";
    } else {
      openSettings("保存失败：" + ((res && res.error) || "未知错误"), true);
    }
  } catch (e) {
    openSettings("保存失败：" + e, true);
  }
}

// ---------- 测试连接（专用命令 test_connection：一次最小模型调用） ----------
// 不走 doctor 体检：doctor 的探针要 spawn 控制台子进程（GUI 下闪黑色
// conhost 窗），而且用户要的是“连没连上”一句话，不是体检清单。
// 计费敏感：30 秒冷却——前端禁用按钮 + 壳层 try_begin_test 双保险，
// 防连打烧额度（每次测试都真实调用一次模型）。
const TEST_COOLDOWN_MS = 30_000; // 与壳层 state::TEST_COOLDOWN_MS 对齐
let lastTestAt = 0;
let testCooldownTimer = null;

function armTestCooldown() {
  lastTestAt = Date.now();
  clearInterval(testCooldownTimer);
  const tick = () => {
    const leftS = Math.ceil((TEST_COOLDOWN_MS - (Date.now() - lastTestAt)) / 1000);
    for (const id of ["btn-test-conn", "wz-test-btn"]) {
      const b = $(id);
      if (!b) continue;
      b.disabled = leftS > 0;
      b.textContent = leftS > 0 ? `测试连接（${leftS}s 后可再测）` : "测试连接";
    }
    if (leftS <= 0) clearInterval(testCooldownTimer);
  };
  tick();
  testCooldownTimer = setInterval(tick, 1000);
}

async function testConnection(resultEl, listEl) {
  const leftMs = TEST_COOLDOWN_MS - (Date.now() - lastTestAt);
  if (lastTestAt > 0 && leftMs > 0) {
    $(resultEl).textContent =
      `测试太频繁：请 ${Math.ceil(leftMs / 1000)} 秒后再试（每次测试都会真实调用一次模型，防止误触烧额度）`;
    $(resultEl).className = "meta test-bad";
    return;
  }
  armTestCooldown();
  $(resultEl).className = "meta";
  $(resultEl).textContent = "测试中…（一次最小模型调用，可能耗时数秒）";
  $(listEl).innerHTML = "";
  try {
    const res = await invoke("test_connection");
    const ok = !!(res && res.ok);
    $(resultEl).textContent = (res && res.message) || (ok ? "连接成功" : "连接失败：未知原因");
    $(resultEl).className = "meta " + (ok ? "test-ok" : "test-bad");
  } catch (e) {
    $(resultEl).textContent = "测试失败：" + e;
    $(resultEl).className = "meta test-bad";
  }
}

// ---------- 首启向导 ----------
function needsWizard() {
  const p = activeProfile();
  return !p || !String(p.base_url ?? "").trim() || !String(p.model ?? "").trim();
}

function openWizard() {
  wzStep = 1;
  $("wz-base-url").value = "";
  $("wz-label").value = "默认配置";
  $("wz-model").value = "";
  $("wz-api-key").value = "";
  $("wz-effort").value = "medium";
  $("wz-max-rounds").value = "12";
  $("wz-test-result").textContent = "";
  $("wz-test-list").innerHTML = "";
  renderWizardStep();
  $("wizard-modal").classList.remove("hidden");
}

function renderWizardStep() {
  document.querySelectorAll(".wizard-step").forEach((el) => {
    el.classList.toggle("hidden", Number(el.dataset.step) !== wzStep);
  });
  document.querySelectorAll(".wz-dot").forEach((el) => {
    const n = Number(el.dataset.step);
    el.classList.toggle("active", n === wzStep);
    el.classList.toggle("done", n < wzStep);
  });
  $("wz-prev").disabled = wzStep <= 1;
  $("wz-next").classList.toggle("hidden", wzStep >= WZ_MAX_STEP);
  $("wz-finish").classList.toggle("hidden", wzStep < WZ_MAX_STEP);
}

function wizardValidate() {
  if (wzStep === 1 && !$("wz-base-url").value.trim()) {
    hint("请先填写模型端点 URL", true);
    return false;
  }
  if (wzStep === 2 && (!$("wz-model").value.trim() || !$("wz-label").value.trim())) {
    hint("请填写配置名称和模型名", true);
    return false;
  }
  return true;
}

async function wizardFinish() {
  const label = $("wz-label").value.trim() || "默认配置";
  const profile = {
    label,
    base_url: $("wz-base-url").value.trim(),
    model: $("wz-model").value.trim(),
    api_key: $("wz-api-key").value.trim() || null,
    reasoning_effort: $("wz-effort").value,
  };
  const maxRounds = Number($("wz-max-rounds").value) || 12;
  try {
    await invoke("save_profiles", {
      profiles: [profile],
      active_profile: label,
      max_rounds: maxRounds,
    });
    await loadSettings();
    $("wizard-modal").classList.add("hidden");
    hint("配置完成，新建对话开始使用");
  } catch (e) {
    hint("保存失败：" + e, true);
  }
}

// ---------- doctor 体检面板 ----------
// run_doctor 返回形态兼容：裸数组（当前后端契约）或 {items:[…]}。
// 形态不对 / 空列表必须报错——“0 项通过 / 0 项失败”是没意义的废话。
function doctorItems(res) {
  if (Array.isArray(res)) return res;
  if (res && Array.isArray(res.items)) return res.items;
  return null;
}

function doctorSummary(items) {
  const bad = items.filter((i) => !i.ok).length;
  return `体检完成：${items.length - bad} 项通过 / ${bad} 项失败（共 ${items.length} 项）`;
}

function doctorCard(i) {
  const fix = !i.ok && i.fix ? `<div class="fix-text">修法：${esc(i.fix)}</div>` : "";
  return `<div class="doctor-item ${i.ok ? "ok" : "bad"}">
    <div class="doctor-head">
      <span class="flag">${i.ok ? "✅" : "❌"}</span>
      <span class="title">${esc(i.title || i.name)}</span>
      <span class="detail">${esc(i.detail || "")}</span>
    </div>
    ${i.hint ? `<div class="hint-text">${esc(i.hint)}</div>` : ""}
    ${fix}
  </div>`;
}

async function openDoctor() {
  $("doctor-modal").classList.remove("hidden");
  await runDoctor();
}

async function runDoctor() {
  $("doctor-running").style.display = "block";
  $("doctor-list").innerHTML = "";
  try {
    const res = await invoke("run_doctor");
    $("doctor-running").style.display = "none";
    const items = doctorItems(res);
    if (!items) {
      $("doctor-list").innerHTML =
        '<div class="trace-warn">体检返回数据形态异常（不是检查项列表），无法统计；请重启程序重试，或查看 .sd-agent/logs/desktop.log。</div>';
      return;
    }
    if (items.length === 0) {
      $("doctor-list").innerHTML =
        '<div class="trace-warn">体检没有返回任何检查项，无法给出结果；请重启程序重试，或查看 .sd-agent/logs/desktop.log。</div>';
      return;
    }
    $("doctor-list").innerHTML =
      `<div class="doctor-summary">${esc(doctorSummary(items))}</div>` + items.map(doctorCard).join("");
  } catch (e) {
    $("doctor-running").style.display = "none";
    $("doctor-list").innerHTML = `<div class="trace-warn">体检失败：${esc(e)}</div>`;
  }
}

// ---------- trace 轨迹面板 ----------
async function openTrace() {
  $("trace-modal").classList.remove("hidden");
  await refreshTraceList();
}

async function refreshTraceList() {
  const box = $("trace-list");
  try {
    const files = await invoke("list_traces");   // 契约：[文件名] 字符串数组
    if (!files || files.length === 0) {
      box.innerHTML = '<div class="meta">暂无轨迹（先跑一次任务）</div>';
      return;
    }
    box.innerHTML = files
      .map((f) => {
        const name = typeof f === "string" ? f : (f.name ?? String(f));
        return `<div class="trace-file" data-file="${esc(name)}">${esc(name)}</div>`;
      })
      .join("");
    box.querySelectorAll(".trace-file").forEach((el) => {
      el.addEventListener("click", () => loadTrace(el.dataset.file, el));
    });
  } catch (e) {
    box.innerHTML = `<div class="trace-warn">列表失败：${esc(e)}</div>`;
  }
}

async function loadTrace(name, el) {
  document.querySelectorAll(".trace-file").forEach((x) => x.classList.remove("active"));
  if (el) el.classList.add("active");
  const view = $("trace-view");
  view.innerHTML = '<div class="meta">读取中…</div>';
  try {
    const dto = await invoke("get_trace_events", { name });
    const rows = ((dto && dto.events) || [])
      .map((ev) => `
        <div class="trace-row">
          <span class="seq">#${esc(ev.seq)}</span>
          <span class="kind">${esc(ev.kind)}</span>
          <span class="pl">${esc(trunc(JSON.stringify(ev.payload ?? {}), 220))}</span>
        </div>`)
      .join("");
    view.innerHTML = rows || '<div class="empty">空轨迹</div>';
    view.scrollTop = 0;
  } catch (e) {
    view.innerHTML = `<div class="trace-warn">读取失败：${esc(e)}</div>`;
  }
}

// ---------- 新建对话 ----------
async function newSession(title) {
  const name = String(title ?? "").trim() || ("新对话 " + fmtTime(Date.now()));
  try {
    const res = await invoke("create_session", { title: name });
    const id = res && typeof res === "object" ? res.id : res;
    if (!id) throw new Error("未返回会话 id");
    upsertSession({
      id,
      title: (res && res.title) || name,
      updated_at_ms: (res && res.updated_at_ms) || Date.now(),
      message_count: (res && res.message_count) || 0,
    });
    await selectSession(id);
    return id;
  } catch (e) {
    hint("新建对话失败：" + e, true);
    return null;
  }
}

// ---------- 发送消息（进当前会话） ----------
async function submitTask(text) {
  const task = String(text ?? "").trim();
  if (!task) {
    hint("消息内容不能为空", true);
    return;
  }
  const missing = missingFields();
  if (missing.length > 0) {
    if (!activeProfile()) openWizard();
    else openSettings("请先完成模型配置，缺少：" + missing.join("、"), true);
    return;
  }

  // 确保有当前会话
  let sid = activeSessionId;
  if (!sid) {
    sid = await newSession(trunc(task, 24));
    if (!sid) return;
  }

  ensureStream(sid).push({ t: "user", text: task });
  renderStream();

  try {
    const res = await invoke("start_task", { task, session_id: sid });
    if (!res || res.ok === false) {
      hint("启动失败：" + ((res && res.error) || "未知错误"), true);
      return;
    }
    if (res.run_id) runToSession.set(res.run_id, sid);
    sessionRun.set(sid, { status: "running", rounds: (sessionRun.get(sid) || {}).rounds || 0 });
    const s = sessions.find((x) => x.id === sid);
    if (s) {
      s.message_count = (s.message_count || 0) + 1;
      s.updated_at_ms = Date.now();
    }
    $("msg-input").value = "";
    renderSessionList();
    renderHeader();
  } catch (e) {
    hint("启动失败：" + e, true);
  }
}

// ---------- 事件订阅 ----------
function routeRun(runId) {
  return runToSession.get(runId) || activeSessionId;
}

async function subscribe() {
  await listen("run_update", (e) => {
    const p = e.payload || {};
    const runId = p.run_id ?? p.id;
    const sid = routeRun(runId);
    if (!sid) return;
    if (runId && !runToSession.has(runId)) runToSession.set(runId, sid);
    sessionRun.set(sid, { status: p.status, rounds: p.rounds });
    renderSessionList();
    if (sid === activeSessionId) renderHeader();
  });

  await listen("agent_event", (e) => {
    const p = e.payload || {};
    const runId = p.run_id;
    // 兼容两种载荷形态：{run_id, event:{kind,payload}} 或 {run_id, kind, payload}
    const ev = p.event || { kind: p.kind, payload: p.payload, ts_unix_ms: p.ts_unix_ms };
    if (!ev || !ev.kind) return;
    const sid = p.session_id || routeRun(runId);
    if (!sid) return;
    if (runId && !runToSession.has(runId)) runToSession.set(runId, sid);
    // 工具调用请求与流式工具卡片对账：命中则参数由卡片就地展示
    //（避免同一份参数画两张卡），事件本身照进条目（右侧时间线要看）。
    if (ev.kind === "tool_call_requested") {
      const entries = stream.get(sid) || [];
      for (let i = entries.length - 1; i >= 0; i--) {
        const en = entries[i];
        if (en.t !== "stream") continue;
        const t = (en.tools || []).find((x) => !x.id && x.name === ev.payload?.tool);
        if (t) {
          t.id = ev.payload?.tool_call_id || "";
          t.done = true;
          if (ev.payload?.args_json) t.args = ev.payload.args_json;
          ev.claimed = true;
          if (sid === activeSessionId) renderStream();
          break;
        }
      }
    }
    ensureStream(sid).push({ t: "event", event: ev });
    if (sid === activeSessionId) {
      renderStream();
      renderDetailEvents();
    }
  });

  // 流式增量：思考（reasoning）与正文（text）逐字推进。
  await listen("stream_delta", (e) => {
    const p = e.payload || {};
    const sid = p.session_id || routeRun(p.run_id);
    if (!sid) return;
    if (p.run_id && !runToSession.has(p.run_id)) runToSession.set(p.run_id, sid);
    const en = liveEntry(sid, p.run_id, p.round);
    if (p.kind === "reasoning") en.reasoning += String(p.delta ?? "");
    else en.text += String(p.delta ?? "");
    if (sid === activeSessionId) patchStreamEntry(en);
  });

  // 工具调用参数流：卡片原地更新（参数 JSON 边生成边显示）。
  await listen("stream_tool_call", (e) => {
    const p = e.payload || {};
    const sid = p.session_id || routeRun(p.run_id);
    if (!sid) return;
    if (p.run_id && !runToSession.has(p.run_id)) runToSession.set(p.run_id, sid);
    const en = liveEntry(sid, p.run_id, p.round);
    let t = en.tools.find((x) => !x.done && x.name === p.name);
    if (!t) {
      t = { name: p.name, args: "", done: false, id: "" };
      en.tools.push(t);
    }
    t.args = String(p.args_so_far ?? "");
    if (sid === activeSessionId) patchStreamEntry(en);
  });

  // 一轮流完：思考块自动收起（点头部可回看全文）。
  await listen("stream_turn_done", (e) => {
    const p = e.payload || {};
    const sid = p.session_id || routeRun(p.run_id);
    if (!sid) return;
    const en = liveTurns.get(`${p.run_id}#${p.round}`);
    if (!en) return;
    en.done = true;
    en.thinkOpen = false;
    en.tools.forEach((t) => { t.done = true; });
    if (sid === activeSessionId) renderStream();
  });

  await listen("model_message", (e) => {
    const p = e.payload || {};
    const sid = p.session_id || routeRun(p.run_id);
    if (!sid) return;
    if (p.run_id && !runToSession.has(p.run_id)) runToSession.set(p.run_id, sid);
    // 流式条目就地定稿（同 run 同轮），不重复造第二个气泡；
    // 没有流式条目（旧路径 / 流事件丢失）才回退成一次性 model 气泡。
    const en = liveTurns.get(`${p.run_id}#${p.round}`);
    if (en) {
      en.done = true;
      en.thinkOpen = false;
      if (p.text) en.text = p.text;
      en.tools.forEach((t) => { t.done = true; });
      liveTurns.delete(`${p.run_id}#${p.round}`);
    } else {
      ensureStream(sid).push({ t: "model", role: p.role || "assistant", text: p.text || "" });
    }
    const s = sessions.find((x) => x.id === sid);
    if (s) {
      s.message_count = (s.message_count || 0) + 1;
      s.updated_at_ms = Date.now();
    }
    if (sid === activeSessionId) {
      renderStream();
      renderHeader();
    }
    renderSessionList();
  });

  await listen("approval_request", (e) => {
    enqueueApproval(e.payload || {});
  });
}

// ---------- 初始化 ----------
function bindUi() {
  $("btn-new-session").addEventListener("click", () => newSession());
  $("btn-send").addEventListener("click", () => submitTask($("msg-input").value));
  $("msg-input").addEventListener("keydown", (ev) => {
    if (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey)) submitTask($("msg-input").value);
  });

  $("quick-profile").addEventListener("change", (ev) => quickSwitchProfile(ev.target.value));
  $("quick-effort").addEventListener("change", (ev) => quickSetEffort(ev.target.value));

  $("btn-settings").addEventListener("click", () => openSettings());
  $("btn-save-settings").addEventListener("click", saveSettings);
  $("btn-profile-add").addEventListener("click", addProfile);
  $("btn-profile-delete").addEventListener("click", deleteProfile);
  $("btn-profile-activate").addEventListener("click", activateProfile);
  $("btn-test-conn").addEventListener("click", () => testConnection("test-result", "test-list"));

  // 向导
  $("wz-next").addEventListener("click", () => {
    if (!wizardValidate()) return;
    wzStep = Math.min(wzStep + 1, WZ_MAX_STEP);
    renderWizardStep();
  });
  $("wz-prev").addEventListener("click", () => {
    wzStep = Math.max(wzStep - 1, 1);
    renderWizardStep();
  });
  $("wz-finish").addEventListener("click", wizardFinish);
  $("wz-test-btn").addEventListener("click", () => testConnection("wz-test-result", "wz-test-list"));

  $("btn-doctor").addEventListener("click", openDoctor);
  $("btn-trace").addEventListener("click", openTrace);
  $("doctor-refresh").addEventListener("click", runDoctor);
  $("trace-refresh").addEventListener("click", refreshTraceList);

  $("ap-approve").addEventListener("click", () => resolveApproval("approved"));
  $("ap-deny").addEventListener("click", () => resolveApproval("denied"));
  $("ap-always").addEventListener("click", () => resolveApproval("always"));

  $("btn-detail").addEventListener("click", toggleDetail);
  $("btn-detail-close").addEventListener("click", toggleDetail);

  // 思考折叠块：点头部展开/收起（流式期间自动展开，流完自动收起）。
  $("stream").addEventListener("click", (ev) => {
    const head = ev.target.closest && ev.target.closest(".think-head");
    if (!head) return;
    const en = eidToEntry.get(Number(head.dataset.eid));
    if (!en) return;
    en.thinkOpen = !en.thinkOpen;
    renderStream();
  });

  document.querySelectorAll("[data-close]").forEach((el) => {
    el.addEventListener("click", () => $(el.dataset.close).classList.add("hidden"));
  });
  // 点遮罩关闭非审批、非向导弹窗（向导不可忽略）
  document.querySelectorAll(".modal-overlay").forEach((ov) => {
    ov.addEventListener("click", (ev) => {
      if (ev.target === ov && ov.id !== "approval-modal" && ov.id !== "wizard-modal") {
        ov.classList.add("hidden");
      }
    });
  });
}

async function init() {
  bindUi();
  await loadSettings();
  await subscribe();
  try {
    const list = await invoke("list_sessions");
    (list || []).forEach(upsertSession);
    if (sessions.length > 0) await selectSession(sessions[0].id);
    else renderHeader();
  } catch {
    renderHeader();
    /* 首次启动无状态可拉 */
  }
  // 首启旅程：模型未配置 → 强制向导
  if (needsWizard()) openWizard();
}

init();
