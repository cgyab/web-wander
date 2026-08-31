//! Deterministic hashing, PRNG, and value noise.
//! No external crates: everything derives from splitmix64.

/// splitmix64 finalizer — fast, high-quality 64-bit mixing.
#[inline]
pub fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// Deterministic hash of a seed + 2D integer coordinate.
#[inline]
pub fn hash2(seed: u64, x: i64, y: i64) -> u64 {
    let mut h = seed ^ 0xD1B54A32D192ED03;
    h = mix64(h ^ (x as u64).wrapping_mul(0xA24BAED4963EE407));
    h = mix64(h ^ (y as u64).wrapping_mul(0x9FB21C651E98DF25));
    mix64(h)
}

/// Map a hash to a float in [0, 1).
#[inline]
pub fn u01(h: u64) -> f32 {
    ((h >> 40) as f32) / ((1u64 << 24) as f32)
}

/// Small stateful PRNG seeded deterministically.
pub struct Rng(u64);

impl Rng {
    #[inline]
    pub fn new(seed: u64) -> Self {
        Rng(mix64(seed ^ 0x2545F4914F6CDD1D))
    }
    #[inline]
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        mix64(self.0)
    }
    #[inline]
    pub fn f01(&mut self) -> f32 {
        u01(self.next())
    }
    /// Float in [lo, hi).
    #[inline]
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f01()
    }
    /// Integer in [0, n).
    #[inline]
    pub fn below(&mut self, n: u32) -> u32 {
        (self.next() % n as u64) as u32
    }
    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.f01() < p
    }
}

#[inline]
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Bilinearly-interpolated value noise in [0, 1].
pub fn value_noise(seed: u64, x: f32, y: f32) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let xf = smooth(x - x0);
    let yf = smooth(y - y0);
    let (ix, iy) = (x0 as i64, y0 as i64);
    let v00 = u01(hash2(seed, ix, iy));
    let v10 = u01(hash2(seed, ix + 1, iy));
    let v01 = u01(hash2(seed, ix, iy + 1));
    let v11 = u01(hash2(seed, ix + 1, iy + 1));
    let a = v00 + (v10 - v00) * xf;
    let b = v01 + (v11 - v01) * xf;
    a + (b - a) * yf
}

/// Fractal Brownian motion: sum of octaves of value noise, normalized to [0, 1].
pub fn fbm(seed: u64, mut x: f32, mut y: f32, octaves: u32) -> f32 {
    let mut amp = 0.5;
    let mut sum = 0.0;
    let mut norm = 0.0;
    for i in 0..octaves {
        sum += amp * value_noise(seed.wrapping_add((i as u64) << 16), x, y);
        norm += amp;
        amp *= 0.5;
        x *= 2.0;
        y *= 2.0;
    }
    sum / norm
}
