//! Procedural weapons.
//!
//! A weapon is fully described by a single `seed`: `generate(seed)` always
//! rebuilds the same weapon. That makes saving trivial — we persist seeds, not
//! stats. Weapons are `base + prefix + element + suffix`, assembled from small
//! rules tables (edit the tables below to add content, no code changes needed).

use crate::rng::Rng;
use crate::{COLD, FIRE, PHYS, PIERCE, POISON, SK_AXE, SK_BOW, SK_SWORD};

// Special-effect flag bits.
pub const SP_POISON_DOT: u8 = 1;
pub const SP_ARMOR_PEN: u8 = 2;

#[derive(Clone)]
pub struct Weapon {
    pub seed: u64,
    pub power: f32,     // difficulty the weapon was rolled at (for exact regen)
    pub durability: f32, // 1.0 = pristine, 0.0 = broken (persistent, not from seed)
    pub unique: bool,   // chest "unique" — regenerated via generate_unique
    pub base: u8,       // index into BASES
    pub dmg_type: u8,
    pub damage: f32,
    pub cooldown: f32,  // seconds between attacks
    pub range: f32,     // world px (melee reach or projectile lifetime * speed)
    pub ranged: bool,
    pub proj_speed: f32,
    pub rarity: u8,     // 0 common .. 3 epic
    pub class_skill: u8,
    pub special: u8,
    pub name: String,
}

// base name, class skill, ranged?, base damage, cooldown, reach, projectile speed
struct Base {
    name: &'static str,
    skill: u8,
    ranged: bool,
    dmg: f32,
    cd: f32,
    range: f32,
    proj: f32,
}

const BASES: &[Base] = &[
    Base { name: "Sword",  skill: SK_SWORD, ranged: false, dmg: 6.0,  cd: 0.42, range: 22.0, proj: 0.0 },
    Base { name: "Axe",    skill: SK_AXE,   ranged: false, dmg: 9.5,  cd: 0.68, range: 20.0, proj: 0.0 },
    Base { name: "Dagger", skill: SK_SWORD, ranged: false, dmg: 3.6,  cd: 0.24, range: 16.0, proj: 0.0 },
    Base { name: "Spear",  skill: SK_SWORD, ranged: false, dmg: 6.8,  cd: 0.5,  range: 30.0, proj: 0.0 },
    Base { name: "Bow",    skill: SK_BOW,   ranged: true,  dmg: 5.5,  cd: 0.5,  range: 150.0, proj: 150.0 },
    Base { name: "Staff",  skill: SK_BOW,   ranged: true,  dmg: 7.0,  cd: 0.62, range: 130.0, proj: 120.0 },
];

// prefix name + damage multiplier + cooldown multiplier
struct Prefix { name: &'static str, dmg: f32, cd: f32 }
const PREFIXES: &[Prefix] = &[
    Prefix { name: "Rusty",   dmg: 0.8,  cd: 1.05 },
    Prefix { name: "Fine",    dmg: 1.1,  cd: 0.98 },
    Prefix { name: "Jagged",  dmg: 1.25, cd: 1.0 },
    Prefix { name: "Heavy",   dmg: 1.5,  cd: 1.2 },
    Prefix { name: "Brutal",  dmg: 1.7,  cd: 1.1 },
];

// element prefix -> damage type
struct Elem { name: &'static str, dtype: u8, special: u8 }
const ELEMS: &[Elem] = &[
    Elem { name: "Flaming",  dtype: FIRE,   special: 0 },
    Elem { name: "Frozen",   dtype: COLD,   special: 0 },
    Elem { name: "Venomous", dtype: POISON, special: SP_POISON_DOT },
    Elem { name: "Piercing", dtype: PIERCE, special: SP_ARMOR_PEN },
];

// suffix name + cooldown mult + range mult
struct Suffix { name: &'static str, cd: f32, range: f32 }
const SUFFIXES: &[Suffix] = &[
    Suffix { name: "of Speed",  cd: 0.75, range: 1.0 },
    Suffix { name: "of Reach",  cd: 1.0,  range: 1.4 },
    Suffix { name: "of Fury",   cd: 0.85, range: 1.0 },
    Suffix { name: "of Ruin",   cd: 1.1,  range: 1.15 },
];

/// Rebuild a weapon from its seed. `power` scales base damage with world
/// difficulty at the drop location.
pub fn generate(seed: u64, power: f32) -> Weapon {
    let mut r = Rng::new(seed ^ 0x5719_A0FE);
    let bi = r.below(BASES.len() as u32) as usize;
    let b = &BASES[bi];

    // Rarity: weighted toward common, nudged up by difficulty.
    let roll = r.f01() + (power - 1.0) * 0.02;
    let rarity: u8 = if roll > 0.94 { 3 } else if roll > 0.80 { 2 } else if roll > 0.5 { 1 } else { 0 };

    let mut damage = b.dmg;
    let mut cooldown = b.cd;
    let mut range = b.range;
    let mut dmg_type = PHYS;
    let mut special = 0u8;
    let mut prefix_word = String::new();
    let mut elem_word = String::new();
    let mut suffix_word = String::new();

    let affixes = rarity; // number of affixes == rarity tier
    let mut applied = 0;

    // Damage prefix
    if affixes > applied {
        let p = &PREFIXES[r.below(PREFIXES.len() as u32) as usize];
        damage *= p.dmg;
        cooldown *= p.cd;
        prefix_word = format!("{} ", p.name);
        applied += 1;
    }
    // Element prefix
    if affixes > applied && r.chance(0.75) {
        let e = &ELEMS[r.below(ELEMS.len() as u32) as usize];
        dmg_type = e.dtype;
        special |= e.special;
        elem_word = format!("{} ", e.name);
        applied += 1;
    }
    // Suffix
    if affixes > applied {
        let s = &SUFFIXES[r.below(SUFFIXES.len() as u32) as usize];
        cooldown *= s.cd;
        range *= s.range;
        suffix_word = format!(" {}", s.name);
    }

    // Scale damage with difficulty (sub-linear so it never runs away).
    damage *= 1.0 + (power - 1.0) * 0.5;

    let name = format!("{}{}{}{}", elem_word, prefix_word, b.name, suffix_word);

    Weapon {
        seed,
        power,
        durability: 1.0,
        unique: false,
        base: bi as u8,
        dmg_type,
        damage,
        cooldown: cooldown.max(0.12),
        range,
        ranged: b.ranged,
        proj_speed: b.proj,
        rarity,
        class_skill: b.skill,
        special,
        name,
    }
}

/// A chest "unique": a deliberately overpowered weapon rolled well above the
/// local difficulty, meant as a leg-up to reach the next checkpoint. Regenerated
/// identically from its seed so it round-trips through the save.
pub fn generate_unique(seed: u64, power: f32) -> Weapon {
    let mut w = generate(seed, power * 1.6 + 2.0);
    w.unique = true;
    w.rarity = 3;
    w.damage *= 1.6;
    w.cooldown = (w.cooldown * 0.85).max(0.1);
    w.name = format!("Ancient {}", w.name);
    // Store the ORIGINAL power, not the inflated `power*1.6+2` that generate()
    // stamped in. Otherwise every save→reload would call generate_unique() with
    // the already-inflated value and re-inflate it (×1.6 each time), compounding
    // damage into the billions after enough pause/resumes.
    w.power = power;
    w
}
