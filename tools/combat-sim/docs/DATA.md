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
  2024 SRD, so the 2024 rules chapters are not in a pack yet.
- Class prose: 5e-database classes carry mechanics, level tables and feature
  text but no class introduction; only the mechanics are imported.
- Reference vocabularies (skills, languages, proficiencies, damage types, magic
  schools, weapon properties) stay inside the structured `data` fields; they
  are not separate entities.
- 2024 magic item `shield` duplicates the equipment id; the equipment entry is
  kept and the magic-item copy is skipped (reported by the CLI as a gap).
- Monsters keep automation for plain attacks/multiattack; special abilities,
  legendary actions and complex riders stay in the statblock text.
