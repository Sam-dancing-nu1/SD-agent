# P0 开工交接（给新对话/其他 AI 的任务书，v2）

> 性质：开工交接文档（含第三方技术名与环境事实，非纯净文档）。配套读：AGENTS.md（纪律）、docs/roadmap.md（P0 定义与执行序）、docs/project-structure.md（结构地图，P0 落点以此为准）、docs/concept-v2.md（设计理念总纲，纯净）。
> 目标：完成最小闭环 demo 并验收——一条命令跑通真实任务，留 diff、退出码、轨迹三样证据，doctor 全绿。

## 一、状态快照（接手时点）

- 已完成：概念总纲（concept-v2.md v3）、决策台账（decisions.md，D1–D11 + 门禁）、起步路线图（roadmap.md）、**项目文件结构规划（project-structure.md，经两路独立审查 30 条问题修订收口）**、工程骨架（Cargo.toml 依赖登记 + src/main.rs 占位 + .gitignore 含 .sd-agent/）。
- 待做：**P0 实现代码全部（一行未动工）**。本任务书就是开工指令。
- 执行序口径：roadmap.md 第七节 8 步；第 1 步（Rust 工具链）已实测完成（cargo/rustc stable 跑通、既有骨架 build 通过），从第 2 步（cargo 工程落地 + 依赖过 D5②）开始。

## 二、环境（已就位，别重装）

- Rust 工具链已装于 E 盘独立目录（一软件一目录铁律），RUSTUP_HOME / CARGO_HOME 已写入用户环境变量。
- 新 bash 里跑 cargo：先 `export RUSTUP_HOME="E:/rust/rustup" CARGO_HOME="E:/rust/cargo"`，或直接用绝对路径 `E:/rust/cargo/bin/cargo`（MSYS PATH 不吃 E:/ 格式，绝对路径最稳）。
- MinGW curl 等 native 工具不吃 /c/ 路径，用 C:/ 前缀；reg 查询加 MSYS_NO_PATHCONV=1。
- 网络代理环境差异已知：bash/curl 默认不走系统代理，模型端点连通失败先查代理。

## 三、模型接入（现成客户端，不自写）

- 端点：OpenAI 兼容 chat_completions（token 计划端点，地址以环境变量 SD_AGENT_BASE_URL 为准）。
- 模型名：以环境变量 SD_AGENT_MODEL 为准。
- 客户端库：async-openai（已在 Cargo.toml 登记，D5② 原则已批）。
- 凭据：只走环境变量 SD_AGENT_API_KEY，**禁落盘、禁进文档、禁打印**（Secret newtype 见 project-structure.md 决策 6）。

## 四、P0 范围（只做这些）

1. CLI 二进制（纯 CLI 无 TUI）：`sd-agent doctor` 与 `sd-agent run "<任务>"`。
2. 工具集四件：read / write / edit / bash（bash 带危险命令黑名单雏形、规则按平台分行；文件操作限工作区内，残余风险声明见 project-structure.md 第六节）。
3. 工具调用循环 + 最大轮熔断（SD_AGENT_MAX_ROUNDS，默认 20）。
4. 轨迹落盘：JSONL 只追加，每行一个事件；事件契约 schema 首日定死（v / trace_id / seq / ts_unix_ms / kind / payload 强类型），落 .sd-agent/traces/（gitignore 已覆盖）。
5. 验证器：收尾产出 git diff --stat 证据 + 验证输出落盘（事件 kind=verify_result + .sd-agent/evidence/），禁口头完工。
6. doctor 六项：API 凭据在位（不显内容）/ 工作区可写 / 命令环境 / 工具执行（走 policy::dispatch 实测真通道）/ 端点连通（一次最小调用）/ 版本与依赖信息。

**不做**：TUI、SQLite、记忆、状态机、多执行体、golden 自造（P1+ 按 roadmap 分期）。

## 五、结构落点（硬约束）

- 逐文件按 docs/project-structure.md 第二节目录树落地（27 个 .rs，禁空壳凑数）。
- 依赖方向只许向下（cli → agent → policy → tools；agent → model/event/context；event/config 最底），实现期只许加文件、换实现，**不许改依赖方向**。
- 事件层四件（contract/sink/source/hooks）与四个接口（EventSink/EventSource/ApprovalPort/ModelClient）首日立桩；tools 执行函数用 pub(in crate::policy) 收进 policy::dispatch 唯一模型驱动入口。
- 依赖引入超出手册三件（async-openai/tokio/serde 族）须先登记 D5② 再引用；CLI 参数手写解析，不引 CLI 框架。

## 六、验收（可复查证据）

- `sd-agent doctor` 全绿（六项齐全）；
- `sd-agent run "<本仓库内一个安全小任务>"` 跑通，产出三样证据：git diff、验证输出（含退出码，落盘文件）、.sd-agent/traces/*.jsonl 完整事件序列；
- cargo build 通过、无告警堆积；
- 提交按 AGENTS.md：小步提交、每条带描述正文、push 后回读验证；文档留痕按 docs/iterations.md 追加。

## 七、纪律速览（AGENTS.md 全文为准）

- 禁虚构、禁口头完工、验证重查真实文件；
- 凭据零入库；
- demo 直接在本仓库跑，任务只做安全小改动，全程 git 可回滚；
- 破坏性操作（删/覆盖/改环境）执行前必须告知用户并等确认；
- 失败 2 次换思路，不瞎试硬扛；排障顺序：git log → diff → 构建 → 配置 → 运行时。

## 八、建议加载的技能（suggested skills）

- git-commit-push-flow：任何提交走它（commit + push + 回读验证，禁只 commit 不 push）。
- docs-iteration-governance：完成后的文档留痕/审计/脱敏自查流程。
- diagnosing-bugs：遇硬 bug 走它的诊断循环（根因，不瞎试）。
- env-change-control / tool-install：若需装软件或改环境，变更单先行等确认（E 盘一软件一目录）。
