# talkdnd

TalkDND play helper. It does the mechanical, non-LLM steps for the AI DM so an
agent can start a scene after reading one short digest. Players run it as
`LetsPlay/bin/talkdnd` (Windows `talkdnd.exe`) from inside `LetsPlay/`.

| Command | What it does |
|---|---|
| `start [--save S] [--no-pdf2md]` | Newest (or named) save: ruleset, `combat-sim content check`, starts `pdf2md run` in the background when `uv` exists, writes `Saves/<slot>/digest.md` (last checkpoint, party, warnings) and prints it |
| `new [--ruleset 2014\|2024] [--adventure NAME]` | New save skeleton (`Characters/ NPCs/ Sessions/ combat/campaign.yaml canon.md dm-secrets.md`) and `chargen.json` |
| `chargen status` | Steps done and the next step |
| `chargen set <field> <value>` | ruleset, adventure, species, class, background, skills, background-skills, armor, shield, equipment, spells, name, personality, level |
| `chargen roll` / `array` / `pointbuy [scores]` | 6×`4d6dl1` through the combat-sim dice engine (logged, final), standard array, or 27-point buy |
| `chargen assign str=.. dex=..` | Player assigns the pool |
| `chargen asi str=2 con=1` / `none` | 2024 background +2/+1 or +1/+1/+1; 2014 half-elf free picks (other 2014 species apply automatically) |
| `chargen validate` / `sheet` / `confirm` | Rule checks; sheet Markdown + `combat yaml` block from the same numbers; mark done |
| `npc random [--count N] [--level L]` | Random SRD species/class/background/abilities for companions (drafts in `npc-drafts/`) |
| `npc sheet <draft> --name X` | Write `NPCs/X.md` after the player confirms |
| `checkpoint <text>` (or stdin) | Append `## 存档点（time）` to the newest session log |
| `log <text>` | Append one timestamped line |
| `options [--ruleset R]` | SRD options |

Validation uses SRD 5.1 (2014) and SRD 5.2.1 (2024) option names and numbers
only. Options from other books are accepted with a warning. The hidden
`--seed` flag exists for tests; never use it in play.

Tests: `cargo test --manifest-path tools/talkdnd/Cargo.toml`.
