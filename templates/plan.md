# 设计目标（PLAN）

> 这是模板：`python scripts/orchestrator.py init` 会在 `docs/PLAN.md` 缺失且不存在 1.3 之前的根 `PLAN.md` 时，把它逐字节复制到 `docs/PLAN.md`（既有文件不会被覆盖；发现旧根 `PLAN.md` 时会提示 `git mv PLAN.md docs/PLAN.md` 迁移）。不要直接修改本模板，复制后可删除本段说明。
> 本文件只承载**设计目标**：为什么做、做到什么程度、原则与边界。阶段目标、任务顺序与状态不写在这里——需要阶段计划时，把 `templates/implementation-plan.md` 复制到项目的 `docs/`（例如 `docs/IMPLEMENTATION-PLAN.md`），`init` 不会自动创建它。
> **配图**：图放 `docs/images/`，在正文中以相对路径引用（如 `![图 1](images/xxx.png)`）。Agent 只在正文明确指向某图时才打开它，不会批量读取 `docs/images/`；设计意图必须文字自足，图是补充——每个图片引用都要有相邻文字说明。公开仓库中 `docs/images/` 是公开的，截图先脱敏。
> 两者冲突时，以本文件的设计目标为准，停下对齐，不得各写一套。

## 问题与目标用户

- 要解决的问题：`FILL_IN`
- 目标用户 / 场景：`FILL_IN`

## 设计目标

<!-- 写系统应具备的能力与不变量；每条都能判断“做到什么程度”。不要写成任务清单或阶段划分。 -->

1. `FILL_IN`

## 设计原则

<!-- 冲突时按什么顺序取舍，例如单一权威、证据先于结论、失败关闭。 -->

- `FILL_IN`

## 范围与非目标

- 范围：`FILL_IN`
- 非目标：`FILL_IN`

## 全局约束

- 技术栈 / 运行环境：`FILL_IN`
- 隐私与数据：`FILL_IN`
- 兼容与变更纪律：`FILL_IN`

## 可度量的全局验收

<!-- 可在当前 head 上重复执行的检查；不要写“已完成”“已优化”这类不可核验的结论。 -->

- [ ] `FILL_IN`

## 计划文件

- 阶段目标与执行路径：`docs/IMPLEMENTATION-PLAN.md`（项目需要阶段计划时从 `templates/implementation-plan.md` 复制；可选）
- 配图：`docs/images/`（相对路径引用；按需查看、图文双写、公开前脱敏——规则见 `AGENTS.md`）
- 项目章程 / 背景：`FILL_IN`（无则写“无”）
