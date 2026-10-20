# Party sheet with two combat blocks; the text fence must be ignored.

```text
id: ignored-creature
ac: 99
hp: 99
```

```combat yaml
id: hero-a
name: Hero A
kind: pc
side: party
size: medium
ac: 15
hp: 20
speed: {walk: 30}
abilities: {str: 10, dex: 14}
conditions: [dazed]
actions:
  - {ref: zap}
```

Some prose between blocks.

```combat json
{
  "id": "hero-b",
  "name": "Hero B",
  "kind": "pc",
  "side": "party",
  "ac": 13,
  "hp": 18,
  "abilities": {"str": 8, "dex": 16}
}
```
