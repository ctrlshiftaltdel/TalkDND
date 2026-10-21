# talkdnd

TalkDND play helper. It does the mechanical, non-LLM steps for the AI DM so an
agent can start a scene after reading one short digest. Players run it as
`LetsPlay/bin/talkdnd` (Windows `talkdnd.exe`) from inside `LetsPlay/`.

| Command | What it does |
|---|---|
| `start [--save S] [--no-pdf2md]` | Newest (or named) save: ruleset, `combat-sim content check`, starts `pdf2md run` in the background when `uv` exists, writes `Saves/<slot>/digest.md` (last checkpoint, party, warnings) and prints it |
| `new [--ruleset 2014\|2024] [--adventure NAME]` | New save skeleton (`Characters/ NPCs/ Sessions/ combat/campaign.yaml canon.md dm-secrets.md`) and `chargen.json` |
| `chargen status` | Steps done and the next step |
| `chargen set <field> <value>` | ruleset, adventure, species, class, subclass, masteries, background, skills, background-skills, armor, shield, equipment, spells, name, personality, level |
| `chargen roll` / `array` / `pointbuy [scores]` | 6×`4d6dl1` through the combat-sim dice engine (logged, final), standard array, or 27-point buy |
| `chargen assign str=.. dex=..` | Player assigns the pool |
| `chargen asi str=2 con=1` / `none` | 2024 background +2/+1 or +1/+1/+1; 2014 half-elf free picks (other 2014 species apply automatically) |
| `chargen validate` / `sheet` / `confirm` | Rule checks; sheet Markdown + `combat yaml` block from the same numbers (attacks, class features, resources, spell slots); mark done |
| `data status [--ruleset R]` / `data import [--ruleset R]` | Local book data: show loaded files; build `Rules/chargen/local/imported-R.yaml` (gitignored) from converted Markdown |
| `npc random [--count N] [--level L]` | Random SRD species/class/background/abilities for companions (drafts in `npc-drafts/`) |
| `npc sheet <draft> --name X` | Write `NPCs/X.md` after the player confirms |
| `sheet export <who\|all> [--format pdf\|html]` | One A4 page per character next to its sheet (`Characters/<name>.pdf`): printed stats plus fillable AcroForm boxes/tick grids for HP, hit dice, death saves, slots, resources, conditions, coins, XP, notes. See "Sheet export" below |
| `checkpoint <text>` (or stdin) | Append `## 存档点（time）` to the newest session log |
| `log <text>` | Append one timestamped line |
| `options [--ruleset R]` | SRD options |

Class progression and weapons live in `data/*.yaml` (SRD 5.1 / 5.2 only,
embedded at build time). Non-SRD data is local: `LetsPlay/Rules/chargen/local/*.yaml`,
same format, merged at runtime, never committed. See `LetsPlay/docs/chargen.md`.

Validation uses SRD 5.1 (2014) and SRD 5.2.1 (2024) option names and numbers
only. Options from other books are accepted with a warning. The hidden
`--seed` flag exists for tests; never use it in play.

Tests: `cargo test --manifest-path tools/talkdnd/Cargo.toml`.

## Sheet export

`talkdnd sheet export` reads the sheet Markdown (title, header bullets,
sections such as 特性/装备/法术/个性/熟练) and the `combat yaml` block (all
numbers), plus XP and coins from `progress.yaml`. The PDF is drawn with
[krilla](https://crates.io/crates/krilla) and a **system CJK font** that is
subset-embedded (nothing font-related is committed). Lookup order:
`TALKDND_CJK_FONT=path[#face-index]`, common Noto CJK / WenQuanYi paths on
Linux, PingFang / Hiragino / STHeiti on macOS, Microsoft YaHei / SimHei /
SimSun on Windows, then `fc-match`. Fillable fields are then added with
[lopdf](https://crates.io/crates/lopdf) (AcroForm text fields and check boxes;
`NeedAppearances` is set so readers draw typed values). Typed values use the
reader's Helvetica: digits and Latin always work, CJK typing depends on the
reader. `--format html` writes the same page as HTML with `<input>`s
(`@page A4`, print from the browser) and needs no font file.
