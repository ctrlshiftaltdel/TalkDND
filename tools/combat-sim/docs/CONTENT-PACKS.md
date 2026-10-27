# Content packs (v1)

Content packs are the versioned container that the AI DM engine uses for SRD,
licensed full-book and local content. A pack wraps the existing content schema
(conditions, masteries, abilities, creatures, AI templates, scenes, encounters,
domain entities and rules) with metadata, an optional search index and an
integrity hash.

This task defines the format and loader only. It does not import SRD or full-book
content, and it does not implement PDF presence gating (that is the 1.2 full-book
track).

## File shape

A pack is JSON or YAML with exactly two required top-level keys, plus an optional
index:

```json
{
  "pack": {
    "id": "example-srd-2024",
    "name": "Example SRD 2024",
    "edition": "2024",
    "source": "SRD 5.2.1",
    "license": "CC-BY-4.0",
    "distribution": "srd",
    "schema_version": 1,
    "content_hash": "fnv1a64:0123456789abcdef:1234",
    "created_at": "2026-10-27T00:00:00Z",
    "description": "Synthetic example"
  },
  "index": {
    "entries": [
      {
        "id": "example-condition",
        "kind": "condition",
        "name": "Example",
        "aliases": ["example"],
        "page": 12,
        "section": "Conditions",
        "refs": ["example-rule"]
      }
    ],
    "images": []
  },
  "content": {
    "conditions": [
      {"id": "example-condition", "name": "Example"}
    ]
  }
}
```

A pack file is recognised by the presence of both top-level `pack` and
`content` keys. Ordinary YAML/JSON/Markdown content keeps loading as before.

## Domain entities

Non-combat content uses the shared `DomainEntity` shape in these container
arrays: `items`, `classes`, `subclasses`, `species`, `backgrounds`, `feats`,
`rule_sections`, `tables`.

Each entity has `id` (required), plus optional `name`, `source`, `page`,
`section`, `tags`, `aliases`, `text`, `refs`, `data`, `manual`, `notes` and
`todo`. `data` is an open JSON payload for future structured use; the current
engine does not read these entities. A pack index may reference them with the
kinds `item`, `class`, `subclass`, `species`, `background`, `feat`,
`rule-section` and `table`.

Domain entities are container-only: a single top-level object with `id` is not
dispatched by guessing its kind. Put them under one of the arrays above.

## Metadata

- `id` (required): unique pack id.
- `name`, `source`, `license`, `created_at`, `description`: descriptive.
- `edition` (required): the rules edition this content belongs to.
- `distribution`: `srd`, `full` or `local`. It is recorded, not enforced here.
- `schema_version`: currently `1`; unknown versions fail closed.
- `content_hash`: FNV-1a 64 over the canonical JSON bytes of `content`,
  formatted as `fnv1a64:<hex>:<length>`. It is an integrity check, not a
  security boundary and not a PDF/book identifier. `ContentPack::new` fills it;
  the loader recomputes and checks it.

## Index

The index is optional. If present:

- Every entry must reference an entity id that exists in `content`, and the
  entry kind must match the entity kind (singular, plural and underscore
  spellings are accepted).
- Entry ids/kinds must not repeat.
- Image ids must not repeat.

The index is metadata for the Agent/search layer. Every entry must reference an
existing entity in the pack. An empty index is valid.

## Loader behaviour

- Pack files are scanned by the same deterministic source walker as other
  content (libraries, adventures, characters).
- Packs and ordinary YAML/JSON/Markdown files can be mixed. **Packs are the
  base layer**: within each source directory they load before ordinary content,
  so hand-written YAML/JSON overrides a pack by entity id. Path order still
  decides between ordinary files and between packs.
- The suffix `.pack.json`, `.pack.yaml` or `.pack.yml` marks a pack file.
- Pack metadata is recorded in `ContentRegistry.packs`; old state files without
  this map load with an empty map.
- A duplicate pack id, duplicate entity id inside a pack, bad index reference,
  bad hash or unknown schema version fails the load with the pack id/file in the
  error.
- A pack does not change entity semantics; it is a container and metadata layer.

## Commands

```bash
combat-sim content pack list --lib <dir>
combat-sim content pack show <pack-id> --lib <dir>
combat-sim content pack verify --lib <dir>
```

`verify` reports the loaded pack metadata and runs the normal content check.
Because the loader verifies the hash before returning, a loaded pack is already
hash-checked; a mismatch fails before `verify` can run.

## SRD packs

The SRD 5.1 and SRD 5.2.1 packs live next to their edition layer, so the normal
`library: [Rules/combat]` entry loads only the save's ruleset:

- `LetsPlay/Rules/combat/2014/srd/srd-5.1-*.pack.json`
- `LetsPlay/Rules/combat/2024/srd/srd-5.2.1-*.pack.json`

They are generated from the 5e-database JSON with
`combat-sim content import-srd-pack --edition <2014|2024> --src <dir> --out <dir>`;
see `DATA.md` for the upstream commit, counts and known gaps. Regenerate rather
than hand-edit. The older `srd-monsters.yaml` / `srd-spells.yaml` stay as the
engine overlay for 2014: they load after the packs and keep the automated
attack/spell fields.

## Distribution policy

- `srd`: SRD 5.1/5.2.1 content, CC-BY-4.0; attribution in `NOTICE`.
- `full`: full-book content distributed under the owner's licence; the 1.2
  track builds the release artifacts and gates access by player-provided PDFs.
- `local`: local/generated content; not distributed.

Do not put full-book text or scanned pages into the Git tree. Full packs are
release artifacts; book-presence detection and unlocking are separate tasks.
