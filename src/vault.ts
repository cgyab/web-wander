// A light "Simon-says" rune puzzle shown in a translucent panel. The world is
// PAUSED while it's open (main.ts gates the sim on `vaultActive`), so the solve
// is a calm brain-break — no monster can attack or interrupt you mid-pattern.
// Input never times out: the puzzle waits for you, round by round. The vault
// flashes a growing sequence of runes; repeat it by clicking/tapping. Complete
// the target-length round to crack it. Resolves via onDone(solved).

type Done = (solved: boolean) => void;

const RUNES = ["✦", "◆", "●", "▲", "■", "✚"]; // 6 widely-supported glyphs
const COLORS = ["#ff6b6b", "#ffd24a", "#7ee078", "#68e0ff", "#c78cff", "#ff9de0"];

function clamp(v: number, a: number, b: number): number {
  return Math.max(a, Math.min(b, v));
}

export class Vault {
  private el: HTMLDivElement;
  private grid: HTMLDivElement;
  private status: HTMLDivElement;
  private prog: HTMLDivElement;
  private btns: HTMLDivElement[] = [];
  private onDone: Done | null = null;

  private seq: number[] = [];
  private input: number[] = [];
  private target = 4;
  private phase: "show" | "input" | "wait" = "show";
  // Pending transitions, driven by tick() from the main animation frame — NOT
  // setTimeout, which mobile Chrome throttles/drops and could leave the puzzle
  // stalled (panel up, input dead). See schedule()/tick().
  private sched: { at: number; fn: () => void }[] = [];

  constructor() {
    this.el = document.createElement("div");
    this.el.id = "vault";
    this.el.className = "hidden";
    const panel = document.createElement("div");
    panel.className = "vpanel";
    const h = document.createElement("div");
    h.className = "vtitle";
    h.textContent = "Rune Vault";
    this.prog = document.createElement("div");
    this.prog.className = "vprog";
    this.grid = document.createElement("div");
    this.grid.className = "vgrid";
    for (let i = 0; i < RUNES.length; i++) {
      const b = document.createElement("div");
      b.className = "vrune";
      b.textContent = RUNES[i];
      b.style.color = COLORS[i];
      b.style.setProperty("--lit", COLORS[i]);
      b.addEventListener("pointerdown", (e) => { e.preventDefault(); this.clickRune(i); });
      this.grid.appendChild(b);
      this.btns.push(b);
    }
    this.status = document.createElement("div");
    this.status.className = "vhint";
    const leave = document.createElement("button");
    leave.className = "vleave";
    leave.textContent = "Leave (Esc)";
    leave.addEventListener("pointerdown", (e) => { e.preventDefault(); this.finish(false); });
    panel.append(h, this.prog, this.grid, this.status, leave);
    this.el.appendChild(panel);
    document.body.appendChild(this.el);
  }

  isOpen(): boolean {
    return this.onDone !== null;
  }

  /** Force-close (e.g. the player died mid-puzzle) — counts as a bail. */
  close() {
    if (this.onDone) this.finish(false);
  }

  open(target: number, onDone: Done) {
    if (this.onDone) return;
    this.onDone = onDone;
    this.target = clamp(Math.round(target), 3, 6);
    this.seq = [];
    this.input = [];
    for (const b of this.btns) b.classList.remove("lit"); // clear any stale glow
    this.el.classList.remove("hidden");
    window.addEventListener("keydown", this.onKey, true);
    this.nextRound();
  }

  private clearTimers() {
    this.sched = [];
  }

  /** Queue a transition `delay` ms out. Fired by tick() (the animation frame),
   *  so it survives mobile timer throttling that could drop a setTimeout. */
  private schedule(delay: number, fn: () => void) {
    this.sched.push({ at: performance.now() + delay, fn });
  }

  /** Advance any due transitions. The main loop calls this every animation frame
   *  (even while the world is paused). Self-healing: after a background/foreground
   *  the overdue steps simply fire on the next frame, in order — the puzzle can
   *  never get permanently stuck waiting on a dropped timer. */
  tick(now: number) {
    if (!this.onDone) return;
    // Fire due events earliest-first; re-scan each pass since a fired step may
    // queue more work or finish the puzzle outright.
    for (;;) {
      let idx = -1;
      let best = Infinity;
      for (let i = 0; i < this.sched.length; i++) {
        const at = this.sched[i].at;
        if (at <= now && at < best) { best = at; idx = i; }
      }
      if (idx === -1) break;
      const ev = this.sched.splice(idx, 1)[0];
      ev.fn();
      if (!this.onDone) break; // finished mid-tick — stop touching state
    }
  }

  private finish(solved: boolean) {
    if (!this.onDone) return;
    const cb = this.onDone;
    this.onDone = null;
    this.clearTimers();
    window.removeEventListener("keydown", this.onKey, true);
    this.el.classList.add("hidden");
    cb(solved);
  }

  private onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault(); e.stopPropagation();
      this.finish(false);
    } else if (/^[1-6]$/.test(e.key)) {
      e.preventDefault(); e.stopPropagation();
      this.clickRune(parseInt(e.key, 10) - 1);
    }
  };

  private nextRound() {
    this.seq.push(Math.floor(Math.random() * RUNES.length));
    this.input = [];
    this.showSequence();
  }

  private showSequence() {
    this.clearTimers();
    this.phase = "show";
    for (const b of this.btns) b.classList.remove("lit"); // no glow carries over
    this.prog.textContent = `Rune ${this.seq.length} / ${this.target}`;
    this.status.textContent = "Watch the runes…";
    let t = 450;
    for (const r of this.seq) {
      this.schedule(t, () => this.flash(r));
      t += 560;
    }
    this.schedule(t, () => {
      this.phase = "input";
      this.status.textContent = "Repeat the sequence — click or 1-6";
      // Test hook: lets tooling read the expected sequence (like window.__ww).
      (window as unknown as Record<string, unknown>).__vaultDbg = { phase: this.phase, seq: [...this.seq] };
    });
  }

  private flash(i: number) {
    const b = this.btns[i];
    b.classList.add("lit");
    this.schedule(320, () => b.classList.remove("lit"));
  }

  private clickRune(i: number) {
    if (this.phase !== "input") return;
    this.flash(i);
    const pos = this.input.length;
    if (i !== this.seq[pos]) {
      // One wrong rune fails the whole vault — no second chances.
      this.phase = "wait";
      this.status.textContent = "Wrong rune — the vault seals shut.";
      this.schedule(800, () => this.finish(false));
      return;
    }
    this.input.push(i);
    if (this.input.length === this.seq.length) {
      if (this.seq.length >= this.target) {
        this.phase = "wait";
        this.status.textContent = "The vault yields!";
        this.schedule(450, () => this.finish(true));
      } else {
        this.phase = "wait";
        this.status.textContent = "Correct—";
        this.schedule(550, () => this.nextRound());
      }
    }
  }
}
