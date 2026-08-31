//! WebWander — deterministic procedural wilderness simulation (WASM side).
//!
//! The whole simulation lives here and in the sibling modules. TypeScript only
//! feeds input, ticks `update`, and reads one serialized snapshot per frame.
//!
//! WASM ABI (raw, no wasm-bindgen):
//!   init(seed)                         start a fresh world
//!   set_input(keys, aimx, aimy, atk, slot)
//!   update(dt_ms)                      advance one frame, rebuild snapshot
//!   snapshot_ptr() / snapshot_len()    visible state for rendering + HUD
//!   equip(inv_idx, slot)               equip an inventory item to a slot
//!   save_ptr() / save_len()            persistent blob (seed+pos+skills+items)
//!   io_ptr() / io_cap()                scratch buffer TS writes into
//!   load_save(len)                     restore from bytes placed in io buffer

mod monster;
mod player;
mod rng;
pub mod sim;
mod weapon;
mod world;

use monster::{elem_mult2, generate as gen_monster, make_champion, make_mega, monster_seed, Monster};
use player::{skill_bonus, skill_level, Player};
use rng::{hash2, Rng};
use weapon::{generate as gen_weapon, generate_unique, Weapon, SP_ARMOR_PEN, SP_POISON_DOT};
use world::{feature_at, hazard, move_cost, passable_px, tile_at, tile_px, TILE};

// Damage types (indices shared with the weapon/monster tables).
pub const PHYS: u8 = 0;
pub const FIRE: u8 = 1;
pub const COLD: u8 = 2;
pub const POISON: u8 = 3;
pub const PIERCE: u8 = 4;

// Skill indices.
pub const SK_SWORD: u8 = 0;
pub const SK_BOW: u8 = 1;
pub const SK_AXE: u8 = 2;
pub const SK_FIRE: u8 = 3;
pub const SK_COLD: u8 = 4;
pub const SK_POISON: u8 = 5;
pub const SK_DEFENSE: u8 = 6;
pub const SK_MOVE: u8 = 7;

const LOGICAL_W: f32 = 320.0;
const LOGICAL_H: f32 = 180.0;
const CHUNK: i64 = 32; // tiles
const CHUNK_PX: f32 = CHUNK as f32 * TILE;
const PLAYER_R: f32 = 4.0;
const BASE_SPEED: f32 = 64.0;
/// Max Move-skill speed bonus (+160% → 2.6× base ≈ 166 px/s on open ground,
/// ~2s to cross the 320px viewport). Keeps top speed controllable.
const MOVE_BONUS_CAP: f32 = 1.6;
const AGGRO: f32 = 150.0;
/// Distance (in tiles, i.e. HUD "Dist" units) between respawn checkpoints. Push
/// past a multiple of this and you bank a new checkpoint to respawn at.
const CHECKPOINT: f32 = 250.0;
/// The mythical goal. Reaching it triggers a one-time celebration (flash mob +
/// stats) before the wilds turn hostile again. The game never ends.
const CELEBRATE_DIST: f32 = 100_000.0;
// One-time flags for the off-the-base-10-grid milestone showers.
const MILESTONE_25K: u32 = 1 << 15; // 25,000: a field of shield wards
const MILESTONE_50K: u32 = 1 << 16; // 50,000: a trove of ancient chests
const MILESTONE_75K: u32 = 1 << 17; // 75,000: a field of teleporters
/// Generous inventory cap — inventory management isn't the point of a fast
/// explore game, but the list can't be unbounded.
const INVENTORY_CAP: usize = 60;
/// How many recently-looted chest/fountain chunks to remember (bounded so the
/// save stays small; distant ones eventually regenerate).
const LOOTED_CAP: usize = 96;
/// Two-ring arena, wrestling-style. The **inner** ring is the combat zone: mobs
/// are confined here and you commit by stepping in. The **outer** ring is the
/// apron — extra room to move that costs health (rot), keeping mobs out but
/// making a retreat a real trade-off. Both are fixed world sizes so the arena
/// plays identically on every device (only how much of the apron is on-screen
/// varies; the apron is allowed to spill off-screen on short landscape views).
const ARENA_INNER: f32 = 128.0;
const ARENA_OUTER: f32 = 200.0;
// Keep arenas apart: never spawn one whose outer ring would touch another's.
// "Connected" arenas render superimposed and force a no-breather chain (clearing
// one instantly consumes the next), so we suppress the overlap at spawn.
const ARENA_MIN_SEP: f32 = 2.0 * ARENA_OUTER;
/// How close to an idle ring the player must be for the entry telegraph (a
/// heads-up prompt + a pulsing ring) to show.
const ARENA_TELEGRAPH: f32 = ARENA_OUTER + 80.0;
/// Brief pause between cleared arena waves.
/// Ready-steady-go countdown (seconds) before each wave spawns, so the player
/// can brace with the audio cues.
const ARENA_COUNTDOWN: f32 = 3.0;
/// From this difficulty tier up, an arena's final wave is a boss finale: a
/// Colossus plus a few minions (a focused fight), and the cache gets a bounty.
/// Gentle low-tier arenas near the origin skip the boss.
const ARENA_BOSS_TIER: u32 = 4;

const MIASMA_R: f32 = 150.0; // cursed-fog radius in world px (~9 tiles)

const DUEL_RADIUS: f32 = 220.0; // near a champion, ambient mobs are kept out

const VAULT_RADIUS: f32 = 30.0; // step this close to start the rune puzzle

const RIFT_RADIUS: f32 = 22.0; // step into a rift to leap forward
const RIFT_JUMP_TILES: f32 = 400.0; // how far ahead a rift throws you (toward the goal)
const RIFT_LANDING_SAFE: f32 = 44.0; // clear natural mobs this close to a rift landing (intended encounter spawns at 48px+)
const RIFT_HUNTER_CARRY: usize = 6; // relic hunters that tear through a rift after you (the curse can't be outrun; the rest of the pack is thinned)

// --- cursed relic ---------------------------------------------------------
/// Duration of the curse in cumulative steps (tiles moved, any direction).
const RELIC_STEPS: f32 = 2500.0;
const RELIC_SPEED_MULT: f32 = 1.85; // the player's blistering speed burst
const RELIC_MON_SPEED_MULT: f32 = 1.35; // monsters speed up too
const RELIC_HUNTER_CAP: usize = 50; // max live hunters at once (bounded growth)
const RELIC_HUNT_INTERVAL: f32 = 1.1; // seconds between hunter spawns
const RELIC_SHIELD_REGEN: f32 = 12.0; // blue shield regen per second
/// Hunters run at a static "move cost" relative to the cursed player's open
/// speed: below the grass/dirt cost (1.0) so you *gain* on those tiles, but
/// above every other tile's cost — so stepping off grass/dirt lets them close.
const RELIC_HUNTER_MOVE_COST: f32 = 1.1;
/// If a hunter falls this far behind (long grass stretches), it surges to close
/// half the gap so it's never lost for good.
const RELIC_HUNTER_FAR: f32 = 400.0;
const RELIC_TURBO_MULT: f32 = 4.0; // surge speed while catching up

// --- campfire / rest site -------------------------------------------------
/// How close you must be to a campfire to rest (trickle-heal).
const CAMPFIRE_REST_RADIUS: f32 = 26.0;
/// Healing rate while resting, as a fraction of max HP per second.
const CAMPFIRE_REGEN_FRAC: f32 = 0.06;
/// Per-second ambush probability at full HP (scales down to 0 at 50% HP).
const CAMPFIRE_AMBUSH_PER_SEC: f32 = 0.42;
/// Breather after an ambush before another can roll.
const CAMPFIRE_AMBUSH_COOLDOWN: f32 = 8.0;

/// A rest site (chunk-bound, like other structures).
struct Campfire {
    x: f32,
    y: f32,
    cx: i64,
    cy: i64,
}

/// How close you must be to make an offering at a shrine.
const SHRINE_RADIUS: f32 = 28.0;

// --- fishing --------------------------------------------------------------
/// Ammo spent as bait per cast.
const FISH_BAIT: u32 = 3;
/// No hostile monster may be within this radius to fish (fishing needs calm).
const FISH_SAFE_RADIUS: f32 = 140.0;

/// An offering shrine (chunk-bound): sacrifice junk items for a reward.
struct Shrine {
    x: f32,
    y: f32,
    cx: i64,
    cy: i64,
}

/// A patch of cursed fog: inside, the view shrinks and monsters lurk unseen
/// (client-side render). A premium cache waits at its heart.
struct Miasma {
    x: f32,
    y: f32,
    cx: i64,
    cy: i64,
    r: f32, // radius in world px
}

/// A rune vault: a light memory/sequence puzzle (solved in a client overlay)
/// opens a cache. Non-combat, but the world keeps running while you solve.
struct Vault {
    x: f32,
    y: f32,
    cx: i64,
    cy: i64,
    opened: bool,
}

/// A rift: step in to leap a big chunk of distance *toward the goal* — but you
/// arrive in a higher danger tier, and something is waiting.
struct Rift {
    x: f32,
    y: f32,
    cx: i64,
    cy: i64,
}

/// Per-second chance of an ambush while resting: zero up to 50% HP, then rising
/// linearly with how far above half you've healed (max at full HP).
fn campfire_ambush_chance(hp: f32, maxhp: f32) -> f32 {
    let over = (hp / maxhp.max(1.0) - 0.5).max(0.0); // 0 at <=50%, 0.5 at 100%
    (over / 0.5) * CAMPFIRE_AMBUSH_PER_SEC
}

/// A crafted "cursed relic" blade — the only weapon usable while the curse is
/// active. Long reach + fast so you can cleave a path through the hunt.
fn make_relic_weapon(power: f32) -> Weapon {
    Weapon {
        seed: 0,
        power,
        durability: 1.0,
        unique: false,
        base: 3, // spear reach
        dmg_type: PHYS,
        damage: 10.0 + power * 1.6,
        cooldown: 0.2,
        range: 42.0,
        ranged: false,
        proj_speed: 0.0,
        rarity: 3,
        class_skill: SK_SWORD,
        special: 0,
        name: "Cursed Relic".into(),
    }
}

/// Active-relic state. `Some` while the curse runs.
struct Relic {
    steps: f32,   // cumulative steps taken this curse (ends at RELIC_STEPS)
    shield: f32,  // blue shield HP that soaks damage before your health
    shield_max: f32,
    hunt_cd: f32, // countdown to the next hunter spawn
    weapon: Weapon,
    power: f32, // for save/regen
}

fn chunk_of(x: f32, y: f32) -> (i64, i64) {
    ((x / CHUNK_PX).floor() as i64, (y / CHUNK_PX).floor() as i64)
}

fn remember_chunk(list: &mut Vec<(i64, i64)>, c: (i64, i64)) {
    if !list.contains(&c) {
        list.push(c);
        if list.len() > LOOTED_CAP {
            list.remove(0);
        }
    }
}
const CELEBRATE_DUR: f32 = 22.0;

/// Escalating toasts at each power of ten — the pattern hints that something big
/// waits at 100,000.
const MILESTONE_LABELS: [&str; 6] = [
    "Welcome!!",
    "Starting!!",
    "Exploring!!",
    "Trailblazing!!",
    "Voyaging!!",
    "Legendary!!",
];

/// Distance from origin in tiles for a world-pixel position.
fn dist_tiles(x: f32, y: f32) -> f32 {
    ((x / TILE).powi(2) + (y / TILE).powi(2)).sqrt()
}

/// Speed (px/s) of a monster's projectile. Always faster than the firer so a
/// monster chasing the player can't outrun its own shots — otherwise fast
/// (high-level) monsters look like they're dropping things instead of firing
/// them forward. The floor keeps slow monsters' shots brisk.
fn monster_proj_speed(monster_speed: f32) -> f32 {
    (monster_speed * 1.4).max(62.0)
}

struct Proj {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    dmg: f32,
    dmg_type: u8,
    special: u8,
    class_skill: u8,
    from_player: bool,
    src_name: String, // the monster that fired it (empty for player shots)
}

/// What dealt a blow to the player, so a death can name its cause.
enum Hurt<'a> {
    Attack { name: &'a str, elem: u8, ranged: bool },
    Swamp,
}

/// A short, readable description of a killing blow for the respawn note.
fn hurt_desc(cause: Hurt) -> String {
    match cause {
        Hurt::Swamp => "the swamp's rot".to_string(),
        Hurt::Attack { name, elem, ranged } => {
            let nm = if name.is_empty() { "a monster" } else { name };
            let noun = match (ranged, elem) {
                (true, FIRE) => "fire bolt",
                (true, COLD) => "frost bolt",
                (true, POISON) => "venom spit",
                (true, PIERCE) => "quill",
                (true, _) => "bolt",
                (false, FIRE) => "burning strike",
                (false, COLD) => "frozen strike",
                (false, POISON) => "venomous bite",
                (false, PIERCE) => "gore",
                (false, _) => "strike",
            };
            format!("{nm}'s {noun}")
        }
    }
}

/// A ground item the player can walk over to collect.
enum Drop {
    Weapon { seed: u64, power: f32, rarity: u8 },
    Ammo(u32),
    Health(f32),
    Chest { seed: u64, power: f32 }, // ruins loot: a "unique" overpowered weapon
    Fountain,                        // rare structure: a full heal
    Relic { power: f32 },            // cursed relic: a high-risk speed sprint
    Shield { amount: f32 },          // shield shrine: a one-time non-recharging ward
}

struct Loot {
    x: f32,
    y: f32,
    kind: Drop,
}

/// A one-time milestone shower scattered across the player's current view.
enum Gift {
    Fountains,  // 1,000: healing springs everywhere
    Ammo(u32),  // 10,000: a total pile of ammunition split into stacks
    Shields,    // 25,000: a field of shield wards (non-additive; a progress marker)
    Chests,     // 50,000: a trove of ancient (unique) chests
    Rifts,      // 75,000: a field of teleporters (only one can be used)
}

/// An optional "survive the waves" point of interest. Stepping into the ring is
/// a real commitment: it's consumed on entry (can't be paused/reloaded into a
/// retry), and only a full clear pays out the loot cache.
#[derive(Clone, Copy, PartialEq, Debug)]
enum ArenaState {
    Idle,    // dormant ring, waiting to be entered
    Active,  // waves in progress
    Cleared, // all waves survived — conquered (inert)
    Done,    // forfeited / died — abandoned (inert)
}

struct Arena {
    x: f32,
    y: f32,
    cx: i64, // home chunk (for streaming retention)
    cy: i64,
    seed: u64, // deterministic wave composition
    state: ArenaState,
    wave: u8,  // current wave (1-based once Active)
    waves: u8, // total waves to survive
    countdown: f32, // ready-steady-go timer before the next wave spawns (0 = fighting)
}

#[derive(Default, Clone, Copy)]
struct Input {
    keys: u32,
    aimx: f32,
    aimy: f32,
    attack: bool,
}

pub struct Game {
    seed: u64,
    player: Player,
    monsters: Vec<Monster>,
    loaded: Vec<(i64, i64)>,
    projectiles: Vec<Proj>,
    loot: Vec<Loot>,
    arenas: Vec<Arena>,
    campfires: Vec<Campfire>, // chunk-bound rest sites
    shrines: Vec<Shrine>,     // chunk-bound offering shrines
    miasmas: Vec<Miasma>,     // chunk-bound cursed-fog patches
    vaults: Vec<Vault>,       // chunk-bound rune puzzle vaults
    rifts: Vec<Rift>,         // chunk-bound forward-leap portals
    resting: bool,            // adjacent to a campfire this frame (for the HUD)
    campfire_ambush_cd: f32,  // breather countdown after an ambush
    dueling: bool,            // currently in a champion's 1v1 (for the intro toast)
    force_fish: bool,         // dev/testing: fishing always available
    event_rng: u64,           // real-time event rolls (ambush); not serialized
    relic: Option<Relic>, // Some while the cursed-relic sprint is active
    // Chunks whose chest / fountain / arena / relic has been collected or entered.
    // Persisted so you can't reload on top of one to farm it — it's spent on
    // return. Arenas are marked the moment you step in (consume-on-entry).
    looted_chests: Vec<(i64, i64)>,
    looted_fountains: Vec<(i64, i64)>,
    looted_arenas: Vec<(i64, i64)>,
    looted_relics: Vec<(i64, i64)>,
    looted_shields: Vec<(i64, i64)>, // spent shield shrines (no reload farm)
    looted_champions: Vec<(i64, i64)>, // felled champions (no reload farm)
    looted_vaults: Vec<(i64, i64)>,  // opened rune vaults (no reload farm)
    looted_rifts: Vec<(i64, i64)>,   // used rifts (no re-leap farm)
    input: Input,
    message: String,
    msg_t: f32,
    last_kill: Option<String>, // what dealt the fatal blow, for the respawn note
    snap: Vec<u8>,
    save: Vec<u8>,
    // Lifetime counters. kills/dmg_taken/healed/deaths are used by the headless
    // simulator; the persisted stats (steps/chests/play_secs) feed the
    // 100,000 celebration.
    pub kills: u32,
    pub mega_kills: u32, // bosses (Colossi) felled — shown in the 100k stats
    pub dmg_taken: f32,
    pub healed: f32,
    pub deaths: u32,
    pub chests_opened: u32,
    pub fountains_used: u32,
    pub steps: f64,     // total distance walked, in tiles
    pub play_secs: f32, // total play time
    celebrating: bool,  // 100,000 flash-mob in progress
    celebrate_t: f32,
    celebrated: bool,   // once-per-character: has the 100k party already fired
    milestone_mask: u32, // which milestone toasts / celebration showers have fired
    milestone_t: f32,    // brief pulse while a mini-milestone toast is up
    view_h: f32,         // logical viewport height (px); set from the device aspect
    godmode: bool,       // dev/testing only: player takes no damage
}

// ---------------------------------------------------------------------------
// Difficulty
// ---------------------------------------------------------------------------

fn difficulty_at(x: f32, y: f32) -> u32 {
    let d = dist_tiles(x, y);
    // Sub-linear curve: near the origin it rises ~1 level per 22 tiles (linear
    // slope), but it compresses far out so the danger doesn't run away. A
    // 1,000,000-tile journey lands around Lv 1500 — a finite, brutal, only-with-
    // a-lucky-seed-and-maxed-gear frontier rather than an astronomical wall.
    //   L(100)=5  L(1k)=30  L(10k)=128  L(100k)=451  L(1M)=~1500
    (1.0 + 25.0 * ((1.0 + d / 270.0).sqrt() - 1.0)) as u32
}

/// Number of connected passable tiles a spawn tile must reach to count as "open"
/// — enough elbow room to walk out, so we never drop the player onto a lone
/// passable tile ringed by deep water/mountains.
const SPAWN_MIN_OPEN: usize = 24;

/// True if the passable tile at `(tx, ty)` belongs to an open region of at least
/// `SPAWN_MIN_OPEN` connected passable tiles (4-neighbour flood, bounded so it
/// stays cheap). A one- or two-tile island in deep water fails this.
fn open_enough(seed: u64, tx: i64, ty: i64) -> bool {
    let mut seen: Vec<(i64, i64)> = vec![(tx, ty)];
    let mut stack: Vec<(i64, i64)> = vec![(tx, ty)];
    while let Some((cx, cy)) = stack.pop() {
        for (nx, ny) in [(cx + 1, cy), (cx - 1, cy), (cx, cy + 1), (cx, cy - 1)] {
            if world::passable(tile_at(seed, nx, ny)) && !seen.contains(&(nx, ny)) {
                seen.push((nx, ny));
                if seen.len() >= SPAWN_MIN_OPEN {
                    return true;
                }
                stack.push((nx, ny));
            }
        }
    }
    false
}

/// Nearest *open* world-pixel position to `(x, y)` (spiral tile search).
/// Guarantees the player never spawns/respawns trapped in water or mountains,
/// nor stranded on a passable-but-enclosed island they can't move off of.
fn safe_spawn(seed: u64, x: f32, y: f32) -> (f32, f32) {
    let tx0 = (x / TILE).floor() as i64;
    let ty0 = (y / TILE).floor() as i64;
    let center = |tx: i64, ty: i64| (tx as f32 * TILE + TILE * 0.5, ty as f32 * TILE + TILE * 0.5);
    // Nearest passable tile of any kind, as a last resort if nothing is open.
    let mut fallback: Option<(i64, i64)> = None;
    for ring in 0..128i64 {
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                // Only the outer edge of each ring (cheap outward spiral).
                if dx.abs() != ring && dy.abs() != ring {
                    continue;
                }
                let (tx, ty) = (tx0 + dx, ty0 + dy);
                if world::passable(tile_at(seed, tx, ty)) {
                    if open_enough(seed, tx, ty) {
                        return center(tx, ty);
                    }
                    if fallback.is_none() {
                        fallback = Some((tx, ty));
                    }
                }
            }
        }
    }
    match fallback {
        Some((tx, ty)) => center(tx, ty),
        None => (x, y),
    }
}

fn elem_skill(dmg_type: u8) -> Option<u8> {
    match dmg_type {
        FIRE => Some(SK_FIRE),
        COLD => Some(SK_COLD),
        POISON => Some(SK_POISON),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Game
// ---------------------------------------------------------------------------

impl Game {
    fn new(seed: u64) -> Self {
        let mut g = Game {
            seed,
            player: Player::new(),
            monsters: Vec::new(),
            loaded: Vec::new(),
            projectiles: Vec::new(),
            loot: Vec::new(),
            arenas: Vec::new(),
            campfires: Vec::new(),
            shrines: Vec::new(),
            miasmas: Vec::new(),
            vaults: Vec::new(),
            rifts: Vec::new(),
            resting: false,
            campfire_ambush_cd: 0.0,
            dueling: false,
            force_fish: false,
            event_rng: seed ^ 0xA11B_0DA5_C0DE_1234,
            relic: None,
            looted_chests: Vec::new(),
            looted_fountains: Vec::new(),
            looted_arenas: Vec::new(),
            looted_relics: Vec::new(),
            looted_shields: Vec::new(),
            looted_champions: Vec::new(),
            looted_vaults: Vec::new(),
            looted_rifts: Vec::new(),
            input: Input::default(),
            message: String::from("You awaken in the wilds."),
            msg_t: 4.0,
            last_kill: None,
            snap: Vec::new(),
            save: Vec::new(),
            kills: 0,
            mega_kills: 0,
            dmg_taken: 0.0,
            healed: 0.0,
            deaths: 0,
            chests_opened: 0,
            fountains_used: 0,
            steps: 0.0,
            play_secs: 0.0,
            celebrating: false,
            celebrate_t: 0.0,
            celebrated: false,
            milestone_mask: 0,
            milestone_t: 0.0,
            view_h: LOGICAL_H,
            godmode: false,
        };
        // Start with a basic Sword so combat is immediately viable.
        g.grant_basic_sword(1.0);
        // Never start trapped in water/mountains at the origin.
        let (sx, sy) = safe_spawn(seed, 0.0, 0.0);
        g.player.x = sx;
        g.player.y = sy;
        g.player.refresh_maxhp();
        g.player.hp = g.player.maxhp;
        g
    }

    /// Grant a basic Sword scaled to `power`, equipped into the first free slot.
    /// Used at the start and as a safety net when a run's last weapon breaks. We
    /// search seeds (deterministically) for a common sword rather than
    /// hand-authoring one, so it still regenerates from its seed on load.
    fn grant_basic_sword(&mut self, power: f32) {
        let mut ws = self.seed ^ 0x57A2_7C31 ^ ((power as u64).wrapping_mul(0x0100_0000_01B3));
        let p = power.max(1.0);
        // Search for a humble Sword (base 0, low rarity). Rarity is nudged up by
        // `power`, so past ~power 40 a common roll is unreachable — the search must
        // be bounded or it spins forever. Keep the lowest-rarity Sword we find as
        // the fallback so a high-distance refit still terminates.
        let mut best: Option<Weapon> = None;
        for _ in 0..256 {
            let cand = gen_weapon(ws, p);
            if cand.base == 0 {
                if cand.rarity <= 1 {
                    best = Some(cand);
                    break;
                }
                if best.as_ref().map_or(true, |b| cand.rarity < b.rarity) {
                    best = Some(cand);
                }
            }
            ws = ws.wrapping_add(0x9E37_79B1);
        }
        let w = best.unwrap_or_else(|| gen_weapon(ws, p));
        let idx = self.player.inv.len();
        self.player.inv.push(w);
        let mut slot = 0;
        for s in 0..4 {
            if self.player.equip[s] < 0 {
                slot = s;
                break;
            }
        }
        self.player.equip[slot] = idx as i32;
        self.player.slot = slot;
    }

    fn set_message(&mut self, m: String) {
        self.message = m;
        self.msg_t = 3.5;
    }

    // --- chunk streaming -------------------------------------------------

    fn player_chunk(&self) -> (i64, i64) {
        (
            (self.player.x / CHUNK_PX).floor() as i64,
            (self.player.y / CHUNK_PX).floor() as i64,
        )
    }

    fn stream_chunks(&mut self) {
        let (pcx, pcy) = self.player_chunk();
        // Despawn monsters and forget chunks outside a 3x3 window — but relic
        // hunters ignore this and pursue until killed (or the curse lifts).
        let in_range = |cx: i64, cy: i64| (cx - pcx).abs() <= 1 && (cy - pcy).abs() <= 1;
        self.monsters.retain(|m| m.hunter || in_range(m.cx, m.cy));
        self.loaded.retain(|&(cx, cy)| in_range(cx, cy));
        // Uncollected ground drops despawn once their chunk unloads, matching
        // the "leave, it's gone" rule and keeping the loot list bounded.
        self.loot.retain(|l| {
            let cx = (l.x / CHUNK_PX).floor() as i64;
            let cy = (l.y / CHUNK_PX).floor() as i64;
            in_range(cx, cy)
        });
        // Arenas belong to their home chunk — walk far enough and the event is
        // gone (same "leave, it's gone" rule as loot).
        self.arenas.retain(|a| in_range(a.cx, a.cy));
        self.campfires.retain(|c| in_range(c.cx, c.cy));
        self.shrines.retain(|s| in_range(s.cx, s.cy));
        self.miasmas.retain(|m| in_range(m.cx, m.cy));
        self.vaults.retain(|v| in_range(v.cx, v.cy));
        self.rifts.retain(|rf| in_range(rf.cx, rf.cy));

        for dy in -1..=1 {
            for dx in -1..=1 {
                let cx = pcx + dx;
                let cy = pcy + dy;
                if !self.loaded.contains(&(cx, cy)) {
                    self.spawn_chunk(cx, cy);
                    self.loaded.push((cx, cy));
                }
            }
        }
    }

    fn spawn_chunk(&mut self, cx: i64, cy: i64) {
        let mut r = Rng::new(hash2(self.seed ^ 0x5A17_9E2B, cx, cy));
        let center_x = (cx as f32 + 0.5) * CHUNK_PX;
        let center_y = (cy as f32 + 0.5) * CHUNK_PX;
        let diff = difficulty_at(center_x, center_y);

        // Most chunks hold a few monsters so the world rarely feels empty;
        // spawn frequency creeps up with distance. The immediate origin stays
        // calm.
        let mut count = 1 + r.below(3); // 1..3
        if diff >= 6 {
            count += 1; // denser in dangerous territory
        }
        if diff <= 1 {
            count = count.min(1);
        }
        for i in 0..count {
            // Pick a passable tile within the chunk.
            let mut placed = false;
            for _ in 0..6 {
                let tx = cx * CHUNK + r.below(CHUNK as u32) as i64;
                let ty = cy * CHUNK + r.below(CHUNK as u32) as i64;
                if world::passable(tile_at(self.seed, tx, ty)) {
                    let x = tx as f32 * TILE + TILE * 0.5;
                    let y = ty as f32 * TILE + TILE * 0.5;
                    let d = difficulty_at(x, y);
                    let ms = monster_seed(self.seed, cx, cy, i);
                    self.monsters.push(gen_monster(ms, cx, cy, x, y, d));
                    placed = true;
                    break;
                }
            }
            let _ = placed;
        }

        // Rare mega-monster: a boss that surprises you and punishes the wrong
        // damage type. Only away from the safe origin; slightly more common
        // deeper out.
        if diff >= 3 && r.chance((0.02 + diff as f32 * 0.0015).min(0.08)) {
            for _ in 0..8 {
                let tx = cx * CHUNK + r.below(CHUNK as u32) as i64;
                let ty = cy * CHUNK + r.below(CHUNK as u32) as i64;
                if world::passable(tile_at(self.seed, tx, ty)) {
                    let x = tx as f32 * TILE + TILE * 0.5;
                    let y = ty as f32 * TILE + TILE * 0.5;
                    let d = difficulty_at(x, y);
                    let ms = monster_seed(self.seed, cx, cy, 900 + r.below(9));
                    let mut mon = gen_monster(ms, cx, cy, x, y, d);
                    make_mega(&mut mon);
                    self.monsters.push(mon);
                    break;
                }
            }
        }

        // Rare champion: a lone named elite that guards a prize — a fair 1v1
        // (ambient mobs are kept out during the fight). Once felled it drops its
        // hoard and won't respawn (dedup via looted_champions).
        if diff >= 2 && r.chance(0.02) && !self.looted_champions.contains(&(cx, cy)) {
            for _ in 0..8 {
                let tx = cx * CHUNK + r.below(CHUNK as u32) as i64;
                let ty = cy * CHUNK + r.below(CHUNK as u32) as i64;
                if world::passable(tile_at(self.seed, tx, ty)) && open_enough(self.seed, tx, ty) {
                    let x = tx as f32 * TILE + TILE * 0.5;
                    let y = ty as f32 * TILE + TILE * 0.5;
                    let d = difficulty_at(x, y);
                    let ms = monster_seed(self.seed, cx, cy, 700 + r.below(9));
                    let mut mon = gen_monster(ms, cx, cy, x, y, d);
                    make_champion(&mut mon);
                    self.monsters.push(mon);
                    break;
                }
            }
        }

        // Rare ruins: a chest holding an overpowered "unique" weapon, scaled to
        // this region's difficulty. Deterministic from the chunk seed — but not
        // if it's already been looted (no reload farming).
        if r.chance(0.05) && !self.looted_chests.contains(&(cx, cy)) {
            for _ in 0..8 {
                let tx = cx * CHUNK + r.below(CHUNK as u32) as i64;
                let ty = cy * CHUNK + r.below(CHUNK as u32) as i64;
                if world::passable(tile_at(self.seed, tx, ty)) {
                    let x = tx as f32 * TILE + TILE * 0.5;
                    let y = ty as f32 * TILE + TILE * 0.5;
                    let power = difficulty_at(x, y) as f32;
                    let seed = hash2(self.seed ^ 0xC4E5_7A11, cx, cy);
                    self.loot.push(Loot { x, y, kind: Drop::Chest { seed, power } });
                    break;
                }
            }
        }

        // Rare health fountains: a full restore. Precious given how scarce
        // healing is — and, like chests, gone once used (no reload-to-heal).
        if r.chance(0.04) && !self.looted_fountains.contains(&(cx, cy)) {
            for _ in 0..8 {
                let tx = cx * CHUNK + r.below(CHUNK as u32) as i64;
                let ty = cy * CHUNK + r.below(CHUNK as u32) as i64;
                if world::passable(tile_at(self.seed, tx, ty)) {
                    let x = tx as f32 * TILE + TILE * 0.5;
                    let y = ty as f32 * TILE + TILE * 0.5;
                    self.loot.push(Loot { x, y, kind: Drop::Fountain });
                    break;
                }
            }
        }

        // Rare arena: an optional "survive the waves" challenge on open ground.
        // Not near the calm origin; consumed on ENTRY (see update_arenas) so it
        // can't be paused/reloaded into a retry.
        if diff >= 2 && r.chance(0.02) && !self.looted_arenas.contains(&(cx, cy)) {
            for _ in 0..10 {
                let tx = cx * CHUNK + r.below(CHUNK as u32) as i64;
                let ty = cy * CHUNK + r.below(CHUNK as u32) as i64;
                if world::passable(tile_at(self.seed, tx, ty)) && open_enough(self.seed, tx, ty) {
                    let x = tx as f32 * TILE + TILE * 0.5;
                    let y = ty as f32 * TILE + TILE * 0.5;
                    // Never place a ring that touches an existing arena's ring —
                    // try another tile in this chunk first, else yield this chunk's
                    // arena to the neighbour that's already here.
                    if self
                        .arenas
                        .iter()
                        .any(|a| ((a.x - x).powi(2) + (a.y - y).powi(2)).sqrt() < ARENA_MIN_SEP)
                    {
                        continue;
                    }
                    let tier = difficulty_at(x, y);
                    let waves = (2 + tier / 40).min(5) as u8; // 2..5, scales with distance
                    self.arenas.push(Arena {
                        x,
                        y,
                        cx,
                        cy,
                        seed: hash2(self.seed ^ 0x5EED_A5EE, cx, cy),
                        state: ArenaState::Idle,
                        wave: 0,
                        waves,
                        countdown: 0.0,
                    });
                    break;
                }
            }
        }

        // Rare cursed relic (a dark chest): a high-risk speed sprint. Only one
        // curse can run, and looted chunks don't respawn it (no reload farm).
        if diff >= 3
            && self.relic.is_none()
            && r.chance(0.01)
            && !self.looted_relics.contains(&(cx, cy))
        {
            for _ in 0..8 {
                let tx = cx * CHUNK + r.below(CHUNK as u32) as i64;
                let ty = cy * CHUNK + r.below(CHUNK as u32) as i64;
                if world::passable(tile_at(self.seed, tx, ty)) {
                    let x = tx as f32 * TILE + TILE * 0.5;
                    let y = ty as f32 * TILE + TILE * 0.5;
                    let power = difficulty_at(x, y) as f32;
                    self.loot.push(Loot { x, y, kind: Drop::Relic { power } });
                    break;
                }
            }
        }

        // Occasional campfire: a rest site to trickle-heal — safely to half, but
        // pushing higher risks an ambush.
        if diff >= 1 && r.chance(0.03) {
            for _ in 0..8 {
                let tx = cx * CHUNK + r.below(CHUNK as u32) as i64;
                let ty = cy * CHUNK + r.below(CHUNK as u32) as i64;
                if world::passable(tile_at(self.seed, tx, ty)) {
                    let x = tx as f32 * TILE + TILE * 0.5;
                    let y = ty as f32 * TILE + TILE * 0.5;
                    self.campfires.push(Campfire { x, y, cx, cy });
                    break;
                }
            }
        }

        // Occasional offering shrine: sacrifice junk items for a reward.
        if diff >= 2 && r.chance(0.02) {
            for _ in 0..8 {
                let tx = cx * CHUNK + r.below(CHUNK as u32) as i64;
                let ty = cy * CHUNK + r.below(CHUNK as u32) as i64;
                if world::passable(tile_at(self.seed, tx, ty)) {
                    let x = tx as f32 * TILE + TILE * 0.5;
                    let y = ty as f32 * TILE + TILE * 0.5;
                    self.shrines.push(Shrine { x, y, cx, cy });
                    break;
                }
            }
        }

        // Rare cursed fog: inside, sight shrinks and monsters lurk unseen. A
        // premium cache (a boosted chest + health + ammo) waits at its heart —
        // deduped via looted_chests, so claiming the chest retires the fog.
        if diff >= 2 && r.chance(0.02) && !self.looted_chests.contains(&(cx, cy)) {
            for _ in 0..10 {
                let tx = cx * CHUNK + r.below(CHUNK as u32) as i64;
                let ty = cy * CHUNK + r.below(CHUNK as u32) as i64;
                if world::passable(tile_at(self.seed, tx, ty)) && open_enough(self.seed, tx, ty) {
                    let x = tx as f32 * TILE + TILE * 0.5;
                    let y = ty as f32 * TILE + TILE * 0.5;
                    let power = difficulty_at(x, y) as f32;
                    let tier = difficulty_at(x, y);
                    self.miasmas.push(Miasma { x, y, cx, cy, r: MIASMA_R });
                    let seed = hash2(self.seed ^ 0xF06E_1A5E_CAFE_00D5, cx, cy);
                    self.loot.push(Loot { x, y, kind: Drop::Chest { seed, power: power * 1.25 } });
                    self.loot.push(Loot { x: x + TILE, y, kind: Drop::Health((40.0 + power).min(140.0)) });
                    self.loot.push(Loot { x: x - TILE, y, kind: Drop::Ammo(40 + tier.min(90)) });
                    break;
                }
            }
        }

        // Occasional shield shrine: touch it for a one-time blue ward that soaks
        // damage until drained — a temporary health buffer, scaled by the region.
        if diff >= 1 && r.chance(0.02) && !self.looted_shields.contains(&(cx, cy)) {
            for _ in 0..8 {
                let tx = cx * CHUNK + r.below(CHUNK as u32) as i64;
                let ty = cy * CHUNK + r.below(CHUNK as u32) as i64;
                if world::passable(tile_at(self.seed, tx, ty)) {
                    let x = tx as f32 * TILE + TILE * 0.5;
                    let y = ty as f32 * TILE + TILE * 0.5;
                    let amount = (50.0 + difficulty_at(x, y) as f32 * 2.0).min(250.0);
                    self.loot.push(Loot { x, y, kind: Drop::Shield { amount } });
                    break;
                }
            }
        }

        // Occasional rune vault: a light memory puzzle (solved in an overlay)
        // opens a cache. The world keeps running while you solve, so monsters
        // can wander in. Deduped via looted_vaults once cracked.
        if diff >= 2 && r.chance(0.02) && !self.looted_vaults.contains(&(cx, cy)) {
            for _ in 0..8 {
                let tx = cx * CHUNK + r.below(CHUNK as u32) as i64;
                let ty = cy * CHUNK + r.below(CHUNK as u32) as i64;
                if world::passable(tile_at(self.seed, tx, ty)) {
                    let x = tx as f32 * TILE + TILE * 0.5;
                    let y = ty as f32 * TILE + TILE * 0.5;
                    self.vaults.push(Vault { x, y, cx, cy, opened: false });
                    break;
                }
            }
        }

        // Rare rift: step in to leap a big chunk of distance toward the goal —
        // but you arrive in a higher danger tier with something waiting. Used up
        // once you step through (dedup via looted_rifts).
        if diff >= 2 && r.chance(0.015) && !self.looted_rifts.contains(&(cx, cy)) {
            for _ in 0..8 {
                let tx = cx * CHUNK + r.below(CHUNK as u32) as i64;
                let ty = cy * CHUNK + r.below(CHUNK as u32) as i64;
                if world::passable(tile_at(self.seed, tx, ty)) {
                    let x = tx as f32 * TILE + TILE * 0.5;
                    let y = ty as f32 * TILE + TILE * 0.5;
                    self.rifts.push(Rift { x, y, cx, cy });
                    break;
                }
            }
        }
    }

    /// Kick off the 100,000 flash mob: bank a checkpoint here, clear threats,
    /// and gather a dancing crowd of monsters and bosses around the player.
    fn start_celebration(&mut self) {
        self.celebrating = true;
        self.celebrate_t = CELEBRATE_DUR;
        self.celebrated = true;
        self.player.cp_x = self.player.x;
        self.player.cp_y = self.player.y;
        self.player.hp = self.player.maxhp;
        self.projectiles.clear();
        self.loot.clear();
        self.monsters.clear();

        let (px, py) = (self.player.x, self.player.y);
        let lvl = difficulty_at(px, py);
        let pc = self.player_chunk();
        let mut r = Rng::new(hash2(self.seed ^ 0x9A17_CE1E, px as i64, py as i64));
        for i in 0..14 {
            let ang = i as f32 / 14.0 * std::f32::consts::TAU;
            let rad = 26.0 + r.range(0.0, 18.0);
            let x = px + ang.cos() * rad;
            let y = py + ang.sin() * rad;
            let ms = monster_seed(self.seed ^ 0xDA_11CE, pc.0, pc.1, 1000 + i);
            let mut m = gen_monster(ms, pc.0, pc.1, x, y, lvl);
            if i % 5 == 0 {
                make_mega(&mut m);
            }
            self.monsters.push(m);
        }
        self.set_message("100,000!! The wilds erupt in a joyous flash mob!".into());
        self.msg_t = CELEBRATE_DUR;
    }

    /// Mark every milestone at or below the current record as already announced —
    /// the base-10 toasts and the 25k/50k/75k celebration showers — so loading a
    /// save or warping doesn't re-fire old ones.
    fn seed_milestones(&mut self) {
        let mut mask = 0u32;
        let mut t = 1.0f32;
        for k in 0..MILESTONE_LABELS.len() {
            if self.player.max_dist >= t {
                mask |= 1 << k;
            }
            t *= 10.0;
        }
        // Off-the-base-10-grid milestones, tracked with their own flags.
        if self.player.max_dist >= 25_000.0 {
            mask |= MILESTONE_25K;
        }
        if self.player.max_dist >= 50_000.0 {
            mask |= MILESTONE_50K;
        }
        if self.player.max_dist >= 75_000.0 {
            mask |= MILESTONE_75K;
        }
        self.milestone_mask = mask;
    }

    /// A one-time milestone shower: fill the current view with celebratory loot
    /// on passable, in-window tiles (so nothing lands in water, and it despawns
    /// normally as the player wanders off). Deliberately doesn't shift the core
    /// economy — ammo halves on death, uniques fade with distance/durability, and
    /// fountains are left behind — it's pure spectacle and a "what's next?" hook.
    fn scatter_view_loot(&mut self, gift: Gift) {
        let (pcx, pcy) = self.player_chunk();
        let ptx = (self.player.x / TILE).floor() as i64;
        let pty = (self.player.y / TILE).floor() as i64;
        // Visible tile bounds mirror the snapshot camera (centred on the player).
        let half_w = LOGICAL_W * 0.5;
        let half_h = self.view_h * 0.5;
        let tx0 = ((self.player.x - half_w) / TILE).floor() as i64;
        let tx1 = ((self.player.x + half_w) / TILE).ceil() as i64;
        let ty0 = ((self.player.y - half_h) / TILE).floor() as i64;
        let ty1 = ((self.player.y + half_h) / TILE).ceil() as i64;

        let mut tiles: Vec<(i64, i64)> = Vec::new();
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                if (tx, ty) == (ptx, pty) {
                    continue; // don't bury the player's own tile
                }
                let (cx, cy) = (tx.div_euclid(CHUNK), ty.div_euclid(CHUNK));
                if (cx - pcx).abs() <= 1
                    && (cy - pcy).abs() <= 1
                    && world::passable(tile_at(self.seed, tx, ty))
                {
                    tiles.push((tx, ty));
                }
            }
        }
        if tiles.is_empty() {
            return;
        }
        // Deterministic shuffle so the scatter looks organic but replays the same.
        let mut r = Rng::new(hash2(self.seed ^ 0x0CE1_EB77, ptx, pty));
        for i in (1..tiles.len()).rev() {
            let j = r.below(i as u32 + 1) as usize;
            tiles.swap(i, j);
        }

        let center =
            |tx: i64, ty: i64| (tx as f32 * TILE + TILE * 0.5, ty as f32 * TILE + TILE * 0.5);
        let power = difficulty_at(self.player.x, self.player.y) as f32;

        match gift {
            Gift::Fountains => {
                let n = tiles.len().min(48);
                for &(tx, ty) in tiles.iter().take(n) {
                    let (x, y) = center(tx, ty);
                    self.loot.push(Loot { x, y, kind: Drop::Fountain });
                }
            }
            Gift::Ammo(total) => {
                let n = tiles.len().min(120).max(1);
                let per = (total / n as u32).max(1);
                let mut left = total;
                for (idx, &(tx, ty)) in tiles.iter().take(n).enumerate() {
                    let amt = if idx == n - 1 { left } else { per.min(left) };
                    left = left.saturating_sub(amt);
                    let (x, y) = center(tx, ty);
                    self.loot.push(Loot { x, y, kind: Drop::Ammo(amt.max(1)) });
                }
            }
            Gift::Chests => {
                let n = tiles.len().min(90);
                for &(tx, ty) in tiles.iter().take(n) {
                    let (x, y) = center(tx, ty);
                    let seed = r.next();
                    self.loot.push(Loot { x, y, kind: Drop::Chest { seed, power } });
                }
            }
            Gift::Shields => {
                // Non-additive wards — grabbing more doesn't stack; it's a marker.
                let amount = (50.0 + power * 2.0).min(250.0);
                let n = tiles.len().min(48);
                for &(tx, ty) in tiles.iter().take(n) {
                    let (x, y) = center(tx, ty);
                    self.loot.push(Loot { x, y, kind: Drop::Shield { amount } });
                }
            }
            Gift::Rifts => {
                // A field of teleporters — step into any one to leap ahead. Keep
                // them a couple tiles clear of the player so the field is seen
                // before one triggers (a rift fires within RIFT_RADIUS).
                let (px, py) = (self.player.x, self.player.y);
                let n = tiles.len().min(24);
                let mut placed = 0;
                for &(tx, ty) in tiles.iter() {
                    if placed >= n {
                        break;
                    }
                    let (x, y) = center(tx, ty);
                    if ((x - px).powi(2) + (y - py).powi(2)).sqrt() < TILE * 3.0 {
                        continue; // too close — would teleport you instantly
                    }
                    let (cx, cy) = (tx.div_euclid(CHUNK), ty.div_euclid(CHUNK));
                    self.rifts.push(Rift { x, y, cx, cy });
                    placed += 1;
                }
            }
        }
    }

    /// Fire the milestone celebrations as the record climbs. Small base-10 toasts
    /// mark the early powers of ten (1,000 showers fountains, 10,000 rains ammo),
    /// but the real progression is a ladder of SPECIFIC-value celebrations — a
    /// field of shields at 25,000, a trove of ancient chests at 50,000, a field of
    /// rifts at 75,000 — building to the 100,000 flash-mob finale, the designed
    /// end-game (playtesting reaches ~50k; 100k is the human-plausible frontier).
    /// 1,000,000 remains only a difficulty landmark, not a milestone anyone is
    /// expected to reach.
    fn check_milestones(&mut self) {
        let mut t = 1.0f32;
        for k in 0..MILESTONE_LABELS.len() {
            if self.player.max_dist >= t && self.milestone_mask & (1 << k) == 0 {
                self.milestone_mask |= 1 << k;
                self.milestone_t = 1.8;
                match k {
                    3 => {
                        self.scatter_view_loot(Gift::Fountains);
                        self.set_message(
                            "1,000! Healing springs burst from the earth all around you!".into(),
                        );
                        self.msg_t = 6.0;
                    }
                    4 => {
                        self.scatter_view_loot(Gift::Ammo(10_000));
                        self.set_message(
                            "10,000!! A storm of ammunition rains across the land — scoop it up!"
                                .into(),
                        );
                        self.msg_t = 6.0;
                    }
                    _ => {
                        self.set_message(format!(
                            "{}  —  {} from origin",
                            MILESTONE_LABELS[k], t as u32
                        ));
                        self.msg_t = 4.5;
                    }
                }
            }
            t *= 10.0;
        }
        // Off-the-base-10-grid milestones on the way to the 100,000 flash mob:
        // a field of shields (25k), a chest trove (50k), a field of rifts (75k).
        if self.player.max_dist >= 25_000.0 && self.milestone_mask & MILESTONE_25K == 0 {
            self.milestone_mask |= MILESTONE_25K;
            self.milestone_t = 1.8;
            self.scatter_view_loot(Gift::Shields);
            self.set_message(
                "25,000!! A field of shield wards shimmers into being — press on!".into(),
            );
            self.msg_t = 6.0;
        }
        if self.player.max_dist >= 50_000.0 && self.milestone_mask & MILESTONE_50K == 0 {
            self.milestone_mask |= MILESTONE_50K;
            self.milestone_t = 1.8;
            self.scatter_view_loot(Gift::Chests);
            self.set_message(
                "50,000!!! A trove of ancient chests erupts — untold riches! (you can carry 60)"
                    .into(),
            );
            self.msg_t = 6.0;
        }
        if self.player.max_dist >= 75_000.0 && self.milestone_mask & MILESTONE_75K == 0 {
            self.milestone_mask |= MILESTONE_75K;
            self.milestone_t = 1.8;
            self.scatter_view_loot(Gift::Rifts);
            self.set_message(
                "75,000!!! A field of rifts tears open — step through to leap ahead!".into(),
            );
            self.msg_t = 6.0;
        }
    }

    // --- arena (survive-the-waves point of interest) ---------------------

    /// Drive at most one active arena: activate on entry (consuming it), advance
    /// waves as they're cleared, pay the cache on a full clear, and forfeit if the
    /// player leaves the ring.
    fn update_arenas(&mut self, dt: f32) {
        let (px, py) = (self.player.x, self.player.y);

        // An active arena takes priority.
        if let Some(i) = self.arenas.iter().position(|a| a.state == ArenaState::Active) {
            let (ax, ay) = (self.arenas[i].x, self.arenas[i].y);
            // Seal the ring: drop any ambient monster that wandered/streamed in so
            // only the wave remains, and keep the wave *inside* the ring — a
            // fleeing or kiting ranged foe must never force the player to leave
            // (which would forfeit), so a melee-only run stays winnable.
            self.monsters.retain(|m| m.from_arena);
            let confine = ARENA_INNER - 6.0; // mobs stay in the inner ring
            for m in self.monsters.iter_mut() {
                let (mdx, mdy) = (m.x - ax, m.y - ay);
                let md = (mdx * mdx + mdy * mdy).sqrt();
                if md > confine {
                    m.x = ax + mdx / md.max(0.001) * confine;
                    m.y = ay + mdy / md.max(0.001) * confine;
                }
            }
            let pd = ((px - ax).powi(2) + (py - ay).powi(2)).sqrt();
            if pd > ARENA_OUTER + 20.0 {
                self.abort_arena_at(i); // left the whole arena — forfeit
                return;
            }
            // The apron (between the rings) is extra room, but it rots your health
            // the longer you linger — room comes at a cost.
            if pd > ARENA_INNER + 4.0 {
                let tier = difficulty_at(ax, ay);
                self.damage_player((2.5 + tier as f32 * 0.2) * dt, Hurt::Swamp);
            }
            // Ready-steady-go: count down, then spawn the wave ("go!").
            if self.arenas[i].countdown > 0.0 {
                self.arenas[i].countdown -= dt;
                if self.arenas[i].countdown <= 0.0 {
                    self.arenas[i].countdown = 0.0;
                    let next = self.arenas[i].wave + 1;
                    let (seed, waves) = (self.arenas[i].seed, self.arenas[i].waves);
                    self.arenas[i].wave = next;
                    let tier = difficulty_at(ax, ay);
                    let boss = next == waves && tier >= ARENA_BOSS_TIER;
                    self.spawn_arena_wave(ax, ay, seed, next, tier, boss);
                    if boss {
                        self.set_message(format!(
                            "Final wave — a Colossus rises! ({} of {})",
                            next, waves
                        ));
                    } else {
                        self.set_message(format!("Wave {} of {} — go!", next, waves));
                    }
                    self.msg_t = 2.5;
                }
                return;
            }
            let alive = self.monsters.iter().filter(|m| m.from_arena).count();
            if alive == 0 {
                let (wave, waves) = (self.arenas[i].wave, self.arenas[i].waves);
                if wave >= waves {
                    let tier = difficulty_at(ax, ay);
                    self.arenas[i].state = ArenaState::Cleared; // conquered
                    self.spawn_arena_cache(ax, ay, tier, tier >= ARENA_BOSS_TIER);
                    self.set_message("Arena cleared — claim your spoils!".into());
                    self.msg_t = 5.0;
                } else {
                    // Wave defeated — brief breather, then count down the next wave.
                    self.arenas[i].countdown = ARENA_COUNTDOWN;
                    self.set_message("Wave cleared! Get ready...".into());
                    self.msg_t = 2.0;
                }
            }
            return;
        }

        // Otherwise: has the player stepped into a fresh ring?
        let enter = self.arenas.iter().position(|a| {
            a.state == ArenaState::Idle
                && !self.looted_arenas.contains(&(a.cx, a.cy))
                && ((px - a.x).powi(2) + (py - a.y).powi(2)).sqrt() <= ARENA_INNER
        });
        if let Some(i) = enter {
            // Consume on ENTRY: pausing, forfeiting, dying or reloading all forfeit
            // identically — only a full clear ever pays out.
            let (cx, cy) = (self.arenas[i].cx, self.arenas[i].cy);
            remember_chunk(&mut self.looted_arenas, (cx, cy));
            self.arenas[i].state = ArenaState::Active;
            self.arenas[i].wave = 0;
            self.arenas[i].countdown = ARENA_COUNTDOWN; // ready-steady-go before wave 1
            // Clear the stage: existing mobs and any incoming shots vanish so the
            // fight is purely the arena's waves.
            self.monsters.clear();
            self.projectiles.clear();
            self.set_message(
                "Arena! Survive the waves — no pausing. Leave the ring or press Esc to forfeit."
                    .into(),
            );
            self.msg_t = 5.0;
        }
    }

    /// Forfeit the arena at `i`: clear its monsters, retire the ring, no reward.
    fn abort_arena_at(&mut self, i: usize) {
        self.arenas[i].state = ArenaState::Done;
        self.monsters.retain(|m| !m.from_arena);
        self.set_message("You abandon the arena — the cache is lost.".into());
        self.msg_t = 3.0;
    }

    /// Forfeit whichever arena is active (called by the menu/Esc path).
    fn abort_active_arena(&mut self) {
        if let Some(i) = self.arenas.iter().position(|a| a.state == ArenaState::Active) {
            self.abort_arena_at(i);
        }
    }

    /// Spawn one wave of hostile monsters near the ring edge (never on top of the
    /// player), tagged so we know when the wave is cleared. A `boss` wave is a
    /// Colossus plus only a few minions — a focused finale, not a swarm.
    fn spawn_arena_wave(&mut self, ax: f32, ay: f32, seed: u64, wave: u8, tier: u32, boss: bool) {
        let mut r = Rng::new(seed ^ (wave as u64).wrapping_mul(0x9E37_79B1_1CE1));
        // Normal waves ramp gently and cap so you're never buried; the boss wave
        // is the Colossus plus just 1..3 minions so the fight stays about the boss.
        let n = if boss {
            1 + (1 + tier / 12).min(3)
        } else {
            (2 + wave as u32 + tier / 8).min(6)
        };
        let (acx, acy) = ((ax / CHUNK_PX).floor() as i64, (ay / CHUNK_PX).floor() as i64);
        for i in 0..n {
            for _ in 0..10 {
                // Random direction (rejection-normalized), out at the ring edge so
                // there's travel time before they reach the player at the center.
                let (mut dx, mut dy) = (r.range(-1.0, 1.0), r.range(-1.0, 1.0));
                let mag = (dx * dx + dy * dy).sqrt();
                if mag < 0.01 {
                    dx = 1.0;
                    dy = 0.0;
                } else {
                    dx /= mag;
                    dy /= mag;
                }
                let d = r.range(ARENA_INNER * 0.78, ARENA_INNER * 0.98);
                let x = ax + dx * d;
                let y = ay + dy * d;
                if passable_px(self.seed, x, y) {
                    let ms = hash2(seed ^ 0xA5EE_0001, wave as i64, i as i64);
                    let mut mon = gen_monster(ms, acx, acy, x, y, difficulty_at(x, y).max(1));
                    mon.from_arena = true;
                    mon.anger = 999.0; // arena foes come at you immediately
                    if boss && i == 0 {
                        make_mega(&mut mon); // the reliable Colossus finale
                    }
                    self.monsters.push(mon);
                    break;
                }
            }
        }
    }

    /// Pay the arena's reward: a few unique chests plus an ammo pile (with an
    /// extra chest for a boss finale), on passable ground so nothing is stranded.
    fn spawn_arena_cache(&mut self, ax: f32, ay: f32, tier: u32, boss_bounty: bool) {
        let power = tier as f32;
        let base = hash2(self.seed ^ 0x1234_A5EE, (ax / TILE) as i64, (ay / TILE) as i64);
        let n_chests = 2 + (tier / 60).min(2) + boss_bounty as u32; // 2..5 uniques
        for k in 0..n_chests {
            let (x, y) = self.drop_pos(ax + (k as f32 - 1.0) * 12.0, ay - 6.0);
            let seed = base ^ (k as u64).wrapping_mul(0x9E37_79B1);
            self.loot.push(Loot { x, y, kind: Drop::Chest { seed, power } });
        }
        let (x, y) = self.drop_pos(ax, ay + 12.0);
        self.loot.push(Loot { x, y, kind: Drop::Ammo(30 + tier.min(120)) });
    }

    /// Keep a champion fight a fair 1v1: while the player is near a live
    /// champion, clear ordinary wandering monsters from the duel radius (relic
    /// hunters and arena waves are left alone). Toasts once when a duel begins.
    fn enforce_duels(&mut self) {
        let (px, py) = (self.player.x, self.player.y);
        let near = self.monsters.iter().any(|m| {
            m.champion && m.hp > 0.0 && ((m.x - px).powi(2) + (m.y - py).powi(2)).sqrt() <= DUEL_RADIUS
        });
        if near {
            self.monsters.retain(|m| {
                m.champion
                    || m.hunter
                    || m.from_arena
                    || ((m.x - px).powi(2) + (m.y - py).powi(2)).sqrt() > DUEL_RADIUS
            });
            if !self.dueling {
                self.dueling = true;
                self.set_message("A champion bars your path — face it alone!".into());
                self.msg_t = 4.0;
            }
        } else {
            self.dueling = false;
        }
    }

    /// Crack the rune vault the player is standing at (the puzzle is solved in
    /// the client overlay): mark it open and spill a cache — a boosted chest, a
    /// health pickup, and an ammo pile — scaled to the region.
    fn open_vault_near(&mut self) {
        let (px, py) = (self.player.x, self.player.y);
        let i = self.vaults.iter().position(|v| {
            !v.opened && ((px - v.x).powi(2) + (py - v.y).powi(2)).sqrt() <= VAULT_RADIUS
        });
        if let Some(i) = i {
            self.vaults[i].opened = true;
            let (vx, vy, vcx, vcy) = (self.vaults[i].x, self.vaults[i].y, self.vaults[i].cx, self.vaults[i].cy);
            remember_chunk(&mut self.looted_vaults, (vcx, vcy));
            // Retire any *other* vaults overlapping this spot (adjacent chunks can
            // drop two within VAULT_RADIUS). Otherwise `at_vault` stays true after
            // the crack and the frame loop instantly re-opens a fresh puzzle — an
            // inescapable "loop" that leaves a still-closed vault beside the loot.
            let cluster: Vec<(i64, i64)> = self
                .vaults
                .iter()
                .filter(|v| !v.opened && ((px - v.x).powi(2) + (py - v.y).powi(2)).sqrt() <= VAULT_RADIUS)
                .map(|v| (v.cx, v.cy))
                .collect();
            for v in self.vaults.iter_mut() {
                if !v.opened && ((px - v.x).powi(2) + (py - v.y).powi(2)).sqrt() <= VAULT_RADIUS {
                    v.opened = true;
                }
            }
            for c in cluster {
                remember_chunk(&mut self.looted_vaults, c);
            }
            let power = difficulty_at(vx, vy) as f32;
            let tier = difficulty_at(vx, vy);
            let base = hash2(self.seed ^ 0x2A17_9E5B_C0DE_4411, vx as i64, vy as i64);
            let (cxp, cyp) = self.drop_pos(vx, vy);
            let (hxp, hyp) = self.drop_pos(vx - TILE, vy);
            let (axp, ayp) = self.drop_pos(vx + TILE, vy);
            self.loot.push(Loot { x: cxp, y: cyp, kind: Drop::Chest { seed: base, power: power * 1.2 } });
            self.loot.push(Loot { x: hxp, y: hyp, kind: Drop::Health((40.0 + power).min(140.0)) });
            self.loot.push(Loot { x: axp, y: ayp, kind: Drop::Ammo(40 + tier.min(90)) });
            self.set_message("The rune vault grinds open — claim the cache!".into());
            self.msg_t = 5.0;
        }
    }

    /// If the player steps into a rift, leap them a big chunk of distance
    /// radially outward (toward the goal) and greet them, unready, with either a
    /// mini-boss or an ambush. No checkpoint is banked — the risk is arriving
    /// deep with your last checkpoint far behind.
    fn check_rifts(&mut self) {
        let (px, py) = (self.player.x, self.player.y);
        let i = self
            .rifts
            .iter()
            .position(|rf| ((px - rf.x).powi(2) + (py - rf.y).powi(2)).sqrt() <= RIFT_RADIUS);
        if let Some(i) = i {
            let (rcx, rcy) = (self.rifts[i].cx, self.rifts[i].cy);
            remember_chunk(&mut self.looted_rifts, (rcx, rcy));
            // A cursed-relic hunt can't be outrun: some pursuers tear through the
            // rift after you. The rift still *thins* the pack (the rest is left
            // behind and the curse refills over time), so it's a real risk/reward
            // — distance for a fresh fight — not a clean escape.
            let carried = if self.relic.is_some() {
                self.monsters.iter().filter(|m| m.hunter).count().min(RIFT_HUNTER_CARRY)
            } else {
                0
            };
            let d = (px * px + py * py).sqrt();
            let (dx, dy) = if d > 1.0 { (px / d, py / d) } else { (1.0, 0.0) };
            let jump = RIFT_JUMP_TILES * TILE;
            let (sx, sy) = safe_spawn(self.seed, px + dx * jump, py + dy * jump);
            self.player.x = sx;
            self.player.y = sy;
            self.projectiles.clear();
            self.monsters.clear(); // the old area's mobs are left far behind
            self.stream_chunks(); // load the new, more dangerous ground
            self.seed_milestones(); // don't spam toasts for everything leapt past
            // Clear a small bubble around the landing so a natural spawn (a mega,
            // even) can't materialise point-blank the instant you arrive — the
            // *intended* encounter below spawns outside this radius (48px+), so it
            // survives. No checkpoint is banked here, so a fair beat matters.
            let (lx, ly) = (self.player.x, self.player.y);
            self.monsters
                .retain(|m| ((m.x - lx).powi(2) + (m.y - ly).powi(2)).sqrt() > RIFT_LANDING_SAFE);
            // Re-summon the hunters that followed you through (off-screen, closing
            // in) so the curse resumes without a free breather.
            for _ in 0..carried {
                self.spawn_hunter();
            }
            let ahead = RIFT_JUMP_TILES as u32;
            if self.roll01() < 0.4 {
                self.spawn_rift_champion();
                self.set_message(format!("Through the rift — {ahead} tiles ahead. A champion bars the way!"));
            } else {
                self.spawn_ambush();
                self.set_message(format!("Through the rift — {ahead} tiles ahead, into deeper danger!"));
            }
            self.msg_t = 5.0;
        }
    }

    /// Drop a lone champion on open ground near the rift landing (falls back to a
    /// swarm ambush if there's no room).
    fn spawn_rift_champion(&mut self) {
        let (px, py) = (self.player.x, self.player.y);
        let tier = difficulty_at(px, py).max(2);
        let mut rr = Rng::new(hash2(self.seed ^ 0x21F7_C0DE_5A11_9E2B, px as i64, py as i64));
        for _ in 0..12 {
            let ang = rr.range(0.0, 6.2831853);
            let dist = rr.range(60.0, 100.0);
            let x = px + ang.cos() * dist;
            let y = py + ang.sin() * dist;
            if passable_px(self.seed, x, y) {
                let (cx, cy) = chunk_of(x, y);
                let mut mon = gen_monster(rr.next(), cx, cy, x, y, tier);
                make_champion(&mut mon);
                self.monsters.push(mon);
                return;
            }
        }
        self.spawn_ambush();
    }

    // --- main tick -------------------------------------------------------

    fn update(&mut self, dt: f32) {
        self.advance(dt);
        self.build_snapshot();
    }

    /// Advance the pure simulation one tick (everything except building the
    /// render snapshot). The headless simulator calls this directly so it isn't
    /// paying to regenerate terrain noise it never reads.
    fn advance(&mut self, dt: f32) {
        self.play_secs += dt;

        // During the 100,000 celebration everyone dances: no movement, no
        // combat. When the music stops, normal rules resume and the wilds swarm.
        if self.celebrating {
            self.celebrate_t -= dt;
            self.stream_chunks();
            if self.celebrate_t <= 0.0 {
                self.celebrating = false;
                self.set_message("The music fades... the wilds remember you.".into());
                self.msg_t = 4.0;
            }
            return;
        }

        self.move_player(dt);
        self.stream_chunks();
        self.check_rifts();
        self.enforce_duels();
        self.player_attack(dt);
        self.update_projectiles(dt);
        self.update_monsters(dt);
        self.update_arenas(dt);
        self.update_relic(dt);
        self.update_campfire(dt);
        self.environment(dt);
        self.pickups();

        // Record the farthest distance ever reached (before any respawn resets
        // position) — this is the persistent run-to-run achievement.
        let d = dist_tiles(self.player.x, self.player.y);
        if d > self.player.max_dist {
            self.player.max_dist = d;
        }

        // Escalating mini-toasts + the 50,000 chest trove (run first so the
        // 100,000 flash-mob message below wins when both fire on the same frame).
        self.check_milestones();
        // Reaching 100,000 triggers the one-time flash mob.
        if !self.celebrated && self.player.max_dist >= CELEBRATE_DIST {
            self.start_celebration();
        }

        // Bank a checkpoint each time we cross into a new CHECKPOINT tier further
        // out than the one already banked, so death doesn't send us all the way
        // back to the origin.
        let cp_d = dist_tiles(self.player.cp_x, self.player.cp_y);
        if (d / CHECKPOINT).floor() > (cp_d / CHECKPOINT).floor() {
            self.player.cp_x = self.player.x;
            self.player.cp_y = self.player.y;
            let tier = (d / CHECKPOINT).floor() * CHECKPOINT;
            self.set_message(format!("Checkpoint banked at distance {}", tier as u32));
        }

        self.respawn_if_dead();

        if self.player.atk_cd > 0.0 {
            self.player.atk_cd -= dt;
        }
        if self.msg_t > 0.0 {
            self.msg_t -= dt;
        }
        if self.milestone_t > 0.0 {
            self.milestone_t -= dt;
        }
        self.player.refresh_maxhp();
    }

    fn move_player(&mut self, dt: f32) {
        let k = self.input.keys;
        let mut dx = 0.0f32;
        let mut dy = 0.0f32;
        if k & 1 != 0 {
            dy -= 1.0;
        }
        if k & 2 != 0 {
            dy += 1.0;
        }
        if k & 4 != 0 {
            dx -= 1.0;
        }
        if k & 8 != 0 {
            dx += 1.0;
        }
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        let len = (dx * dx + dy * dy).sqrt();
        dx /= len;
        dy /= len;

        let tile = tile_px(self.seed, self.player.x, self.player.y);
        // Cap the Move-skill bonus so top speed stays humanly controllable: at
        // MOVE_BONUS_CAP the player crosses the 320px viewport in ~2s, leaving
        // reaction time for hazards/walls appearing at the screen edge. Move
        // levels keep accruing but grant no more speed past the cap.
        let mv = skill_bonus(self.player.skills[SK_MOVE as usize]).min(MOVE_BONUS_CAP);
        let mut speed = BASE_SPEED * (1.0 + mv) / move_cost(tile);
        if self.relic.is_some() {
            speed *= RELIC_SPEED_MULT; // the cursed speed burst
        }
        let step = speed * dt;

        // Axis-separated movement so the player slides along walls.
        let (ox, oy) = (self.player.x, self.player.y);
        let nx = self.player.x + dx * step;
        if passable_px(self.seed, nx, self.player.y) {
            self.player.x = nx;
        }
        let ny = self.player.y + dy * step;
        if passable_px(self.seed, self.player.x, ny) {
            self.player.y = ny;
        }
        // Count actual distance walked (in tiles) — the achievement's "steps".
        let moved = ((self.player.x - ox).powi(2) + (self.player.y - oy).powi(2)).sqrt() / TILE;
        self.steps += moved as f64;
        // The curse ticks down on *any* movement, not just outward progress.
        if let Some(r) = self.relic.as_mut() {
            r.steps += moved;
        }
        self.player.train(SK_MOVE, dt * 0.6);
    }

    fn player_attack(&mut self, _dt: f32) {
        if !self.input.attack || self.player.atk_cd > 0.0 {
            return;
        }
        // While cursed, the relic is the only weapon you can wield.
        let w = if let Some(r) = &self.relic {
            r.weapon.clone()
        } else {
            match self.player.weapon() {
                Some(w) => w.clone(),
                None => return,
            }
        };
        let (mut ax, mut ay) = (self.input.aimx, self.input.aimy);
        let al = (ax * ax + ay * ay).sqrt();
        if al < 0.001 {
            ax = 1.0;
            ay = 0.0;
        } else {
            ax /= al;
            ay /= al;
        }

        // Ranged weapons consume ammo; with an empty pool they can't fire, which
        // is what gives melee weapons a purpose and makes ranged use a resource.
        if w.ranged && self.player.ammo == 0 {
            self.set_message("Out of ammo — switch to a melee weapon (1-4).".into());
            return;
        }

        let class = w.class_skill;
        let cdr = skill_bonus(self.player.skills[class as usize]).min(0.5);
        self.player.atk_cd = w.cooldown * (1.0 - cdr * 0.5);
        self.player.train(class, 0.5);

        if w.ranged {
            self.player.ammo -= 1;
            // Floor the shot's speed above the player's own top speed so a fast
            // (high-Move) player can't outrun their own arrows — the same reason
            // monster_proj_speed exists for monsters. `range = ps * life` is
            // preserved below, so this changes the shot's PACE, not its reach.
            let mv = skill_bonus(self.player.skills[SK_MOVE as usize]).min(MOVE_BONUS_CAP);
            let ps = w.proj_speed.max(BASE_SPEED * (1.0 + mv) * 1.4);
            self.projectiles.push(Proj {
                x: self.player.x,
                y: self.player.y,
                vx: ax * ps,
                vy: ay * ps,
                life: w.range / ps.max(1.0),
                dmg: w.damage,
                dmg_type: w.dmg_type,
                special: w.special,
                class_skill: class,
                from_player: true,
                src_name: String::new(),
            });
        } else {
            // Melee cleave: hit everything in a forward arc within reach.
            let px = self.player.x;
            let py = self.player.y;
            let reach = w.range;
            let mut hits: Vec<usize> = Vec::new();
            for (i, m) in self.monsters.iter().enumerate() {
                let mdx = m.x - px;
                let mdy = m.y - py;
                let d = (mdx * mdx + mdy * mdy).sqrt();
                if d <= reach + m.radius {
                    let dot = if d > 0.001 {
                        (mdx / d) * ax + (mdy / d) * ay
                    } else {
                        1.0
                    };
                    if dot > 0.35 {
                        hits.push(i);
                    }
                }
            }
            for i in hits {
                self.hit_monster(i, &w);
            }
            self.reap_dead();
        }
    }

    /// Keep a ground drop on reachable ground. If `(x, y)` lands on an impassable
    /// tile — e.g. a boss slain at the water's edge flinging loot into deep water
    /// or against a mountain — pull it to the nearest passable tile so the reward
    /// can never be stranded where the player can't walk.
    fn drop_pos(&self, x: f32, y: f32) -> (f32, f32) {
        if passable_px(self.seed, x, y) {
            return (x, y);
        }
        let tx0 = (x / TILE).floor() as i64;
        let ty0 = (y / TILE).floor() as i64;
        for ring in 1..8i64 {
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    if dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let (tx, ty) = (tx0 + dx, ty0 + dy);
                    if world::passable(tile_at(self.seed, tx, ty)) {
                        return (tx as f32 * TILE + TILE * 0.5, ty as f32 * TILE + TILE * 0.5);
                    }
                }
            }
        }
        (x, y)
    }

    /// Apply one weapon hit to monster `i`; handle death, xp and loot.
    fn hit_monster(&mut self, i: usize, w: &Weapon) {
        let (mult, mut def, was_alive);
        {
            let m = &self.monsters[i];
            mult = elem_mult2(w.dmg_type, m.resist, m.weak, m.mega);
            def = m.def;
            was_alive = m.hp > 0.0;
        }
        if !was_alive {
            return;
        }
        let class = w.class_skill;
        let elem = elem_skill(w.dmg_type);
        let mut bonus = skill_bonus(self.player.skills[class as usize]);
        if let Some(e) = elem {
            bonus += skill_bonus(self.player.skills[e as usize]);
        }
        if w.special & SP_ARMOR_PEN != 0 {
            def *= 0.4;
        }
        let mut raw = w.damage * (1.0 + bonus) * mult;
        if w.special & SP_POISON_DOT != 0 {
            raw *= 1.25;
        }
        let dmg = (raw - def).max(1.0);

        // Train from use (the more you hit, the better you get).
        self.player.train(class, 0.05 + dmg * 0.02);
        if let Some(e) = elem {
            self.player.train(e, 0.05 + dmg * 0.02);
        }

        let m = &mut self.monsters[i];
        m.hp -= dmg;
        m.anger = 5.0; // provoked: it will chase and fight back for a while
        if m.hp <= 0.0 {
            self.kills += 1;
            let (mx, my, mxp, level, mega) = (m.x, m.y, m.xp, m.level, m.mega);
            let (champion, ccx, ccy) = (m.champion, m.cx, m.cy);
            if mega {
                self.mega_kills += 1;
            }
            self.player.train(class, mxp * 0.08);
            if let Some(e) = elem {
                self.player.train(e, mxp * 0.05);
            }
            // Slaying a mega is a big deal — a genuine haul worth the risk: TWO
            // unique chests, a full-heal fountain, and a big pile of ammo.
            if mega {
                let power = difficulty_at(mx, my) as f32;
                let base = hash2(self.seed ^ 0x9E2A_7B31, mx as i64, my as i64);
                let (c1x, c1y) = self.drop_pos(mx - 10.0, my);
                let (c2x, c2y) = self.drop_pos(mx + 10.0, my);
                let (fx, fy) = self.drop_pos(mx, my - 10.0);
                let (ax, ay) = self.drop_pos(mx, my + 10.0);
                self.loot.push(Loot { x: c1x, y: c1y, kind: Drop::Chest { seed: base, power } });
                self.loot.push(Loot { x: c2x, y: c2y, kind: Drop::Chest { seed: base ^ 0x5D, power } });
                self.loot.push(Loot { x: fx, y: fy, kind: Drop::Fountain });
                self.loot.push(Loot { x: ax, y: ay, kind: Drop::Ammo(40 + level.min(90)) });
                self.set_message(format!("You felled a Colossus! Two ancient chests spill forth. (Lv {level})"));
                self.msg_t = 6.0;
            }
            // A felled champion yields the prize it guarded: a boosted chest plus
            // a pile of ammo. Marked spent so it can't be reload-farmed.
            if champion {
                let power = difficulty_at(mx, my) as f32;
                let base = hash2(self.seed ^ 0xC4A3_9F17_D0E1_2233, mx as i64, my as i64);
                let (cxp, cyp) = self.drop_pos(mx, my);
                let (axp, ayp) = self.drop_pos(mx + 10.0, my);
                self.loot.push(Loot { x: cxp, y: cyp, kind: Drop::Chest { seed: base, power: power * 1.3 } });
                self.loot.push(Loot { x: axp, y: ayp, kind: Drop::Ammo(40 + level.min(90)) });
                remember_chunk(&mut self.looted_champions, (ccx, ccy));
                self.set_message(format!("The champion falls — claim its prize! (Lv {level})"));
                self.msg_t = 6.0;
            }
            // Drops — all deterministic from the death location. A kill can
            // yield several: ammo is common (so clearing monsters restocks
            // ranged play), weapons are uncommon, healing is rare (health stays
            // scarce). Nudge drops off the exact corpse so multiple are grabbable.
            let lseed = hash2(self.seed ^ 0x100D_7EA1, mx as i64, my as i64);
            let mut lr = Rng::new(lseed);

            if lr.chance(0.5 + (level as f32) * 0.01) {
                let power = difficulty_at(mx, my) as f32;
                let wseed = lr.next();
                let rarity = gen_weapon(wseed, power).rarity;
                let (wx, wy) = self.drop_pos(mx, my);
                self.loot.push(Loot { x: wx, y: wy, kind: Drop::Weapon { seed: wseed, power, rarity } });
            }
            if lr.chance(0.62) {
                let amount = 3 + lr.below(4); // 3..6
                let (ax, ay) = self.drop_pos(mx + lr.range(-6.0, 6.0), my + lr.range(-6.0, 6.0));
                self.loot.push(Loot { x: ax, y: ay, kind: Drop::Ammo(amount) });
            }
            if lr.chance(0.12) {
                let heal = 10.0 + level as f32 * 2.0;
                let (hx, hy) = self.drop_pos(mx + lr.range(-6.0, 6.0), my + lr.range(-6.0, 6.0));
                self.loot.push(Loot { x: hx, y: hy, kind: Drop::Health(heal) });
            }
            // Leave the corpse at hp<=0; it is reaped by `reap_dead` after the
            // current combat pass so we never shift indices mid-iteration.
        }
    }

    /// Remove monsters killed during the current pass.
    fn reap_dead(&mut self) {
        self.monsters.retain(|m| m.hp > 0.0);
    }

    fn update_projectiles(&mut self, dt: f32) {
        let mut i = 0;
        while i < self.projectiles.len() {
            let mut remove = false;
            self.projectiles[i].x += self.projectiles[i].vx * dt;
            self.projectiles[i].y += self.projectiles[i].vy * dt;
            self.projectiles[i].life -= dt;
            if self.projectiles[i].life <= 0.0 {
                remove = true;
            } else if self.projectiles[i].from_player {
                let (px, py) = (self.projectiles[i].x, self.projectiles[i].y);
                let mut target = None;
                for (mi, m) in self.monsters.iter().enumerate() {
                    let dx = m.x - px;
                    let dy = m.y - py;
                    if dx * dx + dy * dy <= (m.radius + 3.0) * (m.radius + 3.0) {
                        target = Some(mi);
                        break;
                    }
                }
                if let Some(mi) = target {
                    let w = Weapon {
                        seed: 0,
                        power: 1.0,
                        durability: 1.0,
                        unique: false,
                        base: 0,
                        dmg_type: self.projectiles[i].dmg_type,
                        damage: self.projectiles[i].dmg,
                        cooldown: 0.0,
                        range: 0.0,
                        ranged: true,
                        proj_speed: 0.0,
                        rarity: 0,
                        class_skill: self.projectiles[i].class_skill,
                        special: self.projectiles[i].special,
                        name: String::new(),
                    };
                    self.hit_monster(mi, &w);
                    remove = true;
                }
            } else {
                // Monster projectile vs player.
                let dx = self.player.x - self.projectiles[i].x;
                let dy = self.player.y - self.projectiles[i].y;
                if dx * dx + dy * dy <= (PLAYER_R + 3.0) * (PLAYER_R + 3.0) {
                    let d = self.projectiles[i].dmg;
                    let elem = self.projectiles[i].dmg_type;
                    let name = self.projectiles[i].src_name.clone();
                    self.damage_player(d, Hurt::Attack { name: &name, elem, ranged: true });
                    remove = true;
                }
            }
            if remove {
                self.projectiles.swap_remove(i);
            } else {
                i += 1;
            }
        }
        self.reap_dead();
    }

    fn update_monsters(&mut self, dt: f32) {
        let (px, py) = (self.player.x, self.player.y);
        let seed = self.seed;
        let mon_speed = if self.relic.is_some() { RELIC_MON_SPEED_MULT } else { 1.0 };
        // Hunters run at a fixed move-cost vs the cursed player's open speed, so
        // you gain on grass/dirt (cost 1.0) but lose ground on anything rougher.
        let mv = skill_bonus(self.player.skills[SK_MOVE as usize]).min(MOVE_BONUS_CAP);
        let hunter_speed = BASE_SPEED * (1.0 + mv) * RELIC_SPEED_MULT / RELIC_HUNTER_MOVE_COST;
        let mut new_projs: Vec<Proj> = Vec::new();
        let mut player_dmg = 0.0f32;
        // Track the hardest-hitting melee attacker this frame so a fatal blow can
        // name its source.
        let mut melee_src: Option<(String, u8)> = None;
        let mut melee_best = 0.0f32;

        for m in self.monsters.iter_mut() {
            if m.regen > 0.0 && m.hp < m.maxhp {
                m.hp = (m.hp + m.regen * dt).min(m.maxhp);
            }
            m.cd -= dt;
            if m.anger > 0.0 {
                m.anger -= dt;
            }
            let dx = px - m.x;
            let dy = py - m.y;
            let dist = (dx * dx + dy * dy).sqrt();

            // A monster that has been hit turns hostile no matter its nature.
            // Relic hunters are always hostile, at any range.
            let provoked = m.anger > 0.0;
            let notices = dist < AGGRO;
            let hostile = m.hunter || ((m.temper == monster::FIGHT || provoked) && notices);
            let fleeing = m.temper == monster::FLEE && !provoked && notices;

            // Decide a movement target + speed scale.
            let (tx, ty, scale) = if hostile {
                if m.ranged {
                    // Kite: hold a firing distance instead of charging into
                    // melee (where it couldn't shoot and would just hide on the
                    // player). Approach if far, back off if too close.
                    let want = 95.0;
                    if dist > want + 22.0 {
                        (px, py, 1.0)
                    } else if dist < want - 22.0 {
                        (m.x - dx, m.y - dy, 0.9)
                    } else {
                        (m.x, m.y, 0.0) // in the pocket — stand and fire
                    }
                } else {
                    (px, py, 1.0) // melee: charge
                }
            } else if fleeing {
                (m.x - dx, m.y - dy, 1.0) // run directly away
            } else {
                // Wander: pick roam targets within the home chunk and amble.
                m.wt -= dt;
                if m.wt <= 0.0 {
                    let hx = (m.cx as f32 + 0.5) * CHUNK_PX;
                    let hy = (m.cy as f32 + 0.5) * CHUNK_PX;
                    m.wx = hx + (m.roll() - 0.5) * CHUNK_PX * 0.8;
                    m.wy = hy + (m.roll() - 0.5) * CHUNK_PX * 0.8;
                    m.wt = 1.5 + m.roll() * 3.0;
                }
                (m.wx, m.wy, 0.5)
            };

            let (mdx, mdy) = (tx - m.x, ty - m.y);
            let md = (mdx * mdx + mdy * mdy).sqrt();
            if md > 0.5 {
                let (ux, uy) = (mdx / md, mdy / md);
                let base = if m.hunter {
                    // md is the distance to the player (hunters always charge it).
                    // Start a catch-up surge when too far behind; end it once the
                    // gap is halved.
                    if m.turbo_to == 0.0 && md > RELIC_HUNTER_FAR {
                        m.turbo_to = md * 0.5;
                    }
                    if m.turbo_to > 0.0 {
                        if md <= m.turbo_to {
                            m.turbo_to = 0.0;
                            hunter_speed
                        } else {
                            hunter_speed * RELIC_TURBO_MULT
                        }
                    } else {
                        hunter_speed
                    }
                } else {
                    m.speed * mon_speed
                };
                let step = base * scale * dt;
                // Axis-separated so monsters respect terrain (water/mountains).
                let nx = m.x + ux * step;
                if passable_px(seed, nx, m.y) {
                    m.x = nx;
                }
                let ny = m.y + uy * step;
                if passable_px(seed, m.x, ny) {
                    m.y = ny;
                }
            }

            // Hard separation: neither side may occupy the other's space. If the
            // monster (or the player walking into it) has overlapped, push the
            // monster back out to the edge. This is what stops ranged monsters
            // from burrowing onto the player and no-op'ing their shots.
            let min_sep = m.radius + PLAYER_R;
            let sdx = m.x - px;
            let sdy = m.y - py;
            let sd = (sdx * sdx + sdy * sdy).sqrt();
            if sd < min_sep {
                let (ux, uy) = if sd > 0.001 { (sdx / sd, sdy / sd) } else { (1.0, 0.0) };
                let tx = px + ux * min_sep;
                let ty = py + uy * min_sep;
                if passable_px(seed, tx, m.y) {
                    m.x = tx;
                }
                if passable_px(seed, m.x, ty) {
                    m.y = ty;
                }
            }

            // Recompute distance after separation for accurate attack checks.
            let dist = ((m.x - px).powi(2) + (m.y - py).powi(2)).sqrt();

            // Attacks. Fighters (and provoked monsters) strike; fleeing ranged
            // monsters kite — shooting back over their shoulder as they run.
            let contact = m.radius + PLAYER_R + 2.0;
            if m.cd <= 0.0 && notices {
                if hostile && !m.ranged && dist < contact {
                    player_dmg += m.atk;
                    if m.atk >= melee_best {
                        melee_best = m.atk;
                        melee_src = Some((m.name.clone(), m.dmg_type));
                    }
                    m.cd = m.cooldown;
                } else if (hostile || fleeing) && m.ranged && dist > contact {
                    let (ux, uy) = (dx / dist, dy / dist);
                    let ps = monster_proj_speed(m.speed);
                    new_projs.push(Proj {
                        x: m.x,
                        y: m.y,
                        vx: ux * ps,
                        vy: uy * ps,
                        life: 3.0,
                        dmg: m.atk,
                        dmg_type: m.dmg_type,
                        special: 0,
                        class_skill: 0,
                        from_player: false,
                        src_name: m.name.clone(),
                    });
                    m.cd = m.cooldown;
                }
            }
        }

        self.projectiles.append(&mut new_projs);
        if player_dmg > 0.0 {
            let (name, elem) = melee_src
                .as_ref()
                .map(|(n, e)| (n.as_str(), *e))
                .unwrap_or(("", PHYS));
            self.damage_player(player_dmg, Hurt::Attack { name, elem, ranged: false });
        }
        self.separate_monsters(seed);
    }

    /// Push overlapping monsters apart so crowds spread into a legible ring
    /// instead of stacking into one square. Only monsters are moved (never the
    /// player), and pushes respect terrain — so a pack can still pin you against
    /// a wall, which is a feature, not a bug. O(n²), but monster counts are
    /// bounded (loaded window + the hunter cap).
    fn separate_monsters(&mut self, seed: u64) {
        let n = self.monsters.len();
        for a in 0..n {
            for b in (a + 1)..n {
                let dx = self.monsters[b].x - self.monsters[a].x;
                let dy = self.monsters[b].y - self.monsters[a].y;
                let d = (dx * dx + dy * dy).sqrt();
                let min_d = self.monsters[a].radius + self.monsters[b].radius;
                if d < min_d && d > 0.001 {
                    let push = (min_d - d) * 0.5;
                    let (ux, uy) = (dx / d, dy / d);
                    let (nax, nay) = (self.monsters[a].x - ux * push, self.monsters[a].y - uy * push);
                    let (nbx, nby) = (self.monsters[b].x + ux * push, self.monsters[b].y + uy * push);
                    if passable_px(seed, nax, self.monsters[a].y) {
                        self.monsters[a].x = nax;
                    }
                    if passable_px(seed, self.monsters[a].x, nay) {
                        self.monsters[a].y = nay;
                    }
                    if passable_px(seed, nbx, self.monsters[b].y) {
                        self.monsters[b].x = nbx;
                    }
                    if passable_px(seed, self.monsters[b].x, nby) {
                        self.monsters[b].y = nby;
                    }
                }
            }
        }
    }

    fn environment(&mut self, dt: f32) {
        let tile = tile_px(self.seed, self.player.x, self.player.y);
        let h = hazard(tile);
        if h > 0.0 {
            let diff = difficulty_at(self.player.x, self.player.y) as f32;
            self.damage_player(h * (0.5 + diff * 0.1) * dt, Hurt::Swamp);
        }
    }

    /// Apply damage to the player. Death/respawn is handled once at the end of
    /// `update` (see `respawn_if_dead`) so we never mutate the monster/projectile
    /// lists while another system is iterating them.
    fn damage_player(&mut self, base: f32, cause: Hurt) {
        if self.godmode {
            return; // dev/testing immunity
        }
        if self.player.hp <= 0.0 {
            return; // already dead this frame — keep the blow that actually killed
        }
        let reduce = skill_bonus(self.player.skills[SK_DEFENSE as usize]).min(0.75);
        let mut dmg = base * (1.0 - reduce);
        // A blue ward (shield shrine) soaks damage before health — until drained.
        if self.player.shield > 0.0 {
            let absorbed = dmg.min(self.player.shield);
            self.player.shield -= absorbed;
            dmg -= absorbed;
        }
        // The cursed relic's blue shield soaks damage before it reaches health.
        if let Some(r) = self.relic.as_mut() {
            let absorbed = dmg.min(r.shield);
            r.shield -= absorbed;
            dmg -= absorbed;
        }
        self.player.hp -= dmg;
        self.dmg_taken += dmg;
        self.player.train(SK_DEFENSE, 0.04 + dmg * 0.03);
        if self.player.hp <= 0.0 {
            self.last_kill = Some(hurt_desc(cause));
        }
    }

    fn respawn_if_dead(&mut self) {
        if self.player.hp > 0.0 {
            return;
        }
        self.deaths += 1;
        let death_d = dist_tiles(self.player.x, self.player.y) as u32;

        // Death cost: lose half your ammo...
        self.player.ammo /= 2;

        // ...and each equipped weapon takes 10% durability damage, breaking at 0.
        let mut worn: Vec<usize> = Vec::new();
        for s in 0..4 {
            let e = self.player.equip[s];
            if e >= 0 && !worn.contains(&(e as usize)) {
                worn.push(e as usize);
            }
        }
        for &i in &worn {
            let w = &mut self.player.inv[i];
            w.durability = (w.durability - 0.10).max(0.0);
        }
        // Reap broken weapons (durability 0) and remap equip references.
        let mut broke: Vec<String> = Vec::new();
        let mut remap = vec![-1i32; self.player.inv.len()];
        let mut kept: Vec<Weapon> = Vec::new();
        for (old, w) in self.player.inv.drain(..).enumerate() {
            if w.durability > 0.0 {
                remap[old] = kept.len() as i32;
                kept.push(w);
            } else {
                broke.push(w.name.clone());
            }
        }
        self.player.inv = kept;
        for s in 0..4 {
            let e = self.player.equip[s];
            self.player.equip[s] = if e >= 0 { remap[e as usize] } else { -1 };
        }

        // Respawn at the furthest banked checkpoint (origin if none).
        let (sx, sy) = safe_spawn(self.seed, self.player.cp_x, self.player.cp_y);
        self.player.x = sx;
        self.player.y = sy;
        self.player.refresh_maxhp();
        self.player.hp = self.player.maxhp;
        self.monsters.clear();
        self.loaded.clear();
        self.projectiles.clear();
        self.relic = None; // death lifts the curse (hunters cleared with the monsters)
        // Dying in an arena forfeits it (it was already consumed on entry, so no
        // cache) — retire any active ring so the wipe isn't read as a "clear".
        for a in &mut self.arenas {
            if a.state == ArenaState::Active {
                a.state = ArenaState::Done;
            }
        }

        // Safety net: if every weapon broke and the pack is empty, a basic blade
        // (scaled to where you respawn) answers your need so you're never stranded
        // unable to fight.
        let refitted = self.player.inv.is_empty();
        if refitted {
            let power = difficulty_at(self.player.x, self.player.y) as f32;
            self.grant_basic_sword(power);
        }

        let cp_d = dist_tiles(sx, sy) as u32;
        let place = if cp_d < 1 {
            "the origin".to_string()
        } else {
            format!("checkpoint {}", cp_d)
        };
        let mut m = format!("You fell at distance {death_d}");
        if let Some(cause) = self.last_kill.take() {
            m.push_str(&format!(" to {cause}"));
        }
        m.push_str(&format!(
            ". Spirit restored — you reawaken at {place}. Half your ammo is lost."
        ));
        if !broke.is_empty() {
            m.push_str(&format!(" Your {} shattered!", broke.join(" and ")));
        }
        if refitted {
            m.push_str(" A basic blade answers your need.");
        }
        self.set_message(m);
        self.msg_t = 6.0; // linger so a respawn reads as an event, not a glitch
    }

    fn pickups(&mut self) {
        let (px, py) = (self.player.x, self.player.y);
        let mut i = 0;
        while i < self.loot.len() {
            let dx = self.loot[i].x - px;
            let dy = self.loot[i].y - py;
            if dx * dx + dy * dy <= 100.0 {
                // Weapons (and chests) are bounded by a generous inventory cap so
                // the list can't grow forever. A full pack leaves the item on the
                // ground until room is freed. Ammo/health/fountains are consumed
                // instantly, so they're never blocked.
                let adds_item = matches!(self.loot[i].kind, Drop::Weapon { .. } | Drop::Chest { .. });
                if adds_item && self.player.inv.len() >= INVENTORY_CAP {
                    self.set_message(format!(
                        "Pack full ({}/{}) — drop items to grab it.",
                        self.player.inv.len(),
                        INVENTORY_CAP
                    ));
                    self.msg_t = 2.0;
                    i += 1;
                    continue;
                }
                let l = self.loot.swap_remove(i);
                match l.kind {
                    Drop::Weapon { seed, power, .. } => {
                        let w = gen_weapon(seed, power);
                        let name = w.name.clone();
                        let idx = self.player.inv.len();
                        self.player.inv.push(w);
                        // Auto-equip into an empty slot for convenience.
                        let mut equipped_to = None;
                        for s in 0..4 {
                            if self.player.equip[s] < 0 {
                                self.player.equip[s] = idx as i32;
                                equipped_to = Some(s + 1);
                                break;
                            }
                        }
                        match equipped_to {
                            Some(s) => self.set_message(format!("Found {} (slot {})", name, s)),
                            None => self.set_message(format!("Found {}", name)),
                        }
                    }
                    Drop::Ammo(n) => {
                        self.player.ammo += n;
                        self.set_message(format!("+{} ammo ({} total)", n, self.player.ammo));
                    }
                    Drop::Health(h) => {
                        let before = self.player.hp;
                        self.player.hp = (self.player.hp + h).min(self.player.maxhp);
                        self.healed += self.player.hp - before;
                        self.set_message(format!("+{} health", h as u32));
                    }
                    Drop::Fountain => {
                        let before = self.player.hp;
                        self.player.hp = self.player.maxhp;
                        self.healed += self.player.hp - before;
                        self.fountains_used += 1;
                        remember_chunk(&mut self.looted_fountains, chunk_of(l.x, l.y));
                        self.set_message("A health fountain — fully restored!".into());
                        self.msg_t = 3.5;
                    }
                    Drop::Chest { seed, power } => {
                        self.chests_opened += 1;
                        remember_chunk(&mut self.looted_chests, chunk_of(l.x, l.y));
                        let w = generate_unique(seed, power);
                        let name = w.name.clone();
                        let idx = self.player.inv.len();
                        self.player.inv.push(w);
                        let mut equipped_to = None;
                        for s in 0..4 {
                            if self.player.equip[s] < 0 {
                                self.player.equip[s] = idx as i32;
                                equipped_to = Some(s + 1);
                                break;
                            }
                        }
                        match equipped_to {
                            Some(s) => self.set_message(format!("Ancient chest! Claimed {} (slot {})", name, s)),
                            None => self.set_message(format!("Ancient chest! Claimed {}", name)),
                        }
                        self.msg_t = 5.0;
                    }
                    Drop::Relic { power } => {
                        remember_chunk(&mut self.looted_relics, chunk_of(l.x, l.y));
                        self.begin_relic(power);
                    }
                    Drop::Shield { amount } => {
                        remember_chunk(&mut self.looted_shields, chunk_of(l.x, l.y));
                        // Non-recharging: replace whatever ward you had with this one.
                        self.player.shield = amount;
                        self.player.shield_max = amount;
                        self.set_message(format!(
                            "A shield shrine wards you — a {} blue shield.",
                            amount as u32
                        ));
                        self.msg_t = 4.0;
                    }
                }
            } else {
                i += 1;
            }
        }
    }

    /// Seize the cursed relic: a blistering speed burst and a blue shield, but a
    /// swelling hunt — and the relic is the only weapon you can use until it fades
    /// (after RELIC_STEPS steps, or on death). Does not use an inventory slot.
    fn begin_relic(&mut self, power: f32) {
        let shield_max = 40.0 + self.player.maxhp * 0.5;
        self.relic = Some(Relic {
            steps: 0.0,
            shield: shield_max,
            shield_max,
            hunt_cd: 0.5,
            weapon: make_relic_weapon(power),
            power,
        });
        self.set_message(
            "A cursed relic seizes you — SPEED and a shield, but the hunt is on. Run!".into(),
        );
        self.msg_t = 6.0;
    }

    /// The curse lifts: clear the hunters it summoned and drop the relic weapon.
    fn end_relic(&mut self, msg: &str) {
        self.relic = None;
        self.monsters.retain(|m| !m.hunter);
        self.set_message(msg.into());
        self.msg_t = 5.0;
    }

    /// One real-time random roll in [0,1) — for ambush timing etc.
    fn roll01(&mut self) -> f32 {
        self.event_rng = rng::mix64(self.event_rng.wrapping_add(0x9E37_79B9_7F4A_7C15));
        rng::u01(self.event_rng)
    }

    /// Campfire rest: trickle-heal while adjacent, and risk an ambush the further
    /// above half HP you push (safe up to 50%).
    fn update_campfire(&mut self, dt: f32) {
        if self.campfire_ambush_cd > 0.0 {
            self.campfire_ambush_cd -= dt;
        }
        let (px, py) = (self.player.x, self.player.y);
        let near = self.campfires.iter().any(|c| {
            ((px - c.x).powi(2) + (py - c.y).powi(2)).sqrt() <= CAMPFIRE_REST_RADIUS
        });
        self.resting = near;
        if !near {
            return;
        }
        // Trickle-heal.
        if self.player.hp < self.player.maxhp {
            let before = self.player.hp;
            self.player.hp =
                (self.player.hp + self.player.maxhp * CAMPFIRE_REGEN_FRAC * dt).min(self.player.maxhp);
            self.healed += self.player.hp - before;
        }
        // Ambush risk (only above half HP), on a Poisson roll, with a breather.
        if self.campfire_ambush_cd <= 0.0 {
            let chance = campfire_ambush_chance(self.player.hp, self.player.maxhp) * dt;
            if chance > 0.0 && self.roll01() < chance {
                self.spawn_ambush();
                self.campfire_ambush_cd = CAMPFIRE_AMBUSH_COOLDOWN;
                self.set_message("Ambush! Something crept up on your rest.".into());
                self.msg_t = 4.0;
            }
        }
    }

    /// Whether the player is on or next to a water tile.
    fn near_water(&self) -> bool {
        let ptx = (self.player.x / TILE).floor() as i64;
        let pty = (self.player.y / TILE).floor() as i64;
        [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .any(|&(dx, dy)| tile_at(self.seed, ptx + dx, pty + dy) <= world::SHALLOW_WATER)
    }

    /// No monster is close enough to make fishing dangerous.
    fn no_threat(&self) -> bool {
        let (px, py) = (self.player.x, self.player.y);
        !self
            .monsters
            .iter()
            .any(|m| ((m.x - px).powi(2) + (m.y - py).powi(2)).sqrt() < FISH_SAFE_RADIUS)
    }

    /// Whether the player can start fishing right now (water + calm + bait).
    fn can_fish(&self) -> bool {
        self.player.ammo >= FISH_BAIT && (self.force_fish || (self.near_water() && self.no_threat()))
    }

    /// Resolve a fishing attempt. `quality`: >=0 a landed catch (0..1 skill), -1
    /// escaped (bait lost), <=-2 cancelled before a bite (no bait spent).
    fn do_fish(&mut self, quality: f32) {
        if quality <= -2.0 || self.player.ammo < FISH_BAIT {
            return; // backed out, or somehow no bait
        }
        self.player.ammo -= FISH_BAIT; // the bait is spent on the cast
        if quality < 0.0 {
            self.set_message("The line goes slack — it slipped off with your bait.".into());
            self.msg_t = 3.0;
            return;
        }
        let q = quality.clamp(0.0, 1.0);
        let power = difficulty_at(self.player.x, self.player.y) as f32;
        let item_p = 0.06 + q * 0.14; // rarer than health; better with a clean catch
        let x = self.roll01();
        if x < item_p {
            let seed = self.roll_u64();
            let w = if self.roll01() < 0.25 {
                generate_unique(seed, power) // a rare sunken treasure
            } else {
                gen_weapon(seed, power)
            };
            let name = w.name.clone();
            self.grant_weapon(w);
            self.set_message(format!("You haul {name} from the depths!"));
        } else if x < item_p + 0.20 {
            let amt = FISH_BAIT + 4 + (q * 20.0) as u32;
            self.player.ammo += amt;
            self.set_message(format!("A lost quiver — +{amt} ammo."));
        } else if x < item_p + 0.24 {
            self.set_message("…an old boot. Nothing but junk.".into());
        } else {
            let heal = self.player.maxhp * (0.12 + q * 0.18); // 12%..30% by skill
            let before = self.player.hp;
            self.player.hp = (self.player.hp + heal).min(self.player.maxhp);
            self.healed += self.player.hp - before;
            self.set_message(format!("A fine catch — +{} health.", heal as u32));
        }
        self.msg_t = 4.0;
    }

    /// Whether the player is close enough to a shrine to make an offering.
    fn shrine_adjacent(&self) -> bool {
        let (px, py) = (self.player.x, self.player.y);
        self.shrines
            .iter()
            .any(|s| ((px - s.x).powi(2) + (py - s.y).powi(2)).sqrt() <= SHRINE_RADIUS)
    }

    /// Sacrifice the selected inventory items (by index) at a shrine and receive a
    /// reward that scales with how many items are offered. Offering an ancient
    /// (unique) item is a gamble: a double blessing, or an enraged guardian.
    fn make_offering(&mut self, indices: &[usize]) {
        if !self.shrine_adjacent() {
            return;
        }
        // Only unequipped, in-range items; de-duped.
        let mut sel: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&i| i < self.player.inv.len() && !self.player.equip.contains(&(i as i32)))
            .collect();
        sel.sort_unstable();
        sel.dedup();
        if sel.is_empty() {
            return;
        }
        let n = sel.len();
        let has_ancient = sel.iter().any(|&i| self.player.inv[i].unique);
        let sac_power: f32 = sel.iter().map(|&i| self.player.inv[i].power).sum();
        // Remove the offered items (descending so indices stay valid).
        for &i in sel.iter().rev() {
            self.drop_item(i);
        }
        // Reward strength scales with the count and the offered items' power.
        let power = difficulty_at(self.player.x, self.player.y) as f32 + n as f32 * 1.2 + sac_power * 0.1;

        let msg = if has_ancient {
            if self.roll01() < 0.5 {
                let a = self.grant_offering_reward(power, n);
                let b = self.grant_offering_reward(power, n);
                format!("The ancients accept your tribute — a DOUBLE blessing: {a} and {b}!")
            } else {
                self.spawn_offering_boss();
                "You fed an ancient to the flames — its enraged guardian rises!".to_string()
            }
        } else {
            let a = self.grant_offering_reward(power, n);
            format!("The shrine consumes {n} offerings and grants {a}.")
        };
        self.set_message(msg);
        self.msg_t = 6.0;
    }

    /// Grant one offering reward (weighted by item count) and return a short
    /// description. More items → far more likely to be a powerful ancient.
    fn grant_offering_reward(&mut self, power: f32, n: usize) -> String {
        let ancient_chance = (0.04 + n as f32 * 0.016).min(0.95); // n=60 -> ~1.0
        if self.roll01() < ancient_chance {
            let seed = self.roll_u64();
            let w = generate_unique(seed, power);
            let name = w.name.clone();
            self.grant_weapon(w);
            format!("an ancient {name}")
        } else {
            match self.roll01() {
                r if r < 0.5 => {
                    let seed = self.roll_u64();
                    let w = gen_weapon(seed, power);
                    let name = w.name.clone();
                    self.grant_weapon(w);
                    format!("a forged {name}")
                }
                r if r < 0.8 => {
                    let amt = 20 + n as u32 * 8;
                    self.player.ammo += amt;
                    format!("{amt} ammo")
                }
                _ => {
                    let heal = self.player.maxhp * (0.2 + n as f32 * 0.01);
                    let before = self.player.hp;
                    self.player.hp = (self.player.hp + heal).min(self.player.maxhp);
                    self.healed += self.player.hp - before;
                    format!("{} health", heal as u32)
                }
            }
        }
    }

    /// Push a reward weapon into the pack, auto-equipping an empty slot.
    fn grant_weapon(&mut self, w: Weapon) {
        let idx = self.player.inv.len();
        self.player.inv.push(w);
        for s in 0..4 {
            if self.player.equip[s] < 0 {
                self.player.equip[s] = idx as i32;
                break;
            }
        }
    }

    /// A fresh deterministic u64 from the real-time event stream (weapon seeds).
    fn roll_u64(&mut self) -> u64 {
        self.event_rng = rng::mix64(self.event_rng.wrapping_add(0x2545_F491_4F6C_DD1D));
        self.event_rng
    }

    /// Spawn a lone enraged boss near the player (angered by an ancient offering).
    fn spawn_offering_boss(&mut self) {
        let tier = difficulty_at(self.player.x, self.player.y);
        let mut r = Rng::new(hash2(self.seed ^ 0x5A1B_0055, (self.play_secs * 977.0) as i64, 0));
        for _ in 0..12 {
            let (mut dx, mut dy) = (r.range(-1.0, 1.0), r.range(-1.0, 1.0));
            let mag = (dx * dx + dy * dy).sqrt();
            if mag < 0.01 {
                dx = 1.0;
                dy = 0.0;
            } else {
                dx /= mag;
                dy /= mag;
            }
            let d = r.range(70.0, 120.0);
            let x = self.player.x + dx * d;
            let y = self.player.y + dy * d;
            if passable_px(self.seed, x, y) {
                let (cx, cy) = chunk_of(x, y);
                let mut mon = gen_monster(r.next(), cx, cy, x, y, tier.max(1));
                make_mega(&mut mon);
                mon.temper = monster::FIGHT;
                mon.anger = 30.0;
                self.monsters.push(mon);
                return;
            }
        }
    }

    /// Spawn a small pack of hostile monsters around the resting player.
    fn spawn_ambush(&mut self) {
        let tier = difficulty_at(self.player.x, self.player.y);
        let n = (2 + tier / 8).min(5);
        let mut r = Rng::new(hash2(
            self.seed ^ 0xCA37_F1AE,
            (self.play_secs * 991.0) as i64,
            self.monsters.len() as i64,
        ));
        for i in 0..n {
            for _ in 0..10 {
                let (mut dx, mut dy) = (r.range(-1.0, 1.0), r.range(-1.0, 1.0));
                let mag = (dx * dx + dy * dy).sqrt();
                if mag < 0.01 {
                    dx = 1.0;
                    dy = 0.0;
                } else {
                    dx /= mag;
                    dy /= mag;
                }
                let d = r.range(48.0, 96.0); // close — a surprise
                let x = self.player.x + dx * d;
                let y = self.player.y + dy * d;
                if passable_px(self.seed, x, y) {
                    let (cx, cy) = chunk_of(x, y);
                    let mut mon = gen_monster(r.next(), cx, cy, x, y, tier.max(1));
                    mon.temper = monster::FIGHT;
                    mon.anger = 8.0; // pounce immediately
                    self.monsters.push(mon);
                    break;
                }
                let _ = i;
            }
        }
    }

    /// Advance the active cursed relic: regen the shield, spawn hunters up to the
    /// cap, and end the curse once the step quota is spent.
    fn update_relic(&mut self, dt: f32) {
        if self.relic.is_none() {
            return;
        }
        let done = {
            let r = self.relic.as_mut().unwrap();
            r.shield = (r.shield + RELIC_SHIELD_REGEN * dt).min(r.shield_max);
            r.hunt_cd -= dt;
            r.steps >= RELIC_STEPS
        };
        if done {
            self.end_relic("The relic crumbles to dust — the hunt is over.");
            return;
        }
        // Warded ground: inside an active arena's sealed ring the curse can't
        // summon new hunters (the chasing pack was cleared on entry, and the ring
        // seal would wipe any arrival anyway). Shield regen and step progress
        // above still run — the sprint keeps advancing and can even finish here —
        // the hunt just pauses. Hold the spawn timer ready so pursuit resumes the
        // instant you step out.
        if self.arenas.iter().any(|a| a.state == ArenaState::Active) {
            if let Some(r) = self.relic.as_mut() {
                r.hunt_cd = r.hunt_cd.max(0.0);
            }
            return;
        }
        // Spawn a hunter when the timer elapses and we're under the cap.
        let spawn = self.relic.as_ref().map_or(false, |r| r.hunt_cd <= 0.0);
        if spawn {
            let live = self.monsters.iter().filter(|m| m.hunter).count();
            if live < RELIC_HUNTER_CAP {
                self.spawn_hunter();
            }
            if let Some(r) = self.relic.as_mut() {
                r.hunt_cd = RELIC_HUNT_INTERVAL;
            }
        }
    }

    /// Spawn a single hunter just beyond the view, on passable ground, hostile.
    fn spawn_hunter(&mut self) {
        let mut r = Rng::new(hash2(
            self.seed ^ 0x4E75_1CE1,
            (self.play_secs * 997.0) as i64,
            self.monsters.len() as i64,
        ));
        let tier = difficulty_at(self.player.x, self.player.y);
        for _ in 0..12 {
            let (mut dx, mut dy) = (r.range(-1.0, 1.0), r.range(-1.0, 1.0));
            let mag = (dx * dx + dy * dy).sqrt();
            if mag < 0.01 {
                dx = 1.0;
                dy = 0.0;
            } else {
                dx /= mag;
                dy /= mag;
            }
            let d = r.range(190.0, 260.0); // just off-screen so they "arrive"
            let x = self.player.x + dx * d;
            let y = self.player.y + dy * d;
            if passable_px(self.seed, x, y) {
                let (cx, cy) = chunk_of(x, y);
                let ms = r.next();
                let mut mon = gen_monster(ms, cx, cy, x, y, tier.max(1));
                mon.hunter = true;
                mon.ranged = false; // melee only — they always close in
                mon.temper = monster::FIGHT;
                mon.anger = 9_999.0; // relentless
                self.monsters.push(mon);
                return;
            }
        }
    }

    /// Remove one inventory item, fixing up equip references.
    fn drop_item(&mut self, i: usize) {
        if i >= self.player.inv.len() {
            return;
        }
        self.player.inv.remove(i);
        let i = i as i32;
        for s in 0..4 {
            if self.player.equip[s] == i {
                self.player.equip[s] = -1;
            } else if self.player.equip[s] > i {
                self.player.equip[s] -= 1;
            }
        }
    }

    /// Remove every unequipped inventory item weaker than item `i` (by dps),
    /// fixing up equip references. Equipped and unique weapons are always kept —
    /// uniques can only be trashed individually with the ✕ button.
    fn drop_below(&mut self, i: usize) {
        if i >= self.player.inv.len() {
            return;
        }
        let dps = |w: &Weapon| w.damage / w.cooldown.max(0.05);
        let threshold = dps(&self.player.inv[i]);
        let equip = self.player.equip;

        let mut remap = vec![-1i32; self.player.inv.len()];
        let mut kept: Vec<Weapon> = Vec::new();
        for (old, w) in self.player.inv.drain(..).enumerate() {
            let equipped = equip.iter().any(|&e| e == old as i32);
            if dps(&w) >= threshold || equipped || w.unique {
                remap[old] = kept.len() as i32;
                kept.push(w);
            }
        }
        self.player.inv = kept;
        for s in 0..4 {
            let e = self.player.equip[s];
            self.player.equip[s] = if e >= 0 { remap[e as usize] } else { -1 };
        }
    }

    // --- snapshot --------------------------------------------------------

    fn build_snapshot(&mut self) {
        // The logical viewport width is fixed (constant horizontal zoom); the
        // height tracks the device aspect ratio so the game fills any screen /
        // orientation instead of letterboxing.
        let view_h = self.view_h;
        let cam_x = self.player.x - LOGICAL_W * 0.5;
        let cam_y = self.player.y - view_h * 0.5;
        let tx0 = (cam_x / TILE).floor() as i64;
        let ty0 = (cam_y / TILE).floor() as i64;
        let cols: u16 = (LOGICAL_W / TILE) as u16 + 2;
        let rows: u16 = (view_h / TILE) as u16 + 2;

        let mut w = Writer::new(&mut self.snap);
        w.f32(cam_x);
        w.f32(cam_y);
        w.i32(tx0 as i32);
        w.i32(ty0 as i32);
        w.u16(cols);
        w.u16(rows);
        // Terrain tiles, then the decorative feature layer (kept in a buffer so
        // we only evaluate the expensive terrain noise once per tile).
        let mut tile_buf: Vec<u8> = Vec::with_capacity((cols as usize) * (rows as usize));
        for ry in 0..rows as i64 {
            for rx in 0..cols as i64 {
                let t = tile_at(self.seed, tx0 + rx, ty0 + ry);
                tile_buf.push(t);
                w.u8(t);
            }
        }
        let mut k = 0usize;
        for ry in 0..rows as i64 {
            for rx in 0..cols as i64 {
                let t = tile_buf[k];
                k += 1;
                w.u8(feature_at(self.seed, tx0 + rx, ty0 + ry, t));
            }
        }

        // Player + progression.
        w.f32(self.player.x);
        w.f32(self.player.y);
        w.f32(self.player.hp);
        w.f32(self.player.maxhp);
        let dist = ((self.player.x / TILE).powi(2) + (self.player.y / TILE).powi(2)).sqrt();
        w.f32(dist);
        w.u16(difficulty_at(self.player.x, self.player.y) as u16);
        w.u16(self.player.ammo.min(65535) as u16);
        w.f32(self.player.max_dist);
        w.f32(dist_tiles(self.player.cp_x, self.player.cp_y)); // checkpoint distance
        w.u8(self.celebrating as u8);
        w.f32(self.celebrate_t);
        w.f32(self.milestone_t);
        for s in 0..8 {
            w.u16(skill_level(self.player.skills[s]) as u16);
            w.f32(self.player.skills[s]);
        }
        w.u8(self.player.slot as u8);

        // Equipped slots (for HUD).
        for s in 0..4 {
            let idx = self.player.equip[s];
            if idx >= 0 {
                if let Some(weap) = self.player.inv.get(idx as usize) {
                    w.u8(1);
                    w.u8(weap.rarity);
                    w.u8(weap.dmg_type);
                    w.u8((weap.durability * 100.0).round() as u8);
                    w.str(&weap.name);
                    continue;
                }
            }
            w.u8(0);
        }

        // Entities: monsters, projectiles, loot, arena rings.
        let count =
            self.monsters.len() + self.projectiles.len() + self.loot.len() + self.arenas.len()
                + self.campfires.len() + self.shrines.len() + self.miasmas.len()
                + self.vaults.len() + self.rifts.len();
        w.u16(count as u16);
        for m in &self.monsters {
            w.u8(1);
            w.f32(m.x);
            w.f32(m.y);
            w.u8(m.radius as u8);
            w.u8(((m.hp / m.maxhp).clamp(0.0, 1.0) * 255.0) as u8);
            // High bits of `shape` flag a mega (0x80), a relic hunter (0x40),
            // and a champion (0x20).
            w.u8(m.body
                | if m.mega { 0x80 } else { 0 }
                | if m.hunter { 0x40 } else { 0 }
                | if m.champion { 0x20 } else { 0 });
            w.u8(m.dmg_type);
        }
        for p in &self.projectiles {
            w.u8(if p.from_player { 2 } else { 3 });
            w.f32(p.x);
            w.f32(p.y);
            w.u8(2);
            w.u8(255);
            w.u8(p.dmg_type);
            w.u8(0);
        }
        for l in &self.loot {
            // kind: 4 weapon, 5 ammo, 6 health, 7 chest, 8 fountain, 10 relic.
            let (kind, shape) = match l.kind {
                Drop::Weapon { rarity, .. } => (4u8, rarity),
                Drop::Ammo(_) => (5u8, 0),
                Drop::Health(_) => (6u8, 0),
                Drop::Chest { .. } => (7u8, 3),
                Drop::Fountain => (8u8, 0),
                Drop::Relic { .. } => (10u8, 0),
                Drop::Shield { .. } => (14u8, 0),
            };
            w.u8(kind);
            w.f32(l.x);
            w.f32(l.y);
            w.u8(3);
            w.u8(255);
            w.u8(shape);
            w.u8(0);
        }
        for a in &self.arenas {
            // kind 9 = arena ring: inner radius in "radius", outer in "dtype",
            // state in "shape".
            w.u8(9);
            w.f32(a.x);
            w.f32(a.y);
            w.u8(ARENA_INNER as u8);
            w.u8(0);
            w.u8(match a.state {
                ArenaState::Active => 1,
                ArenaState::Done => 2,     // forfeited/abandoned
                ArenaState::Cleared => 4,  // conquered — a victory ring
                // 3 = idle + player near: a pulsing "telegraph" ring inviting entry.
                ArenaState::Idle => {
                    let pd = ((self.player.x - a.x).powi(2) + (self.player.y - a.y).powi(2)).sqrt();
                    if pd <= ARENA_TELEGRAPH {
                        3
                    } else {
                        0
                    }
                }
            });
            w.u8(ARENA_OUTER as u8);
        }
        for c in &self.campfires {
            // kind 11 = campfire rest site.
            w.u8(11);
            w.f32(c.x);
            w.f32(c.y);
            w.u8(0);
            w.u8(0);
            w.u8(0);
            w.u8(0);
        }
        for s in &self.shrines {
            // kind 12 = offering shrine.
            w.u8(12);
            w.f32(s.x);
            w.f32(s.y);
            w.u8(0);
            w.u8(0);
            w.u8(0);
            w.u8(0);
        }
        for m in &self.miasmas {
            // kind 13 = cursed fog: radius (px) in "radius"; client draws the
            // haze + a closing vignette and hides monsters while you're inside.
            w.u8(13);
            w.f32(m.x);
            w.f32(m.y);
            w.u8(m.r as u8);
            w.u8(0);
            w.u8(0);
            w.u8(0);
        }
        for v in &self.vaults {
            // kind 15 = rune vault; "shape" carries the opened flag for the sprite.
            w.u8(15);
            w.f32(v.x);
            w.f32(v.y);
            w.u8(0);
            w.u8(0);
            w.u8(v.opened as u8);
            w.u8(0);
        }
        for rf in &self.rifts {
            // kind 16 = rift portal (a forward-leap into deeper danger).
            w.u8(16);
            w.f32(rf.x);
            w.f32(rf.y);
            w.u8(0);
            w.u8(0);
            w.u8(0);
            w.u8(0);
        }

        // Nearest monster as the HUD "target".
        let mut nearest: Option<(&Monster, f32)> = None;
        for m in &self.monsters {
            let dx = m.x - self.player.x;
            let dy = m.y - self.player.y;
            let d = dx * dx + dy * dy;
            if nearest.map_or(true, |(_, bd)| d < bd) {
                nearest = Some((m, d));
            }
        }
        match nearest {
            Some((m, d2)) if d2 < 200.0 * 200.0 => {
                w.u8(1);
                w.str(&m.name);
                w.u16(m.level as u16);
                w.u8(m.weak);
                w.u8(m.resist);
                w.u8((m.hp / m.maxhp * 255.0) as u8);
            }
            _ => w.u8(0),
        }

        // Message.
        if self.msg_t > 0.0 {
            w.str(&self.message);
        } else {
            w.str("");
        }

        // Full inventory (for the I screen).
        w.u16(self.player.inv.len() as u16);
        for (i, weap) in self.player.inv.iter().enumerate() {
            w.u8(weap.rarity);
            w.u8(weap.dmg_type);
            w.u8(weap.base);
            w.f32(weap.damage);
            w.f32(weap.cooldown);
            let mut eq = 255u8;
            for s in 0..4 {
                if self.player.equip[s] == i as i32 {
                    eq = s as u8;
                }
            }
            w.u8(eq);
            w.u8((weap.durability * 100.0).round() as u8);
            w.u8(weap.unique as u8);
            w.str(&weap.name);
        }

        // Lifetime stats (for the HUD and the 100,000 celebration).
        w.i32(self.kills as i32);
        w.i32(self.deaths as i32);
        w.i32(self.chests_opened as i32);
        w.i32(self.fountains_used as i32);
        w.f32(self.steps as f32);
        w.f32(self.play_secs);
        w.i32(self.mega_kills as i32);

        // Active-arena HUD state (0/none, else current wave / total, plus a flag
        // for "in the rotting apron").
        let active = self.arenas.iter().find(|a| a.state == ArenaState::Active);
        w.u8(active.is_some() as u8);
        w.u8(active.map_or(0, |a| a.wave));
        w.u8(active.map_or(0, |a| a.waves));
        let in_rot = active.map_or(false, |a| {
            let pd = ((self.player.x - a.x).powi(2) + (self.player.y - a.y).powi(2)).sqrt();
            pd > ARENA_INNER + 4.0
        });
        w.u8(in_rot as u8);
        // Ready-steady-go countdown, in whole seconds remaining (0 = fighting).
        w.u8(active.map_or(0, |a| a.countdown.ceil().max(0.0) as u8));
        // Entry telegraph: player is near an idle ring they haven't entered yet.
        let near_idle = active.is_none()
            && self.arenas.iter().any(|a| {
                a.state == ArenaState::Idle
                    && ((self.player.x - a.x).powi(2) + (self.player.y - a.y).powi(2)).sqrt()
                        <= ARENA_TELEGRAPH
            });
        w.u8(near_idle as u8);

        // Cursed-relic HUD state: active, steps remaining (of RELIC_STEPS), the
        // blue shield, and the relic weapon's name.
        match &self.relic {
            Some(r) => {
                w.u8(1);
                w.u16(r.steps.max(0.0) as u16);
                w.u16(RELIC_STEPS as u16);
                w.f32(r.shield);
                w.f32(r.shield_max);
                w.str(&r.weapon.name);
            }
            None => {
                w.u8(0);
                w.u16(0);
                w.u16(RELIC_STEPS as u16);
                w.f32(0.0);
                w.f32(0.0);
                w.str("");
            }
        }

        // Campfire rest state: resting (adjacent) + whether it's currently safe
        // (<=50% HP, no ambush risk yet).
        w.u8(self.resting as u8);
        let safe = campfire_ambush_chance(self.player.hp, self.player.maxhp) <= 0.0;
        w.u8(safe as u8);
        // Offering shrine: whether the player is standing at one (inlined so it
        // doesn't borrow all of `self` while the writer holds `self.snap`).
        let (spx, spy) = (self.player.x, self.player.y);
        let at_shrine = self
            .shrines
            .iter()
            .any(|s| ((spx - s.x).powi(2) + (spy - s.y).powi(2)).sqrt() <= SHRINE_RADIUS);
        w.u8(at_shrine as u8);

        // Fishing: on/next to water, calm, and holding bait (inlined for the same
        // disjoint-borrow reason as the shrine check above).
        let ptx = (spx / TILE).floor() as i64;
        let pty = (spy / TILE).floor() as i64;
        let water = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .any(|&(dx, dy)| tile_at(self.seed, ptx + dx, pty + dy) <= world::SHALLOW_WATER);
        let calm = !self
            .monsters
            .iter()
            .any(|m| ((spx - m.x).powi(2) + (spy - m.y).powi(2)).sqrt() < FISH_SAFE_RADIUS);
        w.u8((self.player.ammo >= FISH_BAIT && (self.force_fish || (water && calm))) as u8);

        // Player blue ward (shield shrine): appended so older readers are unaffected.
        w.f32(self.player.shield);
        w.f32(self.player.shield_max);

        // Standing at an unopened rune vault (start the puzzle overlay).
        let at_vault = self
            .vaults
            .iter()
            .any(|v| !v.opened && ((spx - v.x).powi(2) + (spy - v.y).powi(2)).sqrt() <= VAULT_RADIUS);
        w.u8(at_vault as u8);
    }

    // --- persistence -----------------------------------------------------

    fn build_save(&mut self) {
        self.save.clear();
        let mut w = Writer::new(&mut self.save);
        w.u8(b'W');
        w.u8(15); // version
        w.u64(self.seed);
        w.f32(self.player.x);
        w.f32(self.player.y);
        for s in 0..8 {
            w.f32(self.player.skills[s]);
        }
        w.u8(self.player.slot as u8);
        for s in 0..4 {
            w.i32(self.player.equip[s]);
        }
        w.u16(self.player.inv.len() as u16);
        for weap in &self.player.inv {
            w.u64(weap.seed);
            w.f32(weap.power);
            w.u8((weap.durability * 100.0).round() as u8); // v5+
            w.u8(weap.unique as u8); // v5+
        }
        w.u16(self.player.ammo.min(65535) as u16); // v2+
        w.f32(self.player.max_dist); // v3+
        w.f32(self.player.cp_x); // v4+
        w.f32(self.player.cp_y);
        // v6: persistent stats + the once-per-character celebration flag.
        w.i32(self.kills as i32);
        w.i32(self.deaths as i32);
        w.i32(self.chests_opened as i32);
        w.f32(self.steps as f32);
        w.f32(self.play_secs);
        w.u8(self.celebrated as u8);
        w.i32(self.fountains_used as i32); // v7
        // v8: looted chest/fountain chunks (so they can't be reload-farmed).
        w.u16(self.looted_chests.len() as u16);
        for &(cx, cy) in &self.looted_chests {
            w.i32(cx as i32);
            w.i32(cy as i32);
        }
        w.u16(self.looted_fountains.len() as u16);
        for &(cx, cy) in &self.looted_fountains {
            w.i32(cx as i32);
            w.i32(cy as i32);
        }
        w.i32(self.mega_kills as i32); // v9
        // v10: entered/consumed arena chunks (no pause/reload retry).
        w.u16(self.looted_arenas.len() as u16);
        for &(cx, cy) in &self.looted_arenas {
            w.i32(cx as i32);
            w.i32(cy as i32);
        }
        // v11: cursed-relic state (so the sprint survives a reload) + spent relics.
        match &self.relic {
            Some(rel) => {
                w.u8(1);
                w.f32(rel.steps);
                w.f32(rel.power);
                w.f32(rel.shield);
            }
            None => {
                w.u8(0);
                w.f32(0.0);
                w.f32(0.0);
                w.f32(0.0);
            }
        }
        w.u16(self.looted_relics.len() as u16);
        for &(cx, cy) in &self.looted_relics {
            w.i32(cx as i32);
            w.i32(cy as i32);
        }
        // v12: the shield-shrine ward + spent shrine chunks.
        w.f32(self.player.shield);
        w.f32(self.player.shield_max);
        w.u16(self.looted_shields.len() as u16);
        for &(cx, cy) in &self.looted_shields {
            w.i32(cx as i32);
            w.i32(cy as i32);
        }
        // v13: felled champion chunks (so a beaten champion stays beaten).
        w.u16(self.looted_champions.len() as u16);
        for &(cx, cy) in &self.looted_champions {
            w.i32(cx as i32);
            w.i32(cy as i32);
        }
        // v14: opened rune-vault chunks.
        w.u16(self.looted_vaults.len() as u16);
        for &(cx, cy) in &self.looted_vaults {
            w.i32(cx as i32);
            w.i32(cy as i32);
        }
        // v15: used rift chunks.
        w.u16(self.looted_rifts.len() as u16);
        for &(cx, cy) in &self.looted_rifts {
            w.i32(cx as i32);
            w.i32(cy as i32);
        }
    }

    fn load_save(&mut self, bytes: &[u8]) {
        let mut r = Reader::new(bytes);
        let ver = if r.u8() == b'W' { r.u8() } else { 0 };
        if ver == 0 {
            return;
        }
        let seed = r.u64();
        let mut g = Game::new(seed);
        g.player.inv.clear();
        g.player.x = r.f32();
        g.player.y = r.f32();
        for s in 0..8 {
            g.player.skills[s] = r.f32();
        }
        g.player.slot = r.u8() as usize;
        for s in 0..4 {
            g.player.equip[s] = r.i32();
        }
        // Collect raw weapon params now; rebuild them after max_dist is known so
        // we can clamp any corrupt (over-inflated) power to a sane ceiling.
        let n = r.u16() as usize;
        let mut raw_inv: Vec<(u64, f32, f32, bool)> = Vec::with_capacity(n);
        for _ in 0..n {
            let wseed = r.u64();
            let power = r.f32();
            let (durability, unique) = if ver >= 5 {
                (r.u8() as f32 / 100.0, r.u8() != 0)
            } else {
                (1.0, false)
            };
            raw_inv.push((wseed, power, durability, unique));
        }
        if ver >= 2 {
            g.player.ammo = r.u16() as u32; // older saves keep the default
        }
        if ver >= 3 {
            g.player.max_dist = r.f32();
        }
        if ver >= 4 {
            g.player.cp_x = r.f32();
            g.player.cp_y = r.f32();
        }
        if ver >= 6 {
            g.kills = r.i32().max(0) as u32;
            g.deaths = r.i32().max(0) as u32;
            g.chests_opened = r.i32().max(0) as u32;
            g.steps = r.f32() as f64;
            g.play_secs = r.f32();
            g.celebrated = r.u8() != 0;
        }
        if ver >= 7 {
            g.fountains_used = r.i32().max(0) as u32;
        }
        if ver >= 8 {
            let nc = r.u16() as usize;
            for _ in 0..nc {
                g.looted_chests.push((r.i32() as i64, r.i32() as i64));
            }
            let nf = r.u16() as usize;
            for _ in 0..nf {
                g.looted_fountains.push((r.i32() as i64, r.i32() as i64));
            }
        }
        if ver >= 9 {
            g.mega_kills = r.i32().max(0) as u32;
        }
        if ver >= 10 {
            let na = r.u16() as usize;
            for _ in 0..na {
                g.looted_arenas.push((r.i32() as i64, r.i32() as i64));
            }
        }
        if ver >= 11 {
            let active = r.u8() != 0;
            let steps = r.f32();
            let power = r.f32();
            let shield = r.f32();
            if active {
                // Resume the curse: regenerate the relic weapon from its power.
                let shield_max = 40.0 + g.player.maxhp * 0.5;
                g.relic = Some(Relic {
                    steps,
                    shield: shield.min(shield_max),
                    shield_max,
                    hunt_cd: 0.5,
                    weapon: make_relic_weapon(power),
                    power,
                });
            }
            let nr = r.u16() as usize;
            for _ in 0..nr {
                g.looted_relics.push((r.i32() as i64, r.i32() as i64));
            }
        }
        if ver >= 12 {
            g.player.shield = r.f32();
            g.player.shield_max = r.f32();
            let ns = r.u16() as usize;
            for _ in 0..ns {
                g.looted_shields.push((r.i32() as i64, r.i32() as i64));
            }
        }
        if ver >= 13 {
            let nch = r.u16() as usize;
            for _ in 0..nch {
                g.looted_champions.push((r.i32() as i64, r.i32() as i64));
            }
        }
        if ver >= 14 {
            let nv = r.u16() as usize;
            for _ in 0..nv {
                g.looted_vaults.push((r.i32() as i64, r.i32() as i64));
            }
        }
        if ver >= 15 {
            let nrf = r.u16() as usize;
            for _ in 0..nrf {
                g.looted_rifts.push((r.i32() as i64, r.i32() as i64));
            }
        }
        // Rebuild the inventory now that max_dist is known. Clamp each weapon's
        // stored power to the difficulty at the farthest point the player has
        // reached (plus a margin) — legit weapons never exceed that, but a save
        // corrupted by the old unique-power compounding bug is healed back to a
        // sane weapon instead of a multi-trillion-DPS one.
        let ref_d = g.player.max_dist.max(dist_tiles(g.player.x, g.player.y));
        let power_cap = (difficulty_at(ref_d * TILE, 0.0) as f32 * 1.2 + 5.0).max(1.0);
        for (wseed, power, durability, unique) in raw_inv {
            let p = if power > 0.0 { power.min(power_cap) } else { 1.0 };
            let mut weap = if unique { generate_unique(wseed, p) } else { gen_weapon(wseed, p) };
            weap.durability = durability;
            g.player.inv.push(weap);
        }

        g.seed_milestones(); // don't re-toast milestones already passed
        g.player.refresh_maxhp();
        g.player.hp = g.player.maxhp;
        // Now that maxhp is known, size a resumed relic's shield to it.
        if let Some(rel) = g.relic.as_mut() {
            rel.shield_max = 40.0 + g.player.maxhp * 0.5;
            rel.shield = rel.shield.min(rel.shield_max);
        }
        // Rescue saves that were stranded before the spawn-openness fix: if the
        // stored position is trapped (enclosed island/water), nudge to the nearest
        // open tile. safe_spawn's spiral keeps the move tiny, so distance is kept.
        let (ptx, pty) = ((g.player.x / TILE).floor() as i64, (g.player.y / TILE).floor() as i64);
        if !open_enough(g.seed, ptx, pty) {
            let (sx, sy) = safe_spawn(g.seed, g.player.x, g.player.y);
            g.player.x = sx;
            g.player.y = sy;
        }
        *self = g;
    }
}

// ---------------------------------------------------------------------------
// Serialization helpers
// ---------------------------------------------------------------------------

struct Writer<'a> {
    b: &'a mut Vec<u8>,
}
impl<'a> Writer<'a> {
    fn new(b: &'a mut Vec<u8>) -> Self {
        b.clear();
        Writer { b }
    }
    fn u8(&mut self, v: u8) {
        self.b.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.b.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.b.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.b.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.b.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        let by = s.as_bytes();
        let n = by.len().min(255);
        self.u8(n as u8);
        self.b.extend_from_slice(&by[..n]);
    }
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, p: 0 }
    }
    fn u8(&mut self) -> u8 {
        let v = self.b.get(self.p).copied().unwrap_or(0);
        self.p += 1;
        v
    }
    fn u16(&mut self) -> u16 {
        let v = u16::from_le_bytes([self.byte(0), self.byte(1)]);
        self.p += 2;
        v
    }
    fn i32(&mut self) -> i32 {
        let v = i32::from_le_bytes([self.byte(0), self.byte(1), self.byte(2), self.byte(3)]);
        self.p += 4;
        v
    }
    fn u64(&mut self) -> u64 {
        let mut a = [0u8; 8];
        for i in 0..8 {
            a[i] = self.byte(i);
        }
        self.p += 8;
        u64::from_le_bytes(a)
    }
    fn f32(&mut self) -> f32 {
        let v = f32::from_le_bytes([self.byte(0), self.byte(1), self.byte(2), self.byte(3)]);
        self.p += 4;
        v
    }
    fn byte(&self, o: usize) -> u8 {
        self.b.get(self.p + o).copied().unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// WASM exports
// ---------------------------------------------------------------------------

static mut GAME: Option<Game> = None;

const IO_CAP: usize = 1 << 15;
static mut IO: [u8; IO_CAP] = [0; IO_CAP];

fn game() -> &'static mut Game {
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(GAME);
        slot.as_mut().expect("init() not called")
    }
}

#[no_mangle]
pub extern "C" fn init(seed: u32) {
    unsafe {
        *core::ptr::addr_of_mut!(GAME) = Some(Game::new(seed as u64 | ((seed as u64) << 32 ^ 0xA5A5)));
    }
}

#[no_mangle]
pub extern "C" fn set_input(keys: u32, aimx: f32, aimy: f32, attack: u32, slot: u32) {
    let g = game();
    g.input.keys = keys;
    g.input.aimx = aimx;
    g.input.aimy = aimy;
    g.input.attack = attack != 0;
    if slot < 4 {
        g.player.select_slot(slot as usize);
    }
}

#[no_mangle]
pub extern "C" fn update(dt_ms: f32) {
    let dt = (dt_ms / 1000.0).clamp(0.0, 0.05);
    game().update(dt);
}

#[no_mangle]
pub extern "C" fn snapshot_ptr() -> *const u8 {
    game().snap.as_ptr()
}

#[no_mangle]
pub extern "C" fn snapshot_len() -> u32 {
    game().snap.len() as u32
}

/// Debug/testing: teleport the player to a given distance (in tiles) along +x
/// and record it. Used to reach/verify the 100,000 celebration; not called in
/// normal play.
#[no_mangle]
pub extern "C" fn debug_warp(tiles: f32) {
    let g = game();
    let (sx, sy) = safe_spawn(g.seed, tiles * TILE, 0.0); // never land stuck in water
    g.player.x = sx;
    g.player.y = sy;
    if tiles > g.player.max_dist {
        g.player.max_dist = tiles;
    }
    g.seed_milestones(); // skip mini-toasts for everything we warped past
}

/// Debug/testing: drop an arena ring near the player (centered on them when
/// `offset` is 0; placed `offset` px to the east so the entry telegraph can be
/// tested by walking in). Also arms the player with an overpowered spear and
/// damage immunity so mechanics can be exercised without dying. Clears the
/// chunk's "consumed" mark first so it can be re-tested.
#[no_mangle]
pub extern "C" fn debug_arena(offset: f32) {
    let g = game();
    // Testing loadout: a one-shot spear and immunity to any damage (incl. rot).
    let spear = Weapon {
        seed: 0,
        power: 1.0,
        durability: 1.0,
        unique: false,
        base: 3, // Spear (long melee reach)
        dmg_type: PHYS,
        damage: 99_999.0,
        cooldown: 0.12,
        range: 150.0, // covers the whole inner ring, so you can sweep from center
        ranged: false,
        proj_speed: 0.0,
        rarity: 3,
        class_skill: SK_SWORD,
        special: 0,
        name: "DEV Spear of Testing".into(),
    };
    let idx = g.player.inv.len();
    g.player.inv.push(spear);
    g.player.equip[0] = idx as i32;
    g.player.slot = 0;
    g.godmode = true;

    // Ring center: on the player (offset 0) or a bit east, on passable ground.
    let (px, py) = g.drop_pos(g.player.x + offset, g.player.y);
    let (cx, cy) = ((px / CHUNK_PX).floor() as i64, (py / CHUNK_PX).floor() as i64);
    g.looted_arenas.retain(|&c| c != (cx, cy));
    let waves = (2 + difficulty_at(px, py) / 40).clamp(2, 5) as u8;
    g.arenas.push(Arena {
        x: px,
        y: py,
        cx,
        cy,
        seed: hash2(g.seed ^ 0x5EED_A5EE, cx, cy),
        state: ArenaState::Idle,
        wave: 0,
        waves,
        countdown: 0.0,
    });
}

/// Debug/testing: seize a cursed relic on the spot to exercise the sprint.
#[no_mangle]
pub extern "C" fn debug_relic() {
    let g = game();
    let power = difficulty_at(g.player.x, g.player.y) as f32;
    g.begin_relic(power);
}

/// Debug/testing: drop a campfire on the player to exercise resting/ambush.
#[no_mangle]
pub extern "C" fn debug_campfire() {
    let g = game();
    let (cx, cy) = g.player_chunk();
    g.campfires.push(Campfire {
        x: g.player.x,
        y: g.player.y,
        cx,
        cy,
    });
}

/// Debug/testing: drop a cursed-fog patch (with its cache) just east of the
/// player so the vision effect and premium cache can be exercised.
#[no_mangle]
pub extern "C" fn debug_fog() {
    let g = game();
    let (cx, cy) = g.player_chunk();
    let x = g.player.x + 60.0;
    let y = g.player.y;
    g.miasmas.push(Miasma { x, y, cx, cy, r: MIASMA_R });
    let power = difficulty_at(x, y) as f32;
    let tier = difficulty_at(x, y);
    let seed = hash2(g.seed ^ 0xF06E_1A5E_CAFE_00D5, cx, cy);
    g.loot.push(Loot { x, y, kind: Drop::Chest { seed, power: power * 1.25 } });
    g.loot.push(Loot { x: x + TILE, y, kind: Drop::Health((40.0 + power).min(140.0)) });
    g.loot.push(Loot { x: x - TILE, y, kind: Drop::Ammo(40 + tier.min(90)) });
}

/// Debug/testing: spawn a champion a short walk east, with some ambient mobs
/// around it so the "no adds" clearing can be exercised.
#[no_mangle]
pub extern "C" fn debug_champion() {
    let g = game();
    let (cx, cy) = g.player_chunk();
    let x = g.player.x + 90.0;
    let y = g.player.y;
    let d = difficulty_at(x, y).max(3);
    let ms = monster_seed(g.seed, cx, cy, 777);
    let mut mon = gen_monster(ms, cx, cy, x, y, d);
    make_champion(&mut mon);
    g.monsters.push(mon);
    // A couple of ambient wanderers nearby to prove they're cleared during the duel.
    for k in 0..3u32 {
        let mx = x + (k as f32 - 1.0) * 20.0;
        let my = y + 30.0;
        g.monsters.push(gen_monster(monster_seed(g.seed, cx, cy, 20 + k), cx, cy, mx, my, d));
    }
}

/// Solve the rune vault the player stands at (the puzzle ran in the client).
#[no_mangle]
pub extern "C" fn open_vault() {
    game().open_vault_near();
}

/// Debug/testing: grant damage immunity so distant milestones can be walked to.
#[no_mangle]
pub extern "C" fn debug_god() {
    game().godmode = true;
}

/// Debug/testing: clear all monsters (and shots) so a path can be walked.
#[no_mangle]
pub extern "C" fn debug_clear() {
    let g = game();
    g.monsters.clear();
    g.projectiles.clear();
}

/// Debug/testing: kill the player so the death/respawn flow can be exercised.
#[no_mangle]
pub extern "C" fn debug_kill() {
    game().player.hp = -1.0;
}

/// Debug/testing: drop a rift a short walk east so the forward-leap can be tested.
#[no_mangle]
pub extern "C" fn debug_rift() {
    let g = game();
    let (cx, cy) = g.player_chunk();
    g.rifts.push(Rift {
        x: g.player.x + 40.0,
        y: g.player.y,
        cx,
        cy,
    });
}

/// Debug/testing: drop a rune vault right on the player so the puzzle can start.
#[no_mangle]
pub extern "C" fn debug_vault() {
    let g = game();
    let (cx, cy) = g.player_chunk();
    g.vaults.push(Vault {
        x: g.player.x,
        y: g.player.y,
        cx,
        cy,
        opened: false,
    });
}

/// Debug/testing: drop a shield shrine ward on the player (picked up at once).
#[no_mangle]
pub extern "C" fn debug_shield() {
    let g = game();
    let amount = (50.0 + difficulty_at(g.player.x, g.player.y) as f32 * 2.0).min(250.0);
    g.loot.push(Loot { x: g.player.x, y: g.player.y, kind: Drop::Shield { amount } });
}

/// Debug/testing: drop an offering shrine on the player, and fill the pack with
/// junk (plus one ancient) so the sacrifice flow can be exercised.
#[no_mangle]
pub extern "C" fn debug_shrine() {
    let g = game();
    let (cx, cy) = g.player_chunk();
    g.shrines.push(Shrine {
        x: g.player.x,
        y: g.player.y,
        cx,
        cy,
    });
    let power = difficulty_at(g.player.x, g.player.y) as f32;
    for k in 0..12u64 {
        g.player.inv.push(gen_weapon(0x1234 ^ k.wrapping_mul(0x9E37), power * 0.4));
    }
    g.player.inv.push(generate_unique(0x5EED_A11C, power)); // one ancient to gamble
}

/// Resolve a fishing attempt. `quality`: >=0 landed (0..1 skill), -1 escaped
/// (bait lost), <=-2 cancelled before a bite (no bait spent).
#[no_mangle]
pub extern "C" fn fish(quality: f32) {
    game().do_fish(quality);
}

/// Debug/testing: force fishing to be available regardless of terrain/threats
/// and clear nearby monsters so the mini-game can be exercised anywhere.
#[no_mangle]
pub extern "C" fn debug_fish() {
    let g = game();
    g.monsters.clear();
    g.player.ammo = g.player.ammo.max(FISH_BAIT);
    g.force_fish = true;
}

/// Make an offering: the IO buffer holds [count:u16][idx:u16]* of inventory
/// items to sacrifice. Reads them and applies a reward (or a boss).
#[no_mangle]
pub extern "C" fn offer() {
    let io = unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(IO) as *const u8, IO_CAP) };
    let count = u16::from_le_bytes([io[0], io[1]]) as usize;
    let mut idxs: Vec<usize> = Vec::with_capacity(count);
    for i in 0..count {
        let b = 2 + i * 2;
        if b + 1 >= IO_CAP {
            break;
        }
        idxs.push(u16::from_le_bytes([io[b], io[b + 1]]) as usize);
    }
    game().make_offering(&idxs);
}

/// The maximum number of inventory items (weapons) a player can hold.
#[no_mangle]
pub extern "C" fn inventory_cap() -> u32 {
    INVENTORY_CAP as u32
}

/// Forfeit the active arena (if any). Called by the Esc/menu path so opening the
/// menu during an event abandons it — cleanly, since it was consumed on entry.
#[no_mangle]
pub extern "C" fn abort_arena() {
    game().abort_active_arena();
}

/// Set the logical viewport height (px) so the view matches the device aspect
/// ratio. Width stays fixed. No-ops until a game exists.
#[no_mangle]
pub extern "C" fn set_view_h(h: f32) {
    let h = h.clamp(150.0, 1200.0);
    unsafe {
        if let Some(g) = (*core::ptr::addr_of_mut!(GAME)).as_mut() {
            g.view_h = h;
        }
    }
}

#[no_mangle]
pub extern "C" fn equip(inv_idx: u32, slot: u32) {
    let g = game();
    if slot < 4 && (inv_idx as usize) < g.player.inv.len() {
        g.player.equip[slot as usize] = inv_idx as i32;
        g.player.slot = slot as usize;
    }
}

/// Drop/trash an inventory item, fixing up equip references (their indices
/// shift when the Vec compacts).
#[no_mangle]
pub extern "C" fn drop_item(inv_idx: u32) {
    game().drop_item(inv_idx as usize);
}

/// Trash every inventory item weaker (lower dps) than the given one — a quick
/// "clear lower-level gear" action. Equipped weapons are always kept so you
/// don't accidentally destroy your active loadout (e.g. a melee fallback).
#[no_mangle]
pub extern "C" fn drop_below(inv_idx: u32) {
    game().drop_below(inv_idx as usize);
}

#[no_mangle]
pub extern "C" fn save_ptr() -> *const u8 {
    let g = game();
    g.build_save();
    g.save.as_ptr()
}

#[no_mangle]
pub extern "C" fn save_len() -> u32 {
    game().save.len() as u32
}

#[no_mangle]
pub extern "C" fn io_ptr() -> *mut u8 {
    core::ptr::addr_of_mut!(IO) as *mut u8
}

#[no_mangle]
pub extern "C" fn io_cap() -> u32 {
    IO_CAP as u32
}

#[no_mangle]
pub extern "C" fn load_save(len: u32) {
    let n = (len as usize).min(IO_CAP);
    let bytes = unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(IO) as *const u8, n) };
    // Ensure a game exists to load into.
    unsafe {
        if (*core::ptr::addr_of!(GAME)).is_none() {
            *core::ptr::addr_of_mut!(GAME) = Some(Game::new(0));
        }
    }
    game().load_save(bytes);
}

// ---------------------------------------------------------------------------
// Tests (host target: `cargo test`)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: a monster projectile landing the killing blow must not panic
    /// (previously death cleared `projectiles` mid-iteration → out-of-bounds
    /// `swap_remove` → wasm trap → frozen game).
    #[test]
    fn death_by_projectile_does_not_freeze() {
        let mut g = Game::new(12345);
        g.player.hp = 1.0;
        g.projectiles.push(Proj {
            x: g.player.x,
            y: g.player.y,
            vx: 0.0,
            vy: 0.0,
            life: 5.0,
            dmg: 9999.0,
            dmg_type: PHYS,
            special: 0,
            class_skill: 0,
            from_player: false,
            src_name: "Test Wisp".into(),
        });
        g.update(0.016); // must not panic
        assert!(g.player.hp > 0.0, "player should have respawned");
        assert!(g.projectiles.is_empty(), "projectiles cleared on respawn");
    }

    /// The respawn note names the monster and the attack that landed the fatal blow.
    #[test]
    fn respawn_names_the_killer() {
        let mut g = Game::new(1);
        g.player.hp = 1.0;
        g.damage_player(999.0, Hurt::Attack { name: "Frost Wisp", elem: COLD, ranged: false });
        assert!(g.player.hp <= 0.0, "the blow was fatal");
        g.respawn_if_dead();
        assert!(g.message.contains("Frost Wisp"), "names the killer: {}", g.message);
        assert!(g.message.contains("frozen strike"), "describes the blow: {}", g.message);
        assert!(g.last_kill.is_none(), "cause consumed on respawn");
    }

    /// Slaying a mega (Colossus) increments the boss-kill tally shown in the 100k stats.
    #[test]
    fn slaying_a_mega_counts_as_a_boss_kill() {
        let mut g = Game::new(1);
        g.monsters.clear();
        let mut m = monster::generate(2, 0, 0, g.player.x + 3.0, g.player.y, 1);
        make_mega(&mut m);
        m.hp = 0.1;
        m.def = 0.0;
        g.monsters.push(m);
        let mut sword = test_bow();
        sword.ranged = false;
        sword.damage = 50.0;
        sword.class_skill = SK_SWORD;
        g.hit_monster(0, &sword);
        assert!(g.monsters[0].hp <= 0.0, "the mega died");
        assert_eq!(g.mega_kills, 1, "boss kill counted");
        assert_eq!(g.kills, 1, "also a regular kill");
    }

    /// Regression: a melee cleave that kills several monsters in one swing must
    /// not panic (index shifting during removal).
    #[test]
    fn cleave_multi_kill_does_not_panic() {
        let mut g = Game::new(999);
        g.monsters.clear();
        for k in 0..5u64 {
            let mut m = monster::generate(k + 1, 0, 0, g.player.x + 3.0, g.player.y, 1);
            m.hp = 0.1;
            m.def = 0.0;
            g.monsters.push(m);
        }
        g.input.attack = true;
        g.input.aimx = 1.0;
        g.input.aimy = 0.0;
        g.player.atk_cd = 0.0;
        g.update(0.016); // must not panic
    }

    /// Spawn safety: the player never starts on an impassable tile, nor stranded
    /// on a passable-but-enclosed island they can't walk off of. Swept over many
    /// seeds so a "trapped in water surrounded by deep water" spawn can't slip in.
    #[test]
    fn spawn_is_on_passable_tile() {
        for seed in 0u64..600 {
            let g = Game::new(seed);
            let (px, py) = (g.player.x, g.player.y);
            assert!(
                world::passable(tile_px(g.seed, px, py)),
                "seed {seed} spawned on impassable tile",
            );
            let (tx, ty) = ((px / TILE).floor() as i64, (py / TILE).floor() as i64);
            assert!(
                open_enough(g.seed, tx, ty),
                "seed {seed} spawned on an enclosed island (trapped)",
            );
        }
    }

    /// A drop that would land on an impassable tile (a boss slain at the water's
    /// edge) is pulled onto passable ground so the reward is never stranded.
    #[test]
    fn loot_never_stranded_on_impassable_tile() {
        let g = Game::new(42);
        // An impassable tile that borders passable ground — the "next to it" case.
        let mut edge = None;
        'outer: for ty in -300..300i64 {
            for tx in -300..300i64 {
                if !world::passable(tile_at(g.seed, tx, ty))
                    && [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .any(|(ox, oy)| world::passable(tile_at(g.seed, tx + ox, ty + oy)))
                {
                    edge = Some((tx, ty));
                    break 'outer;
                }
            }
        }
        let (tx, ty) = edge.expect("world has an impassable tile bordering land");
        let (x, y) = (tx as f32 * TILE + TILE * 0.5, ty as f32 * TILE + TILE * 0.5);
        assert!(!passable_px(g.seed, x, y), "picked an impassable spot");
        let (dx, dy) = g.drop_pos(x, y);
        assert!(passable_px(g.seed, dx, dy), "drop_pos moved the loot onto passable ground");
    }

    /// Megas amplify elemental interaction and 1,000,000 stays finite/brutal.
    #[test]
    fn megas_punish_wrong_element_and_curve_is_finite() {
        use monster::elem_mult2;
        // Normal: 2x weak, 0.5x resist. Mega: 3x weak, 0.12x resist.
        assert!((elem_mult2(FIRE, COLD, FIRE, false) - 2.0).abs() < 1e-6);
        assert!((elem_mult2(COLD, COLD, FIRE, false) - 0.5).abs() < 1e-6);
        assert!((elem_mult2(FIRE, COLD, FIRE, true) - 3.0).abs() < 1e-6);
        assert!((elem_mult2(COLD, COLD, FIRE, true) - 0.12).abs() < 1e-6);

        // Difficulty rises but 1,000,000 tiles is a finite ~Lv1500 frontier.
        let d = |tiles: f32| difficulty_at(tiles * TILE, 0.0);
        assert_eq!(d(0.0), 1);
        assert!(d(1000.0) > d(100.0) && d(1000.0) < 60, "1k ~ Lv30, was Lv52");
        let m = d(1_000_000.0);
        assert!((1000..2500).contains(&m), "1,000,000 tiles landed at Lv {m}");
    }

    /// DPS-vs-HP scaling check: weapon damage and monster HP both scale ~linearly
    /// with distance, so the *skill-free* one-shot ratio stays bounded (no
    /// distance runaway). Skills add an overkill multiplier on top, but it's
    /// cosmetic — one-shot is one-shot — and megas resist a wrong element so they
    /// are never trivialised. Run with `--nocapture` to see the table.
    #[test]
    fn dps_vs_hp_scaling_stays_sane() {
        let golem_hp = |lvl: u32| (9.0 + 5.0 * lvl as f32) * 1.8; // tanky body
        let golem_def = |lvl: u32| 4.0 + 0.32 * lvl as f32;
        let mut worst_base = 0.0f32;
        for &tiles in &[1_000.0f32, 10_000.0, 30_000.0, 100_000.0] {
            let lvl = difficulty_at(tiles * TILE, 0.0);
            let w = generate_unique(0xABCD ^ tiles as u64, lvl as f32); // strong common gear
            let hp = golem_hp(lvl);
            // Skill-free, neutral element.
            let base = (w.damage - golem_def(lvl)).max(1.0);
            let r_base = base / hp;
            worst_base = worst_base.max(r_base);
            // A heavily trained build (class+elem ~Lv60 each) hitting a weakness.
            let bonus = 60.0 * 0.06 * 2.0; // two skills at level 60
            let trained_weak = ((w.damage * (1.0 + bonus) * 2.0) - golem_def(lvl)).max(1.0);
            eprintln!(
                "d={:>7} Lv{:>4}  golemHP={:>7.0}  uniqDmg={:>7.0}  base×={:>4.1}  trained+weak×={:>6.1}",
                tiles as u32, lvl, hp, w.damage, r_base, trained_weak / hp,
            );
        }
        // The skill-free ratio never runs away with distance (linear vs linear).
        assert!(worst_base < 8.0, "skill-free one-shot ratio should stay bounded, was {worst_base:.1}");
        // A mega resists a wrong element: it takes a tiny fraction, never one-shot.
        let lvl = difficulty_at(30_000.0 * TILE, 0.0);
        let mut mega = gen_monster(0x77, 0, 0, 30_000.0 * TILE, 0.0, lvl);
        make_mega(&mut mega);
        let wrong = elem_mult2(mega.resist, mega.resist, mega.weak, true); // hitting its resistance
        assert!(wrong <= 0.2, "wrong element barely dents a mega (mult {wrong})");
    }

    /// Determinism: the same seed regenerates identical terrain.
    #[test]
    fn terrain_is_deterministic() {
        for (tx, ty) in [(0, 0), (500, -320), (-1000, 1000)] {
            assert_eq!(tile_at(42, tx, ty), tile_at(42, tx, ty));
        }
        assert_eq!(difficulty_at(0.0, 0.0), 1, "origin should be danger Lv 1");
    }

    fn test_bow() -> Weapon {
        Weapon {
            seed: 0,
            power: 1.0,
            durability: 1.0,
            unique: false,
            base: 4,
            dmg_type: PHYS,
            damage: 5.0,
            cooldown: 0.5,
            range: 150.0,
            ranged: true,
            proj_speed: 150.0,
            rarity: 0,
            class_skill: SK_BOW,
            special: 0,
            name: "Test Bow".into(),
        }
    }

    /// Ranged attacks consume ammo and stop firing once the pool is empty.
    #[test]
    fn ranged_consumes_ammo_and_stops_when_empty() {
        let mut g = Game::new(7);
        g.player.inv.clear();
        g.player.inv.push(test_bow());
        g.player.equip = [0, -1, -1, -1];
        g.player.slot = 0;
        g.player.ammo = 2;
        g.input.attack = true;
        g.input.aimx = 1.0;
        g.input.aimy = 0.0;

        g.player.atk_cd = 0.0;
        g.player_attack(0.0);
        assert_eq!(g.player.ammo, 1);
        assert_eq!(g.projectiles.len(), 1);

        g.player.atk_cd = 0.0;
        g.player_attack(0.0);
        assert_eq!(g.player.ammo, 0);

        // Out of ammo: firing does nothing.
        g.player.atk_cd = 0.0;
        let before = g.projectiles.len();
        g.player_attack(0.0);
        assert_eq!(g.player.ammo, 0);
        assert_eq!(g.projectiles.len(), before, "no shot fired with empty ammo");
    }

    /// A fast (high-Move) player can't outrun their own arrows: the shot's speed
    /// is floored above the player's top move speed — the same guarantee
    /// monster_proj_speed gives enemies — and its RANGE is unchanged (the floor
    /// changes pace, not reach).
    #[test]
    fn player_shots_outrun_a_fast_player() {
        let mut g = Game::new(7);
        g.player.inv.clear();
        g.player.inv.push(test_bow());
        g.player.equip = [0, -1, -1, -1];
        g.player.slot = 0;
        g.player.ammo = 5;
        g.player.skills[SK_MOVE as usize] = 1_000_000.0; // Move maxed (cap applies)
        g.input.attack = true;
        g.input.aimx = 1.0;
        g.input.aimy = 0.0;
        g.player.atk_cd = 0.0;
        g.player_attack(0.0);

        assert_eq!(g.projectiles.len(), 1);
        let p = &g.projectiles[0];
        let shot = (p.vx * p.vx + p.vy * p.vy).sqrt();
        let player_top = BASE_SPEED * (1.0 + MOVE_BONUS_CAP); // the fastest a player can move
        assert!(shot > player_top, "arrow speed {shot} must exceed a maxed player's {player_top}");
        // range = speed * lifetime is preserved (the floor doesn't extend reach).
        let reach = shot * p.life;
        assert!((reach - test_bow().range).abs() < 1.0, "reach {reach} != weapon range {}", test_bow().range);
    }

    /// The save blob round-trips the persistent record, ammo, skills and items.
    #[test]
    fn save_roundtrips_record_and_ammo() {
        let mut g = Game::new(24680);
        g.player.max_dist = 137.5;
        g.player.ammo = 42;
        g.player.cp_x = 1234.0;
        g.player.cp_y = -567.0;
        g.player.skills[SK_BOW as usize] = 90.0;
        g.player.inv.push(test_bow());
        g.mega_kills = 7;
        g.looted_arenas.push((3, -4));
        g.build_save();
        let bytes = g.save.clone();

        let mut g2 = Game::new(0);
        g2.load_save(&bytes);
        assert_eq!(g2.seed, 24680);
        assert_eq!(g2.mega_kills, 7, "boss kills persisted");
        assert!(g2.looted_arenas.contains(&(3, -4)), "entered arenas persisted");
        assert!((g2.player.max_dist - 137.5).abs() < 0.01, "record persisted");
        assert_eq!(g2.player.ammo, 42);
        assert!((g2.player.cp_x - 1234.0).abs() < 0.01, "checkpoint persisted");
        assert!((g2.player.cp_y + 567.0).abs() < 0.01);
        assert!((g2.player.skills[SK_BOW as usize] - 90.0).abs() < 0.01);
        assert!(g2.player.inv.len() >= 2, "starter + saved weapon restored");
    }

    /// After banking a checkpoint, death respawns there rather than the origin.
    #[test]
    fn respawn_uses_last_checkpoint() {
        let mut g = Game::new(5);
        // Teleport past one checkpoint threshold and tick to bank it.
        g.player.x = (CHECKPOINT + 6.0) * TILE;
        g.player.y = 0.0;
        g.update(0.016);
        assert!(g.player.cp_x != 0.0, "checkpoint should be banked");

        // Die and respawn — should land near the checkpoint distance, not origin.
        g.player.hp = -1.0;
        g.respawn_if_dead();
        let d = dist_tiles(g.player.x, g.player.y);
        assert!(d > CHECKPOINT * 0.5, "respawned near checkpoint, not origin (d={d})");
    }

    /// Death costs half your ammo and 10% durability on equipped gear, and
    /// breaks gear that hits 0.
    #[test]
    fn death_wears_gear_and_halves_ammo() {
        let mut g = Game::new(7);
        g.player.ammo = 10;
        g.player.inv.clear();
        g.player.equip = [-1; 4];
        g.player.inv.push(test_bow()); // durability 1.0, equipped in slot 0
        g.player.equip[0] = 0;

        g.player.hp = -1.0;
        g.respawn_if_dead();
        assert_eq!(g.player.ammo, 5, "half ammo lost");
        assert!((g.player.inv[0].durability - 0.9).abs() < 1e-4, "10% durability lost");
        assert!(g.player.hp > 0.0, "respawned");

        // Wear it down to breaking (9 more deaths -> 0%).
        for _ in 0..9 {
            g.player.hp = -1.0;
            g.respawn_if_dead();
        }
        // The bow shattered, but the safety net refits a fresh basic sword so the
        // player is never left unable to fight.
        assert_eq!(g.player.inv.len(), 1, "refitted after the last weapon broke");
        assert_eq!(g.player.inv[0].base, 0, "the refit is a basic sword");
        assert!(g.player.weapon().is_some(), "an equipped weapon again");
        assert!(g.player.inv.iter().all(|w| w.name != "Test Bow"), "the broken bow is gone");
    }

    /// Enemy projectiles always travel faster than the enemy that fired them,
    /// so a chasing monster can't overtake (and appear to "drop") its own shots.
    #[test]
    fn enemy_projectiles_outrun_their_firer() {
        // Sweep speeds from the slowest Golem to a very fast high-level monster.
        for s in [10.0, 18.0, 30.0, 46.0, 62.0, 90.0, 140.0, 220.0_f32] {
            assert!(
                monster_proj_speed(s) > s,
                "proj speed {} must exceed firer speed {s}",
                monster_proj_speed(s),
            );
        }
        // Slow monsters still get a brisk floor rather than a crawling shot.
        assert!(monster_proj_speed(10.0) >= 62.0, "slow-monster shots stay brisk");
    }

    /// Regression: the safety-net refit must terminate at high distance. gen_weapon
    /// nudges rarity up with power, so past ~power 40 a common (rarity<=1) roll is
    /// impossible — the bounded search must still return a Sword instead of looping
    /// forever (this test would hang on the old unbounded loop).
    #[test]
    fn refit_terminates_at_high_power() {
        let mut g = Game::new(5430);
        g.player.inv.clear();
        g.player.equip = [-1; 4];
        g.grant_basic_sword(200.0); // power far above the common-rarity ceiling
        assert!(g.player.weapon().is_some(), "refit equipped a weapon");
        assert_eq!(g.player.weapon().unwrap().base, 0, "the refit is a Sword");
    }

    /// After a respawn shatters the active weapon, re-sending the (now empty)
    /// slot must not leave the player weaponless — selection falls back to a
    /// slot that still holds a weapon (mobile can only re-send its held slot).
    #[test]
    fn selecting_an_empty_slot_falls_back_to_a_weapon() {
        let mut g = Game::new(3);
        g.player.inv.clear();
        g.player.inv.push(test_bow()); // the surviving weapon, lands in slot 2
        g.player.equip = [-1, -1, 0, -1];
        g.player.slot = 1; // was holding slot 1, whose weapon just broke (empty)

        // The client keeps sending its selected (empty) slot 1.
        g.player.select_slot(1);
        assert_eq!(g.player.slot, 2, "snapped to the slot that still has a weapon");
        assert!(g.player.weapon().is_some(), "an active weapon to fight with");

        // A valid selection is honoured, and an empty request afterwards is ignored.
        g.player.inv.push(test_bow());
        g.player.equip[0] = 1; // slot 0 now also has a weapon
        g.player.select_slot(0);
        assert_eq!(g.player.slot, 0, "explicit valid selection honoured");
        g.player.select_slot(3); // empty slot
        assert_eq!(g.player.slot, 0, "empty request ignored while current is valid");
    }

    /// "Delete below" trashes weaker gear but preserves stronger and equipped.
    #[test]
    fn drop_below_keeps_stronger_and_equipped() {
        let mut g = Game::new(11);
        g.player.inv.clear();
        g.player.equip = [-1; 4];
        let mk = |dmg: f32, name: &str| Weapon {
            seed: 0,
            power: 1.0,
            durability: 1.0,
            unique: false,
            base: 0,
            dmg_type: PHYS,
            damage: dmg,
            cooldown: 1.0, // dps == damage
            range: 20.0,
            ranged: false,
            proj_speed: 0.0,
            rarity: 0,
            class_skill: SK_SWORD,
            special: 0,
            name: name.into(),
        };
        g.player.inv.push(mk(10.0, "A")); // idx0 dps10
        g.player.inv.push(mk(30.0, "B")); // idx1 dps30
        g.player.inv.push(mk(5.0, "C")); // idx2 dps5 (equipped fallback)
        g.player.inv.push(mk(20.0, "D")); // idx3 dps20
        let mut u = mk(2.0, "U"); // idx4 dps2 but UNIQUE — must be protected
        u.unique = true;
        g.player.inv.push(u);
        g.player.equip[1] = 2; // equip weak "C"

        g.drop_below(3); // threshold = dps 20 -> keep B,D + equipped C + unique U

        let names: Vec<&str> = g.player.inv.iter().map(|w| w.name.as_str()).collect();
        assert!(names.contains(&"B") && names.contains(&"D"), "kept stronger");
        assert!(names.contains(&"C"), "kept equipped even though weaker");
        assert!(names.contains(&"U"), "kept unique even though weaker");
        assert!(!names.contains(&"A"), "dropped the weaker unequipped item");
        let ci = g.player.equip[1];
        assert!(ci >= 0 && g.player.inv[ci as usize].name == "C", "equip index remapped");

        // drop_item removes one and fixes equip references.
        let before = g.player.inv.len();
        let eq_name = g.player.inv[g.player.equip[1] as usize].name.clone();
        g.drop_item(0); // drop "B" (index 0 after the drop_below compaction)
        assert_eq!(g.player.inv.len(), before - 1, "drop_item removes one");
        let e = g.player.equip[1];
        assert!(e >= 0 && g.player.inv[e as usize].name == eq_name, "equip still points at same weapon");
    }

    /// Reaching CELEBRATE_DIST (100,000) fires the flash mob once; it ends after
    /// CELEBRATE_DUR and the wilds (a crowd of dancers) are present.
    #[test]
    fn reaching_celebrate_dist_triggers_celebration() {
        let mut g = Game::new(1);
        g.player.x = CELEBRATE_DIST * TILE;
        g.player.max_dist = CELEBRATE_DIST;
        g.advance(0.016);
        assert!(g.celebrating, "celebration starts at 100,000");
        assert!(g.celebrated, "flagged so it only ever fires once");
        assert!(g.monsters.iter().any(|m| m.mega), "bosses join the party");
        assert!(g.monsters.len() >= 10, "a crowd gathers");
        // Fast-forward past the music.
        for _ in 0..(CELEBRATE_DUR as i32 * 70) {
            g.advance(0.016);
        }
        assert!(!g.celebrating, "celebration ends and normal rules resume");
    }

    /// Base-10 mini-milestones fire once each; loading a high record doesn't
    /// re-toast the ones already passed.
    #[test]
    fn milestones_fire_once_per_power_of_ten() {
        let mut g = Game::new(2);
        g.player.x = 100.0 * TILE; // ~100 tiles from origin
        g.player.max_dist = 100.0;
        g.check_milestones();
        assert!(g.milestone_t > 0.0, "crossing 100 pops a toast");
        assert!(g.message.contains("Exploring"), "label matches the tier");
        let mask = g.milestone_mask;
        g.check_milestones(); // no new milestone -> no change
        assert_eq!(g.milestone_mask, mask, "each milestone fires only once");

        // A freshly-loaded high record has its past milestones pre-marked.
        let mut g2 = Game::new(2);
        g2.player.max_dist = 5000.0;
        g2.seed_milestones();
        g2.milestone_t = 0.0;
        g2.check_milestones();
        assert_eq!(g2.milestone_t, 0.0, "no re-toasting of already-passed milestones");
    }

    /// The 1,000 / 10,000 / 50,000 milestones shower the current view with
    /// fountains / ammo / chests on passable, in-window tiles.
    #[test]
    fn milestone_showers_scatter_view_loot() {
        // 1,000 → healing fountains.
        let mut g = Game::new(7);
        let (sx, sy) = safe_spawn(g.seed, 1_000.0 * TILE, 0.0);
        g.player.x = sx;
        g.player.y = sy;
        g.player.max_dist = 1_000.0;
        g.view_h = 600.0; // a tall (phone) view so there's plenty of ground
        g.milestone_mask = 0b111; // 1/10/100 already passed → only 1,000 fires
        g.check_milestones();
        let fountains = g.loot.iter().filter(|l| matches!(l.kind, Drop::Fountain)).count();
        assert!(fountains >= 8, "many springs scattered, got {fountains}");
        // All land on passable ground.
        assert!(
            g.loot.iter().all(|l| world::passable(tile_px(g.seed, l.x, l.y))),
            "every drop is on a walkable tile",
        );

        // 10,000 → the ammo total is split into stacks that sum back to 10,000.
        let mut g = Game::new(7);
        let (sx, sy) = safe_spawn(g.seed, 10_000.0 * TILE, 0.0);
        g.player.x = sx;
        g.player.y = sy;
        g.player.max_dist = 10_000.0;
        g.view_h = 600.0;
        g.milestone_mask = 0b1111;
        g.check_milestones();
        let ammo: u32 = g
            .loot
            .iter()
            .filter_map(|l| match l.kind {
                Drop::Ammo(n) => Some(n),
                _ => None,
            })
            .sum();
        assert_eq!(ammo, 10_000, "the whole pile is exactly 10,000 ammo");
        assert!(g.loot.len() >= 20, "spread across many stacks, got {}", g.loot.len());

        // 50,000 → a trove of chests (more than the 60-item cap, on purpose).
        let mut g = Game::new(7);
        let (sx, sy) = safe_spawn(g.seed, 50_000.0 * TILE, 0.0);
        g.player.x = sx;
        g.player.y = sy;
        g.player.max_dist = 50_000.0;
        g.view_h = 600.0;
        g.milestone_mask = 0b11111 | MILESTONE_25K; // only the 50k trove remains
        g.check_milestones();
        let chests = g.loot.iter().filter(|l| matches!(l.kind, Drop::Chest { .. })).count();
        assert!(chests > 60, "more riches than you can carry, got {chests}");
        // It fires only once — a second check adds nothing.
        let n = g.loot.len();
        g.check_milestones();
        assert_eq!(g.loot.len(), n, "the 50,000 trove doesn't re-shower");

        // 25,000 → a field of (non-additive) shield wards.
        let mut g = Game::new(7);
        let (sx, sy) = safe_spawn(g.seed, 25_000.0 * TILE, 0.0);
        g.player.x = sx;
        g.player.y = sy;
        g.player.max_dist = 25_000.0;
        g.view_h = 600.0;
        g.milestone_mask = 0b11111; // only the 25k field remains
        g.check_milestones();
        let wards = g.loot.iter().filter(|l| matches!(l.kind, Drop::Shield { .. })).count();
        assert!(wards >= 8, "a field of shield wards, got {wards}");

        // 75,000 → a field of teleporters, none within 3 tiles of the player.
        let mut g = Game::new(7);
        let (sx, sy) = safe_spawn(g.seed, 75_000.0 * TILE, 0.0);
        g.player.x = sx;
        g.player.y = sy;
        g.player.max_dist = 75_000.0;
        g.view_h = 600.0;
        g.milestone_mask = 0b11111 | MILESTONE_25K | MILESTONE_50K; // only the 75k field remains
        g.check_milestones();
        assert!(g.rifts.len() >= 6, "a field of rifts, got {}", g.rifts.len());
        assert!(
            g.rifts.iter().all(|rf| ((rf.x - g.player.x).powi(2) + (rf.y - g.player.y).powi(2)).sqrt() >= TILE * 3.0),
            "no rift spawns close enough to teleport the player instantly",
        );
    }

    /// Build a game with the player standing on an Idle arena ring on open ground.
    #[cfg(test)]
    fn arena_fixture(seed: u64, at: f32, waves: u8) -> (Game, i64, i64) {
        let mut g = Game::new(seed);
        let (sx, sy) = safe_spawn(g.seed, at * TILE, 0.0);
        g.player.x = sx;
        g.player.y = sy;
        g.monsters.clear();
        g.loot.clear();
        let (cx, cy) = ((sx / CHUNK_PX).floor() as i64, (sy / CHUNK_PX).floor() as i64);
        g.arenas.push(Arena {
            x: sx,
            y: sy,
            cx,
            cy,
            seed: hash2(seed ^ 0x5EED_A5EE, cx, cy),
            state: ArenaState::Idle,
            wave: 0,
            waves,
            countdown: 0.0,
        });
        (g, cx, cy)
    }

    /// Test helper: fast-forward the ready-steady-go countdown so the pending
    /// wave spawns this tick.
    #[cfg(test)]
    fn force_wave(g: &mut Game) {
        g.arenas[0].countdown = 0.001;
        g.update_arenas(0.01);
    }

    /// Stepping into the ring consumes it (no reload retry) and starts the waves.
    #[test]
    fn arena_activates_and_spawns_a_wave() {
        let (mut g, cx, cy) = arena_fixture(100, 400.0, 2);
        g.update_arenas(0.016); // enter (begins the countdown)
        assert_eq!(g.arenas[0].state, ArenaState::Active);
        assert!(g.looted_arenas.contains(&(cx, cy)), "consumed on entry");
        assert!(g.arenas[0].countdown > 0.0, "ready-steady-go before wave 1");
        force_wave(&mut g); // finish the countdown -> spawn wave 1
        assert_eq!(g.arenas[0].wave, 1);
        assert!(g.monsters.iter().any(|m| m.from_arena), "a wave spawned");
    }

    /// Arenas never spawn "connected": no two rings overlap, so you can't be
    /// force-chained from one straight into the next.
    #[test]
    fn arenas_never_spawn_connected() {
        let mut g = Game::new(2024);
        // Spawn a wide block of chunks out where arenas roll (diff >= 2). Calling
        // spawn_chunk directly accumulates arenas (no despawn), so the separation
        // guard is exercised against every arena already placed.
        for cx in 20..60 {
            for cy in 0..40 {
                g.spawn_chunk(cx, cy);
            }
        }
        let sites: Vec<(f32, f32)> = g.arenas.iter().map(|a| (a.x, a.y)).collect();
        assert!(sites.len() >= 3, "the sample should produce several arenas (got {})", sites.len());
        for i in 0..sites.len() {
            for j in (i + 1)..sites.len() {
                let d = ((sites[i].0 - sites[j].0).powi(2) + (sites[i].1 - sites[j].1).powi(2)).sqrt();
                assert!(d >= ARENA_MIN_SEP, "two arenas spawned connected (d={d} < {ARENA_MIN_SEP})");
            }
        }
    }

    /// An arena is warded ground against a cursed-relic hunt: the ring holds the
    /// curse at bay (no hunters accrue while you fight the waves), but the hunt
    /// resumes the instant you leave, and the sprint's step quota still ticks.
    #[test]
    fn an_arena_wards_off_the_curse_then_the_hunt_resumes() {
        let (mut g, _cx, _cy) = arena_fixture(555, 400.0, 3);
        g.begin_relic(3.0);
        // A pack is already chasing when you dive into the ring...
        for _ in 0..8 {
            g.spawn_hunter();
        }
        g.update_arenas(0.016); // enter -> Active, entry clear wipes the pack
        assert_eq!(g.arenas[0].state, ArenaState::Active);
        assert_eq!(g.monsters.iter().filter(|m| m.hunter).count(), 0, "entry sheds the pack");

        // Fight through many ticks; the ready timer would normally summon hunters,
        // but warded ground keeps the count at zero the whole time.
        for _ in 0..600 {
            g.update_relic(0.05); // ~30s of curse ticks
            g.update_arenas(0.016);
            assert_eq!(
                g.monsters.iter().filter(|m| m.hunter).count(),
                0,
                "no hunter reaches you inside the warded ring"
            );
            if g.relic.is_none() {
                break; // step quota may complete mid-arena — that's allowed
            }
        }

        // Leave the ring (forfeit) with the curse still active, then let it tick:
        // the hunt picks back up.
        if g.relic.is_none() {
            g.begin_relic(3.0); // re-arm if the quota finished, to check resume
        }
        g.abort_active_arena();
        assert_ne!(g.arenas[0].state, ArenaState::Active, "left the ring");
        let mut resumed = false;
        for _ in 0..60 {
            g.update_relic(0.05);
            if g.monsters.iter().any(|m| m.hunter) {
                resumed = true;
                break;
            }
        }
        assert!(resumed, "the hunt resumes once you leave the warded ring");
    }

    /// Surviving every wave pays the cache, remembers the chunk, and doesn't
    /// respawn an arena there.
    #[test]
    fn surviving_all_waves_pays_the_cache_and_retires() {
        let (mut g, cx, cy) = arena_fixture(101, 500.0, 2);
        g.update_arenas(0.016); // enter
        for _ in 0..10 {
            if g.arenas[0].state == ArenaState::Cleared {
                break;
            }
            force_wave(&mut g); // finish countdown -> spawn the wave
            g.monsters.retain(|m| !m.from_arena); // "kill" it
            g.update_arenas(0.016); // -> clear, or start the next countdown
        }
        assert_eq!(g.arenas[0].state, ArenaState::Cleared, "arena conquered");
        assert!(
            g.loot.iter().any(|l| matches!(l.kind, Drop::Chest { .. })),
            "cleared arena drops a chest cache",
        );
        assert!(g.looted_arenas.contains(&(cx, cy)), "chunk remembered");
        g.spawn_chunk(cx, cy);
        assert_eq!(
            g.arenas.iter().filter(|a| a.state == ArenaState::Idle).count(),
            0,
            "a spent chunk spawns no new arena",
        );
    }

    /// Leaving the ring forfeits: monsters cleared, ring retired, no reward.
    #[test]
    fn leaving_the_ring_forfeits_with_no_reward() {
        let (mut g, _cx, _cy) = arena_fixture(102, 600.0, 3);
        let ring_x = g.arenas[0].x;
        g.update_arenas(0.016); // enter
        force_wave(&mut g); // wave 1
        assert!(g.monsters.iter().any(|m| m.from_arena));
        g.player.x = ring_x + ARENA_OUTER + 40.0; // walk clear out of the apron
        g.update_arenas(0.016);
        assert_eq!(g.arenas[0].state, ArenaState::Done);
        assert!(!g.monsters.iter().any(|m| m.from_arena), "arena foes cleared on forfeit");
        assert!(
            !g.loot.iter().any(|l| matches!(l.kind, Drop::Chest { .. })),
            "a forfeit pays nothing",
        );
    }

    /// The menu/Esc forfeit path (abort_active_arena) does the same as walking out.
    #[test]
    fn menu_abort_forfeits_the_active_arena() {
        let (mut g, _cx, _cy) = arena_fixture(103, 700.0, 3);
        g.update_arenas(0.016); // enter
        force_wave(&mut g); // wave 1
        assert!(g.monsters.iter().any(|m| m.from_arena));
        g.abort_active_arena();
        assert_eq!(g.arenas[0].state, ArenaState::Done);
        assert!(!g.monsters.iter().any(|m| m.from_arena));
    }

    /// Entering despawns loitering ambient mobs, and arena foes are kept inside
    /// the ring (so a fleeing/ranged foe can't force the player to leave).
    #[test]
    fn arena_seals_the_ring_and_despawns_ambient() {
        let (mut g, _cx, _cy) = arena_fixture(200, 400.0, 2);
        // Ambient mobs loitering nearby (and a stray projectile).
        for k in 0..4u64 {
            let mut m = monster::generate(k + 1, 0, 0, g.player.x + 20.0, g.player.y, 5);
            m.from_arena = false;
            g.monsters.push(m);
        }
        g.update_arenas(0.016); // enter -> clears the stage
        assert!(
            !g.monsters.iter().any(|m| !m.from_arena),
            "ambient mobs despawn at arena start",
        );
        force_wave(&mut g); // wave 1 spawns
        assert!(g.monsters.iter().any(|m| m.from_arena), "a wave spawned");

        // Shove a wave monster far outside; the next tick must pull it back in.
        let (ax, ay) = (g.arenas[0].x, g.arenas[0].y);
        if let Some(m) = g.monsters.iter_mut().find(|m| m.from_arena) {
            m.x = ax + ARENA_INNER * 5.0;
            m.y = ay + ARENA_INNER * 3.0;
        }
        g.update_arenas(0.016);
        for m in g.monsters.iter().filter(|m| m.from_arena) {
            let d = ((m.x - ax).powi(2) + (m.y - ay).powi(2)).sqrt();
            assert!(d <= ARENA_INNER, "arena foe confined to the inner ring (d={d:.0})");
        }
    }

    /// A deep arena's final wave is a Colossus finale (a focused boss + few
    /// minions), and clearing it pays an extra chest as a bounty.
    #[test]
    fn final_wave_is_a_boss_finale_with_a_bounty() {
        let (mut g, _cx, _cy) = arena_fixture(600, 5000.0, 2); // deep -> boss finale
        g.update_arenas(0.016); // enter
        force_wave(&mut g); // wave 1 (normal)
        assert!(!g.monsters.iter().any(|m| m.mega), "wave 1 isn't the boss");
        g.monsters.retain(|m| !m.from_arena); // clear wave 1
        g.update_arenas(0.016); // -> countdown for the final wave
        force_wave(&mut g); // final wave (boss)
        let megas = g.monsters.iter().filter(|m| m.mega).count();
        let total = g.monsters.iter().filter(|m| m.from_arena).count();
        assert_eq!(megas, 1, "final wave caps with a Colossus");
        assert!(total <= 4, "boss wave is focused, not a swarm (got {total})");
        // Clear the boss -> a cache with the boss bounty (an extra chest).
        g.monsters.retain(|m| !m.from_arena);
        g.update_arenas(0.016);
        assert_eq!(g.arenas[0].state, ArenaState::Cleared);
        let chests = g.loot.iter().filter(|l| matches!(l.kind, Drop::Chest { .. })).count();
        assert!(chests >= 3, "the bounty adds an extra chest (got {chests})");
    }

    /// A low-tier arena near the origin has no boss finale (gentle intro).
    #[test]
    fn shallow_arena_has_no_boss() {
        let (mut g, _cx, _cy) = arena_fixture(700, 40.0, 2); // shallow -> no boss
        assert!(difficulty_at(g.arenas[0].x, g.arenas[0].y) < ARENA_BOSS_TIER);
        g.update_arenas(0.016); // enter
        for _ in 0..6 {
            force_wave(&mut g);
            assert!(!g.monsters.iter().any(|m| m.mega), "no Colossus in a shallow arena");
            g.monsters.retain(|m| !m.from_arena);
            g.update_arenas(0.016);
            if g.arenas[0].state == ArenaState::Cleared {
                break;
            }
        }
        assert_eq!(g.arenas[0].state, ArenaState::Cleared);
    }

    /// You can approach an idle ring (telegraph range) without committing; only
    /// stepping into the inner ring enters.
    #[test]
    fn approaching_an_idle_ring_does_not_commit() {
        let (mut g, _cx, _cy) = arena_fixture(500, 400.0, 2);
        let (ax, ay) = (g.arenas[0].x, g.arenas[0].y);
        // Stand in the telegraph zone but outside the inner ring.
        g.player.x = ax + (ARENA_INNER + ARENA_TELEGRAPH) * 0.5;
        g.player.y = ay;
        g.update_arenas(0.016);
        assert_eq!(g.arenas[0].state, ArenaState::Idle, "not committed until you step inside");
        // Step into the inner ring — now it commits.
        g.player.x = ax;
        g.update_arenas(0.016);
        assert_eq!(g.arenas[0].state, ArenaState::Active, "stepping inside enters");
    }

    /// The apron (between the rings) drains health but doesn't forfeit; the inner
    /// ring is safe.
    #[test]
    fn arena_apron_rots_the_player() {
        let (mut g, _cx, _cy) = arena_fixture(300, 400.0, 3);
        g.update_arenas(0.016); // enter -> Active
        let (ax, ay) = (g.arenas[0].x, g.arenas[0].y);
        g.player.x = ax + (ARENA_INNER + ARENA_OUTER) * 0.5; // stand in the apron
        g.player.y = ay;
        let hp0 = g.player.hp;
        for _ in 0..30 {
            g.update_arenas(0.05);
        }
        assert!(g.player.hp < hp0, "the apron rots your health");
        assert_eq!(g.arenas[0].state, ArenaState::Active, "but standing in the apron isn't a forfeit");
        // Back inside the inner ring — no more drain.
        g.player.x = ax;
        let hp1 = g.player.hp;
        for _ in 0..20 {
            g.update_arenas(0.05);
        }
        assert!((g.player.hp - hp1).abs() < 0.001, "the inner ring is safe from rot");
    }

    /// Dying mid-arena forfeits it (it was consumed on entry) — no free cache.
    #[test]
    fn dying_in_the_arena_grants_no_cache() {
        let (mut g, _cx, _cy) = arena_fixture(104, 800.0, 3);
        g.update_arenas(0.016); // enter
        force_wave(&mut g); // wave 1
        g.player.hp = -1.0;
        g.respawn_if_dead();
        assert_eq!(g.arenas[0].state, ArenaState::Done, "death retires the ring");
        g.loot.clear();
        g.update_arenas(0.016);
        assert!(
            !g.loot.iter().any(|l| matches!(l.kind, Drop::Chest { .. })),
            "no cache is paid after a death",
        );
    }

    /// The cursed relic activates, ends after its step quota, and clears hunters.
    #[test]
    fn relic_activates_and_ends_at_the_step_quota() {
        let mut g = Game::new(1);
        g.begin_relic(5.0);
        assert!(g.relic.is_some(), "relic active on pickup");
        g.spawn_hunter();
        assert!(g.monsters.iter().any(|m| m.hunter), "a hunter spawned");
        g.relic.as_mut().unwrap().steps = RELIC_STEPS; // reach the quota
        g.update_relic(0.016);
        assert!(g.relic.is_none(), "the curse lifts at the quota");
        assert!(!g.monsters.iter().any(|m| m.hunter), "hunters cleared when it ends");
    }

    /// The blue shield soaks damage before it reaches health.
    #[test]
    fn relic_shield_absorbs_damage_before_health() {
        let mut g = Game::new(2);
        g.player.maxhp = 100.0;
        g.player.hp = 100.0;
        g.begin_relic(1.0);
        let hp0 = g.player.hp;
        let shield0 = g.relic.as_ref().unwrap().shield;
        g.damage_player(10.0, Hurt::Swamp);
        assert!((g.player.hp - hp0).abs() < 0.01, "shield soaked the hit — hp unchanged");
        assert!(g.relic.as_ref().unwrap().shield < shield0, "shield was spent");
    }

    /// A shield-shrine ward soaks damage before health, then health takes the rest
    /// once it's drained — and it never refills on its own.
    #[test]
    fn shield_shrine_ward_soaks_then_drains() {
        let mut g = Game::new(9);
        g.player.maxhp = 100.0;
        g.player.hp = 100.0;
        // Pick up a shield-shrine ward on the spot.
        g.loot.push(Loot { x: g.player.x, y: g.player.y, kind: Drop::Shield { amount: 30.0 } });
        g.pickups();
        assert!((g.player.shield - 30.0).abs() < 0.01, "ward granted");
        // A hit smaller than the ward: hp untouched, ward spent.
        g.godmode = false;
        g.damage_player(20.0, Hurt::Swamp);
        assert!((g.player.hp - 100.0).abs() < 0.01, "ward soaked the hit");
        assert!((g.player.shield - 10.0).abs() < 0.01, "ward drained by the damage");
        // A hit bigger than the remaining ward: ward empties, hp takes the overflow.
        g.damage_player(30.0, Hurt::Swamp);
        assert!(g.player.shield <= 0.01, "ward fully drained");
        assert!(g.player.hp < 100.0, "overflow hit health");
        // It does not recharge over time.
        let s = g.player.shield;
        for _ in 0..30 { g.update(0.05); }
        assert!(g.player.shield <= s + 0.01, "ward never refills on its own");
    }

    /// A champion duel clears ambient adds for a fair 1v1, and felling it drops a
    /// prize and marks the chunk spent (no reload farm).
    #[test]
    fn champion_duel_clears_adds_and_drops_a_prize() {
        let mut g = Game::new(555);
        let (cx, cy) = g.player_chunk();
        let d = 5;
        let mut champ = gen_monster(monster_seed(g.seed, cx, cy, 777), cx, cy, g.player.x + 40.0, g.player.y, d);
        make_champion(&mut champ);
        g.monsters.push(champ);
        for k in 0..3u32 {
            g.monsters.push(gen_monster(monster_seed(g.seed, cx, cy, 30 + k), cx, cy, g.player.x + 30.0, g.player.y + 20.0, d));
        }
        g.enforce_duels();
        assert_eq!(g.monsters.iter().filter(|m| !m.champion).count(), 0, "adds cleared for a 1v1");
        assert!(g.monsters.iter().any(|m| m.champion), "the champion stays");
        // Land the killing blow with a strong weapon.
        let idx = g.monsters.iter().position(|m| m.champion).unwrap();
        g.monsters[idx].hp = 1.0;
        g.monsters[idx].def = 0.0;
        g.loot.clear();
        let w = generate_unique(0xC0FFEE, 50.0);
        g.hit_monster(idx, &w);
        assert!(g.monsters[idx].hp <= 0.0, "champion slain");
        assert!(g.loot.iter().any(|l| matches!(l.kind, Drop::Chest { .. })), "prize chest dropped");
        assert!(g.looted_champions.contains(&(cx, cy)), "champion chunk marked spent");
    }

    /// Stepping into a rift leaps you a long way outward and greets you with
    /// something, and the rift is spent.
    #[test]
    fn rift_leaps_forward_and_greets_you() {
        let mut g = Game::new(4242);
        g.player.x = 800.0 * TILE; // well out from origin so "outward" is defined
        g.player.y = 0.0;
        let d0 = dist_tiles(g.player.x, g.player.y);
        let (cx, cy) = g.player_chunk();
        g.rifts.push(Rift { x: g.player.x, y: g.player.y, cx, cy });
        g.monsters.clear();
        g.check_rifts();
        let d1 = dist_tiles(g.player.x, g.player.y);
        assert!(d1 > d0 + 300.0, "the rift threw you far forward ({d0} -> {d1})");
        assert!(!g.monsters.is_empty(), "something waits at the landing");
        assert!(g.looted_rifts.contains(&(cx, cy)), "the rift is spent (no re-leap)");
        // Nothing lands point-blank: the greeting (and any natural spawn) sits
        // outside the safety bubble, so you get a beat before the fight.
        let (lx, ly) = (g.player.x, g.player.y);
        for m in &g.monsters {
            let d = ((m.x - lx).powi(2) + (m.y - ly).powi(2)).sqrt();
            assert!(d > RIFT_LANDING_SAFE, "a monster spawned point-blank at the landing (d={d})");
        }
    }

    /// A cursed-relic hunt can't be outrun via a rift: some hunters tear through
    /// after you (re-summoned near the landing), while the rest of the pack is
    /// thinned. No pursuer lands point-blank.
    #[test]
    fn rift_during_a_curse_carries_the_hunt() {
        let mut g = Game::new(77);
        g.player.x = 800.0 * TILE;
        g.player.y = 0.0;
        g.begin_relic(3.0);
        // Build up a full pack of pursuers well above the carry cap.
        g.monsters.clear();
        for _ in 0..20 {
            g.spawn_hunter();
        }
        let before = g.monsters.iter().filter(|m| m.hunter).count();
        assert!(before > RIFT_HUNTER_CARRY, "start with more hunters than the rift carries");

        let (cx, cy) = g.player_chunk();
        g.rifts.push(Rift { x: g.player.x, y: g.player.y, cx, cy });
        g.check_rifts();

        let after = g.monsters.iter().filter(|m| m.hunter).count();
        assert!(after >= 1, "the hunt follows you through the rift");
        assert!(after <= RIFT_HUNTER_CARRY, "the rift thins the pack to at most the carry cap (got {after})");
        assert!(g.relic.is_some(), "the curse itself persists through the rift");
        // Carried hunters arrive off-screen, not on top of you.
        let (lx, ly) = (g.player.x, g.player.y);
        for m in g.monsters.iter().filter(|m| m.hunter) {
            let d = ((m.x - lx).powi(2) + (m.y - ly).powi(2)).sqrt();
            assert!(d > RIFT_LANDING_SAFE, "a hunter followed through point-blank (d={d})");
        }
    }

    /// Hunters cap out and ignore chunk despawn (they hunt until killed).
    #[test]
    fn hunters_are_capped_and_persist_across_chunks() {
        let mut g = Game::new(3);
        g.begin_relic(3.0);
        for _ in 0..90 {
            g.relic.as_mut().unwrap().hunt_cd = 0.0;
            g.update_relic(0.016);
        }
        let hunters = g.monsters.iter().filter(|m| m.hunter).count();
        assert!(hunters <= RELIC_HUNTER_CAP, "growth is capped (got {hunters})");
        assert!(hunters >= 40, "hunters do accumulate (got {hunters})");
        // Walk far away: regular monsters would despawn, hunters don't.
        g.player.x = 100_000.0;
        g.player.y = 100_000.0;
        g.stream_chunks();
        let after = g.monsters.iter().filter(|m| m.hunter).count();
        assert_eq!(after, hunters, "hunters ignore chunk despawn");
    }

    /// Hunters are melee-only and their speed is gated by the player's terrain:
    /// outrun-able on grass/dirt, faster than you on anything rougher.
    #[test]
    fn hunters_are_melee_and_gated_by_terrain() {
        let mut g = Game::new(9);
        g.begin_relic(5.0);
        g.spawn_hunter();
        let h = g.monsters.iter().find(|m| m.hunter).expect("a hunter spawned");
        assert!(!h.ranged, "hunters are melee only");
        let mv = skill_bonus(g.player.skills[SK_MOVE as usize]).min(MOVE_BONUS_CAP);
        let open = BASE_SPEED * (1.0 + mv) * RELIC_SPEED_MULT; // player on grass/dirt (cost 1.0)
        let hunter = open / RELIC_HUNTER_MOVE_COST;
        assert!(hunter < open, "on grass/dirt the player outruns the hunters");
        assert!(hunter > open / 1.12, "off grass/dirt (sand cost 1.12) the hunters gain");
    }

    /// A hunter left far behind surges to close about half the gap, so it's never
    /// lost for good on long open stretches.
    #[test]
    fn far_hunters_surge_to_catch_up() {
        let mut g = Game::new(10);
        g.begin_relic(3.0);
        g.spawn_hunter();
        let (px, py) = (g.player.x, g.player.y);
        let hi = g.monsters.iter().position(|m| m.hunter).unwrap();
        g.monsters[hi].x = px + RELIC_HUNTER_FAR + 200.0; // way behind on open ground
        g.monsters[hi].y = py;
        let d0 = (g.monsters[hi].x - px).abs();
        for _ in 0..30 {
            g.update_monsters(0.05); // ~1.5s
        }
        let hi = g.monsters.iter().position(|m| m.hunter).unwrap();
        let d1 = (g.monsters[hi].x - px).abs();
        assert!(d1 < d0 - RELIC_HUNTER_FAR * 0.3, "the hunter surged closer (from {d0:.0} to {d1:.0})");
    }

    /// Any crowd of monsters spreads out instead of stacking on one spot.
    #[test]
    fn monsters_do_not_stack() {
        let mut g = Game::new(11);
        g.monsters.clear();
        let (px, py) = (g.player.x, g.player.y); // player spawn is open ground
        for k in 0..6u64 {
            let mut m = monster::generate(k + 1, 0, 0, px, py, 5);
            m.x = px + (k as f32) * 0.4 - 1.0; // near-coincident (overlapping)
            m.y = py + (k as f32) * 0.3 - 0.75;
            g.monsters.push(m);
        }
        for _ in 0..30 {
            g.separate_monsters(g.seed);
        }
        for a in 0..g.monsters.len() {
            for b in (a + 1)..g.monsters.len() {
                let d = ((g.monsters[a].x - g.monsters[b].x).powi(2)
                    + (g.monsters[a].y - g.monsters[b].y).powi(2))
                .sqrt();
                let min_d = g.monsters[a].radius + g.monsters[b].radius;
                assert!(d >= min_d - 0.6, "monsters separated (d={d:.1}, need {min_d:.1})");
            }
        }
    }

    /// Campfire: ambush chance is zero to 50% HP, then scales with how high you heal.
    #[test]
    fn campfire_ambush_chance_scales_above_half() {
        assert_eq!(campfire_ambush_chance(50.0, 100.0), 0.0);
        assert_eq!(campfire_ambush_chance(30.0, 100.0), 0.0);
        assert!(campfire_ambush_chance(100.0, 100.0) > 0.0);
        assert!(
            (campfire_ambush_chance(75.0, 100.0) - campfire_ambush_chance(100.0, 100.0) * 0.5).abs()
                < 1e-4
        );
    }

    /// Resting adjacent to a campfire trickle-heals you.
    #[test]
    fn campfire_trickle_heals_when_adjacent() {
        let mut g = Game::new(1);
        g.monsters.clear();
        g.player.maxhp = 100.0;
        g.player.hp = 30.0;
        let (cx, cy) = g.player_chunk();
        g.campfires.push(Campfire { x: g.player.x, y: g.player.y, cx, cy });
        g.update_campfire(0.1);
        assert!(g.player.hp > 30.0, "resting heals");
        assert!(g.resting, "resting flag set when adjacent");
    }

    /// Below half HP, resting never triggers an ambush.
    #[test]
    fn campfire_no_ambush_below_half() {
        let mut g = Game::new(3);
        g.monsters.clear();
        g.player.maxhp = 100.0;
        g.player.hp = 20.0;
        let (cx, cy) = g.player_chunk();
        g.campfires.push(Campfire { x: g.player.x, y: g.player.y, cx, cy });
        for _ in 0..30 {
            g.update_campfire(0.1); // ~3s; regen keeps hp under 50
        }
        assert!(g.player.hp < 50.0, "still below half in this window");
        assert!(g.monsters.is_empty(), "no ambush while resting below half HP");
    }

    /// Resting at full HP eventually springs an ambush (and spawns attackers).
    #[test]
    fn campfire_ambush_fires_at_full_hp() {
        let mut g = Game::new(2);
        g.monsters.clear();
        g.player.maxhp = 100.0;
        g.player.hp = 100.0; // max risk
        let (cx, cy) = g.player_chunk();
        g.campfires.push(Campfire { x: g.player.x, y: g.player.y, cx, cy });
        let mut ambushed = false;
        for _ in 0..500 {
            g.update_campfire(0.1);
            if !g.monsters.is_empty() {
                ambushed = true;
                break;
            }
        }
        assert!(ambushed, "resting at full HP eventually triggers an ambush");
    }

    /// Offering junk at a shrine consumes the items and grants a reward.
    #[test]
    fn shrine_offering_consumes_items_and_rewards() {
        let mut g = Game::new(1);
        g.monsters.clear();
        let (cx, cy) = g.player_chunk();
        g.shrines.push(Shrine { x: g.player.x, y: g.player.y, cx, cy });
        g.player.inv.clear();
        g.player.equip = [-1; 4];
        g.player.maxhp = 100.0;
        g.player.hp = 1.0; // so a health reward is detectable
        for _ in 0..10 {
            g.player.inv.push(test_bow());
        }
        let (ammo0, hp0) = (g.player.ammo, g.player.hp);
        g.make_offering(&(0..10).collect::<Vec<_>>());
        assert!(g.player.inv.len() < 10, "the 10 offered items were consumed");
        let rewarded = g.player.ammo > ammo0 || g.player.hp > hp0 || !g.player.inv.is_empty();
        assert!(rewarded, "the shrine granted a reward");
    }

    /// Offering an ancient (unique) is a gamble: a double blessing, or a boss.
    #[test]
    fn offering_an_ancient_doubles_or_angers_a_boss() {
        let (mut saw_double, mut saw_boss) = (false, false);
        for seed in 0..14u64 {
            let mut g = Game::new(seed);
            g.monsters.clear();
            let (cx, cy) = g.player_chunk();
            g.shrines.push(Shrine { x: g.player.x, y: g.player.y, cx, cy });
            g.player.inv.clear();
            g.player.equip = [-1; 4];
            g.player.maxhp = 100.0;
            g.player.hp = 1.0;
            let mut u = test_bow();
            u.unique = true;
            g.player.inv.push(u);
            let ammo0 = g.player.ammo;
            g.make_offering(&[0]);
            if g.monsters.iter().any(|m| m.mega) {
                saw_boss = true;
                assert!(
                    g.player.inv.is_empty() && (g.player.hp - 1.0).abs() < 0.01 && g.player.ammo == ammo0,
                    "the boss outcome gives no reward",
                );
            } else {
                saw_double = true;
                let rewarded = g.player.ammo > ammo0 || g.player.hp > 1.0 || !g.player.inv.is_empty();
                assert!(rewarded, "the blessing outcome grants rewards");
            }
        }
        assert!(saw_double && saw_boss, "both ancient outcomes occur across seeds");
    }

    /// Fishing needs bait and calm; a nearby monster or empty quiver blocks it.
    #[test]
    fn fishing_is_gated_by_bait_and_calm() {
        let mut g = Game::new(2);
        g.monsters.clear();
        g.player.ammo = 0;
        assert!(!g.can_fish(), "no bait -> cannot fish");
        assert!(g.no_threat(), "no monsters -> calm");
        let m = monster::generate(1, 0, 0, g.player.x + 20.0, g.player.y, 3);
        g.monsters.push(m);
        assert!(!g.no_threat(), "a nearby monster breaks the calm");
    }

    /// Bait is spent per cast; escapes still cost it; health is the common catch.
    #[test]
    fn fishing_spends_bait_and_health_is_common() {
        // Cancel before a bite: no bait spent.
        let mut g = Game::new(1);
        g.player.ammo = 10;
        g.do_fish(-2.0);
        assert_eq!(g.player.ammo, 10, "cancelling before a bite costs no bait");

        // Escape: bait lost, no reward.
        let mut g = Game::new(1);
        g.player.ammo = 10;
        g.player.maxhp = 100.0;
        g.player.hp = 50.0;
        g.player.inv.clear();
        g.do_fish(-1.0);
        assert_eq!(g.player.ammo, 7, "an escape still costs the bait");
        assert!((g.player.hp - 50.0).abs() < 0.01 && g.player.inv.is_empty(), "escape yields nothing");

        // Landed catches: over many seeds, a fish (heal) is the most common reward.
        let (mut heals, trials) = (0, 200u64);
        for seed in 0..trials {
            let mut g = Game::new(seed);
            g.player.ammo = 100;
            g.player.maxhp = 100.0;
            g.player.hp = 1.0;
            g.player.inv.clear();
            g.player.equip = [-1; 4];
            g.do_fish(0.8);
            if g.player.hp > 1.0 {
                heals += 1;
            }
        }
        assert!(heals as f32 / trials as f32 > 0.5, "health is the most common catch ({heals}/{trials})");
    }

    /// Death lifts the curse (and clears its hunters).
    #[test]
    fn death_lifts_the_curse() {
        let mut g = Game::new(4);
        g.begin_relic(2.0);
        g.spawn_hunter();
        g.player.hp = -1.0;
        g.respawn_if_dead();
        assert!(g.relic.is_none(), "curse lifts on death");
        assert!(!g.monsters.iter().any(|m| m.hunter));
    }

    /// The relic sprint (and spent-relic memory) survives a save/reload.
    #[test]
    fn relic_state_round_trips() {
        let mut g = Game::new(24680);
        g.begin_relic(7.0);
        g.relic.as_mut().unwrap().steps = 1234.0;
        g.looted_relics.push((5, -6));
        g.build_save();
        let bytes = g.save.clone();
        let mut g2 = Game::new(0);
        g2.load_save(&bytes);
        assert!(g2.relic.is_some(), "the relic sprint resumes after reload");
        assert!((g2.relic.as_ref().unwrap().steps - 1234.0).abs() < 1.0, "steps persisted");
        assert!(g2.looted_relics.contains(&(5, -6)), "spent relic chunk persisted");
    }

    /// A shield-shrine ward (and spent shrine chunks) survive a save/reload.
    #[test]
    fn shield_ward_round_trips() {
        let mut g = Game::new(1357);
        g.player.shield = 42.0;
        g.player.shield_max = 60.0;
        g.looted_shields.push((3, 4));
        g.build_save();
        let bytes = g.save.clone();
        let mut g2 = Game::new(0);
        g2.load_save(&bytes);
        assert!((g2.player.shield - 42.0).abs() < 0.5, "ward persisted through reload");
        assert!((g2.player.shield_max - 60.0).abs() < 0.5, "ward max persisted");
        assert!(g2.looted_shields.contains(&(3, 4)), "spent shield-shrine chunk persisted");
    }

    /// An "ancient" unique must survive repeated save/reload without its damage
    /// creeping up — the old bug re-inflated unique power ×1.6 every reload.
    #[test]
    fn ancient_weapon_survives_reload_without_inflating() {
        let mut g = Game::new(777);
        g.player.max_dist = 25_000.0;
        let power = difficulty_at(25_000.0 * TILE, 0.0) as f32;
        g.player.inv.clear();
        g.player.inv.push(generate_unique(0x0A0C1E27, power));
        let dmg0 = g.player.inv[0].damage;
        for _ in 0..6 {
            g.build_save();
            let bytes = g.save.clone();
            let mut g2 = Game::new(0);
            g2.load_save(&bytes);
            g = g2;
        }
        let dmg1 = g.player.inv[0].damage;
        assert!((dmg1 - dmg0).abs() < 0.01, "ancient damage stable across reloads: {dmg0} -> {dmg1}");
    }

    /// A save corrupted by the old compounding bug (an absurd stored power) is
    /// healed on load by the power clamp — no multi-trillion-DPS weapons.
    #[test]
    fn corrupt_weapon_power_is_clamped_on_load() {
        let mut g = Game::new(888);
        g.player.max_dist = 25_000.0;
        g.player.inv.clear();
        let mut w = generate_unique(0x0BADD00D, difficulty_at(25_000.0 * TILE, 0.0) as f32);
        w.power = 5.0e11; // as if inflated by dozens of reloads
        g.player.inv.push(w);
        g.build_save();
        let bytes = g.save.clone();
        let mut g2 = Game::new(0);
        g2.load_save(&bytes);
        let dmg = g2.player.inv[0].damage;
        assert!(dmg.is_finite() && dmg < 100_000.0, "corrupt power healed to a sane weapon, damage {dmg}");
    }

    /// Stats (steps/kills/deaths/chests) round-trip through the save.
    #[test]
    fn stats_round_trip() {
        let mut g = Game::new(9);
        g.kills = 123;
        g.deaths = 7;
        g.chests_opened = 4;
        g.steps = 55555.0;
        g.play_secs = 3600.0;
        g.celebrated = true;
        g.build_save();
        let bytes = g.save.clone();
        let mut g2 = Game::new(0);
        g2.load_save(&bytes);
        assert_eq!(g2.kills, 123);
        assert_eq!(g2.deaths, 7);
        assert_eq!(g2.chests_opened, 4);
        assert!((g2.steps - 55555.0).abs() < 1.0);
        assert!(g2.celebrated);
    }

    /// Monsters can't occupy the player's space — a ranged monster sitting on
    /// the player is pushed out (so it can actually shoot instead of no-op'ing).
    #[test]
    fn monsters_are_pushed_out_of_player_space() {
        let mut g = Game::new(3);
        g.monsters.clear();
        let mut m = monster::generate(1, 0, 0, g.player.x, g.player.y, 5);
        m.ranged = true;
        m.temper = monster::FIGHT;
        m.radius = 4.0;
        g.monsters.push(m);
        let min_sep = 4.0 + PLAYER_R;
        for _ in 0..40 {
            g.update_monsters(0.016);
            let m = &g.monsters[0];
            let d = ((m.x - g.player.x).powi(2) + (m.y - g.player.y).powi(2)).sqrt();
            assert!(d >= min_sep - 0.6, "monster overlapped the player (d={d}, min={min_sep})");
        }
        // A hostile ranged monster ends up at a firing standoff, not point-blank.
        let m = &g.monsters[0];
        let d = ((m.x - g.player.x).powi(2) + (m.y - g.player.y).powi(2)).sqrt();
        assert!(d > 20.0, "ranged monster keeps its distance (d={d})");
    }

    /// Movement speed is capped no matter how high the Move skill climbs, so it
    /// stays humanly controllable.
    #[test]
    fn move_speed_is_capped() {
        let mut g = Game::new(3);
        g.player.skills[SK_MOVE as usize] = 1_000_000.0; // absurd Move level
        g.input.keys = 8; // hold right
        let dt = 0.016;
        let max_step = BASE_SPEED * (1.0 + MOVE_BONUS_CAP) * dt + 0.01;
        for _ in 0..120 {
            let (ox, oy) = (g.player.x, g.player.y);
            g.move_player(dt);
            let moved = ((g.player.x - ox).powi(2) + (g.player.y - oy).powi(2)).sqrt();
            assert!(moved <= max_step, "one frame moved {moved} > cap {max_step}");
        }
    }

    /// A real melee kill (via `player_attack`) produces ground drops and counts.
    #[test]
    fn kills_produce_drops() {
        let mut g = Game::new(5);
        g.monsters.clear();
        g.loot.clear();
        g.kills = 0;
        // A cluster of trivially-weak monsters right in front of the player; one
        // cleaving swing kills them all.
        for k in 0..40u64 {
            let mut m = monster::generate(k + 1, 0, 0, g.player.x + 10.0, g.player.y, 2);
            m.hp = 0.5;
            m.def = 0.0;
            m.mega = false;
            g.monsters.push(m);
        }
        g.input.attack = true;
        g.input.aimx = 1.0;
        g.input.aimy = 0.0;
        g.player.atk_cd = 0.0;
        g.player_attack(0.0);
        assert!(g.kills >= 1, "melee kills register");
        assert!(!g.loot.is_empty(), "kills scatter ground drops (ammo/weapon/health)");
    }

    /// A looted chest/fountain chunk is remembered, persisted, and won't respawn
    /// (no reload-to-farm).
    #[test]
    fn looted_chest_does_not_respawn() {
        // Picking up a chest remembers its chunk and survives a save round-trip.
        let mut g = Game::new(5);
        g.loot.clear();
        let (px, py) = (g.player.x, g.player.y);
        g.loot.push(Loot { x: px, y: py, kind: Drop::Chest { seed: 1, power: 5.0 } });
        g.pickups();
        let ch = chunk_of(px, py);
        assert!(g.looted_chests.contains(&ch), "chest chunk remembered");
        g.build_save();
        let bytes = g.save.clone();
        let mut g2 = Game::new(0);
        g2.load_save(&bytes);
        assert!(g2.looted_chests.contains(&ch), "looted chest persisted through save");

        // Find a chunk that WOULD spawn a chest, then confirm marking it looted
        // suppresses the chest.
        let mut chest_chunk = None;
        'outer: for cx in 0..400i64 {
            let mut probe = Game::new(5);
            probe.loot.clear();
            probe.spawn_chunk(cx, 7);
            if probe.loot.iter().any(|l| matches!(l.kind, Drop::Chest { .. })) {
                chest_chunk = Some((cx, 7));
                break 'outer;
            }
        }
        let cc = chest_chunk.expect("found a chunk that spawns a chest");
        let mut g3 = Game::new(5);
        g3.looted_chests.push(cc);
        g3.loot.clear();
        g3.spawn_chunk(cc.0, cc.1);
        assert!(!g3.loot.iter().any(|l| matches!(l.kind, Drop::Chest { .. })), "looted chunk skips its chest");
    }

    /// Weapons stop being picked up at the cap; consumables still are.
    #[test]
    fn inventory_is_capped() {
        let mut g = Game::new(4);
        g.player.inv.clear();
        for _ in 0..INVENTORY_CAP {
            g.player.inv.push(test_bow());
        }
        g.loot.clear();
        g.loot.push(Loot { x: g.player.x, y: g.player.y, kind: Drop::Weapon { seed: 1, power: 1.0, rarity: 0 } });
        g.pickups();
        assert_eq!(g.player.inv.len(), INVENTORY_CAP, "no weapon pickup past the cap");
        assert_eq!(g.loot.len(), 1, "the blocked item stays on the ground");

        g.player.ammo = 0;
        g.loot.push(Loot { x: g.player.x, y: g.player.y, kind: Drop::Ammo(5) });
        g.pickups();
        assert_eq!(g.player.ammo, 5, "consumables ignore the cap");
    }

    /// Walking over ground drops grants ammo / heals.
    #[test]
    fn pickups_grant_ammo_and_health() {
        let mut g = Game::new(3);
        g.player.ammo = 0;
        g.player.maxhp = 50.0;
        g.player.hp = 10.0;
        g.loot.push(Loot { x: g.player.x, y: g.player.y, kind: Drop::Ammo(5) });
        g.loot.push(Loot { x: g.player.x, y: g.player.y, kind: Drop::Health(15.0) });
        g.pickups();
        assert_eq!(g.player.ammo, 5);
        assert!((g.player.hp - 25.0).abs() < 0.01);
        assert!(g.loot.is_empty());
    }

    /// Two vaults overlapping the player (adjacent chunks can drop both within
    /// VAULT_RADIUS) must all retire on a single crack — otherwise the puzzle
    /// re-opens on the neighbor in an inescapable loop, leaving a still-closed
    /// vault beside the loot.
    #[test]
    fn overlapping_vaults_all_retire_on_a_single_crack() {
        let mut g = Game::new(3);
        g.vaults.clear();
        let (cx, cy) = g.player_chunk();
        // Two vaults, both inside VAULT_RADIUS (30px) of the player.
        g.vaults.push(Vault { x: g.player.x + 4.0, y: g.player.y, cx, cy, opened: false });
        g.vaults.push(Vault { x: g.player.x - 8.0, y: g.player.y + 6.0, cx: cx + 1, cy, opened: false });

        g.open_vault_near();

        assert!(g.vaults.iter().all(|v| v.opened), "the whole overlapping cluster retires at once");
        // Both chunks are remembered so neither re-spawns on reload.
        assert!(g.looted_vaults.contains(&(cx, cy)));
        assert!(g.looted_vaults.contains(&(cx + 1, cy)));
        // Exactly one cache is granted (one chest among the drops), not two.
        let chests = g.loot.iter().filter(|l| matches!(l.kind, Drop::Chest { .. })).count();
        assert_eq!(chests, 1, "an overlapping cluster yields a single cache");

        // A second crack in the same spot finds nothing left to open.
        let loot_before = g.loot.len();
        g.open_vault_near();
        assert_eq!(g.loot.len(), loot_before, "no reward from an already-cracked cluster");
    }
}

