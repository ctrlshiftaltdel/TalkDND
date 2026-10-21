# 存档槽

每个子文件夹是一份独立开档，命名：`YYYY-MM-DD-序号`（如 `2026-10-19-01`）。存档内容全部留在本地（`.gitignore` 已排除）。

```
Saves/<存档>/
  Characters/           玩家角色卡（Markdown + 末尾 ```combat yaml``` 数据块）
  NPCs/                 同伴与重要 NPC（_party.md 为总览）
  Sessions/             会话日志（每个掷骰都记录，末尾是「## 存档点」）
  combat/campaign.yaml  战斗清单，含 ruleset: "2014" | "2024"
  canon.md              即兴发明、已成定论的公开设定（店名、路人、小道消息……）
  dm-secrets.md         DM 秘密
  digest.md             接档摘要（`bin/talkdnd start` 生成，可随时重建）
  chargen.json          建卡进度（`bin/talkdnd new` / `chargen`）
  npc-drafts/           随机同伴草稿（`bin/talkdnd npc random`）
```

新存档用 `bin/talkdnd new` 自动建好以上结构。手工建时，`campaign.yaml` 的模板在 `../example/save/combat/campaign.yaml`。把它复制到 `Saves/<存档>/combat/campaign.yaml`，再改名字、`ruleset` 和 `party`。模板里的库路径从清单文件本身数，仍然落在 `../Rules/` 和 `../Adventures/` 里面。

规则书与冒险书在 `../Rules/<版本>/`、`../Adventures/`，不随存档复制。
