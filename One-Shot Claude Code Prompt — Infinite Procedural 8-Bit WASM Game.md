You are Claude Code. Build this project completely from scratch.

# Project: Infinite Procedural Wilderness

Create the smallest practical codebase for a playable, top-down, 8-bit-style procedural action RPG that runs as a WebAssembly application in Chromium.

The guiding principle is:

> Build the smallest engine that can generate the game, not a large game made from hand-authored content.

Do not over-engineer. Do not introduce frameworks or abstractions unless they materially reduce complexity.

## Core Experience

The player starts at coordinate `(0,0)` in an effectively infinite procedurally generated world.

There are:

- Procedurally generated terrain
- Procedurally generated monsters
- Procedurally generated weapons
- Procedurally generated item properties
- Skill-based character progression
- Increasing difficulty with distance from origin
- Combat
- Exploration
- Loot
- Permanent character progression

The world has **terrain but no buildings, dungeons, interiors, doors, towns, rooms, or structures to enter**.

Think:

- Dungeon Siege's top-down action/RPG feel
- classic 8-bit/16-bit procedural games
- Minecraft-like infinite coordinates
- roguelike procedural generation
- Diablo-like randomized loot
- but dramatically smaller in implementation

The player should feel like they are wandering through an infinite wilderness where everything is generated from rules.

---

# HARD CONSTRAINT: MINIMAL CODEBASE

Optimize aggressively for simplicity.

Prefer:

- plain data structures
- deterministic functions
- small modules
- pure functions
- integer coordinates
- fixed-size arrays where practical
- procedural drawing
- seeded PRNG
- simple collision
- simple game loop

Avoid:

- React
- Vue
- Svelte
- ECS frameworks
- physics engines
- rendering engines
- asset pipelines
- sprite editors
- databases
- backend servers
- authentication
- multiplayer
- procedural-generation libraries
- dependency-heavy libraries
- unnecessary classes
- dependency injection
- event-bus architectures
- elaborate design patterns

Use browser APIs directly.

The final project should be small enough that a developer can understand essentially the entire game by reading the source.

---

# TECHNOLOGY

Use:

- TypeScript
- Vite
- WebAssembly
- a very small WASM module written in Rust
- HTML/CSS
- Canvas 2D initially

The architecture should deliberately keep the WASM boundary simple.

Prefer:

```text
Browser
  |
  v
TypeScript
  |
  | input / rendering / presentation
  v
WASM
  |
  | deterministic simulation
  v
Game State
```

The WASM module should own the deterministic game simulation.

TypeScript should primarily:

- create the canvas
- collect keyboard/mouse input
- call WASM
- obtain the current visible game state
- render it
- display minimal HUD information

Do not move rendering into WASM unless there is a compelling reason.

---

# WORLD

The world is infinite.

Use integer world coordinates.

The player can theoretically travel indefinitely in all directions.

Do NOT generate the entire world.

Generate only the visible area plus a small margin around it.

Use deterministic generation:

```text
worldSeed + worldX + worldY
```

must always produce the same terrain.

The same location must therefore always regenerate identically.

The player should be able to walk away and return to an area and find the same terrain and naturally regenerated entities.

Use a simple chunk system.

For example:

```text
CHUNK_SIZE = 32
```

Only a small number of chunks surrounding the camera/player need to exist at any time.

Do not persist every visited chunk.

The world should be reconstructable from:

```text
seed
+
coordinates
+
procedural rules
```

---

# TERRAIN

Terrain should be visually simple.

Start with approximately 8–12 terrain types, such as:

- deep water
- shallow water
- sand
- grass
- dense grass
- dirt
- rock
- mountain
- snow
- swamp

Use deterministic noise or layered hash functions to generate large-scale geographic regions.

Do not add complicated erosion simulation.

Terrain should have:

- movement cost
- passability
- visual tile
- environmental difficulty

Terrain should naturally form geographic regions rather than random checkerboard noise.

For example:

```text
noise → temperature
noise → moisture

temperature + moisture → biome
```

Keep the implementation simple.

---

# DISTANCE DIFFICULTY

Distance from origin is the primary difficulty mechanic.

Calculate:

```text
distance = sqrt(x² + y²)
```

or an inexpensive approximation.

Difficulty should increase gradually with distance.

Use distance to influence:

- monster level
- monster health
- monster damage
- monster rarity
- weapon rarity
- weapon power
- monster spawn frequency
- environmental hazards

The origin should be relatively safe.

Farther away should become increasingly dangerous.

There should be no arbitrary maximum level.

The procedural system should theoretically continue indefinitely.

Avoid exponential runaway values.

Use controlled scaling functions such as:

```text
difficulty = 1 + floor(distance / SCALE)
```

with occasional nonlinear scaling for very distant regions.

---

# MONSTERS

Monsters must be procedurally generated.

Do not create hundreds of hand-authored monster definitions.

Instead create a small set of fundamental traits.

For example:

```text
body
movement
attack
element
defense
temperament
weakness
```

A monster is generated by combining traits.

Example conceptual generation:

```text
fast + insect + poison + fragile
slow + armored + fire + vulnerable_to_cold
swarming + beast + physical + low_health
ranged + creature + acid + weak_defense
```

The exact implementation is up to you.

The important requirement:

> A monster should be the result of a procedural recipe, not a hand-authored entity.

Monster generation must be deterministic from:

```text
world seed
monster position
monster generation seed
world difficulty
```

Each monster should have:

- generated name
- level
- health
- attack
- defense
- movement speed
- attack behavior
- elemental affinity
- weakness
- XP value
- loot tendency

---

# MONSTER STRENGTHS AND WEAKNESSES

This is a central mechanic.

Every monster should have meaningful strengths and weaknesses.

Examples:

```text
Fire creature
strong vs fire
weak vs cold

Armored creature
high physical defense
weak to piercing

Fast creature
high movement speed
low health

Poison creature
poison attacks
weak to fire

Regenerating creature
slow movement
high regeneration
weak to burst damage
```

The player should be encouraged to adapt weapons and skills to the enemies they encounter.

Do not make this into a giant elemental RPG system.

A small number of interacting damage types is sufficient.

---

# WEAPONS

Weapons are procedurally generated.

Do not have a giant predefined weapon database.

Instead define a small set of weapon primitives.

For example:

```text
weapon type
damage type
damage
attack speed
range
projectile behavior
special modifier
```

Then procedurally combine them.

Examples:

```text
Iron Axe
+ slow
+ high physical damage

Frost Bow
+ cold damage
+ long range
+ slow projectile

Venom Blade
+ poison
+ fast attack

Piercing Spear
+ piercing
+ armor penetration
```

Each generated weapon should have a deterministic seed.

Weapon generation should depend on:

```text
world seed
location
loot seed
difficulty
```

Weapons should therefore feel unique without requiring manually authored loot.

---

# PROCEDURAL ITEM RULESET

Implement the item system as rules rather than content.

Conceptually:

```text
Base Weapon
    +
Prefix
    +
Suffix
    +
Affixes
    =
Generated Weapon
```

But keep the implementation tiny.

For example:

```text
prefix → damage modifier
suffix → attack-speed modifier
element → damage type
rarity → number of modifiers
```

The system should be extensible by editing a small rules table.

A developer should be able to add:

```text
"Frozen"
"Jagged"
"Heavy"
"Venomous"
"Piercing"
```

without rewriting weapon-generation logic.

---

# SKILL-BASED PROGRESSION

Do NOT use a traditional character-level system as the primary progression mechanism.

Skills improve through use.

Examples:

```text
Swords
Bows
Axes
Fire
Cold
Poison
Defense
Movement
```

If the player repeatedly uses swords:

```text
sword skill ↑
```

If they repeatedly use bows:

```text
bow skill ↑
```

If they repeatedly use fire damage:

```text
fire skill ↑
```

Skills should directly influence effectiveness.

For example:

```text
skill → damage
skill → attack speed
skill → accuracy
skill → cooldown
```

Keep this mathematically simple.

The player should become better at what they actually use.

---

# COMBAT

Combat should be extremely simple.

Top-down real-time combat.

Support:

- movement
- basic attack
- weapon switching
- enemy contact/collision
- enemy attacks
- projectiles where appropriate
- damage
- death
- loot
- XP/skill progression

Do not build a complex animation system.

Use simple procedural shapes/pixels.

---

# INPUT

Desktop keyboard + mouse.

Minimum controls:

```text
WASD / Arrow Keys = movement
Mouse = aim
Left Mouse = attack
1–4 = weapon slots
I = inventory
```

You may simplify controls if necessary.

The game must be playable without a controller.

---

# GRAPHICS

Strong 8-bit aesthetic.

Do not create external art assets.

Everything should be rendered procedurally.

Use:

- rectangles
- simple pixel shapes
- limited palette
- nearest-neighbor scaling
- small logical resolution

Consider rendering at something like:

```text
320 × 180
```

and scaling to the browser window.

This should make the game look intentionally pixelated.

Terrain should be recognizable through simple patterns.

Characters and monsters can be tiny procedural pixel figures.

Weapons can be simple colored/shape representations.

No asset files should be required for the core game.

---

# CAMERA

Camera follows the player.

The world should appear continuous while the player moves.

Use world coordinates internally.

Render coordinates should be:

```text
screen = world - camera
```

The player should remain near the center of the screen.

---

# WORLD ENTITIES

Do not spawn every monster in the entire world.

Generate nearby entities from deterministic seeds.

When the player enters a region:

```text
chunk seed
→ spawn rules
→ entities
```

When they leave:

```text
discard runtime entities
```

When they return:

```text
regenerate
```

This is acceptable because the world is primarily procedural.

If persistent state is necessary, keep it minimal.

---

# SAVE GAME

Do not implement cloud saves.

Use browser local storage only.

Persist the minimum:

```text
world seed
player position
skills
inventory
equipped weapons
```

The world itself should not need to be saved.

---

# ARCHITECTURE

Aim for something approximately like:

```text
src/
  main.ts
  render.ts
  input.ts
  wasm.ts
  ui.ts

game/
  lib.rs
  world.rs
  monster.rs
  weapon.rs
  player.rs

index.html
style.css
Cargo.toml
package.json
vite.config.ts
```

However, this is NOT a requirement.

If fewer files produce a cleaner result, use fewer files.

The architecture should emerge from simplicity rather than ceremony.

---

# WASM API

Keep the WASM interface extremely small.

Prefer a small number of functions such as:

```text
init(seed)
update(input, dt)
get_state()
```

or an equally simple interface.

Do not expose dozens of tiny WASM functions.

Prefer one compact serialized state representation or a shared linear-memory representation over a complicated object API.

The TypeScript side should not need to understand the simulation internals.

---

# DETERMINISM

This is extremely important.

Given:

```text
world seed
+
world coordinates
+
entity seed
```

generation must be deterministic.

Do NOT use JavaScript's `Math.random()` for world generation.

Use a deterministic PRNG/hash implementation in WASM.

The same seed should produce the same universe.

---

# PERFORMANCE

The game should comfortably run at 60 FPS in Chromium.

Do not optimize prematurely.

But avoid obvious mistakes:

- no allocations every frame where avoidable
- no generation of the entire world
- no huge object graphs
- no per-pixel DOM rendering
- no React
- no SVG entity rendering
- no unnecessary serialization

Canvas should render only what is visible.

---

# DEVELOPMENT EXPERIENCE

The project must run with:

```bash
npm install
npm run dev
```

and have a straightforward WASM build process.

Provide:

```bash
npm run build
```

which produces the complete deployable application.

The project should work in Chromium-based browsers.

---

# FIRST PLAYABLE VERSION

Do NOT attempt to implement every possible feature before testing the core loop.

The implementation sequence should be:

1. Vite + TypeScript shell
2. Minimal Rust WASM module
3. Canvas rendering
4. Infinite coordinate system
5. Deterministic terrain
6. Player movement
7. Camera
8. Procedural monsters
9. Combat
10. Procedural weapons
11. Skill progression
12. Loot
13. Difficulty scaling
14. Minimal HUD
15. Local save

At every stage, keep the game playable.

---

# DESIGN TEST

When complete, I should be able to:

1. Start the game.
2. See an infinite procedurally generated wilderness.
3. Walk in any direction indefinitely.
4. See terrain change naturally.
5. Encounter procedurally generated monsters.
6. Fight them.
7. Kill them.
8. Receive procedurally generated weapons/items.
9. Discover that different weapons are useful against different monsters.
10. Improve skills by using weapons/abilities.
11. Travel farther from `(0,0)` and encounter progressively stronger enemies.
12. Return toward the origin and notice that the world is safer.
13. Reload the page and have the same world regenerate from the same seed.

---

# IMPORTANT IMPLEMENTATION PHILOSOPHY

Do not interpret this as a request for a large RPG.

Interpret it as:

> A tiny deterministic procedural simulation that happens to be a game.

If you can remove a system without damaging the core experience, remove it.

If a feature can be represented as data plus a rule, do that instead of writing bespoke code.

If ten monster types can be represented by five traits and a generator, use the generator.

If a hundred weapons can be represented by five weapon bases and procedural affixes, use the generator.

If terrain can be generated from two noise fields and a lookup table, do that.

If a sophisticated architecture requires more code than the feature itself, reject the architecture.

Prefer:

```text
RULES → GENERATORS → STATE → RENDER
```

over:

```text
CONTENT → OBJECTS → MANAGERS → SERVICES → SYSTEMS → FRAMEWORK
```

---

# FINAL REQUIREMENT

Before finishing:

- run the project
- build it
- fix all TypeScript errors
- fix all Rust/WASM errors
- verify the browser application launches
- verify player movement
- verify procedural terrain
- verify procedural monsters
- verify combat
- verify procedural loot
- verify skill progression
- verify increasing difficulty
- verify deterministic regeneration

Then provide a concise README explaining:

- architecture
- how to run
- how procedural terrain works
- how monster generation works
- how weapon generation works
- how skill progression works
- how difficulty scales with distance
- where to modify the procedural rules

Do not add future-feature placeholders unless they are genuinely useful.

Build the smallest complete version first.