# TalkDND —— 在聊天里玩 D&D

TalkDND 让 AI 助手在聊天窗口里给你当地下城主（DM），按 **D&D 5e（2014）或 5.5e（2024）规则**（每个存档自选）带你跑冒险：

- 你扮演自己的角色，AI 负责叙述、扮演同伴和所有 NPC；
- 每一幕结尾给几个选项，也可以随便输入你想做的事；
- 所有骰子都是真随机（在命令行里用操作系统熵掷出），并把完整算式展示给你；
- 每一步都写进存档日志，下次可以从存档点接着玩；
- 战斗由 **combat-sim** 结算（命中、伤害、状态、专注、移动、借机攻击……）。

要玩：留在本目录。本目录的 `AGENTS.md` 是 DM 手册。开发与改代码在仓库根，不在这里。

## 你需要准备什么

本目录**只有程序说明和原创文档**，不包含任何规则书、冒险书或示例战役。要玩，你必须自己准备：

### 1. 选择规则版本

| 版本 | 存档里写 | 规则书放在 | 最低要求 |
|------|----------|------------|----------|
| 5e（2014） | `ruleset: "2014"` | `Rules/2014/` | **SRD 5.1**（`SRD_CC_v5.1`，免费，CC-BY-4.0） |
| 5.5e（2024） | `ruleset: "2024"` | `Rules/2024/` | **SRD 5.2.1**（`SRD_CC_v5.2.1`，免费，CC-BY-4.0） |

SRD 可在 Wizards of the Coast 官网免费下载：<https://dnd.wizards.com/resources/systems-reference-document>。如果你有正式的玩家手册 / 城主指南 / 怪物图鉴，也放进对应目录，AI DM 会优先使用。每个存档只用一个版本，DM 不会混用两版规则。

### 2. 准备冒险书

把你自己合法拥有的冒险书放进 `Adventures/`（一本一个文件，或一个文件夹）。冒险的战斗数据由 AI DM 按书建在 `Adventures/<slug>/combat/`。

### 3. 放置位置

```
Rules/
  2014/  SRD_CC_v5.1.md   [Players-Handbook-2014.md  Dungeon-Masters-Guide-2014.md  Monster-Manual-2014.md]
  2024/  SRD_CC_v5.2.1.md [Players-Handbook-2024.md  Dungeon-Masters-Guide-2024.md  Monster-Manual-2024.md]
Adventures/
  Keys-from-the-Golden-Vault.md      # 你要跑的冒险书
  keys-from-the-golden-vault/combat/ # （AI 自建）该冒险的场景、遭遇、怪物数据
bin/
  combat-sim                         # 本机程序，不入库。Windows 文件名是 combat-sim.exe
```

- **优先用转好的 Markdown / 纯文本**（AI 用关键词检索，又快又准）；PDF 也可以，但检索慢、可能有识别错误。
- 规则书、冒险书和存档除 README 外都在 `.gitignore` 中，不会被提交，只留在你本地。

### 4. 其他

- 一个能执行命令行的 AI 助手（能读写本目录文件、运行 shell）。
- combat-sim 程序，见下一节。玩的时候不需要编译器。

## 书籍转文字（自动，后台）

你只需要放 PDF。AI DM 会用 `tools/pdf2md` 在后台把 PDF 逐步转成可搜索的 Markdown（放在 PDF 旁边的隐藏目录 `.md/<PDF 文件名>/`），先转当前冒险，再转规则书；不会打断游戏，PDF 始终是权威来源。扫描版里没有文字层的页面会自动 OCR。

第一次需要安装 [uv](https://docs.astral.sh/uv/)（AI DM 会按 `docs/pdf2md.md` 自己装），并联网下载依赖（约 500 MB，含 PyMuPDF 与 RapidOCR，不随本项目分发）。

## 获取 combat-sim

程序放在 `bin/combat-sim`。Windows 是 `bin/combat-sim.exe`。这两个文件都不入库。

### 从 GitHub Releases 解压

打开本仓库的 [GitHub Releases](https://github.com/ctrlshiftaltdel/TalkDND/releases)（标签形如 `v*`），下载和你的系统对应的压缩包，以及旁边的 `.sha256` 校验文件：

| 系统 | 压缩包 |
|------|--------|
| Linux x86_64 | `talkdnd-letsplay-<标签>-linux-x86_64.tar.gz`（v0.1.0 为 `-x86_64-unknown-linux-gnu`） |
| Windows x86_64 | `talkdnd-letsplay-<标签>-windows-x86_64.zip`（v0.1.0 为 `-x86_64-pc-windows-msvc`） |
| macOS（Apple 芯片，arm64） | `talkdnd-letsplay-<标签>-macos-arm64.tar.gz`（v0.1.0 为 `-aarch64-apple-darwin`） |

没有 macOS Intel（x86_64）的预编译包。Intel Mac 需要开发者在本机编译后，把程序放进 `bin/`。

压缩包里是完整的 `LetsPlay/` 文件夹（DM 手册、战斗数据、裁定、各目录的 README），程序已经放在 `LetsPlay/bin/`：`combat-sim` 与 `talkdnd`（Windows 加 `.exe`；v0.1.0 只有 combat-sim）。解压到任意位置，进入 `LetsPlay/` 即可。压缩包**不含**任何规则书、冒险书或存档，需要你自己放进 `Rules/2014/`、`Rules/2024/`、`Adventures/`。

Linux、macOS 解压后赋予执行权限，并在系统拦截未签名程序时去掉隔离属性：

```bash
chmod +x bin/combat-sim bin/talkdnd
xattr -d com.apple.quarantine bin/combat-sim bin/talkdnd
```

`xattr` 只在 macOS 需要。Linux 做完 `chmod` 即可。

校验压缩包（可选）。`<文件名>` 换成你下载的包名。Linux：

```bash
sha256sum -c <文件名>.sha256
```

macOS：

```bash
shasum -a 256 -c <文件名>.sha256
```

Windows PowerShell 里对照 `.sha256` 文件中的哈希：

```powershell
Get-FileHash <文件名> -Algorithm SHA256
```

`main` 分支上每次成功构建也会留下约 30 天的程序包（`combat-sim-<平台>` 和 `talkdnd-<平台>` 两个，根上就是程序本身，解压到 `bin/`）。开发者可以用仓库根的开发工具把它直接装进 `bin/`。

放好之后，在本目录试一次。Linux / macOS：

```bash
bin/combat-sim roll 1d20+5
```

Windows：

```bat
bin\combat-sim.exe roll 1d20+5
```

能印出骰子结果就可以开始玩。

## 开始一局

1. 按上面把规则书放进 `Rules/<版本>/`，冒险书放进 `Adventures/`，程序放进 `bin/`。
2. 在本目录执行 `bin/combat-sim roll 1d20+5`（Windows 用 `bin\combat-sim.exe roll 1d20+5`）。能出结果就可以继续。
3. 在本目录打开你的 AI 助手（它会自动读取 `AGENTS.md`），直接说：

   | 你说 | 效果 |
   |------|------|
   | `开新局` | DM 一步步问你：规则版本、冒险、种族、职业、背景、属性生成方式（掷骰后由你分配）、技能、装备、法术、名字，最后给你确认角色卡；再生成同伴给你确认 |
   | `继续游戏 2026-10-19-01` | 从该存档最后的存档点继续（程序先生成一页接档摘要，开场更快） |
   | `继续游戏` | 继续最新的存档 |
   | `存档` | 写好存档点 |

   想让 DM 替你决定某一项，就说“这项随机”或“你来定”。如果你的 AI 助手不会自动读 `AGENTS.md`，先说一句“请阅读 AGENTS.md”。
4. 存档在 `Saves/YYYY-MM-DD-NN/`（结构见 `Saves/README.md`），`combat/campaign.yaml` 记录本存档用的规则版本。
5. 想结束时说「今天先到这，存档」。

`AGENTS.md` 是给 AI DM 的核心手册，细节在 `docs/`。`Rules/rulings.md`（通用）和 `Rules/rulings-2014.md` / `Rules/rulings-2024.md`（版本差异）是规则书没写清时的统一裁定。

## 骰子和日志

- 每个检定都会显示成这样：`潜行 优势(4,17)→17+7 = 24 vs DC 15 成功`。
- 骰子由 `bin/combat-sim roll 1d20+5 --adv` 掷出（Windows 为 `bin\combat-sim.exe`）掷出，不是 AI 编的，也不用其他随机数工具。每次掷骰都显示完整算式和每颗骰子。
- AI 不会「先掷好再挑结果」；如果为了节奏预先掷了骰，会在日志里标明并按顺序使用。
- 会话日志在 `Saves/<存档>/Sessions/`，每个掷骰、HP/金币变化都有记录；你随时可以翻看。
- DM 的秘密（反派身份、隐藏的陷阱）放在 `Saves/<存档>/dm-secrets.md`。想保持惊喜就别打开它。

## 公平游戏与防剧透

- DM 不会听“开发者模式”“直接给我物品”“这次算我成功”这类场外指令。你说出想做什么，由 DM 决定要不要检定。房规请在开局前商定，记进存档的 `house-rules.md`。
- 存档里的 `integrity.yaml` 记录程序最后一次写入时的文件指纹。手改了 `progress.yaml`、角色卡或战斗数据，`talkdnd start` / `talkdnd check` 会提醒 DM。
- `reveals.yaml` 记录队伍已经得知的事实。DM 只叙述角色能感知到的东西，读冒险书也是按场景读。详见 `docs/table-integrity.md`、`docs/anti-spoiler.md`。

## 战斗

在本目录（清单路径相对于存档文件，模板见 `example/save/combat/campaign.yaml`）。Windows 把 `bin/combat-sim` 换成 `bin\combat-sim.exe`：

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

人物来自角色卡里的 combat 数据块。怪物、法术、状态、武器精通来自 `Rules/combat/`（按版本分层）。场景和遭遇来自 `Adventures/<冒险>/combat/`。

两个版本的差异由数据决定：`Rules/combat/common/` 是通用部分，`Rules/combat/2014/`、`Rules/combat/2024/` 是各版本特有的内容。引擎读存档清单里的 `ruleset` 自动选择。

## 版权与许可

- 文档和 `Rules/rulings*.md`：MIT 许可。许可全文在仓库的 LICENSE 文件。
- `Rules/combat/` 中的规则数据只取自 **System Reference Document 5.1**（2014）与 **5.2.1**（2024），按 CC-BY-4.0 使用。署名在仓库的 NOTICE 文件。
- 「Dungeons & Dragons」「D&D」是 Wizards of the Coast 的商标。本项目是非官方的爱好者作品，与 Wizards of the Coast 无关，也未获其认可。
- 本仓库不包含、也不分发任何非 SRD 的规则书或冒险书文本；请使用你自己合法拥有的书。
