# Data sources

- Party: `LetsPlay/Saves/2026-10-19-01/Characters/{Will,Rina-Merryblade,Milo-Quill,Sera-Brightwell}.md` (companions moved from `NPCs/` in save schema 6).
- Monsters: the edition's monster book under `LetsPlay/Rules/2014/` or `LetsPlay/Rules/2024/` (player-supplied text):
  Guard (~line 18800), Animated Armor (~2545), Scarecrow (~31150), Mimic (~24610), Allosaurus (~39605).
- Animatronic allosaurus changes: area V10 of the local converted adventure (`LetsPlay/Adventures/keys-from-the-golden-vault/index/`, paged edition).

Open TODOs (marked in each file's `notes`):
- Guard AC: OCR reads 15; chain shirt + Dex + shield = 16 (16 used).
- Scarecrow / Mimic / Allosaurus: no Multiattack line in OCR — verify in the printed book.
- Allosaurus pounce, Milo's Sleep, Sera's Bless/Sanctuary, Rina's Cunning Action: not automated (manual commands).

## SRD content packs (5.1 / 5.2.1)

The SRD packs are generated from the 5e-database JSON and committed under the
edition layer, so a save loads only its own ruleset:

| Edition | Directory | Packs |
|---|---|---|
| 2014 / SRD 5.1 | `LetsPlay/Rules/combat/2014/srd/` | monsters, spells, classes, species, backgrounds, feats, items, rules |
| 2024 / SRD 5.2.1 | `LetsPlay/Rules/combat/2024/srd/` | monsters, spells, classes, species, backgrounds, feats, items |

Upstream: `github.com/5e-bits/5e-database`, commit
`a6212beb4b278917c2cff41c2b04c4ef4f3e0d6f` (SRD 5.1 = `src/2014/en`,
SRD 5.2.1 = `src/2024/en`). Both SRDs are CC-BY-4.0; the packs keep the
attribution. The upstream JSON is **not** committed.

### Regenerate

```bash
# 1. Fetch the pinned upstream tree (tarball sha256 at generation time:
#    8541f5803280a502381685bb14293eedce9b424c4235c7f6baa60aae40da0249).
mkdir -p /tmp/5e-database && cd /tmp/5e-database
gh api repos/5e-bits/5e-database/tarball/a6212beb4b278917c2cff41c2b04c4ef4f3e0d6f > 5e-database.tar.gz
tar xzf 5e-database.tar.gz --strip-components=1

# 2. Generate the packs (run from the repository root).
cargo run --manifest-path tools/combat-sim/Cargo.toml -- content import-srd-pack \
  --edition 2014 --src /tmp/5e-database/src/2014/en --out LetsPlay/Rules/combat/2014/srd
cargo run --manifest-path tools/combat-sim/Cargo.toml -- content import-srd-pack \
  --edition 2024 --src /tmp/5e-database/src/2024/en --out LetsPlay/Rules/combat/2024/srd
```

The generator is deterministic: the same upstream files produce byte-identical
packs. Regenerate rather than hand-edit; `cargo test --test srd_pack` locks the
committed counts.

### Counts (commit a6212be)

| Category | 2014 (SRD 5.1) | 2024 (SRD 5.2.1) |
|---|---:|---:|
| creatures | 334 | 341 |
| spells | 319 | 339 |
| classes | 12 | 12 |
| subclasses | 12 | 12 |
| species (incl. subraces/subspecies) | 9 + 4 | 9 + 24 |
| backgrounds | 1 | 4 |
| feats | 1 | 17 |
| items (equipment + magic + poisons) | 237 + 362 | 182 + 262 + 14 |
| features (rule sections) | 407 | 232 |
| traits (rule sections) | 38 | 67 |
| rules text (rule sections) | 39 | — |
| class level tables | 12 | 12 |

Pack size: about 4.4 MB (2014) and 4.8 MB (2024). Loading one edition in a
release build adds roughly 0.14–0.26 s to a CLI start (measured on the
development machine with `content list --kind creatures`); a lazy pack loader
is a possible follow-up if that becomes noticeable in play.

### Known gaps

- 2024 rules text: upstream has no `Rules` / `Rule-Sections` dataset for the
  2024 SRD. It is covered by the prose pack below (extracted from the local
  pdf2md output, not from 5e-database).
- Class prose: neither 5e-database nor the SRD PDFs contain class
  introductions; both editions go straight to Class Features / Core Traits.
  Nothing is invented.
- Reference vocabularies (skills, languages, proficiencies, damage types, magic
  schools, weapon properties) stay inside the structured `data` fields; they
  are not separate entities.
- 2024 magic item `shield` duplicates the equipment id; the equipment entry is
  kept and the magic-item copy is skipped (reported by the CLI as a gap).
- Monsters keep automation for plain attacks/multiattack; special abilities,
  legendary actions and complex riders stay in the statblock text.

## SRD 5.2.1 rules prose

The 2024 rules chapters are not in 5e-database, so they are extracted from the
local pdf2md conversion of the player-supplied SRD PDF:

- Source: `LetsPlay/Rules/2024/srd-cc-v5-2-1/index/` — pdf2md 0.6.0, PDF
  `SRD_CC_v5.2.1.pdf`, sha256
  `8974902d109d6e63672d7c490bde9ccf052410503d9cfa768237154fbc5e3d87`,
  364 pages. The pdf2md output stays local and gitignored.
- Output: `LetsPlay/Rules/combat/2024/srd/srd-5.2.1-rules-prose.pack.json`
  (`distribution: srd`, CC-BY-4.0), 213 entities, about 340 KB.

```bash
cargo run --manifest-path tools/combat-sim/Cargo.toml -- content import-srd-prose \
  --edition 2024 \
  --index LetsPlay/Rules/2024/srd-cc-v5-2-1/index \
  --out LetsPlay/Rules/combat/2024/srd
```

| Chapter | Pages | Sections |
|---|---:|---:|
| Playing the Game | 5–18 | 61 |
| Character Creation | 19–27 | 23 |
| Rules Glossary | 176–191 | 104 |
| Gameplay Toolbox | 192–203 | 40 |

The generator finds chapter ranges from `index.json` (no hard-coded pages),
splits on level-2/3 headings, merges repeated headings that continue on the
next page, and skips headings whose body is shorter than 20 characters. The
current run merged 9 repeated headings and skipped 6 short ones. Text is kept
verbatim (including Markdown tables); no image is embedded.

The pack only adds narrative rules text. Structured mechanics stay in the
5e-database packs, so the two sources are complementary.

Known limits:

- Extraction follows the pdf2md heading structure; if a heading is missed by
  the converter, that section is missing from the pack (the chapter/section
  counts above are the audit trail).
- Running headers and page footers are kept when the converter keeps them;
  they are not rewritten.
- The SRD has no class introductions (checked in both editions), so no class
  prose entity exists.
