# SD-agent

在深度使用了多款国内外/开闭源agent后，加上大大小小的模型都体验了以后，我发现每一个agent都有各自的优点，但是想把自己想要的功能整合起来却是难上加难，并且我有些理念让AI自己搓出来以后也因为agent的协议或者其他原因很难达到我想要的效果，提交PR也不一定有反馈，在目前这个agent百花齐放的时候，前脚刚想出来的想法后脚就有人做了出来，我认为这是一个风口并且我想的每一个想法都是比较符合实际的，所以我决定自己手搓一款适合自己的agent，顺便开一个公开仓库练练手

一个中国高中生在AI的帮助下制作的一款AGENT，结合我自己使用agent的实际问题进行开发，并且把agent与“自研”记忆系统深度结合，争取在工作效率与性价比中找到平衡点

---

（以上为作者原话，禁止 AI 改写、润色或删减。以下为项目文档区。）

## 项目是什么

面向重度工程开发者的自研 Agent 运行时底座（Harness）：纯自研、零重型第三方框架、零现金成本约束。
不提高模型智力上限，只守住模型能力下限——用工程纪律和验证闭环，兜住模型的幻觉、遗忘、散漫与失控。

当前阶段：概念设计期（只产出理论规划，不写实现代码）。

## 文档导航

| 文档 | 内容 | 可见性 |
|---|---|---|
| [docs/concept-v2.md](docs/concept-v2.md) | 底层概念体系：四条公理、五层工程地基、十二类架构组件（含认知纪律域、缓存与上下文工程、生命周期钩子）、靶子清单、硬约束、待磨合点 | 可外发（无工具指向性） |
| [docs/evidence.md](docs/evidence.md) | 实证对账：概念与真实运行证据的逐条对照（已脱敏） | 公开 |
| [docs/iterations.md](docs/iterations.md) | 迭代记录：每轮文档迭代的变更摘要、待核验判断点与素材映射（滚动更新，只留这一份） | 公开 |
| [docs/architecture.html](docs/architecture.html) | 架构总览图：分层可视化（浏览器打开，单文件无外部依赖） | 公开 |
| [AGENTS.md](AGENTS.md) | 协作纪律：任何 AI Agent 进场前必读 | 可外发 |

仓库结构约定：根目录只放入口与法务文本（README / LICENSE / AGENTS.md / .gitignore）；工程文档进 docs/；原始素材（对话导出等含交互记录的材料）本地保存于 docs/materials/ 但**不入库**（已 gitignore）；代码（src/）与测试（tests/）开工时再建。文档组织按工程文档类型划分，不采用个人笔记库的 PARA 分类法。

## 四条公理（速览）

1. 下限公理：模型是高智力低纪律的执行体，纪律靠工程不靠自觉。
2. 纪律外置公理：规则的制定权与裁决权用配置和代码表达，不与模型博弈；但对模型的单向注入干预是允许且必需的手段。
3. 腐蚀公理：一切自动化更新都会腐蚀人工治理成果，必须留痕、限次、可回滚。
4. 上下文不可靠公理：上下文随时会被压缩/污染/丢失，唯一副本绝不放在上下文里。

## 协作方式

本项目由多个 AI Agent 协作推进，纪律见 AGENTS.md。核心三条：
- 禁虚构：设计依据必须可核验，核验不到标注"待核验"或不写。
- 去工具指向性：概念与架构表述只用通用设计模式，禁止绑定具体第三方项目 API。
- 变更走 git：小步提交、提交说明写清变更内容、文档只留最新版。

---

## 开源许可 / License

本项目采用 **GNU AGPL v3.0** 许可证（GitHub 官方标准文本，见 [LICENSE](LICENSE)）。
This project is licensed under the **GNU AGPL v3.0** (standard text as provided by GitHub, see [LICENSE](LICENSE)).

- 学习、自用、非商业用途：自由使用、修改、研究，无附加义务。
  Free to use, modify and study for learning, personal and non-commercial purposes.
- 二次开发：修改后分发或对外提供网络服务，必须以 AGPL-3.0 开源你的衍生作品，并注明原作者 Sam-Dancing。
  Derivative works distributed or offered over a network must be open-sourced under AGPL-3.0 and credit the original author Sam-Dancing.
- 商业使用：如需闭源商用或商业授权（双许可模式），请联系作者。
  For closed-source commercial use or commercial licensing (dual licensing), contact the author.

Copyright (C) 2024-2026 Sam-Dancing. 自研设计，非任何现有框架的衍生。
本文档含 AI 协作生成内容 / This document contains AI-generated content.
