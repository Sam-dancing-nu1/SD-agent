# 调研：TUI 版 + 桌面端 GUI 版技术栈选型（三平台同时研发）

> 文档性质：**调研/选型类证据文档**（含第三方工具名与来源 URL），不是纯净概念文档；纯净概念文档（concept-v2）不得引用本文的工具名。核心机制"原生实现、禁重型第三方框架"的铁律不变，本文只评估 UI 壳与通用基础设施类轻量库。
> 事实口径：所有事实格附来源编号（[S*]）；核验不到的一律标"待核验"。stars / 最近提交取自 2026-10-02 的 GitHub API 返回值，会随时间漂移。本文只覆盖 UI 与工程配套，不涉及记忆、状态机、探针等核心机制设计。

---

## 一、结论先行（整组标注【待拍板】）

### 1.1 推荐组合

| 位置 | 选择 | 一句话白话解释 |
|---|---|---|
| 核心语言 | **Rust** | 一种编译型系统语言：跑常驻进程省内存、编译出单个可执行文件直接分发、三平台都成熟；核心库还能被桌面壳直接链接，两套 UI 共用一份核心代码 [S1][S2]。 |
| TUI 框架 | **ratatui（终端层配 crossterm）** | Rust 生态的终端界面库：按"格子画布 + 差分刷新"画表格/弹窗/表单，底层 crossterm 负责跨平台终端控制（官方称支持 UNIX 与 Windows 终端、最低到 Windows 7）[S3][S4][S5]。 |
| 桌面壳 | **Tauri 2.x** | 用系统自带网页内核（不是自带浏览器）当渲染层、外壳逻辑用 Rust 写的桌面框架：包体小、内存低，桌面壳与核心是同一门语言，天然同进程共享核心 [S6][S7]。 |
| 共享核心模式 | **C 混合：A 同进程链接为主 + B 独立进程为辅** | 平时核心编译成库、TUI/桌面壳直接链接（低延迟）；同时核心保留"独立常驻进程 + 本地 RPC"形态，供无头/远程/多客户端场景复用同一协议（见第三节）。 |
| 工程配套 | **GitHub Releases 静态分发 + 各框架自带更新/签名机制 + CI 三平台构建矩阵** | 用托管平台发安装包，桌面端走框架的更新器（更新包强制签名），CI 在三平台各出一份产物；签名证书与公证是主要现实成本（见第四节）[S8][S9]。 |

推荐理由（合并说）：这套组合满足三条硬需求——(1) 核心常驻 + 工具执行 + 事件流，Rust 的内存/并发/单文件分发最贴；(2) 核心与 UI 解耦但不被迫引入 IPC：Rust 核心库可被 ratatui(TUI) 与 Tauri(桌面) 同时链接，一套核心两壳；(3) 三平台打包/签名/自动更新的配套在 Tauri 侧最完整（更新器覆盖 Windows/Linux/macOS 且强制签名）[S8]。该组合也是业界可对照的路线：OpenAI Codex CLI 用 Rust + ratatui 交付单文件终端 Agent [S12][S13]。

### 1.2 备选组合（至多一个，【待拍板】）

**Go 核心 + bubbletea(TUI) + "本地服务 + 浏览器/系统 WebView"桌面形态**。
- Go：谷歌的编译型语言，交叉编译一条命令出三平台单文件，工程速度比 Rust 快 [S10]。
- bubbletea：Go 生态最主流的 TUI 框架，按 Elm 架构组织界面，自带高性能格子渲染器与常用组件库 Bubbles [S10][S11]。
- 本地服务 + 浏览器：核心进程内置 HTTP/事件流服务，桌面端用系统 WebView 薄壳或直接浏览器访问——桌面形态不用另养一套 GUI 技术栈。
选它当备选的逻辑：团队若无 Rust 经验，Go + bubbletea 的上手与交付速度明显更快、单文件分发同样优秀；代价是桌面体验上限低（系统集成、托盘、原生弹窗都要自己补），且"浏览器形态"对权限确认弹窗这类交互的掌控弱于原生壳。

---

## 二、候选对比表

### 2.1 核心语言（常驻 runtime + 工具执行 + 有限状态机 + 事件流）

| 维度 | Rust | Go | TypeScript (Node/Bun) | Python |
|---|---|---|---|---|
| 常驻 runtime 开销 | 无 GC、空闲内存最小 | 有 GC，常驻内存中等 | Node 事件循环常驻内存中等偏上 | 解释器常驻，单进程内并发弱 |
| 工具执行/子进程/信号 | std + 成熟库，细粒度控制 [S2] | os/exec 简单直接，成熟 | child_process 成熟 | subprocess 成熟但异步需事件循环 |
| FSM + 事件流 | 枚举+模式匹配天然适合 FSM；tokio 异步流成熟（属通用基础设施，准入与否见待拍板 5） | goroutine + channel 天然适合事件流 | 单线程事件循环 + 流式 API，够用 | asyncio 够用，吞吐上限低 |
| 单文件分发 | 优秀（静态链接单二进制） | 优秀（单二进制，交叉编译最省事） | 差（需打包器或带运行时，Bun/Node 均需额外处理） | 差（需解释器或 PyInstaller 类打包，体积与兼容问题多） |
| 三平台成熟度 | Windows/macOS/Linux 均成熟（Tier 1）[S2] | 三平台成熟 | 三平台成熟 | 三平台成熟 |
| 与 ratatui/终端 UI 集成 | 原生同语言（ratatui 即 Rust）[S3] | 需另配 bubbletea | 需另配 Ink | 需另配 textual |
| 与桌面 GUI 集成 | Tauri 后端原生 Rust [S6] | 无原生桌面壳，走"本地服务+浏览器"或 WebView 薄壳 | Electron/Tauri 前端均用 JS/TS | 无原生桌面壳（Qt/Flutter 需跨语言绑定） |
| 业界对照 | Codex CLI：Rust 单二进制 + ratatui [S12][S13] | Crush（前 OpenCode 团队成员作品）Go + TUI [S18] | Gemini CLI：TypeScript + Ink [S14][S15]；OpenCode：TypeScript/Bun [S16][S17] | Aider：Python + prompt_toolkit/rich [S19][S20]；Hermes Agent：Python 核心 + TS 多壳 [S21] |

### 2.2 TUI 框架

| 维度 | ratatui (Rust) | bubbletea (Go) | textual (Python) | Ink (React/TS) |
|---|---|---|---|---|
| 渲染模型 | 格子画布 + 差分刷新，流式长输出走滚动区自行组装 [S3] | 官方称"高性能格子渲染器 + 内置色彩降采样" [S10] | 异步框架 + CSS 样式，长输出为组件化富文本 | React 组件渲染到终端，重排由布局引擎处理 [S22] |
| 表格/弹窗/表单组件 | 自带 widgets（表格/列表/弹窗等），组合式；表单类需自己搭 | Bubbles 组件库（输入框、视口、spinner 等）[S10] | 组件最全：按钮、树、数据表、输入、文本区，且带测试框架 [S11] | 生态靠社区 React 组件，无官方重型组件库 |
| 三平台终端兼容 | crossterm：UNIX 与 Windows 终端，最低到 Windows 7 [S5] | 官方 README 未列平台清单（待核验） | README 徽标明列 macOS / Linux / Windows [S11] | 依赖终端 ANSI 能力，Windows 细节待核验 |
| 与核心集成 | Rust 同语言，可同进程链接核心库 | Go 同语言 | Python 同语言 | 需 Node 运行时；若核心非 TS 则必跨进程 |
| 社区活跃度（2026-10-02 GitHub API） | 22.8k stars，最近推送 2026-09-28 [S3][S23] | 45.2k stars，最近推送 2026-10-01 [S10][S23] | 37.4k stars，最近推送 2026-07-11（间隔偏长，关注维护节奏）[S11][S23] | 40.0k stars，最近推送 2026-10-01 [S22][S23] |

### 2.3 桌面端

| 维度 | Tauri 2.x | Electron | Flutter Desktop | Qt / Slint / egui 类 | 本地服务 + 浏览器 |
|---|---|---|---|---|---|
| 三平台支持 | Windows 7+ / macOS 10.15+ / Linux（官方前置条件页明列）[S6] | 官方称原生运行于 macOS/Windows/Linux [S24] | 官方称可编译原生 Windows/macOS/Linux 桌面应用 [S25] | Slint：桌面/嵌入式/Web；egui(eframe)：Web/Linux/Mac/Windows/Android [S26][S27] | 只要浏览器，天然三平台 |
| 包体与内存 | 小（用系统 WebView，不内嵌浏览器）；数字待实测核验 | 大（内嵌 Chromium + Node.js，官方自述）[S24]；数字待核验 | 中（自带渲染引擎与运行时）；数字待核验 | 小-中（原生绘制）；数字待核验 | 客户端近零包体，核心进程常驻 |
| 与核心共享路径 | Rust 核心库直接链接进 Tauri 后端，同进程 | 核心非 JS 则须 Node 原生插件或旁路进程 | 核心须经 FFI/进程通信 | Rust 系（Slint/egui）可同进程；Qt 跨语言 | 天然进程分离，协议即接口 |
| 签名分发 | 官方 distribute 文档分平台给签名方案（macOS 公证、Windows/Linux 签名）[S7] | 自述"多数平台要求代码签名"，Forge 负责打包分发 [S9] | 依赖各平台工具链，配套待核验 | 自行对接各平台工具链 | 无安装包问题；内嵌壳仍需签名 |
| 自动更新 | 官方 updater 插件，覆盖 windows/linux/macos，更新包强制签名 [S8] | 官方 autoUpdater + Squirrel，元数据分 macOS/Windows 两套；更新教程全文未提 Linux（该点待核验）[S28] | 无官方内置更新器，待核验 | 无统一件，自建 | 自更新=替换核心进程，最简单 |

---

## 三、共享核心架构模式（A / B / C）

### 模式 A：核心编译为库，两壳同进程链接

```
 +---------------------+        +----------------------+
 |   TUI 壳 (ratatui)  |        |   桌面壳 (Tauri)     |
 |   +-----------+     |        |   +-----------+      |
 |   | 核心库    |<---->| 事件流 |   | 核心库    |<----->| WebView 渲染
 |   +-----------+     |  直推  |   +-----------+      |
 +---------------------+        +----------------------+
   （两份进程各链接一份核心库，共享同一份核心代码，不共享运行时实例）
```
取舍：事件流/流式 token 零序列化、延迟最低；工具调用权限确认弹窗=核心库回调 UI 层函数，最直接；崩溃隔离差（核心 panic 带走壳）；升级=重编译整个壳。

### 模式 B：核心独立进程，客户端经本地 RPC/IPC

```
 +----------------+     +----------------+     +----------------+
 | TUI 客户端     |     | 桌面客户端     |     | web/无头客户端 |
 +-------+--------+     +-------+--------+     +-------+--------+
         |  本地 RPC（unix socket / named pipe / loopback HTTP+SSE）  |
         +-----------+---------------+---------------+
                             |
                    +--------v---------+
                    | 核心进程（常驻）  |
                    | FSM/事件流/工具   |
                    +------------------+
```
取舍：崩溃隔离好（客户端崩不影响核心）；多客户端可同时接一个核心；升级核心不换壳。代价：协议要版本化、事件要序列化、流式 token 走 IPC 有额外开销；权限确认弹窗要做成"核心发请求 → 某客户端弹窗 → 用户裁决回传"的跨进程往返（好处是无头场景也能把弹窗投递到任意在线客户端）。

### 模式 C（推荐）：混合——同一套核心，两种装配形态

```
        同一套核心代码（crate / 模块）
        ┌─────────────────────────────┐
        │  状态机 / 事件流 / 工具执行   │
        └──────┬───────────────┬──────┘
   编译为库(A) │               │ 编译为可执行体(B)
        ┌──────v─────┐   ┌─────v──────────────┐
        │ TUI / 桌面壳│   │ 核心进程 + RPC 端点 │
        │ 同进程直连  │   │ TUI/桌面/web 远连   │
        └────────────┘   └────────────────────┘
        默认形态：单机单会话低延迟        扩展形态：无头/远程/多客户端
```
取舍：核心对外只暴露两层接口——进程内 API（A 用）与事件协议（B 用），两套 UI 各实现"直连/远连"两种接入；事件流推送、流式 token、权限确认弹窗在 A 是函数调用、在 B 是同构的协议消息，语义一致。崩溃隔离与升级按形态区分：直连形态随壳发布，远连形态独立滚动升级。代价：接口维护两层、协议要长期向后兼容、测试矩阵翻倍。

| 关注点 | A 同进程 | B 独立进程 | C 混合（推荐） |
|---|---|---|---|
| 事件流推送 / 流式 token | 直推，零拷贝 | 序列化 + IPC 开销 | 两形态都有，按场景选 |
| 工具权限确认弹窗 | 回调 UI 函数 | 跨进程请求-裁决-回传 | 协议同构，两形态一致 |
| 崩溃隔离 | 差 | 好 | 远连形态好，直连形态差 |
| 升级 | 随壳整体重发 | 核心独立升级 | 分形态处理 |
| 无头/远程/多客户端 | 不支持 | 天然支持 | 支持 |

---

## 四、三平台覆盖矩阵（推荐组合口径：Rust 核心 + ratatui + Tauri）

| 能力 | Linux | macOS | Windows |
|---|---|---|---|
| TUI 分发 | 单二进制 + 包管理器 | 单二进制 + Homebrew 类 | 单二进制 + winget/zip |
| 桌面包格式（Tauri） | deb / rpm / AppImage / Snap / Flatpak / AUR [S7]；配置参考明列 bundle targets 含 deb、rpm、nsis、msi [S29] | app / dmg（另有 App Store 打包配置）[S7] | msi / nsis（配置参考 [S29]） |
| 签名 | Linux 包签名（官方 distribute 签名分区）[S7] | 代码签名 + 公证（官方签名分区）[S7] | Windows 安装包签名（官方签名分区）[S7] |
| 自动更新 | updater 插件支持（平台键 linux-*）[S8] | updater 插件支持（darwin-*）[S8] | updater 插件支持（windows-*）[S8] |
| 更新包完整性 | 强制签名校验（不可关闭）[S8] | 同左 | 同左 |
| CI 构建 | Linux runner 便宜、可自托管 | macOS runner 计费最贵（数字待核验） | Windows runner 计费中等（数字待核验） |

对照备选（本地服务 + 浏览器）：三平台"分发/签名"两项基本消失（核心进程一份 + 浏览器访问），但 Windows/macOS 若要做托盘/开机自启等系统集成，仍回到各平台壳的签名问题。

---

## 五、风险与代价（推荐组合逐项）

**Rust 核心**
1. 开发速度与学习曲线：编译器严格性带来更慢的迭代节奏，团队无 Rust 经验时前几个月产能显著下降。
2. 编译时间：全量构建分钟级起步，CI 三平台矩阵的构建时长与缓存维护是持续成本。
3. 异步基础设施的准入边界：事件流常要异步运行时（如 tokio 类库），虽属通用基础设施，仍触发"轻量通用库需单独说明理由"铁律，需单独过审（见待拍板 5）。

**ratatui + crossterm**
1. 组件粒度低：表格/弹窗/表单是积木不是成品，表单类交互（如工具权限确认弹窗）要自己实现与维护。
2. 流式长输出的滚动区、超长 diff、终端 resize 重绘都要自己控制，性能调优在自己身上。
3. 高级终端特性（真彩、超链接、图形协议）各终端行为不一，兼容性矩阵要自己测。

**Tauri 2.x**
1. 三套 WebView 行为差异：WKWebView / WebView2 / WebKitGTK 的渲染与 API 有细微差别，UI 样式与富文本渲染要逐平台回归。
2. Windows 的 WebView2 运行时依赖：需要引导安装（官方提供 bootstrapper 等模式）[S29]，离线/内网环境要额外准备。
3. macOS 签名 + 公证流程繁琐，证书采购与账号维护是真实成本（数字待核验）。

**模式 C（混合）**
1. 接口双轨：进程内 API 与事件协议两层都要长期维护，协议要向后兼容，文档与测试成本高。
2. 测试矩阵翻倍：同一功能要在直连/远连两形态各验一遍。
3. 团队纪律要求高：两形态语义必须同构，否则 TUI 与桌面行为分叉，违背"两套 UI 共享同一核心"的初衷。

**工程配套**
1. CI 矩阵现实成本：三平台 ×（签名、公证、打包、更新元数据）流水线的搭建与维护量大，macOS runner 与公证是主要开销（具体数字待核验）。
2. 崩溃诊断：Rust 崩溃栈 + WebView 崩溃栈两套来源，日志/转储收集要分别接（minidump 类方案需引入库，属通用库准入问题）。
3. 更新通道出错即事故：更新包签名密钥的保管、轮换与吊销流程必须先于发布定好。

---

## 六、待拍板清单（5 项，逐项展开）

**决策 1：核心语言定档——Rust（推荐）还是 Go（备选）**
含义：定下常驻 runtime、工具执行、FSM、事件流这四件事的实现语言，也基本定下后续招聘与团队技能结构。影响：Rust 路线下 TUI 与 Tauri 桌面壳都能同语言共享核心库、单文件分发最干净，但迭代速度慢；Go 路线交付更快、交叉编译最省事，但桌面端没有原生壳，只能走"本地服务 + 浏览器"，与 Tauri 类原生壳的集成要绕路。推荐：Rust；若团队明确无 Rust 能力且产品要抢时间，则选 Go 并接受备选组合的桌面形态。

**决策 2：TUI 框架定档——ratatui（推荐）还是 bubbletea/textual/Ink**
含义：定下终端界面的渲染与组件体系，决定流式输出、表格、弹窗、表单的实现成本。影响：跟随核心语言走——Rust 选 ratatui（Codex CLI 同路线 [S13]），Go 选 bubbletea（组件库 Bubbles 现成），Python 选 textual（组件最全但长输出性能与维护节奏需盯），TS 选 Ink（Gemini CLI 同路线 [S15]）但意味着核心也得是 TS 或跨进程。推荐：ratatui；注意表单/弹窗组件要预留自研工作量。

**决策 3：桌面壳定档——Tauri（推荐）还是 Electron 还是"本地服务 + 浏览器"**
含义：定下桌面端的渲染载体、包体内存档次、签名与更新配套的现成程度。影响：Tauri 包体小、与 Rust 核心同进程、更新器与签名配套官方齐 [S7][S8]，但吃 WebView 差异与 macOS 公证成本；Electron 前端生态最顺但内嵌 Chromium、包体内存大 [S24]；"本地服务 + 浏览器"分发最省、天然无头，但系统集成与弹窗体验弱。推荐：Tauri；若桌面端只是"核心的远程视图"而非重度交互工作台，则"本地服务 + 浏览器"可以降级为第三形态而不做重投入。

**决策 4：共享核心模式定档——C 混合（推荐：A 为主、B 为辅）**
含义：定下核心对外的接口层次：只做进程内库（A）、只做独立进程 + RPC（B）、还是两层并存（C）。影响：直接决定流式 token 与事件流的延迟、权限确认弹窗的交互通路、崩溃隔离能力与升级方式（详见第三节对比表）。推荐：C——单机单会话走 A 保证体验，无头/远程/多客户端走 B 复用同一事件协议；代价是接口双轨与测试矩阵翻倍，必须靠"两形态语义同构"的验收纪律兜住。

**决策 5：轻量通用库准入清单 + 分发/签名/更新配套方案**
含义：(a) 逐个过审本文涉及的通用基础库（异步运行时、序列化、HTTP 服务、日志/转储、更新器、嵌入模型库等），明确哪些准入、哪些自研——这是铁律 3"轻量通用库需单独说明理由并等用户确认"的落地动作；(b) 同时定分发策略：自建更新服务器还是托管平台静态 JSON、三平台签名证书的采购与密钥托管、CI 矩阵的 runner 与缓存方案。影响：决定工程配套成本的量级与发布流程的合规性（更新包签名在 Tauri 更新器里不可关闭 [S8]，密钥管理必须先行）。推荐：先出准入清单再动工；分发先走"托管平台静态元数据 + 强制签名更新包"，更新服务器自建押后。

---

## 七、待核验清单（12 条）

1. Claude Code 的 TUI 技术栈（React/Ink）只有二手拆解与 Ink 项目自述列名佐证，未见 Anthropic 官方技术说明。
2. Bubble Tea 在 Windows 终端的渲染兼容性细节：官方 README 未列平台清单。
3. Ink 的 Windows 终端支持细节与其布局引擎实现（一手说明未核验到）。
4. Electron 自动更新对 Linux 的支持：官方更新教程全文未出现 Linux，是否存在官方方案待核验。
5. 各桌面方案的包体/内存量级数字（Electron/Tauri/Flutter 对比）——无一手实测数据，本文只作定性比较。
6. Qt 的许可条款细节（LGPL/GPL/商业三线）未核验官方条款原文；若考虑 Qt 必须先过许可合规。
7. OpenCode 桌面端（packages/desktop）的技术构成与成熟度，仅核到仓库存在该包。
8. Flutter Desktop 的打包/签名/自动更新配套（是否需第三方方案）未核验。
9. Rust 异步运行时、日志/崩溃转储等通用库的"轻量通用库"准入结论（需用户逐项确认）。
10. CI 三平台构建的现实成本数字（macOS runner 计费、Windows 签名证书、macOS 公证费用）。
11. textual 的流式长输出渲染性能（未见一手 benchmark；其最近推送距今约 3 个月，维护节奏待观察）。
12. Aider 终端交互层的内部分工（prompt_toolkit 与 rich 各承担什么）仅核到依赖声明。

---

## 八、来源清单（全部 URL，2026-10-02 检索）

TUI 与终端层：
- [S3] https://github.com/ratatui/ratatui （README/仓库；stars、最近推送另经 [S23]）
- [S4] https://crates.io/crates/ratatui （"A library that's all about cooking up terminal user interfaces"）
- [S5] https://github.com/crossterm-rs/crossterm （README："cross-platform…UNIX and Windows terminals down to Windows 7"）
- [S10] https://github.com/charmbracelet/bubbletea （README：Elm 架构、高性能格子渲染器、Bubbles 组件库）
- [S11] https://github.com/Textualize/textual （README：macOS/Linux/Windows 徽标、CSS、组件库、可在终端或浏览器运行）
- [S22] https://github.com/vadimdemedes/ink （仓库简介："React for interactive command-line apps"；README 列 Claude Code 为其应用）

桌面与分发：
- [S6] https://v2.tauri.app/start/prerequisites/ （系统要求：Linux / macOS Catalina 10.15+ / Windows 7+；Rust 必备、Node 按需）
- [S7] https://v2.tauri.app/distribute/ （Linux：deb/Snap/AppImage/Flatpak/RPM/AUR；macOS：app/dmg；分平台签名章节）
- [S8] https://v2.tauri.app/plugin/updater/ （更新器支持 windows/linux/macos；更新包签名强制、不可关闭）
- [S29] https://v2.tauri.app/reference/config/ （bundle targets："deb, rpm, nsis and msi"；WebView2 安装模式配置）
- [S24] https://www.electronjs.org/ （"Electron embeds Chromium and Node.js"；原生支持 macOS/Windows/Linux）
- [S28] https://www.electronjs.org/docs/latest/tutorial/updates （Squirrel + autoUpdater；macOS 与 Windows 元数据格式不同）
- [S9] https://www.electronjs.org/docs/latest/tutorial/application-distribution （推荐 Electron Forge 打包分发；"多数平台要求代码签名"）
- [S25] https://docs.flutter.dev/desktop （"compiling a native Windows, macOS, or Linux desktop app"）
- [S26] https://github.com/slint-ui/slint （声明式 GUI：嵌入式/桌面/移动/Web）
- [S27] https://github.com/emilk/egui （immediate mode；eframe 支持 Web/Linux/Mac/Windows/Android）
- [S2] https://doc.rust-lang.org/book/（Rust 语言与工具链参考；平台支持以 https://doc.rust-lang.org/nightly/rustc/platform-support.html 为准）

业界对照（UI/语言栈事实）：
- [S12] https://github.com/openai/codex （Codex CLI 仓库，GitHub API 语言统计 Rust）
- [S13] https://github.com/openai/codex/blob/main/codex-rs/tui/Cargo.toml （codex-tui 依赖 ratatui、crossterm、tokio）
- [S14] https://github.com/google-gemini/gemini-cli （TypeScript 仓库）
- [S15] https://github.com/google-gemini/gemini-cli/blob/main/package.json （依赖 ink（fork 发行版），即 React 终端 UI）
- [S16] https://github.com/anomalyco/opencode （原 sst/opencode，TypeScript 仓库）
- [S17] https://github.com/anomalyco/opencode/blob/dev/packages/tui/package.json （@opencode-ai/tui 依赖 @opentui/core、@opentui/solid（SolidJS））；根 package.json 显示 packageManager: bun
- [S19] https://github.com/Aider-AI/aider （Python 仓库；README："AI Pair Programming in Your Terminal"）
- [S20] https://github.com/Aider-AI/aider/blob/main/requirements/requirements.in （依赖含 rich、prompt_toolkit、litellm）
- [S21] https://github.com/NousResearch/Hermes-Agent/blob/main/package.json （npm workspaces：ui-tui、apps/desktop、web；postinstall 指向 python run_agent.py——Python 核心 + TS 多 UI 壳）
- [S18] https://thenewstack.io/terminal-user-interfaces-review-of-crush-ex-opencode-al/ （二手来源：Crush 为 Go TUI，作者有 OpenCode 背景——仅供旁证）

活跃度统计口径：
- [S23] https://api.github.com/repos/<owner>/<repo> （stargazers_count、pushed_at，2026-10-02 取数；涉及 ratatui/ratatui、charmbracelet/bubbletea、Textualize/textual、vadimdemedes/ink、tauri-apps/tauri、electron/electron、flutter/flutter、slint-ui/slint、emilk/egui、openai/codex、google-gemini/gemini-cli、Aider-AI/aider、anomalyco/opencode、NousResearch/hermes-agent）

---
变更说明：本文为首次落盘的选型调研文档；结论全部标注【待拍板】，未经用户拍板前不作为实现依据。
