# combat-sim

无界面的 D&D 5e（2024 规则）战斗工具，供 TalkDND 文字团跑团使用。
v2 引擎是**数据驱动**的：规则引擎只懂通用 5e 规则，职业能力、法术、怪物、
武器专精、状态和场景都来自战役内容文件。

## 构建

```bash
cargo build --release
```

生成的可执行文件在 `target/release/combat-sim`。检查：

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
```

## 内容来源

内容按顺序加载，后加载的同 id 条目覆盖先前的：

1. `--lib DIR`：通用库（可重复）
2. `--adventure DIR`：冒险内容（可重复）
3. `--chars PATH`：角色卡/角色目录（可重复）
4. `--campaign FILE`：战役清单（也可用 `COMBAT_SIM_CAMPAIGN`）

清单示例：

```yaml
name: TalkDND save
library: [../../../Rules/combat]
adventures: [../../../Adventures/keys-from-the-golden-vault/combat]
characters: [../Characters, ../NPCs]
state_dir: .
```

支持 YAML/JSON 文件，以及 Markdown 中的 ` ```combat yaml ` / ` ```combat json `
代码块。所有内容 id 都是小写连字符字符串。

## 状态文件

`new` 会把当前用到的内容快照、场景和战斗单位一起写进状态文件。之后的
`init/status/attack/...` 只需要 `--state`，不需要 `--campaign`。

`--state NAME` 不含 `/` 时：

- `new` 解析到清单的 `state_dir/NAME.state.json`；
- 其他命令优先用 `--campaign`/环境的清单 `state_dir`，否则用
  `./NAME.state.json`；
- 含 `/` 的路径按字面使用。

## 常用命令

```bash
combat-sim --campaign campaign.yaml new \
  --encounter v12-satyr-and-guards --party will,rina,milo,sera \
  --state /tmp/fight.state.json
combat-sim --state /tmp/fight.state.json init
combat-sim --state /tmp/fight.state.json status [--json]
combat-sim --state /tmp/fight.state.json attack will "guard 1" longsword
combat-sim --state /tmp/fight.state.json cast milo magic-missile "guard 1"
combat-sim --state /tmp/fight.state.json condition add rina hidden --rounds 2
combat-sim --state /tmp/fight.state.json next
combat-sim --state /tmp/fight.state.json log --tail 20
```

其他命令：`damage`、`heal`、`save`、`check`、`roll`、`use`、`move`、
`deathsave`、`rest short|long <name|all>`、`activate <name>`、
`condition remove`、`content list|show|check`。`simulate` 在 v2 step 4 前
只打印 `not yet (v2 step 4)`。

聚合步进命令不会强制当前回合归属（DM 可以随时裁定），但会检查动作、
附赠动作、每轮一次的反应和资源消耗。`next` 会重置这些标记。

## 空间

v2 step 2 支持区域图场景：区域、边距离、高度差、掩体和移动消耗。
网格场景、借机攻击、区域效果、场景物件与场景触发器在 step 3 实现；
遇到 `mode: grid` 的场景会明确报错 `grid scenes arrive in v2 step 3`。

## 测试夹具

所有引擎测试使用 `tests/fixtures/` 下的合成内容，不依赖真实战役。
真实战役内容只用于手动烟雾测试，工具不会修改它。
