# Architecture Contract 模板与检查器用法

> 本模板只是一个**可复制起点**，不是默认架构。复制后必须按项目真实模块改写；检查器不会猜测目录、生成模块或自动写入任何文件。
> 语义权威：`docs/architecture/architecture-contract.md`；字段 / 枚举权威：`docs/contracts/schemas/architecture-contract.schema.json`（两者由 #47 冻结，只在仓库中提供；发行包不含 `docs/`）。本文件只说明怎么用。

## 1. 复制到项目

```bash
mkdir -p docs/architecture
cp templates/architecture/contract.example.json docs/architecture/contract.json
```

检查器按以下顺序定位契约：

1. 命令行 `--contract <path>`；
2. `docs/PROJECT.md` 的可选键 `architecture_contract_path`；
3. 默认路径 `docs/architecture/contract.json`。

**没有契约**时检查器等价 `OFF`：只输出只读盘点（文件数、源码根目录），不猜测模块边界、不产生硬失败、退出码 0。
**声明了路径但文件缺失**属于配置错误，失败关闭（退出码 1），不会静默退回 "没有契约"。

## 2. 运行

```bash
# 诊断模式（推荐先跑这个），输出人类可读报告
python scripts/architecture/check.py --project .

# 机器可读报告（供 CI / #50 / #53 读取）
python scripts/architecture/check.py --project . --json

# 指定契约或绑定 head SHA（默认取 git HEAD）
python scripts/architecture/check.py --project . --contract docs/architecture/contract.json \
    --head-sha "$(git rev-parse HEAD)" --json
```

退出码：`0` 无阻断（含无契约、`DIAGNOSTIC_ONLY`）；`1` 存在阻断项或契约不可读/非法；`2` 用法错误。

## 3. 报告状态

| 状态 | 含义 | 是否阻断 |
|---|---|---|
| `ERROR` | 命中 `ERROR` 级规则且未被有效豁免 | 仅 `ENFORCE` 下阻断 |
| `WARNING` | `WARNING` 级问题，或被有效豁免的 `ERROR`（附 `waiver_id`） | 否 |
| `UNKNOWN` | 无法可靠判定：文件无法解析、动态导入、规则未实现、参数不足 | 否，但不得读作通过 |
| `NOT_RUN` | 规则被配置启用但本次未执行：适配器被禁用、工具链缺失、`enforcement_mode=OFF` | 否，但不得读作通过 |
| `PASS` | 本次实际执行且无发现 | 否 |

汇总里每个状态单独计数；`UNKNOWN` / `NOT_RUN` 永不计入 `PASS`。报告始终包含 `head_sha`（非 git 目录写 `null` 且 `head_sha_source: "unavailable"`）、`contract_version` 与 `enforcement_mode`。

## 4. 规则与参数

检查器实现下列规则；未实现的规则种类（`DUPLICATE_BUSINESS_RULE` / `CONTRACT_TEST` / `CUSTOM`）一律报 `UNKNOWN`，不会静默通过。

| `kind` | 用途 | 参数 | 默认状态 |
|---|---|---|---|
| `NO_CYCLE` | 模块依赖成环 | 无 | 按契约 `severity` |
| `FORBIDDEN_EDGE` | 命中 `dependencies.forbidden`（支持 `*`） | 无 | 按契约 `severity` |
| `LAYER_DIRECTION` | 反向层级依赖 | `order`（模块 id 列表）、`direction`（`downward` / `upward`） | 按契约 `severity` |
| `PRIVATE_ACCESS` | 跨模块导入他人 `private_paths` | 无 | 按契约 `severity` |
| `MODULE_FAN_OUT` | 模块出向依赖数超预算 | `max` | 恒 `WARNING` |
| `MODULE_SIZE` | 单文件行数超预算 | `max_lines` | 恒 `WARNING` |
| `INTERFACE_GROWTH` | 公共接口数超过记录基线 | `baseline`（模块 id → 数量） | 恒 `WARNING` |

- 未知参数不是"忽略"而是配置错误：写错参数名会让契约在 `ENFORCE` 下失败关闭。
- 软指标即使被写成 `ERROR` 也只报 `WARNING`（冻结语义禁止用任意阈值硬阻断），消息里会注明 `static metric`。
- 缺少必需参数（如 `LAYER_DIRECTION` 没有 `order`）报 `UNKNOWN`，不报 `PASS`。

## 5. 语言适配能力

| 适配器 | 能力 | 未覆盖时 |
|---|---|---|
| Python（标准库 `ast`） | 静态 `import` / `from ... import` 的模块级与函数级依赖、相对导入 | 动态导入（`importlib.import_module` / `__import__`）、无法解析的目标、无法解析的文件 → `UNKNOWN` |
| Rust（需 `rustc` + `cargo` 在 PATH 上） | `use crate::…` 的词法扫描 | 工具链缺失 → `NOT_RUN` + 能力 `UNKNOWN`；花括号 / 通配 / 宏生成的 `use` → `UNKNOWN` |

跨语言不汇总成单一"通过"：每种适配器的能力分别记录在报告的 `capabilities` 字段。用 `--disable-adapter python|rust` 可以显式关闭某个适配器，其规则即变 `NOT_RUN`。

## 6. 豁免

豁免写在具体规则的 `waivers[]` 里，必须有理由、所有者、到期日与状态；`ACTIVE` 豁免的 `scope` 不得是整仓通配 `*`。到期后必须改为 `EXPIRED`（检查器也不会再采信过期日期），阻断立即恢复。被豁免的发现降为 `WARNING` 并附 `waiver_id`，不会从报告里消失。

## 7. 建议的启用顺序

1. 先 `DIAGNOSTIC_ONLY`，只收集现实依赖与噪音；
2. 校准模块边界与 `private_paths`，给软指标设宽松预算；
3. 只为稳定、可复现的危险模式声明 `ERROR`；
4. 经项目所有者决定（`HD-14`）后才改为 `ENFORCE`。

契约变更与本模板一样属于项目自有文件，不随发行包覆盖。
