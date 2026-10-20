# TalkDND —— 在聊天里玩 D&D

TalkDND 让 AI 助手在聊天窗口里给你当地下城主（DM），按 **D&D 5e（2014）或 5.5e（2024）规则**（每个存档自选）带你跑冒险：
- 你扮演自己的角色，AI 负责叙述、扮演同伴和所有 NPC；
- 每一幕结尾给几个选项，也可以随便输入你想做的事；
- 所有骰子都是真随机（在命令行里用操作系统熵掷出），并把完整算式展示给你；
- 每一步都写进存档日志，下次可以从存档点接着玩；
- 战斗由自带的后台引擎 **combat-sim** 结算（命中、伤害、状态、专注、移动、借机攻击……）。

要玩：把规则书放进 `LetsPlay/Rules/<版本>/`，把冒险书放进 `LetsPlay/Adventures/`，然后 `cd LetsPlay`，在那个目录启动你的 Agent。开发与改代码请看仓库根目录的 `AGENTS.md`（那是开发协作契约，不是 DM 手册）。DM 手册是 `LetsPlay/AGENTS.md`。本仓库的程序和工具用 Rust 编写；开发工具在 `tools/talkdnd-dev`。

## 你需要准备什么

本仓库**只有程序和原创文档**，不包含任何规则书、冒险书或示例战役。要玩，你必须自己准备：

### 1. 选择规则版本

| 版本 | 存档里写 | 规则书放在 | 最低要求 |
|------|----------|------------|----------|
| 5e（2014） | `ruleset: "2014"` | `LetsPlay/Rules/2014/` | **SRD 5.1**（`SRD_CC_v5.1`，免费，CC-BY-4.0） |
| 5.5e（2024） | `ruleset: "2024"` | `LetsPlay/Rules/2024/` | **SRD 5.2.1**（`SRD_CC_v5.2.1`，免费，CC-BY-4.0） |

SRD 可在 Wizards of the Coast 官网免费下载：<https://dnd.wizards.com/resources/systems-reference-document>。如果你有正式的玩家手册 / 城主指南 / 怪物图鉴，也放进对应目录，AI DM 会优先使用。每个存档只用一个版本，DM 不会混用两版规则。

### 2. 准备冒险书

把你自己合法拥有的冒险书放进 `LetsPlay/Adventures/`（一本一个文件，或一个文件夹）。冒险的战斗数据由 AI DM 按书建在 `LetsPlay/Adventures/<slug>/combat/`。

### 3. 放置位置

```
LetsPlay/
  Rules/
    2014/  SRD_CC_v5.1.md   [Players-Handbook-2014.md  Dungeon-Masters-Guide-2014.md  Monster-Manual-2014.md]
    2024/  SRD_CC_v5.2.1.md [Players-Handbook-2024.md  Dungeon-Masters-Guide-2024.md  Monster-Manual-2024.md]
  Adventures/
    Keys-from-the-Golden-Vault.md      # 你要跑的冒险书
    keys-from-the-golden-vault/combat/ # （AI 自建）该冒险的场景、遭遇、怪物数据
```

- **优先用转好的 Markdown / 纯文本**（AI 用关键词检索，又快又准）；PDF 也可以，但检索慢、可能有识别错误。
- 这些目录除 README 外都在 `.gitignore` 中，不会被提交，只留在你本地。

### 4. 其他

- 一个能执行命令行的 AI 助手（能读写本目录文件、运行 shell）。
- Rust 工具链（编译 `tools/combat-sim` 与 `tools/talkdnd-dev`）：<https://rustup.rs>。
- Python 3（可选，用于掷骰的备用方式）。

## 开始一局

1. 克隆仓库，按上面把规则书放进 `LetsPlay/Rules/<版本>/`，冒险书放进 `LetsPlay/Adventures/`。
2. 在仓库根目录编译战斗引擎：
   ```bash
   cd tools/combat-sim && cargo build --release && cd ../..
   LetsPlay/bin/combat-sim roll 1d20+5      # 能出结果就说明编译好了
   ```
3. `cd LetsPlay`，在这个目录打开你的 AI 助手，对它说：
   > 请阅读 AGENTS.md，担任 DM，用 2024 规则和《(冒险名)》开一局新游戏。我的角色是……

   或者继续已有存档：
   > 请阅读 AGENTS.md，从 Saves/<存档名> 最后的存档点继续。
4. AI 会建立 `Saves/YYYY-MM-DD-NN/` 存档（结构见 `Saves/README.md`，仓库路径 `LetsPlay/Saves/`），其中 `combat/campaign.yaml` 记录本存档用的规则版本：
   ```yaml
   ruleset: "2024"   # 或 "2014"
   ```
5. 想结束时说「今天先到这，存档」，AI 会写好存档点。

`LetsPlay/AGENTS.md` 是给 AI DM 的完整工作手册；`LetsPlay/Rules/rulings.md`（通用）和 `LetsPlay/Rules/rulings-2014.md` / `LetsPlay/Rules/rulings-2024.md`（版本差异）是规则书没写清时的统一裁定。

## 骰子和日志

- 每个检定都会显示成这样：`潜行 优势(4,17)→17+7 = 24 vs DC 15 成功`。
- 骰子由 `LetsPlay/bin/combat-sim roll 1d20+5 --adv`（进入 `LetsPlay/` 后即 `bin/combat-sim`）或 Python `random.SystemRandom()` 掷出，不是 AI 编的。
- AI 不会「先掷好再挑结果」；如果为了节奏预先掷了骰，会在日志里标明并按顺序使用。
- 会话日志在 `LetsPlay/Saves/<存档>/Sessions/`，每个掷骰、HP/金币变化都有记录；你随时可以翻看。
- DM 的秘密（反派身份、隐藏的陷阱）放在 `LetsPlay/Saves/<存档>/dm-secrets.md`。想保持惊喜就别打开它 😉。

## 战斗引擎 combat-sim

`tools/combat-sim/` 是一个无界面、数据驱动、与具体战役无关的 Rust 引擎：
- 人物来自角色卡里的 ```` ```combat yaml ```` 数据块，怪物、法术、状态、武器精通来自 `LetsPlay/Rules/combat/`（按版本分层），场景和遭遇来自 `LetsPlay/Adventures/<冒险>/combat/`；程序里不写死任何战役内容。
- 空间以**区域图**为主（区域之间的距离、掩护、高度、门锁、攀爬），重要战斗可以用 5 尺**网格**。
- 支持逐步结算（DM 一条条命令推进）和 Monte Carlo 批量模拟（预估遭遇难度）。

两个版本的差异由数据决定：`LetsPlay/Rules/combat/common/` 是通用部分，`LetsPlay/Rules/combat/2014/`、`LetsPlay/Rules/combat/2024/` 是各版本特有的状态、法术、特性、武器精通（仅 2024）和突袭规则（2014 失去首回合 / 2024 先攻劣势）。引擎读存档清单里的 `ruleset` 自动选择。

在 `LetsPlay/` 里（清单路径相对于存档文件，`../../../Rules/combat` 仍然指向 `LetsPlay/Rules/combat`）：

```bash
CS=bin/combat-sim
M=Saves/<存档>/combat/campaign.yaml
$CS --campaign $M content check
$CS --campaign $M --state fight new --encounter <遭遇id>
$CS --campaign $M --state fight init --surprised "Guard 1"
$CS --campaign $M --state fight attack <角色> "Guard 2" longsword
$CS --campaign $M simulate --encounter <遭遇id> --runs 1000
$CS roll 1d20+5 --adv --dc 15
```

完整说明见 `tools/combat-sim/README.md`、`tools/combat-sim/docs/`。

## 版权与许可

- 代码（`tools/`）、文档、`LetsPlay/Rules/rulings*.md`：MIT 许可，见 `LICENSE`。
- `LetsPlay/Rules/combat/` 中的规则数据（状态、武器精通、法术、职业特性、怪物）只取自 **System Reference Document 5.1**（2014）与 **5.2.1**（2024），按 CC-BY-4.0 使用，署名见 `NOTICE`。
- 「Dungeons & Dragons」「D&D」是 Wizards of the Coast 的商标。本项目是非官方的爱好者作品，与 Wizards of the Coast 无关，也未获其认可。
- 本仓库不包含、也不分发任何非 SRD 的规则书或冒险书文本；请使用你自己合法拥有的书。
