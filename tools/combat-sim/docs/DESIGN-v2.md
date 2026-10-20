# combat-sim v2 设计与实施状态

更新：2026-10-19。状态：全部确认并已实施（step 1–4）。

## 目标

通用于任何 5e 战役的后台战斗引擎。复杂度与实用性平衡：自动化约 80%，其余由 DM
用通用命令手动裁定。

## 三层结构

1. **引擎（代码）**：只懂通用规则——d20 检定、攻击、豁免、伤害/抗性、状态、回合、
   移动、视线/掩护。不包含任何具体职业能力、法术、怪物或战役名。
2. **积木（数据）**：能力/法术/物品/陷阱由通用的效果、修正、谓词、触发器组合。
3. **内容（数据，跟战役走）**：
   - `LetsPlay/Rules/combat/`：通用库（怪物、法术、职业能力、状态、专精）。
   - `LetsPlay/Adventures/<剧本>/combat/`：场景、特殊怪物、机关、道具、遭遇。
   - `LetsPlay/Saves/<存档>/`：角色卡与战斗状态。
   工具目录只放程序和合成测试夹具。

## 实施状态

| 部分 | 状态 | 说明 |
| --- | --- | --- |
| 内容 schema / loader / check | 已实施 | YAML/JSON/Markdown 数据块；清单/显式 source；引用与表达式检查；TODO 作为 warning。 |
| 通用引擎 | 已实施 | 攻击、豁免、伤害/抗性/免疫/易伤、临时 HP、重击、先攻、动作经济、死亡豁免、借机攻击、掩体、光照、专注、基础动作、触发器/反应。 |
| 数据术语 | 已实施 | 效果 14 种、修正、谓词、触发器、资源、状态 flag、专精、场景触发器/物件。 |
| 区域图空间 | 已实施 | 区域/边距离、困难地形、攀爬、锁门、物件、触发器、高度、区域效果。 |
| 网格空间 | 已实施（简化） | 5 尺格、墙/困难/掩体/高度/区域标签、Dijkstra 移动、Bresenham 视线与掩体；高度加入攻击距离。 |
| 状态文件 | 已实施 | 自包含内容快照；`content` 与 `creature` 用 `Arc` 共享，JSON 仍可完整往返。 |
| CLI | 已实施 | `new/init/status/attack/damage/heal/save/check/roll/cast/use/condition/move/interact/scene/deathsave/next/log/rest/activate/content/simulate`。 |
| 通用 AI | 已实施（启发式） | 数据驱动模板；按 `ai.role` 分类；期望值/射程/资源阈值/逃跑/守点/保持距离/多攻击/开场技能/目标偏好；不按名字决策。 |
| 单场模拟 | 已实施 | `simulate --runs 1` 完整日志 + 摘要；`--json` 输出 log+summary。 |
| Monte Carlo | 已实施 | rayon 多线程、一次构建的 AI 计划、每 worker 种子、`NullSink`、胜率/回合/HP 统计。 |
| 测试 | 已实施 | 合成 AI/引擎/CLI/内容测试；`tests/no_content_names.rs` 内容名守卫；`tests/real_content.rs` 真实战役烟雾测试（缺路径时跳过）。 |
| 文档 | 已实施 | 本文件 + `SPEC-v2.md` 行为同步 + 中文 README。 |

## 与最初草案的偏差 / 取舍

- **网格高度**：最初只把高度用于区域图；实施时统一为攻击距离 = 切比雪夫步数 × 5 +
  高度差。移动仍按格子 Dijkstra，高度不额外收费。
- **AI `flee_zone`**：作为 `AiParams` 的可选字段加入（草案只写“move away”）。
- **AI 开场技能**：`CombatantState` 增加 `opening_used` 运行时标记，首次行动后置位；
  状态 JSON 可往返。
- **日志**：引擎改为 `LogSink` 抽象；MC 使用禁用 sink，避免热路径格式化/分配。
- **共享模板**：`EncounterState.content: Arc<ContentRegistry>`、
  `CombatantState.creature: Arc<Creature>`，降低每场 clone 成本；serde 开启 `rc`。
- **`attack_has_property`**：列表语义明确为 any-of（任一属性匹配即可）。
- **模拟来源**：`--encounter`/`--from-state`/`--state` 互斥；`runs=1` 与批量共用同一
  `play` 循环；达到 `--max-rounds` 记 timeout，无存活方记 draw，引擎错误计入
  `errors` 不中断整批。
- **性能目标**：`--runs` 批量路径已做日志禁用、内容/生物 Arc、AI 计划预计算、
  解析缓存、乘员修正快速路径等优化；实测 runs/s 见最终报告。未达到草案中
  “>100k runs/s” 的目标，瓶颈在每次攻击/施法的通用规则结算与状态 clone。

## 已知未完成

- `ready` 准备动作、复杂反应排序、召唤/变形/换装等需要 DM 裁定。
- 网格不建模朝向、体积、三维飞行/掩体高度。
- AI 不做完整博弈搜索；复杂组合法术、条件性目标切换仍可能需手动。
- 真实战役内容在外部仓库，内容作者仍需处理 `todo` 与 `manual` 项。
- Monte Carlo 的 `--seed` 为 worker 派生流，跨机器/跨版本不保证逐次完全一致。
