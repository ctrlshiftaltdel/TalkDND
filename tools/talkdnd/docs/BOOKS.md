# Full-book identification and unlocking (v1.2b)

`talkdnd` identifies the player's own PHB/DMG/MM from the **content** of the
pdf2md output under `Rules/2014/` and `Rules/2024/`. When the save's ruleset is
`5e-full` or `5.5e-full` (the FULL options for 5e and 5.5e), it loads only the
matching full pack. A `5e-srd` / `5.5e-srd` save never loads full packs. When a
FULL save has no matching unlocked pack, or the pack is invalid, `talkdnd start`
and `combat-sim content check` fail closed and name the missing book and the
PDF/pack steps; they never silently fall back to SRD.

This is an access gate, not DRM: the player already owns the PDF. The program
never copies, uploads or prints book text.

## What the scan reads

- The paged Markdown `Rules/<edition>/<book>/index/<stem>.pages.md`, first
  16 PDF pages (or 2 MiB, whichever comes first). pdf2md provides the text or
  OCR; `talkdnd` does not OCR itself.
- Title-level headings from that folder's `index.json` (`headings`,
  `chapters`, `intake.toc`).
- The title page, copyright/publication year and the front-matter table of
  contents. The result is a book type (`phb` / `dmg` / `mm`) and a rules
  edition (`2014` / `2024`, where a 2025 printing of the 2024 rules counts as
  `2024`).

The scan does **not** use:

- the PDF or folder name;
- a PDF hash (different scans of the same book are expected);
- the pack file name;
- book prose after classification. The front matter is dropped after the
  signature is computed.

A PDF must exist in the book folder. A leftover `index/` without its PDF is
reported as unconverted and cannot unlock anything. The scan expects the
standard one-folder-per-book layout; if a PDF is still loose, run
`talkdnd library tidy` (or `talkdnd start`) first.

## Outcomes

| Outcome | Meaning | Full pack |
|---|---|---|
| `identified` | one book type and edition are clear | the matching pack may unlock |
| `ambiguous` | two or more candidates remain | nothing unlocks until the player chooses |
| `unknown` | no usable title/year signal | nothing unlocks; the player may choose |
| `srd only` | the content says System Reference Document | never unlocks |
| `missing index` | PDF without a converted index, or index without the PDF | nothing unlocks |

An ambiguous result is never resolved by guessing. The scan prints the
possible pack ids and the command to choose:

```bash
talkdnd library scan
talkdnd library choose Rules/2014/my-book/index --edition 2014 --kind phb
```

`choose` records the player's explicit decision for that converted source.
The decision is kept while the source file's mtime and length are unchanged;
after a reconversion the scan asks again. `choose` refuses SRD sources and
sources without a PDF/index.

## Full packs

Full packs are separate release artifacts. Extract them into
`Rules/packs/`; one file per book:

```text
Rules/packs/full-2014-phb.pack.json
Rules/packs/full-2014-dmg.pack.json
Rules/packs/full-2014-mm.pack.json
Rules/packs/full-2024-phb.pack.json
Rules/packs/full-2024-dmg.pack.json
Rules/packs/full-2024-mm.pack.json
```

The pack id decides which book it is. A pack unlocks only when all of these
hold:

1. its id is `full-<edition>-<book>` for one of the six expected packs;
2. `pack.edition` matches the id, `pack.distribution` is `full`, and
   `pack.schema_version` is the current version;
3. the pack passes the normal `ContentPack` validation (including its content
   hash — this is pack integrity, not PDF identification);
4. a `BookRecord` identifies the same edition and book from a PDF under
   `Rules/<edition>/`;
5. the save's `ruleset` is `5e-full` or `5.5e-full` for that edition;
   `5e-srd` / `5.5e-srd` never load full packs.

Pack files are release artifacts, never Git content. Do not commit them.

## Status and start

```bash
talkdnd library packs
talkdnd start
```

`library packs` lists the six expected packs and says exactly what is missing
(for example `缺少 2014 Player's Handbook 的已识别 PDF` or
`缺少完整书内容包`). `start` rescans the library and rewrites the save's
`combat/campaign.yaml` `library:` list:

- a FULL save (`5e-full` / `5.5e-full`) gets exactly the full packs unlocked
  for its edition; at least one must be unlocked or `start` fails and names the
  missing book, the PDF folder and the pack path. It does not fall back to SRD.
- an SRD save (`5e-srd` / `5.5e-srd`) removes the managed full-pack entries and
  never loads a full pack.

On a successful FULL start the digest writes:

- `完整书内容包：已解锁 2014 Player's Handbook（SRD 仍是基础层）`

On an SRD save it writes `完整书内容包：仅 SRD（5e SRD 只使用 SRD 层，不加载完整书内容包）`.
If a matching PDF or pack disappears, the next FULL `start` fails closed rather
than silently continuing with SRD content. The base `Rules/combat` library is
never removed, so the SRD layer remains the foundation in every FULL outcome.

## Audit

The scan writes `.talkdnd/books.yaml` (local, gitignored). Each record holds
only:

- `edition`, `book`, `status`;
- `source` (path relative to the play folder);
- `method` (`content` or `manual`), `evidence` (counts and matched feature
  names, never prose), `stamp` (converted-Markdown mtime and length);
- `candidates` for a pending player choice.

No absolute path, book text, page image or PDF hash is stored.

## Limitations

- Identification depends on the first pages containing a recognisable title
  and a copyright/publication year. Unusual scans can stay `ambiguous`;
  choose explicitly or improve the conversion with pdf2md.
- A stale `index/` next to a different PDF cannot be detected without reading
  the PDF; pdf2md treats the PDF as authoritative. Re-run pdf2md if the PDF
  was replaced.
- The check is a convenience gate for a local, single-player library, not a
  security boundary. It never claims a pack is legitimate beyond the metadata
  and content-hash validation above.
