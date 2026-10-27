# LetsPlay/Rules/combat — combat-sim 规则数据（分层）

战役无关的规则数据，供 combat-sim 读取。字段说明在引擎的开发文档里，不在本目录。

引擎读取存档清单 `campaign.yaml` 里的 `ruleset`（四个值之一，旧 `2014` / `2024` 仍可读）。`*-srd` 按原顺序加载整个 edition 层（`common/` + `2014/` 或 `2024/`），另一 edition 目录忽略。`*-full` 先加载 SRD base（`common/` + `<edition>/srd/` + `<edition>/**/srd-*` 覆盖，如 `2024/monsters/srd-spatial.yaml`），再加载已解锁的 `full-<edition>-*` pack，最后加载 `<edition>/` 下其余手写/overlay 文件；冒险/角色覆盖在这些库之后加载。FULL 缺包时失败关闭，不静默回退 SRD。

| 目录 | 内容 |
|------|------|
| `common/` | 两版通用：核心接线 `rules.yaml`、通用状态 `conditions.yaml`、`ai-templates.yaml` |
| `2014/` | 5e（SRD 5.1）：突袭规则（失去首回合，`surprised` 状态）、版本相关状态（力竭/擒抱/失能/躲藏）、2014 版法术与职业特性。**无武器精通**。`monsters/` 只收 SRD 5.1 生物（如 Guard、Animated Armor、Mimic）；非 SRD 的（如 Allosaurus、Scarecrow）放本地并 gitignore |
| `2014/srd/` | `combat-sim content import-srd` 从 SRD 5.1（5e-database JSON）批量生成：`srd-monsters.yaml`（全部 SRD 怪物，普通攻击/多重攻击/豁免/技能自动化，`hover`/`burrow` 为结构化 `speed` 字段，其余能力为 `manual` 文本）与 `srd-spells.yaml`（攻击/豁免伤害/治疗法术自动化，`area_of_effect` 映射为 `AreaSpec` 的 `shape`/`width_ft`，其余为说明）。手写文件里已有的 id 不会生成，重新生成即可，不要手改；生成命令与空间数据缺口见开发文档 `DATA.md` |
| `2014/srd/*.pack.json` | `content import-srd-pack` 生成的 SRD 5.1 全量内容包：怪物、法术、职业/子职/特性、种族/亚种/天赋、背景、专长、装备/魔法物品、规则章节。pack 是基础层，前面的手写 YAML 按 id 覆盖它；生成命令与统计见开发文档 `DATA.md` |
| `2024/` | 5.5e（SRD 5.2.1）：突袭（先攻劣势）、版本相关状态、`masteries.yaml`、法术、特性、`monsters/`。`monsters/srd-spatial.yaml` 是从 SRD 5.2.1 怪物包投影的 hover/burrow 结构化层（仅 SRD，按 id 覆盖 pack）；`spells.yaml` 含六个 SRD 5.2.1 AoE 形状样例（sphere/cone/cube/cylinder/line/emanation），因为 2024 上游 JSON 没有 `area_of_effect` |
| `2024/srd/*.pack.json` | `content import-srd-pack` 生成的 SRD 5.2.1 全量内容包：怪物、法术、职业/子职/特性、物种/亚种/天赋、背景、专长、装备/魔法物品/毒素。2024 SRD 上游没有规则章节数据集，规则文本暂缺；其余统计见开发文档 `DATA.md` |

空间字段按 #170 schema 保存：`Speed.hover` / `Speed.burrow`，以及 `Ability.area` 的 `shape` / `width_ft`。2014 由 `srd.rs` 生成器直接输出；2024 怪物用 `2024/monsters/srd-spatial.yaml`（SRD overlay），2024 法术用 `2024/spells.yaml` 里的六个 SRD 形状样例。未覆盖项——wall 形状、cylinder 高度、2024 全量 AoE——记录在开发文档 `DATA.md`，不得用非 SRD 数据补。

入库数据只取自 SRD（5.1 / 5.2.1，CC-BY-4.0，见仓库根目录 `NOTICE`）。非 SRD 的数据（例如只在怪物图鉴里的生物）放在本地 `local/`（或 `2024/monsters/scarecrow.yaml` 这类被忽略的文件），并加入仓库 `.gitignore`。
