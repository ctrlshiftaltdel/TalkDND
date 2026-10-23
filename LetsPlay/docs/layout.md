# 目录结构与交接清单

> 本文件是 `AGENTS.md` 的参考章节，按需查阅。路径都相对于游戏文件夹（解压出来的那个文件夹，名字随意）。

## 项目结构

| 路径 | 内容 | 是否入库 |
|------|------|----------|
| `Rules/2014/` | 玩家自备的 2014 版规则书（至少 SRD 5.1；可加 PHB/DMG/MM 2014） | ❌ 仅 README 入库 |
| `Rules/2024/` | 玩家自备的 2024 版规则书（至少 SRD 5.2.1；可加 PHB/DMG 2024、MM 2025） | ❌ 仅 README 入库 |
| `Rules/rulings.md` | 两版通用的 DM 裁定（本机副本，原创；升级不覆盖） | ❌（本机文件，不入库） |
| `Rules/rulings-2014.md` / `Rules/rulings-2024.md` | 版本差异裁定（本机副本；突袭、精通、睡眠术、激励、躲藏、力竭、擒抱、状态细节…） | ❌（本机文件，不入库） |
| `Rules/defaults/` | 随版本发布的裁定出厂默认（`rulings*.md`）；`talkdnd start` 只在本机副本缺失时复制，不覆盖已修改的副本 | ✅ |
| `Rules/combat/` | combat-sim 规则数据，分层：`common/` + `2014/` + `2024/`（只收 SRD 内容） | ✅（SRD，CC-BY-4.0） |
| `Adventures/<书名>.md` | 玩家自备的冒险书文本 | ❌ 仅 README 入库 |
| `Adventures/<冒险名>/combat/` | 该冒险的战斗数据：`scenes/`、`encounters.yaml`、`creatures.yaml` | ❌（由书改编，本地自建） |
| `Saves/YYYY-MM-DD-NN/` | 一份存档：`Characters/`（玩家角色卡）、`NPCs/`（同伴与重要 NPC，含 `_party.md`）、`Sessions/`（会话日志）、`combat/`（`campaign.yaml` 清单 + `*.state.json` 战斗状态）、`canon.md`（即兴定论）、`progress.yaml`（经验/金币/物品/当前场景）、`reveals.yaml`、`handouts.yaml` 与 `handouts/`（已给玩家看的资料图）（已揭示知识，`talkdnd reveal`）、`integrity.yaml`（存档校验，程序维护，不要手改）、`save-meta.yaml`（存档结构版本、锁定的偏离度与桌面设置，程序维护）、`original.yaml`（DM 原创内容台账，含隐藏面）、`dm-plan.md`（DM 长期计划，可无）、`secret-terms.yaml`（手动秘密词）、`identified.yaml`（已识别怪物/物品）、`rolls.log`（全部掷骰记录）、`house-rules.md`（可选）、`dm-secrets.md`（🔒）、`digest.md`、`chargen.json`、`sheets/`、`npc-drafts/` | ❌（个人进度） |
| `Saves/_backups/` | 存档结构迁移前的整份备份（自动生成，可删） | ❌ |
| `docs/` | DM 参考章节（本文件、掷骰、叙事、战斗、pdf2md、建卡细节、剧本偏离度），按需读 | ✅ |
| `tools/pdf2md/` | PDF → Markdown 后台转换工具（Python，经 uv 运行）。输出在各 PDF 旁的隐藏目录 `.md/<PDF 文件名>/`，见 `docs/pdf2md.md` | ✅ 源码；❌ 输出与 `.venv` |
| `bin/combat-sim`、`bin/talkdnd` | 战斗程序与游玩助手。Windows 加 `.exe`。不入库。放法见 `README.md` 和 `bin/README.md` | ❌ 本地文件 |

角色卡是 Markdown：上半部分写给人看，末尾有一个 ```` ```combat yaml ```` 数据块给引擎读。两部分都由 `talkdnd char` 从 `sheets/<id>.yaml` 生成，**不要手改**（见 `docs/character-data.md`）。

## 交接清单（换 DM 之前 / 每次结束会话时）

- [ ] 会话日志末尾写好 `## 存档点（现实日期 时间）`：游戏内时间、地点、每个角色 HP/资源/金币/重要物品、未决事项、下一步可能的选项。
- [ ] 本场的 HP、资源、物品、金币变化都用 `talkdnd char` 记过（`talkdnd char <名字>` 看一行状态）；`content check` 0 error；`talkdnd check`（不带参数）无警告。
- [ ] 本次得知的新事实已 `talkdnd reveal add`。
- [ ] 新裁定已写进本机副本 `Rules/rulings.md` 或 `Rules/rulings-<版本>.md`（`Rules/defaults/` 是出厂默认，不要改；升级后本机副本不会被覆盖）。
- [ ] 新秘密/伏笔写进 `dm-secrets.md`（长期计划写 `dm-plan.md`），日志里没有泄露秘密。
- [ ] 本次即兴说出的公开设定已写进 `canon.md` / `NPCs/`。
- [ ] 进行中的战斗：state 文件名写进日志，并注明当前轮次与回合者。
- [ ] 不要把版权文本、存档、`dm-secrets.md` 提交到 git（`.gitignore` 已排除，提交前 `git status` 检查）。
