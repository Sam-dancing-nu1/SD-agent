# 调研：其他开发者与大厂如何研发 Agent、开源项目如何从零起步

> 本文件为调研类参考资料，含第三方项目名，非设计依据。
> 调研范围：Hermes Agent（一手本地仓库）、OpenCode、"PI" 编码代理（身份核验）、六个对照项目（Claude Code / OpenAI Codex CLI / Google Gemini CLI / Aider / Goose / Cline）、大厂（Anthropic / OpenAI / Google）公开工程方法论。
> 方法说明：一手来源优先（官方仓库 git 历史、官方文档站、官方博客、作者原话）；GitHub 元数据经 api.github.com 核验；npm/PyPI 发布时间经 registry.npmjs.org / pypi.org 核验；核验不到的一律进"待核验清单"，二手转述只作旁证并显式标注。
> 调研执行日期：2026-10-02。一条事实一行，括号内为来源。

## 一、Hermes Agent（Nous Research）——一手材料在本地

### 项目是什么
- 自我改进的 AI agent 运行时（harness）："The self-improving AI agent built by Nous Research"，内置学习闭环（技能自创建与自改进、记忆、跨会话检索）。（来源：本地仓库 README.md；https://hermes-agent.nousresearch.com/docs）
- 形态：CLI + TUI + 多平台消息网关（Telegram/Discord/Slack 等）+ cron 自动化 + 子代理委派，可跑在 VPS/沙箱/服务器。（来源：本地仓库 README.md）

### 从零路径（时间线，来源均为该开源项目官方仓库的本地 git 副本，路径从略）
- 2025-07-22 首次提交，共 8 个条目：5 个源文件（run_agent.py、model_tools.py、terminal_tool.py、web_tools.py、requirements.txt）+ 2 个 .pyc + .gitignore（主代理实测 git ls-tree 核对）。
- 首个形态是单脚本：run_agent.py 定义类 AIAgent（324 行），用 OpenAI 兼容客户端做"工具调用循环直到完成"，参数为 model/max_iterations/tool_delay。
- 2025-07-26 的 README 显示早期项目名 hecate，运行方式为 python run_agent.py --query ... --max_turns 20 --model ... --base_url ... --api_key ...（git show 6d346250b:README.md）。
- 2025-07~08 按需求逐步加工具：抓取/爬取内容压缩（2025-07-31）、vision 工具与工具集开关（2025-08-04）、mixture-of-agents 与图像生成（2025-08-09）、改用 firecrawl（2025-08-31 前后）。
- 2025-09-10 抽出 toolsets（"make them easy to create and configure"）；2025-09-12 出现 message graphs 与"more architecture docs"。
- 2026-01-30 引入 skills 工具；2026-02-02 cron 定时任务；2026-02-03 消息网关（telegram）与网关安全配置；2026-02-18 Skills Hub（在线技能搜索/安装）。
- 2026-02-19 持久记忆系统 + SQLite 会话存储；2026-02-20 子代理委派（subagent delegation）落地。
- 项目名 Hermes 在提交记录中最早于 2026-02 出现（分支 atropos-hermes-agent、"Hermes agent" 提交）。
- 提交量（按作者日期统计，仅供参考）：2025 全年约 70 次；2026-02 为 480 次，2026-03 为 2522 次，2026-09 达 18875 次。
- 演进顺序可概括为：单脚本工具循环 → 加工具 → 抽象 toolset/架构文档 → skills/记忆/网关/子代理/多后端。

### 技术栈
- 主体为 Python（OpenAI 兼容 SDK 接模型、SQLite + FTS5 做会话与全文检索存储）。（来源：本地仓库 run_agent.py、website/docs/developer-guide/architecture.md）
- ui-tui 组件为 TypeScript（package.json、tsconfig.json、vitest）；文档站为 Docusaurus。（来源：本地仓库 ui-tui/、website/）
- 分层（官方架构文档）：入口层（CLI、Gateway、ACP 适配器、Batch Runner、API Server、Python 库）→ AIAgent 核心（prompt 组装、provider 解析、3 种 API 模式、工具分发 70+ 工具 / 28 个 toolset）→ 存储与工具后端（SQLite+FTS5；终端 7 种后端、浏览器 5 种、Web 4 种、MCP 动态）。（来源：https://hermes-agent.nousresearch.com/docs/developer-guide/architecture；本地 website/docs/developer-guide/architecture.md）

### 研发方式与迭代节奏
- PR 驱动：首个 PR #1 于 2025-07-26 合并（"Merge pull request #1"），此后持续 PR 合并。（来源：本地 git log）
- 规划留痕：TODO.md 反复修订（如 2026-02-01"引入子代理架构与交互式澄清问题工具"、2026-02-17 重写子代理/任务管理方向）。（来源：本地 git log）
- 贡献优先级成文：bug 修复 > 跨平台兼容 > 安全加固（shell 注入/提示注入/路径穿越）> 性能健壮性 > 新技能 > 新工具（"新工具极少需要，多数能力应做成 skill"）> 文档。（来源：本地仓库 CONTRIBUTING.md）
- 文档先行：developer-guide 有 60+ 篇内部文档（agent-loop、context-compression、gateway-internals、session-storage 等）。（来源：本地 website/docs/developer-guide/）

### 对自研底座的可借鉴点
1. 起步只需"一个循环 + 少数工具"：首提交 5 个文件即跑通，抽象（toolset、message graph）都是后补的，不预建框架。
2. 会话状态先落到单文件 SQLite + FTS5，再长出记忆/检索/多端；持久化早于复杂化（2026-02-19 前后集中落地）。
3. 多入口（CLI/网关/子代理/批处理）共享同一个 agent 门面，避免每个入口重写循环。
4. "能力优先做成技能而非工具"的取舍成文入 CONTRIBUTING，控制核心工具面的膨胀。

## 二、OpenCode（sst/opencode，现 anomalyco/opencode）

### 项目是什么
- 开源编码 agent："The open source coding agent."，官方定位为终端 AI 编码代理，另有桌面 App（beta）与 Web/IDE 客户端。（来源：https://github.com/sst/opencode README）
- 由 SST 团队（Anomaly）维护；GitHub 仓库 sst/opencode 现重定向至 anomalyco/opencode。（来源：api.github.com/repos/sst/opencode 返回 full_name=anomalyco/opencode）

### 从零路径
- 仓库创建于 2025-04-30。（来源：api.github.com/repos/anomalyco/opencode created_at）
- 最早一批提交（2025-04-29~04-30）已包含 LSP 修复（"fix: lsp issues with tmp and deleted files"）、键位设计、自定义主题——第一天就是 TUI + LSP 形态。（来源：api.github.com commits）
- 2025-05-01 即出现编号 #135 的社区 PR（"add xai support (#135)"），起步阶段就接收外部贡献。（来源：api.github.com commits）
- npm 包 opencode-ai 于 2025-05-31 首发（0.0.0）。（来源：registry.npmjs.org/opencode-ai time）
- 迭代方式：npm 上持续发布 0.0.0-dev-<时间戳> 形式的开发构建（如 0.0.0-dev-202610020321）。（来源：registry.npmjs.org/opencode-ai time）
- 现有 "OpenCode v2" 大版本与桌面应用、企业版（enterprise）。（来源：https://opencode.ai/docs/ 顶部横幅；README）

### 技术栈
- 仓库主语言 TypeScript（GitHub API language 字段），monorepo 含 30+ 包（core、server、tui、sdk、desktop、enterprise、plugin 等）。（来源：api.github.com/repos/anomalyco/opencode；api.github.com contents/packages）
- 架构为 client/server："When you run opencode it starts a TUI and a server. Where the TUI is the client that talks to the server."（来源：https://opencode.ai/docs/server/）
- server 暴露 OpenAPI 3.1 规格端点，SDK 由该规格生成；支持 headless `opencode serve` 与 HTTP basic auth。（来源：https://opencode.ai/docs/server/）
- 内置多语言 LSP 集成，把诊断信息作为反馈给 agent。（来源：https://opencode.ai/docs/lsp/）

### 研发方式与迭代节奏
- 单 server 支持多客户端：TUI、CLI、Web、IDE、桌面。（来源：https://opencode.ai/docs/server/、https://github.com/sst/opencode README）
- 契约先行：OpenAPI 规格既对外提供又生成 SDK，客户端与服务端解耦。（来源：https://opencode.ai/docs/server/）
- 高频开发构建 + 后期大版本（v2）演进。（来源：registry.npmjs.org/opencode-ai time；https://opencode.ai/docs/）

### 对自研底座的可借鉴点
1. "一个后端 + 多前端"以 HTTP/OpenAPI 契约切开，UI 形态可随时增删。
2. LSP 等外部工具的诊断回流做成 agent 的感知通道，而不是 UI 层装饰。
3. 开发构建流水线（每次提交一个 dev 版本）让使用者可以跟随主干，降低"发布即大事件"的压力。

## 三、"PI" 编码代理（身份已确认：Mario Zechner / badlogic 的 pi）

### 身份核验
- 候选身份确认：pi.dev 自述 "Pi is a minimal agent harness"；npm 包 @mariozechner/pi-coding-agent；作者 Mario Zechner 的博客文章自述"build my own coding agent harness pi"。（来源：https://pi.dev/；registry.npmjs.org/@mariozechner/pi-coding-agent；https://mariozechner.at/posts/2025-11-30-pi-coding-agent/）
- 作者 X 账号 @badlogicgames 于 2025-11-30 发布该文、2025-11-30 前后提及 pi 在 Terminal-Bench 的成绩（原文抓取受限，仅见搜索摘要）→ 见待核验清单。（来源：x.com/badlogicgames）

### 从零路径
- 工具包仓库 earendil-works/pi 创建于 2025-08-09，描述为 "AI agent toolkit: unified LLM API, agent loop, TUI, coding agent CLI"。（来源：api.github.com/repos/earendil-works/pi）
- npm @mariozechner/pi-ai 创建于 2025-08-30；npm @mariozechner/pi-coding-agent 于 2025-11-12 首发（首个版本 0.6.2）。（来源：registry.npmjs.org time 字段）
- 作者自述构建顺序：先 pi-ai（统一 LLM API）→ pi-agent-core（agent 循环）→ pi-tui（最小 TUI 框架）→ pi-coding-agent（CLI 组装层）。（来源：https://mariozechner.at/posts/2025-11-30-pi-coding-agent/）
- 2025-11-30 作者长文总结经验，文末以 Terminal-Bench 2.0 + Claude Opus 4.5 与 Codex、Cursor 等对比评测。（来源：同上）

### 技术栈
- TypeScript monorepo，四包结构：pi-ai / pi-agent-core / pi-tui / pi-coding-agent。（来源：https://mariozechner.at/posts/2025-11-30-pi-coding-agent/；npm registry）
- pi-ai 统一四类模型 API：OpenAI Completions、OpenAI Responses、Anthropic Messages、Google Generative AI；支持多提供商、流式、TypeBox schema 工具定义、推理痕迹、跨提供商上下文交接、token/成本统计。（来源：同上作者博客）
- pi-ai 带跨提供商测试套件（图片输入、推理痕迹、工具调用等在各提供商/热门模型上跑）。（来源：同上）
- pi-tui 为保留模式（retained mode）+ 差分渲染 + 同步输出（防闪烁）的自研终端 UI 框架。（来源：同上）

### 研发方式与取向（作者原话级事实）
- 明确的"不做清单"：最小工具集（read/write/edit/bash）、默认 YOLO、无内置 todo、无 plan 模式、无 MCP、无后台 bash、无子代理。（来源：同上）
- 信条："if I don't need it, it won't be built."（不需要的不建）。（来源：同上）
- 动机是可控的上下文工程与完全可检视的交互记录（自述现有 harness 背后注入内容、API 是"organic evolution"）。（来源：同上）
- 单人项目 + 博客留痕 + 基准评测自证。（来源：同上；pi.dev）

### 对自研底座的可借鉴点
1. 先做并测稳"统一模型 API 层"（四类 API 的差异点逐项处理），agent 循环反而薄。
2. 写下"不做什么"的清单与"做什么"同等重要，控制 harness 复杂度。
3. UI 独立成包（pi-tui），agent 核心可被不同 UI/SDK 复用。
4. 会话/交互记录格式可后处理、可检视，是调试与评测的前提（作者自述动机）。

## 四、对照组（简要，每个不超过 10 行）

### Claude Code（Anthropic）
- 起步形态：2025-02-24 随 Claude 3.7 Sonnet 发布的"limited research preview"，命令行 agentic coding 工具。（来源：https://www.anthropic.com/news/claude-3-7-sonnet）
- 技术栈：TypeScript；GitHub 仓库 anthropics/claude-code 创建于 2025-02-22。（来源：api.github.com/repos/anthropics/claude-code）
- npm @anthropic-ai/claude-code 于 2025-02-24 首发（0.2.x），到 2026-10-01 已发布 2.1.287——极高频迭代。（来源：registry.npmjs.org/@anthropic-ai/claude-code time）
- 研发方式：闭源产品 + 公开文档/issue 跟踪；官方方法论文档化（见第五节）。（来源：同上；anthropic.com/engineering）

### OpenAI Codex CLI
- 起步形态：开源终端编码 agent（"lightweight open-source coding agent that runs in your terminal"）。（来源：https://github.com/openai/codex README；发布时间线见待核验）
- 仓库 openai/codex 创建于 2025-04-13。（来源：api.github.com/repos/openai/codex）
- 技术栈：monorepo 含 codex-cli、codex-rs（Rust 核心）、sdk，Bazel + pnpm；发布独立二进制。（来源：https://github.com/openai/codex 仓库结构与 README）
- 形态扩展：终端 CLI + IDE 插件 + 桌面 app + 云端 Codex Web。（来源：https://github.com/openai/codex README）

### Google Gemini CLI
- 起步形态：2025-06-25 发布的开源终端 AI agent，个人账号免费额度高，与 Gemini Code Assist 共享技术。（来源：https://blog.google/innovation-and-ai/technology/developers-tools/introducing-gemini-cli-open-source-ai-agent/）
- 仓库 google-gemini/gemini-cli 创建于 2025-04-17，TypeScript，Apache 2.0。（来源：api.github.com/repos/google-gemini/gemini-cli；README）
- npm @google/gemini-cli 2025-06-25 首发 0.1.0，现有每日 nightly 构建。（来源：registry.npmjs.org/@google/gemini-cli time）
- 研发方式：用自家 agent 自动做 issue 分类与 PR review（dogfooding），随后把工作流开源为 Gemini CLI GitHub Actions。（来源：https://blog.google/innovation-and-ai/technology/developers-tools/introducing-gemini-cli-github-actions/）

### Aider
- 起步形态：终端 AI 结对编程工具（"AI pair programming in your terminal"）。（来源：https://aider.chat/）
- 仓库 Aider-AI/aider 创建于 2023-05-09，Python——对照组中最早。（来源：api.github.com/repos/Aider-AI/aider）
- PyPI 包 aider-chat 首个上传 2023-06-08（0.5.0），累计 174 个发布（2023:32、2024:93、2025:48、2026:1）。（来源：pypi.org/pypi/aider-chat/json）
- 技术/功能取向：repo map 映射代码库、自动 git 提交、改动后自动 lint/test 并修错、支持云与本地模型。（来源：https://aider.chat/）

### Goose（Block，现属 AAIF/Linux Foundation）
- 起步形态：Block 开源的通用 agent（不只写代码），桌面 App + CLI + API 三形态。（来源：https://github.com/aaif-goose/goose README）
- 仓库创建于 2024-08-23；现为 Rust 实现（"Built in Rust for performance and portability"）。（来源：api.github.com/repos/aaif-goose/goose；README）
- 技术栈：Rust；15+ 模型提供商；通过 MCP 连 70+ 扩展。（来源：README 同上）
- 归属演进：现为 Linux Foundation 旗下 Agentic AI Foundation（AAIF）项目。（来源：README 同上）
- 由 Python 改写为 Rust 的说法仅见二手报道 → 待核验。（来源：zdnet.com 2025-01-28 报道，二手）

### Cline
- 起步形态：VS Code 插件（市场条目 ID 至今为 saoudrizwan.claude-dev，即早期 Claude Dev 插件）。（来源：https://docs.cline.bot/cline-overview.md 内链接 marketplace.visualstudio.com/items?itemName=saoudrizwan.claude-dev）
- 仓库 cline/cline 创建于 2024-07-06，TypeScript。（来源：api.github.com/repos/cline/cline）
- 现形态：同一 agent core 上的桌面 App + CLI + VS Code/JetBrains 插件 + TypeScript SDK。（来源：https://docs.cline.bot/cline-overview.md）
- 设计原则：human-in-the-loop，每个动作需用户显式批准。（来源：同上）

## 五、大厂研发方法论（一手工程博客/文档）

### Anthropic
- 《Building effective agents》（2024-12-19）：最成功的实现用"简单、可组合的模式"而非复杂框架；区分 workflows（LLM 与工具按预定义代码路径编排）与 agents（LLM 自主决定过程与工具使用）。（来源：https://www.anthropic.com/engineering/building-effective-agents）
- 同文建议：从直接调 LLM API 开始（多数模式几行代码能实现）；框架会加抽象层、遮蔽 prompt 与响应、难调试。（来源：同上）
- 同文方法论："找到最简单的可行方案，只在需要时增加复杂度"，有时意味着不做 agent 系统。（来源：同上）
- 《Effective context engineering for AI agents》（2025-09-29）：context 是有限资源，token 增多会出现 "context rot"（注意力稀释/困惑）；工程重心从"写 prompt"移到"每次推理前策展整个上下文状态"的迭代过程。（来源：https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents）
- 同文结论：'do the simplest thing that works' 仍是给 agent 团队的最佳建议。（来源：同上）
- 《How we built our multi-agent research system》（2025-06-13）：从原型到生产的教训集中在系统架构、工具设计、prompt 工程；子代理并行、各自持独立上下文窗口，先压缩再汇总给主研究 agent。（来源：https://www.anthropic.com/engineering/built-multi-agent-research-system）
- 《Harness design for long-running application development》（2026-03-24）：harness 设计对长时程 agentic coding 性能影响显著；实证做法是任务分解为可管理块、用结构化 artifact 跨会话交接上下文、planner/generator/evaluator 三代理结构、把"设计好不好"这类主观判断改写成可评分标准。（来源：https://www.anthropic.com/engineering/harness-design-long-running-apps）

### Google
- 发布 Gemini CLI 时强调终端优先、开源可检视、与 Gemini Code Assist 共享技术栈（同一能力进 VS Code 与终端）。（来源：https://blog.google/innovation-and-ai/technology/developers-tools/introducing-gemini-cli-open-source-ai-agent/）
- 研发方式实证：用 Gemini CLI 自动做 issue 分类与 PR review，应社区要求把该工作流开源为 GitHub Actions——先自用再产品化。（来源：https://blog.google/innovation-and-ai/technology/developers-tools/introducing-gemini-cli-github-actions/）

### OpenAI
- 《A practical guide to building agents》PDF 确实存在（HTTP 200，7.3MB，Last-Modified 2025-04-11）；本次调研无法提取其正文（PDF 文本层为自定义字体编码，openai.com 网页需 JS 挑战），故其具体主张不进入结论 → 待核验。（来源：https://cdn.openai.com/business-guides-and-resources/a-practical-guide-to-building-agents.pdf）

## 六、横向对比表

| 对象 | 起步形态 | 语言/栈 | UI 形态 | 开发方式 |
|---|---|---|---|---|
| Hermes Agent | 2025-07 单脚本 Python 工具循环（5 文件） | Python + SQLite/FTS5，ui-tui 为 TS | CLI/TUI + 消息网关 + cron + 子代理 | PR 驱动 + TODO 规划 + 文档先行，提交量 2026 起放量 |
| OpenCode | 2025-04 TUI + LSP 的终端 agent | TypeScript monorepo，client/server，OpenAPI 契约 | TUI/CLI/Web/IDE/桌面 | 社区 PR 早入 + 每提交一个 dev 构建 + v2 大版本 |
| Pi | 2025-08 统一 LLM API 包先行，2025-11 CLI | TypeScript 四包 monorepo | 自研 retained-mode TUI（pi-tui），SDK 可复用核心 | 单作者极简路线 + 博客留痕 + Terminal-Bench 自评 |
| Claude Code | 2025-02 research preview 发布 | TypeScript | 终端 CLI（后扩 IDE/桌面） | 闭源高频发版（0.2.x → 2.1.287），官方方法论公开 |
| Codex CLI | 2025-04 开源终端 agent | Rust 核心（codex-rs）+ TS CLI + Bazel | 终端 + IDE + 桌面 + Web | 开源仓库 + 独立二进制发布 + SDK |
| Gemini CLI | 2025-06 开源终端 agent | TypeScript，Apache 2.0 | 终端 CLI + GitHub Actions | nightly 构建 + 用自家 agent 做 triage/review |
| Aider | 2023-05 终端结对编程 | Python | 终端 CLI（可嵌编辑器） | 高频 PyPI 发布（174 版）+ 功能取向明示 |
| Goose | 2024-08 通用 agent（Block） | Rust | 桌面 App + CLI + API | 开源社区 + 基金会化（AAIF），MCP 扩展生态 |
| Cline | 2024-07 VS Code 插件 | TypeScript | IDE 插件 + CLI + 桌面 + SDK | 公开 issue/feature 请求驱动，human-in-the-loop 原则 |

## 七、共性结论（从零起步的共同路径）

1. 起步形态高度收敛：一个终端进程 + 一个"工具调用循环" + 最小工具集（读文件/写文件/改文件/跑命令），Hermes、Pi、OpenCode、Codex CLI、Aider 皆如此。
2. 模型接入层是第一道抽象：要么直接用 OpenAI 兼容客户端（Hermes 首提交），要么自建统一多提供商 API 并配跨提供商测试（Pi 的 pi-ai）。
3. 抽象都是后补的：toolset、架构文档、SDK、插件体系均出现在能跑通的最小版本之后，没有一个项目是先有完整框架再填实现。
4. 单入口长成多入口：先 CLI/TUI，后加 server 化（OpenCode、Cline、Goose、Hermes 网关）与桌面/IDE 形态；核心循环保持单一实现。
5. 状态与记忆分层落地：先会话持久化（SQLite/本地存储），后记忆/检索/技能；持久化是多端与长时程的前提。
6. 工程反馈通道三件套：LSP/测试/评测（OpenCode 的 LSP 诊断回流、Aider 的 lint/test 自动修、Pi 与各家的 Terminal-Bench 类基准）。
7. 发布节奏快且连续：dev 构建 / nightly / 版本号迭代上百次是常态；留痕方式包括 changelog、博客长文、提交正文。
8. 大厂方法论与社区实践一致：简单可组合、按需增复杂度、上下文工程优先、任务分解 + 结构化产物交接、评测驱动。
9. "不做什么"被明确写下（Pi 的无 plan/无 MCP/无子代理；Hermes 的"新工具极少需要，能力做技能"），用来对抗功能蔓延。

## 八、来源清单

本地一手（Hermes，官方仓库的本地 git 副本，路径从略）：
- git log（首提交 21d80ca68，2025-07-22；README 6d346250b，2025-07-26；run_agent.py 首版）
- 本地 website/docs/developer-guide/architecture.md、CONTRIBUTING.md、README.md

网页一手：
- https://hermes-agent.nousresearch.com/docs
- https://github.com/sst/opencode（README）
- https://api.github.com/repos/sst/opencode、https://api.github.com/repos/anomalyco/opencode（元数据、提交历史、包结构）
- https://opencode.ai/docs/、https://opencode.ai/docs/server/、https://opencode.ai/docs/lsp/
- https://registry.npmjs.org/opencode-ai（time 字段）
- https://pi.dev/
- https://mariozechner.at/posts/2025-11-30-pi-coding-agent/（作者原话）
- https://api.github.com/repos/earendil-works/pi
- https://registry.npmjs.org/@mariozechner/pi-ai、https://registry.npmjs.org/@mariozechner/pi-coding-agent
- https://www.anthropic.com/news/claude-3-7-sonnet
- https://www.anthropic.com/engineering/building-effective-agents
- https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- https://www.anthropic.com/engineering/built-multi-agent-research-system
- https://www.anthropic.com/engineering/harness-design-long-running-apps
- https://github.com/openai/codex（README、仓库结构）
- https://api.github.com/repos/openai/codex
- https://cdn.openai.com/business-guides-and-resources/a-practical-guide-to-building-agents.pdf（存在性核验）
- https://blog.google/innovation-and-ai/technology/developers-tools/introducing-gemini-cli-open-source-ai-agent/
- https://blog.google/innovation-and-ai/technology/developers-tools/introducing-gemini-cli-github-actions/
- https://github.com/google-gemini/gemini-cli（README）、https://api.github.com/repos/google-gemini/gemini-cli
- https://registry.npmjs.org/@google/gemini-cli、https://registry.npmjs.org/@anthropic-ai/claude-code
- https://aider.chat/、https://pypi.org/pypi/aider-chat/json、https://api.github.com/repos/Aider-AI/aider
- https://github.com/aaif-goose/goose（README）、https://api.github.com/repos/aaif-goose/goose
- https://docs.cline.bot/cline-overview.md、https://api.github.com/repos/cline/cline

二手（仅旁证，已标注）：
- https://www.zdnet.com/article/blocks-new-open-source-ai-agent-goose-lets-you-change-direction-mid-air/（Goose Python→Rust）
- https://x.com/badlogicgames（作者推文，正文未抓全）

## 九、待核验清单

1. Pi 的"418 行 agent loop"说法：只见于第三方解读（torchtree.com 等引用），未在作者原文/仓库中核到 → 待核验。
2. Pi 会话为树状、可分支/fork/回滚：只见于第三方博客（htdocs.dev 等）→ 待核验。
3. Goose 由 Python 改写为 Rust：仅 zdnet.com 二手报道，未找到 Block/AAIF 一手说明 → 待核验。
4. OpenAI《A practical guide to building agents》的具体主张：PDF 存在已核验，正文本次无法提取，不据其下任何结论 → 待核验。
5. Codex CLI "2025-04-16 发布"的确切日期：openai.com 正文抓取受阻（JS 挑战），仅有搜索摘要与 Wikipedia（二手）→ 待核验；可核验的是仓库创建日 2025-04-13。
6. OpenCode 早期 TUI 的实现语言（第三方 deep-dive 称曾为 Go TUI + JS server）：本次仅一手核到"当前 monorepo 主语言 TypeScript、TUI 与 server 分进程"，早期语言归属 → 待核验。
7. OpenCode 由 SST 更名/迁移至 Anomaly（anomalyco）的官方公告：仅核到 GitHub 仓库重定向事实，未找到公告原文 → 待核验。
8. Cline 早期名"Claude Dev"的一手声明：仅由 VS Code 市场条目 ID（saoudrizwan.claude-dev）旁证，未见官方历史说明 → 待核验。
9. Claude Code 闭源部分的研发流程（团队如何迭代/评测）：无一手公开材料 → 未覆盖。
10. Hermes 提交量统计按 git 作者日期聚合，含机器/批量提交，仅作趋势参考，不作精确指标。
11. 作者推文（x.com/badlogicgames）称 pi 在 Terminal-Bench 获第 8 名：正文未抓全，仅搜索摘要 → 待核验（作者博客仅自述做过 Terminal-Bench 2.0 对比测试）。
