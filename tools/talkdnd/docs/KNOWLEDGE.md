# 知识 API（`talkdnd knowledge`）

`talkdnd knowledge` 是 DM Agent 的**只读**知识入口：把 SRD、玩家自备书索引、冒险索引、存档知识、原创台账和桌上裁定放在同一组稳定命令后面。它不写存档、不建立持久化索引、不联网、不改变存档 schema，也不新增依赖。

## 命令

```text
talkdnd knowledge search <query...> [--scope srd|book|adventure|save|original|all] [--limit N] [--include-hidden]
talkdnd knowledge get <id> [--include-hidden]
talkdnd knowledge cite <id>
talkdnd knowledge context <key>
talkdnd knowledge related <id> [--depth 1|2]
```

- `search`：按 id、标题和短摘录匹配；默认 `--scope all`、`--limit 20`。
- `get`：返回单条目标内容与引用；`excerpt` 是较长的正文摘录。
- `cite`：只输出引用，不输出正文，也不接受 `--include-hidden`。
- `context`：按固定顺序汇总当前场景、队伍、已知、关系、规则指针。
- `related`：图谱关系与共享引用；默认 depth 1，只允许 1–2。

退出码：0 成功；1 工具错误（未知 id、越界 id、损坏索引、隐藏内容未加开关、参数非法）。空搜索结果不是错误，会打印可操作提示。

## 结果字段

每条 `search`/`get`/`related` 结果至少包含：

| 字段 | 含义 |
|------|------|
| `id` | 稳定引用，见下一节 |
| `authority` | `ruling` / `save` / `book` / `srd` / `adventure` / `original` |
| `title` | 条目标题或名称 |
| `citation` | 可定位引用；不含个人绝对路径 |
| `excerpt` | 有长度上限的短摘录（`get` 更长） |
| `known` | 玩家是否已知 |
| `visibility` | `player` / `dm` |

`related` 额外输出 `via`，说明是关系、同页还是共享引用。

## 稳定 id

| 来源 | 形式 | 例子 |
|------|------|------|
| SRD 内容包 | `srd:<kind>:<id>` | `srd:rule-section:rule-2024-rules-glossary-initiative` |
| 玩家书索引 | `book:<edition>:<slug>:<anchor>` | `book:2014:player-handbook:p123` |
| 冒险索引 | `adventure:<slug>:<anchor>` | `adventure:example-adventure:p42` |
| 存档知识 | `save:<entity\|memory\|reveal\|relation>:<id>` | `save:entity:E-001` |
| 原创台账 | `original:<id>` | `original:O-001` |
| 桌上裁定 | `ruling:<edition\|common>:<anchor>` | `ruling:common:R-01` |

`anchor` 来自书籍/冒险索引的页锚点；同一页有多个条目时，按索引顺序追加 `-2`、`-3` 等后缀。分卷冒险的锚点会带分卷前缀。同一输入重复运行必须得到相同 id。

## 来源权威（不是全局优先级）

`authority` 只表示来源类别，**不存在单一全局优先级**：

| authority | 含义 | 怎么用 |
|-----------|------|--------|
| `ruling` | 存档的 `Rules/rulings*.md`；本机副本缺失时读 `Rules/defaults/rulings*.md` | 桌上的裁定权威；规则书没写清楚时以它为准 |
| `save` | `entities.yaml`、`relations.yaml`、`memory.yaml`、`reveals.yaml` | 本战役已经发生/玩家已经知道的事实与关系 |
| `book` | `Rules/<edition>/<slug>/index/structured.json` 与对应页码版 | 玩家自备书的目录、页码和短摘录；引用用书名 + 页码 |
| `srd` | `Rules/combat/<edition>/srd/*.pack.json` 的 `rule_sections` 等条目 | 随程序分发的 SRD 规则原文；引用用章节 + 条目 id |
| `adventure` | 本存档冒险的 `structured.json`、页码版与玩家可见资料图 | 剧情事实；引用用相对文件路径 + 页锚点 |
| `original` | `original.yaml` | DM 原创内容；隐藏面在揭示前只作 DM 内容 |

规则裁定以存档的 `Rules/rulings*.md` 为桌上权威；书/SRD 提供规则原文；冒险提供剧情事实；原创是 DM 创作。不要把冒险事实标成规则，也不要把原创标成正典。

## 可见性

默认只返回玩家已知或可公开的内容：

- `reveals.yaml` 的事实永远按玩家已知处理。
- `memory.yaml` 只有 `source: player-confirmed` 默认可见；`import` / `manual` 是 DM 内容。
- `entities.yaml` 的 `dm_only` 实体、`relations.yaml` 的 `dm_only` 关系默认隐藏。
- `original.yaml` 中未揭示的 `hidden` 条目默认隐藏；揭示后用 `reveal add --original O-xxx` 自动转为玩家可见。
- 冒险索引中超出当前进度或标为 DM-only 的小节默认隐藏；玩家可见资料图才默认出现。
- `dm-secrets.md`、`dm-plan.md` 和手动 `secret-terms.yaml` 的词条只用于**过滤**，永不作为知识条目输出。
- 命中未揭示秘密词的条目即使加 `--include-hidden` 也不会输出；先 `talkdnd reveal add` 记录揭示。

`--include-hidden` 只扩大 save / adventure / original 的 DM 视图，结果标 `visibility: dm`、`known: false`。它**不会**输出 `dm-secrets.md` 的内容，也不会把秘密词命中项放出来。

## context 顺序

`context <key>` 固定按以下顺序输出，缺项写 `无`，每项标来源。`key` 可以是实体 id/名称；是实体时关系只列与该实体相连的条目，否则作为场景/主题标签用于规则匹配：

1. **当前场景**：`progress.yaml` 的 `scene`；`key` 是实体时额外给出焦点。
2. **队伍**：`Characters/` 与队伍清单中的角色；职业、等级、HP、AC。
3. **已知**：已揭示事实、player-confirmed 记忆、已出场原创、已识别名字、已展示资料图。
4. **关系**：当前会话生效、非 `dm_only` 的关系；实体焦点只列与该实体相连的关系。
5. **规则**：匹配的桌上裁定条目；没有匹配时列本地裁定文件指针。

`context` 的输出有固定条数上限，长战役不会把整份账本倒进上下文。

## related

`related <id>` 合并：

- `relations.yaml` 中当前生效、非 `dm_only` 的关系（关系行本身与对端实体）；
- `reveals.yaml` / `memory.yaml` 的 `npc:`、`entity:`、`original:` 标签引用；
- 共享名称/别名的实体、原创、冒险条目；
- 同一冒险页的条目（`via: same page ...`）。

`--depth 1` 只展开直接邻居；`--depth 2` 再展开一层。结果去重、按来源类别与 id 稳定排序，并有总量上限。`dm_only` 或秘密词命中的种子 id 会被拒绝。

## 规模与确定性

- `--limit` 默认 20，硬上限 500；`--limit 0` 表示使用硬上限。超过 500 是参数错误。
- `context` 各节有固定上限（队伍 20、已知 20、关系 20、规则 5）。
- `related` 有总量上限 100、每层前沿上限 50。
- 所有结果由 `BTreeMap`/`BTreeSet`、固定来源顺序和 id 排序产生；不使用 HashMap 迭代顺序或浮点排序。重复运行同一输入必须逐字节一致。
- 测试用 1000 条合成索引验证结果上限和确定性，不做 wall-clock 断言。

## 缺失与损坏

- 缺失的账本/索引 = 空来源（功能未启用），不是错误。
- 缺失 `structured.json` = 该书没有构建索引；先运行 `talkdnd adventure index build`。
- 损坏的 YAML/JSON、过高版本、未知/越界 id、非法 `--depth`、`--limit` 超限一律 fail closed，退出码 1，不 panic。
- 错误信息使用游戏目录内相对路径；不输出个人绝对路径。

## 只读保证

`knowledge` 不进入 `lib.rs` 的 integrity 跟踪分支，不写 `integrity.yaml`、不写 `secret-terms.cache.json`、不建立持久化索引。秘密词过滤直接读取 `dm-secrets.md` / `dm-plan.md` / `secret-terms.yaml` 并只保留词条用于匹配，词条本身不进入输出。

## 测试

```bash
cargo test --manifest-path tools/talkdnd/Cargo.toml --test knowledge
```

测试使用合成存档、合成书索引和合成 SRD pack；不读取真实存档、DM 秘密或书籍正文。
