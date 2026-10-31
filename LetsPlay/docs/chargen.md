# 建卡细节（`talkdnd chargen`）

> 本文件是 `AGENTS.md`「开新局」的参考，按需查阅。路径都相对于游戏文件夹（解压出来的那个文件夹，名字随意）。`T=bin/talkdnd`（Windows：`bin\talkdnd.exe`）。

每一步：用中文向玩家说明可选项（必要时查 `Rules/<edition>/` 里的书或 `$T options --ruleset <规则集>`），**等玩家回答**，再用命令记录。程序只记录和检查，不替玩家做决定。下文用 5e（2014）/ 5.5e（2024）说明两套 edition 的机制；存档的 `ruleset` 只写四个选项之一。

| 步骤 | 问玩家什么 | 记录命令 |
|------|-----------|----------|
| 规则集 | 5.5e FULL / 5e FULL / 5.5e SRD / 5e SRD；先看 `Rules/2014/`、`Rules/2024/` 里实际有哪版书。FULL 需要至少一本已解锁的对应 edition 完整书，否则 `start` 会失败 | `$T new --ruleset 5.5e-srd`（或 `5e-srd` / `5.5e-full` / `5e-full`；旧值 `2024` / `2014` 仍可用） |
| 剧本偏离度 | 开档时问一次：scope 0（只补空白）/1（次要原创）/2（完整支线）；scope 1/2 再问比例 5–25%。**开档后锁定**，设置前向玩家解释清楚 | 和规则集一起：`$T new --ruleset 5.5e-srd --scope 2 --ratio 15`；交互终端不带参数时程序会中文询问，非交互默认 scope 0。详见 `docs/deviation.md` |
| 冒险 | `Adventures/` 里有哪些书 | `$T chargen set adventure "Keys from the Golden Vault"` |
| 种族 / 物种 | 列出该版可选项，各一句特点 | `$T chargen set species hill-dwarf` |
| 职业 | 同上 | `$T chargen set class cleric` |
| 起始等级 | 按冒险建议范围（`$T chargen status` 的下一步会显示该范围）；必须在 1..20 | `$T chargen set level 1` |
| 子职 | 等级达到子职等级时**必须**选（2024 全部 3 级；2014 牧师/术士/邪术师 1 级，德鲁伊/法师 2 级，其他 3 级），否则 `validate` 报错 | `$T chargen set subclass life` |
| 战斗风格 | 2014：战士 1 级（箭术、防御、对决、巨武器战斗、守护、双武器战斗），圣武士 2 级（防御、对决、巨武器战斗、守护），游侠 2 级（箭术、防御、对决、双武器战斗）；2024：战士 1 级、圣武士/游侠 2 级选一个战斗风格专长（圣武士可改选受祝福的战士，游侠可改选德鲁伊战士）。到等级没选 `validate` 报错 | `$T chargen set fighting-style great-weapon-fighting` |
| 背景 | 同上（2024 背景决定属性加值和起源专长）。背景特性、工具、装备和起始金币按数据自动加入：SRD 背景内置，其他背景来自本地规则书导入（见下）；都没有时用 `set equipment` 和 `char … gold add` 补 | `$T chargen set background noble`；2024 再问 `$T chargen set background-equipment A`；没有导入数据的背景再 `$T chargen set background-skills a,b` |
| 属性生成方式 | 4d6 去最低 / 标准数组 / 购点 | `$T chargen roll` / `$T chargen array` / `$T chargen pointbuy` |
| 分配 | 把掷出的 6 个数（或数组）告诉玩家，**由玩家决定**放到哪项；购点由玩家报六项数值 | `$T chargen assign str=15 dex=14 con=13 int=8 wis=12 cha=10` |
| 属性加值 | 2024：背景三项里 +2/+1 或 +1/+1/+1；2014：种族固定加值自动生效，半精灵另选两项 +1；没有可选加值时写 none | `$T chargen asi wis=2 con=1` / `$T chargen asi none` |
| 技能 | 职业列表里选够数量（背景和种族给的自动计入） | `$T chargen set skills insight,medicine` |
| 语言与工具 | 背景/种族给的自选语言和工具；没有写 none | `$T chargen set languages "dwarvish, giant"`、`$T chargen set tools "dice set"` |
| 专长 | 2014 变体人类等、2024 额外专长（背景的起源专长自动加入） | `$T chargen set feats "Alert"` |
| 职业起始装备 | 把选项念给玩家。2014：每组选一项（字母按组顺序），或改掷起始金币；2024：A/B（战士有 C），最后一项是纯金币。物品与金币自动加入；`{martial weapon}` 这类占位项由玩家说具体武器 | `$T chargen set class-equipment a,b,a,a` / `gold`（掷骰记日志）/ `A`；再 `$T chargen set equipment greatsword` |
| 其他装备 | 额外购买的物品；护甲和盾牌可单列。**职业（含子职）不熟练的护甲或盾牌会让 `validate` 报错** | `$T chargen set armor chain-mail`、`$T chargen set shield yes`、`$T chargen set equipment "mace; holy symbol"` |
| 武器精通（仅 2024，野蛮人/战士/圣武士/游侠/盗贼） | 持有武器多于可选数量时由玩家选 | `$T chargen set masteries longsword,javelin` |
| 法术 | 施法职业按书选戏法和法术；非施法职业写 none。程序按职业数据核对**戏法数**与**已知/可准备法术数**（2014 准备施法者 = 施法属性调整值 + 等级，至少 1；2024 用表格）：过多报错，过少只警告。schema 10 起 `set spells` 会把自由文本**精确对号**成结构化 `spell_lists`（id 或名字 slug；对不上的进 `legacy`，职业未定或非施法职业则只存旧文本），`chargen validate` 优先按结构化列表核对；确认后的角色卡带 `spells` 块，且 sheet 生成时把当前可施法列表写成 combat 块的法术 refs（契约施法者带 `spell-slot-L` 成本覆盖，arcanum 带 `arcanum-N`），见 `docs/character-data.md`「法术列表」与「法术命令」 | `$T chargen set spells "sacred flame, cure wounds"` |
| 名字 | 名字 | `$T chargen set name Bruni` |
| 性格四项与阵营 | 性格特点、理想、牵绊、缺点（可参考背景的建议表，由玩家定）、阵营 | `$T chargen set traits "..."`、`set ideals`、`set bonds`、`set flaws`、`set alignment "混乱善良"` |
| 确认 | 生成角色卡给玩家看，玩家说可以再确认 | `$T chargen validate` → `$T chargen sheet --player <玩家名>` → `$T chargen confirm` |

- **剧本偏离度**在 `new` 时随规则版本一起定，之后不能改；`$T start` 摘要会显示当前档位、比例和本章预算。DM 用它约束原创内容，玩家不需要管理；详见 `docs/deviation.md`。

- `$T chargen status` 随时查看进度和下一步；中途断了，下次 `talkdnd start` 的摘要会提示从哪一步接着做。
- `chargen roll` 用 combat-sim 的骰子引擎掷 6 次 4d6dl1，结果自动写入会话日志。**掷过就不能重掷，也不能改用数组或购点**。
- `validate` 有 error 必须改到 0；warning（例如非 SRD 选项）向玩家说明后可以保留。
- 起始等级范围内置于程序（仓库跟踪的冒险数据，只有事实没有书文）；冒险不在表里时只警告，不报错。
- **种族特性**：黑暗视觉、伤害抗性、速度、种族特性概括会自动写进角色卡，并同步到 `combat yaml` 的 `senses` / `resistances`。
- `sheet` 写 `Characters/<名字>.md`，人类可读部分和 `combat yaml` 数据块用同一组数值生成，并把角色加进 `combat/campaign.yaml` 的 `party`。程序生成：
  - **攻击**：装备里的武器（命中 = 属性调整值 + 熟练（熟练时）；灵巧取力量/敏捷较高者；远程用敏捷；武僧武器用武术骰），外加徒手打击；2024 带武器精通。
  - **资源**：生命骰、法术位（全/半施法者、契约魔法）、戏法数、已知/可准备法术数，以及狂暴、吟游激励、引导神力、荒野变形、回气、动作如潮、不屈、气/功力、术法点、圣疗、奥术回复等次数与恢复方式；邪术师 11/13/15/17 级另有 Mystic Arcanum（`arcanum-6..9`，长休各 1 次，不占法术位）。
  - **职业特性**：到当前等级为止的职业与子职特性（名称 + 一句话概括）、属性提升等级；偷袭骰数等随等级的数值。
  - 有 combat-sim 引用的（偷袭、灵巧动作、回气、生命门徒）自动写进数据块；其他特性由 DM 按书裁定。
  - **战斗风格**：程序直接改数据块，combat-sim 自动结算——巨武器战斗（双手/可双手近战武器；2014 伤害骰 1、2 重掷一次，2024 视为 3；可双手武器另生成 `<武器>-2h` 双手攻击）、防御（穿护甲 AC +1）、对决（单手近战伤害 +2）、箭术（远程攻击 +2）。其余风格写进角色卡，由 DM 按书裁定。
  - 角色卡末尾「待补（DM）」列出程序没法补的项目。然后跑 `bin/combat-sim content check`。
- `sheet` 同时生成 `sheets/<id>.yaml`（角色数据的唯一来源）。**确认后角色卡只用 `$T char` 改**，见 `docs/character-data.md`。`confirm` 把起始金币记进钱袋。

### 旧存档补录（不重建角色）

旧角色卡缺战斗风格、背景装备等栏目时，不要手写备注，用 `char`（第一次使用时自动把角色卡迁移成结构化数据，原内容保留）：

```
$T char <名字> set fighting-style great-weapon-fighting
$T char <名字> set background noble            # 背景特性、工具、装备、起始金币（已手动入账就加 --no-gold，物品已有就加 --no-items）
$T char <名字> item equip greatsword           # 装备武器/护甲后自动重算攻击与 AC
```

## 本地规则书数据

**完整规则书优先，SRD 只兜底。** 玩家有完整规则书时，DM 查规则、法术、怪物、背景、专长先读 `Rules/<版本>/<slug>/index/<书名>.pages.md`（pdf2md 生成的页码版，`<a id="pN"></a>` = PDF 第 N 页；没整理过的旧书在 `Rules/<版本>/.md/<书名>/`）；书里找不到才看 SRD。`$T start` 摘要的「规则来源」和 `$T data status` 会列出这些文件。

仓库只带 SRD 数据（12 个职业 + 各自的 SRD 子职、SRD 武器、SRD 背景）。其他书里的子职、职业、武器、**背景（特性、技能、工具、语言数、装备、金币）**从你自己的书在本地生成，写到 `Rules/chargen/local/`（已 gitignore，**永不提交**）。程序运行时把它们叠加到 SRD 数据上。

**自动导入**：`$T start` 发现有转好的书而 `imported-<版本>.yaml` 不存在、比书旧或缺背景数据时，会自动运行导入并在摘要里写一行。`$T data status` 也会提示。

**什么时候手动做**（只有这四种情况）：
1. 首次在这台电脑上开局（建卡之前），且没有先运行 `$T start`。
2. 玩家要用非 SRD 的职业 / 子职 / 武器，而 `$T data status --ruleset <规则集>` 的 local entries 里没有。
3. `Rules/<版本>/` 新加了 PDF。
4. `talkdnd` 打印了 `warning: ... has no ... data`：**照抄它给出的命令执行**。

**怎么做**：
1. `$T data status --ruleset 5.5e-srd`：看有没有转好的 Markdown（`Rules/2024/<slug>/index/<书名>.pages.md`）和已加载的本地文件。
2. 有 Markdown：`$T data import --ruleset 5.5e-srd` → 写 `Rules/chargen/local/imported-2024.yaml`，再重新 `$T chargen sheet`。
3. 没有 Markdown（pdf2md 还没转完）：
   - 能开后台任务：后台跑 `uv run --project tools/pdf2md python -m pdf2md run`，不等它；
   - 只需要某职业那几页：`uv run --project tools/pdf2md python -m pdf2md run --book "Players Handbook" --pages <页码> --budget 120`，然后回到第 2 步；
   - 都来不及：建卡照常进行，缺的特性先由 DM 按 PDF 手写到 `Rules/chargen/local/manual-<版本>.yaml`（格式见下），并告诉玩家“这几项暂按书人工记录”。
4. 导入是**尽力而为**：只抓 `Level N: 特性名` 这类标题，OCR 噪声会产生错名或漏项（2014 PHB 的写法基本抓不到）。导入后对照 PDF 检查所用子职；要修改就写进 `manual-<版本>.yaml`（`imported-*.yaml` 每次导入会被覆盖）。

手写格式（与程序内置的 SRD 职业数据相同，可只写需要的部分）：

```yaml
ruleset: "5.5e-srd"   # 也可写 "5e-srd"；旧值 "2024" / "2014" 仍可读
classes:
  fighter:
    subclasses:
      battle-master:
        name: Battle Master
        features:
          - [3, Combat Superiority, 4 枚 d8 卓越骰，短休恢复]
    resources: []          # 子职资源可加在职业下：{id, name, recharge: short|long, from, by_level: [20 个数] 或 formula: level|prof|cha|5*level}
weapons: []                # 新武器：{id, name, category: simple|martial, ranged, dice, v, type, properties, range, mastery}
```

## 同伴（DM 操控的队友）

1. 先问玩家要几个同伴、要不要自己指定种族或职业（玩家指定的照办）。
2. 随机部分用程序：`$T companion random --count 3`（按存档 ruleset；掷骰自动记日志）。
3. DM 为每个草稿补名字、来历、性格、说话方式，**整体给玩家看，确认或要求调整**。
4. 玩家确认后：`$T companion sheet <草稿号> --name <名字> --personality "..."`（写到 `Characters/<名字>.md`，`controller: dm`），再把声音和关系写进 `Characters/_party.md`。旧命令名 `npc random` / `npc sheet` 仍可用。

## 开新局之后

建卡和同伴都确认后：写 `dm-secrets.md`（冒险书里与本局相关的秘密）、在会话日志写开场，然后进入第一个场景。
