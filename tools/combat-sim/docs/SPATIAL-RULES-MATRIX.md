# Spatial rules matrix (P0)

- Status: frozen by TASK-0085 (#169) on 2026-10-27.
- Applies to: 2014 / SRD 5.1 and 2024 / SRD 5.2.1, one ruleset per save.
- Companion: `SPATIAL-CONTRACT-v1.md` (engine-facing types and queries).
- Sources checked: local printed PHB/DMG (2014 and 2024) and SRD 5.1 / 5.2.1.
  Page numbers are printed pages, not PDF pages.

This file paraphrases rules; it never copies rulebook text. "RAW" means the
rule follows the named book. "OPTIONAL" is an official optional rule.
"HOUSE" is a project decision. "RULING" is a table ruling from
`LetsPlay/Rules/rulings*.md`. "GAP" means the data or rule is not available in
the current source and must not be claimed as implemented.

Engine status: **A** already automated by `src/space.rs`/engine, **M** manual
(DM command), **P1–P5** planned by the spatial roadmap (#170–#174), **N** not
planned in this program.

## 0. Decisions taken in P0

1. Units are 5-foot squares on a grid and feet for range/height; zone mode is a
   graph, not a metric space.
2. Diagonal movement costs one square (5 ft) in both editions by default.
   2014's optional 5-10-5 rule and Facing are opt-in profiles, off by default.
3. 2024 has no separate squeezing clause in SRD 5.2.1: a narrow opening one
   size smaller is Difficult Terrain. 2014 keeps PHB p.192 squeezing.
4. Tiny creatures occupy one grid cell in v1. The 4-per-square rule is recorded
   but deferred; nothing in P1 may assume four Tiny per cell.
5. A creature can provide cover; the default degree is half cover and multiple
   sources use only the most protective degree.
6. Zone queries treat "same zone" as distance 0 for reach/travel; display text
   may say "same zone (≈5 ft)". Ranged bands use zone edges.
7. `can_reach` is geometric range only. Line of sight, cover and occupancy are
   separate queries; callers must combine them.
8. Default profiles never add facing, flanking or turning costs.

## 1. Distance, grid and diagonals

| ID | Topic | Ed | Source | Interpretation | Kind | Status |
|---|---|---|---|---|---|---|
| S-01 | Grid square | both | 2014 PHB p.192; 2024 PHB p.25 / SRD p.13 | One square is 5 ft; Speed is spent in 5-ft segments | RAW | A |
| S-02 | Diagonal cost | both | 2014 PHB p.192; 2024 SRD PDF p.13 | Entering an orthogonally or diagonally adjacent square costs one square | RAW | A |
| S-03 | Optional 5-10-5 | 2014 | 2014 DMG p.252 | Every second diagonal costs 10 ft; profile switch | OPTIONAL | P1 |
| S-04 | Corner cutting | both | 2014 PHB p.192; 2024 SRD PDF p.13 | A diagonal may not cross a corner filled by a wall/tree/terrain | RAW | A (grid), P2 (3D) |
| S-05 | Range on a grid | both | 2024 SRD PDF p.13; 2014 PHB p.192 | Count squares by the shortest route from a square adjacent to the source to the target's space | RAW | A |
| S-06 | 3D distance | both | no explicit 3D grid rule | Chebyshev cells × 5 ft plus altitude difference in feet; documented as HOUSE; profile may change | HOUSE | P1 |
| S-07 | Zone distance | both | `src/space.rs` (SAME_ZONE_DISTANCE); SPEC-v2 differs | Same zone = 0 ft for reach/travel; zone edges carry their own distance | PROJECT | A |

Note: `SPEC-v2.md` says "same zone ⇒ within 5 ft". This matrix supersedes that
for engine queries: 0 is the "no travel" value. Display text may approximate.

## 2. Movement costs

| ID | Topic | Ed | Source | Interpretation | Kind | Status |
|---|---|---|---|---|---|---|
| M-01 | Difficult Terrain | both | 2014 PHB p.190–192; 2024 SRD PDF p.14/p.181 | Each foot costs 1 extra; not cumulative; a space either is or is not difficult | RAW | A |
| M-02 | Standing up | both | 2014 PHB p.191; 2024 `Prone` glossary | Costs half Speed (rounded down in squares) | RAW | A |
| M-03 | Crawling | both | 2014 PHB p.182; 2024 SRD PDF p.179 | Each foot costs 1 extra (2 extra in Difficult Terrain) | RAW | M |
| M-04 | Swimming / Climbing | both | 2014 PHB p.182; 2024 SRD PDF p.179/p.189 | Each foot costs 1 extra unless using the matching Speed | RAW | M |
| M-05 | Jumping | both | 2014 PHB p.182; 2024 SRD PDF p.184 | Long/High Jump; movement can be split; no automation | RAW | N |
| M-06 | Squeezing, 2014 | 2014 | 2014 PHB p.192 | A Large creature can squeeze through a 5-ft opening; 1 extra ft per foot; disadvantage on attacks and Dex saves; attacks against it have advantage | RAW | P0 (policy), P3 (movement) |
| M-07 | Squeezing, 2024 | 2024 | 2024 SRD PDF p.181 (Difficult Terrain) | No separate squeezing clause; a narrow opening one size smaller is Difficult Terrain; no disadvantage from the SRD text | RAW | P0 (policy), P3 |
| M-08 | Forced movement | both | 2014 PHB p.195–196; 2024 glossary | Pushing/pulling does not spend the mover's Speed and does not provoke by itself; per-foot resolution is P3 | RAW | M |
| M-09 | Carrying/dragging a creature | both | `rulings.md` R-10 | Speed halved while carrying/dragging | RULING | M |

2024 note: printed PHB may add squeezing text that SRD 5.2.1 omits. Until the
full-book pack is available, the 2024 profile follows SRD 5.2.1 (Difficult
Terrain only); this is recorded as a GAP for the full ruleset, not invented.

## 3. Size, space and occupancy

| ID | Topic | Ed | Source | Interpretation | Kind | Status |
|---|---|---|---|---|---|---|
| O-01 | Size → space | both | 2014 PHB p.191–192; 2024 SRD PDF p.14 | Tiny 2½ ft (4/square), Small/Medium 1 square, Large 2×2, Huge 3×3, Gargantuan 4×4 | RAW | A (distance), P1 (footprint) |
| O-02 | Tiny sharing | both | 2024 SRD PDF p.14 | Four Tiny per square; v1 keeps one token per cell (explicit approximation) | RAW + HOUSE limit | P1 (documented limit) |
| O-03 | Occupied space | 2024 | 2024 SRD PDF p.185 | A space is occupied if a creature is in it or objects fill it | RAW | P2 |
| O-04 | Moving through creatures | both | 2014 PHB p.191; 2024 SRD PDF p.14 | Pass through ally, Incapacitated, Tiny, or a creature two sizes apart; other creatures' spaces are Difficult Terrain | RAW | P3 |
| O-05 | Ending a move | both | 2014 PHB p.191; 2024 SRD PDF p.14 | Cannot willingly end in an occupied space; if forced, fall Prone unless Tiny or larger than the other creature | RAW | P3 |
| O-06 | Footprint check | both | derived from O-01 | Every step checks all cells the footprint covers; no squeezing through a 1-cell gap unless M-06/M-07 allows it | PROJECT | P3 |

## 4. Cover and line of sight

| ID | Topic | Ed | Source | Interpretation | Kind | Status |
|---|---|---|---|---|---|---|
| C-01 | Cover degrees | both | 2014 PHB p.196; 2024 SRD PDF p.15/p.179 | Half +2 AC/Dex saves; three-quarters +5; total cannot be targeted directly | RAW | A |
| C-02 | Creatures give cover | both | 2014 PHB p.196; 2024 SRD PDF p.15 | Walls, trees, creatures and other obstacles can provide cover | RAW | P2 |
| C-03 | Multiple sources | both | 2014 PHB p.196; 2024 SRD PDF p.15 | Use only the most protective degree; never add degrees | RAW | P2 |
| C-04 | Opposite-side rule | 2024 | 2024 SRD PDF p.15 | Cover applies only when the attack/effect originates on the opposite side of the cover | RAW | P2 |
| C-05 | Creature cover degree | both | not fixed by RAW | Default: a creature gives half cover; a much larger creature or a tight formation may give three-quarters/total by DM call | HOUSE | P2 |
| C-06 | Line of sight | both | 2014 PHB p.183–184, p.196; 2024 SRD PDF p.15 | Terrain and total cover block sight; obscurement affects perception, not geometry | RAW | A (Bresenham), P2 (volumes) |
| C-07 | Low obstacles | both | 2014 PHB p.196 | A low wall can give cover without blocking sight; the 3D volume decides | RAW | P2 |
| C-08 | Illusion blocks sight | both | `rulings.md` R-17 | An unpenetrated Minor Illusion blocks sight for opportunity attacks | RULING | P2 (LOS blockers) |

## 5. Areas of effect

| ID | Topic | Ed | Source | Interpretation | Kind | Status |
|---|---|---|---|---|---|---|
| A-01 | Sphere | both | 2014 PHB p.204–205; 2024 SRD PDF p.188 | Radius from an origin; origin included | RAW | P2 |
| A-02 | Cone | both | 2014 PHB p.204–205; 2024 SRD PDF p.178 | Width equals length at the far edge; origin not included | RAW | P2 |
| A-03 | Cube | both | 2014 PHB p.204–205; 2024 SRD PDF p.179 | Side length; origin on a face | RAW | P2 |
| A-04 | Cylinder | both | 2014 PHB p.204–205; 2024 SRD PDF p.179 | Radius and height; origin at the base center | RAW | P2 |
| A-05 | Line | both | 2014 PHB p.204–205; 2024 SRD PDF p.184 | Length and width from the origin | RAW | P2 |
| A-06 | Emanation | 2024 | 2024 SRD PDF p.181 | Extends from a creature/object in all directions and moves with it; origin excluded unless stated | RAW | P2 |
| A-07 | Shape availability | both | edition lists | 2014 has no Emanation; 2024 has all six | RAW | P2 |
| A-08 | Cover vs areas | both | 2014 PHB p.196; 2024 SRD PDF p.179 | Half/three-quarters cover does not reduce area damage by itself; total cover blocks direct targeting. DM decides edge cases | RAW + HOUSE | P2 |

Data gap: `AreaSpec` currently stores only `radius`; shapes and origins are P1
schema work and real shape data is TASK-0094 (#178). P2 tests use synthetic
fixtures until then.

## 6. Height, flight, hover and falling

| ID | Topic | Ed | Source | Interpretation | Kind | Status |
|---|---|---|---|---|---|---|
| V-01 | Altitude | both | no unified 3D rule | Track altitude in feet above the floor; ground = 0 | HOUSE | P1 |
| V-02 | Flying | both | 2014 PHB p.191; 2024 SRD PDF p.182 | A Fly Speed keeps you aloft until you land, fall or die; falling if Incapacitated/Prone or Fly Speed 0 | RAW | P4 |
| V-03 | Hover | both | 2014 PHB p.191; 2024 SRD PDF p.183 | Hover avoids the fall from those conditions; data field absent today | RAW | P4 + TASK-0094 |
| V-04 | Falling | both | 2014 PHB p.183; 2024 SRD PDF p.182 | 1d6 bludgeoning per 10 ft, max 20d6; land Prone unless no damage | RAW | P4 |
| V-05 | Falling into liquid | 2024 | 2024 SRD PDF p.182 | Reaction, DC 15 Str (Athletics) or Dex (Acrobatics) to enter head/feet first; success halves damage | RAW | P4 |
| V-06 | Fall ruling | both | `rulings.md` R-09 | Drop: 1d6/10 ft, Prone; DC 10 Dex (Acrobatics) within 10 ft to avoid Prone | RULING | M |
| V-07 | Burrow | both | 2014 SRD monster notes; 2024 none | No structured field; P4 does not automate burrowing; TASK-0094 may add data | GAP | N (documented) |
| V-08 | Vertical movement cost | both | 2014 PHB p.182 | Climbing costs 1 extra ft per foot without a Climb Speed | RAW | P3 |

## 7. Facing and flanking

| ID | Topic | Ed | Source | Interpretation | Kind | Status |
|---|---|---|---|---|---|---|
| F-01 | Facing | 2014 | 2014 DMG p.252 | Optional front/side/rear arcs; off by default | OPTIONAL | P1 profile |
| F-02 | Facing | 2024 | 2024 SRD (no such rule) | Not a 2024 rule; house rule only if enabled | GAP / HOUSE | N by default |
| F-03 | Flanking | 2014 | 2014 DMG p.251 | Optional +2 attack bonus when flanked; off by default | OPTIONAL | N (not in P1–P5) |
| F-04 | Default policy | both | this matrix | No facing/flanking/turning cost unless an explicit profile enables it | PROJECT | P0 |

## 8. Reach and opportunity attacks

| ID | Topic | Ed | Source | Interpretation | Kind | Status |
|---|---|---|---|---|---|---|
| R-01 | Reach | both | 2014 PHB p.195; 2024 SRD PDF p.186 | Default reach 5 ft; weapons/stat blocks may extend it | RAW | A |
| R-02 | Opportunity attack trigger | both | 2014 PHB p.195; 2024 SRD PDF p.185 | Reaction when a visible creature leaves your reach using its action/bonus/reaction/speed | RAW | A |
| R-03 | Forced movement | both | 2014 PHB p.195 | Being pushed/pulled does not provoke | RAW | A |
| R-04 | Reach in 3D | both | derived | Reach is 3D distance; altitude differences count | HOUSE | P4 |
| R-05 | Illusion blocks trigger | both | `rulings.md` R-17 | No opportunity attack against a creature the observer cannot see | RULING | P2 |

## 9. Project rulings mapped into the contract

| Ruling | Topic | Spatial effect |
|---|---|---|
| R-09 | Climbing/falling | climb cost, fall damage, DC 10 Acrobatics within 10 ft |
| R-10 | Carrying/dragging | half Speed, no extra reach |
| R-11 | Doors/locks | door cells are blockers until opened; noise is not geometric |
| R-17 | Minor Illusion | an unpenetrated illusion is a sight blocker for opportunity attacks |

## 10. Version deltas (summary)

| Topic | 2014 | 2024 |
|---|---|---|
| Diagonal default | 5 ft | 5 ft |
| 5-10-5 | official optional (DMG p.252) | not present |
| Facing | official optional (DMG p.252) | not present |
| Flanking | official optional (DMG p.251) | not present |
| Squeezing | PHB p.192, extra cost + disadvantage | SRD: narrow opening = Difficult Terrain; no separate clause |
| Tiny sharing | 2½ ft space (4/square implied) | 4 per square explicit |
| Emanation | absent | present |
| Cover wording | degrees + most protective | adds opposite-side rule |
| Surprise | lose first turn | Initiative disadvantage |
| Falling into liquid | not in SRD text used | DC 15 reaction, half damage |

## 11. Data coverage and gaps

| Topic | Current data | Gap | Owner |
|---|---|---|---|
| AoE shapes | `AreaSpec.radius` only | cone/cube/cylinder/line/sphere/emanation | P1 schema + TASK-0094 |
| Hover / burrow | 2014 SRD only in notes; 2024 none | structured fields + monster coverage | P1 + TASK-0094 |
| 2024 monsters | 5 hand-written + 341 imported | 2024 SRD import done in #182 | — |
| Pages/sections | packs carry section, not printed page | page numbers for citations | TASK-0106/next |
| Facing/flanking | no data | profile switch only, no automation | not planned |
| Tiny sub-cells | one token per cell | four-per-square model | deferred |

## 12. Open decisions for the owner

1. Tiny sharing in v1: keep one token per cell (recommended) or build sub-cell
   coordinates now.
2. Creature cover: default half (recommended) or let size difference decide
   three-quarters.
3. Zone mode: confirm same-zone = 0 ft for engine queries (recommended) and
   keep display text approximate.
4. 2024 squeezing: follow SRD 5.2.1 (Difficult Terrain only) until the full
   PHB pack exists, then re-check the printed book.
