# 项目文件结构规划（改方向版 v3，决策台账类）

> 性质：决策/结构类文档（含第三方技术名与来源引用，非纯净文档）。
> 拍板：2026-10-02 初版（目标形态/目录树/文件粒度经作者拍板，两路盲审 30 条修订收口）；**2026-10-05 改方向修订**（workspace 拆分已实际发生 + 分发链 + 双终端 + 快捷命令）。
> 状态口径：已定 = 作者拍板 / 官方文档实锤 / 实测；待磨合 = 推演或机制设计；待核验 = 须实测才能定。
> 配套读：docs/roadmap.md（T 序列执行序）、docs/concept-v2.md（设计理念，纯净总纲）。

## 一、定位与目标形态

- 目标形态【已定】：**一个核心 + 两套壳 + 一条分发链**——核心 lib（业务全在此）+ TUI 壳 + 桌面壳（Tauri）+ launcher/安装（`sdagent` 统一命令）。核心与 UI 解耦。
- 平台【已定】：Windows / Linux 先行（2026-10-05 拍板），macOS 后置（无实机验证条件）。
- 命令面【已定】：`sdagent`（TUI 主页）/ `sdagent --desktop`（桌面）/ `sdagent doctor` / `sdagent run`（CLI 兼容保留）。桌面另有快捷方式入口。
- 文件粒度【已定】：文件数量不是约束（业界成熟项目成百上千文件为常态），以模块职责清晰、可维护为准；禁止为凑数建空壳文件。
- 实现期只许"加文件、换实现"，不许改依赖方向。

## 二、目录树（现状 v2：workspace 三 crate）

```
sd-agent/
├── Cargo.toml / Cargo.lock        # [workspace] members=["apps/tui","apps/desktop"]，resolver="3"（非虚拟）
├── docs/                          # 设计文档（含本文件）
├── src/                           # root 包 = 核心 lib + sd-agent CLI bin（29 文件，实名职责）
│   ├── main.rs                    # 薄壳：解析参数 → 调 lib；零业务逻辑
│   ├── lib.rs                     # 模块树根 + crate 级文档
│   ├── cli.rs                     # 子命令面 doctor / run（手写解析，不引 CLI 框架）
│   ├── config/                    # mod（三类档案门面+域数据位）/ settings（设置档案，多模型配置链）/ env（环境档案）/ secret（Secret newtype）/ resource（资源分配+副作用登记）
│   ├── event/                     # mod（稳定门面）/ contract（事件契约）/ sink（JSONL 实现）/ source（回放）/ hooks（钩子唯一收敛点）
│   ├── model.rs                   # ModelClient + OpenAI 兼容适配（byot，MiMo reasoning_content 全能力）
│   ├── session.rs                 # 会话库（会话持久层，轨迹/会话事实源之一，hub/worker 统一取数来源）
│   ├── sys.rs                     # 跨平台单点：路径归一化/进程终止/执行器选择/no_console_window
│   ├── tools/                     # mod（目录 schema+字节稳定断言）/ read / write / edit / bash / fs_guard
│   ├── policy/                    # mod（dispatch 唯一模型驱动执行入口）/ rules（危险命令表按平台分行）/ approval（ApprovalPort）
│   ├── context.rs                 # 上下文组装：static_prefix() / dynamic_appendix() 分建 + 段标记 + 目标锚（T3 系统提示词落地点）
│   ├── agent.rs                   # 工具调用循环 + 轮数熔断（SD_AGENT_MAX_ROUNDS，默认 20）
│   ├── verify.rs                  # 验证器：证据 = 事件(kind=verify_result) + 落盘 .sd-agent/evidence/
│   └── doctor.rs                  # 六项体检（工具探针走 policy::dispatch 实测真通道）
├── apps/tui/                      # TUI 壳 crate（sd-tui.exe）：main/app/bridge/run/slash/ui
│   └── （T1 扩展位）              # hub 主页模块 / worker 会话终端 / stats 统计聚合 / 鼠标交互 / 流式渲染
├── apps/desktop/                  # 桌面壳 crate（sd-desktop.exe，Tauri 2）：build.rs + tauri.conf.json + dist/ 前端（编译期嵌入）
├── assets/logo/                   # [T2 扩展位] 作者 SVG → chafa 构建期转 ANSI art（待 Logo 交付）
├── tests/                         # P1 起：契约 fixture + common/ + CARGO_BIN_EXE 冒烟
└── .sd-agent/{traces,evidence,logs,doctor-probe}/  # 运行时产物（gitignore，含交互内容不入库）
```

产物现状（实测）：target/debug/ 三 exe——sd-agent.exe（CLI）/ sd-tui.exe / sd-desktop.exe。

## 三、布局决策（v2 修订）

1. **workspace 已拆**【已定，2026-10-03 实际发生】：root 包（核心 lib + CLI bin）+ apps/tui + apps/desktop，非虚拟 workspace、resolver="3"。初版"单 crate 起步、二进制封顶 2 个"口径已被现实演进取代——现实三 exe；拆分触发条件（第二独立发布产物）正是拆分依据，触发兑现、预案落地。
2. **按组件职责分模块，不按五层工程地基分目录**【已定】（同初版决策 2，不变）。十二件 × 五层是正交视角，跨层组件按职责安家，模块头注释标注所服务的层。
3. **依赖方向单向，执行边界物理化**【已定】：依赖图只许向下（cli → agent → policy → tools；agent → model/event/context；verify/doctor → config + 探针接口；event/config 最底零业务依赖）；**核心 lib 严禁依赖任何壳 crate**；壳只做四件事（TeeSink 事件双写 / ApprovalPort 阻塞桥 / 模型装饰器旁路正文 / 生命周期管理）。唯一**模型驱动**执行入口 = policy::dispatch()（pub(in crate::policy) 收口，口径同初版决策 3：verify/doctor 确定性执行是 Harness 侧独立窄通道，不经模型路径）。
4. **事件层四件首日立桩**【已定】（contract 强类型+演化规则 / sink 提交事件语义 / source 读侧回放 / hooks 唯一收敛点，对齐硬约束 14；公共类型路径 mod.rs 门面永不变。口径同初版决策 4）。
5. **双终端扩展位（hub/worker）**【待磨合 → T1 实测收口】：hub 主页进程 + worker 会话终端进程，统一从事实源（event/source + 会话库）取数（concept-v2 §8"统一从事实源取数"）；worker 由 hub 以系统默认终端 spawn（Windows Terminal 优先，conhost fallback）。
6. **launcher 位**【待磨合】：`sdagent` 统一命令入口（root bin 扩展子命令或独立 shim），安装注册 PATH；桌面快捷方式指向 sd-desktop.exe，`sdagent --desktop` 为等价命令入口。
7. **抽象的度：四接口，其余具体类型**【已定】（EventSink / EventSource / ApprovalPort / ModelClient，同初版决策 7）；stats 统计聚合为纯函数模块（读事件 → 聚合表 → 壳渲染），不新增接口【待磨合】。
8. **安全单点 + 残余风险显式声明**【已定】（同初版决策 6）：Secret newtype 单点防泄密；fs_guard 单点管文件类工具路径（双字段：原始输入 + 规范化形态进事件，canonical 真实位置防线防符号链接/junction 穿透）；危险命令规则表纯数据、按平台分行（raw string 写模式，配真实攻击样例测试）。
9. **核心文件行数上限 ≤500 行/文件**【已定，2026-10-05 作者拍板】：防止核心代码质量劣化（限值由早期更严口径放宽而来，原话含粗俗比喻已转述）。超标即拆：按职责拆文件不拆模块、公共类型路径不变（mod.rs 门面）、行为零改动、回归全过。现状超标 4 件（model.rs 1251 / config/settings.rs 787 / doctor.rs 532 / policy/mod.rs 518）列入 roadmap T1.5 整改；此后新增文件一律合规。

## 四、分期扩展位（对齐 roadmap T 序列；P0 不建空壳，只留缝）

- **T1（apps/tui）**：hub 主页（Logo + 输入框 + 历史列表 + Token 热力图）/ worker 会话终端 / 鼠标全操控 / 流式渲染节奏修复 / stats 聚合。
- **T2（分发链）**：release profile（Windows + Linux）、sdagent 入口与 PATH 注册、桌面快捷方式、assets/logo 构建期转换（chafa）。
- **T3（src/context.rs，必要时新增 src/prompt/ 位 [待磨合]）**：系统提示词六段骨架落地（骨架 v3，结构已定稿），硬约束 15/16/17 与四段结构不变。
- **T4（apps/desktop）**：TUI 成果同构复刻 + 字体锐化（WebView2 字体栈/DPI）+ 布局重做。
- **T5（核心机制，原 P1–P3/P5）**：event/sink_sqlite.rs 换实现（契约不动）、state.rs 状态机（流转矩阵表驱动）、tests/ 契约 fixture + golden 回归架（用现成任务集）、token 预算 + context 四段全量（冻结前缀/域快照/历史/增量附录）、记忆 L0 + 目标锚持久层（单源注入 + 强制重注入，硬约束 5/17）、policy 交集裁决（域白名单 ∩ 状态准入）+ 探针 L1 挂 hooks + 收敛三态、记忆 L1/L2 检索与技能——落点同初版第四节，只改排期不改落点。

## 五、残余风险与边界声明（初版 6 条保留 + 新增 2 条）

1. bash 逃逸面：黑名单可被参数化执行绕过（解释器 -c 类）；P0 边界 = 黑名单雏形 + cwd 钉死工作区 + 全量审计留痕；真正封闭靠隔离执行体（硬约束 6）分期到位。
2. TOCTOU：路径校验与打开之间存在竞态，尽力校验，不宣称安全封闭。
3. 进程终止：Windows 强杀默认不清理子进程树，超时命令可能留残留进程；策略集中 sys.rs，进程树清理留升级位。
4. 轨迹写入：单写者约定（单进程写 JSONL），不做跨进程文件锁（Windows 纯追加模式与锁语义互斥）。
5. 崩溃安全准确口径：整行写入 + flush，崩溃最多损失最后一行，读端容忍并丢弃末尾半行。
6. macOS 兼容性【待核验】：无实机验证条件，条件编译 + CI 矩阵补；当前后置，分发 macOS 前必须过。
7. **hub/worker 跨进程**[新增]：worker 被杀留半截轨迹（与桌面关窗同口径）；读侧容忍半行，T1 起补终态事件【待磨合】。
8. **默认终端 spawn**[新增]：依赖系统终端关联（wt/conhost/gnome-terminal/Terminal.app），按平台查表 + fallback，失败如实报错【待核验】。

## 六、来源（供溯源，均为参考、非需求）

初版 [S1]–[S10] 保留：Cargo Book Package Layout / Cargo Targets & Workspaces（虚拟 workspace 必设 resolver）/ std fs::canonicalize（Windows verbatim 路径）/ Tauri Project Structure / Rust Book 测试组织 / 一核多壳实证（codex-rs、lapce、espanso）/ lib.rs vs main.rs 社区共识 / Microsoft Naming a File（NTFS 大小写与 MAX_PATH）/ std process 与文件锁语义 / agentic harness 参考架构综述（gist: amazingvince）。
新增：
- [S11] chafa（hpjansson/chafa）：图像/SVG → ANSI/Unicode/Sixel 终端图形（含 C 库）——Logo 终端渲染成品方案；SVG 支持依赖可选构建组件 librsvg，构建环境须启用【待核验】。
- [S12] 冬瓜教程站（buchidonggua/dg-ai-notes）：Pi-Agent（earendil-works/pi）源码解读 + 二开实战（产出 docs/survey-dg-ai-notes.md）。
- [S13] 竞品机制调研（docs/survey-competitor-mechanisms.md，2026-10-05 已产出）：JEV fork layer / Pi 生态 / OpenCode TUI。
- [S14] 系统提示词考古（docs/survey-agent-prompts-*.md，2026-10-05 已产出）：8 家 agent 提示词全历史差异。

## 七、变更说明
- **v3（2026-10-05 复查整改）**：目录树计数 27→29（`find src -name '*.rs'` 实测），补 src/session.rs（会话库）与 src/config/settings.rs（设置档案）两行；T3 骨架引用 v2→v3（结构已定稿）；[S13][S14]"进行中"改"已产出"；[S11] chafa 补 librsvg 可选依赖前提【待核验】；布局决策 9 用户原话转述（公开仓库脱敏）。
- **v2（2026-10-05 改方向修订）**：目录树更新为 workspace 现状（三 crate + T1/T2 扩展位 + 产物实测）；"单 crate 起步 / 二进制封顶 2"口径废除（现实三 exe，拆分触发条件已兑现）；布局决策新增 5/6/7（双终端、launcher、stats），8 为安全单点复述并入实测加固事实；残余风险新增 7/8；来源新增 S11–S14。初版"分期扩展位 P1–P5"与"P5 拆分预案"收进本文第四节 T5 与布局决策 1；附录（两路盲审 30 条对账表）为历史审查记录，见 git 历史版本，本文不重复保留。
- v1（2026-10-02）：初版收口（27 文件落点、四接口、事件层四件、残余风险 6 条、两路盲审 30 条对账）。
