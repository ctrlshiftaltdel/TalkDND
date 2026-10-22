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
| `xp add [N] --category C --reason R [--difficulty D] [--again]` / `xp show` | Hybrid advancement ledger (`progress.yaml` `xp`, `xp_log`): small awards, ruleset difficulty tables, one-level-ahead cap, no double awards; `show` prints the adventure track (Golden Vault heist levels) |
| `level-up <who\|all> [--milestone --reason R]` | Level from XP, or a milestone level for a major story beat (XP raised to the threshold; once per event) |
| `check` (no arguments) | Save audit: integrity manifest (`integrity.yaml`: fingerprints of `progress.yaml`, sheets, `combat/*.yaml`, `reveals.yaml`, `house-rules.md` after every tool write) plus reconciliation of HP, coins, items and XP changes against new session-log lines and `xp_log`. `start` runs it too; old saves get a baseline on first run |
| `check <who> <what> [--dc N]` | Ability/skill check or save from the sheet (logged) |
| `integrity status` / `integrity accept --reason R` | Show issues; accept the current files and values (logged) |
| `reveal add <fact> [--source S] [--tag T]` / `reveal list [--tag T]` / `reveal check <term>` | Revealed-knowledge ledger (`reveals.yaml`): what the party has actually learned; check before naming a fact |
| `handout show <id\|search> [--crop x,y,w,h] [--png]` / `handout list [--dm]` / `handout mark <id> --player\|--dm [--crop …]` / `handout build` | Book images for the players. Registry `Adventures/…/.md/<Book>/handouts.yaml` (local, from the pdf2md index); everything DM-only by default; only player-safe entries (or their registered crop) are exported to `Saves/<slot>/handouts/`, printed as a path and Markdown image line, and recorded in reveals and the session log |
| `char <who> <op>` | Character sheet changes: `damage`/`heal`/`temp`, `use`/`restore` (slots, resources), `rest short|long [--hd N]`, `condition`, `exhaustion`, `inspiration`, `death-save`, `gold add|spend`, `item add|remove|equip|unequip|attune`, `ammo`, `note`, `dm-note`, `set` (fighting-style, background, feats, languages, tools, …), `batch "op; op"`. Source of truth `Saves/<slot>/sheets/<id>.yaml` + `combat/party-state.yaml`; the Markdown sheet is rendered; every change is logged and integrity-clean. Old sheets migrate on first use |
| `chargen set fighting-style|class-equipment|background-equipment|languages|tools|feats|traits|ideals|bonds|flaws|alignment` | Sheet fields after the official character sheet; starting equipment and gold applied from data (SRD built in, other books from `data import`) |
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
