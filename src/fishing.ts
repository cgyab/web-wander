// A simple, one-input fishing mini-game shown as a pop-up while the world is
// paused. It's all click / tap (mouse, touch, or Space):
//   CAST : hold to cast, release to let it fly. The distance (and so the fish
//          you might land) is chosen at random, weighted by the water.
//   HOOK : watch the float — tap the moment it bobs to set the hook.
//   REEL : a queue of markers scrolls into a hit line; tap in time with each.
//          Stay close enough and you land the fish.
// Resolves with `quality` in 0..1 (landed), -1 (escaped, bait lost) or -2
// (cancelled). The caller applies it via the wasm `fish` export.

import { TouchControls } from "./touch";

type Sfx = (name: string) => void;

const W = 256;
const H = 160;
const WATER_TOP = 92;
const SHORE_X = 66;

// Reel rhythm tuning.
const HIT_X = 74;          // where markers should be tapped
const SCROLL = 118;        // px / second the queue scrolls
const PERFECT = 0.09;      // |timing error| for a perfect (seconds)
const GOOD = 0.2;          // ...for a good
const NOTE_GAP_MIN = 0.44; // random spacing between markers (seconds)...
const NOTE_GAP_MAX = 0.88; // ...so every reel has a different rhythm

function clamp(v: number, a: number, b: number): number {
  return Math.max(a, Math.min(b, v));
}

interface Note { at: number; judged: boolean; result: "perfect" | "good" | "miss" | null; }

export class Fishing {
  private el: HTMLDivElement;
  private cv: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private status: HTMLDivElement;
  private raf = 0;
  private last = 0;
  private onDone: ((q: number) => void) | null = null;
  private touch = TouchControls.isTouch();

  private phase: "cast" | "hook" | "reel" = "cast";
  private pt = 0;   // phase timer
  private t = 0;    // running clock (animation)

  // Cast.
  private holding = false;
  private power = 0;      // grows while held; controls how far it flies
  private castTier = 0;   // 0..2, farther/deeper = better fish + a longer reel
  private castDist = 0.5; // 0..1 landing reach (from power)
  private flying = false; // the lure is mid-arc after release
  private flyT = 0;
  private flyDur = 0.6;
  // Read-the-water: random spots seeded per cast along the distance axis.
  private zones: { kind: "prize" | "good" | "snag"; at: number }[] = [];
  private snagged = false;
  private foul = 0; // countdown showing a fouled line before it resolves

  // Hook.
  private bobAt = 2;
  private bobbing = false;
  private misses = 0;      // missed bobs
  private early = 0;       // jumped-the-gun taps
  private flash = 0;       // brief feedback pulse

  // Reel.
  private notes: Note[] = [];
  private perfects = 0;
  private goods = 0;
  private missed = 0;
  private allowed = 3;
  private judgeFlash: { t: number; kind: string } | null = null;

  constructor(private sfx: Sfx) {
    this.el = document.createElement("div");
    this.el.id = "fishing";
    this.el.className = "hidden";
    this.cv = document.createElement("canvas");
    this.cv.width = W;
    this.cv.height = H;
    this.el.appendChild(this.cv);
    this.status = document.createElement("div");
    this.status.className = "fhint";
    this.el.appendChild(this.status);
    document.body.appendChild(this.el);
    this.ctx = this.cv.getContext("2d", { alpha: false })!;
    this.ctx.imageSmoothingEnabled = false;
  }

  isOpen(): boolean {
    return this.onDone !== null;
  }

  open(onDone: (q: number) => void) {
    if (this.onDone) return;
    this.onDone = onDone;
    this.touch = TouchControls.isTouch();
    this.phase = "cast";
    this.pt = 0; this.t = 0;
    this.holding = false; this.power = 0;
    this.castTier = 0; this.castDist = 0.5;
    this.flying = false; this.flyT = 0;
    this.snagged = false; this.foul = 0;
    this.seedZones();
    this.bobbing = false; this.misses = 0; this.early = 0; this.flash = 0;
    this.notes = []; this.perfects = this.goods = this.missed = 0;
    this.judgeFlash = null;
    this.el.classList.remove("hidden");
    window.addEventListener("keydown", this.onKey, true);
    window.addEventListener("keyup", this.onKeyUp, true);
    window.addEventListener("mousedown", this.onDown, true);
    window.addEventListener("mouseup", this.onUp, true);
    this.el.addEventListener("touchstart", this.onTouchStart, { passive: false });
    this.el.addEventListener("touchend", this.onTouchEnd, { passive: false });
    this.last = performance.now();
    this.raf = requestAnimationFrame(this.frame);
  }

  private finish(q: number) {
    if (!this.onDone) return;
    const cb = this.onDone;
    this.onDone = null;
    cancelAnimationFrame(this.raf);
    window.removeEventListener("keydown", this.onKey, true);
    window.removeEventListener("keyup", this.onKeyUp, true);
    window.removeEventListener("mousedown", this.onDown, true);
    window.removeEventListener("mouseup", this.onUp, true);
    this.el.removeEventListener("touchstart", this.onTouchStart);
    this.el.removeEventListener("touchend", this.onTouchEnd);
    this.holding = false;
    this.el.classList.add("hidden");
    cb(q);
  }

  // --- input: one action ("press"/"release"), from mouse, touch, or Space ---
  private onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault(); e.stopPropagation();
      this.finish(this.phase === "cast" ? -2 : -1);
      return;
    }
    if (e.key === " " || e.key === "f" || e.key === "F" || e.key === "Enter") {
      e.preventDefault(); e.stopPropagation();
      if (!e.repeat) this.press();
    }
  };
  private onKeyUp = (e: KeyboardEvent) => {
    if (e.key === " " || e.key === "f" || e.key === "F" || e.key === "Enter") this.release();
  };
  private onDown = (e: MouseEvent) => { if (e.button === 0) { e.preventDefault(); this.press(); } };
  private onUp = (e: MouseEvent) => { if (e.button === 0) this.release(); };
  private onTouchStart = (e: TouchEvent) => { e.preventDefault(); this.press(); };
  private onTouchEnd = (e: TouchEvent) => { e.preventDefault(); this.release(); };

  private press() {
    if (this.phase === "cast") this.holding = true;
    else this.tap();
  }
  private release() {
    if (this.phase === "cast" && this.holding) { this.holding = false; this.doCast(); }
  }

  // Re-roll the water: a prize (deep-ish), a calm "good" spot, and 1-2 snags —
  // one lurking near the prize so reaching for the big fish carries real risk.
  private seedZones() {
    const prize = 0.55 + Math.random() * 0.35;
    const s1 = clamp(prize + (Math.random() < 0.5 ? -1 : 1) * (0.1 + Math.random() * 0.12), 0.2, 0.97);
    const good = 0.22 + Math.random() * 0.22;
    const zones: { kind: "prize" | "good" | "snag"; at: number }[] = [
      { kind: "prize", at: prize },
      { kind: "snag", at: s1 },
      { kind: "good", at: good },
    ];
    if (Math.random() < 0.4) zones.push({ kind: "snag", at: clamp(good + 0.14 + Math.random() * 0.3, 0.2, 0.97) });
    this.zones = zones;
  }

  private landingOutcome(d: number): "prize" | "good" | "snag" | "open" {
    for (const z of this.zones) if (z.kind === "snag" && Math.abs(z.at - d) < 0.055) return "snag";
    for (const z of this.zones) if (z.kind === "prize" && Math.abs(z.at - d) < 0.07) return "prize";
    for (const z of this.zones) if (z.kind === "good" && Math.abs(z.at - d) < 0.085) return "good";
    return "open";
  }

  private doCast() {
    // How far it flies is set by how long you held — a clear, learnable arc.
    this.castDist = clamp(0.16 + this.power * 0.8, 0, 1);
    // Where it lands decides the payoff: prize/good/open fish, or a snag.
    const out = this.landingOutcome(this.castDist);
    this.snagged = out === "snag";
    this.castTier = out === "prize" ? 2 : out === "good" ? 1 : 0;
    this.sfx("cast");
    this.flying = true; this.flyT = 0;
    this.flyDur = 0.4 + this.castDist * 0.4; // farther = longer to splash down
  }

  private tap() {
    if (this.phase === "hook") {
      if (this.bobbing) {
        this.sfx("bite");
        this.startReel();
      } else {
        // Jumped the gun — spook it a touch and push the bob back.
        this.early++; this.flash = 0.3;
        this.bobAt += 0.5;
        if (this.early >= 4) { this.sfx("snap"); this.finish(-1); }
      }
    } else if (this.phase === "reel") {
      this.judgeTap();
    }
  }

  private startReel() {
    this.phase = "reel"; this.pt = 0;
    const n = 7 + this.castTier; // 7..9 markers
    this.notes = [];
    let at = 1.0;
    for (let i = 0; i < n; i++) {
      this.notes.push({ at, judged: false, result: null });
      at += NOTE_GAP_MIN + Math.random() * (NOTE_GAP_MAX - NOTE_GAP_MIN); // random rhythm
    }
    this.perfects = this.goods = this.missed = 0;
    this.allowed = Math.ceil(n * 0.4);
    this.judgeFlash = null;
  }

  private judgeTap() {
    // Judge the nearest un-judged marker.
    let best: Note | null = null;
    let bestErr = Infinity;
    for (const nt of this.notes) {
      if (nt.judged) continue;
      const err = Math.abs(nt.at - this.pt);
      if (err < bestErr) { bestErr = err; best = nt; }
    }
    if (!best || bestErr > GOOD * 1.6) { // a stray tap
      this.judgeFlash = { t: 0.25, kind: "miss" };
      return;
    }
    best.judged = true;
    if (bestErr < PERFECT) { best.result = "perfect"; this.perfects++; this.judgeFlash = { t: 0.25, kind: "perfect" }; this.sfx("splash"); }
    else { best.result = "good"; this.goods++; this.judgeFlash = { t: 0.25, kind: "good" }; this.sfx("bite"); }
    this.checkReelEnd();
  }

  private checkReelEnd() {
    if (this.missed > this.allowed) { this.sfx("snap"); this.finish(-1); return; }
    if (this.notes.every((n) => n.judged)) {
      if (this.missed > this.allowed) { this.sfx("snap"); this.finish(-1); return; }
      const score = (this.perfects + this.goods * 0.6) / this.notes.length;
      const q = clamp(0.3 + score * 0.55 + this.castTier * 0.07, 0.15, 1);
      this.sfx("splash");
      this.finish(q);
    }
  }

  private frame = (now: number) => {
    const dt = Math.min(0.05, (now - this.last) / 1000);
    this.last = now;
    this.t += dt; this.pt += dt;
    if (this.flash > 0) this.flash = Math.max(0, this.flash - dt);
    if (this.judgeFlash) { this.judgeFlash.t -= dt; if (this.judgeFlash.t <= 0) this.judgeFlash = null; }

    if (this.phase === "cast") {
      if (this.foul > 0) {
        this.foul -= dt;
        if (this.foul <= 0) { this.finish(-1); return; } // snag: bait lost, no reel
      } else if (this.flying) {
        this.flyT += dt;
        if (this.flyT >= this.flyDur) {
          this.flying = false;
          if (this.snagged) { this.foul = 0.85; this.sfx("snap"); }
          else {
            this.sfx("splash");
            this.phase = "hook"; this.pt = 0;
            this.bobAt = 1.2 + Math.random() * 1.6;
            this.bobbing = false; this.misses = 0; this.early = 0;
          }
        }
      } else if (this.holding) {
        this.power = clamp(this.power + dt * 0.82, 0, 1); // ~1.2s to a full cast
      }
    } else if (this.phase === "hook") {
      const inWindow = this.pt >= this.bobAt && this.pt < this.bobAt + 0.62;
      this.bobbing = inWindow;
      if (this.pt >= this.bobAt + 0.62) {
        this.misses++;
        this.flash = 0.3;
        this.bobAt = this.pt + 0.9 + Math.random() * 1.3;
        this.bobbing = false;
        if (this.misses >= 3) { this.sfx("snap"); this.finish(-1); return; }
      }
    } else {
      // Auto-miss markers that scroll past the line untapped.
      for (const nt of this.notes) {
        if (!nt.judged && this.pt > nt.at + GOOD) {
          nt.judged = true; nt.result = "miss"; this.missed++;
          this.judgeFlash = { t: 0.2, kind: "miss" };
        }
      }
      this.checkReelEnd();
    }

    this.render();
    if (this.onDone) this.raf = requestAnimationFrame(this.frame);
  };

  // --- rendering ---
  private render() {
    const ctx = this.ctx;
    const sky = ctx.createLinearGradient(0, 0, 0, WATER_TOP);
    sky.addColorStop(0, "#2a3a6a");
    sky.addColorStop(1, "#6a86c0");
    ctx.fillStyle = sky;
    ctx.fillRect(0, 0, W, WATER_TOP);
    ctx.fillStyle = "#25507a";
    ctx.fillRect(0, WATER_TOP, W, H - WATER_TOP);
    ctx.fillStyle = "#2e6796";
    for (let i = 0; i < 5; i++) ctx.fillRect(0, WATER_TOP + 8 + i * 12, W, 2);
    ctx.fillStyle = "#3a6b2f";
    ctx.fillRect(0, 78, SHORE_X, H - 78);
    ctx.fillStyle = "#caa96a";
    ctx.fillRect(SHORE_X - 8, 88, 12, H - 88);

    const rod = this.drawAngler();
    if (this.phase === "cast") this.drawCast(rod);
    else if (this.phase === "hook") this.drawHook(rod);
    else this.drawReel(rod);

    this.status.textContent = this.hint();
    // Lets tooling inspect/drive the mini-game (harmless, like window.__ww).
    (window as unknown as Record<string, unknown>).__fishDbg = {
      phase: this.phase, bobbing: this.bobbing, pt: this.pt, castTier: this.castTier,
      zones: this.zones.map((z) => ({ kind: z.kind, at: z.at })),
      previewDist: this.phase === "cast" && !this.flying && this.foul <= 0 ? clamp(0.16 + this.power * 0.8, 0, 1) : null,
      notes: this.notes.map((n) => ({ at: n.at, judged: n.judged })),
    };
  }

  // Angler matched to the in-game sprite: dark hair, tan tunic, blue legs.
  private drawAngler(): { x: number; y: number } {
    const ctx = this.ctx;
    const ax = 22, feet = 98;
    ctx.fillStyle = "#3a5bd0";
    ctx.fillRect(ax, feet - 8, 9, 8);
    ctx.fillStyle = "#f4d8b0";
    ctx.fillRect(ax, feet - 22, 9, 14);
    ctx.fillStyle = "#2a2a30";
    ctx.fillRect(ax - 1, feet - 28, 11, 6);
    ctx.fillStyle = "#f4d8b0";
    ctx.fillRect(ax + 8, feet - 20, 4, 4);
    const bend = this.phase === "reel" && this.judgeFlash?.kind === "perfect" ? 3 : this.phase === "hook" && this.bobbing ? 4 : 0;
    const gripX = ax + 11, gripY = feet - 20;
    const tipX = gripX + 20, tipY = gripY - 22 + bend;
    ctx.strokeStyle = "#6d5330";
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(gripX, gripY);
    ctx.quadraticCurveTo(gripX + 12, gripY - 16, tipX, tipY);
    ctx.stroke();
    return { x: tipX, y: tipY };
  }

  private reach(dist: number): { x: number; y: number } {
    const x = SHORE_X + 26 + dist * 158;
    const y = WATER_TOP + 12 + dist * 30; // farther = deeper (lower on screen)
    return { x: clamp(x, SHORE_X + 20, 236), y: clamp(y, WATER_TOP + 8, H - 12) };
  }
  private lurePos(): { x: number; y: number } {
    return this.reach(this.castDist);
  }

  private line(rod: { x: number; y: number }, x: number, y: number, color: string) {
    const ctx = this.ctx;
    ctx.strokeStyle = color;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(rod.x, rod.y);
    ctx.lineTo(x, y);
    ctx.stroke();
  }

  private drawZones() {
    const ctx = this.ctx;
    for (const z of this.zones) {
      const p = this.reach(z.at);
      if (z.kind === "prize") { // gold star
        ctx.fillStyle = "#ffd24a";
        ctx.fillRect(p.x - 1, p.y - 4, 2, 8);
        ctx.fillRect(p.x - 4, p.y - 1, 8, 2);
        ctx.fillStyle = "#fff6c0";
        ctx.fillRect(p.x - 1, p.y - 1, 2, 2);
      } else if (z.kind === "good") { // green ring
        ctx.strokeStyle = "#7ee078";
        ctx.lineWidth = 1;
        ctx.beginPath(); ctx.arc(p.x, p.y, 3, 0, Math.PI * 2); ctx.stroke();
      } else { // red X snag
        ctx.strokeStyle = "#ff5a5a";
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.moveTo(p.x - 3, p.y - 3); ctx.lineTo(p.x + 3, p.y + 3);
        ctx.moveTo(p.x + 3, p.y - 3); ctx.lineTo(p.x - 3, p.y + 3);
        ctx.stroke();
      }
    }
  }

  private drawCast(rod: { x: number; y: number }) {
    const ctx = this.ctx;
    this.drawZones(); // the water's prize / good / snag spots
    if (this.foul > 0) {
      const l = this.lurePos();
      this.line(rod, l.x, l.y, "#ff6a5a");
      ctx.fillStyle = "#ff5a5a";
      ctx.font = "bold 10px monospace";
      ctx.fillText("SNAGGED!", l.x - 20, l.y - 8);
      return;
    }
    if (this.flying) {
      // The lure arcs from the rod tip to its landing spot, rising then dropping.
      const u = clamp(this.flyT / this.flyDur, 0, 1);
      const land = this.lurePos();
      const x = rod.x + (land.x - rod.x) * u;
      const arc = 52 * this.castDist * Math.sin(Math.PI * u); // up then down
      const y = rod.y + (land.y - rod.y) * u - arc;
      this.line(rod, x, y, "rgba(230,240,255,0.6)");
      ctx.fillStyle = "#e23b1e";
      ctx.fillRect(x - 2, y - 2, 4, 4);
      return;
    }
    // While charging: preview the landing spot and the arc it'll take, so the
    // power→distance→drop relationship is visible.
    const pd = clamp(0.16 + this.power * 0.8, 0, 1);
    const land = this.reach(pd);
    ctx.strokeStyle = "rgba(255,240,180,0.55)";
    ctx.setLineDash([2, 3]);
    ctx.beginPath();
    for (let s = 0; s <= 1.0001; s += 0.08) {
      const ax = rod.x + (land.x - rod.x) * s;
      const ay = rod.y + (land.y - rod.y) * s - 52 * pd * Math.sin(Math.PI * s);
      if (s === 0) ctx.moveTo(ax, ay); else ctx.lineTo(ax, ay);
    }
    ctx.stroke();
    ctx.setLineDash([]);
    // Target ring where it will drop.
    ctx.strokeStyle = "rgba(255,240,180,0.85)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.arc(land.x, land.y, 4, 0, Math.PI * 2);
    ctx.stroke();
    // Power bar.
    ctx.fillStyle = "rgba(10,14,26,0.7)";
    ctx.fillRect(6, 120, 44, 8);
    ctx.fillStyle = `rgb(${Math.round(120 + this.power * 120)},${Math.round(210 - this.power * 90)},80)`;
    ctx.fillRect(7, 121, Math.round(42 * this.power), 6);
  }

  private drawHook(rod: { x: number; y: number }) {
    const ctx = this.ctx;
    const l = this.lurePos();
    // The bobber dips and ripples during the strike window.
    const dip = this.bobbing ? 4 + Math.sin(this.t * 30) * 1 : Math.sin(this.t * 3) * 1;
    this.line(rod, l.x, l.y + dip, this.bobbing ? "#ffffff" : "rgba(230,240,255,0.55)");
    if (this.bobbing) {
      ctx.strokeStyle = "rgba(255,255,255,0.6)";
      ctx.lineWidth = 1;
      const rr = 4 + ((this.t * 20) % 8);
      ctx.beginPath(); ctx.arc(l.x, l.y + 3, rr, 0, Math.PI * 2); ctx.stroke();
    }
    ctx.fillStyle = "#e23b1e";
    ctx.fillRect(l.x - 2, l.y - 2 + dip, 4, 3);
    ctx.fillStyle = "#f0f0f0";
    ctx.fillRect(l.x - 2, l.y + 1 + dip, 4, 1);
    if (this.bobbing) {
      ctx.fillStyle = "#ffe14a";
      ctx.font = "bold 12px monospace";
      ctx.fillText("!", l.x - 2, l.y - 8);
    }
    if (this.flash > 0) {
      ctx.fillStyle = `rgba(255,90,90,${this.flash})`;
      ctx.fillRect(0, 0, W, 4);
    }
  }

  private drawReel(rod: { x: number; y: number }) {
    const ctx = this.ctx;
    // Fish being pulled toward shore as you land markers.
    const done = this.notes.filter((n) => n.judged).length;
    const prog = done / this.notes.length;
    const fx = 214 - prog * 130;
    const fy = WATER_TOP + 26 + Math.sin(this.t * 3) * 3;
    this.line(rod, fx, fy, this.judgeFlash?.kind === "miss" ? "#ff6a5a" : "#8fe088");
    ctx.fillStyle = "rgba(10,20,30,0.62)";
    ctx.fillRect(fx - 5, fy - 2, 10, 4);
    ctx.fillRect(fx + 4, fy - 3, 3, 6);

    // Rhythm track (in the sky band).
    const trackY = 34;
    ctx.fillStyle = "rgba(10,14,26,0.72)";
    ctx.fillRect(0, trackY - 9, W, 18);
    // Hit line + tolerance band.
    ctx.fillStyle = "rgba(126,224,120,0.18)";
    ctx.fillRect(HIT_X - GOOD * SCROLL, trackY - 8, GOOD * SCROLL * 2, 16);
    ctx.fillStyle = "#ffd86b";
    ctx.fillRect(HIT_X - 1, trackY - 8, 2, 16);
    // Markers scrolling right -> left toward the line.
    for (const nt of this.notes) {
      const x = HIT_X + (nt.at - this.pt) * SCROLL;
      if (x < -6 || x > W + 6) continue;
      let color = "#cfe0ff";
      if (nt.judged) color = nt.result === "perfect" ? "#7ee078" : nt.result === "good" ? "#f0d24a" : "#ff5a5a";
      ctx.fillStyle = color;
      ctx.fillRect(x - 3, trackY - 4, 6, 8);
    }
    // Judgement pop.
    if (this.judgeFlash) {
      const c = this.judgeFlash.kind === "perfect" ? "#7ee078" : this.judgeFlash.kind === "good" ? "#f0d24a" : "#ff5a5a";
      ctx.fillStyle = c;
      ctx.font = "7px monospace";
      ctx.fillText(this.judgeFlash.kind.toUpperCase(), HIT_X - 12, trackY + 20);
    }
    // Miss budget.
    ctx.fillStyle = "#c8c8d8";
    ctx.font = "6px monospace";
    ctx.fillText(`slips ${this.missed}/${this.allowed}`, 4, 12);
  }

  private hint(): string {
    const tapWord = this.touch ? "Tap" : "Click";
    if (this.phase === "cast") {
      if (this.foul > 0) return "Snagged — the line fouled, bait lost.";
      if (this.flying) return "Casting…";
      const verb = this.touch ? "Touch & hold" : "Click & hold";
      return `${verb} to reach the ★ prize — avoid the ✕ snags — release to cast`;
    }
    if (this.phase === "hook") {
      return `Watch the float — ${tapWord.toLowerCase()} the moment it bobs!`;
    }
    return `${tapWord} in time as each marker hits the line — keep the fish coming!`;
  }
}
