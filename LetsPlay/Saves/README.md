# 存档槽

每个子文件夹是一份独立开档，命名：`YYYY-MM-DD-序号`（如 `2026-10-19-01`）。存档内容全部留在本地（`.gitignore` 已排除）。

```
Saves/<存档>/
  Characters/           全部队伍成员的角色卡：玩家角色和 DM 操控的同伴（Markdown + 末尾 ```combat yaml``` 数据块；_party.md 为同伴总览）
  NPCs/                 剧情 NPC（可为空；队友不放这里）
  Sessions/             会话日志（每个掷骰都记录，末尾是「## 存档点」）
  combat/campaign.yaml  战斗清单，含 ruleset: "2014" | "2024"
  canon.md              即兴发明、已成定论的公开设定（店名、路人、小道消息……）
  dm-secrets.md         DM 秘密
  digest.md             接档摘要（`bin/talkdnd start` 生成，可随时重建）
  chargen.json          建卡进度（`bin/talkdnd new` / `chargen`）
  npc-drafts/           随机同伴草稿（`bin/talkdnd companion random`）
  save-meta.yaml        存档结构版本、迁移记录与开档时锁定的剧本偏离度（程序维护，不要手改）
  original.yaml         DM 的原创内容台账（`talkdnd original`；只给 DM，不要给玩家看）
  integrity.yaml        存档校验清单（程序维护）
  sheets/               角色数据源 yaml（`talkdnd char` 维护；`controller: player` 玩家角色 / `controller: dm` 同伴）
```

队友和玩家角色放在一起，靠 `controller` 字段区分，不再靠文件夹；旧存档（schema 5 及以前）把队友放在 `NPCs/`，`talkdnd start` 会自动迁移到 `Characters/`（先整份备份），剧情 NPC 留在原处。

整份存档的迁移备份放在 `Saves/_backups/<存档>-<时间>-schema<N>/`；`_` 和 `.` 开头的目录不是存档槽，程序不会把它们当成可读档。

新存档用 `bin/talkdnd new` 自动建好以上结构。手工建时，`campaign.yaml` 的模板在 `../example/save/combat/campaign.yaml`。把它复制到 `Saves/<存档>/combat/campaign.yaml`，再改名字、`ruleset` 和 `party`。模板里的库路径从清单文件本身数，仍然落在 `../Rules/` 和 `../Adventures/` 里面。

规则书与冒险书在 `../Rules/<版本>/`、`../Adventures/`，不随存档复制。
