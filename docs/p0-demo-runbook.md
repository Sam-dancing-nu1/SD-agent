# P0 最小闭环 + 双壳 demo 晨检手册（2026-10-03 交付）

> 性质：操作/交接说明（含第三方技术名与环境变量名，非纯净文档）。配套读：docs/iterations.md（实现与复查留痕）、docs/decisions.md（D5② 登记 + 两条实现磨合记录）。
> 状态：**全部改动未 git 提交**（按你的指令留待晨检）；三个二进制已编译就位（target/debug/）。

## 一、怎么跑两个 demo（3 条命令）

前置：每条命令前先设环境变量三件（值不落盘，只进会话环境）：

    export SD_AGENT_BASE_URL="<你的 OpenAI 兼容端点，含 /v1>"
    export SD_AGENT_MODEL="<模型名>"
    export SD_AGENT_API_KEY="<你的 key>"

（本次实测可用的组合：MiMo payg 端点 + 对应 key + mimo-v2.6-pro-ultraspeed；token-plan 端点的 key 未在本机环境里找到，若你要用 token-plan 端点需自备 key。Windows cmd 用 set。）

1. TUI（终端界面，五模式）：`E:/rust/cargo/bin/cargo run -p sd-tui`
   - 模式 1 BOT 启动台（输入任务回车启动，可并发多 run）/ 2 会话流 / 3 事件观察（r 键回放轨迹）/ 4 doctor 体检 / 5 工具与环境
   - Tab 或数字键 1-5 切模式（输入框聚焦时先 Esc）；工具审批弹窗 y/n/a；q 退出
   - 无环境自检：`cargo run -p sd-tui -- --selfcheck`（六帧全 PASS）
2. 桌面端（Tauri 窗口）：`E:/rust/cargo/bin/cargo run -p sd-desktop`
   - 侧栏任务列表 + 新建任务、会话流实时气泡、审批弹窗、doctor 面板、trace 面板
3. CLI（P0 本体）：`cargo run -p sd-agent -- doctor` / `cargo run -p sd-agent -- run --yes "<任务>"`

## 二、验收证据（真实文件，可复查）

| 证据 | 路径 | 内容 |
|---|---|---|
| 验收轨迹 | .sd-agent/traces/run-1790964408689-27056.jsonl | 78 事件，seq 0–77 连续，含审批往返与 verify_result 全链 |
| 验证输出（含退出码） | .sd-agent/evidence/verify-1790964417814.txt | 3 探针输出 + exit_code |
| doctor 证据 | .sd-agent/evidence/doctor-1790966738581.txt | 六项全绿快照（复查修复后复跑） |
| git diff | 用 `git diff` 现场看 | 验收任务产物 = docs/iterations.md 末尾追加行 |
| 双壳测试轨迹 | .sd-agent/traces/run-1790965449805-1.jsonl | 桌面壳 mock 端点闭环实测产物（18 行，契约格式，run_finished 收尾） |
| 失败样例轨迹 | .sd-agent/traces/run-1790965380736-0.jsonl | 桌面壳早期测试的失败样例（5 行，run_failed 收尾：模型响应缺字段）——保留作失败路径实证 |

**证据时效对账（诚实口径）**：verify-*.txt 是 run 时点快照（当时 git diff 4 文件）；之后文档修订使当前 git diff 为 5 文件（iterations +13、decisions +20）+ 18 项 untracked（核心与双壳实现本体）。验收证据对"任务改动"仍精确（iterations +1 行）；实现本体的 diff 在你提交前用 `git diff` / `git status` 现场看（untracked 文件不进 git diff）。

## 三、复查与修复摘要（独立挑刺审查 4H/9M/11L）

- HIGH 4 项全修：①三处字节截断 panic（统一 sys::truncate_char_boundary，多字节安全测试钉死）②符号链接/junction 逃逸（fs_guard 加 canonical 真实位置防线）③黑名单设备路径死条目（raw string 修正 + 真实样例测试）④doctor 全绿无证据（现在落盘 doctor-*.txt，测试断言证据生成）
- MED 9 项全修：管道排空挂死（5s 超时）/ 大输出炸内存（限量读）/ 两处失实措辞同步 / trace_integrity 加首尾锚点+半行容忍 / 参数拼错静默忽略（deny_unknown_fields）/ doctor 测试打真网络（测试内隔离）/ workspace 形态磨合记录（决策 1）/ 证据时效（本表）/ cmd OEM 编码口径如实标注
- LOW：已修 6（锁中毒容错、versions 真判、字节稳定进 doctor、失败路径 RunEnd 钩子、空 old_string 拒绝、max_rounds 钳制 [1,200]、doctor 探针垃圾面）；**记录在案 5**（见下）
- 测试：36 全过（+3 新回归测试，删 1 凑数）；workspace build 零告警；TUI selfcheck 六帧 PASS

## 四、待你拍板/记录在案（不阻塞使用）

1. **project-structure.md 两处同步**（决策 3 run_tool 收口口径、目录树补 apps/ 两壳）——结构性文档改动按纪律等你确认，改动内容已写在 decisions.md 两条磨合记录里。
2. **D5② 表正式补签**（UI 三件依赖以你深夜双壳指令为引入授权，等你确认）。
3. 记录在案（P1 候选）：轨迹文件 trace_id 冲突时 append 混写（单写者约定内）；catalog 字节稳定缺跨构建 golden 基线；tool_call_id 去重校验；read 结果未带 raw+normalized 双字段尾注（write/edit 有）；依赖图补 sys 节点（lib.rs 口径）。
4. 记录在案（第二轮双壳审查）：**退出/关窗杀在飞 run 留半截轨迹**（无 run_finished 终态事件；读侧已容忍半行，风险=审计缺终态，P1 做取消广播+补终态）；TUI run 列表 CJK 列宽错位（修需引 unicode-width 依赖，走 D5② 登记后再引）；两壳事件流/会话流无上限（P1 环形缓冲）；桌面 icon 为极小占位图（正式图标后补）。
5. 双壳已知设计取舍：TUI 会话流模型正文走 ModelClient 装饰器旁路（事件契约不带大文本）；桌面多任务审批弹窗串行；两壳无 run 中止功能。
6. 本次实测环境三件值来源：SD_AGENT_API_KEY 测试时从本机 Hermes 配置盲注（未打印未落盘）；BASE_URL/MODEL 同为会话注入。**生产使用请自行设置你自己的三件**。
