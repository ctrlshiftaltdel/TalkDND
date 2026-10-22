# 版本号规则

TalkDND 用 `主版本.次版本.补丁` 三段版本号，tag 写成 `vX.Y.Z`。

| 段 | 什么时候加 | 例子 |
| --- | --- | --- |
| 补丁 `0.0.x` | 小修复：bug 修正、文档订正、数据小改，不增删功能 | v0.5.0 → v0.5.1 |
| 次版本 `0.x.0` | 增加或删除功能，或较大的改动（新命令、存档结构变化、自动迁移） | v0.5.1 → v0.6.0 |
| 主版本 `x.0.0` | 只用于真正的大版本（整体重做、正式版） | v0.x → v1.0.0 |

- 每段是独立的整数，不是小数：`0.9` 之后是 `0.10`，不是 `1.0`。
- 加次版本时补丁归零，加主版本时另两段归零。
- 三个 crate（`tools/combat-sim`、`tools/talkdnd`、`tools/talkdnd-dev`）的版本与发布 tag 保持一致，`Cargo.lock` 同步更新。
- 存档结构有变化的版本必须是次版本或主版本，并在 `talkdnd` 里加对应的自动迁移步骤（见 `LetsPlay/docs/layout.md` 的「存档结构版本」）。

## 发布流程

1. 在 `CHANGELOG.md` 顶部写新版本小节（中文；新功能 / 变更 / 修复 / 更新说明·迁移 / 已知限制），日期用发布当天。
2. 提升三个 crate 的版本号，随功能 PR 一起合并（CI 全绿后正常合并）。
3. 在 main 上打 tag `vX.Y.Z` 并推送；`release.yml` 构建三个平台包和 sha256，并用 `talkdnd-dev release-notes vX.Y.Z` 从更新日志生成 Release 说明。没有对应小节时发布失败。
4. 改已发布版本的说明：先改 `CHANGELOG.md`，再 `gh release edit vX.Y.Z --notes-file <生成的说明>`。
