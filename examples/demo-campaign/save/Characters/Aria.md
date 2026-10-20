# 艾莉亚（Aria）——示例角色卡

> 原创示例角色：人类 战士 1。叙述部分随意写，下面的数据块供 combat-sim 读取。

- 职业：战士 1　背景：士兵
- HP 12　AC 16（链甲衫 + 盾）　速度 30 尺
- 武器：长剑 +5（1d8+3 挥砍，精通 Sap）

```combat yaml
id: aria
name: Aria
kind: pc
side: party
size: medium
creature_type: humanoid
level: 1
ac: 16
hp: 12
speed: {walk: 30}
init_bonus: 1
prof: 2
abilities: {str: 16, dex: 12, con: 14, int: 10, wis: 12, cha: 10}
saves: {str: 5, con: 4}
skills: {athletics: 5, perception: 3}
resources:
  second-wind: {max: 2, recharge: long, short_rest_regain: 1}
attacks:
  - {id: longsword, name: Longsword, kind: melee, ability: str, to_hit: 5, damage: [{dice: "1d8+3", type: slashing}], reach: 5, properties: [versatile], mastery: sap}
actions:
  - {ref: second-wind}
ai: {template: defender}
```
