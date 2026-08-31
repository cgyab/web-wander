// Procedural audio: synthesized combat/monster SFX plus a very subtle ambient
// music bed that shifts with biome and combat tension (calm → combat → boss →
// celebration). Everything is generated with Web Audio oscillators/noise — no
// asset files, matching the rest of the game.

type Sfx =
  | "shoot" | "swing" | "hit" | "death" | "hurt"
  | "ammo" | "health" | "item" | "chest" | "mega" | "respawn" | "cheer" | "milestone"
  | "count" | "go" | "waveclear" | "arenanear" | "relic" | "ambush"
  | "cast" | "bite" | "splash" | "snap";

// Per-biome musical character: [root Hz, scale (semitones), waveform].
const BIOME: [number, number[], OscillatorType][] = [
  [110, [0, 3, 5, 7, 10], "sine"],      // 0 deep water — low minor
  [123, [0, 3, 5, 7, 10], "sine"],      // 1 shallow water
  [146, [0, 2, 4, 7, 9], "triangle"],   // 2 sand — major pentatonic
  [164, [0, 2, 4, 7, 9], "triangle"],   // 3 grass
  [130, [0, 2, 4, 7, 9], "triangle"],   // 4 dense grass
  [110, [0, 2, 3, 7, 8], "sawtooth"],   // 5 dirt — darker
  [98, [0, 3, 5, 6, 10], "sawtooth"],   // 6 rock
  [87, [0, 3, 5, 6, 10], "sawtooth"],   // 7 mountain
  [196, [0, 2, 4, 7, 9], "sine"],       // 8 snow — airy
  [92, [0, 1, 3, 6, 8], "sawtooth"],    // 9 swamp — dissonant
];

const semi = (root: number, s: number) => root * Math.pow(2, s / 12);

// Master output level when unmuted. Doubled from the old 0.9 so the volume
// sliders default to the middle (0.5) yet stay usable, with headroom to push
// louder for quiet laptop speakers by dragging up.
const MASTER_GAIN = 1.8;

function clampVol(raw: string | null, fallback: number): number {
  const v = raw == null ? fallback : parseFloat(raw);
  return Number.isFinite(v) ? Math.max(0, Math.min(1, v)) : fallback;
}

export class AudioEngine {
  private ctx: AudioContext | null = null;
  private master!: GainNode;
  private musicGain!: GainNode;
  private sfxGain!: GainNode;
  private noise!: AudioBuffer;
  private nextNote = 0;
  private biome = 3;
  private tension = 0; // 0 calm, 1 combat, 2 boss, 3 celebration
  private muted = false;
  // User volume multipliers (0..1), persisted.
  private musicVol = clampVol(localStorage.getItem("webwander.vol.music"), 0.5);
  private sfxVol = clampVol(localStorage.getItem("webwander.vol.sfx"), 0.5);

  /** Start the audio context if it isn't already (must be from a user gesture). */
  ensureStarted() {
    this.start();
  }

  /** Must be called from a user gesture (the menu "Play" click). Also the
   *  join/resume reset: if a prior tab-hide left the context suspended or the
   *  master gain silenced, resume and restore it so sound never gets stuck
   *  "off" while the menu still reads "on". */
  start() {
    if (this.ctx) {
      if (this.ctx.state !== "running") this.ctx.resume();
      // Re-assert the master level (setActive(false) drops it to 0 on tab-hide;
      // a blocked resume can leave it there). Clear any stuck ramp first.
      const g = this.master.gain;
      g.cancelScheduledValues(this.ctx.currentTime);
      g.setValueAtTime(this.muted ? 0 : MASTER_GAIN, this.ctx.currentTime);
      return;
    }
    const ctx = new AudioContext();
    this.ctx = ctx;

    this.master = ctx.createGain();
    this.master.gain.value = this.muted ? 0 : MASTER_GAIN; // honour a pre-start mute toggle
    this.master.connect(ctx.destination);

    this.musicGain = ctx.createGain();
    this.musicGain.gain.value = 0.0001;
    this.musicGain.connect(this.master);

    this.sfxGain = ctx.createGain();
    this.sfxGain.gain.value = 0.34 * this.sfxVol;
    this.sfxGain.connect(this.master);

    // Noise buffer for percussive/whoosh SFX.
    this.noise = ctx.createBuffer(1, ctx.sampleRate * 0.4, ctx.sampleRate);
    const nd = this.noise.getChannelData(0);
    for (let i = 0; i < nd.length; i++) nd[i] = Math.random() * 2 - 1;

    // Gently fade the music bed in.
    this.musicGain.gain.linearRampToValueAtTime(0.12 * this.musicVol, ctx.currentTime + 3);
    this.nextNote = ctx.currentTime + 0.2;
    window.setInterval(() => this.schedule(), 25);
    // Some browsers create the context suspended even inside a gesture — resume.
    ctx.resume();
  }

  /** Inspect the live audio state (for tests/diagnostics). */
  state(): { ctx: string; muted: boolean; master: number } {
    return {
      ctx: this.ctx ? this.ctx.state : "none",
      muted: this.muted,
      master: this.master ? this.master.gain.value : -1,
    };
  }

  /** Pause/resume the whole engine — silences it cleanly on tab-hide / app exit
   *  so teardown doesn't produce a click, and saves CPU while backgrounded. */
  setActive(on: boolean) {
    if (!this.ctx) return;
    if (on) {
      this.ctx.resume();
      this.master.gain.setTargetAtTime(this.muted ? 0 : MASTER_GAIN, this.ctx.currentTime, 0.05);
    } else {
      this.master.gain.setValueAtTime(0, this.ctx.currentTime); // silence before suspend/teardown
      this.ctx.suspend?.();
    }
  }

  isMuted(): boolean {
    return this.muted;
  }
  toggleMute(): boolean {
    this.muted = !this.muted;
    if (this.master && this.ctx) {
      this.master.gain.setTargetAtTime(this.muted ? 0 : MASTER_GAIN, this.ctx.currentTime, 0.05);
    }
    return this.muted;
  }

  // --- user volume controls ---------------------------------------------

  getMusicVolume(): number {
    return this.musicVol;
  }
  getSfxVolume(): number {
    return this.sfxVol;
  }
  setMusicVolume(v: number) {
    this.musicVol = Math.max(0, Math.min(1, v));
    localStorage.setItem("webwander.vol.music", String(this.musicVol));
    if (this.ctx) {
      // Immediate preview level; the game loop's setEnvironment refines it.
      this.musicGain.gain.setTargetAtTime(0.14 * this.musicVol, this.ctx.currentTime, 0.08);
    }
  }
  setSfxVolume(v: number) {
    this.sfxVol = Math.max(0, Math.min(1, v));
    localStorage.setItem("webwander.vol.sfx", String(this.sfxVol));
    if (this.ctx) this.sfxGain.gain.setTargetAtTime(0.34 * this.sfxVol, this.ctx.currentTime, 0.05);
  }

  /** Play a short sample so the player can set levels in the menu. */
  previewMusic() {
    if (!this.ctx) return;
    const t = this.ctx.currentTime;
    const notes = [523, 659, 784, 1047];
    notes.forEach((f, i) => {
      const o = this.ctx!.createOscillator();
      o.type = "triangle";
      o.frequency.value = f;
      const g = this.ctx!.createGain();
      const when = t + i * 0.16;
      g.gain.setValueAtTime(0.0001, when);
      g.gain.exponentialRampToValueAtTime(0.09, when + 0.02);
      g.gain.exponentialRampToValueAtTime(0.0001, when + 0.4);
      o.connect(g);
      g.connect(this.musicGain);
      o.start(when);
      o.stop(when + 0.45);
    });
  }
  previewSfx() {
    this.sfx("shoot");
    setTimeout(() => this.sfx("hit"), 120);
    setTimeout(() => this.sfx("item"), 260);
  }

  /** Called each frame with the biome under the player and the combat tension. */
  setEnvironment(biome: number, tension: number) {
    this.biome = Math.max(0, Math.min(9, biome | 0));
    this.tension = tension;
    if (!this.ctx) return;
    const t = this.ctx.currentTime;
    // Music swells slightly with danger; brighter/higher filter under tension.
    const vol = (tension >= 3 ? 0.26 : 0.11 + tension * 0.035) * this.musicVol;
    this.musicGain.gain.setTargetAtTime(vol, t, 0.4);
  }

  // --- music scheduler ---------------------------------------------------

  private beat(): number {
    if (this.tension >= 3) return 0.26; // celebration — lively
    if (this.tension >= 2) return 0.42; // boss urgency
    if (this.tension >= 1) return 0.62; // combat
    return 1.15; // calm
  }

  private schedule() {
    const ctx = this.ctx;
    if (!ctx) return;
    while (this.nextNote < ctx.currentTime + 0.12) {
      this.playNote(this.nextNote);
      this.nextNote += this.beat();
    }
  }

  private playNote(when: number) {
    const ctx = this.ctx!;
    const [root, scale, wave] = BIOME[this.biome];
    // Rest sometimes when calm; play more when tense.
    if (this.tension < 1 && Math.random() < 0.45) return;
    const celebrate = this.tension >= 3;
    const sc = celebrate ? [0, 4, 7, 12] : scale; // major arpeggio for the party
    let s = sc[(Math.random() * sc.length) | 0];
    if (Math.random() < (celebrate ? 0.6 : 0.25)) s += 12;
    const freq = semi(celebrate ? root * 1.5 : root, s) * 2;

    const o = ctx.createOscillator();
    o.type = celebrate ? "square" : wave;
    o.frequency.value = freq;
    const g = ctx.createGain();
    const peak = (celebrate ? 0.12 : 0.075) * (0.7 + this.tension * 0.15);
    g.gain.setValueAtTime(0.0001, when);
    g.gain.exponentialRampToValueAtTime(peak, when + 0.01);
    g.gain.exponentialRampToValueAtTime(0.0001, when + (celebrate ? 0.22 : 0.5));
    const f = ctx.createBiquadFilter();
    f.type = "lowpass";
    f.frequency.value = 1600 + this.tension * 600;
    o.connect(g);
    g.connect(f);
    f.connect(this.musicGain);
    o.start(when);
    o.stop(when + 0.6);
  }

  // --- one-shot SFX ------------------------------------------------------

  sfx(kind: Sfx) {
    const ctx = this.ctx;
    if (!ctx) return;
    const t = ctx.currentTime;
    switch (kind) {
      case "shoot": this.tone(t, "triangle", 820, 260, 0.09, 0.18); break;
      case "swing": this.burst(t, 0.06, 0.14, 1400); break;
      case "hit": this.tone(t, "square", 520, 380, 0.04, 0.16); break;
      case "death": this.tone(t, "sawtooth", 300, 70, 0.26, 0.2); break;
      case "hurt": this.tone(t, "sine", 150, 55, 0.18, 0.32); this.burst(t, 0.05, 0.1, 500); break;
      case "ammo": this.tone(t, "square", 700, 700, 0.03, 0.12); this.tone(t + 0.05, "square", 900, 900, 0.03, 0.12); break;
      case "health": this.tone(t, "sine", 500, 900, 0.18, 0.16); break;
      case "item": this.tone(t, "triangle", 600, 600, 0.05, 0.14); this.tone(t + 0.07, "triangle", 800, 800, 0.06, 0.14); break;
      case "chest": this.arp(t, [523, 659, 784, 1047], 0.09, 0.16); break;
      case "mega": this.tone(t, "sawtooth", 70, 45, 0.55, 0.3); this.burst(t, 0.4, 0.14, 240); break;
      case "respawn": this.tone(t, "sine", 200, 480, 0.5, 0.14); break;
      case "cheer": this.arp(t, [523, 659, 784, 1047, 1319], 0.1, 0.22); break;
      case "milestone": this.arp(t, [659, 880, 1319], 0.08, 0.16); break;
      // Arena: countdown tick, wave-start "go", and wave-cleared chime.
      case "count": this.tone(t, "square", 640, 640, 0.1, 0.17); break;
      case "go": this.tone(t, "square", 523, 1046, 0.24, 0.24); this.burst(t, 0.12, 0.12, 1800); break;
      case "waveclear": this.tone(t, "triangle", 784, 784, 0.09, 0.18); this.tone(t + 0.1, "triangle", 1175, 1175, 0.16, 0.18); break;
      case "arenanear": this.tone(t, "sine", 330, 494, 0.22, 0.12); break; // soft "something ahead"
      case "relic": // ominous curse: a low swell + a dark shimmer
        this.tone(t, "sawtooth", 130, 70, 0.6, 0.22);
        this.tone(t + 0.06, "sine", 415, 622, 0.4, 0.14);
        this.burst(t, 0.5, 0.1, 300);
        break;
      case "ambush": // sudden startle: a rising stinger + a hiss
        this.tone(t, "sawtooth", 220, 660, 0.18, 0.24);
        this.burst(t, 0.14, 0.14, 900);
        break;
      // Fishing.
      case "cast": this.burst(t, 0.18, 0.1, 2200); break; // whoosh of the line
      case "bite": this.tone(t, "sine", 880, 620, 0.06, 0.16); this.tone(t + 0.07, "sine", 700, 520, 0.05, 0.12); break; // plip-plip
      case "splash": this.burst(t, 0.14, 0.14, 700); this.tone(t, "sine", 300, 620, 0.2, 0.14); break; // catch splash
      case "snap": this.tone(t, "square", 500, 120, 0.14, 0.14); break; // line snaps
    }
  }

  private tone(when: number, wave: OscillatorType, f0: number, f1: number, dur: number, gain: number) {
    const ctx = this.ctx!;
    const o = ctx.createOscillator();
    o.type = wave;
    o.frequency.setValueAtTime(f0, when);
    o.frequency.exponentialRampToValueAtTime(Math.max(20, f1), when + dur);
    const g = ctx.createGain();
    g.gain.setValueAtTime(0.0001, when);
    g.gain.exponentialRampToValueAtTime(gain, when + 0.008);
    g.gain.exponentialRampToValueAtTime(0.0001, when + dur);
    o.connect(g);
    g.connect(this.sfxGain);
    o.start(when);
    o.stop(when + dur + 0.02);
  }

  private burst(when: number, dur: number, gain: number, cutoff: number) {
    const ctx = this.ctx!;
    const src = ctx.createBufferSource();
    src.buffer = this.noise;
    const f = ctx.createBiquadFilter();
    f.type = "bandpass";
    f.frequency.value = cutoff;
    const g = ctx.createGain();
    g.gain.setValueAtTime(gain, when);
    g.gain.exponentialRampToValueAtTime(0.0001, when + dur);
    src.connect(f);
    f.connect(g);
    g.connect(this.sfxGain);
    src.start(when);
    src.stop(when + dur + 0.02);
  }

  private arp(when: number, freqs: number[], gain: number, spacing: number) {
    freqs.forEach((f, i) => this.tone(when + i * spacing * 0.35, "square", f, f, spacing, gain));
  }
}
