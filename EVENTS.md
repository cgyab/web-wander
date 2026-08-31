# WebWander — Event / POI roadmap

Procedural points-of-interest that punctuate the walk. Design rule: each should
change the moment-to-moment decision without changing the core loop (distance
from origin). Most reuse existing primitives — `spawn_chunk`, chunk-bound
structs on `Game`, the arena/campfire/relic patterns, loot caches, `drop_pos`,
and snapshot entity kinds.

## ✅ Already in the world
Chests · Health fountains · Colossus mega-bosses · Milestone showers
(1k/10k/100k) · Arena ring (two-ring + apron) · Cursed relic (speed sprint +
hunters) · Campfire rest site · Offering shrine · **Fishing mini-game
(#10/#15)** — cast "read the water" (aim the ★ prize, dodge ✕ snags) → bob-hook
→ scrolling-tap reel · **Cursed fog (#13)** — ambush blindness + premium cache ·
**Shield shrine** — touch for a one-time non-recharging blue ward (temporary HP buffer) ·
**Champion's duel (#12)** — a lone named elite (gold crown), fair 1v1, prize on death ·
**Rune vault (#16)** — Simon-style puzzle overlay (world runs while you solve), cache on solve ·
**Rift (#1)** — step in to leap ~400 tiles forward into deeper danger (mini-boss or ambush).

## 🎯 20 candidates

**Carried over from the earlier brainstorm**
1. **Rift / teleporter** — step through to jump forward *toward the goal*, but land in a higher danger tier unprepared (or a mini-boss spawns). *Free distance; you arrive unready.* **(built)** step into the portal → leap ~400 tiles radially outward (real, banked progress); on arrival, 40% a champion mini-boss else a swarm ambush; consumed on use (looted_rifts); dev hook `?rift=1`.
2. **Collapsing vault** — opening it starts a countdown as guardians wake; grab as much as you dare before it seals. *Greed vs. safety.*
3. **Elemental trial totem** — a foe vulnerable to only one element (2× weakness, near-immune otherwise). *Trivial + rich loot if your build fits; punishing if not.*
4. **Meteor / hazard field** — rhythmic falling projectiles; dodge inward to reach center loot. *Pure reflex.*
5. **Will-o'-wisp chase** — catch a fleeing sprite within a time limit for a unique; it lures you off-path. *Time + positioning risk.*
6. **Gamble shrine (dice)** — feed ammo or a weapon for double-or-nothing loot. *Pure risk/reward.*

**New ideas**
7. **Wandering merchant** — trade spare gear for ammo/health/a reroll. Rare **mimic** chance it's actually a monster.
8. **Cursed graveyard** — cross it and undead keep rising until you leave; tombs hold loot. *Attrition zone.*
9. **Blessing obelisk** — touch for a timed buff (damage/speed/element), but it empowers & aggros every monster around it until you leave.
10. **Sanctuary grove** — a *rare, truly safe* heal-to-full refuge monsters can't enter (opposite of the risky campfire). Very rare; big time cost to reach.
11. **Beacon / siege fire** — light it to summon a timed horde-siege for a huge cache. *A self-inflicted arena you choose to trigger.*
12. **Champion's duel** — a lone named elite guarding a prize; a fair 1v1 mini-boss with no adds. **(built)** a tagged mini-boss (hp×5, gold aura + crown, "♛ CHAMPION" target tag); ambient adds cleared within a duel radius; drops a boosted prize chest + ammo on death; dedup via looted_champions; dev hook `?champion=1`.
13. **Miasma / cursed fog** — a patch where the view shrinks and monsters go near-invisible; rich loot for braving the dark. **(built)** ambush visibility (mobs hidden until ~2 tiles), blindness-only risk, premium cache (boosted chest + health + ammo) at the heart; dedup via looted_chests, dev hook `?fog=1`.
14. **Whirlpool current** (on water) — a fast current sweeps you a big distance, but through deep-water danger / drops you off-path. *Distance shortcut with a catch.*
15. **Fishing / harvest node** — a timing/patience mini-game at water for ammo or healing. **(built as the fishing mini-game)**
16. **Rune puzzle vault** — a light memory/sequence puzzle opens a cache; a non-combat brain break. **(built)** Simon-style rune sequence (length 3-6 by region) in a fixed-size overlay; **pauses the world** so it's a calm puzzle (playtest showed running-the-world mid-solve got you killed unfairly); bail with Esc; solving drops a cache (chest + health + ammo); dedup via looted_vaults; dev hook `?vault=1`.
17. **Lorestone / ancient library** — meditate to gain a burst of XP in a chosen skill, at a time + ambush cost.
18. **Meteor storm** (regional timed event) — periodic sky-fire sweeps the region; dodge, and fresh craters drop ore/ammo. *Weather-like, not a fixed POI.*
19. **Time-trial gate** — reach the next gate within a countdown for an escalating reward chain. *Rewards fast, aggressive travel.*
20. **Treasure-guardian statue** — a statue that animates into a boss the instant you take its loot. *"Do you dare grab it" bait.*

## Priority notes
- **User's favorites / planned order:** ~~#12~~ → ~~#13~~ → ~~#16~~ → ~~#1~~. **All favorites shipped.** (#10/#15 fishing, #13 fog, #12 champion, #16 vault, #1 rift.) Next candidates by cheapness: #2, #3, #6, #7, #8, #9, #11, #20.
- **Cheapest to build** (reuse existing primitives): 2, 3, 6, 7, 8, 9, 11, 12, 16, 20.
- **Most on-theme for the distance goal**: 1 (rift), 14 (current), 19 (time-trial gate).
- **Heaviest lifts** (new rendering/mechanics): 13 (fog/vision), 18 (regional weather).
