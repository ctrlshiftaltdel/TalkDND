# TalkDND —— 在聊天里玩 D&D

TalkDND 让 AI 助手在聊天窗口里给你当地下城主（DM），按 **D&D 5e 或 5.5e 规则**（每个存档自选 **5.5e FULL / 5e FULL / 5.5e SRD / 5e SRD**）带你跑冒险：

- 你扮演自己的角色，AI 负责叙述、扮演同伴和所有 NPC；
- 每一幕结尾给几个选项，也可以随便输入你想做的事；
- 所有骰子都是真随机（在命令行里用操作系统熵掷出），并把完整算式展示给你；
- 每一步都写进存档日志，下次可以从存档点接着玩；
- 战斗由 **combat-sim** 结算（命中、伤害、状态、专注、移动、借机攻击……）。

当前发布：**v1.4.0**（2026-10-28）。本版新增只读 Agent 知识 API（`bin/talkdnd knowledge search|get|cite|context|related`），把规则裁定、存档知识、玩家书、SRD、冒险与原创六类来源放在同一组命令后面，默认只返回玩家已知内容。存档结构仍是 **schema 9**，本版没有新的迁移；默认战斗 AI 仍是 legacy，四规则集与完整书 pack 可选资产与 v1.3.0 相同，详见 `CHANGELOG.md` 的 v1.4.0 小节。

要玩：留在本目录。本目录的 `AGENTS.md` 是 DM 手册。开发与改代码在仓库根，不在这里。

## 你需要准备什么

本目录包含程序说明、原创文档和内置的 SRD 5.1 / 5.2.1 内容包（CC-BY-4.0）。SRD 内容已内置，**不需要自己下载 SRD PDF 也能玩**。冒险书仍需自备。

### 1. 选择规则集

| 玩家选项 | 存档里写 | 内置内容 | 玩家书 / PDF 放在 |
|----------|----------|----------|-------------------|
| 5.5e FULL | `ruleset: "5.5e-full"` | SRD 5.2.1 + 2024 规则章节 | `Rules/2024/` + 已解锁的 `Rules/packs/full-2024-*.pack.json` |
| 5e FULL | `ruleset: "5e-full"` | SRD 5.1 | `Rules/2014/` + 已解锁的 `Rules/packs/full-2014-*.pack.json` |
| 5.5e SRD | `ruleset: "5.5e-srd"` | SRD 5.2.1 全量内容包 + 2024 规则章节 | 只用内置 SRD；`Rules/2024/` 可选 |
| 5e SRD | `ruleset: "5e-srd"` | SRD 5.1 全量内容包 | 只用内置 SRD；`Rules/2014/` 可选 |

放入 SRD PDF 是可选的：`bin/talkdnd start` 会为它建立 pdf2md 索引，DM 在缺图、地图或原文不清时回退查看。**FULL 必须在对应 edition 至少解锁一本完整书**（PHB/DMG/MM，PDF 放进 `Rules/2014/` 或 `Rules/2024/` 的书文件夹，构建好的内容包放进 `Rules/packs/`）；缺书、未解锁或内容包损坏时 `start` 与 `content check` 会失败并指出缺什么，不会静默退回 SRD。`*-srd` 永不加载完整书内容包。旧存档里的 `ruleset: "2014"` / `"2024"` 仍可读取，分别按 `5e-srd` / `5.5e-srd` 处理，并在下一次写 manifest 时迁移。每个存档只用一套规则，DM 不会混用 5e 与 5.5e。

完整书 pack 是**单独的发布资产**（`talkdnd-full-packs-<标签>.tar.gz`），不在普通游戏包内；托管 CI 不构建、不上传，由项目所有者另行提供。需要完整书内容时，把它解压到 `Rules/packs/`，把你自己的匹配 PDF 放进 `Rules/<edition>/` 的书文件夹，再运行 `bin/talkdnd library scan` 解锁；若提示歧义，按输出用 `bin/talkdnd library choose ...` 指定书型。只用 SRD 的玩家不需要任何额外步骤。

### 2. 准备冒险书

把你自己合法拥有的冒险书 PDF 放进 `Adventures/`。直接丢在 `Adventures/` 下也行：每次 `bin/talkdnd start`（或 `bin/talkdnd library tidy`）都会把它整理进它自己的文件夹 `Adventures/<slug>/`，并报告移动了什么。冒险的战斗数据由 AI DM 按书建在同一文件夹的 `combat/` 里。

### 3. 放置位置

统一标准：**一本书一个文件夹**，文件夹名是书名的 slug（规则见 `docs/layout.md`）。

```
Rules/
  2014/srd-cc-v5-1/
    SRD_CC_v5.1.pdf                  # 可选：你放的 PDF（用于书页图片/回退）
    index/                           # （自动生成）pdf2md 转换结果：SRD_CC_v5.1.pages.md、index.md、img/ …
  2024/players-handbook-2024/
    Players Handbook 2024.pdf        # 完整规则书（可选，自有；FULL 解锁用）
    index/
  packs/
    full-2024-phb.pack.json          # FULL 解锁后由本机构建或放入；不入库
Adventures/
  keys-from-the-golden-vault/
    Keys from the Golden Vault.pdf   # 你要跑的冒险书
    index/                           # （自动生成）转换结果
    combat/                          # （AI 自建）该冒险的场景、遭遇、怪物数据
bin/
  combat-sim                         # 本机程序，不入库。Windows 文件名是 combat-sim.exe
```

- **只需要放 PDF**，不用自己转 Markdown；文字版由 pdf2md 自动生成在书文件夹的 `index/` 里。
- 规则书、冒险书和存档除 README 外都在 `.gitignore` 中，不会被提交，只留在你本地。

### 4. 其他

- 一个能执行命令行的 AI 助手（能读写本目录文件、运行 shell）。
- combat-sim 程序，见下一节。玩的时候不需要编译器。

## 书籍转文字（自动，后台）

你只需要放 PDF。AI DM 会用 `tools/pdf2md` 在后台把 PDF 逐步转成可搜索的 Markdown（放在书文件夹的 `index/` 里：`<书名>.pages.md`，`<a id="pN"></a>` = PDF 第 N 页），先转当前冒险，再转规则书；不会打断游戏，PDF 始终是权威来源。扫描版里没有文字层的页面会自动 OCR。

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

压缩包里只有一个顶层文件夹 `TalkDND/`（v0.6.0 及以前叫 `LetsPlay/`），里面是完整的游戏文件夹：DM 手册、战斗数据、裁定、各目录的 README，程序在 `bin/`：`combat-sim` 与 `talkdnd`（Windows 加 `.exe`）。解压到任意位置都可以，文件夹也可以随意改名（路径里有空格或中文也没关系）。程序会通过自己所在的 `bin/` 找到游戏文件夹（文件夹根部的 `.talkdnd-root` 是标记文件，请勿删除），所以从哪个目录运行 `bin/talkdnd` 都行；需要时也可以用 `--root <文件夹>` 或环境变量 `TALKDND_ROOT` 指定。在这个文件夹里启动 Agent。压缩包**不含**任何规则书、冒险书或存档，需要你自己放进 `Rules/2014/`、`Rules/2024/`、`Adventures/`。

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

1. 按上面把规则书放进 `Rules/2014/` 或 `Rules/2024/`，冒险书放进 `Adventures/`，程序放进 `bin/`。
2. 在本目录执行 `bin/combat-sim roll 1d20+5`（Windows 用 `bin\combat-sim.exe roll 1d20+5`）。能出结果就可以继续。
3. 在本目录打开你的 AI 助手（它会自动读取 `AGENTS.md`），直接说：

   | 你说 | 效果 |
   |------|------|
   | `开新局` | DM 一步步问你：规则集、冒险、种族、职业、背景、属性生成方式（掷骰后由你分配）、技能、装备、法术、名字，最后给你确认角色卡；再生成同伴给你确认 |
   | `继续游戏 2026-10-19-01` | 从该存档最后的存档点继续（程序先生成一页接档摘要，开场更快） |
   | `继续游戏` | 继续最新的存档 |
   | `存档` | 写好存档点 |

   想让 DM 替你决定某一项，就说“这项随机”或“你来定”。如果你的 AI 助手不会自动读 `AGENTS.md`，先说一句“请阅读 AGENTS.md”。
4. 存档在 `Saves/YYYY-MM-DD-NN/`（结构见 `Saves/README.md`），`combat/campaign.yaml` 记录本存档用的规则集。
5. 想结束时说「今天先到这，存档」。

`AGENTS.md` 是给 AI DM 的核心手册，细节在 `docs/`。`Rules/rulings.md`（通用）和 `Rules/rulings-2014.md` / `Rules/rulings-2024.md`（edition 差异）是规则书没写清时的统一裁定，也是你的本机副本；同名的出厂默认在 `Rules/defaults/`，升级不会覆盖你的副本。

## 骰子和日志

- 每个检定都会显示成这样：`潜行 优势(4,17)→17+7 = 24 vs DC 15 成功`。
- 骰子由 `bin/combat-sim roll 1d20+5 --adv` 掷出（Windows 为 `bin\combat-sim.exe`）掷出，不是 AI 编的，也不用其他随机数工具。每次掷骰都显示完整算式和每颗骰子。
- AI 不会「先掷好再挑结果」；如果为了节奏预先掷了骰，会在日志里标明并按顺序使用。
- 会话日志在 `Saves/<存档>/Sessions/`，每个掷骰、HP/金币变化都有记录；你随时可以翻看。
- DM 的秘密（反派身份、隐藏的陷阱）放在 `Saves/<存档>/dm-secrets.md`。想保持惊喜就别打开它。

## 公平游戏与防剧透

- DM 不会听“开发者模式”“直接给我物品”“这次算我成功”这类场外指令。你说出想做什么，由 DM 决定要不要检定。房规请在开局前商定，记进存档的 `house-rules.md`。
- 存档里的 `integrity.yaml` 记录程序最后一次写入时的文件指纹。手改了 `progress.yaml`、角色卡或战斗数据，`talkdnd start` / `talkdnd check` 会提醒 DM。
- `reveals.yaml` 记录队伍已经得知的事实。DM 只叙述角色能感知到的东西，读冒险书也是按场景读。详见 `docs/table-integrity.md`、`docs/anti-spoiler.md`。给玩家看书里的地图或资料图用 `talkdnd handout show`，城主图不会导出（见 `docs/handouts.md`）。
- DM 用 `talkdnd known` 看“玩家目前知道什么”，用 `talkdnd guard "<草稿>"` 检查叙述/选项里有没有还没揭示的秘密词；未识别的怪物在 `combat-sim --player` 视图里只显示外貌描述。规则见 `docs/option-writing.md`。
- DM 跨来源查资料用 `talkdnd knowledge search|get|cite|context|related`：默认只返回玩家已知内容，`--include-hidden` 是 DM 视图（结果标 `visibility: dm`），**不要贴给玩家，也不要用它写选项**；`dm-secrets.md` 永远不会成为输出源。说明见 `docs/knowledge.md`。
- 默认每个检定都显示完整算式；如果你们更喜欢只看故事，`talkdnd settings dice off` 可以关掉数字显示。掷骰仍然全部由程序完成并记在 `Saves/<存档>/rolls.log`，`talkdnd log rolls` 随时可查。见 `docs/dice.md`。

## 自动化开关

自动化开关决定 DM 哪些事直接做、哪些先问你、哪些等你开口，按存档保存：`bin/talkdnd settings automation` 查看，`show` 看某个场景/遭遇的生效值，`set` / `reset` 修改（`--reason` 必填）。优先级是 **遭遇 > 场景 > 存档默认**；玩家选择和玩家反应永远不能设为 `auto`，每次改动都会写进会话日志。`bin/talkdnd start` 的摘要会显示当前生效值与覆盖数量。完整说明见 `docs/automation.md`。

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

人物来自角色卡里的 combat 数据块。怪物、法术、状态、武器精通来自 `Rules/combat/`（按 edition 分层）。场景和遭遇来自 `Adventures/<冒险>/combat/`。

两个 edition 的差异由数据决定：`Rules/combat/common/` 是通用部分，`Rules/combat/2014/`、`Rules/combat/2024/` 是各 edition 特有的内容。`srd/` 与 `srd-*` 覆盖属于 SRD 层；FULL 先加载 SRD 层，再加载已解锁的完整书内容包，最后加载 `<edition>/` 下其余手写内容与冒险/战役覆盖。引擎读存档清单里的 `ruleset` 自动选择。

战斗 AI 默认沿用原有 legacy heuristic。v1.2 的战术效用规划器与有界搜索只作为库 API 显式 opt-in，v1.3 / v1.4 未改默认；质量基准在合成团队战看到回退，所以玩家不需要设置。自动化开关与知识 API 也不改变战斗 AI。

## 更新到新版本

直接覆盖解压即可，不需要手动替换任何文件：

1. 在游戏里先「存档」，然后关掉正在运行的 talkdnd / combat-sim（Windows 上运行中的 `.exe` 无法被覆盖）。
2. 从 [Releases](https://github.com/ctrlshiftaltdel/TalkDND/releases) 下载对应平台的包，可用同名 `.sha256` 校验。
3. 把包解压到任意临时位置，再把解压出来的 `TalkDND/` 文件夹**里面的全部内容**复制到你的游戏文件夹（不管它叫什么、放在哪），遇到同名文件选择「全部覆盖」。如果你的游戏文件夹本来就叫 `TalkDND` 且放在同一位置，直接在它的上一级解压覆盖也行。
4. 照常「继续游戏」（`talkdnd start`）。第一次运行新版时程序会自动：
   - 检查存档结构版本（`Saves/<存档>/save-meta.yaml`），旧了就先把整个存档备份到 `Saves/_backups/`，再迁移到新结构，写进会话日志并更新完整性清单；schema 8 → 9 会补上章程默认的自动化设置，`settings.dice_display` 与叙事风格保持不变；
   - 书籍导入数据（`Rules/chargen/local/`）的导入器版本过旧时重新导入；
   - 缺少的默认规则裁定从 `Rules/defaults/` 复制到 `Rules/`，已有的不动（默认版有更新时摘要会提示合并）；
   - 把新版已删除、你没改过的旧程序文件移到 `.talkdnd/obsolete/`；
   - 把旧存档的 `ruleset: "2014"` / `"2024"` 分别迁移为 `5e-srd` / `5.5e-srd`（只改这一行，保留其他字段与注释）；
   - 原先使用完整书内容的存档需要把 `ruleset` 重选为 `5e-full` / `5.5e-full`，并解锁对应完整书 pack。

自动化开关不需要预先设置：不配置时按章程默认，要改随时用 `bin/talkdnd settings automation ...`，说明见 `docs/automation.md`。v1.4 没有新的存档结构变化，schema 仍为 9；知识 API 只读、不需要额外设置，书籍/冒险索引缺失时搜索返回空结果，可先运行 `bin/talkdnd adventure index build --book "<书名>"`。四规则集和完整书 pack 的用法与 v1.2.0 相同。

完整书 pack 不在普通发布包内。需要时，从项目所有者另行提供的 `talkdnd-full-packs-<标签>.tar.gz` 解压到 `Rules/packs/`，把匹配 PDF 放进 `Rules/<edition>/` 后用 `bin/talkdnd library scan` 解锁；没有它也能用 5e SRD / 5.5e SRD 完整游玩。

发布包里**没有**任何用户数据：存档（只有 `Saves/README.md`）、规则书和冒险书（只有 README）、本地导入、`fixes/`、资料图登记表、你自己的规则裁定都不会被覆盖，打包时会自动检查。确认迁移没问题后，`Saves/_backups/` 里的备份可以自行删除。每个版本改了什么见仓库里的 `CHANGELOG.md`；v1.4.0 的已知限制（知识索引需要先构建、`--include-hidden` 是 DM 视图、AI 默认策略、完整书 pack）也在那里。

## 版权与许可

- 文档、`Rules/defaults/rulings*.md` 与本机 `Rules/rulings*.md`：MIT 许可。许可全文在仓库的 LICENSE 文件。
- `Rules/combat/` 中的规则数据只取自 **System Reference Document 5.1**（2014）与 **5.2.1**（2024），按 CC-BY-4.0 使用。署名在仓库的 NOTICE 文件。
- 「Dungeons & Dragons」「D&D」是 Wizards of the Coast 的商标。本项目是非官方的爱好者作品，与 Wizards of the Coast 无关，也未获其认可。
- 本仓库不包含、也不分发任何非 SRD 的规则书或冒险书文本；请使用你自己合法拥有的书。
