性质：复查报告（对抗性审查，只读审查+本报告）

# 2026-10-05 改方向规划产出 · 对抗性复查报告

> 复查时间：2026-10-05。审查对象只读，本报告为唯一写入物，未做任何 git 操作。
> 审查对象（8 件）：roadmap.md（v2）/ project-structure.md（v2）/ prompt-skeleton.md / survey-agent-prompts-pi-opencode-kimi.md / survey-agent-prompts-hermes-mimo.md / survey-agent-prompts-codex-claude-deepseek.md / survey-competitor-mechanisms.md / survey-dg-ai-notes.md。
> 方法：逐文档互对 + 对源码/构建产物实测（wc -l / find / cargo test / workspace 配置）+ 证据锚点联网抽查（GitHub API 与 raw 内容，经代理）。网络抽查时间为复查当日，抽验命中如实记录，抽验不到的标"未核实"，不做推测。
> 备注：docs/prompt-draft.md 于复查进行中（21:52）出现，不属本次审查范围，仅在下文作旁证引用一处。

---

## 一、关键结论清单逐条核对（①–⑩）

| # | 结论 | 判定 | 依据/问题 |
|---|---|---|---|
| ① | hub 两态布局（初始=大 Logo+空白对话框；运行时=顶栏 Logo 左上+对话框右、下方左 1/3 历史右 2/3 Token 统计） | **半成立** | 初始态与 roadmap §三.6 一致；**运行时布局（顶栏/左右分栏/1:2 分割）在全部产出中零命中**（检索"顶栏/1/3/2/3/两态"无结果），结论未落盘（问题 M2） |
| ② | 干预透明=DSH 式对话内灰色半透明显示、非独立面板 | **无证据** | 全部文档检索"灰色/半透明"零命中；DSH（DeepSeek Harness）调研证据面仅为提示词快照，不含任何 UI 呈现形态描述，"DSH 式"出处不可核验（问题 H2） |
| ③ | 并发不设人为上限 | **未落盘** | 无任何文档记载；反见 survey-competitor-mechanisms 记录 JEV 实现 `--max-agents`（1–6，默认 6，已联网核实 install.py:82 默认 6）——参考实现有上限、我方结论无上限属独立设计决策，但没有任何文档留痕（问题 M2） |
| ④ | 审批分级=规则表 L1+JEV 式语义探针 L2 双层 | **成立** | 与 concept-v2 探针分层（L1 规则/L2 本地轻量分类/决策链无大模型）及 prompt-skeleton §四.3"探针贴标签→配置查表→代码裁决"一致；唯 prompt-skeleton 将触发输入挂靠"本地路由决策"而该机制在 roadmap T5 为[待磨合]（问题 M5） |
| ⑤ | JEV fork layer 与概念书探针"贴标签+置信度→查表裁决"同构 | **成立** | survey-competitor-mechanisms §零.2 与 prompt-skeleton §四.3 口径一致，且 JEV 机制本体已实证（见第二节） |
| ⑥ | 提示词六段骨架与静态预算 ≤1.3K token | **成立（口径微差）** | prompt-skeleton 六段表+"静态正文目标 ≈1.3K"；"≈1.3K"与下游"≤1.3K"口径略异（问题 L3） |
| ⑦ | 核心文件 ≤500 行约束、超标 4 件 1251/787/532/518 | **成立（实测复核）** | `wc -l` 实测完全吻合：src/model.rs 1251 / src/config/settings.rs 787 / src/doctor.rs 532 / src/policy/mod.rs 518 |
| ⑧ | 8 家考古核心结论（工具描述 57–89% / 静态动态分离共识 / Pi≈0.8K、OpenCode≈3.7K、Kimi≈4.4K / 防御三层 / 写入权收窄） | **基本成立** | 档位、三层防御、写入权收窄、静态动态分离均在调研正文有据；唯工具占比区间下限对不上自表数据（问题 L2） |
| ⑨ | 平台先 WIN+LINUX、统一命令 sdagent、chafa Logo 工作流 | **成立** | roadmap §三.2/4/6、project-structure §一/T2 一致；chafa SVG 支持为可选构建依赖（问题 L4） |
| ⑩ | JEV 出处 hpvdev/agent-tree-workflow、pi issue #6621 等锚点真实 | **成立（联网核实）** | 见第二节，全部抽验命中 |

---

## 二、证据锚点抽查（联网核实，2026-10-05）

全部经 GitHub API / raw 实际请求，逐条命中：

| 锚点 | 声称内容 | 核验结果 |
|---|---|---|
| hpvdev/agent-tree-workflow | "Executable Jev fork layer plus audited Astra checkpoints" 文档串 | ✅ templates/control.py 原文命中（注：实际路径 templates/control.py，非仓库根） |
| 同上 | classify() 三闸 sharp 判定（confidence/probability/margin） | ✅ control.py:81 逐字吻合 |
| 同上 | fork 四类 which_agent/which_file/which_tool/retry_or_stop；which_agent 首问、归属权特例（margin>0 即 sharp）、"Sol must not resolve a which_agent split…"、confirm-agent、retro 记账 jev_forks/jev_sharp/jev_split、"不得谎称通过审计" | ✅ control.py 与 templates/workflow.md 全部命中（含 :370-372 记账、:186-187 归属权注释） |
| 同上 | JEV_DEFAULTS confidence .85 等、--max-agents 默认 6 | ✅ install.py:14（confidence .85）、:82（max_agents 默认 6） |
| earendil-works/pi issue #6621 | "Prevent accidental cache invalidation due to dynamic system prompt" | ✅ 标题逐字命中 |
| earendil-works/pi issue #5132 | 提示词指引与真实工具集不符 | ✅ 标题 "System prompt lists exploration tools that aren't registered" |
| earendil-works/pi | README "Pi ships with powerful defaults but skips features…"、.pi/prompts/deslop.md、examples/extensions/jev-router.ts（Sol/Terra→Luna、一换一 miss） | ✅ 原文命中；jev-router 实际路径 packages/coding-agent/examples/extensions/（问题 L4） |
| MoonshotAI/kimi-cli | PR #1392 plan mode / #1700 hierarchical AGENTS.md / #2186 Shell→git-bash；system.md "You are Kimi Code CLI" | ✅ 全部命中（PR 标题逐字吻合） |
| anomalyco/opencode | PR #19487/#20771/#33039、#5550 缓存相关提交 | ✅ 全部命中（sst/opencode 301 重定向至此，两处署名均真实） |
| WEIFENG2333/phistory / phistory.cc | 快照仓库与站点 | ✅ 仓库存在（描述吻合），站点 HTTP 200 |
| buchidonggua/dg-ai-notes | 冬瓜教程站 | ✅ 存在 |
| limeiwang/unbot | MIT、中文去 AI 套话规则引擎 | ✅ 存在、license=MIT |
| hpjansson/chafa | SVG→ANSI 成品方案 | ✅ 存在；SVG 支持为 optional 构建依赖（librsvg） |

**代码/产物实测**（真实性复核）：超标行数 4 件逐一对上（见⑦）；`cargo test --workspace` = 97 passed（84+13）/ 0 failed / 1 ignored——roadmap"97 测试"成立（1 ignored 未提及，无实质影响）；doctor.rs 六项体检、SD_AGENT_MAX_ROUNDS 默认 20、tools/mod.rs 字节稳定断言、workspace resolver="3" 非虚拟、target/debug 三 exe（sd-agent/sd-tui/sd-desktop）均与文档一致。

**未能核验（保持"未核实"）**：OpenCode 1.16.0 工具区 -35% 瘦身的直接 commit 动机（调研已自标[待核验]，合规）；各厂商演化内部动机（调研均标[待核验]，合规）；Jev 商用性能价格数字（调研已标[待核验]，合规）。五份调研的[待核验]标注纪律整体良好，未发现"该标未标"。

---

## 三、问题清单

### 高（3）

**H1｜prompt-skeleton.md 版本与状态自相矛盾，且跨文档版本口径不一**
- 位置：prompt-skeleton.md:1（标题"结构骨架 v2（待作者逐段审）"）vs :4（"状态：待作者审"）vs :45（"第四节…2026-10-05 作者逐条拍板，全部 [已定]"）vs :55（变更说明"v3（2026-10-05 拍板收口）"）；roadmap.md:57-58 与 project-structure.md:62 仍写"骨架 v2"。
- 问题：同一文档标题 v2、变更说明 v3；"待作者审"与"作者逐条拍板、全部[已定]"互斥。旁证：复查期间出现的 prompt-draft.md 头部引"骨架 v3 全部拍板结果"。这直接冲击"禁止 AI 擅自定稿/逐段拍板"流程的可判读性（AGENTS.md 文档规范 3、铁律 5）。
- 处置：标题与状态统一为真实态（骨架结构与第四节=已定，提示词正文=待作者审），变更说明版本号与标题一致；roadmap T3.2/T3.3、project-structure T3 的"骨架 v2"同步改为定稿版本号。

**H2｜结论②"干预透明=DSH 式对话内灰色半透明显示"无任何证据支撑，且未落盘**
- 位置：结论清单②；全 8 份产出检索"灰色/半透明/DSH 式"零命中（仅 handoff-20261003.md 有"Codex/DSH 式布局"，指桌面布局，与此无关）。
- 问题：DSH 调研（survey-agent-prompts-codex-claude-deepseek 第 5 节）证据面仅为提示词文本，不含 UI 呈现形态；"DSH 式对话内灰色半透明"属无可核验出处的断言（幻觉风险类），违反铁律 1"核验不到就标注待核验或不写"。若该形态系我方自研设计，也不该挂第三方标签。
- 处置：要么作为自研交互设计写入 roadmap T1（去"DSH 式"标签，注明"自研形态"），要么标注[待核验]并补出处；在证据补齐前禁止以"DSH 式"名义写入任何设计文档。

**H3｜脱敏违规：本地绝对路径含账户名与运行环境标识入库**
- 位置：survey-agent-prompts-hermes-mimo.md:15：本地绝对路径含 Windows 账户名与本机目录布局（原值已去标识，占位形态 `<LOCAL_HOME>/hermes-agent`；同段还列出本机读取的内部源码文件清单）。
- 问题：违反 AGENTS.md §3.2（不写个人身份信息）与 §3.5（公开仓库禁"记忆库与运行环境的具体数值与标识"）。公开仓库 push 即公开，Windows 账户名 + 本机目录布局属可识别环境指纹。
- 处置：改为去标识占位（如 `<LOCAL_HOME>/hermes-agent`，与同文档 `$PHISTORY_HOME` 占位惯例一致）；推送历史若已含该路径需按脱敏预案处理（本复查不执行任何 git 操作）。

### 中（5）

**M1｜project-structure.md 文件清单与实测不符：27 vs 29，树中漏 2 文件**
- 位置：project-structure.md:22（"src/ …（27 文件，实名职责）"）与第二节目录树。
- 问题：`find src -name '*.rs'` 实测 29 个；树中缺 **src/session.rs**（会话库，正是 roadmap T1.4"统一从事实源（轨迹/会话库）取数"的事实源之一）与 **src/config/settings.rs**（恰是同文档 :56 引用的超标文件之一）。文档自称"现状 v2/实测"，却保留 P0 期 27 文件旧数（p0-brief 历史口径为 27）。
- 处置：树补 session.rs 与 config/settings.rs，计数改 29；后续结构变更同步刷新。

**M2｜关键结论①运行时布局、③并发不设上限未落盘（产物留痕违规）**
- 位置：roadmap §三.6/7、§四 T1（仅"主页=大 Logo+空白对话框+历史列表+热力图"，无两态布局、无分栏比例）；全库无"并发不设人为上限"记载。
- 问题：违反 AGENTS.md §2.5 产物落盘留痕——结论只存在于对话，后续接手者会按 docs 现状实现，与已形成的结论漂移。
- 处置：把 hub 运行时布局（顶栏/分栏/比例）与并发上限口径补进 roadmap T1（或标[待磨合]显式留问）；并发口径建议注明"不设人为上限"针对的是哪一层（run 数/子代理数），并处理与 JEV 参考实现 --max-agents 的关系说明。

**M3｜fork layer 适配口径三处不一致 + 微决策类别漏项**
- 位置：survey-competitor-mechanisms.md:10/54（split=回主模型带证据裁决，低置信→人）；roadmap.md:65（"sharp→直接执行 / split→升级主模型"，且只列"which file / which tool / retry or stop 三类"）；prompt-skeleton.md:49（"sharp 放行 / split 升级人工"）。
- 问题：同一机制的 split 去向出现"主模型/人工"两种口径；调研实证 fork 为四类（which_agent 每轮首问，已核实 workflow.md），roadmap 漏 which_agent。
- 处置：统一为两级表述（split→回主模型裁决，低置信/不可逆→升级人工），roadmap 补 which_agent 或注明我方不采纳该类的理由。

**M4｜脱敏/内部交互痕迹：用户原话与内部协商过程进入公开文档**
- 位置（逐条，原话引用按公开仓库脱敏纪律转述）：
  - prompt-skeleton.md:11 用户问句原话记录（已改中性表述）；
  - prompt-skeleton.md:36 "待你拍板"对话口吻（文档自标"可外发"）；
  - prompt-skeleton.md:47、:51 内部编排角色"定案"标注与要求转述；
  - prompt-skeleton.md:52 用户原话（含粗俗比喻，已转述）+ "300 行放宽到 500"协商过程；
  - project-structure.md:56 同引该原话；roadmap.md:47 同引其粗俗简称。
- 问题：AGENTS.md §3.5 禁内部交互过程与对话细节入库、实证引用须去原话。"作者拍板"作为台账惯例可留（concept-v2 有"核心命题（作者原话）"先例），但 300→500 的放宽协商过程、用户问句、"待你拍板"口吻属交互痕迹；prompt-skeleton 自标"可外发"使问题加重。
- 处置：保留结论与"作者拍板"标注，删原话与协商过程；"待你拍板"改"待作者拍板"；"用户问的"改中性表述（如"用户关切：环境防御"）。

**M5｜[已定] 结论挂靠[待磨合]机制，依赖关系未标注**
- 位置：prompt-skeleton.md:49（第四节整体标"全部 [已定]"，其中环境防御"触发由用户设置与本地路由决策共同决定"）vs roadmap.md:65（本地路由层/fork layer 列为[待磨合]候选）。
- 问题：已定结论的触发链依赖未转正机制，落地时会发现输入端缺失；AGENTS.md 文档规范 3 要求状态与内容相符。
- 处置：在 prompt-skeleton §四.3 补依赖说明（"路由层未落地前退化为用户设置单输入"），或把该句拆出标[待磨合]。

### 低（6）

**L1｜历史记录计数错误**：roadmap.md:17 "doctor 五项"（P0 定义历史条目）vs p0-brief §四.6"doctor 六项"、doctor.rs 实测六项、roadmap:14 自己也写"doctor 六项全绿"。处置：历史条目改"六项"。

**L2｜工具描述占比区间口径不严谨**：survey-pi-opencode-kimi "57%~72%"（其自报数据 Pi 3,165/6,013≈53%，下限 57% 无出处），prompt-skeleton:8 合并两份调研为"57%~89%"未注明口径合并。处置：统一按自报数据重算区间并注明"跨 6 家合并"。

**L3｜现状描述与代码实况不符 + 预算口径微差**：prompt-skeleton:40 "现 src/context.rs 四段（角色/服从契约/目标锚/附录）"漏记工具契约段（context.rs:14 自述"角色与纪律+服从契约+工具契约+目标锚"+附录）；"≈1.3K"（骨架 :27）与下游"≤1.3K"表述不一。处置：改准现状枚举，统一≈/≤口径。

**L4｜引用路径简写与前提漏标**：survey-competitor 引 "examples/extensions/jev-router.ts" 实为 packages/coding-agent/examples/extensions/（内容已核实）、"control.py" 实为 templates/control.py；chafa SVG→ANSI 依赖可选构建组件 librsvg（README 标 svg optional），roadmap/project-structure 未标该前提[待核验]。处置：路径补全、chafa 补依赖前提。

**L5｜状态词超纲**：roadmap:56 "[进行中]"、project-structure [S13][S14]"进行中"不属于 AGENTS.md 三态口径（已定/待磨合/待核验）。处置：改标或在文档口径行补"进行中"定义。

**L6｜Kimi 产品命名疑点（未核实项）**：MoonshotAI/kimi-cli 现为 archived，GitHub 说明称后继产品为 "Kimi Code CLI"（MoonshotAI/kimi-code）；survey-pi-opencode-kimi:84 称 captures/kimi（kimi-cli）自述 "Kimi Code CLI" 且与 kimi-code"是两个产品"。PR 级证据已核实为真，但产品命名归属易混淆。处置：补一句命名澄清（区分"仓库 kimi-cli/其提示词自称 Kimi Code CLI/后继产品 kimi-code"）。

---

## 四、状态标注合规小结

- roadmap / project-structure：三态口径使用规范、状态与内容相符（[待核验]风险项与正文一致），仅 L5"进行中"超纲。
- prompt-skeleton：H1（状态互斥）+ M5（已定挂待磨合）两项不合规，其余条目标注可判读。
- 五份调研：[待核验]纪律良好，均带"待核验清单"节，归因与事实分离清楚——未发现"推断写成实锤"的情形，此项优于一般调研文档。

## 五、概念书（concept-v2）一致性

- 四公理方向无偏离：roadmap"守底线不拔上限/参考不是需求（关键设计思想 12 引用正确）"、prompt-skeleton"提示词零裁决权/探针贴标签→配置查表→代码裁决"（与硬约束 11 修复版措辞一致）、"决策链零大模型走本地（硬约束 13）"均对齐。
- 硬约束 15/16/17、四段结构（冻结前缀/域快照/历史/增量附录）、硬约束 21 的引用与 concept-v2 原文口径一致；"四段结构"（上下文层）与 prompt-skeleton"六段"（提示词层）系两层概念，未见实质冲突，但同名"四段"在 prompt-skeleton:40 另指 context.rs 段落（L3），建议消歧。
- 唯一系统性张力：已定结论挂靠待磨合机制（M5），非公理级偏离。

## 六、GO / NO-GO 判定

**判定：NO-GO（整改后可转 GO）。**

理由：产出整体质量高于基线——证据锚点抽验 100% 命中（12 组锚点含源码级细节全部属实）、4 件超标行数与 97 测试实测吻合、调研[待核验]纪律良好、概念书一致性无系统偏离。但存在 3 条高优先级问题阻断"可信可用"：H1 版本/状态互斥直接影响"禁止擅自定稿"的审批流程判读；H2 无证据断言（DSH 式呈现）若流入设计将成幻觉源；H3 脱敏违规在公开仓库属一票否决项（push 即公开）。另有 2 条关键结论（①运行时布局、③并发口径）未落盘，文档现状与已形成结论之间已经漂移。

**转 GO 条件（全部满足即可复审通过）**：
1. H1：prompt-skeleton 版本号与状态统一，roadmap/project-structure 引用同步；
2. H2：结论②补出处标[待核验]或改写为自研设计并落盘，去"DSH 式"标签；
3. H3：本地路径去标识化；
4. M1/M2：结构树补 2 文件改计数；结论①③落盘 roadmap（或显式标待磨合）；
5. M3/M4/M5 建议同批处理（fork 口径统一、交互痕迹清理、依赖标注），低优先级 6 条可随后续迭代。

（本报告仅复查，不改审查对象一字；所有实测命令与联网核验结果均可按第二节复跑复查。）

## 七、整改记录（2026-10-05 整改后补记，非复查原文）

14 问题整改位置（文件:行，行号为整改后）：

| # | 整改位置 | 处置 |
|---|---|---|
| H1 | prompt-skeleton.md:1/4/16、roadmap.md:32/57/58、project-structure.md:62 | 标题/状态统一为 v3 真实态（骨架结构+第四节 [已定]、正文 [待作者审]），跨文档"骨架 v2"引用同步 v3 |
| H2 | roadmap.md:32（§三.12 新增行） | 干预透明=对话内灰色弱化显示落盘为独立设计决策（自研形态 [已定]），"DSH 式"出处不可核验标【待核验】 |
| H3 | survey-agent-prompts-hermes-mimo.md:15 | 本地绝对路径去标识（`<LOCAL_HOME>/hermes-agent` 占位） |
| M1 | project-structure.md:22 及目录树 config/model 行后 | 计数 27→29（实测），补 src/session.rs、src/config/settings.rs |
| M2 | roadmap.md:41-42（T1.3/T1.4） | hub 两态布局（含运行时顶栏/左 1/3 右 2/3 分栏）+ 并发不设人为上限（run 数口径、与 `--max-agents` 关系）落盘 |
| M3 | survey-competitor-mechanisms.md:10/54/77、roadmap.md:65、prompt-skeleton.md:49 | split 去向统一（回主模型带证据裁决，低置信/不可逆→升级人工），roadmap 补 which_agent 四类 |
| M4 | prompt-skeleton.md:11/36/47/51/52、project-structure.md:56、roadmap.md:47 | 用户原话转述、协商过程与内部角色标注删除、"待你拍板"→"待作者拍板" |
| M5 | prompt-skeleton.md:49 | 补依赖说明（本地路由层 [待磨合]，未落地前触发输入退化为用户设置单输入） |
| L1 | roadmap.md:17 | "doctor 五项"→"六项" |
| L2 | survey-agent-prompts-pi-opencode-kimi.md:8/142、prompt-skeleton.md:8 | 区间按自报数据重算（53%~72%；合并 6 家 = ≈53%–89%）并注明跨 6 家合并 |
| L3 | prompt-skeleton.md:27/40 | context.rs 现状枚举改准（五段+附录）、预算口径统一 ≤1.3K |
| L4 | survey-competitor-mechanisms.md:10/38/40/56/92/93/96/108、roadmap.md:28、project-structure.md:81 | 引用路径补全（templates/control.py、packages/coding-agent/examples/extensions/jev-router.ts）、chafa 补 librsvg 可选依赖前提【待核验】 |
| L5 | roadmap.md:56/65、project-structure.md:83/84 | "进行中"改三态口径（[已定]/已产出） |
| L6 | survey-agent-prompts-pi-opencode-kimi.md:84 | Kimi 命名澄清（仓库 kimi-cli / 提示词自称 Kimi Code CLI / 后继产品 kimi-code） |

版本号规划落盘：docs/roadmap.md 第七节（变更说明前），语义化版本 + 唯一出处=根 Cargo.toml。本整改未触碰任何代码与 Cargo.toml。
