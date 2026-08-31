//! Headless progression simulator.
//!
//! Drives the *real* `Game` simulation with a simple bot that pushes outward,
//! fights what it meets, grabs nearby drops, and uses its best usable weapon
//! (falling back to melee when out of ammo). Because it runs the actual rules,
//! its output is real evidence of the challenge/economy — not a re-derivation.
//!
//! Run: `cargo run --release --bin economy -- [--seeds N] [--target D]
//!       [--minutes T] [--deaths M] [--band B] [--json]`

use crate::player::skill_level;
use crate::{difficulty_at, Game, CHECKPOINT, SK_DEFENSE};

struct Config {
    seeds: u64,
    target: f32,  // stop a run once max distance (tiles) reaches this
    minutes: f32, // wall-clock game-time cap per run
    max_deaths: u32,
    band: f32, // record a milestone every `band` tiles of new best distance
    dt: f32,
    json: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            seeds: 6,
            target: 800.0,
            minutes: 15.0,
            max_deaths: 400,
            band: 100.0,
            dt: 1.0 / 30.0, // coarse but stable (update clamps dt); ~2x faster
            json: false,
        }
    }
}

#[derive(Clone)]
struct Milestone {
    dist: f32,
    secs: f32,
    deaths: u32,
    def_lvl: u32,
    maxhp: f32,
    kills: u32,
    dmg_taken: f32,
    healed: f32,
    best_dps: f32,
    difficulty: u32,
}

struct RunResult {
    seed: u64,
    reached: f32,
    secs: f32,
    deaths: u32,
    milestones: Vec<Milestone>,
}

fn outward(px: f32, py: f32) -> (f32, f32) {
    let l = (px * px + py * py).sqrt();
    if l < 1.0 {
        (1.0, 0.0)
    } else {
        (px / l, py / l)
    }
}

fn keybits(dx: f32, dy: f32) -> u32 {
    let l = (dx * dx + dy * dy).sqrt();
    if l < 1e-4 {
        return 0;
    }
    let (ux, uy) = (dx / l, dy / l);
    let mut k = 0;
    if uy < -0.38 {
        k |= 1;
    }
    if uy > 0.38 {
        k |= 2;
    }
    if ux < -0.38 {
        k |= 4;
    }
    if ux > 0.38 {
        k |= 8;
    }
    k
}

/// Best weapon by raw dps found so far (ignores ammo — "quality of loot").
fn best_dps(g: &Game) -> f32 {
    g.player
        .inv
        .iter()
        .map(|w| w.damage / w.cooldown.max(0.05))
        .fold(0.0, f32::max)
}

/// Equip the best *usable* weapon (ranged only if we have ammo) into slot 0.
fn pick_weapon(g: &mut Game) {
    let ammo = g.player.ammo;
    let mut best = -1i32;
    let mut best_dps = -1.0f32;
    for (i, w) in g.player.inv.iter().enumerate() {
        let usable = if w.ranged { ammo > 0 } else { true };
        if !usable {
            continue;
        }
        let dps = w.damage / w.cooldown.max(0.05);
        if dps > best_dps {
            best_dps = dps;
            best = i as i32;
        }
    }
    if best >= 0 {
        g.player.equip[0] = best;
        g.player.slot = 0;
    }
}

/// One simulation tick of the bot.
fn step(g: &mut Game, dt: f32, wander: f32) {
    pick_weapon(g);
    let (px, py) = (g.player.x, g.player.y);

    // Nearest monster (for aim + attack).
    let mut nm: Option<(f32, f32, f32)> = None;
    for m in &g.monsters {
        let (dx, dy) = (m.x - px, m.y - py);
        let d2 = dx * dx + dy * dy;
        if nm.map_or(true, |(_, _, b)| d2 < b) {
            nm = Some((m.x, m.y, d2));
        }
    }

    let (ranged, range) = g
        .player
        .weapon()
        .map(|w| (w.ranged, w.range))
        .unwrap_or((false, 20.0));

    let (mut aimx, mut aimy) = (1.0, 0.0);
    let mut attack = false;
    if let Some((mx, my, d2)) = nm {
        aimx = mx - px;
        aimy = my - py;
        let reach = if ranged { range } else { range + 6.0 };
        attack = d2.sqrt() <= reach;
    }
    g.input.aimx = aimx;
    g.input.aimy = aimy;
    g.input.attack = attack;

    // Movement: divert to nearby drops (models grabbing ammo/health), else push
    // outward along a heading rotated by `wander` (to route around obstacles).
    let mut nl: Option<(f32, f32, f32)> = None;
    for l in &g.loot {
        let (dx, dy) = (l.x - px, l.y - py);
        let d2 = dx * dx + dy * dy;
        if nl.map_or(true, |(_, _, b)| d2 < b) {
            nl = Some((l.x, l.y, d2));
        }
    }
    let (mvx, mvy) = match nl {
        Some((lx, ly, d2)) if d2 < 60.0 * 60.0 => (lx - px, ly - py),
        _ => {
            let (ox, oy) = outward(px, py);
            let (c, s) = (wander.cos(), wander.sin());
            (ox * c - oy * s, ox * s + oy * c)
        }
    };
    g.input.keys = keybits(mvx, mvy);

    // `advance` runs the pure simulation without rebuilding the render snapshot.
    g.advance(dt);
}

fn run_one(seed: u64, cfg: &Config) -> RunResult {
    let mut g = Game::new(seed);
    let mut secs = 0.0f32;
    let mut next_band = cfg.band;
    let mut milestones = Vec::new();

    // Anti-stuck: rotate the outward heading if best distance stops improving.
    let mut wander = 0.0f32;
    let mut last_best = 0.0f32;
    let mut stall = 0.0f32;

    let max_secs = cfg.minutes * 60.0;
    while g.player.max_dist < cfg.target && secs < max_secs && g.deaths < cfg.max_deaths {
        step(&mut g, cfg.dt, wander);
        secs += cfg.dt;

        if g.player.max_dist > last_best + 0.1 {
            last_best = g.player.max_dist;
            stall = 0.0;
        } else {
            stall += cfg.dt;
            if stall > 5.0 {
                wander += 1.2; // try a different route around water/mountains
                stall = 0.0;
            }
        }

        while g.player.max_dist >= next_band {
            milestones.push(Milestone {
                dist: next_band,
                secs,
                deaths: g.deaths,
                def_lvl: skill_level(g.player.skills[SK_DEFENSE as usize]),
                maxhp: g.player.maxhp,
                kills: g.kills,
                dmg_taken: g.dmg_taken,
                healed: g.healed,
                best_dps: best_dps(&g),
                difficulty: difficulty_at(g.player.x, g.player.y),
            });
            next_band += cfg.band;
        }
    }

    RunResult {
        seed,
        reached: g.player.max_dist,
        secs,
        deaths: g.deaths,
        milestones,
    }
}

pub fn run_cli() {
    let cfg = parse_args();
    let runs: Vec<RunResult> = (0..cfg.seeds)
        .map(|i| {
            let r = run_one(i + 1, &cfg);
            eprintln!(
                "  [seed {:>3}] reached {:>4.0} tiles, {:>3} deaths, {:.0}s",
                r.seed, r.reached, r.deaths, r.secs
            );
            r
        })
        .collect();

    if cfg.json {
        print_json(&runs, &cfg);
    } else {
        print_report(&runs, &cfg);
    }
}

fn parse_args() -> Config {
    let mut cfg = Config::default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let mut val = || {
            i += 1;
            args.get(i).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0)
        };
        match a {
            "--seeds" => cfg.seeds = val() as u64,
            "--target" => cfg.target = val(),
            "--minutes" => cfg.minutes = val(),
            "--deaths" => cfg.max_deaths = val() as u32,
            "--band" => cfg.band = val().max(10.0),
            "--json" => cfg.json = true,
            _ => {}
        }
        i += 1;
    }
    cfg
}

/// Average the milestone rows that every completed run reached.
fn averaged(runs: &[RunResult], cfg: &Config) -> Vec<(f32, Milestone, usize)> {
    let mut out = Vec::new();
    let mut band = cfg.band;
    loop {
        let rows: Vec<&Milestone> = runs
            .iter()
            .filter_map(|r| r.milestones.iter().find(|m| (m.dist - band).abs() < 0.01))
            .collect();
        if rows.is_empty() {
            break;
        }
        let n = rows.len() as f32;
        let avg = Milestone {
            dist: band,
            secs: rows.iter().map(|m| m.secs).sum::<f32>() / n,
            deaths: (rows.iter().map(|m| m.deaths).sum::<u32>() as f32 / n).round() as u32,
            def_lvl: (rows.iter().map(|m| m.def_lvl).sum::<u32>() as f32 / n).round() as u32,
            maxhp: rows.iter().map(|m| m.maxhp).sum::<f32>() / n,
            kills: (rows.iter().map(|m| m.kills).sum::<u32>() as f32 / n).round() as u32,
            dmg_taken: rows.iter().map(|m| m.dmg_taken).sum::<f32>() / n,
            healed: rows.iter().map(|m| m.healed).sum::<f32>() / n,
            best_dps: rows.iter().map(|m| m.best_dps).sum::<f32>() / n,
            difficulty: (rows.iter().map(|m| m.difficulty).sum::<u32>() as f32 / n).round() as u32,
        };
        out.push((band, avg, rows.len()));
        band += cfg.band;
    }
    out
}

fn print_report(runs: &[RunResult], cfg: &Config) {
    println!("\nWebWander progression simulator — {} runs, target {} tiles, cap {:.0} min / {} deaths\n",
        cfg.seeds, cfg.target as u32, cfg.minutes, cfg.max_deaths);

    println!("Per-run outcome:");
    for r in runs {
        println!("  seed {:>3}: reached {:>4.0} tiles in {:>5.0}s over {:>3} deaths",
            r.seed, r.reached, r.secs, r.deaths);
    }
    let reached_avg = runs.iter().map(|r| r.reached).sum::<f32>() / runs.len() as f32;
    let deaths_avg = runs.iter().map(|r| r.deaths as f32).sum::<f32>() / runs.len() as f32;
    println!("  average: reached {:.0} tiles, {:.1} deaths\n", reached_avg, deaths_avg);

    println!("Averaged progression (n = runs reaching that band):");
    println!("{:>5} {:>5} {:>7} {:>6} {:>4} {:>5} {:>6} {:>7} {:>7} {:>6} {:>5}",
        "dist", "Lv", "time_s", "deaths", "Def", "maxHP", "kills", "dmgTkn", "healed", "heal%", "wDps");
    for (_band, m, n) in averaged(runs, cfg) {
        let heal_pct = if m.dmg_taken > 0.0 { 100.0 * m.healed / m.dmg_taken } else { 0.0 };
        let ckpt = if (m.dist / CHECKPOINT).fract() < 1e-3 && m.dist >= CHECKPOINT { " <ckpt" } else { "" };
        println!("{:>5.0} {:>5} {:>7.0} {:>6} {:>4} {:>5.0} {:>6} {:>7.0} {:>7.0} {:>5.0} {:>5.1}  (n={}){}",
            m.dist, m.difficulty, m.secs, m.deaths, m.def_lvl, m.maxhp, m.kills,
            m.dmg_taken, m.healed, heal_pct, m.best_dps, n, ckpt);
    }

    // Headline economy numbers between consecutive bands.
    let avg = averaged(runs, cfg);
    if avg.len() >= 2 {
        println!("\nEconomy per {} tiles of progress (marginal):", cfg.band as u32);
        for w in avg.windows(2) {
            let (a, b) = (&w[0].1, &w[1].1);
            let dd = b.dmg_taken - a.dmg_taken;
            let dh = b.healed - a.healed;
            let ddeath = b.deaths as i64 - a.deaths as i64;
            println!("  {:>4.0}->{:<4.0}: {:>6.0} dmg taken, {:>5.0} healed ({:>4.0}% funded), {:>2} deaths",
                a.dist, b.dist, dd, dh, if dd > 0.0 { 100.0 * dh / dd } else { 0.0 }, ddeath);
        }
    }
    println!();
}

fn print_json(runs: &[RunResult], cfg: &Config) {
    let mut s = String::from("{\"bands\":[");
    for (i, (_b, m, n)) in averaged(runs, cfg).iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"dist\":{},\"difficulty\":{},\"secs\":{:.1},\"deaths\":{},\"def\":{},\"maxhp\":{:.0},\"kills\":{},\"dmg_taken\":{:.0},\"healed\":{:.0},\"best_dps\":{:.1},\"n\":{}}}",
            m.dist, m.difficulty, m.secs, m.deaths, m.def_lvl, m.maxhp, m.kills, m.dmg_taken, m.healed, m.best_dps, n));
    }
    s.push_str("],\"runs\":[");
    for (i, r) in runs.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"seed\":{},\"reached\":{:.0},\"secs\":{:.0},\"deaths\":{}}}",
            r.seed, r.reached, r.secs, r.deaths));
    }
    s.push_str("]}");
    println!("{s}");
}
