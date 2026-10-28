# PLAN — TalkDND Design Goals

> 本文件是 TalkDND 的**设计目标权威**：为什么做、做到什么程度、原则与边界。
> 项目章程与决策记录在 `docs/PROJECT-CHARTER.md`；任务顺序与状态只在 GitHub Issues。
> 本文件不承载任务清单，也不与 Issue 双写。

## 设计目标

- 让玩家在聊天里跑完一场 D&D：AI DM 负责叙述、NPC、选项与自由输入，玩家扮演自己的角色，存档可跨会话继续。
- 所有骰子都由 `combat-sim` 的同一套 Rust 引擎产生并完整显示；战斗（攻击、伤害、条件、专注、移动、借机攻击）由该引擎结算，不靠模型口算。
- 战斗核心动作（擒抱、推撞、徒手打击、双武器战斗、额外攻击）用同一套动作/附赠动作经济、条件与强制移动内核结算；2014 与 2024 的对抗方式、徒手伤害与额外攻击数据由 `rules.combat_actions` 和生物/被动特性数据决定，不按职业或生物名硬编码。
- 空间规则只走一个内核：触及按目标体积、视线与掩护按同一组障碍体积、范围效果与目标体积求交、移动逐步校验；飞行与坠落是显式引擎命令，旧存档未启用空间字段时保持旧结果。
- 支持 5.5e 与 5e 两套规则，玩家可选 **5.5e FULL / 5e FULL / 5.5e SRD / 5e SRD**，一档一版；SRD 只用该 edition 的 SRD 层，FULL 在 SRD 层之后加载已解锁的完整书内容包。
- 把玩家自备的规则书、冒险书、个人存档和 DM 秘密留在本机；仓库只收程序、原创文档、SRD 5.1 / 5.2.1 内容包和 SRD 派生的战斗数据。
- 用 `tools/talkdnd` 把开局、建卡、同伴、存档点、信息边界和完整性检查做成可复用的 DM 助手，而不是把规则散落在提示词里。
- 为 DM Agent 提供只读知识 API：稳定 id、固定来源类别、默认玩家已知过滤，把 SRD、玩家书索引、冒险、存档、原创与桌上裁定放在同一组 `search/get/cite/context/related` 后面；秘密过滤与 `guard` 同源且只读，不新增持久化索引或秘密缓存、不改存档 schema、不联网。
- 自动化开关按存档保存：固定类别 + `manual / assisted / auto` 级别，存档默认可被场景/遭遇按 id 覆盖，有效顺序为遭遇 > 场景 > 存档默认；玩家选择与反应永不自动，接近上限先警告。
- 把产品决策写进 `docs/PROJECT-CHARTER.md` 的 Decision Log，后续任务按同一份契约推进，不因会话更替而漂移。
- 在 per-save 的偏离范围内允许 DM 原创内容，并用 DM-only 台账控制类型、稀有度和章节预算。

## 设计原则

- **Rust 优先，例外封闭。** 项目程序、工具与脚本用 Rust；只有 AgentOrch Python 套件、CI YAML 和 `LetsPlay/tools/pdf2md/` 是例外，且不得扩大。
- **一本书一个文件夹。** 规则书与冒险按 `<slug>/` 存放，pdf2md 输出在 `index/`；不把书页图片或正文提交进仓库。
- **单一事实来源。** `docs/PROJECT.md` 决定底座；`docs/PLAN.md` 决定设计目标；GitHub Issues 决定任务状态；`docs/PROJECT-CHARTER.md` 决定产品决策。
- **失败关闭。** 底座、存档 schema、规则版本、完整性清单不合法时停下报告，不猜测、不静默降级。
- **自动化是设置，不是规则。** 自动化开关只决定 DM 何时自己结算、何时先问、何时等待玩家；它不改变战斗规则、骰子或 AI 默认，也不能替玩家做选择或花反应。
- **真实证据。** 只有真实执行过的命令、输出、退出码、Diff、CI 记录与 head SHA 才算证据。
- **可恢复。** 存档有检查点与结构化 diff；套件升级有基线、漂移报告和人工决策门。
- **不双写。** 不建第二套任务库、不建第二份计划状态、不把 `local` 任务库当成 `github` 载体的替代。

## 范围与非目标

**范围：** 聊天跑团体验、`combat-sim` 战斗引擎、5e/5.5e 规则数据、`talkdnd` DM 助手、`talkdnd-dev` 开发工具、`LetsPlay/` 自包含游玩目录、AgentOrch `github` 底座协作流程。

**非目标：**

- 不发行受版权保护的冒险书或印刷版规则书；SRD 5.1 / 5.2.1 内容包按 CC-BY-4.0 随程序分发，玩家不必自备 SRD PDF；PHB/DMG/MM 完整书内容包按所有者 2026-10-27 的分发授权随程序提供，但保持锁定直到玩家用匹配的 PDF 解锁，玩家自备的 PDF 只在本机使用。
- 不提交存档、角色卡、会话日志或 DM 秘密；`LetsPlay/Saves/README.md` 是唯一例外。
- 不在同一存档里混用 5e 与 5.5e 规则。
- 不把非 SRD 的怪物、法术或战役硬编码进引擎或已跟踪数据。
- 不引入网络服务、账号系统或托管游戏。
- 不维护第二套任务系统；`editions/local/` 只因为套件校验要求两套底座文件而留在树里。

## 全局约束

- 平台：本地 CLI；Linux、Windows、macOS 是玩家平台，发布目标为 Linux x86_64、Windows x86_64、macOS arm64。
- 隐私：书籍、存档、DM 秘密、凭据永不进入 Issue / PR / commit / CI 日志；图片入库前脱敏。
- 规则数据：只收 SRD 5.1 / 5.2.1，署名见 `NOTICE`；非 SRD 数据只留本地且被 `.gitignore` 覆盖。完整书内容包只作发布资产：从本机 pdf2md 索引构建，永不入 Git，CI 只用合成 fixture、不构建或下载。
- 完整书解锁：玩家提供 PDF 只为按内容识别并解锁对应 full pack；不校验 hash；授权范围变化时撤下或替换 full pack。`*-srd` 永不加载 full pack；`*-full` 至少需要一本同 edition 的已解锁 full pack，缺失、未解锁或损坏时 `start` 与 `content check` 失败关闭，不静默退回 SRD。
- 版本：三个 crate 版本一致；存档 schema 变更必须带自动迁移并升次版本或主版本；发布前 `CHANGELOG.md` 必须有对应小节。
- 源码只读：`tools/combat-sim/src` 默认不可写；修改前 `chmod u+w`，验证后 `chmod a-w`。
- 套件：AgentOrch Format 1.6，官方 `MANIFEST.json` 基线；本地差异与预期漂移记录在 `docs/PROJECT.md`。

## 可度量的全局验收

- 玩家在 `LetsPlay/` 启动 Agent，用 `LetsPlay/bin/combat-sim` 完成一次真实掷骰与一场战斗结算；SRD 内容已随程序提供，放入自己的 SRD PDF 是可选的。
- 存档的 `ruleset` 是四个值之一：`*-srd` 只加载对应 edition 的 SRD 层，`*-full` 要求同 edition 至少一本已解锁完整书，另一版目录不被读取；旧值 `2014` / `2024` 可读并在下次写入时迁移。
- `uv run --no-project python scripts/validate_kit.py --project-ready`、`check-leaks`、`check-letsplay` 在无书籍、无存档的干净树上通过。
- 三个 crate 的 `cargo test`、`cargo fmt --check`、`cargo clippy -D warnings` 通过。
- 战斗核心动作在 2014/2024 下都按 `rules.combat_actions` 结算；擒抱/推撞失败与非法防御不消耗错误资源，推撞走强制移动并记录阻挡，额外攻击只由数据决定；擒抱/推撞消耗攻击动作的一次攻击，额外攻击生物可继续剩余攻击；`attack --one`/`--unarmed --one` 只结算一次并保留剩余攻击；旧 combat state 仍以 schema 1 加载。
- `talkdnd knowledge` 的五个只读命令对同一输入给出稳定 id 与确定性排序；默认不泄露 `dm-secrets.md`、未揭示原创隐藏面、DM-only 冒险内容或未识别名字；`--include-hidden` 的结果标 `visibility: dm`。
- AgentOrch 完整套件 `unittest discover -s tests` 通过；`check_drift` 只报告 `docs/PROJECT.md` 中列出的预期漂移。
- 发布自动化产出章程列出的三个平台制品；发布包只含已跟踪的 `LetsPlay/` 内容与二进制。完整书 pack 作为独立发布资产由本地构建产生，仓库、LetsPlay 包与 CI 日志都不含书籍正文或扫描页。
- 改变范围、目标、技术栈、规则支持或用户决定的 PR，在同一 PR 内更新 `docs/PROJECT-CHARTER.md`。

## 计划文件

- **本文件**（`docs/PLAN.md`）= 设计目标权威。
- **GitHub Issues** = 任务顺序、依赖、状态与证据的唯一载体。
- **`docs/PROJECT-CHARTER.md`** = 产品决策、范围与活章程。
- 本仓库不建立 `docs/implementation-plan.md` 或其它任务台账；`templates/implementation-plan.md` 只是套件模板。未来若需要阶段计划，由 Master 明确决定，并保持 Issue 为状态权威，不双写。
