# ISSUE-TEMPLATE.md — github 底座任务单

每个 Issue 的正文按下列五节填写。五节与 `local` 底座的任务包字段一一对应，缺任一节不得进入 `READY`。

## 目标

- 单一、可验收的目标。
- 背景与用户可感知的结果。
- 明确不包含的内容（非目标）。

## 验收标准

- 逐条编号，每条都要能在当前 head 上判定通过 / 不通过。
- 写明每条对应的证据形式（命令输出、CI 运行、Diff 位置）。
- 部分通过不算通过。

## 可改路径

- 完整列出允许修改的文件或目录。
- 路径粒度与该任务实际改动一致；不要用整个仓库兜底。
- 与 `ORCH-TASK-REGISTRY` 的 `locked_paths` 完全一致。

## 禁改路径

- 列出本任务禁止改动的文件或目录；未列出的路径默认仍受项目级禁改清单约束。
- 写明是否需要人工批准才能扩大范围。

## 依赖

- 列出必须先达到 `VERIFIED` 的 Issue 编号。
- 写明依赖的完成判据（合并 + 主分支验证）。
- 无依赖时写“无”。

---

标签建议：`status:ready`、`status:in-progress`、`status:review`、`status:blocked`，以及类型标签（`type:feature`、`type:fix`、`type:docs`、`type:chore`）。状态标签必须与 `ORCH-TASK-REGISTRY` 的 `status` 保持一致。
