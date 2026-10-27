# 剧本偏离度与原创内容台账（`talkdnd original`）

> 本文件是 DM 用的规则说明；玩家不需要读。路径都相对于游戏文件夹。`T=bin/talkdnd`（Windows：`bin\talkdnd.exe`）。
>
> 一句话：每个存档在开档时锁定一个「能离剧本多远」的档位和一个每章原创比例上限；所有原创内容必须先登记，预算超了就回主线。

## 1. 两个设定：scope 与 ratio

开新档时设定，写入 `Saves/<存档>/save-meta.yaml` 的 `deviation:`，**开档后锁定，没有修改命令**：

```yaml
deviation:
  scope: 2
  ratio: 15
```

| scope | 含义 | 可以原创的类型 | 档位稀有度上限 |
|-------|------|----------------|----------------|
| 0 | 不偏离：只补必要空白 | 无 | 不适用 |
| 1 | 次要原创：临时遭遇、配角、地点、物品 | 遭遇、NPC、物品、地点 | `uncommon` |
| 2 | 完整支线：在 1 的基础上可以加支线 | 遭遇、NPC、物品、地点、支线 | `rare` |

- `ratio` 是**每章**原创内容占「原创 + 正典进度」的比例上限，整数 5–25（百分比）。scope 0 不设比例，存为 `null`。
- scope 3 及以上暂未开放；给 3 会被明确拒绝。
- scope 0 只允许补空白：**给书里没名字的 NPC 起名、给对白换措辞、用自己话描述场景**都不算原创，也不用登记。
- 改编已有的剧情节点、替换关键 NPC 的动机、改结局方向，不是「原创」，任何 scope 都不允许。

### 开档时怎么问玩家

`talkdnd new` 的 `--scope` / `--ratio`：

```bash
$T new --ruleset 5.5e-srd --scope 2 --ratio 15
```

- 在终端里不带这两个参数时，程序会用中文逐项询问（scope 0/1/2；scope 1/2 再问 5–25 的比例）。
- 非交互环境（Agent 调用命令、管道）不带参数时按 **scope 0** 处理，并打印使用了默认值。
- 玩家一旦确认，本次存档就不能改。想换档位只能开新档；不要让玩家以为可以「玩到一半再放宽」。

## 2. 原创内容台账

台账在 `Saves/<存档>/original.yaml`，**只给 DM 看**，不要贴给玩家，也不要提交到 git。每个条目：

```yaml
version: 2
entries:
  - id: O-001
    type: encounter
    name: 断桥伏击
    rarity: uncommon
    chapter: The Stygian Gambit
    note: 玩家在酒馆惹到的人来报复
    hidden: 伏击者是 [[红面具]] 的手下
    player_hook: true
    logged: 2026-10-22 21:10
```

命令（都只操作当前存档；`--save` 可指定存档）：

```bash
# 先登记，再在游戏里引入
$T original add --type encounter --name "断桥伏击" --rarity uncommon --note "玩家惹到的人来报复" --hook player

# 物品必须给稀有度
$T original add --type item --name "月刃" --rarity rare --note "旧王室遗物"

# 隐藏面（只给 DM；会进入防剧透秘密词清单）
$T original add --type npc --name "馆长" --hidden "馆长是 [[红面具]] 的内应"

# 创建长期计划模板（只给 DM，已 gitignore）
$T original plan

# 玩家已经知道隐藏面后，记一条揭示
$T reveal add --original O-001 --source "调查检定 18"

# 看全部登记（DM 屏幕）
$T original list

# 看档位、比例、允许类型、稀有度上限、本章预算和全战役总数
$T original status
```

- **先登记再引入**：没登记的原创内容不能出现在叙事里。`talkdnd check` 会尽量检查手改痕迹，但 DM 的自觉是第一道防线。
- 采用了玩家提出的点子（玩家钩子）也算原创，登记时加 `--hook player`；这不改变预算。
- 条目 id 自动编号 `O-001`、`O-002`……不要手改。
- `hidden` 是 DM 的隐藏面（幕后关系、真实动机、还没被发现的伏笔）。它只在 `original.yaml` 里，玩家永远看不到；会进入 `talkdnd guard` 的秘密词清单。玩家已经知道后用 `$T reveal add --original O-001` 记录，`original list` 会标「隐藏面已揭示」，`$T check` 也不再把它算作泄漏。`talkdnd known` 把没有 `hidden` 的条目视为已出场，有 `hidden` 的到揭示后才列入。
- `dm-plan.md` 是本地长期计划（`$T original plan` 创建，已 gitignore），用 `- term: <词>` 或 `[[<词>]]` 标记关键秘密；这些词同样进入 `guard` 清单。不要把它贴给玩家或提交。
- `type` 只能是 `encounter`、`npc`、`item`、`location`、`sidequest`（也接受「遭遇」「NPC」「物品」「地点」「支线」）。
- `rarity` 只能是 `common`、`uncommon`、`rare`、`very-rare`、`legendary`。非物品条目不给时按 `common`；物品必须给。

## 3. 稀有度上限

实际上限取「档位上限」和「队伍等级上限」中较低的一个：

| 队伍等级（最低角色等级） | 等级上限 |
|--------------------------|----------|
| 1–4 | `uncommon` |
| 5–10 | `rare` |
| 11–16 | `very-rare` |
| 17+ | `legendary` |

例：scope 1 的队伍即使到了 17 级，原创物品最高仍是 `uncommon`；scope 2 在 1–4 级时最高只能是 `uncommon`，5 级以后才是 `rare`。超过上限的登记会被拒绝并说明是哪个上限卡住。

## 4. 章节与预算怎么算

### 当前章节

- 存档有冒险进度轨（`progress.yaml` 的 `track`，或内置的冒险轨）时：当前章节 = **队伍最低等级**（角色卡 `level` 的最小值）所在或之前最后一个进度轨章节。冒险轨按「预期队伍等级」排列，队伍 5 级就落在标 5 级的那一章。
- 没有进度轨（自制冒险、还没设置章节）时：全部记为 `未分章`，按同一个「章」计算。
- 自制章节可以在 `progress.yaml` 写 `track:`（`[{name, level}]`），格式与内置轨一致。

### 正典进度（canon）

正典进度不是字数，而是「已经推进/揭示的内容单元」：

- `progress.yaml` 的 `xp_log` 每条经验/里程碑记录算 1 项；
- `reveals.yaml` 的每条已揭示事实算 1 项。

**本章正典数**：从最近一条 `milestone-major`（`level-up --milestone`）记录开始，含该条，到当前为止的 `xp_log` 条数，加上日期不早于该里程碑的已揭示事实数。没有里程碑时，本章正典数 = 全战役正典数（开档以来的全部单元）。

**全战役正典数**：全部 `xp_log` 条数 + 全部已揭示事实数。

这是一条可复现的近似规则：`xp_log` 和 `reveals.yaml` 没有章节字段，程序用「上一次大章节里程碑」作为本章起点，日期精确到分钟；同一分钟内的记录算入新章。DM 不需要手工维护章节计数，只要按正常流程记 `xp add` / `level-up --milestone` / `reveal add` 即可。

### 预算公式（加一项之后检查）

```
原创 + 1 <= floor(ratio% × (原创 + 1 + 本章正典数))
```

等价于「加完后 原创 / (原创 + 正典) <= ratio%」。整数运算，不四舍五入。

- 本章原创数只统计 `chapter` 与当前章节相同的台账条目。
- 全战役总数和比例只用于展示，**不作为拒绝依据**；拒绝只看本章预算。
- 早期正典数很少时可能连 1 项都放不下（例如比例 5%、正典 0 项）。这时程序会拒绝并提示**回到主线**，先推进或揭示正典内容，再登记原创。
- `original status` 会显示本章还剩几项，以及全战役的原创/正典总数和比例。

## 5. 关键锚点：任何档位都不能改

无论 scope 多高，下面这些属于剧本骨架，不能被原创替换或改写：

- **主反派**（谁是最终敌人、其核心目的）；
- **核心秘密**（谜底、背叛者、真正身份等关键真相）；
- **结局方向**（故事最终走向、必须发生的关键事件）；
- 冒险书明确要求的关键场景、关键道具和关键 NPC 的作用。

DM 把这些锚点记在 `Saves/<存档>/dm-secrets.md`（只给 DM，**已 gitignore，不要提交**），每章开始前核对一次：原创内容只能加在锚点周围，不能动锚点本身。

## 6. 玩家自由不变

偏离度限制的是 **DM 添加原创剧情**，不是玩家。玩家想做什么仍然由玩家决定；DM 不能因为「预算满了」阻止玩家行动，只能让原创内容不进入正式剧情，或把结果落回书里的主线内容。

## 7. `char item add` 与原创物品

物品栏命令会做一次来源检查：

```bash
$T char Aria item add longsword                    # 规则/数据里有：直接通过
$T char Aria item add "rope (50 ft)"               # 常见装备：通过
$T char Aria item add "sting" --canon              # 书里有但不在数据文件：DM 明确确认
$T char Aria item add "月刃" --original O-001      # 已登记的原创物品
```

- 认得出的来源：武器数据（含本地导入）、护甲、盾牌，以及一小份常见装备名单（箭、绳、火把、口粮、背包、药水、工具等）。
- 不认识的名称且没有 `--canon` / `--original`：拒绝，并提示两条路。
- `--canon` 表示「我查过书，书里确实有这件东西」；它只跳过名称检查，不会登记原创、也不占预算。
- `--original O-xxx` 必须是本存档台账里 **type 为 item** 的条目。手改台账或指向 NPC 条目都会被拒绝。
- 起始装备、背景装备、建卡流程不经过这项检查，保持原样。
- 原创标记只存在 `sheets/<id>.yaml`（DM 数据源），**不会写进给玩家看的角色卡、资料图或摘要**。`char` 的状态行和 `start` 摘要也不要念给玩家。

## 8. 存档校验与手改

`talkdnd check`（不带参数）和 `talkdnd start` 会报告：

- `save-meta.yaml` 在工具以外被改过（包括手改 `deviation:`）；
- `original.yaml` 在工具以外被改过；
- 台账条目的类型/稀有度超出当前 scope 或队伍等级上限（例如手改成支线、传奇物品）；
- 角色卡物品带 `original:` 标记，但台账里没有对应条目，或对应条目不是物品；
- 已登记的原创 `hidden` 文本（或其 `[[词]]`）原样出现在会话日志里，却没有 `reveal add --original O-xxx` 记录。

看到这些警告：先跳出角色问玩家/核对日志；确实是 DM 自己改的，用 `$T integrity accept --reason "…"` 记录原因。不要为了「通过检查」删掉台账或改 `save-meta.yaml`。

## 9. 迁移与旧存档

- 旧存档（schema 3 及以前）没有偏离度设定。`talkdnd start` / `talkdnd migrate` 会把整份存档备份到 `Saves/_backups/`，再写入 `scope: 0`、`ratio: null`，并在会话日志记一行。
- schema 4 的旧存档在升级到 0.8.0 后迁移到 schema 5：新增 `settings: { dice_display: true }`，同样先备份并记日志；`original.yaml` 台账从版本 1 升到 2（新增空 `hidden`），旧条目不受影响。
- schema 5 的旧存档在升级到 0.9.0 后迁移到 schema 6：队友（同伴）角色卡和 `_party.md` 从 `NPCs/` 移到 `Characters/`，`sheets/<id>.yaml` 加 `controller: player|dm`；偏离度不变，同样先备份并记日志。
- schema 6 的旧存档在升级到 0.10.0 后迁移到 schema 7：存档文本里指向书的旧路径（`Adventures/<书名>.pdf`、`.md/<书名>/…`、书库整理移动过的路径）改成一书一文件夹的新路径（`Adventures/<slug>/`、`<slug>/index/`）；其余内容和偏离度不变，同样先备份、记日志并更新存档校验。
- 旧存档就是 scope 0：只能补空白，不能登记原创。想要原创只能开新档。
- 更新程序用覆盖式更新（见发布说明），存档目录原样保留。

## 10. 已知限制

- 章节靠队伍等级和进度轨推断；自制冒险没有 `track` 时全部算 `未分章`，预算按整场战役累计。
- 正典计数是「记录条数」，不是剧情重要性；DM 可以通过正常记 `xp add` / `reveal add` 推进计数，但不能为了腾预算灌水。
- 名称来源检查只覆盖规则数据里的武器、护甲、盾牌和常见装备；魔法物品、书里特有装备需要 `--canon` 或登记原创。
- 隐藏面泄漏检查是逐字/子串匹配，只检查会话日志，不扫描冒险书；改写过的泄密说法可能漏掉，DM 的纪律仍是第一道防线。
- 存档校验是防手滑，不是防作弊的安全边界；`original.yaml`、`dm-plan.md` 和摘要都只给 DM 看。
