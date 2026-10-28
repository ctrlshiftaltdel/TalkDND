# 知识检索（`knowledge`）

`knowledge` 是 DM 的只读查表入口。它把规则裁定、剧本事实、玩家书/SRD、存档里的已知信息和原创台账放在同一组命令后面；结果带来源类别、引用和玩家可见性。

`T=bin/talkdnd`（Windows：`bin\talkdnd.exe`）。

## 什么时候用

| 你想知道 | 命令 |
|----------|------|
| 某个词在哪些来源出现过 | `$T knowledge search "<词>"` |
| 只看规则原文 | `$T knowledge search "<词>" --scope srd` |
| 只看玩家自备书索引 | `$T knowledge search "<词>" --scope book` |
| 只看冒险 | `$T knowledge search "<词>" --scope adventure` |
| 只看本战役已知/记忆/揭示/关系 | `$T knowledge search "<词>" --scope save` |
| 只看原创台账 | `$T knowledge search "<词>" --scope original` |
| 取一条完整内容 | `$T knowledge get <id>` |
| 只要引用，不要正文 | `$T knowledge cite <id>` |
| 进入一个场景前看全局 | `$T knowledge context <实体或场景>` |
| 顺着关系/引用继续查 | `$T knowledge related <id> [--depth 2]` |

## 先搜索，再取用

```text
$T knowledge search "擒抱"
$T knowledge search "馆长" --scope adventure
$T knowledge search "E-001" --scope save
$T knowledge get adventure:example-adventure:p42
$T knowledge cite ruling:common:R-07
```

`search` 默认返回 20 条，`--limit` 最大 500。没有结果时会提示换更短的词或先构建书籍索引。

书籍索引不是自动常驻的：如果某本书还没有 `index/structured.json`，先运行：

```text
$T adventure index build --book "<书名>"
```

## 来源类别不是“谁更大”

结果里的 `authority` 只说明来源：

- `ruling`：存档 `Rules/rulings*.md` 的桌上裁定。规则书没写清楚时以它为准。
- `save`：本战役的实体、关系、记忆、已揭示事实。
- `book`：玩家自备规则书的索引（书名 + 页码）。
- `srd`：随程序分发的 SRD 规则原文（章节 + 条目 id）。
- `adventure`：本存档冒险的剧情事实（相对文件 + 页锚点）。
- `original`：DM 原创内容。

不要把冒险事实当成规则，也不要把原创当成正典。引用时优先用 `cite`，它会按来源给出书名/页码、章节锚点或文件条目。

## 默认只给玩家已知

默认输出已经过滤：

- `known` 是玩家已知；`visibility: player` 才能直接用于叙述和选项。
- `dm_only` 实体/关系、`import`/`manual` 记忆、未揭示的原创隐藏面、超出进度的冒险小节、DM-only 资料图默认不出现。关系的任一端实体是 `dm_only` 时，整条关系也按 DM 内容隐藏；没有 `progress.yaml` / 自定义 track 时，`Chapter` / `Part` / `第 N 章` 且 N > 1 的章节按未来内容隐藏。
- 秘密过滤来源与 `guard` 相同且只读：`dm-secrets.md`、`dm-plan.md`、手动秘密词、未揭示原创隐藏面、未来/DM-only 冒险专名。它不写 `secret-terms.cache.json`；来自其他条目的命中项不会输出。DM-only 条目自带的隐藏面/未来章节可在 DM 视图查看，但仍标 `dm`。

`--include-hidden` 只给 DM 自己看，结果会标 `visibility: dm`、`known: false`。它仍然不会输出 `dm-secrets.md` 的内容，也不会把跨条目的秘密词命中项放出来。**不要把 DM 视图直接贴给玩家，也不要用它写选项。**

## context 的五段

`$T knowledge context <实体或场景>` 固定输出（实体 id/名称会筛选关系；其他词作为场景/主题标签用于规则匹配）：

1. 当前场景（`progress.yaml`）
2. 队伍（角色、职业、等级、HP、AC）
3. 已知（揭示、记忆、已出场原创、已识别名字、已展示资料图）
4. 关系（当前会话生效、非 DM-only）
5. 规则（匹配的裁定；没有匹配时给本地裁定文件指针）

缺项写“无”，每项都标来源。进新场景前先跑一次，比翻整本日志快。

## related

`$T knowledge related save:entity:E-001` 会给出：

- 当前生效的关系行和对端实体；
- 带 `npc:` / `entity:` / `original:` 标签的揭示和记忆；
- 共享名称/别名的原创、冒险条目；
- 同一冒险页的条目。

默认只展开一层；`--depth 2` 再展开一层。未知 id、DM-only 种子或秘密词命中会报错。

## 边界

- 只读：不改存档、不写日志、不建索引、不联网。
- 不替 DM 裁定；`ruling` 只是指向桌上裁定文件。
- 不输出个人绝对路径。
- 不把书籍正文、存档内容或 `dm-secrets.md` 贴到 Issue / PR / 聊天记录。
- 检索不到不等于不存在：可能还没构建索引、还没揭示，或用了同义词。先换词，再查 `known`/`adventure section`。
