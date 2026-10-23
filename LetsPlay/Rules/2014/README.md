# Rules/2014 — 5e（2014 版）规则书（玩家自备）

存档 `ruleset: "2014"` 时，AI DM 只查阅本目录。至少放一份 SRD。统一标准：**每本规则书一个文件夹**，文件夹名是书名的 slug（规则见 `../../docs/layout.md`），里面放 PDF，转换结果自动生成在 `index/`：

```
Rules/2014/
  srd-cc-v5-1/
    SRD_CC_v5.1.pdf
    index/SRD_CC_v5.1.pages.md   # （自动生成）<a id="pN"></a> = PDF 第 N 页
```

| PDF | 内容 | 必需 |
|-----|------|------|
| `SRD_CC_v5.1.pdf` | System Reference Document 5.1（CC-BY-4.0，可从 Wizards 官网免费下载） | 至少要有它 |
| 玩家手册 2014 | 完整规则书 | 可选（自有） |
| 城主指南 2014 | 完整规则书 | 可选（自有） |
| 怪物图鉴 2014 | 完整规则书 | 可选（自有） |

- 只需要放 **PDF**；可以直接放在 `Rules/2014/` 下，`bin/talkdnd start` 或 `bin/talkdnd library tidy` 会把它移进自己的文件夹并报告。不需要自己转 Markdown，pdf2md 会在后台生成页码版。
- 放在 `Rules/` 顶层、不知道版本的 PDF 不会被移动，只会提示你放进 `Rules/2014/` 或 `Rules/2024/`。
- 本目录除本 README 外全部被 `.gitignore` 排除，不会被提交。
