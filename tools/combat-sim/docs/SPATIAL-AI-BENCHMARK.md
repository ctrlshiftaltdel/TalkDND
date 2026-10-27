# 空间与 AI 质量基准（P8 / TASK-0093 / Issue #177）

状态：质量门报告。本文记录**可复现的合成基准**、P8 默认策略结论、启用/禁用条件与已知限制。它不改变规则引擎、存档 schema、`Ai::prepare` 或 CLI 默认值。

- 测量 commit：`8405747d3707f00248c3bf9bdf28bea80e3e1921`
- 基线：`origin/main` @ `ada82c0621dab21e8d71adeff88087e262c3da27`
- 代码：`tests/spatial_regression.rs`、`tests/ai_quality.rs`（本 PR 新增）
- 依赖结论：
  - #179 `AI-SEARCH-PERF-SPIKE.md`：方案 A，1–2 层、256–512 节点、100 ms 软上限 / 300 ms 硬上限，不做 MCTS。
  - #178 `DATA.md`：2014 有 7 个 hover / 21 个 burrow 与 81 个结构化 AoE；2024 有 38 个 hover/burrow 覆盖与 6 个形状示例；仍存在真实数据缺口。

## 1. 结论摘要

1. **P8 不改变默认策略。** `Ai::prepare` 与 CLI 仍使用 legacy heuristic；UtilityPlanner 与 SearchPlanner 保持显式 opt-in。原因是本基准同时观察到改进和明确回归，不能用一个场景的目标完成率覆盖另一个场景的胜率下降。
2. **UtilityPlanner 提高了目标完成率，但在团队战出现回归。** 目标场景 8/8 vs legacy 0/8；团队场景 1/8 vs legacy 7/8 胜；近战场景三者同为 1/8 胜。
3. **SearchPlanner 没有在这些场景上超过 UtilityPlanner。** 目标、近战、团队三种场景的胜负与目标结果和 Utility 相同；决策 P50 从 9–24 µs 升到 91–144 µs，P95 最高 448 µs，并增加 1,670 个搜索节点/8 次团队运行。
4. **搜索预算本身可行。** 当前默认 `SearchConfig` 为 2 层、top-K 8、对手 top-K 4、每个候选 2 个样本、384 节点、100 ms 软 / 300 ms 硬。实测每个决策远低于硬上限；节点总数也低于 `384 × 决策数`。
5. **search fallback 主要来自没有候选的敌方 actor。** 目标场景总 fallback 50%（48/96 个 actor 回合），团队场景 22.8%（49/215）；party-only fallback 在三个场景都是 0%。报告同时给出总率和 party-only 率，避免把“木桩敌人无法行动”误读成 party planner 失败。
6. **不把合成 fixture 当成真实 SRD 覆盖。** 本文的 10 个空间回归和 3 个 AI 质量场景全部是通用合成数据；真实内容覆盖与缺口见 §7。

## 2. 环境与复现

| 项 | 值 |
|----|----|
| OS / kernel | Linux 6.12.113+deb13-amd64（x86_64） |
| CPU | Intel(R) Xeon(R) CPU E3-1230 V2 @ 3.30GHz，4 逻辑 CPU |
| Rust | rustc 1.99.1 (58cf8f9c3 2026-10-13) / cargo 1.99.1 |
| Profile | `release`（仓库 `Cargo.toml`：`lto = "fat"`、`codegen-units = 1`、`panic = "abort"`） |
| Sink | `Vec<String>`，仅基准内解析每回合日志；生产搜索仍用 `NullSink` |
| 测量 commit | `8405747d3707f00248c3bf9bdf28bea80e3e1921` |
| 固定 seed | 基准 `0x5eed0093`；8 个派生 seed：`1592590483, 4247026252, 6901462021, 9555897790, 12210333559, 14864769328, 17519205097, 20173640866` |
| 场景 | `objective`、`melee`、`team`，每个策略/seed 使用同一初始 state |

显式大规模基准（本次实际运行，退出码 `0`）：

```bash
CARGO_BUILD_JOBS=2 cargo test --release \
  --manifest-path tools/combat-sim/Cargo.toml \
  --test ai_quality quality_benchmark_prints_environment_and_metrics -- --ignored --nocapture
```

普通 CI 只运行小型确定性测试，不执行该 `#[ignore]` 基准，也不断言 wall-clock：

```bash
CARGO_BUILD_JOBS=2 cargo test \
  --manifest-path tools/combat-sim/Cargo.toml \
  --test ai_quality --test spatial_regression
```

## 3. 指标定义

| 指标 | 定义 |
|------|------|
| `legal_action_rate` | 产生至少一次引擎接受的状态变化（动作、移动、交互、资源/条件/触发器变化）的 actor 回合 / 全部 actor 回合。仅 wait 的回合是合法的，但不算 action 回合。 |
| `party_legal_action_rate` | 同上，仅统计 party actor。 |
| `fallback_rate` | `search planner fallback` 或 `search planner failed`，加 Utility 回退到 legacy 的次数 / 全部 actor 回合。一次搜索回退同时触发“search fallback”和后续 Utility 路径，只按一次搜索回退计。 |
| `party_fallback_rate` | 同上，仅统计 party actor。 |
| `objectives` | 目标场景：lever 的 `toggle_trigger` 已关闭；战斗场景：party 获胜。 |
| `wins/draws/timeouts` | `CombatOutcome`，或达到 `max_rounds`。目标场景固定 6 轮；战斗场景 20 轮。 |
| `p50_us/p95_us` | 每次 `Ai::take_turn` 的墙钟耗时，按所有 actor 回合汇总。单机数字，不是跨机器保证。 |
| `nodes` | 从 search 日志解析的 `nodes=` 总和。legacy/utility 不扩展搜索节点，记为 0/N/A。 |

## 4. 测量结果

`runs=8` / 场景 / 策略。表内为 8 次运行汇总。

| 策略 | 场景 | legal | party legal | wins | draws | timeouts | objectives | avg rounds | avg party HP | P50 µs | P95 µs | fallback | party fallback | nodes |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| legacy | objective | 0.000 | 0.000 | 0 | 0 | 8 | 0/8 | 6.00 | 20.00 | 1 | 1 | 0.000 | 0.000 | 0 |
| legacy | melee | 1.000 | 1.000 | 1 | 0 | 0 | 1/8 | 3.25 | 1.00 | 13 | 26 | 0.000 | 0.000 | 0 |
| legacy | team | 0.598 | 0.864 | 7 | 0 | 1 | 7/8 | 14.12 | 18.25 | 9 | 26 | 0.000 | 0.000 | 0 |
| utility | objective | 0.083 | 0.167 | 0 | 0 | 8 | 8/8 | 6.00 | 20.00 | 9 | 26 | 0.000 | 0.000 | 0 |
| utility | melee | 1.000 | 1.000 | 1 | 0 | 0 | 1/8 | 3.25 | 1.00 | 24 | 68 | 0.000 | 0.000 | 0 |
| utility | team | 0.668 | 0.813 | 1 | 0 | 0 | 1/8 | 7.38 | 1.25 | 21 | 68 | 0.000 | 0.000 | 0 |
| search | objective | 0.083 | 0.167 | 0 | 0 | 8 | 8/8 | 6.00 | 20.00 | 34 | 37 | 0.500 | 0.000 | 96 |
| search | melee | 1.000 | 1.000 | 1 | 0 | 0 | 1/8 | 3.25 | 1.00 | 91 | 151 | 0.000 | 0.000 | 293 |
| search | team | 0.665 | 0.785 | 1 | 0 | 0 | 1/8 | 7.38 | 0.12 | 144 | 448 | 0.228 | 0.000 | 1,670 |

资源：三个场景的 party 资源池都从 1.00 开始；本基准没有可消耗的 party 资源，因此 `resource_ratio=1.000`，不能据此推断资源策略质量。`avg party HP` 是终局剩余 HP 的简单平均。

## 5. 解读与 P8 决定

### 5.1 改进

- **目标场景**：Utility 与 Search 都在 8/8 次运行完成 lever 目标；legacy 0/8。这个结果与 P6 的合成对照一致。
- **可复现性**：固定 state、seed 与配置下，三个策略的胜负、回合数和目标结果可重复；search 的节点与排序在相同输入下稳定。
- **性能预算**：search 的 P50 为 91–144 µs、P95 为 151–448 µs，低于 #179 的 100 ms 软上限；每决策节点数受 384 限制。搜索在现有引擎上不需要方案 B 的 `engine.rs` preview。

### 5.2 回归与不采纳默认切换的理由

- **团队战胜率**：同一 2v2 场景，legacy 7/8，Utility 1/8，Search 1/8。Utility/Search 的终局 party HP 也更低（1.25 / 0.12 vs 18.25）。这是明确的质量回归，不能用目标场景的改进抵消。
- **Action 率**：团队场景 party legal action 率 legacy 0.864、Utility 0.813、Search 0.785；目标场景三者的 party action 率都低（0.167 或 0），因为目标场景只需要一次交互，随后是无战斗的回合。该数字不能单独当作“动作合法性”排名。
- **搜索没有质量增益**：Search 与 Utility 在上述三种场景的胜负和目标结果完全相同，却增加决策延迟和节点；因此“默认启用 Search”没有证据支持。
- **Fallback**：party-only fallback 为 0；总 fallback 主要来自目标场景的孤立敌方 actor（没有可执行候选）与团队场景的敌方 actor。总 fallback 高不等于 party planner 失败，但说明“所有 actor 一律启用 Search”会把大量无候选决策交给 fallback 路径。

### 5.3 P8 默认值与启用条件

**默认保持：**

- `Ai::prepare` / CLI：legacy heuristic。
- `Ai::prepare_with(state, AiStrategy::Utility)`：显式 Utility opt-in。
- `Ai::prepare_with_search(state, SearchConfig)` / `AiStrategy::Search`：显式 Search opt-in。
- `SearchConfig::default()`：`max_depth=2`、`top_k=8`、`opponent_top_k=4`、`samples_per_candidate=2`、`node_budget=384`、`soft_time_limit_ms=100`、`hard_time_limit_ms=300`、固定 `seed=0x5eed0092`。

**为什么没有改 `sim.rs` 默认值：** 本任务允许在 `sim.rs` 设置模拟默认策略，但 `sim.rs` 的 `play`/`monte_carlo` 接收调用方传入的 `Ai`，而 CLI 在禁改路径 `cli.rs` 中直接使用 `Ai::prepare`。在证据显示 Utility/Search 会让团队战胜率从 7/8 降到 1/8 的情况下，改任何“模拟默认”只会制造一个 CLI 不使用的第二默认，不能改善玩家行为，反而增加误解。只有在未来任务能以真实内容证明质量不回退、并同时更新 CLI/sim 默认时，才应改变默认。当前结论是：**legacy 默认，Utility/Search opt-in**。

**未来启用 Utility/Search 的最低条件（全部满足才建议另行评估）：**

1. 在真实或至少非木桩的合成遭遇中，目标完成率不下降，且团队胜率/终局 HP 不低于 legacy 的统计区间下限。
2. party-only fallback ≤ 10%；总 fallback 的原因可解释，且不会掩盖无候选 actor。
3. 单决策 P95 不超过硬上限（300 ms）；节点数不超过配置预算。
4. 固定 seed/配置下排序与统计可复现。
5. 有明确的禁用/回退开关和真实内容覆盖说明；不得把合成 fixture 当成 SRD 覆盖。

**禁用与回退：**

- 禁用 Search：改用 `Ai::prepare_with(state, AiStrategy::Utility)` 或 `Ai::prepare(state)`。
- 禁用 Utility：使用 `Ai::prepare(state)`。
- Search 内部失败、预算耗尽、时间上限、无候选或未支持效果 → 回退 Utility；Utility 找不到可执行候选或执行失败 → 回退 legacy。两条路径都由现有测试覆盖（`tests/ai_search.rs`、`tests/ai_tactics.rs`）。
- 旧存档不因策略选择而迁移；`legacy` 保存的 2D 位置、无 pose/vertical 的状态继续走旧路径（见 `tests/spatial_regression.rs` 的 legacy 行）。

## 6. 空间回归矩阵

`tests/spatial_regression.rs` 的 10 个确定性测试全部通过（debug test profile）。矩阵与规则 ID 的对应：

| 行 | 覆盖 | 规则/契约 | 断言 |
|---|---|---|---|
| 1 | 体型 / footprint | O-01, O-06 | Large 目标的近边缘在 5 ft 内时命中；anchor cell 距离 10 ft 不影响 |
| 2 | 3D 高度 | S-06, R-04 | 10 ft 高度差在 5 ft reach 外；5 ft 在内 |
| 3 | 掩护 / 视线 | C-01..C-07 | 3 ft 低墙给半掩体但不挡视线；10 ft 墙拒绝攻击并保持目标 HP |
| 4 | AoE 球体 / 目标体积 | A-01, A-08 | 5 ft 球体命中地面目标，不命中 10 ft 高飞行目标 |
| 5 | AoE 锥形 | A-02, A-07 | 前方目标命中，施法者身后目标不命中 |
| 6 | 墙角 / 窄道 | S-04, M-06, M-07, O-06 | 对角穿墙角被拒绝；大型 footprint 按 2014/2024 squeeze 政策通过；`Squeeze::Forbid` 拒绝 |
| 7 | 飞行 / 坠落 | V-02..V-06 | 15 ft 天花板把 5 ft 生物停在 10 ft；10 ft 坠落应用伤害、落地与 R-09 Prone |
| 8 | 借机攻击 | R-02..R-04 | 地面相邻观察者触发 OA；同位置 10 ft 高观察者不触发 |
| 9 | 旧存档 | 契约 §11 | 无 schema/pose/vertical/spatial 字段的 JSON 可加载、可重存；legacy anchor-cell reach 仍拒绝 10 ft |
| 10 | profile 开关 | S-03 | 默认 2024 为 5-ft 对角；显式 5-10-5 profile 才改变成本 |

运行：

```bash
CARGO_BUILD_JOBS=2 cargo test --manifest-path tools/combat-sim/Cargo.toml --test spatial_regression
```

本次结果：`10 passed; 0 failed`（退出码 `0`）。

## 7. 真实 SRD 覆盖与已知限制

### 7.1 已覆盖的真实数据（引 #178）

- 2014 怪物：7 个结构化 `hover`、21 个结构化 `burrow`；2014 法术：81 个结构化 AoE（37 sphere、26 cube、9 cylinder、6 cone、3 line）。
- 2024：`2024/monsters/srd-spatial.yaml` 覆盖 38 个 SRD 生物的 hover/burrow（17 hover、21 burrow）；`2024/spells.yaml` 有 6 个形状示例（sphere、cone、cube、cylinder、line、emanation）。
- 来源、生成命令与署名见 `docs/DATA.md`；规则差异见 `SPATIAL-RULES-MATRIX.md`。

### 7.2 已知限制

- **合成 vs 真实**：本文的 10 个空间回归和 3 个 AI 场景全部是通用合成数据。它们证明矩阵与 planner 接口在该 fixture 上成立，**不证明**每个 SRD 生物/法术的 shape、hover/burrow 或 AI 行为都已被覆盖。
- **2024 形状覆盖是代表性而非穷尽**：只有 6 个 SRD 法术有结构化 shape；上游 5e-database 没有 2024 `area_of_effect`，不能声称全量覆盖。
- **缺口**：2014 的 wall 类 line 法术（Blade Barrier、Prismatic Wall、Wall of Fire、Wall of Thorns、Wind Wall）没有映射；`AreaSpec`/`AreaSpecV1` 只有 cylinder 半径，没有 cylinder 高度；2024 完整怪物包尚未用新生成器重生成。以上是 #178 已记录的 follow-up，不在 P8 修补。
- **交互的动作经济**：本基准的目标场景通过 `engine::interact` 完成 lever；引擎当前不把 scene interaction 记为 action。P8 因此把 `objectives` 当作目标完成指标，而不是动作经济证据；没有在 `engine.rs`/`ai.rs` 中修改该行为（禁改路径）。
- **AI 场景规模**：目标场景是 1 个 party actor + 1 个孤立敌方 actor；团队是 2v2；近战是 1v1。没有真实怪物、法术、条件链、legendary/lair action 或大型战场。不能把 7/8 之类的数字外推为真实战役胜率。
- **单机数字**：P50/P95 只代表本机、本 commit；P95 对负载敏感。CI 不跑该基准，也不把墙钟写成断言。
- **未做 MCTS**：#179 的结论仍然有效；P8 没有新增 rollout/MCTS 实现或质量声明。
- **没有真实内容上的 AI 质量结论**：需要后续任务用 #178 的真实 SRD 数据重复本基准，并优先覆盖非木桩敌人、真实法术 shape 与真实怪物 AI。

## 8. 复现清单

1. 检出本 PR head（或至少包含 `tests/spatial_regression.rs`、`tests/ai_quality.rs` 的 commit）。
2. 运行 §2 的 release 基准命令；确认输出含 `profile=release`、`commit=`、固定 seed、三个 strategy/scenario 行。
3. 重复 2–3 次；P50 应在本文的 ±10–15% 内，P95 受负载影响更大。若偏差更大，先确认没有其他重负载进程，并记录实际硬件/OS。
4. 普通 CI 命令只跑小测试，不执行 `#[ignore]` 基准；确认不会有 wall-clock 断言。
5. 任何真实内容扩展都应另开任务，并更新 `docs/DATA.md` 与本文的覆盖/缺口段，不得把合成 fixture 结果写成 SRD 覆盖。
