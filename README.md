# TalkDND —— 在聊天里玩 D&D

TalkDND 让 AI 助手在聊天窗口里给你当地下城主（DM），按 **D&D 第五版 2024 规则**带你跑官方或自制冒险：
- 你扮演自己的角色，AI 负责叙述、扮演同伴和所有 NPC；
- 每一幕结尾给几个选项，也可以随便输入你想做的事；
- 所有骰子都是真随机（在命令行里用操作系统熵掷出），并把完整算式展示给你；
- 每一步都写进存档日志，下次可以从存档点接着玩；
- 战斗由自带的后台引擎 **combat-sim** 结算（命中、伤害、状态、专注、移动、借机攻击……）。

## 你需要准备什么

本仓库**不包含**任何受版权保护的规则书或冒险书内容，你需要使用自己**合法拥有**的书，转成 Markdown 文本后放到下面的位置：

```
Rules/
  Players-Handbook-2024.md        # 玩家手册 2024
  Dungeon-Masters-Guide-2024.md   # 城主指南 2024
  Monster-Manual-2024.md          # 怪物图鉴 2024
Adventures/
  Keys-from-the-Golden-Vault.md   # 你要跑的冒险书（文件名随意，建议用英文书名）
  keys-from-the-golden-vault/     # （可选）该冒险的战斗数据，让 AI 按书自建
    combat/
      scenes/*.yaml
      encounters.yaml
      creatures.yaml
```

这些文件已在 `.gitignore` 中排除，只留在你本地。

另外需要：
- 一个能执行命令行的 AI 助手（能读写本目录文件、运行 shell）。
- Rust 工具链（编译 combat-sim）：<https://rustup.rs>。
- Python 3（可选，用于掷骰的备用方式）。

## 开始一局

1. 克隆仓库，放好规则书和冒险书。
2. 编译战斗引擎：
   ```bash
   cd tools/combat-sim && cargo build --release && cd ../..
   tools/bin/combat-sim --campaign examples/demo-campaign/save/combat/campaign.yaml content check
   ```
3. 在本目录打开你的 AI 助手，对它说：
   > 请阅读 AGENTS.md，担任 DM，用《(冒险名)》开一局新游戏。我的角色是……
   
   或者继续已有存档：
   > 请阅读 AGENTS.md，从 Saves/2026-10-19-01 最后的存档点继续。
4. AI 会建立 `Saves/YYYY-MM-DD-NN/` 存档：角色卡、同伴、会话日志、战斗清单。
5. 想结束时说「今天先到这，存档」，AI 会写好存档点。

`AGENTS.md` 是给 AI DM 的完整工作手册（桌规、存档格式、战斗引擎用法、交接清单）；`Rules/rulings.md` 是规则书没写清时的统一裁定。

## 骰子和日志

- 每个检定都会显示成这样：`潜行 优势(4,17)→17+7 = 24 vs DC 15 成功`。
- 骰子由 `tools/bin/combat-sim roll 1d20+5 --adv` 或 Python `random.SystemRandom()` 掷出，不是 AI 编的。
- AI 不会「先掷好再挑结果」；如果为了节奏预先掷了骰，会在日志里标明并按顺序使用。
- 会话日志在 `Saves/<存档>/Sessions/`，每个掷骰、HP/金币变化都有记录；你随时可以翻看。
- DM 的秘密（反派身份、隐藏的陷阱）放在 `Saves/<存档>/dm-secrets.md`。想保持惊喜就别打开它 😉。

## 战斗引擎 combat-sim

`tools/combat-sim/` 是一个无界面、数据驱动、与具体战役无关的 Rust 引擎：
- 人物来自角色卡里的 ```` ```combat yaml ```` 数据块，怪物、法术、状态、武器精通来自 `Rules/combat/`，场景和遭遇来自 `Adventures/<冒险>/combat/`；程序里不写死任何战役内容。
- 空间以**区域图**为主（区域之间的距离、掩护、高度、门锁、攀爬），重要战斗可以用 5 尺**网格**。
- 支持逐步结算（DM 一条条命令推进）和 Monte Carlo 批量模拟（预估遭遇难度）。

```bash
CS=tools/bin/combat-sim
M=examples/demo-campaign/save/combat/campaign.yaml
$CS --campaign $M --state demo new --encounter watchtower-ambush
$CS --campaign $M --state demo init
$CS --campaign $M --state demo attack Aria "Tower Warden 1" longsword --nonlethal
$CS --campaign $M --state demo next
$CS --campaign $M simulate --encounter watchtower-ambush --runs 1000
$CS roll 1d20+5 --adv --dc 15
```

完整说明见 `tools/combat-sim/README.md`、`tools/combat-sim/docs/`。

## 示例战役

`examples/demo-campaign/` 是一个**原创**小示例（旧瞭望塔里的两名守卫），角色和场景都是原创，怪物用 SRD 5.2 的 Guard。可以直接用来测试引擎，也可以照它的结构建立你自己的存档。

## 版权与许可

- 代码（`tools/`）、文档、`Rules/rulings.md`、示例战役：MIT 许可，见 `LICENSE`。
- `Rules/combat/` 中的规则数据（状态、武器精通、法术、职业特性、怪物）只取自 **System Reference Document 5.2**，按 CC-BY-4.0 使用，署名见 `NOTICE`。
- 「Dungeons & Dragons」「D&D」是 Wizards of the Coast 的商标。本项目是非官方的爱好者作品，与 Wizards of the Coast 无关，也未获其认可。
- 本仓库不包含、也不分发任何非 SRD 的规则书或冒险书文本；请使用你自己合法拥有的书。
