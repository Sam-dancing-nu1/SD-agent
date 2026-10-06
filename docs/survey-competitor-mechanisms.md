性质：调研文档（含第三方名称引用，非纯净文档）

# 竞品机制调研：pi-agent 生态 / Codex Agent Tree（JEV fork layer + ASTRA）/ OpenCode TUI

调研日期：2026-10-05。方法：本地克隆源码只读分析（earendil-works/pi、sst/opencode，克隆于 $TMPDIR，未写入本仓库）+ GitHub raw 读取（hpvdev/agent-tree-workflow 安装器/控制器/测试）+ 网络检索。凡出处不明的概念一律标注 [待核验]。对照基准：docs/concept-v2.md（十二组件、21 硬约束）。

## 零、结论速览

1. **pi-agent 生态**（earendil-works/pi，TypeScript/Bun，OpenClaw 的底层 SDK）是一个"最小内核 + 示例扩展库"形态的 Agent 工具箱：内核只给会话/工具管线/事件/压缩/持久化骨架，子代理、计划模式、todo、权限门、记忆类功能全部以扩展示例形式外置。对我们的十二组件，它印证了"低耦合插件面 + 工具管线统一拦截"两件已有方向；我们缺的"痕迹层条目化结构"可从其 durable 条目模型取形态参考；其无内置记忆层的取舍与我们"记忆是纪律不是聊天历史"一致，不照搬。
2. **Codex Agent Tree 的 JEV fork layer 与 ASTRA 是真实可运行实现**（社区 Codex skill，非官方功能），可核验文本源头为 hpvdev/agent-tree-workflow 的 templates/control.py 文档串"Executable Jev fork layer plus audited Astra checkpoints"。它把"选谁做/看哪个文件/用哪个工具/重试还是停"四类有界微决策交给不出文本、返回类型化选择+校准概率的快决策模型，高置信（sharp）直接代码派发、split（未过三闸）回主模型带证据裁决（低置信或不可逆动作再升级人工），并有 forks/sharp/split 记账。**设计价值与我们探针"贴标签+置信度→配置查表裁决"同构**，但其决策层是外部付费快模型，我们按硬约束 13 走本地规则/分类器+查表，只取其 sharp/split 双路结构与记账形态。
3. **OpenCode TUI**（sst/opencode）现版本为 client-server 架构、TUI 用 SolidJS + 自研终端渲染器（@opentui）；主页 = 大 Logo + 空白对话框 + 占位建议，与我们 TUI 优先形态同形，可直接对齐；多会话/工作区分组、鼠标可配置开关、权限三段式内联面板、实时 token 占用显示都可作组件级参考。它是单终端多路由，**"双终端观察面/工作面物理分离"三者皆无先例**，是我们的差异化设计。
4. 三个直接设计参考：TUI 主页形态照 OpenCode；本地路由层照 JEV fork layer 的 sharp/split + 记账结构（决策层保持本地+查表）；双终端观察面按"只读订阅网关事实源"实现（与 OpenCode 多前端经 HTTP+SSE 取数同构）。

## 一、pi-agent 生态：功能簇全景与 SDK 机制

**形态总述**。pi 自述"minimal agent harness"："Pi ships with powerful defaults but skips features like sub-agents and plan mode. Ask Pi to build what you want, or install a package that does it your way."（README 原话）。仓库含 14 个包：ai（统一 LLM API）、agent（agent loop）、coding-agent（CLI/TUI/会话内核）、tui、mcp、telemetry、durable（持久化条目模型）、evals、chord、server/client/rpc 等。README 提及 OpenClaw 是"real-world integration"，即 pi 是被 OpenClaw 嵌入的底层 SDK。

**扩展怎么挂载**：扩展 = 默认导出函数，接收 ExtensionAPI（`export default function (pi: ExtensionAPI)`）；加载方式两种——`pi --extension <路径>` 或放入 `~/.pi/agent/extensions/` 自动发现，jiti 加载 TS 源码（examples/with-deps 演示带独立 package.json 的扩展）。扩展面（core/extensions/types.ts，1900+ 行类型）覆盖：注册工具/虚拟模型/MCP 服务器、改系统提示、注册消息与条目渲染器、UI 部件（setWidget/setFooter/setHeader/自定义编辑器）、键位、斜杠命令、资源发现。

**事件订阅怎么用**：`pi.on("事件名", handler)` 订阅生命周期事件（tool_call / tool_result / session_start / model_select / project_trust / input / resources_discover 等）；`tool_call` 钩子返回 `{block, reason}` 即阻断工具执行——同步决策、异步观察的分工与我们硬约束 14 同构。另有进程内事件总线 `pi.events`（emit/on，on 返回退订函数，handler 异常被隔离不拖垮总线——与我们"订阅者各自熔断"要求同构）。

各功能簇的机制设计与三态标注：

1. **上下文压缩**（对照组件 5）→ **已有**。core/compaction/：压缩逻辑是纯函数（compaction.ts），I/O 与重载由会话管理器承担；压缩摘要不是泛泛总结——utils.ts 先做 **File Operation Tracking**（从消息流抽取文件操作、computeFileLists、SUMMARIZATION_SYSTEM_PROMPT 序列化对话），另含 branch-summarization.ts（分支摘要）。压缩可被扩展替换（examples/custom-compaction.ts）或按阈值触发（trigger-compact.ts：上下文超 10 万 token 触发 + `/trigger-compact` 命令）。印证：压缩是显式可触发事件、摘要须带"动了哪些文件"的结构化事实——正对我们"压缩禁区/长尾约束"关切。
2. **MCP 外部工具**（对照组件 9、5）→ **已有**。extensions/mcp：`mcp.json` + `registerMcpServer()` 双入口，同名时配置优先；连接在后台进行（首提示只等 direct 工具的服务器）；工具名 `mcp__<server>__<tool>`。**暴露四档**：codemode（默认，只供脚本经 searchTools() 调用，不进模型工具声明）/ deferred（tool_search 按需装载后才声明）/ direct（直接声明）/ hidden（不可达），单工具可覆盖。关键注释："Every call runs through pi's tool pipeline, so tool_call/tool_result hooks and permission extensions apply to MCP tools the same way they do to built-in tools."——外部能力与内置工具过同一拦截管线，正是我们"能力进出只过网关、标签、白名单、检查点"的第三方印证。
3. **子 Agent 编排**（对照组件 10、11）→ **已有（原则层）**。内核刻意不做（README 原话见上）；examples/subagent/ 是完整参考实现：每个子代理 = 独立 pi 进程、隔离上下文窗口；角色定义在 markdown（scout 快速侦察回压缩上下文 / planner / reviewer 代码审查 / worker 全能力）；工作流模板（implement = scout→planner→worker；implement-and-review = worker→reviewer→worker）；并行流式输出、每代理 usage 追踪（轮次/token/成本/上下文占用）、Ctrl+C 中止传播杀子进程。印证我们"默认单执行体 + 辅助执行体按需派生、只回压缩摘要、可中止"。
4. **UI 人机交互**（对照组件 8）→ **缺（实现层），方向已有**。question.ts 用 `ctx.ui.select()` 提问、questionnaire.ts 多问表格单（tab 导航）、todo.ts（工具 + `/todos` 命令 + 自定义渲染 + 状态持久化）、timed-confirm.ts（AbortSignal 让确认框限时自动过期）、qna.ts（把上文问题抽进编辑器）、widget-placement/setWidget 部件位。印证我们"界面承担结构化引导：表单/滑块而非自然语言博弈"。
5. **代码质量回环**（对照组件 3）→ **已有（我们形态更强）**。pi 内核无 lint/review 引擎；质量回环靠编排模板（reviewer 代理 + implement-and-review 循环）与守卫扩展（dirty-repo-guard 未提交禁切会话、git-checkpoint 每轮 stash 检查点、auto-commit-on-exit、protected-paths 保护 .env/.git）。注意：其 reviewer 是"模型审模型"，与我们硬约束"运行时批评家必须是代码（测试/编译/静态检查/真库重查）"相悖——只取其循环编排，不取其评审主体。
6. **可观测与权限**（对照组件 6、7）→ **已有**。telemetry 包（index/memory/noop 三实现，可替换）；cache-stats、cache-warmer（带 CacheWarmingDecisionEvent 决策事件）、timings、usage-totals。权限是扩展职责：permission-gate.ts 正则识别危险 bash（rm -rf/sudo/chmod 777）→ 有 UI 弹 select 确认、**无 UI（非交互模式）默认阻断**；tool-override.ts 可给内置工具加审计/访问控制；output-guard.ts、trust-manager/project-trust（项目信任事件）、prompt 侧 project-trust.ts。印证"误报可降级放行、无 UI 从严"的动作分级思路。
7. **记忆持久化**（对照组件 4）→ **我们已有、pi 不做（不照搬）**。pi 无内置记忆层；持久化 = 会话树（session-manager，条目类型 Assistant/ToolResult/User/System/Compaction/Reset Entry）+ durable 包（defineDoc/defineEntry、CompactionCheckpoint、DEFAULT_COMPACTION/PROGRESS/RETRY_POLICY、harness/agent.ts 的 AgentDoc）+ 扩展自有状态（todo/snake 示例）。handoff.ts 演示会话间上下文转移。可取之处仅在**条目化形态**：痕迹层数据结构可参考"typed entry + checkpoint + 策略常量"的写法；其"无记忆层"反向印证记忆不是 SDK 的默认件，我们自建记忆的必要性成立。

## 二、Codex Agent Tree：JEV fork layer 与 ASTRA 机制

**出处考证（真实实现 vs 概念构想）**：判定为**真实可运行的社区实现**，但非官方功能、且最初概念出处（X/博客/视频）未能定位 [待核验]。可核验的证据链：

- "Jev" = TypeSafe AI 于 2026-09-15 发布的 "System One" 决策模型：不生成文本，输入状态 + 类型化问题，返回类型化选择 + 校准概率，宣称 70–500ms、$0.042/MTok（输出免费）、分类任务较通用 LLM 快至 200 倍（均为厂商/二手口径 [待核验]）。
- 同类公开仓库时间线：Madikhan33/jev_codex（2026-09-21，"Context-aware routing for Codex: classify prompts, choose agent profiles, coordinate subagents, and verify results"）→ evgyur/codex-agent-tree（2026-09-29，"Codex skill for configuring and verifying a lean custom-agent tree"，Sol 主会话 + 三个工作角色 + 可选 Astra 审查者）→ **hpvdev/agent-tree-workflow（2026-10-03，本文机制来源）**。即 Jev 发布后约三周内在 Codex skill 生态里长出的一族实现。
- hpvdev/agent-tree-workflow 是完整工程：install.py 安装器（往项目写 .codex/skills/agent-tree/、.codex/agents/agent_tree_*.toml、.agent-tree/，不改 AGENTS.md、不挂全局钩子）、templates/control.py 控制器（文档串即"Executable Jev fork layer plus audited Astra checkpoints"——"JEV fork layer"一词的可核验出处）、observer.py 观察者、tests/（test_forks.py、test_workflow.py）。

**机制拆解**（以下均出自 hpvdev README + templates/workflow.md + templates/control.py 源码）：

- **角色分工**：Sol/main（gpt-6-sol high：编排、计划、集成，仅当 Jev 选中才亲自做）、Worker（sol medium：在指派范围内改码）、Explorer/Researcher（luna medium：读码找调用点 / 查资料）、Astra（gpt-6-astra：只咨询不改码）、Jev（jev-latest：选执行者与有限语义分支）。
- **fork 分类**（微决策点）：`which_agent`（每轮首个 fork 必问，Sol 提供选项含 main 与至少一个有界子代理）、`which_file`、`which_tool`、`retry_or_stop`——且"仅当存在真实备选时才立 fork，已知事实与确定性协调不得编造 fork"。
- **sharp/split 判定**（classify() 函数）：margin = 首选概率 − 次选概率；`sharp = choice ≠ "sol" 且 confidence ≥ 阈 且 P(choice) ≥ 阈 且 margin ≥ 阈`，三闸皆过 → sharp = 直接派发所选本地动作；否则 split = 回 Sol 带证据裁决。阈值来自策略配置（JEV_DEFAULTS = confidence .85 / probability .85 / margin .20 / timeout 15s / max_retries 1，可经 configure 修改）。概念图里的"sharp(≥~0.8)→直接代码执行"与默认 .85 大致吻合；"split(<~0.5)→回主模型"在实现中**不存在 0.5 常量**——split 即"未过 sharp 三闸"，含中间置信带，回退动作是"Sol 收集窄证据→resolve stop→重问 Jev" [待核验：0.5 数值出处不明]。
- **which_agent 特例（归属权）**：Jev 排名最高的非 fallback 选项即使低于通用阈值也定执行者（margin>0 即 sharp）；fallback/并列/失败/超时 → split，且"Sol must not resolve a which_agent split to its own preferred agent"——执行者归属权在决策层，主模型不得趁 split 自我任命。选中子代理后须 `confirm-agent` 由观察者核对子会话的角色/模型/父子关系/创建时间，未确认前该分支悬置。
- **并发控制**：install.py `--max-agents`（取值 1–6，默认 6）——即概念图 "spawn agents max_threads" 的对应物（字面名不同 [待核验]）。
- **账面 forks 计数**：journal 逐事件留痕（fork.started / decided / ready / executed / agent_confirmed / cancelled / native_started / native_returned）；retro.json 汇总 `jev_forks`、`jev_sharp`、`jev_split` 三项计数——衡量的是"编排"不是"代码质量"（README 明说）。
- **ASTRA 机制**：三个固定检查点 `before_plan`（定范围前）、`before_done`（收尾前）、`error_repeats`（同一失败重复后、重试前）。流程：controller 发检查点请求 → spawn 原生 agent_tree_astra 供目标/计划/差异/错误 → 等建议 → 以真实 agent_id 闭环检查点。Astra 永不改码；检查点可审计（观察者从 Codex 本地元数据核对角色/模型/时机/完成），证据不足必须报"无法核验"，不得默认通过。新的 fork 或相关变更会使 before_done 评审失效。
- **诚实边界（值得学的纪律）**：skill 自己声明——无 hooks 时无法从精确输入/输出证明原生工具动作，"do not claim that such a branch passed audit"；不授权工具权限、不强制每个未上报的语义决策、不证明代码质量。

**设计价值：低成本路由决策层如何省主模型 token**：

1. 把"有界、类别化"的微决策（选谁/看哪个文件/用哪个工具/重试还是停）从大模型生成里剥离：这类决策不需要生成能力，只需要一次分类；快决策层不出文本、返回类型化结果+校准概率，单价比大模型低数个量级（厂商口径 [待核验]），大模型只花 token 在研究与写作上。
2. sharp/split 双闸是"守下限"结构：高置信直发省一整轮大模型往返；split（含中间置信带）一律回主模型带证据裁决、低置信或不可逆动作再升级人工，宁可多花也不让低置信决策自动执行——与我们"低置信升级人工、阈值按后果分级"同构。
3. 与我们的关键差异：其决策层是外部 API（Jev），我们的硬约束 13 规定决策链零大模型、探针 L1/L2 本地 + 配置查表。**取其结构（三闸阈值、sharp/split、归属权特例、fork 记账、"真实备选才立决策点"），不取其依赖**。
4. 旁证（pi 自带 packages/coding-agent/examples/extensions/jev-router.ts）：用 Jev 分类器做虚拟模型路由（规划用 Sol/Terra、首次成功 edit/write 后切 Luna 且全会话保持），"一个会话只切一次模型、只吃一次 prompt-cache miss"，压缩摘要等循环外请求走便宜模型，路由状态存会话分支、随压缩存活——印证路由决策必须把缓存失效成本计入（对我们组件 5"切域不毁前缀"的第三方印证）。

## 三、OpenCode TUI 形态与 TUI 主页 / 双终端 / 本地路由层设计参考

**总体架构**：client-server——后端负责 LLM 推理、工具执行、会话持久化、MCP、权限（packages/opencode/src 含 session/permission/mcp/plugin/bus/worktree/skill/snapshot 等），多前端（TUI/desktop/web/console）经 HTTP+SSE 取数。TUI 只是前端之一。

**技术栈**：现仓库（sst/opencode main）TUI 为 TypeScript/TSX，SolidJS + @opentui/core（自研终端渲染器）+ @opentui/keymap/@opentui/solid，包名 @opencode-ai/tui。注意：部分二手资料（DeepWiki）仍记"Go + Bubble Tea"，那是旧版形态，以仓库现状为准 [待核验：版本演进未逐一核实]。

**主页/会话/组件形态**：

- 主页（routes/home.tsx）：**大 Logo**（component/logo.tsx 纯字符图形 + 主题着色/阴影）+ 居中 Prompt 输入框 + 轮换占位建议（普通模式"Fix a TODO in the codebase…"、shell 模式"ls -la…"）+ HomeSessionDestination（决定这次输入开新会话还是进当前会话）。与我们"主页大 Logo + 空白对话框"同形。
- 会话页（routes/session/）：时间线（dialog-timeline、**dialog-fork-from-timeline 从时间线分叉会话**）、sidebar（会话/工作区上下文、每会话 token 显示）、footer、prompt、permission.tsx、question.tsx、dialog-subagent（子代理会话可跳转打开）、subagent-footer。
- 多会话管理：dialog-session-list（搜索/过滤/重命名/删除失败会话/移入工作区）、dialog-move-session、工作区三件套（workspace-create/list/file-changes）、dialog-stash/tag/variant；服务端 session/ 目录是一台会话状态机（compaction/overflow/retry/revert/summary/run-state/reminders/status）。
- 组件库：command-palette、dialog-model/provider/mcp/skill/status/theme-list/debug、todo-item、spinner、bg-pulse、logo、SplitBorder、toast、主题/语法高亮体系；插件槽位（plugin/slots、feature-plugins/builtins）。
- **鼠标**：app.tsx `useMouse: !Flag.OPENCODE_DISABLE_MOUSE && input.config.mouse`——配置开关 + 环境级总闸；组件层处理 onMouseDown/Up（如右键菜单）。参考：鼠标作为可配置的一等输入。
- **Token 统计**：prompt 组件实时显示最近助手消息的 token 合计（input+output+reasoning+cache 读写）与上下文占用百分比；sidebar 按会话显示 token。它是点状数字，**没有热力图**——我们"历史 + Token 统计热力图"是加码设计，可拿它的数据项做热力图的行/列素材。

**三个设计参考落点**：

1. **TUI 主页**：照 OpenCode 形态——大 Logo + 空白对话框 + 占位建议 + 提交目的地路由（对应我们"主页输入回车弹新终端跑会话"的入口分流）。差异点：我们的回车是"弹新终端跑会话"（工作面），OpenCode 是路由到本终端的会话页。
2. **双终端（观察面/工作面分离）**：三者（pi TUI、OpenCode TUI、Codex skill）皆为单终端多路由，无物理分离先例——此为差异化设计，无处可抄。架构建议：观察面 = 只读订阅网关事实源的第二个消费者（与 OpenCode 多前端经 HTTP+SSE 订阅同构，也与我们组件 6"网关单一事实源"一致）；两个终端复用同一 TUI 渲染栈的两个实例，避免两套渲染技术。观察面信息项可直接取 OpenCode 已有数据项（token/上下文占用/会话状态）+ 我们特有项（域、探针标签、拦截与干预记录、决策点计数）。
3. **本地路由层**：pi/OpenCode 都没有本地置信度路由（OpenCode 选模型是 dialog-model 手选；pi 的路由是 Jev 外部分类器）。最接近的结构是 JEV fork layer：三闸阈值（confidence/probability/margin）→ sharp 直发 / split 回主模型带证据裁决（低置信或不可逆→升级人工）+ fork 记账。落到我们探针体系：分类器（L1 规则/L2 本地轻量模型）贴标签+置信度 → 认知纪律域阈值查表 → sharp（放行/直发）或 split（回主模型裁决，低置信/不可逆→升级人工），决策点与 sharp/split 计数入事件账本、观察面可见；阈值随域加载（对应其策略可配置）。"仅真实备选才立决策点"一条对我们收敛检测同样有效：不为每个动作强行过路由。

## 机制印证表

| 机制 | 来源 | 我们现状 | 建议 |
|---|---|---|---|
| 扩展=默认导出函数+事件订阅，tool_call 钩子返回 {block, reason} 同步阻断 | pi extensions/types.ts、examples/permission-gate.ts | 组件 6/7 已有（单一调度入口、决策链同步） | 钩子返回值收敛为三态（放行/阻断/询问），与查表裁决同构 |
| 外部工具暴露四档（codemode/deferred/direct/hidden），默认不进模型工具声明 | pi extensions/mcp | 组件 5 已有方向（上限留缓存、纪律移拦截） | 工具目录分档照此落地：全量进前缀、可调用子集由拦截层收紧 |
| MCP 工具与内置工具过同一 tool pipeline，权限钩子同等作用 | pi extensions/mcp 注释 | 组件 6 已有（能力进出只过网关/白名单/检查点） | 直接印证，无需改设计 |
| 压缩摘要先做文件操作追踪再总结；压缩可按 token 阈值显式触发 | pi compaction/、examples/trigger-compact.ts | 组件 5 已有（压缩显式事件、压缩禁区） | 压缩摘要固定携带"改动文件/标识符清单"，配合硬约束 16；触发条件参数化 |
| 分支摘要（branch summarization） | pi compaction/branch-summarization.ts | 缺（痕迹层未定型） | 印证辅助执行体"只回压缩摘要"；摘要协议可参考 |
| 子代理=独立进程+隔离上下文+usage 追踪+中止传播 | pi examples/subagent | 组件 10/11 已有（按需派生、只回摘要、可中止） | 派生形态照此；补"每执行体 usage 记账"进观察面 |
| 质量回环=编排模板（worker→reviewer→worker） | pi examples/subagent/prompts | 组件 3 已有（证据强制绑定更强） | 取循环编排；评审主体坚持代码验证器，不引"模型审模型" |
| 持久化条目模型（typed entry + checkpoint + 策略常量） | pi durable/src | 缺（痕迹层数据结构待磨合，待磨合点 7） | 痕迹层条目化参考此形态：类型化写入物 + 检查点 + 衰减/重试策略常量 |
| 危险命令确认框：有 UI 才询问、无 UI 默认阻断 | pi examples/permission-gate.ts | 组件 1 已有（动作分级与放行） | 印证"非交互模式从严"；人工确认入口必附风险解释 |
| 缓存感知模型路由：一会话一切换一 miss，路由状态随会话树存活 | pi packages/coding-agent/examples/extensions/jev-router.ts | 组件 5 已有（切域不毁前缀） | 印证切域协议；路由状态属会话级、不进前缀 |
| 快决策层三闸（confidence/probability/margin）→ sharp 直发 / split 回主模型 | hpvdev/agent-tree-workflow templates/control.py | 组件 1 同构但更严（决策链零大模型，硬约束 13） | 取 sharp/split 结构与三闸形态；决策器保持本地规则/分类器+域阈值查表 |
| 执行者归属权在决策层：split 不得由主模型自决下级 | hpvdev workflow.md | 组件 10 已有（路由不由模型自由决定） | 落成硬约束式条款：拆分悬置时主模型不得自我任命执行体 |
| 仅真实备选才立决策点，禁止编造 fork | hpvdev workflow.md | 部分已有（有界干预/有界验收） | 并入收敛检测：确定性动作不强行过路由 |
| fork 记账（total/sharp/split + 逐事件 journal + retro） | hpvdev templates/control.py | 组件 6 已有（事件为唯一事实源） | 观察面展示决策点计数/直发率/回退率；retro 只度量编排不度量代码质量 |
| 三固定检查点（before_plan / before_done / error_repeats）+ 不可核验即报缺证 | hpvdev workflow.md | 组件 3 已有（有界验收待磨合点 6） | 验收触发条件表可先取这三个时机为最小集 |
| TUI 主页=大 Logo+空白对话框+占位建议+提交目的地路由 | opencode routes/home.tsx | 组件 8 待磨合 | 主页形态直接对齐 |
| 鼠标=配置开关+环境总闸 | opencode app.tsx | 缺 | 鼠标事件入观察面一等输入，双开关设计 |
| 权限内联面板三段式（permission/always/reject） | opencode routes/session/permission.tsx | 组件 8 已有（结构化引导） | 人工确认界面照三段式+风险解释+always 白名单落盘 |
| 会话分叉（从时间线 fork） | opencode routes/session/dialog-fork-from-timeline.tsx | 组件 11 已有（回滚锚） | 回滚锚的 UI 形态可参考"选历史节点→分叉/回滚" |
| 热力图无先例（token 只有点状数字） | opencode prompt/sidebar | 缺（我们规划项） | 热力图是加码设计；数据项取 token 分解（input/output/reasoning/cache 读写）+ 上下文占用率 |

## 待核验清单

1. 概念图"split(<~0.5)→回主模型"的 0.5 数值：实现中无此常量，split=未过 sharp 三闸（默认 confidence .85 / probability .85 / margin .20），0.5 出处不明。
2. "spawn agents max_threads"字面名未见于任何实现；对应物为 `--max-agents`（1–6，默认 6）。
3. "Codex Agent Tree / JEV fork layer"的最初出处（X/博客/视频）未定位；可核验最早文本为 hpvdev/agent-tree-workflow（2026-10-03）templates/control.py 文档串；此前有 Madikhan33/jev_codex（09-21）、evgyur/codex-agent-tree（09-29）。
4. Jev 性能与价格（70–500ms、$0.042/MTok、快至 200 倍）为厂商与二手评测口径，未独立实测。
5. OpenCode TUI 技术栈版本演进（旧版 Go/Bubble Tea vs 现版 SolidJS+@opentui）未逐一核实，本文件以克隆仓库现状为准。
6. 模型名 GPT-6 Sol/Terra/Luna/Astra 及其档位配置来自第三方配置文件与博客引用，模型真实规格未核验。
7. "pi 为 OpenClaw 底层 SDK"的说法来自 OpenClaw 文档与第三方站点；本次仅核验 pi 仓库自身（README 称 OpenClaw 为 real-world integration），嵌入方式（AgentSession 直接导入等）未核验源码。
