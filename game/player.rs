//! Player state and use-based skills.
//!
//! There is no character level. Eight skills grow from use and directly feed
//! back into effectiveness (damage, attack speed, move speed, defense).

use crate::weapon::Weapon;
use crate::SK_DEFENSE;

pub const N_SKILLS: usize = 8;

pub struct Player {
    pub x: f32,
    pub y: f32,
    pub hp: f32,
    pub maxhp: f32,
    pub shield: f32,             // blue ward (e.g. a shield shrine) soaked before hp
    pub shield_max: f32,         // for the HUD bar; non-recharging once granted
    pub ammo: u32,               // shared pool consumed by ranged weapons
    pub max_dist: f32,           // farthest distance ever reached (persistent record)
    pub cp_x: f32,               // furthest banked checkpoint (respawn point)
    pub cp_y: f32,
    pub skills: [f32; N_SKILLS], // xp per skill
    pub inv: Vec<Weapon>,
    pub equip: [i32; 4],         // inventory index per slot, -1 = empty
    pub slot: usize,             // active weapon slot
    pub atk_cd: f32,
}

impl Player {
    pub fn new() -> Self {
        Player {
            x: 0.0,
            y: 0.0,
            hp: 50.0,
            maxhp: 50.0,
            shield: 0.0,
            shield_max: 0.0,
            ammo: 15, // enough to make an early ranged find immediately usable
            max_dist: 0.0,
            cp_x: 0.0,
            cp_y: 0.0,
            skills: [0.0; N_SKILLS],
            inv: Vec::new(),
            equip: [-1, -1, -1, -1],
            slot: 0,
            atk_cd: 0.0,
        }
    }

    /// Currently equipped weapon, if any.
    pub fn weapon(&self) -> Option<&Weapon> {
        let idx = self.equip[self.slot];
        if idx >= 0 {
            self.inv.get(idx as usize)
        } else {
            None
        }
    }

    /// Point the active slot at `slot`. If that slot holds a weapon, select it.
    /// If it's empty and the current slot is *also* empty — which happens after a
    /// respawn when the active weapon shattered — snap to the first slot that
    /// still has a weapon so the player is never left swinging nothing. A valid
    /// current selection is kept when an empty slot is requested.
    pub fn select_slot(&mut self, slot: usize) {
        if slot >= 4 {
            return;
        }
        if self.equip[slot] >= 0 {
            self.slot = slot;
        } else if self.equip[self.slot] < 0 {
            for cand in 0..4 {
                if self.equip[cand] >= 0 {
                    self.slot = cand;
                    break;
                }
            }
        }
    }

    /// Grant skill xp (skills improve through use).
    pub fn train(&mut self, skill: u8, amount: f32) {
        self.skills[skill as usize] += amount;
    }

    /// Recompute max hp from the Defense skill and clamp current hp.
    pub fn refresh_maxhp(&mut self) {
        self.maxhp = 50.0 + skill_level(self.skills[SK_DEFENSE as usize]) as f32 * 8.0;
        if self.hp > self.maxhp {
            self.hp = self.maxhp;
        }
    }
}

/// Skill level from accumulated xp (sqrt curve — cheap and never caps).
#[inline]
pub fn skill_level(xp: f32) -> u32 {
    (xp.max(0.0) / 6.0).sqrt() as u32
}

/// Effectiveness bonus from a skill: +6% per level.
#[inline]
pub fn skill_bonus(xp: f32) -> f32 {
    skill_level(xp) as f32 * 0.06
}
