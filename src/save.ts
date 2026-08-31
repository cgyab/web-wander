// Local save with 4 slots so one browser can host a 4-player distance
// challenge. Each slot stores the WASM save blob plus a tiny JSON metadata
// record (best/current distance) so the menu can show a scoreboard without
// loading a slot into the simulation.

import type { WasmExports } from "./wasm";

export const SLOTS = 4;
const slotKey = (s: number) => `webwander.save.slot${s}`;
const metaKey = (s: number) => `webwander.meta.slot${s}`;

export interface SlotMeta {
  best: number;
  dist: number;
}

export function saveSlot(wasm: WasmExports, slot: number, meta: SlotMeta) {
  const ptr = wasm.save_ptr();
  const len = wasm.save_len();
  const bytes = new Uint8Array(wasm.memory.buffer, ptr, len).slice();
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  localStorage.setItem(slotKey(slot), btoa(bin));
  localStorage.setItem(metaKey(slot), JSON.stringify(meta));
}

/** Returns true if the slot had a save that was loaded into the module. */
export function loadSlot(wasm: WasmExports, slot: number): boolean {
  const b64 = localStorage.getItem(slotKey(slot));
  if (!b64) return false;
  try {
    const bin = atob(b64);
    const cap = wasm.io_cap();
    if (bin.length > cap) return false;
    const io = new Uint8Array(wasm.memory.buffer, wasm.io_ptr(), cap);
    for (let i = 0; i < bin.length; i++) io[i] = bin.charCodeAt(i);
    wasm.load_save(bin.length);
    return true;
  } catch {
    return false;
  }
}

export function resetSlot(slot: number) {
  localStorage.removeItem(slotKey(slot));
  localStorage.removeItem(metaKey(slot));
}

export function slotMeta(slot: number): SlotMeta | null {
  const raw = localStorage.getItem(metaKey(slot));
  if (!raw) return null;
  try {
    return JSON.parse(raw) as SlotMeta;
  } catch {
    return null;
  }
}
