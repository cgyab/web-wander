// Procedural canvas renderer. Everything is drawn from rectangles and tiny pixel
// shapes at a fixed logical resolution (320x180); the browser scales it up with
// nearest-neighbor for the 8-bit look. No image assets.

import type { Snapshot } from "./snapshot";

export const LOGICAL_W = 320;
export const LOGICAL_H = 180;
const TILE = 16;

// Terrain palette, indexed by tile id (matches game/world.rs). [base, accent]
const TERRAIN: [string, string][] = [
  ["#12365f", "#0e2c4e"], // 0 deep water
  ["#2f6fb0", "#3a7cbd"], // 1 shallow water
  ["#d9c68a", "#cdb87a"], // 2 sand
  ["#4a9a3a", "#3f8a32"], // 3 grass
  ["#2f6f28", "#276021"], // 4 dense grass
  ["#7a5a3a", "#6b4e32"], // 5 dirt
  ["#7d7d84", "#6d6d74"], // 6 rock
  ["#55555c", "#484850"], // 7 mountain
  ["#e6eef0", "#d3dde0"], // 8 snow
  ["#3b5236", "#324629"], // 9 swamp
];

// Element / damage-type accent colors.
const ELEM = ["#dcdce4", "#ff5a2a", "#66ccff", "#7ee04a", "#ffcf4a"];
// Loot rarity colors.
const RARITY = ["#e8e8f0", "#7bd88f", "#5aa0ff", "#c774ff"];

function hash(tx: number, ty: number): number {
  let h = (tx * 374761393 + ty * 668265263) | 0;
  h = (h ^ (h >>> 13)) * 1274126177;
  return (h ^ (h >>> 16)) >>> 0;
}

export class Renderer {
  private ctx: CanvasRenderingContext2D;
  private flashUntil = 0; // timestamp; red vignette after a respawn
  private trail: { x: number; y: number; t: number }[] = []; // cursed-relic comet trail
  constructor(canvas: HTMLCanvasElement) {
    canvas.width = LOGICAL_W;
    canvas.height = LOGICAL_H;
    const ctx = canvas.getContext("2d", { alpha: false });
    if (!ctx) throw new Error("no 2d context");
    ctx.imageSmoothingEnabled = false;
    this.ctx = ctx;
  }

  /** Resize the pixel buffer (logical resolution). Width is fixed; height tracks
   *  the device aspect so the game fills the screen. */
  setLogicalSize(w: number, h: number) {
    const c = this.ctx.canvas;
    if (c.width !== w || c.height !== h) {
      c.width = w;
      c.height = h;
      this.ctx.imageSmoothingEnabled = false; // reset by a resize
    }
  }

  private get W() { return this.ctx.canvas.width; }
  private get H() { return this.ctx.canvas.height; }

  /** Trigger the post-respawn red flash. */
  flash() {
    this.flashUntil = performance.now() + 700;
  }

  draw(s: Snapshot, aimX: number, aimY: number) {
    this.drawTerrain(s);
    this.drawFeatures(s);

    // During the 100,000 flash mob everyone bobs to the beat.
    const now = performance.now();
    const hop = (worldX: number, worldY: number) =>
      s.celebrating ? Math.round(Math.abs(Math.sin(now / 130 + (worldX + worldY) * 0.3)) * -5) : 0;

    // Arena rings + cursed-fog haze lie on the ground, under monsters and loot.
    const fogList = s.entities.filter((e) => e.kind === 13);
    for (const e of s.entities) {
      if (e.kind === 9) this.drawArenaRings(e.x - s.camX, e.y - s.camY, e.radius, e.dtype, e.shape, now);
      else if (e.kind === 13) this.drawMiasmaHaze(Math.round(e.x - s.camX), Math.round(e.y - s.camY), e.radius, now);
    }
    // How deeply the player is inside cursed fog (0 = outside, 1 = at a center).
    const pFog = this.fogAt(fogList, s.px, s.py);

    // Entities are sorted by y so nearer ones overlap farther ones.
    const ents = [...s.entities].sort((a, b) => a.y - b.y);
    for (const e of ents) {
      const sx = Math.round(e.x - s.camX);
      const sy = Math.round(e.y - s.camY) + hop(e.x, e.y);
      if (sx < -16 || sx > this.W + 16 || sy < -16 || sy > this.H + 16) continue;
      switch (e.kind) {
        case 1: {
          // Ambush: in fog, monsters are hidden until ~2 tiles away, then fade in.
          let a = 1;
          if (pFog > 0) {
            const d = Math.hypot(e.x - s.px, e.y - s.py);
            const vis = Math.max(0, Math.min(1, (70 - d) / 50));
            a = 1 - pFog * (1 - vis);
            if (a <= 0.02) break;
          }
          this.drawMonster(sx, sy, e.radius, e.shape & 0x0f, e.dtype, e.hpFrac, (e.shape & 0x80) !== 0, (e.shape & 0x40) !== 0, a, (e.shape & 0x20) !== 0);
          break;
        }
        case 2: this.drawDot(sx, sy, ELEM[e.dtype] ?? "#fff", 2); break;
        case 3: this.drawDot(sx, sy, "#ff7070", 2); break;
        case 4: this.drawLoot(sx, sy, e.shape); break;
        case 5: this.drawAmmo(sx, sy); break;
        case 6: this.drawHealth(sx, sy); break;
        case 7: this.drawChest(sx, sy); break;
        case 8: this.drawFountain(sx, sy, now); break;
        case 10: this.drawRelic(sx, sy, now); break;
        case 11: this.drawCampfire(sx, sy, now); break;
        case 12: this.drawShrine(sx, sy, now); break;
        case 14: this.drawShieldShrine(sx, sy, now); break;
        case 15: this.drawVault(sx, sy, e.shape === 1, now); break;
        case 16: this.drawRift(sx, sy, now); break;
      }
    }

    // Comet-trail sparkles behind the player during the cursed speed burst.
    if (s.relic.active) this.cometTrail(s, now);
    else if (this.trail.length) this.trail.length = 0;

    // Player is always centered (and hops during the party).
    const psx = Math.round(s.px - s.camX);
    const psy = Math.round(s.py - s.camY) + hop(s.px, s.py);
    this.drawPlayer(psx, psy, aimX, aimY, s);

    // Cursed fog closes the view in around the player while they're inside it.
    if (pFog > 0) this.drawFogVignette(pFog);

    if (s.celebrating) this.confetti(now, 40);
    else if (s.milestoneT > 0) this.confetti(now, 14);

    // Respawn vignette.
    if (now < this.flashUntil) {
      this.ctx.fillStyle = `rgba(150,20,20,${((this.flashUntil - now) / 700) * 0.55})`;
      this.ctx.fillRect(0, 0, this.W, this.H);
    }
  }

  /** How deep a world point is inside any cursed fog (0 outside → 1 at center). */
  private fogAt(fog: { x: number; y: number; radius: number }[], wx: number, wy: number): number {
    let m = 0;
    for (const f of fog) {
      const d = Math.hypot(wx - f.x, wy - f.y);
      if (d < f.radius) m = Math.max(m, 1 - d / f.radius);
    }
    return m;
  }

  /** A sickly haze over the fog patch so you can see it (and choose to enter). */
  private drawMiasmaHaze(x: number, y: number, r: number, now: number) {
    const ctx = this.ctx;
    if (x + r < 0 || x - r > this.W || y + r < 0 || y - r > this.H) return;
    const g = ctx.createRadialGradient(x, y, r * 0.2, x, y, r);
    g.addColorStop(0, "rgba(74,104,74,0.42)");
    g.addColorStop(0.7, "rgba(56,82,60,0.30)");
    g.addColorStop(1, "rgba(50,72,56,0)");
    ctx.fillStyle = g;
    ctx.beginPath(); ctx.arc(x, y, r, 0, Math.PI * 2); ctx.fill();
    // Drifting darker wisps for a slow swirl.
    for (let i = 0; i < 3; i++) {
      const a = now / 1500 + i * 2.1;
      const bx = x + Math.cos(a) * r * 0.4, by = y + Math.sin(a * 1.3) * r * 0.3;
      const bg = ctx.createRadialGradient(bx, by, 0, bx, by, r * 0.5);
      bg.addColorStop(0, "rgba(30,46,36,0.22)");
      bg.addColorStop(1, "rgba(30,46,36,0)");
      ctx.fillStyle = bg;
      ctx.beginPath(); ctx.arc(bx, by, r * 0.5, 0, Math.PI * 2); ctx.fill();
    }
  }

  /** The view closes in around the player, tighter the deeper they're in. */
  private drawFogVignette(intensity: number) {
    const ctx = this.ctx;
    const cx = this.W / 2, cy = this.H / 2;
    const maxR = Math.hypot(this.W, this.H) / 2;
    const clear = maxR * (0.6 - 0.42 * intensity); // sight radius shrinks with depth
    const g = ctx.createRadialGradient(cx, cy, Math.max(4, clear * 0.5), cx, cy, maxR);
    g.addColorStop(0, "rgba(18,28,20,0)");
    g.addColorStop(1, `rgba(10,20,14,${0.62 + 0.34 * intensity})`);
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, this.W, this.H);
    // Faint green pall over the whole view.
    ctx.fillStyle = `rgba(58,88,60,${0.05 + 0.09 * intensity})`;
    ctx.fillRect(0, 0, this.W, this.H);
  }

  private drawArenaRings(x: number, y: number, inner: number, outer: number, state: number, now: number) {
    const ctx = this.ctx;
    const onScreen = (r: number) => !(x + r < 0 || x - r > this.W || y + r < 0 || y - r > this.H);
    let color = "rgba(150,150,175,0.30)"; // idle (dormant)
    let lw = 2;
    if (state === 1) {
      const p = 0.5 + 0.5 * Math.sin(now / 170); // active: pulsing red
      color = `rgba(255,${Math.round(70 + p * 40)},70,${0.5 + p * 0.35})`;
      lw = 3;
    } else if (state === 2) {
      color = "rgba(120,120,140,0.28)"; // forfeited/abandoned: dead grey
    } else if (state === 3) {
      const p = 0.5 + 0.5 * Math.sin(now / 240); // idle + player near: amber telegraph
      color = `rgba(255,200,90,${0.42 + p * 0.35})`;
      lw = 3;
    } else if (state === 4) {
      color = "rgba(255,216,107,0.85)"; // cleared: a bright, steady victory gold
      lw = 3;
    }
    ctx.save();
    // Conquered arena: a soft gold wash inside the inner ring as a trophy marker.
    if (state === 4 && onScreen(inner)) {
      const g = ctx.createRadialGradient(x, y, inner * 0.2, x, y, inner);
      g.addColorStop(0, "rgba(255,216,107,0.10)");
      g.addColorStop(1, "rgba(255,216,107,0)");
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.arc(x, y, inner, 0, Math.PI * 2);
      ctx.fill();
    }
    // Outer ring (the apron boundary): dashed, in a bright violet that reads on
    // every tile type. A dark under-stroke gives it an outline so it never
    // blends into dirt/rock/water/snow — leaving this ring forfeits.
    if (outer > 0 && onScreen(outer)) {
      ctx.setLineDash([6, 5]);
      ctx.strokeStyle = "rgba(0,0,0,0.55)";
      ctx.lineWidth = 4;
      ctx.beginPath();
      ctx.arc(x, y, outer, 0, Math.PI * 2);
      ctx.stroke();
      ctx.strokeStyle = state === 1 ? "rgba(214,102,255,0.9)"
        : state === 3 ? "rgba(214,140,255,0.7)" // telegraph: brighter than dormant
        : state === 4 ? "rgba(255,216,107,0.6)" // cleared: gold
        : state === 2 ? "rgba(120,120,140,0.35)" // forfeited: dead grey
        : "rgba(190,150,225,0.5)";
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(x, y, outer, 0, Math.PI * 2);
      ctx.stroke();
      ctx.setLineDash([]);
    }
    // Inner ring (the combat boundary): solid.
    if (onScreen(inner)) {
      ctx.strokeStyle = color;
      ctx.lineWidth = lw;
      ctx.beginPath();
      ctx.arc(x, y, inner, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.restore();
  }

  private drawFeatures(s: Snapshot) {
    for (let ry = 0; ry < s.rows; ry++) {
      for (let rx = 0; rx < s.cols; rx++) {
        const f = s.features[ry * s.cols + rx];
        if (f === 0) continue;
        const tx = s.tx0 + rx;
        const ty = s.ty0 + ry;
        const h = hash(tx, ty);
        // Sub-tile offset so features don't sit on a rigid grid.
        const ox = 3 + (h % 7);
        const oy = 3 + ((h >> 3) % 7);
        const x = Math.round(tx * TILE - s.camX) + ox;
        const y = Math.round(ty * TILE - s.camY) + oy;
        this.drawFeature(f, x, y);
      }
    }
  }

  private drawFeature(f: number, x: number, y: number) {
    const ctx = this.ctx;
    switch (f) {
      case 1: // broadleaf tree
        ctx.fillStyle = "#5a3b22";
        ctx.fillRect(x, y + 2, 2, 4);
        ctx.fillStyle = "#2f7d2f";
        ctx.fillRect(x - 2, y - 2, 6, 5);
        ctx.fillStyle = "#3a9a3a";
        ctx.fillRect(x - 1, y - 3, 3, 2);
        break;
      case 2: // pine
        ctx.fillStyle = "#4a3018";
        ctx.fillRect(x, y + 3, 1, 3);
        ctx.fillStyle = "#1f5f2a";
        ctx.fillRect(x - 2, y + 1, 5, 2);
        ctx.fillRect(x - 1, y - 1, 3, 2);
        ctx.fillRect(x, y - 3, 1, 2);
        break;
      case 3: // rock/boulder
        ctx.fillStyle = "#8a8a92";
        ctx.fillRect(x - 2, y, 6, 4);
        ctx.fillStyle = "#6d6d75";
        ctx.fillRect(x - 1, y + 2, 4, 2);
        break;
      case 4: // bush
        ctx.fillStyle = "#357a35";
        ctx.fillRect(x - 1, y, 4, 3);
        ctx.fillStyle = "#46934a";
        ctx.fillRect(x, y - 1, 2, 1);
        break;
      case 5: // saguaro cactus: a trunk with two arms that bend upward
        ctx.fillStyle = "#3f8f4a";
        ctx.fillRect(x, y - 3, 2, 8); // trunk
        // left arm: out, then up
        ctx.fillRect(x - 2, y + 1, 2, 1);
        ctx.fillRect(x - 2, y - 2, 1, 3);
        // right arm: out a touch higher, then up (asymmetry looks natural)
        ctx.fillRect(x + 2, y + 2, 2, 1);
        ctx.fillRect(x + 3, y - 1, 1, 3);
        // sunlit highlight down the trunk's left edge
        ctx.fillStyle = "#57ab5f";
        ctx.fillRect(x, y - 3, 1, 8);
        break;
      case 6: // reeds
        ctx.fillStyle = "#6f8f4a";
        ctx.fillRect(x - 1, y - 2, 1, 5);
        ctx.fillRect(x + 1, y - 3, 1, 6);
        ctx.fillRect(x + 3, y - 1, 1, 4);
        break;
      case 7: // flowers
        ctx.fillStyle = "#e8d24a";
        ctx.fillRect(x, y, 1, 1);
        ctx.fillStyle = "#e06a9a";
        ctx.fillRect(x + 2, y + 1, 1, 1);
        ctx.fillStyle = "#e8e8f0";
        ctx.fillRect(x + 1, y + 2, 1, 1);
        break;
    }
  }

  private drawChest(x: number, y: number) {
    const ctx = this.ctx;
    // Ruined stone around a golden chest — makes ruins read at a glance.
    ctx.fillStyle = "#6d6d75";
    ctx.fillRect(x - 7, y + 3, 3, 2);
    ctx.fillRect(x + 5, y - 4, 2, 3);
    ctx.fillRect(x + 5, y + 3, 2, 2);
    ctx.fillStyle = "#5a5a62";
    ctx.fillRect(x - 7, y - 4, 2, 4);
    // Chest body
    ctx.fillStyle = "#7a4a1c";
    ctx.fillRect(x - 3, y - 1, 7, 5);
    ctx.fillStyle = "#a86a24";
    ctx.fillRect(x - 3, y - 3, 7, 2);
    // Gold trim + lock, gently pulsing.
    const glow = (performance.now() / 350) % 2 < 1;
    ctx.fillStyle = glow ? "#ffe27a" : "#ffcf4a";
    ctx.fillRect(x - 3, y - 1, 7, 1);
    ctx.fillRect(x, y - 1, 1, 3);
  }

  // A campfire rest site: logs and a flickering flame with a warm glow.
  private drawCampfire(x: number, y: number, now: number) {
    const ctx = this.ctx;
    const flick = 0.5 + 0.5 * Math.sin(now / 90) * Math.cos(now / 47);
    // Warm glow.
    ctx.globalAlpha = 0.14 + flick * 0.08;
    ctx.fillStyle = "#ff8a3a";
    this.circle(x, y, 9);
    ctx.globalAlpha = 1;
    // Logs (crossed).
    ctx.fillStyle = "#5a3b22";
    ctx.fillRect(x - 4, y + 2, 8, 2);
    ctx.fillStyle = "#3f2a18";
    ctx.fillRect(x - 3, y + 3, 8, 1);
    // Flame: red base, orange mid, yellow tip (height flickers).
    const h = 3 + Math.round(flick * 2);
    ctx.fillStyle = "#e23b1e";
    ctx.fillRect(x - 1, y - 1, 3, 3);
    ctx.fillStyle = "#ff7a1e";
    ctx.fillRect(x, y - h + 1, 2, h);
    ctx.fillStyle = "#ffd257";
    ctx.fillRect(x, y - h + 1, 1, 2);
  }

  // An offering shrine: a stone altar with a soft, holy glow.
  private drawShrine(x: number, y: number, now: number) {
    const ctx = this.ctx;
    const pulse = 0.28 + 0.22 * Math.sin(now / 380);
    ctx.globalAlpha = pulse;
    ctx.fillStyle = "#7fd0ff";
    this.circle(x, y - 1, 9);
    ctx.globalAlpha = 1;
    // Stone base + pillar.
    ctx.fillStyle = "#7a7a86";
    ctx.fillRect(x - 4, y + 3, 9, 2); // base
    ctx.fillStyle = "#8f8f9c";
    ctx.fillRect(x - 3, y - 1, 7, 4); // altar block
    ctx.fillStyle = "#6a6a76";
    ctx.fillRect(x - 3, y + 2, 7, 1); // shadow line
    // Rune light on top, gently pulsing.
    ctx.fillStyle = (now / 500) % 2 < 1 ? "#bfe8ff" : "#7fd0ff";
    ctx.fillRect(x - 1, y - 3, 3, 2);
    ctx.fillRect(x, y - 4, 1, 1);
  }

  // A shield shrine: a small pedestal cradling a floating blue shield.
  private drawShieldShrine(x: number, y: number, now: number) {
    const ctx = this.ctx;
    const pulse = 0.3 + 0.25 * Math.sin(now / 300);
    ctx.globalAlpha = pulse;
    ctx.fillStyle = "#7fd0ff";
    this.circle(x, y - 2, 9);
    ctx.globalAlpha = 1;
    // Stone pedestal.
    ctx.fillStyle = "#7a7a86";
    ctx.fillRect(x - 4, y + 3, 9, 2);
    ctx.fillStyle = "#8f8f9c";
    ctx.fillRect(x - 2, y, 5, 3);
    // Floating shield crest, bobbing gently.
    const b = Math.round(Math.sin(now / 260));
    ctx.fillStyle = "#3aa0e0";
    ctx.fillRect(x - 3, y - 6 + b, 6, 5);
    ctx.fillStyle = "#8fd8ff";
    ctx.fillRect(x - 2, y - 6 + b, 4, 2); // sheen
    ctx.fillStyle = "#eaf6ff";
    ctx.fillRect(x - 1, y - 3 + b, 2, 2); // boss stud
  }

  // A rune vault: a stone door inscribed with glowing runes (dim once opened).
  private drawVault(x: number, y: number, opened: boolean, now: number) {
    const ctx = this.ctx;
    const glow = opened ? 0.12 : 0.28 + 0.2 * Math.sin(now / 340);
    ctx.globalAlpha = glow;
    ctx.fillStyle = opened ? "#6a7a9a" : "#9a7fe0";
    this.circle(x, y - 1, 9);
    ctx.globalAlpha = 1;
    // Stone frame + door.
    ctx.fillStyle = "#6a6a76";
    ctx.fillRect(x - 5, y - 6, 11, 11); // frame
    ctx.fillStyle = opened ? "#20242e" : "#3a3450"; // doorway (dark once open)
    ctx.fillRect(x - 3, y - 4, 7, 9);
    if (!opened) {
      // Three glowing rune marks on the door.
      const p = (now / 300) % 3;
      for (let i = 0; i < 3; i++) {
        ctx.fillStyle = Math.floor(p) === i ? "#e6d8ff" : "#8f78d0";
        ctx.fillRect(x - 1, y - 3 + i * 3, 2, 2);
      }
    }
    // Base.
    ctx.fillStyle = "#55555f";
    ctx.fillRect(x - 5, y + 4, 11, 2);
  }

  // A rift: a swirling portal that leaps you forward. Bright, ominous rings.
  private drawRift(x: number, y: number, now: number) {
    const ctx = this.ctx;
    const p = 0.5 + 0.5 * Math.sin(now / 220);
    // Outer glow.
    ctx.globalAlpha = 0.25 + 0.2 * p;
    ctx.fillStyle = "#00e0d0";
    this.circle(x, y, 10);
    ctx.globalAlpha = 1;
    // Concentric swirling rings.
    for (let i = 0; i < 3; i++) {
      const rr = 8 - i * 2.4;
      ctx.strokeStyle = i % 2 === 0 ? "#3affe6" : "#6a3aff";
      ctx.lineWidth = 1.5;
      ctx.globalAlpha = 0.65 + 0.3 * Math.sin(now / 160 + i);
      ctx.beginPath();
      ctx.arc(x, y, rr, now / 300 + i, now / 300 + i + Math.PI * 1.5);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
    // Bright core.
    ctx.fillStyle = "#eafffb";
    ctx.fillRect(x - 1, y - 1, 2, 2);
  }

  // The cursed relic: a dark, ominous chest with a sickly violet glow.
  private drawRelic(x: number, y: number, now: number) {
    const ctx = this.ctx;
    const pulse = 0.35 + 0.35 * Math.sin(now / 240);
    ctx.globalAlpha = pulse;
    ctx.fillStyle = "#8a2be2";
    this.circle(x + 1, y, 8);
    ctx.globalAlpha = 1;
    // Dark chest body.
    ctx.fillStyle = "#241826";
    ctx.fillRect(x - 3, y - 1, 7, 5);
    ctx.fillStyle = "#3a2440";
    ctx.fillRect(x - 3, y - 3, 7, 2);
    // Violet trim + cursed lock, pulsing.
    ctx.fillStyle = (now / 300) % 2 < 1 ? "#c060ff" : "#7a2fd0";
    ctx.fillRect(x - 3, y - 1, 7, 1);
    ctx.fillRect(x, y - 1, 1, 3);
  }

  // Comet-trail sparkles behind the fast-moving cursed player.
  private cometTrail(s: Snapshot, now: number) {
    const px = s.px, py = s.py;
    const last = this.trail[this.trail.length - 1];
    if (!last || Math.hypot(px - last.x, py - last.y) > 3) {
      this.trail.push({ x: px, y: py, t: now });
      if (this.trail.length > 24) this.trail.shift();
    }
    const ctx = this.ctx;
    for (const p of this.trail) {
      const age = (now - p.t) / 650; // fade over ~0.65s
      if (age >= 1) continue;
      const sx = Math.round(p.x - s.camX);
      const sy = Math.round(p.y - s.camY);
      ctx.globalAlpha = (1 - age) * 0.7;
      ctx.fillStyle = (Math.floor(p.t / 60) & 1) ? "#c8a0ff" : "#8fd0ff";
      const sz = Math.max(1, Math.round((1 - age) * 3));
      ctx.fillRect(sx - (sz >> 1), sy - (sz >> 1), sz, sz);
    }
    ctx.globalAlpha = 1;
  }

  private drawTerrain(s: Snapshot) {
    const ctx = this.ctx;
    for (let ry = 0; ry < s.rows; ry++) {
      for (let rx = 0; rx < s.cols; rx++) {
        const id = s.tiles[ry * s.cols + rx];
        const [base, accent] = TERRAIN[id] ?? TERRAIN[3];
        const wx = (s.tx0 + rx) * TILE;
        const wy = (s.ty0 + ry) * TILE;
        const sx = Math.round(wx - s.camX);
        const sy = Math.round(wy - s.camY);
        ctx.fillStyle = base;
        ctx.fillRect(sx, sy, TILE, TILE);

        // Cheap deterministic texture: speckles on land, bands on water.
        ctx.fillStyle = accent;
        if (id <= 1) {
          if (((s.tx0 + rx + s.ty0 + ry) & 1) === 0) ctx.fillRect(sx + 2, sy + 6, 6, 2);
        } else {
          const h = hash(s.tx0 + rx, s.ty0 + ry);
          ctx.fillRect(sx + (h % 12), sy + ((h >> 4) % 12), 2, 2);
          ctx.fillRect(sx + ((h >> 8) % 12), sy + ((h >> 12) % 12), 2, 2);
          if (id === 6 || id === 7) ctx.fillRect(sx + ((h >> 16) % 10) + 2, sy + ((h >> 20) % 10) + 2, 3, 3);
        }
      }
    }
  }

  private drawMonster(x: number, y: number, r: number, shape: number, dtype: number, hpFrac: number, mega = false, hunter = false, alpha = 1, champion = false) {
    const ctx = this.ctx;
    const col = ELEM[dtype] ?? "#ccc";
    ctx.globalAlpha = alpha; // faded to invisible while lurking in cursed fog

    if (champion && !mega) {
      // A lone champion wears a regal gold aura, ring, and a tiny crown.
      const p = 0.4 + 0.3 * Math.sin(performance.now() / 150);
      ctx.globalAlpha = p * alpha;
      ctx.fillStyle = "#ffd24a";
      this.circle(x, y, r + 4);
      ctx.globalAlpha = alpha;
      ctx.strokeStyle = "#ffe98a";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(x, y, r + 3, 0, Math.PI * 2);
      ctx.stroke();
      ctx.fillStyle = "#ffd24a";
      ctx.fillRect(x - 3, y - r - 5, 6, 2); // crown band
      ctx.fillRect(x - 3, y - r - 7, 1, 2);
      ctx.fillRect(x - 1, y - r - 7, 1, 2);
      ctx.fillRect(x + 1, y - r - 7, 1, 2);
    }

    if (hunter && !mega) {
      // Relic hunters wear an ominous dark-violet aura + ring so they stand out.
      const p = 0.4 + 0.3 * Math.sin(performance.now() / 130);
      ctx.globalAlpha = p * alpha;
      ctx.fillStyle = "#7a2fd0";
      this.circle(x, y, r + 3);
      ctx.globalAlpha = alpha;
      ctx.strokeStyle = "#c060ff";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(x, y, r + 2, 0, Math.PI * 2);
      ctx.stroke();
    }

    if (mega) {
      // Pulsing menacing aura tinted by the mega's element.
      const p = 0.35 + 0.25 * Math.sin(performance.now() / 160);
      ctx.globalAlpha = p * alpha;
      ctx.fillStyle = col;
      this.circle(x, y, r + 4);
      ctx.globalAlpha = alpha;
      ctx.strokeStyle = "#ff3030";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(x, y, r + 3, 0, Math.PI * 2);
      ctx.stroke();
    }

    ctx.fillStyle = col;
    const sz = Math.max(4, r * 2);
    switch (shape) {
      case 1: // insect: diamond
        ctx.beginPath();
        ctx.moveTo(x, y - r); ctx.lineTo(x + r, y); ctx.lineTo(x, y + r); ctx.lineTo(x - r, y);
        ctx.closePath(); ctx.fill();
        break;
      case 2: // golem: blocky square with border
        ctx.fillRect(x - r, y - r, sz, sz);
        ctx.fillStyle = "#000";
        ctx.fillRect(x - r + 1, y - 1, 2, 2);
        break;
      case 3: // wisp: soft circle
        ctx.globalAlpha = 0.8 * alpha;
        this.circle(x, y, r);
        ctx.globalAlpha = alpha;
        break;
      case 4: // brute: big square
        ctx.fillRect(x - r - 1, y - r - 1, sz + 2, sz + 2);
        break;
      default: // beast: circle
        this.circle(x, y, r);
    }
    // Eyes for a bit of character.
    ctx.fillStyle = "#000";
    ctx.fillRect(x - 2, y - 1, 1, 1);
    ctx.fillRect(x + 1, y - 1, 1, 1);

    if (mega || hpFrac < 0.999) {
      const bw = mega ? sz + 4 : sz;
      const bx = x - bw / 2;
      ctx.fillStyle = "#300";
      ctx.fillRect(bx, y - r - 4, bw, mega ? 2 : 1);
      ctx.fillStyle = mega ? "#ff7a2a" : "#e04040";
      ctx.fillRect(bx, y - r - 4, Math.round(bw * hpFrac), mega ? 2 : 1);
    }
    ctx.globalAlpha = 1; // restore for later draws
  }

  private drawPlayer(x: number, y: number, aimX: number, aimY: number, s: Snapshot) {
    const ctx = this.ctx;
    // Body
    ctx.fillStyle = "#f4d8b0"; // skin/tunic
    ctx.fillRect(x - 3, y - 4, 6, 8);
    ctx.fillStyle = "#3a5bd0"; // legs
    ctx.fillRect(x - 3, y + 1, 6, 3);
    ctx.fillStyle = "#2a2a30"; // head hair
    ctx.fillRect(x - 2, y - 6, 4, 3);

    // Weapon nub pointing toward aim, tinted by equipped element.
    const len = Math.hypot(aimX, aimY) || 1;
    const ux = aimX / len, uy = aimY / len;
    const eq = s.equipped[s.slot];
    ctx.strokeStyle = eq && eq.present ? ELEM[eq.dtype] ?? "#ddd" : "#ddd";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(x + ux * 2, y + uy * 2);
    ctx.lineTo(x + ux * 8, y + uy * 8);
    ctx.stroke();

    // Player hp pip bar.
    ctx.fillStyle = "#300";
    ctx.fillRect(x - 5, y - 10, 10, 1);
    ctx.fillStyle = "#40e060";
    ctx.fillRect(x - 5, y - 10, Math.round(10 * Math.max(0, s.hp / s.maxhp)), 1);
  }

  private drawLoot(x: number, y: number, rarity: number) {
    const ctx = this.ctx;
    const blink = (performance.now() / 300) % 2 < 1;
    ctx.fillStyle = RARITY[rarity] ?? "#fff";
    ctx.fillRect(x - 2, y - 2, 4, 4);
    if (blink) {
      ctx.fillStyle = "#fff";
      ctx.fillRect(x - 1, y - 3, 2, 1);
    }
  }

  private drawAmmo(x: number, y: number) {
    // A small quiver of arrows: amber shafts.
    const ctx = this.ctx;
    ctx.fillStyle = "#e0b24a";
    ctx.fillRect(x - 2, y - 2, 1, 5);
    ctx.fillRect(x, y - 2, 1, 5);
    ctx.fillRect(x + 2, y - 2, 1, 5);
    ctx.fillStyle = "#fff2c0";
    ctx.fillRect(x - 2, y - 3, 1, 1);
    ctx.fillRect(x, y - 3, 1, 1);
    ctx.fillRect(x + 2, y - 3, 1, 1);
  }

  private drawHealth(x: number, y: number) {
    // A red cross (rare).
    const ctx = this.ctx;
    const blink = (performance.now() / 250) % 2 < 1;
    ctx.fillStyle = blink ? "#ff5a5a" : "#e03030";
    ctx.fillRect(x - 1, y - 3, 2, 6);
    ctx.fillRect(x - 3, y - 1, 6, 2);
  }

  private drawFountain(x: number, y: number, now: number) {
    const ctx = this.ctx;
    // Stone basin.
    ctx.fillStyle = "#8a8a92";
    ctx.fillRect(x - 5, y + 1, 10, 4);
    ctx.fillStyle = "#6d6d75";
    ctx.fillRect(x - 5, y + 4, 10, 2);
    // Glowing water + a red cross (health).
    ctx.fillStyle = "#4fc3ff";
    ctx.fillRect(x - 4, y + 1, 8, 2);
    // Bubbling spout.
    const b = Math.sin(now / 180) * 2;
    ctx.fillStyle = "#bfeaff";
    ctx.fillRect(x - 1, y - 4 + b, 2, 5);
    ctx.fillStyle = "#e03030";
    ctx.fillRect(x - 1, y - 6, 2, 3);
    ctx.fillRect(x - 2, y - 5, 4, 1);
  }

  private drawDot(x: number, y: number, color: string, r: number) {
    this.ctx.fillStyle = color;
    this.ctx.fillRect(x - r, y - r, r * 2, r * 2);
  }

  private confetti(now: number, count: number) {
    const ctx = this.ctx;
    const cols = ["#ff5a2a", "#66ccff", "#7ee04a", "#ffcf4a", "#c774ff", "#ffffff"];
    for (let i = 0; i < count; i++) {
      const x = (i * 71 + now * 0.04) % this.W;
      const y = (i * 53 + now * 0.09) % this.H;
      ctx.fillStyle = cols[i % cols.length];
      ctx.fillRect(x | 0, y | 0, 2, 2);
    }
  }

  private circle(x: number, y: number, r: number) {
    const ctx = this.ctx;
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fill();
  }
}
