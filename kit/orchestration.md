# orchestration.md — 会话入口 · 状态机与合并门 · 通用验收程序

> 本文件由 1.4 及之前的 `kit/orchestration.md`、`kit/orchestration.md`、`kit/orchestration.md` 三份合并而来（1.5 起唯一权威）。

# 第一部分：会话入口（原 kit/orchestration.md）

# kit/orchestration.md — 会话自动引导入口

本文件定义每次 Agent 会话开始的确定性入口。无论会话由谁发起、上下文是否丢失，都要按本流程恢复状态并自动路由，不依赖聊天记忆或人工提醒。

## 1. 引导流程

### 第一步：读取权威文件与底座

按顺序读取：

1. `docs/PROJECT.md` — 读取 `edition`，取值必须是 `local` 或 `github`。
2. `AGENTS.md` — 核心协作契约（edition-neutral），含「阅读口径（唯一权威）」段。
3. `kit/orchestration.md` 第一部分（会话入口）与第二部分（状态机、合并门与能力契约）——**必读**；第三部分（通用验收程序）按需读，见下。
4. `editions/<edition>/master.md` — 当前底座的 Master 手册。

**阅读口径（只回指，不另立一套）**：必读集合与「第三部分按需」规则以 [AGENTS.md](../AGENTS.md) 的「阅读口径（唯一权威）」段为唯一权威。落到本文件：第一、二部分属必读集合；第三部分**按需读**，触发条件与本文件第三部分开头一致（每次首次接入、变更调度器 / 提示词，或更换 Agent 运行时后执行），本节不新增触发项、不缩小既有触发项；触发条件未满足时，缺少该部分的逐段引用不构成验收缺失。

**底座判定规则**：

- 从 `docs/PROJECT.md` 的 `edition` 键读取取值，允许值只有 `local`、`github`；该键是底座选择的唯一权威来源，`.orchestration/config.json` 只描述本地 CLI 运行参数，即使残留 `edition` 键也不参与路由。
- 文件缺失、`edition` 键缺失、值为空或不在允许集合内（含类型错误，如 `1`、`true`）：输出明确错误并**停止**，不得猜测、不得按默认值继续、不得自行切换底座。错误信息必须包含缺失项、实际读到的值和建议修复位置。
- 读取成功后，后续所有路由、命令和证据路径都以该底座为准。
- 本地 CLI 只服务 `local` 底座：声明 `github` 时，`task add` / `task ready` / `task dispatch` / `task status` / `task workspace` / `task amend` / `task unlock` 会拒绝执行并给出 Issue / PR 指引；`init`、`backup`、`config show`、`task list`、`task show` 仍可用。完整矩阵见 `README.md` 6.2。

### 第二步：恢复状态

用真实输出恢复状态，不从聊天记忆重建。先执行两个底座共用的 Git 检查：

```bash
git status --short
git branch --show-current
git log -1 --oneline
```

再按底座执行任务载体查询：

`local`：

```bash
python scripts/orchestrator.py task list
```

`github`：

```bash
gh issue list --state open --limit 100
gh pr list --state open --limit 100
```

必要时可补充 `git worktree list`、任务库事件查询、Issue 详情 / PR 时间线与最近提交记录。

**计划权威（两层）**：恢复“方向对不对”时读 `docs/PLAN.md`——它是**设计目标权威**（为什么做、做到什么程度、原则与边界），任务方向不得与它冲突；若项目根目录仍有 1.3 之前的根 `PLAN.md`，先提示所有者 `git mv PLAN.md docs/PLAN.md` 迁移，迁移前以根 `PLAN.md` 为准、不得两处并用。恢复“计划到哪一步、下一步做什么”时读 `docs/` 里的阶段计划（项目需要时从 `templates/implementation-plan.md` 复制，`init` 不创建它）；阶段计划承载阶段目标、任务顺序、依赖与状态。`templates/plan.md` 只用于初始化复制，不得当作项目计划读取。两者冲突时，以 `docs/PLAN.md` 的设计目标为准，停下对齐并说明差异，不得各写一套。`docs/PLAN.md` 缺失时，提示用户运行 `python scripts/orchestrator.py init` 生成副本，不得凭聊天记忆补计划；阶段计划缺失时按项目的实际任务进展汇报，不要凭空补一份。

### 第三步：按状态自动路由

根据第一步与第二步的事实，按顺序匹配下列分支，命中即加载对应提示词并继续；都不命中时落入 d。`<edition>` 指第一步读到的底座。

| # | 条件 | 动作 |
|---|------|------|
| a | `docs/PROJECT.md` 存在 `FILL_IN` 占位符，或章程缺失 | 加载 `templates/project-charter.md`（两个底座共享），先补齐章程再进入编排 |
| b | 章程已填写完整，且当前底座没有任何任务（新项目） | 加载 `editions/<edition>/master.md` 的“新项目启动”章节，进入初始规划 |
| c | 当前底座已有活动任务（`DISPATCHED` / `IN_PROGRESS` / `PR_OPEN` 等），或项目已有推进记录 | 加载 `editions/<edition>/master.md` 的“已有项目接入”章节，先汇报当前状态、活动任务与阻塞，再继续调度 |
| d | 当前底座没有任务，且用户本轮未给出目标 | 简要汇报当前状态（底座、分支、工作区、最近提交、章程情况）后等待用户目标，不自行造任务 |

判断以底座载体与 Git 的真实状态为准；多种条件同时命中时，按 a → b → c 优先级处理（例如章程缺失时先补章程，即使已有任务）。

共享提示词固定为 `templates/project-charter.md` 与 `templates/review-checklist.md`；其余启动与 Master 提示词一律从 `editions/<edition>/` 读取，不要跨底座加载。

### 自我完善的触发（只路由，不定义流程）

用户以明确执行意图说出“进行自我完善”或其同义表述（例如“根据其他 AgentOrch 项目的经验改进本项目”“总结近期开发教训并推进改进”）时，先按本部分第一步、第二步恢复状态，再加载 `docs/self-improvement/self-improvement-loop.md`（仓库检出中读取；发行包内读取 `templates/self-improvement/README.md` 的操作说明）并按其闭环执行；触发语与非触发的完整边界见 `docs/self-improvement/entrypoints-and-decisions.md` §1（仓库检出中读取）。

- 普通提问（如“自我完善机制是什么”“更新一下文档”）只回答，不启动闭环；`更新 AgentOrch` 属于更新引擎，不进入该闭环。
- 本节只做路由引用：任务状态机、路径 / 资源锁、证据矩阵与合并门仍只由第二部分与 4.1 节定义，自我完善**不新增、不改写**任何状态、锁或门。
- 触发不构成对任何具体改动的批准；权限 / 安全 / 锁 / 状态机 / 迁移 / 外部依赖 / 隐私 / 兼容 / 不可逆事项仍按第 5 节与 `docs/self-improvement/entrypoints-and-decisions.md` §5 的 `HD-*` 进入 `NEEDS_HUMAN_DECISION`。
- 载体操作由 `editions/<edition>/` 的提示词负责：`local` 只写任务库，`github` 只写 Issue / PR，不双写。

### 更新 AgentOrch 的触发（只路由，不定义流程）

用户以明确执行意图说出“更新 AgentOrch”“升级 AgentOrch”或“把项目中的 AgentOrch 更新到最新版”时，先按本部分第一步、第二步恢复状态（确认 edition、当前版本 / Format、工作区状态与已安装基线），再运行 `scripts/update_agentorch/entry.py`（`python -m scripts.update_agentorch.entry --project <项目根> --request "<用户原话>"`），由它调用 #51 安全更新引擎。

- **非触发**：“更新一下文档”“更新依赖”“更新这个 Issue”以及只询问版本号等普通请求不启动套件更新。措辞有歧义时先问一次“是更新套件本身还是更新项目内容”，不得默认执行。
- 入口只做路由、环境识别与载体适配：不重写 #51 的来源校验、逐文件分类、备份 / 回滚语义；默认 `PLAN`，`APPLY` 只在隔离分支 / Worktree 中准备变更，不提交、不合并。
- 路由规则复用 `scripts/self_improvement/planning.py` 的 `route_request()`；`更新 AgentOrch` 不进入自我完善闭环，自我完善触发也不进入本入口。
- `local` 只准备任务包 / 分支交付，任务库写入仍由 `scripts/orchestrator.py` 完成；`github` 只准备 Issue / Draft PR，入口不调用 `gh`。两者都不双写，也不假装本地底座支持 GitHub 自动化。
- 更新安全语义的唯一权威是 `docs/self-improvement/update-safety.md`（仓库检出中读取）；发行包内以 `templates/self-improvement/README.md` 与引擎报告为准。冲突、上游删除 / 重命名、无基线、未提交改动默认停止并交人工决策。

## 2. 引导纪律

- 引导产出的是“状态与路由”汇报，它不替代 Master 的任务汇报模板（结论、用户可感知变化、验收结果、风险与下一步）。
- 不得用聊天记忆代替任务载体状态；所有“有任务 / 有阻塞 / 有活动锁”的结论都必须有命令输出或文件证据支撑。
- 引导本身不改产品代码、不改编排状态；发现的冲突、缺失或异常先登记为任务或标记 `NEEDS_HUMAN_DECISION`，再进入对应提示词流程。
- 底座不一致（例如项目声明 `github` 但只有本地任务库）属于阻断问题：停止派发，报告实际观察与所需决定，不得自行迁移或双写。


# 第二部分：状态机与合并门（原 kit/orchestration.md）

# kit/orchestration.md — 核心编排细则（Edition-Neutral）

本文件定义两个底座共用的编排契约：状态机、任务锁、合并门、运行时能力探测与切换规则。任务载体和证据来源由 `docs/PROJECT.md` 的 `edition` 决定：`local` 使用 SQLite 任务库与原子路径锁，`github` 使用 Issue 任务单、Draft PR 与 CI。若某项能力依赖运行时支持而当前环境并不具备，必须降级并记录依据，不得假装执行。

## 1. 适用方式

- 核心条款只有这一份。底座手册（`editions/<edition>/master.md` 与对应提示词）只能细化命令和载体操作，**不得复制或改写出另一套核心条款**；发现底座手册与本文件冲突时，以本文件为准并登记任务修正。
- 状态、门禁、审批与合并的语义在两个底座完全一致，差异只体现在“写到哪里、用什么命令查”。
- 任何一项能力或检查未实际执行时，状态不得前进；“未知”一律按未完成处理。

## 2. 任务状态机

```text
PLANNED → READY → DISPATCHED → IN_PROGRESS → [PR_OPEN] → REVIEW_READY → MASTER_REVIEW → APPROVED → MERGED → VERIFIED
非成功状态：BLOCKED / NEEDS_HUMAN_DECISION / FAILED / CANCELED
```

- `PLANNED`：任务草稿已登记，但依赖或验收可能尚未齐备。
- `READY`：目标、路径、验收和依赖均明确；所有依赖任务为 `VERIFIED`。
- `DISPATCHED`：已取得活动任务锁（`local`：完整路径锁；`github`：Issue 登记块 + 开放 PR 冲突检查），任务包已交给真实执行实例。
- `IN_PROGRESS`：执行实例已开始；应记录分支 / Worktree。
- **`PR_OPEN`**：Draft PR 已创建并在 Issue 中登记。**仅 github 底座使用**；`local` 底座由 `IN_PROGRESS` 直接进入 `REVIEW_READY`，不存在该状态。
- `REVIEW_READY`：实现者交回 Diff、head SHA、验证命令 / 输出和剩余风险；`github` 底座要求 PR 已转为 Ready for review 且 CI 在当前 head 全绿。
- `MASTER_REVIEW`：Master 独立检查任务、变更范围、证据和风险。
- `APPROVED`：验收全部通过；批准必须绑定精确 head SHA（`github` 为 PR 最新 commit）。
- `MERGED`：已确认合并到主分支。远端或本地分支清理状态应单独记录，不可混为合并成功的判据。
- `VERIFIED`：在合并后的主分支上完成必要验证，记录命令 / 退出码 / 结果。完成后释放活动任务锁。
- `BLOCKED`：外部依赖、文件冲突或环境问题暂时阻止推进；记录恢复条件。
- `NEEDS_HUMAN_DECISION`：需项目所有者选择或授权；必须记录具体问题与选项。
- `FAILED`：执行实例失败、超时、取消或产物不可用；**状态未知一律视为未完成**。
- `CANCELED`：项目所有者或 Master 明确取消；记录原因后释放锁。

### 2.1 两底座状态对照

| 状态 | `local` 载体 | `github` 载体 |
|------|--------------|---------------|
| `PLANNED` / `READY` | 任务库记录 + 依赖检查 | Issue（正文五节 + label） |
| `DISPATCHED` | `task dispatch` 原子取得全部路径锁 | Issue 内 `ORCH-TASK-REGISTRY` 块 + 开放 PR 同文件检查 |
| `IN_PROGRESS` | 分支 / Worktree 记录 | 分支已推送；Draft PR 尚未创建 |
| `PR_OPEN` | 不使用 | Draft PR 已创建并回链 Issue |
| `REVIEW_READY` | 交回 head SHA 与本地验证输出 | PR 转 Ready；CI 在当前 head 全绿 |
| `MASTER_REVIEW` | Master 查看 Diff 与日志 | Master 查看 PR Diff、CI 与评论 |
| `APPROVED` | 绑定 head SHA | 绑定 PR 最新 commit SHA |
| `MERGED` | `git merge --no-ff`（按项目配置） | 平台合并（策略按项目配置） |
| `VERIFIED` | 主分支重跑检查 | 主分支重跑检查（不只看 CI） |

状态改变由底座载体记录：`local` 通过 `scripts/orchestrator.py` 写入任务库，不要手工编辑 SQLite；`github` 通过 Issue / PR 状态转换与评论记录。载体提供的是治理记账，不会代替运行时真正派发或运行测试。

## 3. 派发、活动任务锁与依赖

### 3.1 取得活动任务锁

`local`：

1. 任务先进入 `READY`，然后运行 `python scripts/orchestrator.py task dispatch <ID>`。CLI 使用 SQLite `BEGIN IMMEDIATE` 原子检查锁并登记 `DISPATCHED`。
2. 锁范围由任务允许路径确定；比较采用路径相同或父子路径重叠的方式。例如 `src/` 与 `src/core.py` 冲突，`src/a.py` 与 `src/b.py` 不冲突。
3. 只有当全部路径可锁定时才一次性取得全部锁；冲突则整个派发失败，不允许半锁状态。
4. 任务处于活动状态时，不得修改锁定路径集合。确需扩大范围时，先阻塞任务、记录理由、检查新路径冲突并更新登记。
5. 仅进入 `VERIFIED` 或 `CANCELED` 自动释放锁。失败、超时、未知状态、审查退回或单纯删除 Worktree 都不自动释放。异常清理需运行显式 `task unlock` 并记录理由；强制解锁可能允许冲突，应先确认原任务已停止。

`github`：

1. 任务 `READY` 后，在 Issue 内写入 `ORCH-TASK-REGISTRY` 登记块（字段见 `editions/github/active-task-registry.md`），声明 `status`、`mode`、`locked_paths`、`branch`、`pr`、`runner`、时间戳与 `head_sha`。
2. 写入登记块前必须列出所有开放 PR 及其变更文件，检查是否与本次 `locked_paths` 相交；相交则整个派发失败，不得“先开工再协调”。
3. 登记块写入成功且无路径冲突，才可把任务包交给执行实例。
4. 活动期间不得悄悄扩大 `locked_paths`；确需扩大时先更新登记块并在 Issue 评论说明，再重新做冲突检查。
5. 只有任务进入 `VERIFIED` 或 `CANCELED` 才把登记块标记为释放；`FAILED` 与状态未知保持锁定并写 `blocked_reason`。
6. 每次启动或恢复 Master 时，先读取 Issue 登记块与开放 PR 列表，再核对 `git status`、主分支与分支 head；不要从聊天记忆重建。

### 3.2 依赖与并行

- 依赖应构成有向无环图。依赖必须达到 `VERIFIED`，不能仅因为代码已提交、已合并或 PR 已批准就派发下游任务。
- 同一文件或目录前缀冲突的任务串行；不冲突且无依赖的任务可以并行。
- 并行度按机器余量决定，不写固定数字。派发前观察 CPU / 内存 / 磁盘 / 测试资源争用，有余量即可并行；同一 Worktree 同一时刻只允许一个测试进程。
- 派发前估一次这次任务包 + 必读集合的量级（文件数、章节范围），把估计写进任务包 `Context` 或证据 `notes`，让接手者知道要读多少；这只是记录口径，不设数值门槛、不新增字段、不新增指标。
- 不要让两个实例编辑同一个 Worktree；`github` 底座也不允许两个 PR 同时改同一锁定路径。
- 如果运行时不能真实 spawn / wait / collect，Master 产出任务包，由同一位操作者在另一个会话启动执行实例；Master 不声称它已启动。能力判定见第 5 节。

### 3.3 具名资源锁（契约见 `docs/design/resource-locks.md`）

3.1 的路径锁只判定“是否改同一批文件”，无法表达测试数据库、共享构建环境等具名资源冲突。具名资源锁的语义（数据模型、冲突矩阵、原子集合获取、租约与恢复、底座能力边界、故障场景）**只在 `docs/design/resource-locks.md` 定义**，本节不重述、不改写；本文件的状态机、门禁与锁取得条件（3.1）保持不变。

- 资源锁是路径锁之外的**增量**判定：两者都通过才可派发；未声明资源的任务行为不变。
- `local` 可声明“同一数据库文件范围内的原子互斥”；`github` 只能登记协作性占用并做冲突检查，**不得宣称原子锁**（依据见契约第 7 节）。
- 只有 `VERIFIED` / `CANCELED` 自动释放；`FAILED`、`BLOCKED` 与状态未知保持占用，过期不等于释放。
- 契约改动必须另开 Issue 重新冻结，实现任务不得在实现 PR 中顺手修改语义。

操作入口（判定语义只在契约里，本节只列命令与配置键）：

| 目的 | 命令 | 前置条件 |
|------|------|----------|
| 声明资源集合 | `task add ... --resources <id:mode>[:<ttl>] ...`（可选） | 资源 ID 已按契约 §2.1 规范化；不声明即不参与 |
| 派发并取得整批锁 | `task dispatch <ID> --resource <id:mode>[:<ttl>] --holder <ID> --evidence <ref>` | 任务 `READY`；`.orchestration/config.json` 显式配置租约 |
| 查看锁与阻塞 | `task resources [--resource <id>] [--all]`、`task list`、`task show <ID>` | 只读；`github` 底座只列出已存在记录 |
| 续租自己的锁 | `task renew <ID> --holder <ID> [--reason ...]` | 只有持有者可续租；已强制解锁的任务得到 `LOCK_NOT_HELD` |
| 回收过期锁 | `task reclaim <ID> --resource <id> --reason ... --evidence ...` | 已 `EXPIRED`、冷却期已过、附终局证据 |
| 单资源释放 / 强制解锁 | `task unlock <ID> --resource <id> --reason ... [--force --authorized-by <ID>]` | 强制解锁必须有理由；非终局任务还需授权人 |
| 跨会话重建事实 | `task reconcile [--reason ...] [--evidence ...]` | 只读锁表与审计事件；缺证据者进入隔离 |

配置键（`.orchestration/config.json`，无内置默认、只在声明了资源的任务上读取）：`resource_lease_ttl`、`resource_reclaim_grace`。缺失或非法时派发被拒绝，并指名要修的键。

## 4. 合并门 G1–G6

`MASTER_REVIEW` 必须逐门检查。任一门的结论不是“通过”，就退回 `IN_PROGRESS` 或 `BLOCKED`，写明具体缺口；不得用其他门的通过来抵消。

| 门 | 要求 | `local` 证据 | `github` 证据 | 原审查检查项 |
|----|------|--------------|---------------|--------------|
| **G1 验收逐条通过** | 每条验收标准都有对应证据，部分通过不算通过 | 任务记录 + 命令输出 | Issue 验收清单 + PR 评论逐条结论 | 1 |
| **G2 范围合规** | 实际变更路径全部属于允许路径，无未授权文件，无夹带重构 | `git diff --name-only` + `git status --short` | PR changed files + 任务允许路径 | 2、3 |
| **G3 基线最新且可合并** | 分支基于最新主分支；无冲突；合并预演可完成 | 主分支 SHA + 本地 merge 预演结果 | PR 为 `MERGEABLE` 且 base 为最新主分支 | 3 |
| **G4 必需检查全绿** | 配置要求的检查在**当前 head SHA** 上全部通过 | 配置的测试 / lint / typecheck / build 退出码为 0 | CI 工作流在当前 head SHA 全绿 | 4 |
| **G5 证据真实可复现** | 命令、输出、退出码与 head SHA 一致；失败测试未被删除、跳过或弱化 | 本地日志 + head SHA | CI 日志 + PR 评论 + head SHA | 5、7 |
| **G6 风险已处置** | 新依赖、迁移、权限 / 安全变化、未实现行为与回退方案已登记或升级为 `NEEDS_HUMAN_DECISION` | 任务记录的风险节 | PR 风险节 + Issue 决策记录 | 6 |

门的执行细则：

- **G1**：Master 不采信“已完成”的自述，也不把 CI 绿灯当作验收通过；逐条对照验收标准给出依据。逐条依据的记账格式见 4.1 节：验收标准未映射到任何 evidence item 时按未通过处理。
- **G3**：基线落后时必须先 rebase / merge 最新主分支并重新验证；不得在过期基线上批准。
- **G4**：检查运行在哪个 SHA 必须可定位。`github` 底座只承认当前 head 的 CI 结果；重跑旧 commit 或本地通过不能替代。`local` 底座不得把“配置里写了命令”当作“命令已通过”。
- **G5**：证据不全会直接退回。任何“历史运行过”的说法都要求能定位到当前 head；无法复现即视为未验证。
- **G6**：出现数据删除 / 迁移、权限或安全模型变化、新增外部服务、不可逆动作、重大架构取舍、产品语义歧义时，必须先记录选项与影响，进入 `NEEDS_HUMAN_DECISION`，不得由执行者或 Master 单方面决定。
- **G6 风险填写深度**（`Risks / human decisions` 写到什么程度算够；触发项清单只以上一条 G6 原文为准）：
  - 命中上一条列出的任一触发项时，风险节必须写清两件事：**(a) 最坏会发生什么**——具体的失败后果与影响面（丢哪些数据、谁失去什么权限 / 能力、哪条对外行为改变），不写「有一定风险」这类空泛描述；**(b) 因此多做的一次复核**——合并前由谁额外检查什么，以及这次复核的证据落在哪里（命令、证据矩阵的哪一行、PR 评论的定位）。
  - 一条触发项都不命中时写 `none identified`（样例见 `templates/task-packet.md` 的 Sample A）。
  - 深度要求不按任务体量分级；小任务五字段可写 `N/A: <理由>` 的既有减负不变（见 4.1）。
  - 这是填写口径，不是机器门：`validate_kit` 仍只校验该字段非空，深度是否足够由 G6 审查判定；旧任务包不追溯（见 4.1 旧任务兼容策略）。`AGENTS.md` 第 5 节等更宽的 `NEEDS_HUMAN_DECISION` 清单仍然有效，但不因本口径变成风险深度的额外机器失败条件。
  - 本口径只在本节定义；`templates/task-packet.md` 与其它载体只引用它，不另写一套触发项清单或第二套措辞。

门禁之后：

- `APPROVED` 必须带当前完整 commit SHA。工作树或提交头有任何变化，批准立即失效，需重新审查。
- 只有 Master 执行合并（`local` 例如 `git merge --no-ff <branch>`，`github` 例如平台合并按钮 / `gh pr merge`；具体策略由项目配置决定）。执行者不能自批自合。
- 合并前备份或确认可恢复；遇到冲突不自动覆盖他人 / 主分支内容。
- 合并后运行 `docs/PROJECT.md` 指定的验证命令，保存日志与退出码；全部通过后转 `VERIFIED`。
- 合并成功与分支清理分开记账。清理失败不能触发重复合并。
- 默认最多 3 轮审查—修复往返；达到上限仍未通过 → `BLOCKED`，由项目所有者决定拆分、扩大范围或取消。

### 4.1 验收证据矩阵（Evidence Matrix）

G1 的“逐条验收”以**证据矩阵**记账：每条验收标准（AC）至少映射一条 evidence item，同一 AC 可以有多条（不同方法、不同环境、不同负向场景）。

字段名与判定规则只在本节定义。`templates/task-packet.md`、`editions/github/issue-template.md` 与底座手册只引用这些字段名，并说明证据存放在哪里、如何取回，**不得另写一套定义或改写取值含义**。两个底座的字段完全相同，差异只在载体：`local` 写入任务记录 / 证据文件，`github` 写入 Issue 验收清单与 PR 评论。

**证据条目（evidence item）字段** — 唯一权威定义：

| 字段 | 必填 | 含义 | 取值 / 规则 |
|------|------|------|-------------|
| `ac_id` | 是 | 该条证据证明的验收标准 | 与任务包 / Issue 中的编号一致（`AC-1`、`AC-2`…）；不得指向不存在的 AC |
| `method` | 是 | 取证方法 | 可执行命令（含参数与范围）、测试用例名、人工检查步骤 |
| `expected` | 是 | 期望结果 | 与验收标准一致、可判定的描述或期望输出 |
| `actual` | 是 | 实际观察到的结果 | 真实输出摘要 / 实测值，不得写“应该通过” |
| `status` | 是 | 该条证据的判定 | `PASS` / `FAIL` / `NOT_RUN` / `BLOCKED` / `NOT_APPLICABLE`，只允许这五个值 |
| `environment` | 是 | 证据的运行环境 | 操作系统、运行时及版本、底座、分支 / Worktree 等可复现信息 |
| `commit_sha` | 是 | 证据所属的完整 commit SHA | 必须等于矩阵声明的 head SHA；`NOT_APPLICABLE` 可写 `(none)` |
| `evidence_ref` | 是 | 证据的可定位来源 | PR / Issue 评论、日志文件、Diff 路径、任务记录条目 |
| `notes` | 是 | 备注与限制 | 无补充写 `(none)`；`NOT_APPLICABLE` 时必须写不适用的理由 |

`status` 的语义：

- `PASS`：实测结果与 `expected` 一致，且属于矩阵声明的 head SHA。
- `FAIL`：实测结果与 `expected` 不一致。
- `NOT_RUN`：本次没有执行该检查；**不得折算为通过**。`actual` 写明“未执行”的事实，`commit_sha` 写判定所依据的 head。
- `BLOCKED`：本应执行但被外部依赖或环境阻塞；**不得折算为通过**，原因写进 `notes`。
- `NOT_APPLICABLE`：该 AC 对本任务确实不适用；必须在 `notes` 写明理由，且不参与“已通过”的结论。

判定规则（G1 的执行细则）：

1. 每条 AC 至少一条 evidence item；没有任何证据的 AC 一律视为未通过。
2. AC 只有在它的**全部**证据行都是 `PASS` 或 `NOT_APPLICABLE`、且至少有一行 `PASS` 时才能标记为通过；出现 `FAIL` / `NOT_RUN` / `BLOCKED` 即视为未通过（部分通过不算通过）。
3. `actual` 与 `commit_sha` 在 `NOT_APPLICABLE` 之外为空属于证据缺失，不得算作通过。
4. `status` 写 `PASS` 但 `actual` / `notes` 实际表达“未执行 / 被阻塞 / 未验证”的，视为把未执行标成通过，直接退回。
5. 证据行 `commit_sha` 与矩阵声明的 head SHA 不一致即为证据过期；head 变化后必须在新 head 上重新取证，旧证据只能作为历史记录。
6. CI 绿灯、Agent 自述、聊天记录、历史 head 上的运行结果都不是当前 head 的充分证据。

**任务包结构化扩展字段** — 唯一权威定义（`local` 任务包与 `github` Issue 用同一组字段名）：

| 字段 | 必填 | 含义 | 填写规则 |
|------|------|------|----------|
| `invariants` | 是 | 必须始终成立、且可能被本次改动破坏的不变量 | 跨模块 / 跨步骤约束，或系统必须保持的性质 |
| `negative_cases` | 是 | 必须被拒绝、保持拒绝或保持不变的输入与操作路径 | 非法输入、越权路径、重复操作、冲突状态 |
| `migration_cases` | 是 | 旧数据 / 旧配置 / 旧版本输入的迁移与降级路径 | 含不迁移时的兼容行为与回滚动作 |
| `integration_paths` | 是 | 与其它模块、进程或外部系统的接触面 | 被改动的调用方、被依赖方与协议边界 |
| `unverified_assumptions` | 是 | 尚未验证、一旦不成立会推翻结论的假设 | 写明后续如何验证或由谁确认 |

这五个字段不得留空。小任务允许写 `N/A: <理由>`（必须带具体理由）；只写 `N/A` 或不写视为字段缺失，不得进入 `READY`。

结构检查：`python scripts/validate_kit.py` 默认校验模板内嵌样例与两底座字段一致性；`python scripts/validate_kit.py --task-packet <path>` 可单独校验一份填好的任务包（缺字段、空 AC、AC 无证据、`NOT_RUN` 被标成通过、SHA 不一致都会报错）。结构检查只证明“矩阵格式合法且没有把未执行标成通过”，不能代替 Master 的 G1 判定。

**旧任务兼容策略**：4.1 是**增量**要求，不改变既有状态机、门禁、权限与 `APPROVED` / `MERGED` / `VERIFIED` 的语义，也不引入新的状态。启用证据矩阵之前已派发或已进入审查的任务，其自然语言验收清单继续有效：Master 仍按 G1 逐条判定，只需在 PR 评论 / 任务记录中按 AC 编号补齐结论与证据引用，不必回填完整九字段矩阵。任务一旦在其任务包或 Issue 中声明使用证据矩阵，该任务整体按本节判定，不得部分回填后又按自然语言清单宣称通过。结构检查不追溯既有任务，也不把旧任务的缺失字段判为失败。

## 5. 运行时能力契约 C1–C4

在声称“可以派发子代理”之前，必须逐条探测并留下证据。**不得声称具备未探测的能力**；提示词里写了“支持”不等于运行时真的支持。

| 能力 | 契约要求 | 探测方式 | 通过判据 |
|------|----------|----------|----------|
| **C1 启动（spawn）** | 能启动一个执行实例，并返回可寻址句柄（ID / URL / 会话标识） | 启动一个只读探测任务，记录返回的句柄 | 句柄可再次定位到该实例；无句柄视为不通过 |
| **C2 隔离上下文** | 执行实例拥有独立上下文，不与其他任务共享可写会话状态 | 同时启动两个只读探测任务，检查彼此不可见对方的中间状态 | 上下文相互隔离；共享同一会话视为不通过 |
| **C3 等待取回** | 能等待实例结束，并取回状态、产物与证据（文件、命令输出、退出码、head SHA） | 让探测任务产生一个已知输出，再等待并取回 | 状态、产物、证据三者齐全且可核对 |
| **C4 区分终局** | 能区分成功、失败、取消，未知或空结果不得当作成功 | 分别制造成功、失败、取消三种探测终局 | 三种终局可区分；无法区分即不通过 |

探测流程：

1. 在任务记录（`local`：任务库事件；`github`：Issue 评论）写明运行时名称 / 版本、探测时间、探测任务 ID。
2. 逐条执行 C1–C4，记录命令、返回句柄、取回内容与终局判定。
3. 任一条不满足，则“由 Master 真实 spawn 并推进到 `DISPATCHED`”的路径**不可达**；必须降级为人工交接（`local` 的多会话交接或 `github` 的人工执行会话），并在任务记录中写明缺失能力、探测证据和降级方式。
4. 只有四条全部通过，才可使用子代理驱动路径；后续运行时升级或更换后必须重新探测。
5. 超时、取消、结果为空、句柄失效或状态不可判定，一律不得推进任务状态。

能力记录的载体是 [templates/runtime-capability.md](../templates/runtime-capability.md)：逐条记录 C1–C4 的探测结论，以及调度真正依赖的更细边界（spawn 句柄、上下文隔离、wait-collect、终局分类、取消、文件 / 命令 / CI 权限、工具与测试环境、探测时间、证据引用与限制）。未探测、证据已失效、或运行时 / 版本 / 底座 / 执行模式变化的能力一律写 `UNKNOWN`，并先降级再重测，不得沿用旧结论。**能力记录只说明“运行时被观察到能做什么”，不授予工具权限，也不替代项目的访问控制和 G1–G6。**

## 6. 底座切换

`local` → `github` 是唯一支持的迁移方向，必须一次性完成并记录切换点。**不允许双写**：同一任务不能同时在任务库和 Issue 中维护状态。

迁移清单：

1. **冻结写入**：停止 `local` 派发与状态变更，记录切换点的主分支 commit SHA 和任务库备份位置。迁移期间只允许只读查询。
2. **任务导出为 Issue**：每个未完成任务导出一个 Issue，字段按 `editions/github/issue-template.md` 的五节填写（目标 / 验收标准 / 可改路径 / 禁改路径 / 依赖）；已完成任务只归档摘要，不重建活动 Issue。
3. **锁机制替换**：路径锁不再由 SQLite 承担。开工前在 Issue 写 `ORCH-TASK-REGISTRY` 登记块，并检查所有开放 PR 的变更文件是否与 `locked_paths` 相交；两者都通过才算取得锁。
4. **状态映射**：`PLANNED` / `READY` → Issue 创建并标注状态；`DISPATCHED` / `IN_PROGRESS` → 登记块 + 分支；`REVIEW_READY` → 先开 Draft PR（`PR_OPEN`）再转 Ready；`APPROVED` 的 head SHA 必须原样带入 PR；`MERGED` 以平台合并记录为准。
5. **证据迁移**：本地验证日志转为 PR 评论或 Issue 附件，并注明“迁移前本地证据”；迁移前的证据不能冒充迁移后 head 的 CI 结果，合并后必须在主分支重跑。
6. **合并后重验**：`local` 的 `VERIFIED` 不能直接继承。合并到主分支后按 G1–G6 重跑必需检查，通过后才在新底座标记 `VERIFIED`。
7. **收尾**：把任务库标记为只读归档，在 Issue 与 README 记录切换日期、切换点 SHA 和执行人；之后所有状态只写入 Issue / PR。

约束：

- 迁移是单向的。本套件不提供 `github` → `local` 的自动回退；确需回退时由项目所有者单独立项，重新导出为新的任务记录并重新验证。
- 迁移过程中不得让同一任务的旧状态与新状态同时被不同 Agent 当作事实来源；发现双写立即停止并升级为 `NEEDS_HUMAN_DECISION`。
- 底座切换属于重大架构取舍；执行前必须获得项目所有者批准并登记。

## 7. 项目接入与新项目

新项目的技术栈、目录与验证命令可能不存在。首次任务应是“项目初始化 / 技术方案 / 最小骨架”，而不是假设已有测试脚本。先记录产品目标、限制、假设与待人拍板问题，再通过当前底座的任务载体派发初始化任务。将未知验证项明确记为“待建立”，创建相应任务；不能声称尚不存在的命令已经通过。

## 8. 阶段收口

阶段只在必需任务达到 `VERIFIED`、相关风险已登记、阻塞与人工决策有明确去向、下一阶段任务已记录时算完成。汇报至少包含：范围与状态、已验证任务、仍阻塞事项、实际验证证据、未解决风险、下一步及需用户决定的事项。

交付质量指标的口径**只在** [templates/metrics.md](../templates/metrics.md) 定义，报告由 `python scripts/report_metrics.py`（`local` 任务库或事件样本，`--json` 输出稳定字段）从任务库事件与证据引用只读计算。任何统计值必须同时给出样本量、统计窗口与缺失率；缺数据记 `unknown` / coverage，不从聊天记录或估算补齐，也不得当作没有上下文的 Agent 排名。指标不改变状态机、门禁或 `VERIFIED` 的语义。


# 第三部分：通用验收程序（原 kit/orchestration.md）

# kit/orchestration.md — 通用验收程序

每次首次接入、变更调度器 / 提示词，或更换 Agent 运行时后执行。证据必须来自真实命令和当前提交；不能将空模板、历史记录或 CI 绿灯当成已验收。本程序对两个底座通用，底座特有的检查项在标题中标注。修改核心契约、证据字段、模板、校验器、状态机 / 锁语义或发行流程时，除本程序对应段落外，还必须执行 **I 段（合成回归场景）**。

## A. 配置与底座识别

- [ ] `docs/PROJECT.md` 中不再有未处理的 `FILL_IN`。
- [ ] `edition` 取值为 `local` 或 `github`；缺失或非法时引导流程会停止并报错，而不是猜测。
- [ ] 底座只由 `docs/PROJECT.md` 的 `edition` 决定；`.orchestration/config.json` 里即使残留 `edition` 键也不影响路由（`config show` 标注为 ignored）。
- [ ] `edition` 缺失、为空、非法字符串、类型错误或文件不可读时，调度命令（`task add` / `ready` / `dispatch` / `status` / `workspace` / `amend` / `unlock`）以非零退出并报出实际值与修复位置；`init`、`backup`、`config show`、`task list`、`task show` 仍可用。
- [ ] 声明 `github` 时本地 CLI 拒绝调度并把下一步指向 Issue / PR，不冒充已支持 `github` 调度；`retry_cap` 的非法回退默认 `3` 语义未被牵连改变。
- [ ] 当前会话只使用一个底座；没有同时读写任务库与 Issue 的双写路径。
- [ ] 新 Master 上下文仅根据当前底座载体、Git 状态和证据记录恢复任务、活动锁、修复轮次与阻塞原因。
- [ ] 载体不可读 / 不可达时停止调度，而非从聊天记忆猜测。

## B. 活动任务锁与并行

- [ ] 两个路径完全不相交且无依赖的任务可分别取得锁。
- [ ] 同文件冲突及父目录 / 子文件冲突都被拒绝。
- [ ] 多路径任务若其中一条冲突，会整体拒绝；没有半套锁。
- [ ] 活动任务未生成分支 / Worktree 之前，锁也已登记。
- [ ] `VERIFIED` / `CANCELED` 后锁释放；`FAILED` / 状态未知时锁不自动释放。
- [ ] 强制解锁必须填写原因，并在事件记录中可追溯。
- [ ] `github`：开工前已检查所有开放 PR 的变更文件；登记块字段完整且时间戳单调递增。

### B2. 具名资源锁（语义只在 `docs/design/resource-locks.md`，本节只列检查点）

- [ ] 任务声明了资源时，派发已同时通过路径锁与资源锁判定；未声明资源的任务行为与未启用资源锁时一致。
- [ ] 资源 ID 按契约 §2.1 规范化：拒绝大写 / 空格 / 通配 / `..`；冲突比较为精确匹配，不做前缀推导。
- [ ] 冲突矩阵与契约 §3.1 一致：`shared`/`shared` 允许，`shared`/`exclusive` 与 `exclusive`/`exclusive` 拒绝；`EXPIRED` / `QUARANTINED` 仍参与冲突判定。
- [ ] 多资源获取是原子集合：任一冲突则整批失败，失败后该任务非 `RELEASED` 锁行数为 `0`；申请顺序为确定性顺序。
- [ ] 只有 `VERIFIED` / `CANCELED` 自动释放；`FAILED` / `BLOCKED` / 状态未知保持占用，且不把过期当释放。
- [ ] 强制解锁与回收均有理由、授权与审计事件；无事件的锁记录被判为证据缺失并隔离，而非当作有效或空闲。
- [ ] `local` 只按契约 §7.1 的范围内声明原子互斥；`github` 的登记块与交付报告中没有“原子锁 / 并发已保证”一类表述。
- [ ] 契约 §9.1 的故障场景（F1–F9）在隔离环境演练过，每个场景有预期行为与证据形式；`FAILED` / 状态未知的自动释放未被演练“修复”为通过。
- [ ] 锁数据结构新增字段已随实现任务落地：`templates/task-packet.md` 的 `declared_resources` 与 `editions/github/issue-template.md` 的 `locked_resources` 均为**可选**，留空不判失败，填写时只校验 `<id>:<mode>[:<ttl>]` 格式与逗号分隔。

## C. 合并门 G1–G6

- [ ] G1 每条验收标准都有独立依据；没有“部分通过即通过”。
- [ ] G1 的依据以 `kit/orchestration.md` 4.1 节的证据矩阵记账：每条 AC 至少一条 evidence item，九个字段齐全，`actual` / `commit_sha` 在 `NOT_APPLICABLE` 之外不为空。
- [ ] G1 没有把 `NOT_RUN` / `BLOCKED` / `NOT_APPLICABLE` 折算为通过；带这类行的 AC 未被标记为通过。
- [ ] G1 证据行的 `commit_sha` 与矩阵声明的 head SHA 一致；head 变化后已在新 head 上重新取证。
- [ ] `python scripts/validate_kit.py` 通过；`--task-packet` 对合法样例判定通过、对缺字段 / 空 AC / 无证据映射 / `NOT_RUN` 冒充通过 / SHA 不一致判定失败。
- [ ] 旧任务按 4.1 节的兼容策略处理：未回填完整矩阵的既有任务仍按自然语言清单逐条判定，且没有被宣称“已通过矩阵验收”。
- [ ] G2 变更路径全部属于允许路径；没有未授权文件或夹带重构。
- [ ] G3 分支基于最新主分支，合并预演无冲突。
- [ ] G4 必需检查在当前 head SHA 全绿；`local` 为配置中的测试与静态检查，`github` 为 CI 工作流（`.github/workflows/ci.yml`：`pull_request`→`main`、`push`→`main`、`workflow_dispatch`；Python 3.12 / 3.13 矩阵；`permissions: contents: read`）。`github` 底座下 CI 含打包自检：两个 Python 版本都执行 `python scripts/build_release.py`，失败即 job 失败（无 `continue-on-error` / `|| true`），因此打包回归在 PR 上即可暴露；`py_compile` 同时覆盖 `scripts/check_drift.py`。
- [ ] `github`：G4 以**当前 head SHA** 的 CI 运行为主要证据，证据矩阵记录运行 URL 与 head SHA；可在 PR 的 Checks 区域或 `gh pr checks <PR 编号>` 复核。本地四条命令（`validate_kit`、`unittest`、`py_compile`、`build_release`）是补充证据，不能替代 CI；CI 绿灯同样不能折算为 G1 的逐条通过。

- [ ] G5 命令、输出、退出码与 head SHA 一致；失败测试未被删除、跳过或弱化。
- [ ] G6 新依赖、迁移、权限 / 安全变化与未实现行为已登记；需要决策时已进入 `NEEDS_HUMAN_DECISION`。
- [ ] 执行者无法直接把任务标记为 `APPROVED` / `MERGED` / `VERIFIED` 而绕过 Master 审查（至少在运行纪律及工具使用上验证）。
- [ ] head 变化会使先前 `APPROVED` 失效；`github` 底座还必须在新 head 上重跑 CI，旧 head 的绿灯不作为证据。
- [ ] 合并只由 Master 操作；合并后验证未通过时不能标记 `VERIFIED`。

## D. 运行时能力探测 C1–C4

- [ ] C1 启动返回可寻址句柄，且可再次定位。
- [ ] C2 两个探测实例的上下文相互隔离。
- [ ] C3 等待取回同时得到状态、产物与证据。
- [ ] C4 成功、失败、取消三种终局可区分。
- [ ] 任一能力不满足时，`DISPATCHED` 由 Master 真实推进的路径被判定为不可达，已记录探测证据并降级为人工交接。
- [ ] 没有把未探测的能力写进交付报告或提示词。

## E. 可控失败演练

在隔离的测试仓库或演练任务中执行，不得把故意缺陷合入真实主分支。

- [ ] 制造一项可控的验收缺口，Master 能指出具体门（G1–G6）、证据缺失和修复要求。
- [ ] 修复后在新的 head 上重新运行验证。
- [ ] 默认最多 3 轮审查—修复；第 3 轮仍失败则 `BLOCKED`，不派发第 4 轮。
- [ ] spawn 失败、超时、取消、结果为空或状态不可判定时，均不当作成功。
- [ ] `github`：CI 失败或 PR 变为不可合并时，不进入 `MASTER_REVIEW`。
- [ ] 演练记录含日期、运行时 / 版本、底座、任务 ID、每轮 head SHA、命令 / 退出码、结论和清理结果。

## F. 阶段收口审计（`templates/milestone-audit.md`）

局部任务审查通过不等于阶段可以收口。阶段收口时必须用 `templates/milestone-audit.md` 逐类给结论，而不是凭记忆宣布完成。

- [ ] 阶段收口使用 `templates/milestone-audit.md`，十类检查逐条有结论；不适用项写 `N/A: <理由>`，不以空白或删除标题代替。
- [ ] 每条结论都能回到任务、证据与 head SHA；缺其中任一项的结论不算已验证，不得记为通过。
- [ ] 结论只使用 `BLOCKING` / `NON_BLOCKING_HARDENING` / `OPTIONAL` / `NEEDS_HUMAN_DECISION` 四类（核实无问题记为 `PASS`）；`NOT_RUN` / `BLOCKED` / 未验证项没有被计为通过，也没有进入通过计数。
- [ ] 存在任一 `BLOCKING` 或 `NEEDS_HUMAN_DECISION` 时，阶段没有被标记为完成，也没有在计划、任务载体或交付报告里宣称已交付。
- [ ] 冻结契约与阶段审计对同一不变量给出一致结论；契约的任何改动都有 `templates/contract-decision-log.md` 记录（ID、日期、决定人、影响任务、变更前后、理由、状态、证据链接）。
- [ ] 已冻结、未决、后续可变更三类决策分别可查；未决项在收口结论里体现为 `NEEDS_HUMAN_DECISION`，没有被当成默认值。
- [ ] 审计没有复制 `kit/orchestration.md` 的状态机或 G1–G6 定义，只引用；局部任务审查清单的结果没有被当作阶段收口证据。

## G. 证据模板

```text
日期与运行环境：
项目 / 分支：
底座（local / github）：
测试任务 ID：
任务载体备份 / Issue 链接：
调度能力实际探测（C1–C4）：
活动任务锁测试命令与结果：
具名资源锁检查与演练场景编号（F1–F9）及结论：
证据矩阵位置（任务记录 / PR 评论）：
矩阵声明的 head SHA 与各证据行 commit_sha 是否一致：
CI 运行 URL 与 head SHA（`github` 底座）：
阶段收口审计位置（`templates/milestone-audit.md`）与审计 head SHA：
阶段审计结论计数（BLOCKING / NON_BLOCKING_HARDENING / OPTIONAL / NEEDS_HUMAN_DECISION）：
是否存在 BLOCKING 或 NEEDS_HUMAN_DECISION（存在时阶段不得标为完成）：
冻结契约 / 决策日志位置与未决项（`templates/design.md`、`templates/contract-decision-log.md`）：
未执行项（NOT_RUN / BLOCKED / NOT_APPLICABLE）及其理由：
失败演练轮次与 head SHA：
第 3 轮后是否 BLOCKED：
合并后验证命令 / 退出码：
能力登记位置（`templates/runtime-capability.md`）与记录状态（COMPLETE / PARTIAL / STALE）：
未探测或已失效的能力（`UNKNOWN`）及对应降级方式：
指标报告命令、窗口与来源（`scripts/report_metrics.py`，`local` 任务库或事件样本）：
各指标的样本量 / 缺失率 / coverage（未采集项标 `unknown`）：
已知限制与未测试项：
最终结论：通过 / 部分通过 / 未通过
```

## H. 能力登记与交付度量

D 段只回答“C1–C4 是否通过”。本节回答“这些结论记在哪里、什么时候失效”以及“交付质量数字是怎么算出来的”。两者都不改变状态机、G1–G6 或 `VERIFIED` 的语义。

- [ ] 每个真实使用过的运行时 / 版本 / 底座 / 执行模式组合都有一份 [templates/runtime-capability.md](../templates/runtime-capability.md) 记录；同一运行时换了版本即视为新记录。
- [ ] 记录含运行时名称与精确版本、底座、执行模式、探测时间（UTC）、探测任务 ID、证据引用与限制；缺任一字段不得记为 `COMPLETE`。
- [ ] 未探测的能力写 `UNKNOWN`，没有把“提示词里写了支持”当作能力；C1–C4 任一条不是 `PASS` 时，子代理路径被判定为不可达并已降级为人工交接。
- [ ] 运行时升级 / 降级、底座切换、执行模式或证据载体变化后，受影响能力已降级为 `UNKNOWN` 并重新探测；没有出现“其余照旧”式的整体沿用。
- [ ] 能力记录的 `Limits` 写明了未覆盖范围；记录中没有把 `PASS` 表述成“已获得权限”，也没有替代任务允许路径、项目访问控制与 G4 的 CI 检查。
- [ ] 记录中不含凭据、会话信息或提示词原文。
- [ ] [templates/metrics.md](../templates/metrics.md) 的七项指标都有起止事件、时区、排除项与缺失处理；口径改动有日期与理由，脚本与文档没有两套定义。
- [ ] `python scripts/report_metrics.py --gold-sample --since 2026-01-01 --until 2026-02-01` 复现金样本；数值与模板中记载的期望值一致，说明样本可复现。
- [ ] 空输入得到 `no data` 与 coverage 0，而不是 0 值；缺事件的任务记为 `missing`，未采集的记录类型记为 `unknown`。
- [ ] 报告只读取事件与证据引用，没有读取凭据、提示词或消息正文；`--json` 的字段名与顺序稳定。
- [ ] 任何被引用的指标都附带样本量、窗口与缺失率，且没有被用作没有上下文的 Agent / 人员 / 运行时排名。
- [ ] `validate_kit.py` 校验两份模板的必备章节；章节被删除即失败。

## I. 合成回归场景（套件重大变更必跑）

修改 `AGENTS.md` / `kit/orchestration.md` 的核心条款、4.1 证据字段、模板结构、`scripts/validate_kit.py`、状态机或锁语义、发行流程时，必须执行本节。场景目录、注入的缺陷与预期拦截门只在 `docs/testing/delivery-regression-scenarios.md`，本节只列验收动作。

- [ ] `python scripts/run_delivery_scenarios.py --json` 退出码为 `0`；退出码 `1`（有场景失败）与 `2`（运行器未能开跑）不得当作通过。
- [ ] `totals.negative_passed` 等于 `totals.negative`：负向组“通过”表示拦截真的发生了，不是任务真的失败了。至少一个正向控制组为 `PASS`，否则不能排除“所有任务都失败”。
- [ ] `totals.errors` 为 `0`；若不为 `0`，报告必须保留异常类型与堆栈，并且剩余场景仍已执行完（单场景失败不吞掉后续）。
- [ ] 本次变更涉及的场景已同步更新：场景表的 ID 与运行器注册一致（`tests/test_run_delivery_scenarios.py` 会检查 `| <ID> |` 行）。
- [ ] 报告的 `generated_at` / `python` / `platform` 与本次证据记录一致；报告用 `--output` 写到仓库外，未提交进仓库。
- [ ] `local` 可离线完成的范围：A–D 四组的核心门语义（证据矩阵与空 AC、`NOT_RUN` 冒充 `PASS`、旧 head、edition fail-closed、路径锁与具名资源锁冲突、修复轮数、阶段审计 `BLOCKING` / `NEEDS_HUMAN_DECISION`）。
- [ ] `github` 可离线完成的范围：同一套字段、模板与校验函数；Issue / PR / 登记块 / CI 端到端**无法离线验证**，未实际运行即标 `BLOCKED`，不得用 `local` 结果替代。
- [ ] 场景只证明套件自身的门禁；不得据此宣称对任何外部产品项目做过真实数据或自动化验证。


