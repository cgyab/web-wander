# WebWander — a "Walking Around" game

[![CI](https://github.com/cgyab/web-wander/actions/workflows/ci.yml/badge.svg)](https://github.com/cgyab/web-wander/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

An infinite, procedurally generated top-down 8‑bit action‑RPG that runs in the
browser. The whole point is **distance**: how far from the origin `(0,0)` can you
get? It's tuned so a million tiles is a legendary, ~sub-200-hour frontier. The deterministic simulation is a tiny Rust module compiled to raw
WebAssembly; TypeScript only handles input, rendering, and the HUD.

> The whole game is a small set of **rules + generators**. There is no
> hand‑authored content — terrain, monsters, and weapons are all produced from
> seeds. You can read essentially the entire game in an afternoon.

Walk out of the origin, fight procedurally generated monsters, loot
procedurally generated weapons, get better at the weapons you actually use, and
watch the world grow more dangerous the farther you roam. Reload the page and
the same universe regenerates.

## Controls

| Input | Action |
|-------|--------|
| `WASD` / arrows | move |
| mouse | aim |
| left click | attack |
| `1`–`4` / scroll wheel | select weapon slot (wheel cycles equipped slots; the top-right box shows the active weapon + its element) |
| `I` | inventory (sorted by dps, scrollable, weapon-type filter buttons) — click to equip to active slot, `v` deletes all weaker items (keeps equipped **and uniques**), ✕ drops one |
| `F` | fish (at a calm water's edge) |
| `H` | toggle HUD detail |
| `Esc` | cancel an active arena / close an open overlay (fishing, vault), else close inventory, else pause + open the slot menu |
| `M` | mute/unmute |

The player is always at screen center; distance from `(0,0)` (shown as **Dist**)
drives the **Danger** level.

**Keyboard aiming** (Settings → Controls) offers three schemes for players without
a good mouse (laptop touchpads, TrackPoints):
- **Mouse** (default) — point and hold-click to fire.
- **Move-aim** — aim in your movement direction; fire with a key (default `Enter`).
- **Aim keys** — a right-hand cluster (default `IJKL`) aims *and* fires while held;
  hold two for a 45° diagonal. Movement stays on `WASD`, and the Bag key moves to
  `B` (since `I` becomes Aim-up). Every key is remappable, and a key can never be
  bound to two actions (rebinding an in-use key swaps them; `M`/`Esc` are fixed).

**Touch devices** get on-screen **twin-stick** controls: a left virtual joystick
moves, a right joystick aims and fires, with on-screen buttons for weapon slots
`1-4`, Bag (inventory), Menu, and Mute.

Along the way, **mini-milestones** toast as your record climbs — 1k fountains,
10k ammo, a **field of shield wards** at 25k, a **trove of ancient chests** at
50k, a **field of teleporters** at 75k (visual progress markers; step through a
rift to leap ahead), and the big **flash mob** at **100,000**. If your last
weapon ever breaks, a basic sword scaled to where you respawn is provided so
you're never left defenceless.

## Run it

```bash
npm install
npm run dev      # builds the wasm, starts Vite dev server
```

Then open the printed URL in a Chromium‑based browser.

Production build:

```bash
npm run build    # -> dist/ (static; deploy anywhere)
npm run preview  # serve the build locally
```

Deploy the contents of `dist/` to any static host. For **Apache hosts
(e.g. DreamHost)** a `public/.htaccess` ships in the build and sets the WASM MIME
type, security headers (X-Frame-Options, nosniff, Referrer-Policy,
Permissions-Policy), and cache rules — fingerprinted `/assets/` cached
immutably, while `index.html`/`game.wasm` revalidate so a redeploy reaches
returning/installed clients. A Content-Security-Policy is set via `<meta>` in
`index.html`. On non-Apache hosts, port those headers to your host's config.

Installable as a browser app (PWA): `public/manifest.webmanifest` + a footprints
favicon/icon set (`favicon.svg`, `icon-192.png`, `icon-512.png`). Regenerate the
PNGs from the SVG with `node scripts/gen-icons.mjs` (uses headless Chrome).

### Requirements
- Node 18+
- Rust with the `wasm32-unknown-unknown` target:
  `rustup target add wasm32-unknown-unknown`

`npm run dev`/`build` run `scripts/build-wasm.sh`, which does a plain
`cargo build --release --target wasm32-unknown-unknown` and copies the resulting
module to `public/game.wasm`. **No wasm-bindgen / wasm-pack** — the boundary is
the bare wasm ABI plus shared linear memory.

### Progression simulator (balance evidence)
A headless bot drives the **real** `Game` simulation (via `advance()`, skipping
only the render snapshot) — pushing outward, fighting what it meets, grabbing
drops, and using its best usable weapon (melee when out of ammo). It reports how
far you get, how many deaths, and the damage/health economy per distance band,
so damage and health-scarcity tuning can be judged on evidence, not vibes.

```bash
npm run sim                                   # defaults: 6 seeds, 15 min, target 800
npm run sim -- --seeds 8 --minutes 20 --target 1000 --band 100
npm run sim -- --json                          # machine-readable
```

Because it links the real crate, it can never drift from the game rules. It
lives in `game/sim.rs` (bot + reporting) and `game/economy.rs` (bin entry).

### Verify (optional)
With a Chromium install at `/usr/bin/google-chrome`:

```bash
npm run build && npm run preview &   # serve on :4173
node scripts/verify.mjs              # drives real input, asserts every mechanic
```

## Architecture

```
Browser
  └─ TypeScript (src/)         input · rendering · HUD · localStorage
        │  set_input(); update(dt); read one snapshot buffer
        ▼
     WASM (game/, Rust)        deterministic simulation + world generation
        ▼
     Game state                regenerated from seed + coordinates + rules
```

The TypeScript side never understands simulation internals. Each frame it:
1. writes input via `set_input(keys, aimX, aimY, attack, slot)`,
2. calls `update(dt_ms)`,
3. reads a single serialized **snapshot** from wasm memory
   (`snapshot_ptr()`/`snapshot_len()`) and renders it.

### WASM API (all of it)

```
init(seed)                     start a fresh world
set_input(keys,ax,ay,atk,slot) feed input
update(dt_ms)                  advance one frame, rebuild the snapshot
snapshot_ptr() / snapshot_len()  visible state for rendering + HUD
equip(inv_idx, slot)           equip an inventory item
drop_item(inv_idx)             drop/trash an inventory item
save_ptr() / save_len()        persistent blob
io_ptr() / io_cap()            scratch buffer TS writes a save into
load_save(len)                 restore from that buffer
```

### Files

```
game/                Rust simulation (compiled to wasm)
  lib.rs             game loop, combat, chunk streaming, snapshot, save/load, exports
  rng.rs             splitmix64 hashing, PRNG, value noise / fbm
  world.rs           terrain generation + tile rules (movement cost, passability)
  monster.rs         monster traits + generator + weakness interaction
  weapon.rs          weapon bases + affix rules + generator
  player.rs          player state + use-based skills
src/                 TypeScript client
  main.ts            entry point + game loop
  wasm.ts            loads the raw wasm module
  snapshot.ts        parses the binary snapshot (mirror of build_snapshot)
  input.ts           keyboard/mouse
  render.ts          procedural canvas renderer (320x180, nearest-neighbor)
  ui.ts              HUD + inventory DOM
  save.ts            localStorage
scripts/
  build-wasm.sh      cargo build -> public/game.wasm
  verify.mjs         headless-Chrome runtime checks
```

## How it works

### Procedural terrain (`game/world.rs`)
Terrain is a pure function `tile_at(seed, tileX, tileY)`. Three fbm noise fields
— **elevation**, **temperature**, **moisture** — are combined by a small lookup
into 10 tile types (deep/shallow water, sand, grass, dense grass, dirt, rock,
mountain, snow, swamp). Low base frequencies make large coherent regions rather
than checkerboard noise. Nothing is stored: the same coordinate always
regenerates identically. Each tile carries movement cost, passability, and an
optional hazard.

### Chunk streaming (`stream_chunks` in `lib.rs`)
The world is divided into 32×32‑tile chunks. Only the 3×3 chunks around the
player exist at once. Entering a chunk deterministically spawns its monsters
from the chunk seed; leaving discards them; returning regenerates them. The
world never needs saving.

### Procedural monsters (`game/monster.rs`)
A monster is a recipe, not a definition: pick a **body** (Beast/Insect/Golem/
Wisp/Brute), an elemental **resistance**, a **weakness**, and a melee/ranged
style, then scale stats by the local difficulty. Names are assembled from the
traits (e.g. *Swift Frost Insect*). The core loop is
**strengths & weaknesses**: an attack does **2×** damage on a monster's weakness
and **0.5×** on its resistance (`elem_mult`), so you're rewarded for switching
weapons to match the enemy.

Each monster also has a **temperament** (`update_monsters` in `lib.rs`):
**fight** (charges and attacks), **flee** (runs away — ranged fleers kite and
shoot over their shoulder), or **wander** (roams its home area, indifferent
until provoked). Any monster that gets hit becomes hostile for a few seconds
(`anger`), so even a wanderer fights back. Ranged attackers are uncommon near
the origin and grow more frequent with distance.

### Procedural weapons & items (`game/weapon.rs`)
A weapon is fully described by a single `seed`; `generate(seed, power)` rebuilds
it deterministically. Weapons are `base + prefix + element + suffix`, assembled
from small rules tables. **Rarity** (Common→Epic) sets how many affixes apply.
Because weapons are seed‑defined, saving just stores seeds and regenerates the
stats — no weapon database.

### Resources: ammo & healing (`hit_monster` / `pickups` in `lib.rs`)
Ranged weapons draw from a shared **ammo** pool (one per shot); with an empty
pool they can't fire, so melee weapons stay relevant and ranged use carries a
cost. Kills scatter ground drops — **ammo is common** (so clearing monsters
restocks your ranged play, and revisiting regenerated areas is worthwhile),
weapons are uncommon, and **healing is rare** (health stays scarce to keep
movement and target selection meaningful). Melee kills cost no ammo, making
melee the way to stockpile it. Walk over a drop to collect it. Rare **health
fountains** (like chests, ~4% of chunks) give a full restore. The pack holds a
generous but bounded number of weapons (`INVENTORY_CAP`); at the cap, new
weapons stay on the ground until you drop something (the inventory list scrolls
and shows the count). **Slaying a mega**
is a real haul: two unique chests, a fountain, and a big ammo pile.

### Ruins & unique chests (`spawn_chunk` / `generate_unique`)
~5% of chunks contain a **ruin with a chest** (a golden sprite amid rubble).
Opening it grants a deliberately **overpowered "unique" weapon** scaled to the
region's difficulty — a leg-up to punch through to the next checkpoint. Uniques
carry an explicit flag so they round-trip through the save (regenerated via
`generate_unique`). Like all world entities they regenerate on revisit, but the
gear scales to location. **Once looted, a chest (and a used health fountain) is
gone** — the chunk is remembered (persisted in the save), so you can't reload on
top of one to farm it.

### Death penalties (`respawn_if_dead`)
Death is no longer free. On each death you **lose half your ammo** and every
**equipped weapon loses 10% durability**, breaking (and vanishing) at 0% — so
your active loadout wears out and must be replaced from loot. Combined with the
checkpoint respawn and full heal, this makes the death loop a real economy: you
still progress by dying, but it costs resources. Durability shows in the HUD
(`90%`) and inventory; a respawn flashes the screen red and posts an evocative
message so it reads as an event, not a glitch.

### Decorative scatter (`feature_at` in `world.rs`)
A deterministic second layer places trees, pines, rocks, bushes, cacti, reeds,
and flowers on top of the terrain (by biome) so the world feels filled. Purely
visual — it doesn't affect movement — and sent as a per-tile feature byte in the
render snapshot.

### Skill‑based progression (`game/player.rs`)
There is **no character level**. Eight skills (Sword, Bow, Axe, Fire, Cold,
Poison, Defense, Move) gain XP from use and feed straight back into
effectiveness: damage, attack speed, movement speed, and damage reduction. Use
swords and your sword skill climbs; take hits and Defense (and max HP) climbs.
The curve is a simple `level = sqrt(xp/6)` — no cap on levels, though **Move's
speed bonus is clamped** (`MOVE_BONUS_CAP`) so top speed stays humanly
controllable (~2s to cross the viewport). A **? How to play** button on the
slot-select screen explains the live status and skills in-app.

### Difficulty scaling (`difficulty_at` in `lib.rs`)
```
difficulty = 1 + floor( 25 * ( sqrt(1 + distance/270) - 1 ) )   // distance in tiles
```
A sub-linear curve: near the origin it rises ~1 level per 22 tiles (linear
slope), then compresses far out so danger never runs away. In practice
**100,000 is the designed end-game** — playtesting reaches ~50k, so 100k is the
human-plausible frontier and where the flash-mob finale fires. The math still
keeps going: **1,000,000 tiles is a finite ~Lv 1500** — a theoretical landmark
no human is expected to reach, not an astronomical wall. Reference points:
`L(100)=5, L(1k)=30, L(10k)=130, L(100k)=457, L(1M)≈1497`. Difficulty raises
monster level, health, damage, spawn rate, loot rarity, and mega frequency. The
origin is deliberately safe; return there and the world is calm again.

### Mega-monsters (`make_mega` / `elem_mult2`)
Away from the origin, ~2–8% of chunks (rising with distance) spawn a **mega**: a
Colossal boss with ~10× health, heavier hits, and a huge presence (pulsing aura,
always-visible HP bar; the HUD flags it with ☠ MEGA and shows what to `use:`).
Crucially, **resistances are amplified for megas** — the wrong damage type does
just **0.12×** (you'll die before you dent it) while its weakness does **3×**. So
a mega forces the right weapon/element choice. Felling one is a real haul: **two
ancient chests, a health fountain, and a big ammo pile**.

### Points of interest & events (`spawn_chunk` in `lib.rs`)
Beyond monsters and chests, chunks occasionally seed **optional encounters** —
each deterministic from the chunk seed and, where it grants a reward,
**remembered once resolved** (persisted in the save) so you can't reload to farm
it:

- **Arenas** — a two-ring *survive the waves* challenge on open ground. Stepping
  in **consumes it** and starts timed waves (a Colossus finale deep out);
  clearing every wave pays a cache. Leaving the ring or pressing `Esc` forfeits.
  Two arenas never spawn overlapping.
- **Cursed relics** — a dark chest that begins a high-risk **speed sprint**:
  you move faster, but a relentless, capped pack of **hunters** chases you until
  you bank enough distance (or die). Only one curse runs at a time; a rift can't
  outrun it, but an arena's sealed ring holds it at bay.
- **Champions** — a lone named elite guarding a hoard. Ambient monsters are kept
  clear for a fair **1v1**; felling it drops the prize and it never respawns.
- **Rune vaults** — a light Simon-says rune puzzle (the world **pauses** while
  you solve). Repeat the growing pattern to crack it for a boosted cache; one
  wrong rune seals it.
- **Rifts** — step through to **leap ~400 tiles outward** toward the goal, but
  you arrive in a higher danger tier with a champion or ambush waiting and no
  checkpoint banked.
- **Cursed fog (miasma)** — sight shrinks and monsters lurk unseen; a premium
  cache waits at its heart.
- **Shield shrines** — touch for a one-time blue **ward** that soaks damage
  until drained.
- **Offering shrines** — sacrifice unwanted inventory items for a reward.
- **Campfires** — rest to trickle-heal safely to half; pushing higher risks an
  ambush.
- **Fishing** — at a calm water's edge, cast (`F` / the mobile Fish button) and
  time the reel.

Milestone showers (`scatter_view_loot`) drop **fields** of some of these as
visual progress markers — shield wards at 25k, a trove of ancient chests at 50k,
a field of rifts at 75k.

### Audio (`src/audio.ts`)
Fully synthesized (Web Audio, no asset files). Combat/monster SFX are derived
from snapshot diffs in `main.ts` (a shot when ammo drops, a death when a monster
vanishes, hurt/heal on hp changes, a roar when a mega appears, etc.). A very
subtle ambient music bed picks its root/scale from the **biome under the player**
and swells with **combat tension** (calm → combat → boss urgency → celebration).
Unlocked by the menu "Play" click (browser autoplay policy); `M` mutes.

### The 100,000 celebration (`start_celebration` in `lib.rs`)
Reaching 100,000 tiles fires a one-time **flash mob**: a crowd of monsters and
bosses gathers and everyone dances (bobbing + confetti) to upbeat music while an
overlay tallies your run — distance record, steps taken, play time, monsters
slain, deaths, chests opened. After ~22s the music stops and the wilds swarm
(you'll likely die and respawn at the 100,000 checkpoint). **The game never
ends** — keep pushing for a higher number. Test it with `?seed=1&warp=100000`.
(`CELEBRATE_DIST` in `lib.rs`; further celebrations may be added if playtests
show players pushing well past it.)

### Save & the 4-player distance challenge (`src/save.ts`, menu in `main.ts`)
On load you get a **slot menu**: 4 save slots so one browser can host a
**4-player distance challenge**. Each slot has its own world/character; the menu
shows every slot's best distance (a scoreboard, leader marked ★) and a **Reset**
button. Press **Esc** in-game to return to the menu and hand off to the next
player. `?seed=N` in the URL is a slotless dev/determinism mode.

Persistence is `localStorage` only. Persists the minimum — world seed, position, skill XP,
inventory (as weapon seeds), equipped slots, ammo, and the **farthest distance
ever reached** (`max_dist`). That record is the primary run-to-run achievement:
it survives death, so every respawn is a push to beat your best (shown as
`★ Best` in the HUD).

**Checkpoints** avoid the long walk back: pushing past each multiple of
`CHECKPOINT` distance (250 tiles, `lib.rs`) banks a respawn point at that spot,
so death returns you to your furthest checkpoint (`⚑ Checkpoint` in the HUD)
instead of the origin. Lower the constant for more frequent checkpoints.

The world regenerates from the seed. Load `?seed=N` in the URL to start a
specific fresh world.

## Where to tweak the rules

| Want to change… | Edit |
|---|---|
| terrain thresholds / new biomes | `tile_at` and the tables in `game/world.rs` |
| add a prefix/element/suffix or weapon base | the `PREFIXES` / `ELEMS` / `SUFFIXES` / `BASES` tables in `game/weapon.rs` |
| monster bodies, stats, naming | `BODIES` and `generate` in `game/monster.rs` |
| drop rates (ammo/weapon/health) | the drop rolls in `hit_monster` in `game/lib.rs` |
| starting ammo / ammo per shot | `Player::new` and `player_attack` in the Rust sources |
| weakness/resistance math | `elem_mult2` in `game/monster.rs` |
| mega frequency / power | the mega roll in `spawn_chunk` and `make_mega` |
| skill curve / bonuses | `skill_level` / `skill_bonus` in `game/player.rs` |
| difficulty pacing | `difficulty_at` in `game/lib.rs` |
| colors / pixel look | palettes in `src/render.ts` |

After editing Rust, rebuild the wasm (`npm run wasm`, or just `npm run dev`).
