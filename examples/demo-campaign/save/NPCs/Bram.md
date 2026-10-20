# 布拉姆（Bram）——示例同伴 NPC

> 原创示例：半身人 牧师 1（生命领域要到 3 级才有）。性格：絮叨、怕黑、关键时刻很靠得住。

```combat yaml
id: bram
name: Bram
kind: pc
side: party
size: small
creature_type: humanoid
level: 1
ac: 15
hp: 10
speed: {walk: 30}
init_bonus: 1
prof: 2
abilities: {str: 10, dex: 12, con: 14, int: 10, wis: 16, cha: 12}
saves: {wis: 5, cha: 3}
spellcasting: {ability: wis, attack: 5, dc: 13}
resources:
  spell-slot-1: {max: 2, recharge: long}
attacks:
  - {id: mace, name: Mace, kind: melee, ability: str, to_hit: 2, damage: [{dice: "1d6", type: bludgeoning}], reach: 5}
actions:
  - {ref: sacred-flame}
  - {ref: cure-wounds}
  - {ref: bless}
ai: {template: healer}
```
