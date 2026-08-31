//! Terrain generation and tile rules.
//!
//! Terrain is a pure function of `(seed, tileX, tileY)`. Two noise fields
//! (temperature, moisture) plus an elevation field select one of 10 tile
//! types via a small lookup. Nothing is stored; the same coordinate always
//! regenerates identically.

use crate::rng::{fbm, hash2, u01};

// Tile ids (also used as palette indices on the TS side).
pub const DEEP_WATER: u8 = 0;
pub const SHALLOW_WATER: u8 = 1;
pub const SAND: u8 = 2;
pub const GRASS: u8 = 3;
pub const DENSE_GRASS: u8 = 4;
pub const DIRT: u8 = 5;
pub const ROCK: u8 = 6;
pub const MOUNTAIN: u8 = 7;
pub const SNOW: u8 = 8;
pub const SWAMP: u8 = 9;

pub const TILE: f32 = 16.0;

/// Elevation / temperature / moisture at a tile. Large-scale regions come from
/// the low base frequency; a few octaves add coastline detail.
pub fn fields(seed: u64, tx: i64, ty: i64) -> (f32, f32, f32) {
    let (fx, fy) = (tx as f32, ty as f32);
    let e = fbm(seed ^ 0xE1E7A701, fx * 0.032, fy * 0.032, 5);
    let t = fbm(seed ^ 0x7E77A9F1, fx * 0.011 + 137.0, fy * 0.011, 3);
    let m = fbm(seed ^ 0x33A17B9D, fx * 0.019 - 61.0, fy * 0.019, 3);
    (e, t, m)
}

/// Classify a tile from its noise fields.
pub fn tile_at(seed: u64, tx: i64, ty: i64) -> u8 {
    let (e, t, m) = fields(seed, tx, ty);
    if e < 0.30 {
        DEEP_WATER
    } else if e < 0.37 {
        SHALLOW_WATER
    } else if e < 0.41 {
        SAND
    } else if e > 0.84 {
        SNOW
    } else if e > 0.75 {
        MOUNTAIN
    } else if e > 0.69 {
        ROCK
    } else if t < 0.35 {
        // cold lowlands
        if m > 0.55 {
            SNOW
        } else {
            DIRT
        }
    } else if t < 0.65 {
        // temperate
        if m < 0.35 {
            DIRT
        } else if m < 0.62 {
            GRASS
        } else {
            DENSE_GRASS
        }
    } else {
        // hot
        if m < 0.35 {
            SAND
        } else if m < 0.60 {
            GRASS
        } else {
            SWAMP
        }
    }
}

// Decorative second-layer scatter features (drawn on top of terrain, do not
// affect movement). Deterministic per tile.
pub const F_NONE: u8 = 0;
pub const F_TREE: u8 = 1;
pub const F_PINE: u8 = 2;
pub const F_ROCK: u8 = 3;
pub const F_BUSH: u8 = 4;
pub const F_CACTUS: u8 = 5;
pub const F_REED: u8 = 6;
pub const F_FLOWER: u8 = 7;

/// Which scatter feature (if any) sits on a tile — a pure function of position,
/// so the world stays deterministic. Density/type depend on the terrain.
pub fn feature_at(seed: u64, tx: i64, ty: i64, tile: u8) -> u8 {
    let h = hash2(seed ^ 0xF3A7_1C5D, tx, ty);
    let r = u01(h); // presence roll
    let pick = ((h >> 21) & 0xff) as f32 / 255.0; // variety roll
    match tile {
        GRASS => {
            if r < 0.14 {
                if pick < 0.4 { F_TREE } else if pick < 0.7 { F_BUSH } else { F_FLOWER }
            } else { F_NONE }
        }
        DENSE_GRASS => {
            if r < 0.36 {
                if pick < 0.6 { F_TREE } else { F_PINE }
            } else { F_NONE }
        }
        DIRT => {
            if r < 0.10 {
                if pick < 0.5 { F_BUSH } else { F_ROCK }
            } else { F_NONE }
        }
        ROCK => if r < 0.24 { F_ROCK } else { F_NONE },
        MOUNTAIN => if r < 0.30 { F_ROCK } else { F_NONE },
        SNOW => if r < 0.16 { F_PINE } else { F_NONE },
        SAND => if r < 0.06 { F_CACTUS } else { F_NONE },
        SWAMP => if r < 0.26 { F_REED } else { F_NONE },
        _ => F_NONE,
    }
}

/// Can an entity stand on this tile?
#[inline]
pub fn passable(tile: u8) -> bool {
    tile != DEEP_WATER && tile != MOUNTAIN
}

/// Movement multiplier (>1 means slower).
#[inline]
pub fn move_cost(tile: u8) -> f32 {
    match tile {
        SHALLOW_WATER => 2.2,
        SWAMP => 1.8,
        DENSE_GRASS => 1.35,
        SNOW => 1.3,
        SAND => 1.12,
        ROCK => 1.15,
        _ => 1.0,
    }
}

/// Per-second hazard damage while standing on a tile (scaled by difficulty).
#[inline]
pub fn hazard(tile: u8) -> f32 {
    match tile {
        SWAMP => 0.6,
        _ => 0.0,
    }
}

/// True if the world position (in pixels) is on a passable tile.
pub fn passable_px(seed: u64, x: f32, y: f32) -> bool {
    let tx = (x / TILE).floor() as i64;
    let ty = (y / TILE).floor() as i64;
    passable(tile_at(seed, tx, ty))
}

/// The tile at a world pixel position.
pub fn tile_px(seed: u64, x: f32, y: f32) -> u8 {
    let tx = (x / TILE).floor() as i64;
    let ty = (y / TILE).floor() as i64;
    tile_at(seed, tx, ty)
}
