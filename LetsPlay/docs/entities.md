# 实体与关系台账

> 本文件是 `AGENTS.md` 的参考章节，按需查阅。权威数据契约是同目录的 `campaign-continuity.md`（§4.3/§4.4/§5.3），本文只讲怎么用。

实体与关系台账给战役里的「人、地、组织、任务、物品」发稳定 id，并记录它们之间随时间生效的关系。它是接档时的记忆锚：新会话不用重读全部日志，查台账就能知道「这是谁、和谁有什么关系、从哪场会话开始」。

- 与 `canon.md`（公开定论）、`reveals.yaml`（玩家已知事实）的边界：台账只登记**实体与关系本身**，不定论、不记事件进程。
- 台账由 `talkdnd entity` / `talkdnd relation` 命令族写入；手改会被 `talkdnd check` 报告（integrity 在文件存在时纳入）。
- 文件：`Saves/<slot>/entities.yaml` 与 `Saves/<slot>/relations.yaml`，各自 `version: 1`；缺失 = 还没登记过，首次写入时创建。

## 会话标识

- 一个会话 = `Sessions/` 下一个日志文件，标识就是文件名（如 `2026-10-24-session-01.md`）。
- 当前会话 = 文件名排序最新的日志。时点比较按文件名字典序，不引入会话编号，也不依赖世界时间。
- 命令里传会话标识时写文件名；省略 `.md` 的写法会先按现有日志补全（唯一匹配才生效），也可带 `#锚点`（锚点不参与先后比较）。

## 实体（entities.yaml）

| 字段 | 说明 |
|---|---|
| `id` | `E-001` 起顺序编号，不重用、不随改名变化 |
| `type` | npc \| location \| faction \| quest \| item |
| `name` | 主名称；与既有主名/别名大小写不敏感比较 |
| `aliases` | 别名列表（改名时旧名自动进入这里） |
| `first_seen` | 首见会话标识，默认当前会话日志文件名，可带 `#锚点` |
| `status` | active（默认）\| retired |
| `dm_only` | true 时仅 DM 视图可见 |

### 命令

```
talkdnd entity add --type npc --name Mira --alias Bentwood --dm-only
talkdnd entity add --type faction --name 贤者商会 --first-seen 2026-10-24-session-01.md#宴会厅
talkdnd entity list                     # 默认：active 且非 dm_only（玩家视图）
talkdnd entity list --type npc --all    # DM 视图：含 retired 与 dm_only，带标记
talkdnd entity show E-001               # 也接受名称/别名，大小写不敏感
talkdnd entity rename E-001 "Mira Bentwood"
talkdnd entity retire E-001
```

规则：

- **冲突显式选择，不自动合并**：主名或别名与任何现有实体撞名时拒绝（退出码 1）并列出冲突方——同名同类型的重复登记同样拒绝，台账保持不变；由 DM 改名或改用既有实体。
- **rename 保留历史**：旧名进入 `aliases`，改名写入会话日志；重复 rename 同名是空操作；rename 成自己已有的别名 = 换回旧名。
- **retire 只改状态**：记录保留，历史查询不受影响；重复 retire 是空操作。
- **候选只提示不写入**：从日志或资料里发现疑似新实体时只建议，`entity add` 是唯一写入路径。
- `entity show`、`relation context` 对 `dm_only` 实体直接拒绝，避免细节进入玩家可看的输出。

## 关系（relations.yaml）

| 字段 | 说明 |
|---|---|
| `id` | `R-001` 起顺序编号 |
| `type` | member \| ally \| enemy \| located_at \| holds \| seeks \| knows |
| `from` / `to` | 实体 id（命令行接受 id 或唯一名称，落库一律 id） |
| `since_session` | 生效会话标识，默认当前会话 |
| `until_session` | 结束会话标识；空 = 仍生效 |
| `source` | 来源（日志锚点 / reveal 编号 / manual） |
| `dm_only` | true 时仅 DM 视图可见 |
| `note` | 可选说明（更正原因写在这里） |

### 命令

```
talkdnd relation add --type ally --from Mira --to 贤者商会 --source "reveal #12"
talkdnd relation list                          # 当前会话生效中的关系（玩家视图）
talkdnd relation list --entity E-001 --at 2026-10-24-session-00-setup.md
talkdnd relation list --all                    # DM 视图：含已结束与 dm_only，带标记
talkdnd relation end R-001 --note "登记错误：应为 enemy"
talkdnd relation show R-001
talkdnd relation context Mira                  # 实体摘要 + 当前生效关系（玩家过滤）
```

规则：

- **时点语义**：关系在会话 S 生效，当且仅当 `since_session ≤ S` 且（`until_session` 为空或 `S < until_session`），按文件名字典序。已结束关系不出现在其后会话的查询与 `relation context`。
- **更正可追溯，不物理抹除**：登记错了用 `relation end` + `--note` 说明，再 `relation add` 正确的一条；旧行保留，audit 轨迹在会话日志里。
- **幂等**：重复 add 同一 (type, from, to) 且未结束 = 返回既有 id；重复 end = 空操作；`end --at` 早于 `since` 会被拒绝。
- `from`/`to` 指向不存在的实体会被拒绝；`relation add` 按 (type, from, to) 判重，`note`/`source` 不参与。

## 玩家视图（默认输出）

`entity list`、`entity show`、`relation list`、`relation context` 的默认输出是玩家视图：

1. `dm_only` 的实体与关系一律不出现（`--all` 是 DM 视图，带 `[DM-only]` / `[retired]` / `[已结束 …]` 标记）。
2. `retired` 实体不出现在默认列表与 `relation context`（`entity show` 仍可查，状态标 retired）。
3. 拿到 DM 全量输出后，不要把它带进玩家可看的材料——`guard` 是闸门。

## 损坏与手改

- YAML 非法或 `version` 高于 1：拒绝并提示更新程序。
- 手改 `entities.yaml` / `relations.yaml`：`talkdnd check` 按 integrity 报告 Modified，确认是自己的改动后 `talkdnd integrity accept --reason`。
- `talkdnd check`（不带参数）还做跨引用审计（契约 §3.4）：`from`/`to` 指向不存在的实体 id、主名/别名重复或互相冲突（大小写不敏感全表扫描）会以 `cross-ref:` 警告列出。这是提示不是错误，check 仍退出 0；详见 `table-integrity.md` 第 4 节。
- 回退：删文件 = 清空台账（先备份）；检查点是更好的回退方式（见 `campaign-continuity.md`）。
