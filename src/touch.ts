// Touch controls for phones/tablets: a twin-stick scheme. Left thumb (left half
// of the screen) is a virtual joystick for movement; right thumb (right half) is
// an aim stick that also fires while held. On-screen buttons cover weapon slots,
// inventory, menu, and mute. Feeds the same Input the keyboard/mouse path uses.

import type { Input } from "./input";

interface Callbacks {
  toggleInventory: () => void;
  openMenu: () => void;
  toggleMute: () => void;
  toggleHud: () => void;
}

const MOVE_BITS = 1 | 2 | 4 | 8;
const MAX_R = 46; // joystick knob travel (px)

export class TouchControls {
  static isTouch(): boolean {
    return "ontouchstart" in window || navigator.maxTouchPoints > 0;
  }

  private moveId = -1;
  private aimId = -1;
  private moveOrigin = { x: 0, y: 0 };
  private aimOrigin = { x: 0, y: 0 };
  private moveKnob!: HTMLElement;
  private aimKnob!: HTMLElement;

  attach(input: Input, cb: Callbacks) {
    document.body.classList.add("touch");

    const pad = el("div", "touchpad");
    pad.id = "touchpad";
    document.body.appendChild(pad);
    this.moveKnob = el("div", "tknob move hidden");
    this.aimKnob = el("div", "tknob aim hidden");
    document.body.appendChild(this.moveKnob);
    document.body.appendChild(this.aimKnob);

    // --- buttons ---
    const mkBtn = (label: string, cls: string, onTap: () => void) => {
      const b = el("div", `tbtn ${cls}`);
      b.textContent = label;
      const fire = (e: Event) => { e.preventDefault(); onTap(); };
      b.addEventListener("touchstart", fire, { passive: false });
      document.body.appendChild(b);
      return b;
    };
    const slots = el("div", "tslots");
    document.body.appendChild(slots);
    for (let i = 0; i < 4; i++) {
      const b = el("div", "tbtn slot");
      b.textContent = String(i + 1);
      b.addEventListener("touchstart", (e) => { e.preventDefault(); input.slot = i; }, { passive: false });
      slots.appendChild(b);
    }
    mkBtn("Bag", "inv", cb.toggleInventory);
    mkBtn("HUD", "hud", cb.toggleHud).id = "thud";
    mkBtn("Menu", "menu", cb.openMenu);
    mkBtn("Sound", "mute", cb.toggleMute).id = "tmute";

    // --- joystick touches on the pad ---
    const halfW = () => window.innerWidth / 2;

    pad.addEventListener("touchstart", (e) => {
      e.preventDefault();
      for (const t of Array.from(e.changedTouches)) {
        if (t.clientX < halfW() && this.moveId < 0) {
          this.moveId = t.identifier;
          this.moveOrigin = { x: t.clientX, y: t.clientY };
          show(this.moveKnob, t.clientX, t.clientY);
        } else if (t.clientX >= halfW() && this.aimId < 0) {
          this.aimId = t.identifier;
          this.aimOrigin = { x: t.clientX, y: t.clientY };
          show(this.aimKnob, t.clientX, t.clientY);
        }
      }
    }, { passive: false });

    pad.addEventListener("touchmove", (e) => {
      e.preventDefault();
      for (const t of Array.from(e.changedTouches)) {
        if (t.identifier === this.moveId) {
          const [dx, dy] = clamp(t.clientX - this.moveOrigin.x, t.clientY - this.moveOrigin.y);
          knob(this.moveKnob, this.moveOrigin, dx, dy);
          // 8-direction movement bitmask (deadzone near center).
          let k = input.keys & ~MOVE_BITS;
          const len = Math.hypot(dx, dy);
          if (len > 10) {
            const ux = dx / len, uy = dy / len;
            if (uy < -0.38) k |= 1;
            if (uy > 0.38) k |= 2;
            if (ux < -0.38) k |= 4;
            if (ux > 0.38) k |= 8;
          }
          input.keys = k;
        } else if (t.identifier === this.aimId) {
          const [dx, dy] = clamp(t.clientX - this.aimOrigin.x, t.clientY - this.aimOrigin.y);
          knob(this.aimKnob, this.aimOrigin, dx, dy);
          const len = Math.hypot(dx, dy);
          if (len > 6) {
            input.aimDX = dx / len;
            input.aimDY = dy / len;
            input.aimActive = true;
            input.attack = true; // hold-to-fire
          }
        }
      }
    }, { passive: false });

    const end = (e: TouchEvent) => {
      for (const t of Array.from(e.changedTouches)) {
        if (t.identifier === this.moveId) {
          this.moveId = -1;
          input.keys &= ~MOVE_BITS;
          this.moveKnob.classList.add("hidden");
        } else if (t.identifier === this.aimId) {
          this.aimId = -1;
          input.attack = false;
          input.aimActive = false;
          this.aimKnob.classList.add("hidden");
        }
      }
    };
    pad.addEventListener("touchend", end, { passive: false });
    pad.addEventListener("touchcancel", end, { passive: false });
  }
}

function el(tag: string, cls: string): HTMLElement {
  const e = document.createElement(tag);
  e.className = cls;
  return e;
}
function show(knob: HTMLElement, x: number, y: number) {
  knob.classList.remove("hidden");
  knob.style.left = `${x}px`;
  knob.style.top = `${y}px`;
  knob.style.setProperty("--kx", "0px");
  knob.style.setProperty("--ky", "0px");
}
function knob(knob: HTMLElement, origin: { x: number; y: number }, dx: number, dy: number) {
  knob.style.left = `${origin.x}px`;
  knob.style.top = `${origin.y}px`;
  knob.style.setProperty("--kx", `${dx}px`);
  knob.style.setProperty("--ky", `${dy}px`);
}
function clamp(dx: number, dy: number): [number, number] {
  const len = Math.hypot(dx, dy);
  if (len <= MAX_R) return [dx, dy];
  return [(dx / len) * MAX_R, (dy / len) * MAX_R];
}
