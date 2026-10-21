# 建卡细节（`talkdnd chargen`）

> 本文件是 `AGENTS.md`「开新局」的参考，按需查阅。路径相对于 `LetsPlay/`。`T=bin/talkdnd`（Windows：`bin\talkdnd.exe`）。

每一步：用中文向玩家说明可选项（必要时查 `Rules/<版本>/` 里的书或 `$T options --ruleset <版本>`），**等玩家回答**，再用命令记录。程序只记录和检查，不替玩家做决定。

| 步骤 | 问玩家什么 | 记录命令 |
|------|-----------|----------|
| 规则版本 | 2014 还是 2024（只列 `Rules/2014/`、`Rules/2024/` 里有书的版本） | `$T new --ruleset 2014`（或先 `$T new`，再 `$T chargen set ruleset 2014`） |
| 冒险 | `Adventures/` 里有哪些书 | `$T chargen set adventure "Keys from the Golden Vault"` |
| 种族 / 物种 | 列出该版可选项，各一句特点 | `$T chargen set species hill-dwarf` |
| 职业 | 同上 | `$T chargen set class cleric` |
| 子职 | 等级达到子职等级时（2024 全部 3 级；2014 牧师/术士/邪术师 1 级，德鲁伊/法师 2 级，其他 3 级） | `$T chargen set subclass life` |
| 背景 | 同上（2024 背景决定属性加值和起源专长） | `$T chargen set background acolyte`；非 SRD 背景再 `$T chargen set background-skills a,b` |
| 属性生成方式 | 4d6 去最低 / 标准数组 / 购点 | `$T chargen roll` / `$T chargen array` / `$T chargen pointbuy` |
| 分配 | 把掷出的 6 个数（或数组）告诉玩家，**由玩家决定**放到哪项；购点由玩家报六项数值 | `$T chargen assign str=15 dex=14 con=13 int=8 wis=12 cha=10` |
| 属性加值 | 2024：背景三项里 +2/+1 或 +1/+1/+1；2014：种族固定加值自动生效，半精灵另选两项 +1；没有可选加值时写 none | `$T chargen asi wis=2 con=1` / `$T chargen asi none` |
| 技能 | 职业列表里选够数量（背景和种族给的自动计入） | `$T chargen set skills insight,medicine` |
| 装备 | 起始装备包或金币购买；护甲和盾牌单列；武器用英文 id 或常见中文名（长剑、匕首…） | `$T chargen set armor chain-mail`、`$T chargen set shield yes`、`$T chargen set equipment "mace; holy symbol"` |
| 武器精通（仅 2024，野蛮人/战士/圣武士/游侠/盗贼） | 持有武器多于可选数量时由玩家选 | `$T chargen set masteries longsword,javelin` |
| 法术 | 施法职业按书选戏法和法术；非施法职业写 none | `$T chargen set spells "sacred flame, cure wounds"` |
| 名字与性格 | 名字、外貌、性格、理想、牵绊、缺点 | `$T chargen set name Bruni`、`$T chargen set personality "..."` |
| 确认 | 生成角色卡给玩家看，玩家说可以再确认 | `$T chargen validate` → `$T chargen sheet --player <玩家名>` → `$T chargen confirm` |

- `$T chargen status` 随时查看进度和下一步；中途断了，下次 `talkdnd start` 的摘要会提示从哪一步接着做。
- `chargen roll` 用 combat-sim 的骰子引擎掷 6 次 4d6dl1，结果自动写入会话日志。**掷过就不能重掷，也不能改用数组或购点**。
- `validate` 有 error 必须改到 0；warning（例如非 SRD 选项）向玩家说明后可以保留。
- `sheet` 写 `Characters/<名字>.md`，人类可读部分和 `combat yaml` 数据块用同一组数值生成，并把角色加进 `combat/campaign.yaml` 的 `party`。程序生成：
  - **攻击**：装备里的武器（命中 = 属性调整值 + 熟练（熟练时）；灵巧取力量/敏捷较高者；远程用敏捷；武僧武器用武术骰），外加徒手打击；2024 带武器精通。
  - **资源**：生命骰、法术位（全/半施法者、契约魔法）、戏法数、已知/可准备法术数，以及狂暴、吟游激励、引导神力、荒野变形、回气、动作如潮、不屈、气/功力、术法点、圣疗、奥术回复等次数与恢复方式。
  - **职业特性**：到当前等级为止的职业与子职特性（名称 + 一句话概括）、属性提升等级；偷袭骰数等随等级的数值。
  - 有 combat-sim 引用的（偷袭、灵巧动作、回气、生命门徒）自动写进数据块；其他特性由 DM 按书裁定。
  - 角色卡末尾「待补（DM）」列出程序没法补的项目。改等级、装备后重新跑 `sheet`。然后跑 `bin/combat-sim content check`。

## 本地规则书数据

仓库只带 SRD 数据（12 个职业 + 各自的 SRD 子职、SRD 武器）。其他书里的子职、职业、武器要从你自己的书在本地生成，写到 `Rules/chargen/local/`（已 gitignore，**永不提交**）。程序运行时把它们叠加到 SRD 数据上。

**什么时候做**（只有这四种情况）：
1. 首次在这台电脑上开局（建卡之前）。
2. 玩家要用非 SRD 的职业 / 子职 / 武器，而 `$T data status --ruleset <版本>` 的 local entries 里没有。
3. `Rules/<版本>/` 新加了 PDF。
4. `talkdnd` 打印了 `warning: ... has no ... data`：**照抄它给出的命令执行**。

**怎么做**：
1. `$T data status --ruleset 2024`：看有没有转好的 Markdown（`Rules/2024/.md/<书名>/<书名>.md`）和已加载的本地文件。
2. 有 Markdown：`$T data import --ruleset 2024` → 写 `Rules/chargen/local/imported-2024.yaml`，再重新 `$T chargen sheet`。
3. 没有 Markdown（pdf2md 还没转完）：
   - 能开后台任务：后台跑 `uv run --project tools/pdf2md python -m pdf2md run`，不等它；
   - 只需要某职业那几页：`uv run --project tools/pdf2md python -m pdf2md run --book "Players Handbook" --pages <页码> --budget 120`，然后回到第 2 步；
   - 都来不及：建卡照常进行，缺的特性先由 DM 按 PDF 手写到 `Rules/chargen/local/manual-<版本>.yaml`（格式见下），并告诉玩家“这几项暂按书人工记录”。
4. 导入是**尽力而为**：只抓 `Level N: 特性名` 这类标题，OCR 噪声会产生错名或漏项（2014 PHB 的写法基本抓不到）。导入后对照 PDF 检查所用子职；要修改就写进 `manual-<版本>.yaml`（`imported-*.yaml` 每次导入会被覆盖）。

手写格式（与程序内置的 SRD 职业数据相同，可只写需要的部分）：

```yaml
ruleset: "2024"
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

## 同伴 NPC

1. 先问玩家要几个同伴、要不要自己指定种族或职业（玩家指定的照办）。
2. 随机部分用程序：`$T npc random --count 3`（按存档 ruleset；掷骰自动记日志）。
3. DM 为每个草稿补名字、来历、性格、说话方式，**整体给玩家看，确认或要求调整**。
4. 玩家确认后：`$T npc sheet <草稿号> --name <名字> --personality "..."`，再把声音和关系写进 `NPCs/_party.md`。

## 开新局之后

建卡和同伴都确认后：写 `dm-secrets.md`（冒险书里与本局相关的秘密）、在会话日志写开场，然后进入第一个场景。
