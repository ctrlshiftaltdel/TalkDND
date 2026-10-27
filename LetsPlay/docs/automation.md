# 自动化开关（settings automation）

> 本文件是 `AGENTS.md` 的参考章节，按需查阅。路径都相对于游戏文件夹。

自动化开关决定**哪些事由 DM 直接做、哪些先问玩家、哪些只等玩家开口**。它是每个存档自己的设置，不会改变规则、骰子或战斗 AI；所有骰子仍由程序掷出并记录。

## 三个级别

| 级别 | DM 的做法 |
|------|-----------|
| `manual`（手动） | 不主动做；玩家明确要求时才做。 |
| `assisted`（辅助） | DM 主动推进，但把玩家留在环里：会替玩家角色做决定或花反应时先问，接近上限先警告；NPC 台词照常演，玩家可随时打断。 |
| `auto`（自动） | 直接做完并照常记日志，不用每步询问。 |

**玩家选择和玩家反应永远不能设为 `auto`**：属于玩家角色的决定、护盾术/幸运/激励/借机攻击等反应，只能 `assisted`（先问）或 `manual`（等玩家说）。程序会拒绝这种设置。`npc` 设为 `assisted` 时 DM 正常演 NPC，不是每句都问；玩家插话时停下即可。

## 固定类别与默认值

| 类别 | 管什么 | 默认 |
|------|--------|------|
| `resolution` | 检定、伤害、HP/资源记账等没有玩家选择的结算 | `auto` |
| `combat` | 先攻、怪物回合、条件、移动等战斗流程 | `auto` |
| `npc` | NPC 的台词、语气与临场反应 | `assisted` |
| `player_choice` | 属于玩家角色的选择与决定 | `assisted` |
| `reaction` | 护盾术、幸运、激励、借机攻击等玩家反应 | `assisted` |
| `near_limit` | HP 见底、法术位/资源将尽、力竭或负重接近上限 | `assisted` |

默认值遵循章程的自动化原则：**无选择的结算自动、玩家选择先问、接近上限先警告**。`near_limit` 为 `assisted` 时，接近上限要先提醒玩家（例如“你的法术位只剩一个了”），不要等越过后才说。

## 设置层级与优先级

一个存档有三层：

1. **存档默认**：所有场景和遭遇的底值。
2. **场景覆盖**：只对某个场景 id 生效。
3. **遭遇覆盖**：只对某个遭遇 id 生效。

生效顺序是 **遭遇 > 场景 > 存档默认**。覆盖只写你改的那几个类别，其余仍继承上一层。场景/遭遇 id 来自当前场景、战斗状态和冒险的 `combat/` 数据；不认识的 id 会被拒绝。

## 命令

```bash
$T settings automation                         # 看存档默认 + 所有覆盖
$T settings automation show --scene crypt-entry
$T settings automation show --encounter goblin-ambush
$T settings automation set npc manual --scene crypt-entry --reason "本场景玩家自己演 NPC"
$T settings automation set combat assisted --encounter goblin-ambush --reason "怪物回合先问战术"
$T settings automation reset npc --scene crypt-entry --reason "改回默认"
$T settings automation reset all --encounter goblin-ambush --reason "这场结束"
$T settings automation reset resolution --reason "回到章程默认"
```

- `show` 会逐项写出**每个值的来源**：存档默认、场景 `<id>` 或遭遇 `<id>`。
- `set` / `reset` 的 `--reason` 必填，会连同旧值、新值、层级和时间写进最新会话日志。不要在 reason 里写秘密或存档正文。
- `--scene` 和 `--encounter` 一次只能用一个；不带就是改存档默认。
- `reset` 场景/遭遇时删除该覆盖，回落上一层；`reset` 存档默认时回到章程默认。
- `$T settings` 与 `$T settings dice on|off` 保持原样；`settings.dice_display` 不受影响。

## 开局摘要

`$T start` 的摘要会有一行「自动化」，显示当前场景下的生效值、覆盖数量，并提示用 `$T settings automation` 查看来源。接档时先读这一行，再决定本场怎么说。

## 什么时候改

- 玩家说“这些小事你直接处理”——把相应类别设成 `auto`（但不能是 `player_choice` / `reaction`）。
- 玩家想自己掌控某类决定——设成 `assisted` 或 `manual`。
- 某个场景要特殊处理——用 `--scene`；某场战斗要特殊处理——用 `--encounter`。场景/遭遇结束后可以 `reset`。
- 玩家改主意时随时改；每次都有日志，不需要重开存档。

## 旧存档

旧存档第一次被 `start`、`migrate` 或任何 `settings` 写入使用时，会自动升级到 schema 9：先整份备份到 `Saves/_backups/`，再写入章程默认。旧存档的骰子显示和叙事风格保持不变；不设置自动化时行为与以前一致。

## 边界

- 自动化开关不替代规则，也不改变战斗 AI 的默认策略。
- 不能自动替玩家做选择、花反应、决定角色行动。
- 接近上限只能警告或询问，不能悄悄替玩家使用资源。
- 读取存档/冒险里的场景与遭遇元数据（场景 id/名称、遭遇 id 与所属场景）；其他字段由类型化读取忽略，不保留。永不输出、写日志或持久化 DM secrets、书籍正文或存档正文；日志只记设置元数据。
