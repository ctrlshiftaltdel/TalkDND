# 建卡细节（`talkdnd chargen`）

> 本文件是 `AGENTS.md`「开新局」的参考，按需查阅。路径相对于 `LetsPlay/`。`T=bin/talkdnd`（Windows：`bin\talkdnd.exe`）。

每一步：用中文向玩家说明可选项（必要时查 `Rules/<版本>/` 里的书或 `$T options --ruleset <版本>`），**等玩家回答**，再用命令记录。程序只记录和检查，不替玩家做决定。

| 步骤 | 问玩家什么 | 记录命令 |
|------|-----------|----------|
| 规则版本 | 2014 还是 2024（只列 `Rules/2014/`、`Rules/2024/` 里有书的版本） | `$T new --ruleset 2014`（或先 `$T new`，再 `$T chargen set ruleset 2014`） |
| 冒险 | `Adventures/` 里有哪些书 | `$T chargen set adventure "Keys from the Golden Vault"` |
| 种族 / 物种 | 列出该版可选项，各一句特点 | `$T chargen set species hill-dwarf` |
| 职业 | 同上 | `$T chargen set class cleric` |
| 背景 | 同上（2024 背景决定属性加值和起源专长） | `$T chargen set background acolyte`；非 SRD 背景再 `$T chargen set background-skills a,b` |
| 属性生成方式 | 4d6 去最低 / 标准数组 / 购点 | `$T chargen roll` / `$T chargen array` / `$T chargen pointbuy` |
| 分配 | 把掷出的 6 个数（或数组）告诉玩家，**由玩家决定**放到哪项；购点由玩家报六项数值 | `$T chargen assign str=15 dex=14 con=13 int=8 wis=12 cha=10` |
| 属性加值 | 2024：背景三项里 +2/+1 或 +1/+1/+1；2014：种族固定加值自动生效，半精灵另选两项 +1；没有可选加值时写 none | `$T chargen asi wis=2 con=1` / `$T chargen asi none` |
| 技能 | 职业列表里选够数量（背景和种族给的自动计入） | `$T chargen set skills insight,medicine` |
| 装备 | 起始装备包或金币购买；护甲和盾牌单列 | `$T chargen set armor chain-mail`、`$T chargen set shield yes`、`$T chargen set equipment "mace; holy symbol"` |
| 法术 | 施法职业按书选戏法和法术；非施法职业写 none | `$T chargen set spells "sacred flame, cure wounds"` |
| 名字与性格 | 名字、外貌、性格、理想、牵绊、缺点 | `$T chargen set name Bruni`、`$T chargen set personality "..."` |
| 确认 | 生成角色卡给玩家看，玩家说可以再确认 | `$T chargen validate` → `$T chargen sheet --player <玩家名>` → `$T chargen confirm` |

- `$T chargen status` 随时查看进度和下一步；中途断了，下次 `talkdnd start` 的摘要会提示从哪一步接着做。
- `chargen roll` 用 combat-sim 的骰子引擎掷 6 次 4d6dl1，结果自动写入会话日志。**掷过就不能重掷，也不能改用数组或购点**。
- `validate` 有 error 必须改到 0；warning（例如非 SRD 选项）向玩家说明后可以保留。
- `sheet` 写 `Characters/<名字>.md`，人类可读部分和 `combat yaml` 数据块用同一组数值生成，并把角色加进 `combat/campaign.yaml` 的 `party`。攻击、职业特性、资源由 DM 按规则书补进数据块，然后跑 `bin/combat-sim content check`。2014 存档不能写 `mastery`。

## 同伴 NPC

1. 先问玩家要几个同伴、要不要自己指定种族或职业（玩家指定的照办）。
2. 随机部分用程序：`$T npc random --count 3`（按存档 ruleset；掷骰自动记日志）。
3. DM 为每个草稿补名字、来历、性格、说话方式，**整体给玩家看，确认或要求调整**。
4. 玩家确认后：`$T npc sheet <草稿号> --name <名字> --personality "..."`，再把声音和关系写进 `NPCs/_party.md`。

## 开新局之后

建卡和同伴都确认后：写 `dm-secrets.md`（冒险书里与本局相关的秘密）、在会话日志写开场，然后进入第一个场景。
