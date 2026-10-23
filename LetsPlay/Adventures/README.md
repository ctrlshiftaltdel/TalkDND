# Adventures — 冒险书（玩家自备）

把你自己合法拥有的冒险书 PDF 放在这里。统一标准：**每本冒险一个文件夹**，文件夹名是书名的 slug（规则见 `../docs/layout.md`）：

```
Adventures/
  keys-from-the-golden-vault/          # 一本书一个文件夹，<slug> = 小写英文加连字符
    Keys from the Golden Vault.pdf     # 冒险书 PDF（权威来源）
    index/                             # （自动生成）pdf2md 转换结果：<书名>.pages.md、index.md、img/ …
    combat/                            # 该冒险的战斗数据，由 AI DM 按书自建
      scenes/*.yaml
      encounters.yaml
      creatures.yaml                   # 冒险特有的怪物/NPC
```

- PDF 直接放在 `Adventures/` 下也可以：`bin/talkdnd start` 或 `bin/talkdnd library tidy` 会把它（连同已有的转换结果）移进自己的文件夹，并报告移动了什么；从不删除书文件，重名时两份都保留并提示。
- 不需要自己转 Markdown；文字版由 pdf2md 在后台生成到 `index/`，见 `../docs/pdf2md.md`。
- 存档清单怎么指向这里，见 `../example/save/combat/campaign.yaml`。那条路径从清单文件数起，仍然落在本目录里。
- 冒险书本身不区分版本；用 2024 规则跑 2014 冒险也可以（反之亦然），由存档的 `ruleset` 决定按哪版结算。
- 本目录除本 README 外全部被 `.gitignore` 排除，不会被提交。
