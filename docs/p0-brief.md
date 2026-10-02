# P0 开工交接（给新对话/其他 AI 的任务书）

> 性质：开工交接文档。配套读：docs/roadmap.md（P0 定义与执行序）、AGENTS.md（纪律）。
> 目标：完成最小闭环 demo 并验收——一条命令跑通真实任务，留 diff、退出码、轨迹三样证据，doctor 全绿。

## 一、环境（已就位，别重装）

- Rust 工具链在 E:\rust（RUSTUP_HOME=E:\rust\rustup、CARGO_HOME=E:\rust\cargo，已写用户环境变量）。
- 新 bash 里跑 cargo 用绝对路径 `E:/rust/cargo/bin/cargo` 或先 `export RUSTUP_HOME="E:/rust/rustup" CARGO_HOME="E:/rust/cargo"`（MSYS PATH 不吃 E:/ 格式，绝对路径最稳）。
- **装机铁律：任何软件工具一律装 E 盘一软件一目录，严禁 C 盘用户目录。**
- MinGW curl 等 native 工具不吃 /c/ 路径，用 C:/ 前缀；reg 查询加 MSYS_NO_PATHCONV=1。

## 二、模型接入（现成客户端，不自写）

- 端点：https://token-plan-cn.xiaomimimo.com/v1（OpenAI 兼容 chat_completions）。
- 模型名：mimo-v2.6-pro。
- 客户端库：async-openai（已在 Cargo.toml 登记，D5② 原则已批）。
- 凭据：只走环境变量 SD_AGENT_API_KEY，内容从 hermes vault / 本地配置注入，**禁落盘、禁进文档、禁打印**。

## 三、P0 范围（只做这些）

1. CLI 二进制（纯 CLI 无 TUI）：`sd-agent doctor` 与 `sd-agent run "<任务>"`。
2. 工具集四件：read / write / edit / bash（bash 带危险命令黑名单雏形；文件操作限工作区内）。
3. 工具调用循环 + 最大轮熔断（SD_AGENT_MAX_ROUNDS，默认 20）。
4. 轨迹落盘：JSONL 只追加，每行一个事件；事件契约 schema 首日定死（字段：v/trace_id/seq/ts_unix_ms/kind/payload），落 .sd-agent/traces/（已 gitignore 规则需确认覆盖）。
5. 验证器：收尾产出 git diff --stat 证据，禁口头完工。
6. doctor：API 凭据在位（不显内容）/ 工作区可写 / 命令环境 / 工具执行 / 端点连通（一次最小调用）/ 版本。

**不做**：TUI、SQLite、记忆、状态机、多执行体、golden 自造（P1+ 按 roadmap 分期）。

## 四、验收（可复查证据）

- `sd-agent doctor` 全绿；
- `sd-agent run "<本仓库内一个安全小任务>"` 跑通，产出：git diff、验证输出、.sd-agent/traces/*.jsonl 完整事件序列；
- cargo build 通过，无告警堆积；
- 提交按 AGENTS.md：小步提交、每条带描述正文、push 验证。

## 五、纪律速览（AGENTS.md 全文为准）

- 禁虚构、禁口头完工、验证重查真实文件；
- 凭据零入库；
- demo 直接在本仓库跑，任务只做安全小改动，全程 git 可回滚；
- 依赖引入超出手册三件（async-openai/tokio/serde 族）须先登记 D5② 再引用。
