//! Procedural monsters.
//!
//! A monster is a recipe: pick a `body`, an elemental `resist`, a `weakness`,
//! and a movement/attack style, then scale stats by the world difficulty at its
//! location. There are no hand-authored monsters — only traits and a generator.

use crate::rng::{hash2, Rng};

// Temperament — how a monster behaves when it notices the player.
pub const FIGHT: u8 = 0; // charges and attacks
pub const FLEE: u8 = 1; // runs away (ranged ones kite and shoot while fleeing)
pub const WANDER: u8 = 2; // roams the area, indifferent until provoked

pub struct Monster {
    pub cx: i64,
    pub cy: i64,
    pub x: f32,
    pub y: f32,
    pub hp: f32,
    pub maxhp: f32,
    pub atk: f32,
    pub def: f32,
    pub speed: f32,
    pub dmg_type: u8,
    pub resist: u8,  // takes half damage from this type
    pub weak: u8,    // takes double damage from this type
    pub ranged: bool,
    pub mega: bool,       // rare boss: huge, and resistances matter a lot
    pub temper: u8,       // FIGHT / FLEE / WANDER
    pub anger: f32,       // seconds it stays hostile after being hit
    pub cooldown: f32,
    pub cd: f32,          // current attack cooldown timer
    pub regen: f32,       // hp/sec
    pub level: u32,
    pub xp: f32,
    pub body: u8,         // render shape hint
    pub radius: f32,
    pub rng: u64,         // per-monster stream for wander targets
    pub wx: f32,          // current wander target
    pub wy: f32,
    pub wt: f32,          // time until a new wander target is picked
    pub name: String,
    pub from_arena: bool, // spawned by an arena wave (sim-only; not serialized)
    pub hunter: bool,     // cursed-relic hunter: ignores chunk despawn, hunts until killed
    pub turbo_to: f32,    // hunter catch-up surge: >0 = surging until within this range
    pub champion: bool,   // a lone named elite: a fair 1v1 mini-boss guarding a prize
}

impl Monster {
    /// Advance the monster's private RNG stream, returning a float in [0,1).
    pub fn roll(&mut self) -> f32 {
        self.rng = crate::rng::mix64(self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15));
        crate::rng::u01(self.rng)
    }
}

// body traits: name, hp mult, atk mult, def base, speed px/s
struct Body {
    noun: &'static str,
    hp: f32,
    atk: f32,
    def: f32,
    speed: f32,
    regen: f32,
}
const BODIES: &[Body] = &[
    Body { noun: "Beast",  hp: 1.0, atk: 1.0, def: 1.0, speed: 30.0, regen: 0.0 },
    Body { noun: "Insect", hp: 0.6, atk: 0.8, def: 0.5, speed: 46.0, regen: 0.0 },
    Body { noun: "Golem",  hp: 1.8, atk: 1.1, def: 4.0, speed: 18.0, regen: 0.0 },
    Body { noun: "Wisp",   hp: 0.7, atk: 1.2, def: 0.5, speed: 34.0, regen: 1.2 },
    Body { noun: "Brute",  hp: 1.6, atk: 1.5, def: 1.5, speed: 22.0, regen: 0.0 },
];

const ELEM_WORDS: [&str; 5] = ["", "Fiery", "Frost", "Venom", "Razor"];
const SPEED_WORDS: [&str; 2] = ["Swift", "Lurching"];

/// Deterministic seed for the i-th monster of a chunk.
pub fn monster_seed(world_seed: u64, cx: i64, cy: i64, i: u32) -> u64 {
    hash2(world_seed ^ 0x4D0F_51E2, cx.wrapping_mul(73_856_093) ^ i as i64, cy.wrapping_mul(19_349_663))
}

/// Generate a monster at world-pixel position `(x, y)` with the given seed and
/// difficulty `level`.
pub fn generate(seed: u64, cx: i64, cy: i64, x: f32, y: f32, level: u32) -> Monster {
    let mut r = Rng::new(seed);
    let bi = r.below(BODIES.len() as u32) as usize;
    let b = &BODIES[bi];

    let resist = r.below(5) as u8;
    // Weakness is a different damage type from the resistance.
    let mut weak = r.below(5) as u8;
    if weak == resist {
        weak = (weak + 1) % 5;
    }
    // Attack element leans toward the resistance (a fire creature spits fire).
    let dmg_type = if r.chance(0.6) { resist } else { r.below(5) as u8 };
    // Ranged attackers grow more common with distance but are present from the
    // start, so bows/staves have targets to matter against early on.
    let ranged_chance = (0.14 + 0.03 * level as f32).min(0.4);
    let ranged = r.chance(ranged_chance);
    // Temperament: mostly fighters, some roamers, a few skittish fleers. Any
    // monster that gets hit turns hostile regardless (see `anger`).
    let troll = r.f01();
    let temper = if troll < 0.58 {
        FIGHT
    } else if troll < 0.82 {
        WANDER
    } else {
        FLEE
    };

    let lvl = level.max(1);
    let lf = lvl as f32;
    // Early monsters are soft but not trivial, ramping steadily with level.
    let maxhp = (9.0 + 5.0 * lf) * b.hp;
    let atk = (1.7 + 1.2 * lf) * b.atk;
    let def = b.def + 0.32 * lf;
    let speed = b.speed * r.range(0.9, 1.1) + lf * 0.4;
    // Ranged monsters fire slowly so their shots can be seen and dodged.
    let cooldown = if ranged { 1.7 } else { 0.9 };
    let xp = (6.0 + 4.0 * lf) * (0.6 + 0.4 * b.hp);

    let name = build_name(&mut r, resist, dmg_type, b.noun, speed);

    Monster {
        cx,
        cy,
        x,
        y,
        hp: maxhp,
        maxhp,
        atk,
        def,
        speed,
        dmg_type,
        resist,
        weak,
        ranged,
        mega: false,
        temper,
        anger: 0.0,
        cooldown,
        cd: r.range(0.0, cooldown),
        regen: b.regen * (0.5 + 0.3 * lf),
        level: lvl,
        xp,
        body: bi as u8,
        radius: (2.5 + b.hp).min(6.0),
        rng: r.next(),
        wx: x,
        wy: y,
        wt: 0.0,
        name,
        from_arena: false,
        hunter: false,
        turbo_to: 0.0,
        champion: false,
    }
}

/// Turn a freshly-generated monster into a rare mega-boss: a huge, slow, always-
/// hostile threat. Its elemental resistance/weakness are amplified elsewhere so
/// bringing the wrong damage type is close to hopeless.
pub fn make_mega(m: &mut Monster) {
    m.mega = true;
    m.maxhp *= 10.0;
    m.hp = m.maxhp;
    m.atk *= 1.7;
    m.def *= 1.5;
    m.speed *= 0.72;
    m.radius = 10.0;
    m.xp *= 8.0;
    m.regen *= 4.0;
    m.temper = FIGHT;
    m.name = format!("Colossal {}", m.name);
}

/// Turn a freshly-generated monster into a champion: a lone named elite tuned as
/// a fair 1v1 — tankier and hard-hitting, but slower and dodgeable (no swarm).
pub fn make_champion(m: &mut Monster) {
    m.champion = true;
    m.maxhp *= 5.0;
    m.hp = m.maxhp;
    m.atk *= 1.4;
    m.def *= 1.3;
    m.speed *= 0.85;
    m.radius = 8.0;
    m.xp *= 5.0;
    m.regen *= 2.0;
    m.temper = FIGHT;
    m.anger = 1e9; // always committed once roused
    m.name = format!("Champion {}", m.name);
}

fn build_name(r: &mut Rng, resist: u8, dmg_type: u8, noun: &str, speed: f32) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if speed > 40.0 {
        parts.push(SPEED_WORDS[0]);
    } else if speed < 22.0 {
        parts.push(SPEED_WORDS[1]);
    }
    // Prefer the elemental word of its attack, else its resistance.
    let ew = ELEM_WORDS[dmg_type as usize];
    let rw = ELEM_WORDS[resist as usize];
    if !ew.is_empty() && r.chance(0.7) {
        parts.push(ew);
    } else if !rw.is_empty() {
        parts.push(rw);
    }
    let mut s = String::new();
    for p in parts {
        s.push_str(p);
        s.push(' ');
    }
    s.push_str(noun);
    s
}

/// Damage-type interaction: 2x on weakness, 0.5x on resistance, else 1x.
/// Megas amplify this hard — the wrong element barely scratches them (0.12x)
/// while their weakness melts them (3x), so weapon choice is decisive.
#[inline]
pub fn elem_mult2(dmg_type: u8, resist: u8, weak: u8, mega: bool) -> f32 {
    if mega {
        if dmg_type == weak { 3.0 } else if dmg_type == resist { 0.12 } else { 0.6 }
    } else if dmg_type == weak {
        2.0
    } else if dmg_type == resist {
        0.5
    } else {
        1.0
    }
}

