# 感官与遮蔽（combat-senses）

本页说明 `bin/combat-sim` 如何判断“谁看得见谁”。规则数据来自当前存档的 `ruleset`：2014 与 2024 使用同一套引擎规则，版本差异由 `Rules/combat/2014/` 或 `Rules/combat/2024/` 的数据提供。

## 生物的感官

生物数据里的 `senses` 仍是自由文本，程序在结算时解析，不会改写角色卡或存档。支持的写法（大小写不敏感，逗号或分号分隔）：

```yaml
senses: "Blindsight 60 ft.; Darkvision 120 ft.; Passive perception 21"
senses: "darkvision 60 feet; passive perception 12"
senses: "Truesight 120"
```

可识别 `darkvision`、`blindsight`、`tremorsense`、`truesight` 与 `passive perception`。缺少单位也可以；缺失、0、负数或写错的距离按“没有该感官”处理，不会让程序崩溃。同一感官写两次时取较大值。

没有 `senses` 的生物按普通视觉处理：明亮正常看见，微光轻度遮蔽，黑暗重度遮蔽。

## 光照与遮蔽

场景和区域可以写 `light` 与 `obscured`：

```yaml
scenes:
  - id: foggy-road
    mode: zones
    light: dim
    obscured: light
    zones:
      - {id: open, light: bright}
      - {id: fog-bank, light: bright, obscured: heavy}
```

- `light`：`bright` / `dim` / `dark`。
- `obscured`：`none` / `light` / `heavy`，可省略，默认 `none`。
- 区域场景先看该区域的值，没有再回退到场景值；网格场景只看场景级 `obscured`，写在区域上会被 `content check` 警告并忽略。

两者取更严重的一项：

| 光照 | 遮蔽 | 有效可见度 |
|------|------|------------|
| 明亮 | 无 | 清晰 |
| 微光 | 无 | 轻度遮蔽 |
| 明亮 | 轻度 | 轻度遮蔽 |
| 黑暗 | 无 | 重度遮蔽 |
| 明亮 | 重度 | 重度遮蔽 |
| 黑暗 | 轻度 | 重度遮蔽 |

只有重度遮蔽或完全看不见目标才影响攻击检定。微光本身不给攻击劣势；它影响的是察觉，而察觉由 DM 手动掷骰。

## 感官规则

- **黑暗视觉 Darkvision**：范围内把黑暗当微光、微光当明亮；不穿透轻/重度遮蔽，范围外无效。
- **盲视 Blindsight**：范围内忽略光照与遮蔽，也能感知躲藏和隐形的生物；仍受全掩护/视线阻挡。
- **震颤感知 Tremorsense**：范围内感知接触地面的生物，忽略光照、遮蔽与掩护；飞行或悬浮（即使高度 0）不算接触地面，带 `incorporeal` 标签的生物也感知不到；能感知躲藏的生物。
- **真实视觉 Truesight**：范围内看穿普通黑暗与 `invisible`，仍受全掩护阻挡；不能看穿普通躲藏（`hidden`）。
- **`hidden`（躲藏）**：普通视觉和黑暗视觉都看不见；盲视/震颤感知在范围内可以感知；真实视觉不能。
- **`invisible`（隐形）**：普通视觉和黑暗视觉看不见；盲视、震颤感知（需接触地面）和真实视觉可以感知。

**魔法黑暗不在本版本范围**。程序不知道某个黑暗效果是否魔法；需要“连黑暗视觉也看不穿”的区域时，可以把场景写成 `dark` + `obscured: heavy`，但这是数据表达，不是法术识别。

## 攻击与日志

攻击、能力攻击和 AI 评估使用同一个判断：

- 攻击者看不见目标 → 劣势，日志来源写 `darkness`、`heavy obscurement` 或 `unseen`，并附感官/光照原因；
- 目标看不见攻击者 → 优势，日志来源写 `unseen`；
- 两者同时成立 → 按优势与劣势互相抵消，普通掷骰。

全掩护不会被重复算成“看不见目标”的劣势；掩护仍按原来的 AC 与合法性规则处理。躲藏/隐形的数据修正只在观察者有对应感官时被抑制，避免和感官重复计算。

## 状态查看

`bin/combat-sim status --json` 的每个战斗者会多出：

- `senses`：原始感官文本；
- `vision`：`light`、`effective_light`、`obscurement`，以及可用的 `darkvision_ft`、`blindsight_ft`、`tremorsense_ft`、`truesight_ft`、`passive_perception`。

文本 `status` 仍是一行一个战斗者，必要时在行尾显示 `lightly obscured`、`heavily obscured` 或 `unseen`。给玩家看时继续用 `--player`，名字遮蔽行为不变。

## 给 DM 的用法

1. 战斗开始前在场景/区域里写清 `light` 与 `obscured`；没有写就是明亮、无遮蔽。
2. 怪物数据保留规则书/内容包给的 `senses` 原文，不要为了引擎改写。
3. 玩家主动搜索、聆听或判断雾里有什么时，照常手动掷感知（察觉），DC 由你定；引擎不会自动重掷察觉。
4. 需要“魔法黑暗”时，按数据写成 `dark` + `obscured: heavy`，并在日志里说明这是魔法效果；不要把普通黑暗误写成连黑暗视觉也无效。
5. 攻击日志出现 `cannot see target` 时，先看括号里的光照/遮蔽/感官原因，再决定叙述；不要只报“劣势”。

## 兼容性

- `obscured` 是可省略字段，旧内容与旧战斗存档照常加载。
- 战斗状态 schema 版本不变，没有新增持久化字段，也没有 `talkdnd` 迁移。
- 没有 `senses` 的生物按普通视觉；旧 `attack` / `use` 命令的参数和行为不变。
