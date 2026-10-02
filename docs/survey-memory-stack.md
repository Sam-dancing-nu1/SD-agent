# 记忆系统技术栈选型调研（memory-stack survey）

状态：调研文档，供拍板用；推荐项均标 **[待拍板]**，事实格附来源编号（[S#] 对应文末来源清单）。
范围：记忆系统（概念总纲"组件 4 记忆系统"记忆 L0–L4，见 F:\sd_agent\docs\concept-v2.md，只读引用）的存储层 / 嵌入模型 / 推理运行时 / 检索方案选型。本文件允许出现具体技术名（选型调研性质）；概念总纲仍守纯净纪律，本调研结论不得直接写入纯净文档。
硬约束回放：核心机制原生实现，现成 agent 记忆框架/中间件只能作参照系（第五节），不得作为引入对象；轻量通用库（数据库驱动、嵌入模型库）可引入但需单独说明理由；零现金成本、本地运行优先、Linux/Windows/macOS 三平台。
调研环境说明：本机网络部分站点（huggingface.co、ai.google.dev 等）无法直连，凡仅经搜索引擎摘要交叉印证的条目已在正文标注或列入待核验清单；GitHub/ sqlite.org / arxiv.org 为直读原文。

## 一、结论先行（推荐组合 [待拍板]）

**推荐组合**（一句话）：存储用 **SQLite 单文件库**（关系表承载事实/痕迹/实体邻接表 + **sqlite-vec** 扩展走向量检索 + **FTS5** 走 BM25 全文检索），按 L1 / L2+L3 / L4 分三个库文件；嵌入用 **bge-m3**（轻量档备选 EmbeddingGemma），经 **ONNX Runtime** 在**语言无关的记忆服务进程**里跑；检索走 BM25+向量双路 + RRF 融合 + 本地轻量 reranker（bge-reranker-v2-m3）+ 查询时时间衰减与作用域过滤。全程本地、零现金成本。[待拍板]

名词白话解释（它是谁/干嘛/为什么选它）：
- **SQLite**：一个进程内运行的单文件关系型数据库，三大平台、零服务部署，几乎所有语言都有驱动——满足"本地零服务 + 语言可换"两个硬条件。
- **sqlite-vec**：SQLite 的向量检索扩展（纯 C、无依赖，Linux/macOS/Windows/WASM 都能跑 [S2]），把向量存成 vec0 虚拟表、在 SQL 里做 KNN 查询——向量和关系数据同一个库、同一个事务，备份回滚只管一个文件。
- **FTS5**：SQLite 内置全文检索模块，自带 bm25() 排序函数 [S4]——关键词检索零新增依赖。
- **bge-m3**：智源（BAAI）的多语言嵌入模型，中文友好，最长输入 8192 token，同时产出稠密/稀疏/多向量三种表示 [S16][S17]——中文语义检索主力。
- **EmbeddingGemma**：Google 的 308M 参数嵌入模型，多语言、量化后可在 <200MB 内存跑 [S19][S20]——低配机器的轻量档。
- **ONNX Runtime**：微软的跨平台推理运行时（MIT 许可，支持 Windows/Linux/macOS CPU 构建 [S25][S26]）——嵌入模型的执行引擎，与宿主语言解耦。
- **bge-reranker-v2-m3**：智源的多语言轻量交叉编码器重排模型 [S18]——把混合检索的前 N 条重排，压"召回了但排不对"。
- **RRF（倒数排名融合）**：把多路检索各自排名按名次倒数加权合并的通用技术，不依赖任何框架——混合检索的融合层。
- **记忆服务进程**：自研的独立本地进程，独占记忆存储与嵌入推理，对外只开窄接口；核心运行时换语言（Rust/Go/TS/Python 任一）不牵动记忆栈。

**备选组合**（一句话）：存储层换成 **LanceDB**（嵌入式列存向量库，向量+全文+SQL 查询、自动版本化 [S7]），承载 L2 向量与全文检索，其余（嵌入模型、运行时、检索策略）不变；换它换来版本化与大数据量性能上限，代价是双存储格式、实体关系仍要自建、Go 侧绑定现状待核验。[待拍板]

## 1.1 存储层候选对比（维度 × 候选）

| 候选 | 本地零服务 | 三平台 | 并发写入 | 备份回滚 | 腐蚀公理适配（追加留痕/可回滚） | 语言面 | 来源 |
|---|---|---|---|---|---|---|---|
| SQLite + sqlite-vec + FTS5 | 进程内单文件，零服务 | 官方声明 Linux/macOS/Windows/WASM | WAL 模式：多读者+单写者互不阻塞 [S5] | VACUUM INTO / 备份 API / 文件级快照 [S6] | 事务+追加式表设计天然适配：只插新版本行、旧行标 superseded，人工标记列由代码保护 | C 库；驱动覆盖四语言候选；sqlite-vec 有 Python/Node/Ruby/Go/Rust 绑定 [S2] | [S2][S4][S5][S6] |
| 纯文件（JSONL/Markdown/Parquet） | 零服务 | 文件系统差异（编码/换行/路径）需自管 | 需自建文件锁，无事务 | 复制即备份，可挂版本控制 | 追加天然适配；但检索/去重/衰减全靠自建 | 各语言自带 | 概念总纲已判"拒绝纯文档存储"作主存储 [S1]；仅适合归档导出 |
| LanceDB（嵌入式） | 进程内嵌入式，无服务器 [S7] | Rust 核心，跨平台（具体预编译包待核验） | 事务/并发写语义待核验 | 自带自动版本化（zero-copy）[S7] | 版本化利于回滚；"只追加留痕"语义需自建 | Python/TypeScript/Rust SDK [S7]；Go 绑定待核验 | [S7][S8] |
| faiss / hnswlib（纯算法库） | 纯库，无存储层 | faiss C++/Python [S9]；hnswlib 头文件 C++11/Python [S10] | 不适用（无存储） | 持久化/备份全自建 | 不适用（无审计语义） | C++ 为主 | [S9][S10] |
| Chroma embedded | Python 进程内嵌入式 [S11] | Python/JS 客户端 [S11] | 待核验 | 持久化目录语义待核验 | 待核验 | Python/JS | [S11][S12] |
| Qdrant embedded（local mode） | Python 客户端 local mode：同 API、免服务器，:memory: 或本地路径 [S13] | 本体 Rust，三平台（待核验预编译现状） | 待核验 | 待核验 | 待核验 | Python/JS/Rust 等客户端 [S13] | [S13][S14] |
| PostgreSQL + pgvector | 需常驻服务进程（违反"零服务"倾向） | 可跑但运维重 | 强（服务端） | 服务端备份体系成熟（待核验细节） | 靠表设计可满足 | 各语言驱动齐全 | [S15] |

结论要点：SQLite 组合在"零服务、三平台、备份回滚、追加留痕"四项上都有可核验的原生机制；faiss/hnswlib 只是算法库、不含存储与留痕语义，适合做内部检索内核而非存储层；PostgreSQL+pgvector 能力强但引入常驻服务，与"本地优先、零服务部署"冲突，仅当未来出现多机共享 L4 需求时再议。

## 1.2 嵌入模型候选（中文效果 / 内存 / 维度 / CPU / 三平台）

| 候选 | 中文/多语言 | 维度与输入 | 内存/CPU | 三平台可用性 | 来源 |
|---|---|---|---|---|---|
| bge-m3（推荐主力，待拍板） | 100+ 语言，中文为主打语种之一 | 稠密 1024 维（待核验）；最长 8192 token [S17]；兼产稀疏/多向量 [S16][S17] | 中大型（约 0.5B 级，待核验），CPU 可跑、吞吐待实测 | 权重通用，推理运行时决定平台；GGUF 与 ONNX 转换件均有社区版（待核验） | [S16][S17] |
| EmbeddingGemma 300m（轻量档候选） | 100+ 语言 [S19][S20] | 768 维，MRL 可截到 128 维；输入 2048 token [S21]（待核验） | 308M 参数，量化后 <200MB RAM [S20]，面向端侧 | 权重通用（待核验） | [S19][S20][S21] |
| gte-multilingual-base | 多语言（含中文） | 768 维、8192 token、约 305M 参数 [S22][S23]（待核验） | CPU 可跑（待实测） | 权重通用（待核验） | [S22][S23] |
| nomic-embed-text-v1.5 | 以英文为主 | 768 维（MRL 64–768）、8192 token [S24] | 轻量 | 权重通用 | [S24] |
| bge 系列小模型（如 bge-small-zh 等） | 中文专用档 | 见 FlagEmbedding 模型清单 [S18]（具体参数待核验） | 最轻 | 权重通用 | [S18] |

选择倾向：中文效果优先 → bge-m3 主力；内存敏感机器 → EmbeddingGemma 或 bge 小模型；nomic-embed 英文为主，不做主力。中文效果的量化对比（C-MTEB 等榜单随时间变动）须按拍板时点复核，见待核验清单。

## 1.3 推理运行时候选

| 运行时 | 平台 | 语言面 | 量化/CPU | 来源 |
|---|---|---|---|---|
| ONNX Runtime（推荐，待拍板） | Windows/Linux/macOS CPU 构建明确列出 [S26]；MIT 许可 [S25] | C API 为核心，多语言绑定（清单待核验） | int8 量化 + 图优化，CPU 友好 | [S25][S26] |
| llama.cpp 系（备选） | Unix + Windows 均有官方示例命令 [S29] | C/C++ 核心，社区绑定多（待核验） | GGUF 量化，CPU 场景成熟；llama-embedding 工具直出嵌入 [S29] | [S28][S29] |
| candle（Rust 原生候选） | 跨平台（Rust 编译目标） | Rust 一体；其他语言需 FFI | 支持量化与 GPU [S27] | [S27] |

## 1.4 检索方案（落地方式）

1. **双路召回**：FTS5 `MATCH` + `bm25()` 排序 [S4]（关键词路）；vec0 表 `MATCH` 向量 + `ORDER BY distance`（语义路，[S2] 用法见官方 README）。两路各自带 WHERE 过滤：作用域（会话/用户/仓库/域）、时间窗口、类型。
2. **融合**：RRF 按名次融合（通用算法，自研约几十行），宁缺毋滥 + 硬性 token 预算截断（概念总纲纪律 [S1]）。
3. **重排**：本地 bge-reranker-v2-m3（多语言轻量交叉编码器 [S18]）只对前 N 条重排；低配可降级为"仅双路融合"。CPU 延迟待实测。
4. **时间衰减**：不写回、不改行——衰减在查询时对 `timestamp` 做纯函数计算并入总分，历史原样保留（"过期但曾经成立"可追溯，天然防腐蚀）。
5. **置信度字段**：事实表列（confidence + evidence_source + scope + 生效窗口）；置信度变更 = 插入新版本行 + 旧行标 superseded，禁止 UPDATE 改写（腐蚀公理：只追加留痕、可回滚 [S1]）。人工标记为独立列，代码层拦截任何后台任务写该列。
6. **矛盾检测**：入库前对相似候选（向量近邻）做语义相反检测，命中走人工确认或降置信度（概念纪律 [S1]；判别实现自研，不引框架）。
7. **实体关系**：关系表/邻接表（entity 表 + edge 表），检索时按实体过滤/扩展（概念总纲明确不引重型图数据库 [S1]）。

## 1.5 五层落点映射表（L0–L4 × 推荐落点 × 理由）

| 层 | 推荐落点 [待拍板] | 理由 |
|---|---|---|
| L0 工作记忆 | 进程内存结构（目标锚区 + 当前目标/证据的结构化对象）；关键副本单源落 L2+L3 库的"工作记忆快照表"，每轮由代码重注入 | L0 是高频读写、会话内生命周期；内存是唯一低延迟载体，但唯一副本绝不放上下文（公理四 [S1]），故持久层留单源副本 |
| L1 情景记忆（全量轨迹） | 独立 SQLite 库文件 trajectory.db：追加式事件表（时间戳、事件类型、追踪标识），只按时间窗/事件类型检索 | 写多读少、体量最大；独立文件便于整库归档/删除/回滚，崩溃域与记忆库隔离；全量轨迹不过嵌入、不进向量索引 |
| L2 语义记忆 | 与 L3 同库 memory.db：事实表（证据来源/置信度/时间戳/作用域/生效窗口/superseded 链）+ vec0 向量表 + FTS5 表 + 实体邻接表 | 结构化+向量+全文一体，一事务内写入保证一致；双路召回（语义/关键词）+ 两路过滤（实体/时间）在同一 SQL 里组合（口径与 1.4 一致） |
| L3 反思记忆 | memory.db 内独立表组（提炼产物：经验/心智模型/踩坑），异步批量写入；产物一律落盘可复查 | 与 L2 同库省一次部署但分表隔离；反思是异步限量（概念纪律 [S1]），独立表便于单独回滚/重提炼 |
| L4 共享记忆（独立子组件） | 独立库文件 shared.db + 独立读写接口（记忆服务进程暴露窄 API），痕迹表结构同 L2 事实表（来源/时间戳/置信度/作用域/目标锚/证据+衰减字段） | 概念定案 L4 与私有记忆是并列独立组件、只传结构化痕迹 [S1]：物理隔离防污染，读写走接口便于审计与权限分级 |
| 跨层共用/隔离 | L2+L3 共库分表；L0 内存+快照表；L1、L4 各自独立库文件 | 隔离判据：体量与备份粒度（L1）、信任边界（L4）、生命周期（L0 会话级）；备份=按库文件分别快照，回滚粒度与腐蚀面匹配 |

## 1.6 语言无关 vs 语言绑定（集成模式分析）

并行调研尚未定核心语言（Rust/Go/TypeScript/Python 四候选），本节只分析集成模式，不替语言选型做决定。

模式 A：语言无关（独立记忆服务进程）
```
 +-----------------------------+   本地窄接口    +--------------------------------+
 | 核心运行时（四语言任一）      | <=============> | 记忆服务进程（自研）            |
 |  任务循环/网关/TUI/插件      | HTTP/unix socket|  SQLite+sqlite-vec+FTS5 存储   |
 |                             |   /stdio 任选   |  ONNX Runtime 嵌入与重排        |
 +-----------------------------+                 |  混合检索/衰减/四杠杆（自研）   |
                                                 +--------------------------------+
```
模式 B：语言绑定（同进程库）
```
 +--------------------------------------------------------------+
 | 核心运行时进程（语言定死后绑死生态）                            |
 |  任务循环  |  SQLite 驱动 + sqlite-vec 绑定（同进程 C 库）       |
 |            |  ONNX Runtime / llama.cpp 绑定（FFI）              |
 |            |  检索/衰减/合并逻辑随宿主语言实现                   |
 +--------------------------------------------------------------+
```

绑定现状事实：SQLite 是 C 库、四语言均有官方或成熟驱动（覆盖面广，具体清单待核验）；sqlite-vec 明确提供 Python/Node/Ruby/Go/Rust 绑定 [S2]；ONNX Runtime 以 C API 为核心、多语言绑定（清单待核验 [S25]）；llama.cpp 社区绑定多（待核验）；LanceDB 官方 SDK 为 Python/TS/Rust [S7]——四语言候选里 Go 侧最可能缺官方绑定（待核验）；Chroma 仅 Python/JS [S11]。

对比：
| 维度 | 模式 A 语言无关 | 模式 B 语言绑定 |
|---|---|---|
| 换核心语言成本 | 近零（接口不动） | 换语言=重接全部绑定与 FFI |
| 性能 | 多一跳本地 IPC（毫秒级，检索路径可批量化摊薄） | 零 IPC，嵌入批处理最省 |
| 部署 | 多一个进程（可由单一启动入口托管，符合硬约束 8 [S1]） | 单进程最简 |
| 崩溃域 | 记忆写入与主循环隔离（符合"观察者异步"思想） | 共命运 |
| 与腐蚀公理 | 一切记忆写入收口到单进程单入口，留痕规则只写一处 | 规则随宿主语言走，多语言试点时易分叉 |
| 推荐 | **主推**：存储用 C 级库（SQLite 系，任意语言都能绑），记忆引擎（写入裁决/衰减/合并/检索决策）收进独立服务进程 | 仅当核心语言提前定案且永不换时采用；否则绑定维护成本按语言数翻倍 |

## 二、参照系（只提取事实，不作引入建议）

| 产品 | 存储/检索/嵌入事实（附来源） | 共性坑线索 |
|---|---|---|
| mem0 | 开源库模式默认：向量存本地 Qdrant（/tmp/qdrant）、历史存 SQLite（~/.mem0/history.db）、嵌入与 LLM 默认走 OpenAI 云端模型；自托管服务模式默认 Postgres+pgvector [S31]。检索为语义+BM25+实体匹配多信号融合，带实体链接与时序推理 [S30]。自家评测口径自报（LoCoMo 92.5 等）[S32] | 云 API 默认依赖（违背零现金约束）；记忆操作含增改删，更新链路会腐蚀人工治理（本项目实测事故口径，见 docs/evidence.md） |
| Letta（原 MemGPT） | 记忆块（core blocks 常驻上下文）+ 归档/召回外部存储的分层设计（V1 服务器已归档，现行代码迁至 letta-code，含 harness/服务端/桌面端）[S33]；提供 SQLite/Postgres 后端（历史版本口径，待核验）[S34] | 自我编辑记忆块=自动化更新链路，与"人工标记不可覆盖"冲突；项目形态剧变（V1 归档）是依赖外部框架的活风险 |
| Zep | 论文口径：面向 agent 的记忆层服务，核心是时序知识图谱引擎 Graphiti，动态合成会话与业务数据，在 DMR 等基准自评超越 MemGPT [S35]；商业产品形态 [S36] | 服务化+商业云形态，本地零现金不可用；底层图存储细节（Neo4j/FalkorDB 等）待核验；评测为自家论文口径 |
| Lore（withlore.ai） | 本地优先：会话原文/蒸馏/长期记忆/实体/向量全部落在本地 SQLite，"SQLite 文件即向量库"；策展知识放仓库根 .lore.md（Markdown+Git）[S39] | 同名产品多（agentkitai/lore 等 [S40]），选型引用须辨识；其"全存原文+蒸馏"路线与本项目"宁缺毋滥+全量轨迹独立层"不同 |
| Hindsight（vectorize-io） | 存储为 PostgreSQL 系（Docker 内嵌 .pg0 或外部 PostgreSQL；企业档支持 Oracle）[S37]；概念含 retain/recall/reflect 三操作、observation（证据支持的整合信念）、mental model/knowledge page、memory bank（含按仓库自动构建的项目记忆）[S37]；MIT 许可、有论文 [S37][S38] | 服务化部署（Docker/K8s）+ LLM 云端默认；"mental model 后台改写"属自动化更新链路，同样触腐蚀公理 |

共性坑归纳（跨产品）：①写入污染——全量入记忆导致召回垃圾（与本项目实测"仅约两成有效"一致，见 docs/evidence.md）；②更新腐蚀——自动整合/改写洗掉人工标记；③默认云 API 依赖，本地零现金需自换组件；④评测多为自家口径，量化结论只作方向性参考（概念总纲待核验口径 [S1]）。

## 三、风险与代价（推荐项逐项）

**SQLite + sqlite-vec + FTS5**
1. sqlite-vec 尚处 pre-v1，官方明示会有破坏性变更 [S2]；其 ANN 索引能力与百万级向量性能上限待核验，规模大了可能要换内核。
2. 单写者模型（WAL [S5]）意味着写入必须串行化/批量化，异步观察者风暴要靠写队列削峰，这是自研成本。
3. FTS5 中文分词默认不带，需自备分词（如结巴类轻量库，或存双语料列），分词质量影响 BM25 路召回。

**bge-m3 + ONNX Runtime（记忆服务进程）**
1. bge-m3 属中大型模型，CPU 推理吞吐有限，批量写入时嵌入是记忆写入的主要延迟与电费成本（待实测）。
2. ONNX 转换/量化要自己做一次工程化（模型来源、转换、量化、三平台打包），换模型即重做；跨平台预编译包现状待核验。
3. 独立进程带来 IPC 与生命周期管理成本（启动入口、崩溃重启、版本升级），须并入单一启动链（硬约束 8 [S1]）。

**混合检索 + bge-reranker-v2-m3 + 时间衰减**
1. reranker 是交叉编码器，CPU 上对前 N 条逐对打分，N 与预算要参数化调优，搞不好重排本身成延迟大头。
2. RRF/衰减/置信度的权重是待磨合参数，没有免费最优解，需实测回归数据才能定。
3. 多路检索 + 融合 + 重排的实现全部自研（符合铁律），是记忆栈里代码量最大的一块。

## 四、待拍板清单（决策项：含义 / 影响 / 推荐）

1. **存储层定 SQLite+sqlite-vec+FTS5 还是 LanceDB**。含义：记忆五层的物理承载选关系库路线（SQL 统管结构化+向量+全文）还是列存向量库路线。影响：决定备份/回滚/留痕机制的实现方式、L2 数据模型（关系表/邻接表 vs Lance 表）、以及未来规模上限。推荐：SQLite 路线——与腐蚀公理的事务/追加语义最贴合、单一文件备份回滚简单、四语言绑定全；LanceDB 留作规模超限后的备选。
2. **嵌入模型选型（bge-m3 / EmbeddingGemma / gte-multilingual-base / bge 小模型）**。含义：决定中文语义检索质量与嵌入计算成本。影响：向量维度（1024 vs 768）影响存储与检索耗时；模型体量影响 CPU 吞吐与内存。推荐：bge-m3 主力（中文与长文本），EmbeddingGemma 为低配档；拍板前补一轮中文小样本实测（列入待核验）。
3. **推理运行时（ONNX Runtime / llama.cpp / candle）**。含义：嵌入与重排模型的执行引擎。影响：三平台打包方式、量化路径、与核心语言的解耦程度。推荐：ONNX Runtime（跨平台承诺明确、MIT、C API 语言面最广）；llama.cpp 为 GGUF 量化路线备选；candle 仅在核心语言定为 Rust 时再议。
4. **集成模式（语言无关独立记忆服务 vs 语言绑定同进程库）**。含义：记忆引擎是否随核心语言走。影响：换语言成本、部署形态、崩溃域、写入纪律收口程度。推荐：语言无关独立进程 + SQLite 系 C 库绑定双轨——存储可被任意语言直读（对账/排障友好），写入裁决与检索决策收口在服务进程单入口。
5. **混合检索融合与重排策略**。含义：RRF 还是加权和、是否引入 bge-reranker、时间衰减/置信度的计分公式与预算截断参数。影响：召回质量与延迟、记忆栈自研代码量。推荐：RRF 先行（参数少、无训练），reranker 按 CPU 实测决定开关，衰减与置信度纯查询时计算（不写回）；参数全部进配置（纪律外置）。
6. **L4 共享层的存储与接口形态**。含义：shared.db 是独立库文件还是独立服务实例、接口走进程内 API 还是本地 IPC、跨机器共享是否留扩展口。影响：多执行体协作的痕迹流转与审计边界。推荐：独立库文件 + 记忆服务进程窄接口，先单机；跨机共享留作后续（PostgreSQL 服务路线届时再议），本次不引入服务化数据库。

## 五、待核验清单（12 条）

1. sqlite-vec 当前版本的 ANN 索引能力与百万级向量性能（官方自述"small, fast enough"、pre-v1 [S2]；索引路线图需查官方文档/issue）。
2. bge-m3 稠密向量 1024 维与模型体量（约 0.5B 级）的确切口径——本机无法直连模型卡，经搜索引擎摘要与论文交叉印证，需直读模型卡复核。
3. EmbeddingGemma 的 <200MB RAM 量化口径、2048 token 输入上限与中文效果实测。
4. 嵌入模型中文效果对比（C-MTEB/MTEB 排名随时间变动，须按拍板时点复核）。
5. bge-m3 / reranker 模型的 ONNX 转换件与 int8 量化件的来源与质量（社区转换 vs 自行转换）。
6. ONNX Runtime 各语言绑定清单与三平台预编译包现状 [S25]。
7. llama.cpp 生态语言绑定清单与 embedding API 形态（工具 vs server /embed）。
8. LanceDB 并发写入/事务语义、备份回滚细节、Go 绑定现状 [S7]。
9. Chroma embedded 的存储构成（SQLite+HNSW 索引说）与持久化/并发语义（现引资料为第三方口径 [S11][S12]）。
10. Qdrant local mode 的持久化格式与并发语义 [S13]。
11. Letta 现行代码（letta-code）的存储后端与记忆块机制细节 [S33][S34]；Zep/Graphiti 底层图存储选型 [S35]。
12. "Lore" 品牌辨识（withlore.ai / agentkitai/lore / loremem.com / getlore.tech 同名多项目 [S39][S40]）与 Hindsight 论文口径 [S38]。

## 六、来源清单

- [S1] 概念总纲（本地）：F:\sd_agent\docs\concept-v2.md（组件 4 记忆系统、公理三/四、硬约束 1/2/17）
- [S2] https://github.com/asg017/sqlite-vec （README：vec0 虚拟表、纯 C 无依赖、Linux/macOS/Windows/WASM、Python/Node/Ruby/Go/Rust 绑定、pre-v1 声明）
- [S3] https://alexgarcia.xyz/sqlite-vec/ （安装与文档站）
- [S4] https://sqlite.org/fts5.html （FTS5 虚拟表模块、内置 bm25() 排序函数）
- [S5] https://sqlite.org/wal.html （WAL 模式：读者不阻塞写者、写者不阻塞读者）
- [S6] https://sqlite.org/lang_vacuum.html （VACUUM INTO 作为在线备份手段）
- [S7] https://github.com/lancedb/lancedb （嵌入式、Lance 列存、向量+全文+SQL、自动版本化、Python/TS/Rust SDK）
- [S8] https://docs.lancedb.com/
- [S9] https://github.com/facebookresearch/faiss （相似搜索/聚类库，C++/Python，GPU 可选，Meta FAIR）
- [S10] https://github.com/nmslib/hnswlib （头文件 C++ HNSW 实现，Python 绑定，支持增改）
- [S11] https://github.com/chroma-core/chroma （Python/JS 客户端、进程内嵌入式用法、Apache 2.0）
- [S12] https://docs.trychroma.com/
- [S13] https://github.com/qdrant/qdrant-client （local mode：同 API 免服务器，:memory: 或本地路径）
- [S14] https://qdrant.tech/documentation/
- [S15] https://github.com/pgvector/pgvector （Postgres 向量扩展：精确+近似检索、多向量类型、支持 Postgres 13+）
- [S16] https://huggingface.co/BAAI/bge-m3 （多语言/多功能/多粒度）
- [S17] https://arxiv.org/abs/2402.03216 （M3-Embedding 论文：最长 8192 token 输入）
- [S18] https://github.com/FlagOpen/FlagEmbedding （bge 系列与 bge-reranker-v2-m3 等重排模型清单）
- [S19] https://ai.google.dev/gemma/docs/embeddinggemma
- [S20] https://deepmind.google/models/gemma/embeddinggemma/ （308M、量化后 <200MB RAM）
- [S21] https://huggingface.co/google/embeddinggemma-300m （768 维、2048 token、MRL 降维）
- [S22] https://huggingface.co/Alibaba-NLP/gte-multilingual-base
- [S23] https://mteb-leaderboard.hf.space/ （305M 参数、768 维、8192 token 条目）
- [S24] https://huggingface.co/nomic-ai/nomic-embed-text-v1.5 （MRL 64–768 维、8192 token、英文为主）
- [S25] https://github.com/microsoft/onnxruntime （跨平台推理、MIT）
- [S26] https://onnxruntime.ai/docs/build/inferencing.html （CPU：Windows/Linux/macOS 构建支持）
- [S27] https://github.com/huggingface/candle （Rust 极简 ML 框架、GPU/量化支持、MIT/Apache）
- [S28] https://github.com/ggml-org/llama.cpp
- [S29] https://github.com/ggml-org/llama.cpp/tree/master/examples/embedding （llama-embedding 工具、Unix/Windows 命令、pooling/归一化参数）
- [S30] https://github.com/mem0ai/mem0 （多信号检索：语义+BM25+实体、实体链接、时序推理）
- [S31] https://docs.mem0.ai/open-source/quickstart （默认组件表：本地 Qdrant + SQLite 历史库 + OpenAI 云嵌入；服务模式 Postgres+pgvector）
- [S32] https://mem0.ai/research （自家评测口径）
- [S33] https://github.com/letta-ai/letta （V1 归档、现行 letta-code）
- [S34] https://docs.letta.com/
- [S35] https://arxiv.org/abs/2501.13956 （Zep 论文：时序知识图谱引擎 Graphiti、DMR 自评对比）
- [S36] https://www.getzep.com/
- [S37] https://github.com/vectorize-io/hindsight （retain/recall/reflect、observation、mental model/knowledge page、memory bank、PostgreSQL 系存储、MIT）
- [S38] https://arxiv.org/abs/2512.12818 （Hindsight 论文，口径待核验）
- [S39] https://withlore.ai/ （Lore：本地 SQLite 即向量库、Markdown+Git 策展知识）
- [S40] https://github.com/agentkitai/lore （同名项目的辨识材料）

