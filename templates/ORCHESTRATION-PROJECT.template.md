# ORCHESTRATION-PROJECT.md — Per-Project Configuration

> 这是项目适配层。首次接入时填写；不要把特定项目的技术栈、测试路径或产品约束写进通用规则文件。

## 项目身份

- 项目名称：`FILL_IN`
- 一句话目标：`FILL_IN`
- 现有项目 / 新项目：`FILL_IN`
- 本地主分支：`FILL_IN`（例如 `main` 或 `master`；不要假设名称）
- 语言 / 框架 / 运行方式：`FILL_IN`
- 重要设计 / 需求文件：`FILL_IN`（无则写“无”）

## 范围与安全

- 允许改动路径：`FILL_IN`（按任务进一步收窄）
- 永远禁止改动路径 / 文件：`FILL_IN`
- 秘密与隐私文件模式：`FILL_IN`（例如 `.env*`、密钥、个人数据）
- 破坏性操作和迁移政策：`FILL_IN`
- 新依赖政策：`FILL_IN`
- 需要人工批准的变化：`FILL_IN`

## 验证命令

每条命令必须在本机真实探测；未配置的检查写“待建立”，不要假定通过。

- 快速测试：`FILL_IN`
- 完整回归：`FILL_IN`
- 静态检查 / 类型检查：`FILL_IN`（示例：`python -m py_compile <path1> <path2>` 这类零依赖语法检查；不引入第三方依赖）
- 构建 / 打包：`FILL_IN`
- 手工 / UI 冒烟检查：`FILL_IN`
- 合并后必须执行：`FILL_IN`

## 运行配置

生效值由 `.orchestration/config.json` 提供，`python scripts/orchestrator.py config show` 可查看每个键的生效值与来源；文件缺失或值非法时回落到内置缺省。

- `edition`：`local` | `github`（二选一；缺省 `local`，当前实现只支持 `local`）
- 并发策略：**按机器余量动态决定，不设固定上限**。派发前观察 CPU / 内存 / 磁盘 / 测试资源争用，有余量即可并行；同一 worktree 同一时刻只允许一个测试进程。把「本机实测可承载并发」作为参考记录，而不是规则常量。
- 本地私有目录：`.local/`（`.env`、机器信息、发布扫描规则），永不入库、永不打包。

## 本地工作流

- 分支命名：`task/<ID>-<short-name>`
- Worktree 根目录：`.worktrees/`
- 工作目录要求：一个执行实例一个 Worktree；不共享可写工作区。
- 默认审查修复上限：`3`（可通过 `.orchestration/config.json` 的 `retry_cap` 调整；只有用户显式修改此值才可改变）
- 本地证据保留方式：`.orchestration/`；日志中需脱敏。
- 备份策略：`FILL_IN`

## 新项目启动产物

最少应建立：项目章程 / 范围与非目标、关键假设、架构决策记录（若适用）、目录结构、首个可运行垂直切片、验证方式和后续任务路线。按项目规模调整；不要求为了模板完整而过度文档化。
