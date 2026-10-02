# 项目文件结构规划（P0 地基，决策台账类）

> 性质：决策/结构类文档（含第三方技术名与来源引用，非纯净文档）。
> 拍板：2026-10-02 —— 目标形态、目录树整体、文件粒度（多文件不限，以达到设计要求为准）经作者拍板；本规划经两路独立盲审（共 30 条问题）对抗性修订后收口。
> 配套读：docs/roadmap.md（P0–P5 分期与执行序）、docs/p0-brief.md（P0 范围）、docs/concept-v2.md（设计理念，纯净总纲）。
> 状态口径：**已定** = 作者拍板 / 官方文档实锤 / 实测；**待磨合** = 推演或机制设计；**待核验** = 须实测才能定。

## 一、定位与目标形态

- 目标形态【已定】：一个核心 + 两套壳（TUI 终端界面 + 桌面端 GUI），核心与 UI 解耦；平台 Windows / Linux / macOS，macOS 优先级低（无实机验证条件）。
- 文件粒度【已定】：文件数量不是约束（参照业界成熟项目成百上千文件的常态），以模块职责清晰、可维护为准；禁止为凑数建空壳文件。
- 阶段：P0 最小闭环开工前的地基确认。本规划是 P0–P5 全程的结构地图，实现期只许"加文件、换实现"，不许改依赖方向。

## 二、目录树（P0 落地：27 个 .rs 文件，均有实名职责）

```
sd-agent/
├── Cargo.toml / Cargo.lock
├── docs/                        # 设计文档（含本文件）
├── src/
│   ├── main.rs                  # 薄壳：解析参数 → 调 lib；零业务逻辑
│   ├── lib.rs                   # 模块树根 + crate 级文档
│   ├── cli.rs                   # 子命令面 doctor / run（手写解析，不引 CLI 框架）
│   ├── config/
│   │   ├── mod.rs               # 三类档案门面（环境/模型/任务）+ 认知纪律域数据位（白名单表/阈值表，纯数据）
│   │   ├── env.rs               # 环境档案：os_family / shell_kind / 编码 / 行尾 / 路径约定 / 代理 / 进程终止策略
│   │   ├── secret.rs            # Secret newtype（禁 Debug/Display，防日志泄密）
│   │   └── resource.rs          # 运行期资源分配 schema + 非差异型副作用登记表（硬约束 20）
│   ├── event/
│   │   ├── mod.rs               # 稳定门面（re-export 公共类型，路径永不变）
│   │   ├── contract.rs          # 事件契约：v / trace_id / seq / ts_unix_ms / kind / payload（强类型）+ 演化规则 + 分级处置标
│   │   ├── sink.rs              # EventSink（提交事件语义）+ JSONL 实现（P1 换 SQLite 实现）
│   │   ├── source.rs            # 读侧 replay（从 seq 回放 + 增量）+ 版本不匹配策略
│   │   └── hooks.rs             # 生命周期钩子唯一收敛点 emit_hook()（P0 只落盘；P1 扇出换实现）
│   ├── model.rs                 # ModelClient + 现成 OpenAI 兼容客户端库适配器
│   ├── sys.rs                   # 跨平台单点：路径归一化 / 进程终止 / 执行器选择（cfg 集中处）
│   ├── tools/
│   │   ├── mod.rs               # 工具目录 schema（稳定序列化 + 字节稳定断言）
│   │   ├── read.rs / write.rs / edit.rs / bash.rs   # 四工具（bash 名从模型惯例，执行器由环境档案裁决）
│   │   └── fs_guard.rs          # 路径基线：双字段记录 / 组件级比较 / 平台大小写策略
│   ├── policy/
│   │   ├── mod.rs               # dispatch 裁决（唯一模型驱动执行入口）
│   │   ├── rules.rs             # 纯数据规则表：危险命令黑名单（按平台分行）
│   │   └── approval.rs          # ApprovalPort 裁决往返口（P0=CLI stdin；P4=TUI 弹窗；P5=远程审批回传）
│   ├── context.rs               # 上下文组装：static_prefix() / dynamic_appendix() 分建 + 段标记 + 目标锚结构化
│   ├── agent.rs                 # 工具调用循环 + 轮数熔断（SD_AGENT_MAX_ROUNDS，默认 20）
│   ├── verify.rs                # 验证器：证据 = 事件(kind=verify_result) + 落盘 .sd-agent/evidence/
│   └── doctor.rs                # 六项体检（工具探针走 policy::dispatch 实测真通道）
├── tests/                       # P1 起：契约 fixture + common/ + CARGO_BIN_EXE 冒烟
└── .sd-agent/{traces,evidence}/ # 运行时产物（gitignore，含交互内容不入库）
```

## 三、布局决策

1. **单 crate 起步（lib + 薄壳 bin），不预先拆 workspace**【已定】
   - main.rs 只做装配、逻辑全进 lib（Cargo 官方布局与社区共识 [S1][S7]；集成测试可直调 lib）。
   - 拆分触发条件写死：出现第二个独立发布产物（P5 桌面壳）或编译时间实测成痛才拆。P4 观察面用 src/bin/ 多二进制复用 lib（二进制数量封顶 2 个 [待磨合]）。

2. **按组件职责分模块，不按五层工程地基分目录**【已定】
   - 十二件 × 五层是正交视角、组件跨层（concept-v2 第四节），按层建目录会让跨层组件无处安家。每模块头注释标注所服务的层。

3. **依赖方向单向，执行边界物理化**【已定，机制形态待磨合】
   - 依赖图（只许向下）：cli → agent → policy → tools；agent → model / event / context；verify / doctor → config + 各层探针接口；event、config 位于最底、零业务依赖。核心 lib 严禁依赖任何壳 crate。
   - 唯一**模型驱动**执行入口 = policy::dispatch()：tools 执行函数用受限可见性（pub(in crate::policy)）只允许 policy 调用。注意这是"模型驱动执行"的唯一入口，不是"进程内唯一执行通道"——verify / doctor 的确定性执行（git、探针）是 Harness 侧独立窄通道，显式声明、不经模型路径。doctor 的工具探针走 dispatch 实测真通道（防"doctor 全绿但通道坏"）。

4. **事件层四件首日立桩**【已定】
   - contract（契约 + payload 按 kind 强类型 + 演化规则：只增不改名、旧 kind 语义冻结）/ sink（提交事件语义，同步与异步由网关层决定，接口不承诺同步）/ source（读侧回放，支撑断点恢复与观察面）/ hooks（生命周期钩子唯一收敛点，对齐硬约束 14"单一调度入口"）。
   - 理由：两路盲审互证的头号缺口——只有写盘口则 P4 观察面订阅、P1 断点恢复、契约迁移全是结构性返工。拆文件不改公共类型路径（mod.rs 门面 re-export）。

5. **配置 = 三类档案 + 域数据位 + 平台字段**【已定】
   - 环境档案按硬约束 9 显式承载系统/shell/编码/路径约定/网络差异；resource.rs 按硬约束 20 预留端口/DB 实例/环境变量/依赖锁分配与副作用登记（P0 空表 + 登记点）；认知纪律域的白名单与阈值表以纯数据入档（P3 取数）。
   - 工具名 bash 保留【待磨合】：名从模型生态惯例（调用成功率优先），实际执行器（bash/cmd/powershell）由环境档案 shell_kind 查表裁决。

6. **安全单点 + 残余风险显式声明**【已定】
   - Secret newtype 单点防泄密；fs_guard 单点管文件类工具路径（双字段：原始输入 + 规范化形态进事件，绝不只存 canonicalize 形态——Windows verbatim 路径问题 [S3][S8]）；危险命令规则表纯数据、按平台分行（POSIX 模式拦不住 rd /s 一类）。
   - 残余风险（P0 不宣称封闭，见第六节）：bash 逃逸面、TOCTOU、进程树残留。

7. **抽象的度：四个口，其余具体类型**【已定】
   - 只立四个接口：EventSink / EventSource（事件层）、ApprovalPort（裁决往返：核心→UI→用户→回传）、ModelClient（P5 双模型实测）。其余用具体类型 + 模块可见性分层，不搞接口先行。
   - 工具目录序列化字节稳定（固定字段序 + 断言两次序列化字节相同），防毁前缀缓存（靶子 18）。

## 四、分期扩展位（P0 不建空壳，只按本节留缝）

- **P1**：event/sink_sqlite.rs 换实现（契约不动）；state.rs 状态机（流转矩阵表驱动、状态准入白名单为数据，P3 只加行不重构）；tests/ 契约 fixture + golden 回归架（用现成任务集，不自造）。
- **P2**：token 预算；context 四段全量（冻结前缀/域快照/对话历史/增量附录，只填 context.rs 实现不动依赖图）；记忆 L0 工作记忆 + 目标锚持久层（单源注入 + 强制重注入，硬约束 5/17）。
- **P3**：policy 交集裁决（域白名单 ∩ 状态准入）+ 探针 L1（哈希规则）挂 hooks；收敛三态检测。
- **P4**：src/bin/sd-observe.rs（TUI 观察面，经 event/source.rs 订阅，agent 调用点不返工）。
- **P5**：拆分预案见下节；记忆 L1/L2 检索、技能、探针小模型适配。

## 五、P5 拆分预案（双壳落地，届时复核）【待磨合】

- 形态：**非虚拟 workspace**——root 包保留 src/ + `[workspace] members=["apps/desktop"]`，从根上规避搬 src/ 与"虚拟 workspace 忘设 resolver"的坑（resolver 必设 '3'，[S2]）。
- apps/desktop/：桌面壳独立 crate（Tauri 官方工程结构：build.rs + 壳配置 + icons/ + capabilities/，[S4]）；ui/：前端资源目录。
- 协议 crate：事件/裁决协议的传输壳（独立进程 RPC 与远程只读接入，对应共享核心 D4"B 形态按需"）。
- 业界实证三例（GitHub 仓库实查，[S6]）：codex-rs、lapce、espanso 均为 workspace + 核心 lib crate + 薄壳装配 crate——与本预案同构。

## 六、残余风险与边界声明

1. bash 逃逸面：命令黑名单按模式匹配可被参数化执行绕过（如解释器 -c 类），bash 也能改变工作目录。P0 边界 = 黑名单雏形 + cwd 钉死工作区 + 全量审计留痕；真正的封闭靠隔离执行体（硬约束 6）分期到位。
2. TOCTOU：路径校验与打开之间存在竞态，P0 做尽力校验，不宣称安全封闭。
3. 进程终止：Windows 强杀默认不清理子进程树，超时命令可能留残留进程；处理策略集中在 sys.rs，进程树清理留升级位。
4. 轨迹写入：单写者约定（单进程写 JSONL），不做跨进程文件锁（Windows 下纯追加模式与锁语义互斥，[S9]）。
5. "崩溃安全"准确口径：整行写入 + flush，崩溃最多损失最后一行，历史行可回放，读端容忍并丢弃末尾半行。
6. macOS 兼容性【待核验】：无实机验证条件，三平台兼容以条件编译 + CI 矩阵补（P5 分发前必须过）。

## 七、来源（供溯源，均为参考、非需求）

- [S1] Cargo Book — Package Layout：https://doc.rust-lang.org/cargo/guide/project-layout.html （src/lib.rs + src/main.rs + src/bin/ + tests/ 惯例）
- [S2] Cargo Reference — Cargo Targets / Workspaces：https://doc.rust-lang.org/cargo/reference/cargo-targets.html 、https://doc.rust-lang.org/cargo/reference/workspaces.html （bin required-features 为包级、CARGO_BIN_EXE、虚拟 workspace 必设 resolver）
- [S3] Rust std — fs::canonicalize：https://doc.rust-lang.org/std/fs/fn.canonicalize.html （Windows 返回扩展长度路径、与其他应用不兼容）
- [S4] Tauri 官方 — Project Structure：https://v2.tauri.app/start/project-structure/ （桌面壳独立 crate + 前端资源目录）
- [S5] Rust Book 11.3 — Test Organization：https://doc.rust-lang.org/book/ch11-03-test-organization.html （单元测试测私有接口、集成测试走公开接口）
- [S6] 业界一核多壳实证：openai/codex（codex-rs workspace）、lapce/lapce（非虚拟 workspace）、espanso（workspace），各自仓库根 Cargo.toml
- [S7] 社区共识 — lib.rs vs main.rs：https://www.rustfaq.org/en/how-to-use-librs-vs-mainrs/ 、https://stackoverflow.com/questions/57756927 （bin 只调 lib，逻辑进 lib 才能被集成测试覆盖）
- [S8] Microsoft — Naming a File：https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file （NTFS 大小写不敏感、MAX_PATH、verbatim 前缀语义）
- [S9] Rust std — process::Command / process::Child / fs::File::lock 文档（参数不经 shell、kill 无优雅终止语义、Windows 纯追加打开无法加锁）
- [S10] 2026 agent harness 参考架构综述（gist: amazingvince，Blueprint for a Modern Agentic Harness）：runtime 拥有 loop/state/control、tool router 是副作用唯一网关、surface 层可替换——与决策 3/7 同构，取原理不照搬。

## 附录：独立审查对账（两路盲审，2026-10-02）

判定：两路均 PASS_WITH_CHANGES。问题 30 条（A 路结构/安全 17 条 + B 路双壳/跨平台 13 条），其中 high 10 条。全部处置如下：

| # | 路 | 级 | 问题（摘要） | 处置 |
|---|---|---|---|---|
| 1 | A | 高 | 提示组装无独立落点，P2 四段结构将重写循环体 | 吸收：context.rs 独立（决策 4 位） |
| 2 | A | 高 | "唯一执行入口"超出 Rust 可见性可保证范围 | 吸收：改"唯一模型驱动执行入口"+ pub(in crate::policy)（决策 3） |
| 3 | A | 高 | 三类档案 + 认知纪律域数据位无家 | 吸收：config/ 门面（决策 5） |
| 4 | A | 高 | 记忆 L0/目标锚持久层落点晚于 P2 排期 | 吸收：P2 扩展位补齐（第四节） |
| 5 | A | 高 | 平台差异不进环境档案，Windows 首日踩坑 | 吸收：env.rs 字段 + rules 按平台分行（决策 5/6） |
| 6 | B | 高 | 桌面壳不能是 src/bin/，官方结构要独立 crate | 吸收：P5 拆分预案（第五节） |
| 7 | B | 高 | 事件层只有写口，订阅/回放无接口位 | 吸收：source.rs + hooks.rs（决策 4） |
| 8 | B | 高 | fs_guard 缺跨平台基线（verbatim/大小写/符号链接） | 吸收：fs_guard 三基线 + 双字段（决策 6） |
| 9 | B | 高 | shell/编码/行尾不显式承载违反硬约束 9 | 吸收：env.rs 字段（决策 5） |
| 10 | B | 高 | 运行期资源分配/副作用登记无落点违反硬约束 20 | 吸收：resource.rs（决策 5） |
| 11 | A | 中 | EventSink 读侧（回放/断点）该立未立 | 吸收：并入 #7 |
| 12 | A | 中 | payload 无类型则契约只锁字段名 | 吸收：强类型 + 演化规则（决策 4） |
| 13 | A | 中 | event 拆目录改公共路径毁契约 | 吸收：mod.rs 门面（决策 4） |
| 14 | A | 中 | bash 是 fs_guard 覆盖不到的逃逸面 | 吸收：残余风险声明（第六节 1） |
| 15 | A | 中 | 状态机三态写死则 P3 重构 | 吸收：流转矩阵表驱动（第四节 P1） |
| 16 | A | 中 | policy 单文件扛不下 P3，规则与裁决混放 | 吸收：policy/ 三件（决策 3） |
| 17 | A | 中 | "单一调度入口"被偷换成 dispatch 点 | 吸收：hooks.rs 收敛点（决策 4） |
| 18 | A | 中 | 验证器产出无落盘点，"证据绑定"悬空 | 吸收：verify 双证据（事件 + 落盘文件） |
| 19 | B | 中 | 缺裁决往返口（确认弹窗跨进程） | 吸收：ApprovalPort（决策 7） |
| 20 | B | 中 | "同步落盘"固化进 sink 语义锁死 P1/P4 | 吸收：提交事件语义（决策 4） |
| 21 | B | 中 | 进程终止/文件锁缺位（Windows 语义差异） | 吸收：sys.rs + 单写者约定（第六节 3/4） |
| 22 | B | 中 | src/bin/ 多二进制的 feature/编译代价 | 吸收：二进制封顶 2（决策 1） |
| 23 | A | 低 | 工具目录序列化不确定毁前缀稳定 | 吸收：字节稳定断言（决策 7） |
| 24 | A | 低 | "崩溃安全"措辞过度 | 吸收：准确口径（第六节 5） |
| 25 | A | 低 | 遗漏三处枚举（MAX_ROUNDS/doctor 项/golden） | 吸收：目录树与 P1 扩展位补齐 |
| 26 | A | 低 | 布局决策未标状态、无来源位 | 吸收：全文标注 + 来源节 |
| 27 | B | 低 | 测试布局边界（common/ + CARGO_BIN_EXE） | 吸收：tests/ 注释 + [S2][S5] |
| 28 | B | 低 | 虚拟 workspace 必设 resolver 的坑 | 吸收：P5 预案（第五节） |
| 29 | B | 低 | 业界一核多壳实证缺位 | 吸收：三例实证（第五节 [S6]） |
| 30 | B | 低 | mod.rs 布局风格无官方偏好 | 不需改（口味问题，记录在案） |
