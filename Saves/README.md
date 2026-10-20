# 存档槽

每个子文件夹是一份独立开档，命名：`YYYY-MM-DD-序号`（如 `2026-10-19-01`）。存档内容全部留在本地（`.gitignore` 已排除）。

```
Saves/<存档>/
  Characters/           玩家角色卡（Markdown + 末尾 ```combat yaml``` 数据块）
  NPCs/                 同伴与重要 NPC（_party.md 为总览）
  Sessions/             会话日志（每个掷骰都记录，末尾是「## 存档点」）
  combat/campaign.yaml  战斗清单，含 ruleset: "2014" | "2024"
  dm-secrets.md         DM 秘密
```

`campaign.yaml` 示例：

```yaml
name: "我的战役"
ruleset: "2024"                     # 本存档使用的规则版本：2014 或 2024
library: [../../../Rules/combat]     # 引擎自动加载 common/ + 对应版本目录
adventures: [../../../Adventures/<slug>/combat]
characters: [../Characters, ../NPCs]
state_dir: .
party: [角色id, ...]
```

规则书与冒险书在项目根目录的 `Rules/<版本>/`、`Adventures/`，不随存档复制。
