# LetsPlay/Rules/combat — combat-sim 规则数据（分层）

战役无关的规则数据，供 combat-sim 读取。字段说明在引擎的开发文档里，不在本目录。

引擎读取存档清单 `campaign.yaml` 里的 `ruleset`（`2014` 或 `2024`，缺省 `2024`），先加载 `common/`，再加载对应版本目录，另一版本目录忽略。

| 目录 | 内容 |
|------|------|
| `common/` | 两版通用：核心接线 `rules.yaml`、通用状态 `conditions.yaml`、`ai-templates.yaml` |
| `2014/` | 5e（SRD 5.1）：突袭规则（失去首回合，`surprised` 状态）、版本相关状态（力竭/擒抱/失能/躲藏）、2014 版法术与职业特性。**无武器精通**。`monsters/` 只收 SRD 5.1 生物（如 Guard、Animated Armor、Mimic）；非 SRD 的（如 Allosaurus、Scarecrow）放本地并 gitignore |
| `2014/srd/` | `combat-sim content import-srd` 从 SRD 5.1（5e-database JSON）批量生成：`srd-monsters.yaml`（全部 SRD 怪物，普通攻击/多重攻击/豁免/技能自动化，其余能力为 `manual` 文本）与 `srd-spells.yaml`（攻击/豁免伤害/治疗法术自动化，其余为说明）。手写文件里已有的 id 不会生成，重新生成即可，不要手改 |
| `2014/srd/*.pack.json` | `content import-srd-pack` 生成的 SRD 5.1 全量内容包：怪物、法术、职业/子职/特性、种族/亚种/天赋、背景、专长、装备/魔法物品、规则章节。pack 是基础层，前面的手写 YAML 按 id 覆盖它；生成命令与统计见开发文档 `DATA.md` |
| `2024/` | 5.5e（SRD 5.2.1）：突袭（先攻劣势）、版本相关状态、`masteries.yaml`、法术、特性、`monsters/` |
| `2024/srd/*.pack.json` | `content import-srd-pack` 生成的 SRD 5.2.1 全量内容包：怪物、法术、职业/子职/特性、物种/亚种/天赋、背景、专长、装备/魔法物品/毒素。2024 SRD 上游没有规则章节数据集，规则文本暂缺；其余统计见开发文档 `DATA.md` |

入库数据只取自 SRD（5.1 / 5.2.1，CC-BY-4.0，见仓库根目录 `NOTICE`）。非 SRD 的数据（例如只在怪物图鉴里的生物）放在本地 `local/`（或 `2024/monsters/scarecrow.yaml` 这类被忽略的文件），并加入仓库 `.gitignore`。
