# LetsPlay/Adventures — 冒险书（玩家自备）

把你自己合法拥有的冒险书放在这里，每本冒险一个文件或文件夹：

```
Adventures/
  Keys-from-the-Golden-Vault.md        # 冒险书文本（Markdown 优先，PDF 也可）
  keys-from-the-golden-vault/          # （可选）该冒险的资料目录，<slug> 用小写英文加连字符
    combat/                            # 该冒险的战斗数据，由 AI DM 按书自建
      scenes/*.yaml
      encounters.yaml
      creatures.yaml                   # 冒险特有的怪物/NPC
```

- 存档清单怎么指向这里，见 `../example/save/combat/campaign.yaml`。那条路径从清单文件数起，仍然落在本目录里。
- 冒险书本身不区分版本；用 2024 规则跑 2014 冒险也可以（反之亦然），由存档的 `ruleset` 决定按哪版结算。
- 本目录除本 README 外全部被 `.gitignore` 排除，不会被提交。
