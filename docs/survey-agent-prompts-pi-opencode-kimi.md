性质：调研文档（含第三方提示词引用，非纯净文档）
# 业界 Agent 系统提示词考古：Pi / OpenCode / Kimi CLI
来源：github.com/WEIFENG2333/phistory（MIT）版本化快照 captures/{pi,opencode,kimi}，辅以三家源码仓库的 commit/issue 证据。快照中的 $PHISTORY_* 为抓取工具的环境变量占位。token 量为估算值（英文字符数 ÷ 3.3，只计系统消息；标注"全量"者含工具描述）。行文引用第三方原文均注明出处。

## 0. 结论（先看这段）

1. **系统消息规模三档**：Pi ≈ 0.8K token（全量 ~1.8K）；OpenCode ≈ 3.7K（全量 ~9.9K）；Kimi CLI ≈ 4.4K（全量 ~15.8K）。差 5 倍，但都自洽——规模由"面向谁+防线放哪"决定，不由模型强弱决定。
2. **工具描述是 token 大头**（占全量 53%~72%，按各家自报数据重算：Pi 3,165/6,013≈53% / OpenCode 20,558/32,780≈63% / Kimi 37,356/52,102≈72%），是经济性优化的第一现场：OpenCode 在 1.16.0 把工具区砍掉 35%，Pi 干脆只有 4 个工具。
3. **静态/动态分离是共识**：动态内容（日期、env、目录树、skills 清单）要么追加在前缀尾部，要么直接删（Pi 删日期、OpenCode 删 `<directories>`）——目的都是 prefix cache 命中率，有 commit/issue 直接证据（pi issue #6621、opencode #19487/#20771）。
4. **引导设计两派**：OpenCode 重"流程剧本"（10 步 Workflow + todo 断点续 + 强制联网研究）；Kimi CLI 重"行为守则 + 任务分类"（什么时候动手、什么时候直接答、MINIMAL changes、收口口诀）；Pi 几乎零流程引导，只给工具使用规则。
5. **防御写法一家一个样**：Kimi CLI 最完整——通道分层（`<system-reminder>` = 权威指令）+ 非沙箱声明 + 越界禁令 + git 每次确认 + 隔离环境装包；OpenCode 只有 git 安全协议；Pi 一字不写，全部押在 harness 层。
6. **演化规律**：骨架一旦定型就冻结（Kimi 1.40→1.51、OpenCode 1.4→1.18 系统消息几乎不动），增量全走"功能上线→提示词加一段/加一行 doc 引用"；删减动机有据可查的全是"缓存失效"与"指引与真实工具集不符"两类。

## 1. Pi（earendil-works/pi，npm @earendil-works/pi-coding-agent）——经济性标杆

快照：captures/pi，54 版（0.74.0 → 1.0.2，1.0.2 发布 2026-10-04）。

### 1.1 最新版（1.0.2）骨架与篇幅

系统消息 2,715 字符 ≈ 0.8K token；全量 prompt.md 6,013 字符（工具 schema 3,165）。

| 分节 | 字符 | 占比 | 内容 |
|---|---|---|---|
| 角色一句话 | 170 | 6% | "You are an expert coding assistant operating inside pi, a coding agent harness."（出处：captures/pi/1.0.2/prompt.md） |
| `<tools>` | 330 | 12% | 4 个工具一行一个：read / bash / edit / write，加一句"可能有项目自定义工具" |
| `<rules>` | 830 | 31% | 10 条：bash 干 ls/rg/find；read 替代 cat/sed；可查 PI_* 环境变量；edit 精确替换、多处改动合一次调用、oldText 对原文件匹配且尽量短小、禁重叠；write 仅新文件/全重写；回复简洁；显示文件路径 |
| `<docs>` | 1,320 | 49% | 自文档路径清单（README/docs/examples），"read only when the user asks about pi itself"，按需读 |
| `<cwd>` | 25 | 1% | 工作目录 |

工具描述区另计：bash/edit/read/write 各带 JSON schema；edit 的描述里再强调一遍"每次 oldText 对原文件匹配、就近合并、勿带大段未变内容"。

### 1.2 演化与归因（diff + commit 证据）

- **结构标签化**：0.74.0–0.85.0 用 `Available tools:` / `Guidelines:` / `##` 分节；0.87.1 起改为 `<tools>/<rules>/<docs>/<cwd>`（实测 0.85.0 无标签、0.87.1 有）。归因（commit 2026-05-17/18，earendil-works/pi）："use xml boundaries instead of using `##` so that agents are less likely to ingest a prompt with unclear/inconsistent boundaries"——边界清晰、系统与上下文文件合并时边界一致。
- **删日期**：0.80.5 有 `Current date: …`，0.85.0 起消失。归因（commit 2026-07-14 "remove current date from system prompt, fixes #6621"；issue #6621 标题 "Prevent accidental cache invalidation due to dynamic system prompt"）：本地推理 prefill 慢，动态日期毁前缀缓存。
- **删失真指引**：commit 2026-05-28 "Remove unavailable tool preference guideline, closes #5132"；issue #5132：提示词硬编码 "Prefer grep/find/ls tools over bash"，但这些工具未必注册，"the model is told to prefer tools that don't exist"。归因：指引必须与真实工具集一致，否则砍。
- **docs 清单增量**：0.85.0 +environment-variables、0.99.x +mcp、1.0.x +codemode。归因（commit 2026-01-26 "expand pi documentation references in system prompt"）：列出理由——要求读完整 .md 并跟随交叉引用、按功能加引用条目。即"每上线一个功能子系统加一行引用，主体不动"。
- **token 经济意识明示**：commit 2026-10-01 "shrink codemode prompt…"："Move the codemode script reference… into docs/codemode.md and keep only one line per global in the tool description… With the default tools a GPT-5.6 request drops from about 5,300 to 3,300 tokens."（出处：earendil-works/pi commit message）——把长参考移出提示词、按需读文档，有量化验收。

### 1.3 精简性分析：砍了什么、留了什么

砍掉（对照 OpenCode/Kimi 的同类内容）：Workflow 流程剧本、todo/计划工具、联网研究、沟通风格与示例、记忆机制、git 规则、安全/沙箱/权限声明、反注入措辞、AGENTS.md、skills、子代理——一律不进系统提示词。
留下：工具契约（含 edit 的精确匹配细则）、最小使用规则、按需自文档入口。
方法论佐证：pi 仓库自带提示模板 `.pi/prompts/deslop.md`（"Remove speculative defensive programming inside trusted code. Validate at real trust boundaries"）——精简美学贯穿其工程文化。**Pi 的做法是把引导和防御外包给 harness（工具白名单、参数 schema、扩展机制）**[其内部意图待核验]，提示词只管"怎么用工具"。

### 1.4 引导与防御

- 引导：仅工具习惯（read 优先于 cat、一次 edit 多段、oldText 最小化、write 只做全量）+ "Be concise"。零流程引导。
- 防御：**系统提示词层面为零**（安全词频扫描：sandbox/injection/untrusted/cautious/destructive 全部 0 次）。安全完全依赖 harness 与人工审批层。

## 2. OpenCode（opencode-ai，npm）——流程剧本派

快照：captures/opencode，118 版（1.4.0 → 1.18.34，1.18.34 发布 2026-09-30）。

### 2.1 最新版（1.18.34）骨架与篇幅

系统消息 12,087 字符 ≈ 3.7K token；全量 32,780 字符（工具区 20,558，10 个工具）。

| 分节 | 字符 | 占比 | 内容 |
|---|---|---|---|
| 开场执行力宣言 | 3,725 | 31% | "You are opencode, an agent - please keep going until the user's query is completely resolved"；"You MUST iterate and keep going"；"THE PROBLEM CAN NOT BE SOLVED WITHOUT EXTENSIVE INTERNET RESEARCH"；"Your knowledge on everything is out of date"；"You can definitely solve this problem without needing to ask the user"（出处：captures/opencode/1.18.34/prompt.md） |
| `# Workflow` | 4,696 | 39% | 10 步编号流程 + 7 个详节（抓 URL→理解问题→查代码库→联网研究→计划→改码→调试） |
| `# Communication Guidelines` | 911 | 8% | 语气 + `<examples>` 6 句对话示例；"Do not display code to the user unless they specifically ask" |
| `# Memory` | 589 | 5% | 记忆存 `.github/instructions/memory.instruction.md`，front matter `applyTo: '**'` |
| `# Reading Files and Folders` | 611 | 5% | 禁重复读文件（省 token 的显式规则） |
| `# Writing Prompts` | 403 | 3% | 提示词/todo 必须 markdown + 三反引号 |
| `# Git` | 1,146 | 10% | "You are NEVER allowed to stage and commit files automatically." + 动态 `<env>`/`<available_skills>` 尾巴 |

工具区大头：bash 5,117（含 "Git and GitHub" 短协议）、task 3,879、todowrite 2,829（When to use / When NOT to use / States / Rules / Examples 五段）。

### 2.2 演化与归因

- **系统消息冻结**：1.4.0（2026-04-08）→1.18.34，正文 11,596→12,087 字符，差异仅：删 `<directories>` 块（1.14.17 前）、日期与 skills 槽位内容变化、skill location 格式微调。骨架（10 步 Workflow + 各节）一字未动。
- **1.16.0（2026-06-05）工具描述大瘦身**：工具区 35,335→23,026（-35%），到 1.18.34 再降到 20,547。bash 10,624→5,779、todowrite 9,794→2,890、task 5,400→3,940；skill 从"No skills are currently available"占位变为通用加载说明（1,741）。砍掉的内容可核对：约 50 行 Claude Code 风格 "Git Safety Protocol"（含 --amend 五条件、--no-verify 禁令等）换成 8 行短协议（"Only commit, amend, push, or create PRs when explicitly requested… Do not update git config, skip hooks, use interactive -i, force-push…"）；todowrite 长篇散文换要点清单。**本次瘦身的直接 commit 动机未检索到，标 [待核验]**；邻近证据显示该项目长期为缓存优化提示词：commit 2026-03-27 "tweak: adjust bash tool description to increase cache hit rates between projects (#19487)"、2026-04-02 "fix: rm dynamic part from bash tool description again to restore cache hits across projects (#20771)"、2025-12-15 "rejoin system prompt … to preserve caching (#5550)"、2026-06-19 "tweak: remove steering wrapper that can bust cache (#33039)"（出处：anomalyco/opencode commit 历史）。
- **1.16.0 新增**：`/tmp/opencode` 临时区说明（对应 commit 2026-04-30 "core: clarify that temp directory already exists for AI agents"）与 `<available_skills>`（skill 系统上线）。

### 2.3 引导与防御

- 引导（重流程）：固定 10 步 + 每步详节；todo 用 markdown `[x]` 勾选、每勾一步展示更新；"resume/continue/try again" 时从 todo 最后未完成项续做；执行纪律密集（"when you say you will make a tool call, actually make it"、"NEVER end your turn without…"）；强制递归抓取链接 + "用 Google 验证第三方库用法"；改码前先读 2000 行；`.env` 缺失时自动建占位文件并告知用户（注意：这是主动写入动作，与"不越界"哲学相悖，属产品取舍 [待核验其理由]）。
- 防御（薄）：git 破坏操作防线（上文短协议）；临时文件指定 /tmp/opencode；"Do not display code to the user unless asked"。**无任何反提示注入/不可信内容措辞**（injection/untrusted/malicious 0 次）。

## 3. Kimi CLI（MoonshotAI/kimi-cli，GitHub 发布）——行为守则 + 通道分层派

快照：captures/kimi，23 版（1.0 → 1.51.0，1.51.0 发布 2026-09-21）。命名澄清（三者交叉易混，引用一律以仓库坐标为准）：本节对象 = 仓库 MoonshotAI/kimi-cli（现为 archived），其提示词自称 "Kimi Code CLI"；GitHub 归档说明所称后继产品 "Kimi Code CLI"（MoonshotAI/kimi-code）是另一仓库/产品。phistory 另有 captures/kimi-code（@moonshot-ai/kimi-code），与本节对象是两个产品，未纳入本表。
提示词源码：src/kimi_cli/agents/default/system.md（MoonshotAI/kimi-cli）。

### 3.1 最新版（1.51.0）骨架与篇幅

developer message 14,610 字符 ≈ 4.4K token；全量 52,102 字符（工具区 37,356，15 个工具，Agent 5,784 / Shell 4,330 / Grep 4,516 为大头）。

| 分节 | 字符 | 占比 | 内容 |
|---|---|---|---|
| 角色+目标 | 382 | 3% | "You are Kimi Code CLI, an interactive general AI agent running on a user's computer"；"help users with software engineering tasks by taking action"（出处：captures/kimi/1.51.0/prompt.md） |
| `# Prompt and Tool Use` | 4,527 | 31% | 任务分类（简单问答直接答、其余动手；"When the request could be interpreted as either a question or a task, treat it as a task"）；工具调用不解释；Agent 委托规则（新实例带全上下文、优先 resume、默认前台）；并行调用；`<system>` 与 `<system-reminder>` 通道语义；Background Bash 用法；统一审批运行时；"use the SAME language as the user" |
| `# General Guidelines for Coding` | 2,444 | 17% | 从零构建 4 步；工具落地三则（代码写文件才算数、Shell 跑测试、失败迭代）；改现有代码分 bugfix/feature/refactor 三套路；"Make MINIMAL changes to achieve the goal"；explore 子代理触发阈值（>3 次搜索）；git 变更禁令 |
| `# … Research and Data Processing` | 1,123 | 8% | 先计划再研究；精准搜索；装第三方包必须虚拟/隔离环境；产出多媒体文件后回读核对；"Avoid installing or deleting anything to/from outside of the current working directory" |
| `# Working Environment` | 1,361 | 9% | OS/shell 声明；**"The operating environment is not in a sandbox… you MUST be extremely cautious… never access (read/write/execute) files outside of the working directory"**；日期；工作目录 + 目录树（"… and N more" 提示） |
| `# Project Information` | 1,959 | 13% | AGENTS.md 机制（含 Why AGENTS.md 引文）：多级文件合并、深目录优先、对话内用户指令最高、改了 AGENTS.md 涉及物必须同步更新 |
| `# Skills` | 2,040 | 14% | skills 概念 + scope 分组（Project>User>Extra>Built-in）+ "Only read skill details when needed to conserve the context window" |
| `# Ultimate Reminders` | 767 | 5% | 收口口诀："ALWAYS, keep it stupidly simple"；"Never give the user more than what they want"；"Try your best to avoid any hallucination. Do fact checking"；"Think about the best approach, then take action decisively"；"Never treat displaying code in your response as a substitute for actually writing it" |

### 3.2 演化与归因（diff + kimi-cli commit 证据）

- **1.0–1.6（2026-01/02）**：9,327 字符、9 工具（含 Task）。人格句 "HELPFUL and POLITE, CONCISE and ACCURATE, PATIENT and THOROUGH"、"Think twice before you act"、目标是 "answer questions and/or finish tasks safely and efficiently"。
- **1.35.0（2026-04-15）：14,210 字符、15 工具**——最大一次跳变，逐条可归因到提交：
  - plan mode（EnterPlanMode/ExitPlanMode 工具 + What Happens in Plan Mode）← feat: add plan mode (#1392, 2026-03-10)；
  - Background Bash + TaskList/TaskOutput/TaskStop ← feat: add background bash tasks and notification infrastructure (#1477, 03-17)；
  - Agent 委托 + "unified approval runtime" 措辞 ← refactor(subagents): unify subagent execution, approvals, and tracing (#1552, 03-23)；
  - OS/shell 注入声明 ← fix(system): inject OS and shell info into system prompt for Windows compatibility (#1673, 03-31)；
  - `subagent_type="explore"` ← feat(explore): enhance explore agent… (#1675, 03-31)；
  - AGENTS.md 层级合并 ← feat(agents): hierarchical AGENTS.md loading (#1700, 04-01)；
  - `<system-reminder>` 权威通道 ← 该版新增（另有提交 "strengthen agent system prompt" #1575, 03-25），精确动机 [待核验]；
  - 语气转向："Think twice before you act" → "Think about the best approach, then take action decisively"；目标句从"安全高效答问题"改为"by taking action — make real changes on the user's system"；新增"MUST use tools … do not just describe the solution in text"。
- **1.40.0（2026-04-28）**：+400 字符，skills 按 scope 分组 + 优先级 ← fix(skill): scope-group the skills system prompt and honor project overrides (#2044, 04-24)。目录树截断提示 ← fix(context): cap list_directory to 500 entries (GH-1809) (#1827, 04-10)。
- **1.40.0→1.51.0 系统消息冻结**（仅 skill 路径里 python3.13→3.12 之类环境回显差异）。Shell 后端 PowerShell→git-bash 为 2026-05-09 commit #2186（提示词不动，动态注入）。
- 归纳：**Kimi 的提示词增长 = 功能清单映射**，每加一个运行时能力（计划模式/后台任务/子代理/审批）就在提示词加一段"该能力的使用契约"，然后整体冻结。

### 3.3 引导与防御

- 引导（重守则）：任务分类学（答 vs 做，歧义当任务）；"代码必须落盘"；构建/修 bug/加功能/重构四类各有套路；"MINIMAL changes"；研究类先计划、产出后回读核对；收口口诀（KISS、不过度交付、事实核查、果断行动）。比 OpenCode 少"步骤剧本"，多"决策规则"。
- 防御（三家最全）：
  1. **通道分层**：`<system-reminder>` = "authoritative system directives that you MUST follow… they may override or constrain your normal behavior"，与 `<system>` 补充信息严格区分（出处：captures/kimi/1.51.0/prompt.md）——可信指令走专属标记，是反注入的结构性手段；
  2. **非沙箱声明 + 越界禁令**（Working Environment 节，见上）；
  3. Shell 工具 "Guidelines for safety and security"：不用 `..` 越界、不改工作目录外文件、"Never run commands that require superuser privileges unless explicitly instructed to do so"；
  4. **git 变更每次确认**："DO NOT run `git commit`, `git push`, `git reset`, `git rebase` … unless explicitly asked. Ask for confirmation each time … even if the user has confirmed in earlier conversations"；
  5. 第三方包必须装进虚拟/隔离环境；
  6. 权限模式语义写进工具描述（"Yolo mode only bypasses permission approval. It does not make the session non-interactive"；afk 模式禁交互工具）。
- 未见对网页/文件中恶意指令的直接措辞；kimi-cli 仓库存在测试 tests/core/test_skip_afk_prompt_injection.py，说明 afk 模式有注入防护测试，具体机制 [待核验]。

## 4. 横向对照表

| 维度 | Pi | OpenCode | Kimi CLI |
|---|---|---|---|
| 骨架 | 角色句 + `<tools>/<rules>/<docs>/<cwd>` 四标签 | 执行力宣言 + 10 步 Workflow（7 详节）+ 4 个杂项节 + 动态 env 尾巴 | 能力契约 6 节（工具用法/编码/研究/环境/项目/skills）+ 收口口诀 |
| 篇幅（系统消息 / 全量，token 估算） | 0.8K / 1.8K | 3.7K / 9.9K | 4.4K / 15.8K |
| 引导 | 仅工具使用规则，零流程 | 流程剧本：步骤、todo 断点续、执行纪律、强制联网 | 行为守则：任务分类、四类套路、MINIMAL、KISS 口诀 |
| 防御 | 无（押 harness 层） | git 短协议 + 临时目录隔离 | 通道分层 + 非沙箱声明 + 越界/提权禁令 + git 每次确认 + 隔离装包 |
| 演化规律 | 标签化→删动态内容（缓存）→删失真指引→docs 行随功能增；删减有 token 量化验收 | 正文冻结 118 版；1.16.0 工具描述 -35%（动机[待核验]，邻近 commit 均为缓存） | 骨架冻结；增长=功能上线→加一段使用契约（逐条 commit 可对） |

## 5. 经济与性能平衡点

1. **预算落点**：三家把 53%~72% 的全量 token 花在工具描述上（自报数据重算，见 §0.2）。优化顺序应为：工具数量与描述（Pi 4 个工具 / OpenCode 1.16.0 砍 35%）→ 静态前缀稳定化 → 才考虑砍系统消息正文。
2. **缓存纪律**（双方有硬证据）：动态内容（日期/env/目录树/skills）绝不进前缀头部；能删则删（pi issue #6621；opencode #19487/#20771/#5550）。骨架冻结本身就是性能设计。
3. **引导密度与用户群匹配**：Pi 假定用户是开发者、harness 有护栏，提示词可以只有 0.8K；Kimi 面向通用用户且明示无沙箱，必须用 ~4.4K 行为守则补安全下限。规模不是越小越好，是"防线放哪"的问题。
4. **执行纪律条文是共识投入**："说到就调工具/别中途停/别问用户"类条文在 OpenCode（31% 开场）与 Kimi（收口口诀）都占显著篇幅，Pi 完全不写——agentic 增强是否写进提示词，取决于 harness 是否另有机制 [待核验]。
5. **对本项目的启示**（供参考，非需求）：骨架与缓存纪律取 Pi 式（标签分节 + 静态/动态分离）；安全取 Kimi 式（可信指令通道 + 越界/提权禁令 + 破坏性操作逐次确认）；流程引导按用户群折算，勿默认抄 OpenCode 的全量剧本。

## 6. 未核验项汇总

- OpenCode 1.16.0 工具描述大瘦身的直接 commit 动机（仅获邻近缓存优化提交，未获对应提交）。
- Kimi CLI `<system-reminder>` 通道的引入动机（1.35.0 新增，未获对应提交说明）。
- Kimi CLI afk 模式注入防护（仅见测试文件名 test_skip_afk_prompt_injection.py，机制未读）。
- Pi 把安全完全外置给 harness 的意图（提示词零防御为事实，意图属推断）。
- OpenCode 自动创建 .env 占位文件的理由。
- captures/kimi-code（@moonshot-ai/kimi-code）未纳入分析范围。
