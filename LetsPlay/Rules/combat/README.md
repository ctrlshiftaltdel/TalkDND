# LetsPlay/Rules/combat — combat-sim 规则数据（分层）

战役无关的规则数据，供 combat-sim 读取。字段说明在引擎的开发文档里，不在本目录。

引擎读取存档清单 `campaign.yaml` 里的 `ruleset`（`2014` 或 `2024`，缺省 `2024`），先加载 `common/`，再加载对应版本目录，另一版本目录忽略。

| 目录 | 内容 |
|------|------|
| `common/` | 两版通用：核心接线 `rules.yaml`、通用状态 `conditions.yaml`、`ai-templates.yaml` |
| `2014/` | 5e（SRD 5.1）：突袭规则（失去首回合，`surprised` 状态）、版本相关状态（力竭/擒抱/失能/躲藏）、2014 版法术与职业特性。**无武器精通**。`monsters/` 只收 SRD 5.1 生物（如 Guard、Animated Armor、Mimic）；非 SRD 的（如 Allosaurus、Scarecrow）放本地并 gitignore |
| `2024/` | 5.5e（SRD 5.2.1）：突袭（先攻劣势）、版本相关状态、`masteries.yaml`、法术、特性、`monsters/` |

入库数据只取自 SRD（5.1 / 5.2.1，CC-BY-4.0，见仓库根目录 `NOTICE`）。非 SRD 的数据（例如只在怪物图鉴里的生物）放在本地 `local/`（或 `2024/monsters/scarecrow.yaml` 这类被忽略的文件），并加入仓库 `.gitignore`。
