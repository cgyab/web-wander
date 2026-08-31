// Keyboard + mouse capture. Produces the movement bitmask and mouse position in
// logical (320x180) pixels; main.ts turns that into an aim vector. In keyboard
// aim modes it also drives aim/attack directly (see keybinds.ts AimMode).

import { keybinds } from "./keybinds";

const K_UP = 1, K_DOWN = 2, K_LEFT = 4, K_RIGHT = 8;

function bitmaskVec(bits: number): [number, number] {
  let x = 0, y = 0;
  if (bits & K_LEFT) x -= 1;
  if (bits & K_RIGHT) x += 1;
  if (bits & K_UP) y -= 1;   // screen coords: up is -y
  if (bits & K_DOWN) y += 1;
  const l = Math.hypot(x, y);
  return l > 0.001 ? [x / l, y / l] : [x, y];
}

export class Input {
  keys = 0;
  mouseX = 160;
  mouseY = 90;
  attack = false;
  slot = 0;
  // Direct aim direction (used by touch and by the mode-2 aim cluster; overrides
  // mouse-derived aim).
  aimDX = 1;
  aimDY = 0;
  aimActive = false;
  private inventoryToggled = false;
  // Attack has three possible sources; `attack` is their OR (see refreshAttack).
  private mouseAtk = false; // left mouse button (mouse aim)
  private keyAtk = false;   // the `attack` key (mode 1)
  private aimKeys = 0;      // held aim-cluster keys (mode 2) — also fires while > 0
  // client -> logical transform (set by main on resize)
  private scale = 1;
  private offX = 0;
  private offY = 0;

  attach(canvas: HTMLCanvasElement) {
    const down = (e: KeyboardEvent) => this.key(e, true);
    const up = (e: KeyboardEvent) => this.key(e, false);
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);

    canvas.addEventListener("mousemove", (e) => {
      this.mouseX = (e.clientX - this.offX) / this.scale;
      this.mouseY = (e.clientY - this.offY) / this.scale;
    });
    canvas.addEventListener("mousedown", (e) => {
      if (e.button === 0) { this.mouseAtk = true; this.refreshAttack(); }
    });
    window.addEventListener("mouseup", (e) => {
      if (e.button === 0) { this.mouseAtk = false; this.refreshAttack(); }
    });
    canvas.addEventListener("contextmenu", (e) => e.preventDefault());
  }

  setViewport(scale: number, offX: number, offY: number) {
    this.scale = scale;
    this.offX = offX;
    this.offY = offY;
  }

  /** Returns true once after each pack-open press. */
  consumeInventoryToggle(): boolean {
    const v = this.inventoryToggled;
    this.inventoryToggled = false;
    return v;
  }

  private refreshAttack() {
    this.attack = this.mouseAtk || this.keyAtk || this.aimKeys !== 0;
  }

  private updateAim() {
    if (this.aimKeys === 0) {
      this.aimActive = false;
    } else {
      const [x, y] = bitmaskVec(this.aimKeys);
      this.aimDX = x;
      this.aimDY = y;
      this.aimActive = true;
    }
    this.refreshAttack();
  }

  private key(e: KeyboardEvent, pressed: boolean) {
    const k = e.key.toLowerCase();
    const setMove = (bit: number) => {
      if (pressed) this.keys |= bit;
      else this.keys &= ~bit;
    };
    const setAim = (bit: number) => {
      if (pressed) this.aimKeys |= bit;
      else this.aimKeys &= ~bit;
      this.updateAim();
    };
    // Arrows are fixed movement alternates; everything else is remappable and
    // routed through the (mode-aware) keybinds resolver.
    const arrow =
      k === "arrowup" ? "up" : k === "arrowdown" ? "down" :
      k === "arrowleft" ? "left" : k === "arrowright" ? "right" : null;
    const action = arrow ?? keybinds.actionFor(k);
    switch (action) {
      case "up": setMove(K_UP); break;
      case "down": setMove(K_DOWN); break;
      case "left": setMove(K_LEFT); break;
      case "right": setMove(K_RIGHT); break;
      case "slot1": if (pressed) this.slot = 0; break;
      case "slot2": if (pressed) this.slot = 1; break;
      case "slot3": if (pressed) this.slot = 2; break;
      case "slot4": if (pressed) this.slot = 3; break;
      case "inventory": if (pressed) this.inventoryToggled = true; break;
      case "attack": this.keyAtk = pressed; this.refreshAttack(); break;
      case "aimup": setAim(K_UP); break;
      case "aimdown": setAim(K_DOWN); break;
      case "aimleft": setAim(K_LEFT); break;
      case "aimright": setAim(K_RIGHT); break;
      default: return; // fish/hud handled in main.ts; unbound keys ignored
    }
    if (arrow || action === "attack") e.preventDefault(); // Enter/arrows shouldn't scroll or submit
  }
}
