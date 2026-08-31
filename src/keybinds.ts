// User-remappable keyboard bindings + device-aware control labels. The keyboard
// path (input.ts, main.ts) resolves a pressed key to an action through here, and
// the help/menu/HUD text asks here how to describe a control on the current
// device (a key badge on desktop, an on-screen control name on touch).
//
// Aim mode (0/1/2) selects how a keyboard player aims:
//   0 — mouse aim (default): point with the pointer, hold click to fire.
//   1 — aim in the movement direction; fire with the `attack` key (default Enter).
//   2 — a right-hand aim cluster (default I/J/K/L): hold a direction to aim AND
//       fire; hold two for a 45° diagonal. Movement stays on WASD. Because aim-up
//       takes `i`, the Bag key relocates to `b` when this mode is entered.

export type Action =
  | "up" | "down" | "left" | "right"
  | "slot1" | "slot2" | "slot3" | "slot4"
  | "inventory" | "fish" | "hud"
  | "attack" | "aimup" | "aimdown" | "aimleft" | "aimright";

export type AimMode = 0 | 1 | 2;

const DEFAULTS: Record<Action, string> = {
  up: "w", left: "a", down: "s", right: "d",
  slot1: "1", slot2: "2", slot3: "3", slot4: "4",
  inventory: "i", fish: "f", hud: "h",
  attack: "enter", aimup: "i", aimleft: "j", aimdown: "k", aimright: "l",
};

// Keys owned by hardcoded handlers (mute, menu) — never bindable, so a rebind
// can't create a collision the swap logic (which only knows Actions) can't see.
const RESERVED = new Set(["m", "escape"]);

// The remap list is assembled by mode: the base rows always show; mode 1 adds the
// attack key; mode 2 adds the aim cluster.
export const REMAP_BASE: { action: Action; label: string }[] = [
  { action: "up", label: "Move up" },
  { action: "left", label: "Move left" },
  { action: "down", label: "Move down" },
  { action: "right", label: "Move right" },
  { action: "slot1", label: "Weapon 1" },
  { action: "slot2", label: "Weapon 2" },
  { action: "slot3", label: "Weapon 3" },
  { action: "slot4", label: "Weapon 4" },
  { action: "inventory", label: "Open pack" },
  { action: "fish", label: "Fish" },
  { action: "hud", label: "Toggle HUD" },
];
const REMAP_MODE1: { action: Action; label: string }[] = [
  { action: "attack", label: "Attack" },
];
const REMAP_MODE2: { action: Action; label: string }[] = [
  { action: "aimup", label: "Aim up" },
  { action: "aimleft", label: "Aim left" },
  { action: "aimdown", label: "Aim down" },
  { action: "aimright", label: "Aim right" },
];

// The remap rows for a given aim mode.
export function remapList(mode: AimMode): { action: Action; label: string }[] {
  if (mode === 1) return [...REMAP_BASE, ...REMAP_MODE1];
  if (mode === 2) return [...REMAP_BASE, ...REMAP_MODE2];
  return REMAP_BASE;
}

const STORE = "webwander.keys";
const MODE_STORE = "webwander.aimmode";
const ACTIONS = Object.keys(DEFAULTS) as Action[];
const AIM_ACTIONS: Action[] = ["aimup", "aimdown", "aimleft", "aimright"];

class KeyBinds {
  private map: Record<Action, string> = { ...DEFAULTS };
  private mode: AimMode = 0;

  constructor() {
    try {
      const raw = localStorage.getItem(STORE);
      if (raw) {
        const o = JSON.parse(raw) as Partial<Record<Action, string>>;
        for (const a of ACTIONS) {
          const v = o[a];
          if (typeof v === "string" && v) this.map[a] = v.toLowerCase();
        }
      }
    } catch { /* corrupt/no storage: keep defaults */ }
    try {
      const m = parseInt(localStorage.getItem(MODE_STORE) ?? "", 10);
      if (m === 1 || m === 2) this.mode = m;
    } catch { /* ignore */ }
    if (this.mode === 2) this.resolveAimCollisions();
  }

  private persist() {
    try { localStorage.setItem(STORE, JSON.stringify(this.map)); } catch { /* ignore */ }
  }

  get aimMode(): AimMode { return this.mode; }

  setAimMode(mode: AimMode) {
    this.mode = mode;
    if (mode === 2) this.resolveAimCollisions();
    try { localStorage.setItem(MODE_STORE, String(mode)); } catch { /* ignore */ }
    this.persist();
  }

  /** Is this action live in the current aim mode? Aim-cluster keys are inert
   *  outside mode 2, the attack key outside mode 1; everything else is always on.
   *  Routing uses this so an inactive binding never steals a key (e.g. aim-up's
   *  default `i` doesn't shadow Open-pack while the pointer aims). */
  private active(a: Action): boolean {
    if (AIM_ACTIONS.includes(a)) return this.mode === 2;
    if (a === "attack") return this.mode === 1;
    return true;
  }

  /** Entering the aim-cluster mode, `i` becomes Aim-up — so relocate any always-on
   *  action (in practice the Bag) that shares a key with the cluster to a free key,
   *  preferring `b`. Runs only for mode 2; other modes keep the classic layout. */
  private resolveAimCollisions() {
    const aimKeys = new Set(AIM_ACTIONS.map((a) => this.map[a]));
    const alwaysOn: Action[] = ["up", "down", "left", "right", "slot1", "slot2", "slot3", "slot4", "inventory", "fish", "hud"];
    const candidates = ["b", "g", "p", "v", "n", "x", "z"];
    for (const a of alwaysOn) {
      if (!aimKeys.has(this.map[a])) continue;
      const used = new Set(ACTIONS.map((x) => this.map[x]));
      const free = candidates.find((c) => !used.has(c) && !aimKeys.has(c) && !RESERVED.has(c));
      if (free) this.map[a] = free;
    }
  }

  get(a: Action): string { return this.map[a]; }

  /** The active action a pressed key maps to, or null. Case-insensitive. */
  actionFor(key: string): Action | null {
    const k = key.toLowerCase();
    for (const a of ACTIONS) if (this.active(a) && this.map[a] === k) return a;
    return null;
  }

  /** Bind `key` to `action`. If another action holds it, the two swap so no
   *  action is ever left unbound and no key is ever bound twice. */
  rebind(action: Action, key: string) {
    const k = key.toLowerCase();
    if (RESERVED.has(k)) return; // guarded again in the capture UI
    for (const other of ACTIONS) {
      if (other !== action && this.map[other] === k) this.map[other] = this.map[action];
    }
    this.map[action] = k;
    this.persist();
  }

  reset() {
    this.map = { ...DEFAULTS };
    if (this.mode === 2) this.resolveAimCollisions();
    this.persist();
  }
}

export const keybinds = new KeyBinds();

/** Is a key reserved for a hardcoded handler (mute / menu)? */
export function isReservedKey(key: string): boolean {
  return RESERVED.has(key.toLowerCase());
}

/** A key that can be bound (single character, an arrow, or Enter). */
export function isBindableKey(key: string): boolean {
  const k = key.toLowerCase();
  return key.length === 1 ||
    k === "arrowup" || k === "arrowdown" || k === "arrowleft" || k === "arrowright" ||
    k === "enter";
}

/** Human-readable label for a raw key string. */
export function keyLabel(key: string): string {
  if (!key) return "—";
  const k = key.toLowerCase();
  const named: Record<string, string> = {
    " ": "Space", enter: "Enter",
    arrowup: "↑", arrowdown: "↓", arrowleft: "←", arrowright: "→",
  };
  if (named[k]) return named[k];
  return key.length === 1 ? key.toUpperCase() : key;
}

/** The bound key for an action, ready to display. */
export function keyName(a: Action): string { return keyLabel(keybinds.get(a)); }

export interface ControlNames {
  move: string; aim: string; attack: string; weapons: string;
  pack: string; menu: string; hud: string; fish: string; mute: string;
}

/** How to describe each control on the current device — a key badge on
 *  desktop, or the on-screen control's name on touch. Values are HTML. */
export function controls(isTouch: boolean): ControlNames {
  const badge = (t: string) => `<span class="k">${t}</span>`;
  if (isTouch) {
    return {
      move: "the left stick",
      aim: "the right stick",
      attack: "hold the right stick",
      weapons: "the weapon buttons",
      pack: badge("Bag"),
      menu: badge("Menu"),
      hud: badge("HUD"),
      fish: badge("Fish"),
      mute: badge("Sound"),
    };
  }
  const key = (a: Action) => badge(keyName(a));
  const mode = keybinds.aimMode;
  const aimCluster = badge(keyName("aimup") + keyName("aimleft") + keyName("aimdown") + keyName("aimright"));
  const aim = mode === 2 ? aimCluster
    : mode === 1 ? "your movement direction"
    : "the mouse";
  const attack = mode === 2 ? "hold an aim key"
    : mode === 1 ? key("attack")
    : `${badge("click")}/hold`;
  return {
    move: `${badge(keyName("up") + keyName("left") + keyName("down") + keyName("right"))}/arrows`,
    aim,
    attack,
    weapons: `${key("slot1")}-${key("slot4")} or the scroll wheel`,
    pack: key("inventory"),
    menu: badge("Esc"),
    hud: key("hud"),
    fish: key("fish"),
    mute: badge("M"),
  };
}
