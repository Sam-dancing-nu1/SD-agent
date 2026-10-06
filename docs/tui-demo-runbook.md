# TUI demo 晨检手册（2026-10-06 交付）

> 性质：交付说明（含命令与路径，非纯净文档）。对应版本 0.1.0-alpha.1（0.x 迭代期，版本号规划见 docs/roadmap.md）。

## 一、怎么跑（30 秒上手）

```bash
cd F:/sd_agent
./target/debug/sd-tui          # hub 主页（大 Logo + 任务输入 + 历史 + Token 统计）
```

- hub 输入任务回车 → 弹新终端跑会话（WT/conhost；失败自动内嵌降级，任务不丢）。
- 会话终端里看流式输出、工具卡、审批弹窗；收尾自动存档，hub 历史即时刷新。
- `./target/debug/sd-tui --help` 看全量键位；`--version` 看版本；`--selfcheck` 无头自检。

## 二、键位（唯一出处 apps/tui/src/keymap.rs，? 键同源显示）

| 键 | 作用 |
|---|---|
| Enter | hub：新开会话并弹终端 / worker：发送 |
| Alt+Enter | 输入框换行 |
| Esc | 运行中：取消显示（真中断需核心取消通道，[待磨合]）/ 弹窗关闭 |
| q / Ctrl+C×2 | 退出（q 需输入框为空） |
| ? | 键位帮助浮层 |
| / | 斜杠命令（/doctor /stats /effort /retry /model /new …） |
| ↑ ↓ / PgUp PgDn / 滚轮 | 列表选择、输入回翻、对话区滚动 |
| Tab | 焦点轮转 |
| R | 失败后重试上一条任务 |
| 审批弹窗 | Y 放行 · A 本次全放行 · N 拒绝 |

## 三、demo 六要素落点

1. **审美**：apps/tui/src/ui/theme.rs（深色简洁技术风、琥珀金强调、暗灰干预透明）；hub 两态布局（初始=大 Logo+对话框；WORK=顶栏 Logo+输入、下方左 1/3 历史右 2/3 统计）。
2. **性能**：事件 16ms poll + 后台消息帧合并重绘（不逐字重绘）；流式逐字流入（核心 StreamObserver 三路 delta 直转）。
3. **最小闭环**：已真跑验收——`sd-agent run --yes "读取 src/lib.rs 第一行写入 .sd-agent/evidence/first_line.txt"` 跑通，git diff/验证输出/轨迹 42 行事件三样证据齐（轨迹 .sd-agent/traces/run-1791214120023-41868.jsonl）。
4. **修复能力**：错误必带修法（"按 R 重试"）、doctor 失败项给修复步骤、panic hook 恢复终端、非 TTY 快速拒绝、审批断开按拒绝收尾不悬死。
5. **诊断能力**：/doctor 六项体检（已实测全绿）+ /trace 轨迹 + /stats 统计 + selfcheck 六帧渲染回归。
6. **版本号规划**：workspace.package 统一 0.1.0-alpha.1（唯一出处根 Cargo.toml）；doctor/--version/状态栏同源显示；0.1.0-alpha.N→0.1.0→0.2.0→0.3.0→1.0.0 路线见 docs/roadmap.md 版本号规划节。

## 四、实测证据（2026-10-05 夜）

- `cargo build --workspace`：0 告警（Finished 10.19s）。
- `cargo test --workspace`：91 passed / 0 failed / 1 ignored。
- `sd-tui --selfcheck`：6 帧全过（hub 初始态 / worker 对话流 / 错误修复 / 帮助浮层 / 热力图 / 窄屏）。
- `sd-agent doctor`：六项全绿（版本栏显示 0.1.0-alpha.1）。
- 非 TTY 拒绝：`echo hi | sd-tui --worker` → exit 2。
- bash 无人值守审批：默认拒绝（安全侧行为，非 bug）；带 --yes 或 TUI 内按 Y 放行。

## 四·补丁：对抗性复查修复（复查子代理 24 问题 + 11 处注释失实，全部处置）

- 硬伤 4 件全修：工具卡按 tool_call_id 状态机（Hook 插队不断链、被拒不悬卡）；会话首次存档不再落空（追加语义）；assistant 消息带思维链正文入档（续跑回传防 400）；存档只追加新增（--session 冷启动 / /clear 不再吃掉旧消息）。
- 并发/体验：inbox 文件名毫秒+pid 防串号；拒绝熔断接入核心账本（RunApprovalState）；allow_all 会话切换即复位；Esc 取消后审批直拒；q 单按直退、Ctrl+C 双按确认（键位文案同步）；/new 先存档再重置；hub 里 /doctor 有状态栏回显。
- 滚动改尾部窗口语义（0=贴底跟随，流式输出自动跟随尾部）；光标按显示宽度定位（CJK=2 列近似）。
- 键位纪律收口：全部按键判定收进 keymap.rs（含编辑键），主循环零硬编码分支。
- 死代码/幻觉注释清零：ModelTurn 死路删除、11 处"注释与实现不符"逐条改真（含 /new 存档、q 语义、滚动语义、帧 3 自检名实相符等）。
- 文件纪律：app/mod.rs 拆出 app/archive.rs、app/commands.rs，全部 ≤500 行。
- 修复后复验：cargo build --workspace 0 告警 / cargo test 91 过 / --selfcheck 6 帧全过（帧 3 现覆盖真审批弹窗渲染）。

## 五、已知边界（如实，待拍板/待磨合）

1. "热键文件"按"键位定义文件"落地（apps/tui/src/keymap.rs + ?/help 同源表）——若原意是系统全局热键（如呼出 sdagent），明早说，另行补。
2. 鼠标：滚轮滚动已支持；点击选中历史、拖选复制（Shift+拖为终端原生）未做全量，[待磨合]。
2b. 会话存档只收对话正文（user/assistant 含思维链），工具调用与干预条目的完整留痕在轨迹文件（有意取舍，防会话库膨胀）；入档时间戳为存档时刻非原始时刻。
3. Esc 取消为 UI 级止损（停止显示后续输出，run 线程自然收尾）；真中断需核心取消通道。
4. /model /settings /conversations /trace 面板为文本态（弹出面板形式 [待磨合]）；/effort 已可改思考强度落盘；Esc 取消为 UI 级止损（run 线程自然收尾，真中断需核心取消通道）。
4b. 弹终端成功判定只到"进程拉起"（wt/cmd start 返回成功≠窗口出现）；窗口未出现时任务文件保留在 .sd-agent/inbox/ 待认领。
5. Logo 为 ASCII 占位，等 SVG 到货走构建期转换（chafa 流程 [待磨合]）。
6. 双终端弹新终端的实机旅程未代跑（会弹真实窗口）——请晨检时亲手走一遍：hub 输任务回车 → 看新终端跑会话 → 回 hub 看历史/统计。
