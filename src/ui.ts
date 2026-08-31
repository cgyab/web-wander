// DOM HUD + inventory overlay. Rendering the world happens on the canvas; text
// is easier and sharper as plain DOM.

import type { Snapshot } from "./snapshot";
import { keyName } from "./keybinds";
import { TouchControls } from "./touch";

const SKILL_NAMES = ["Sword", "Bow", "Axe", "Fire", "Cold", "Poison", "Defense", "Move"];
const TYPE_NAMES = ["Physical", "Fire", "Cold", "Poison", "Pierce"];
const ELEM_COLORS = ["#dcdce4", "#ff5a2a", "#66ccff", "#7ee04a", "#ffcf4a"];
const BASE_NAMES = ["Sword", "Axe", "Dagger", "Spear", "Bow", "Staff"];
const RARITY_COLORS = ["#e8e8f0", "#7bd88f", "#5aa0ff", "#c774ff"];
const RARITY_NAMES = ["Common", "Uncommon", "Rare", "Epic"];

function esc(s: string): string {
  return s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c]!));
}

function durColor(pct: number): string {
  if (pct <= 20) return "#ff6060";
  if (pct <= 50) return "#e0b24a";
  return "#7bd88f";
}

function fmtTime(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = Math.floor(secs % 60);
  return h > 0 ? `${h}h ${m}m ${s}s` : `${m}m ${s}s`;
}

export class UI {
  private hud: HTMLElement;
  private inv: HTMLElement;
  private celebrate: HTMLElement;
  private activeItem: HTMLElement;
  private invOpen = false;
  private invSig = ""; // content signature; the DOM is only rebuilt on change
  private celebrateShown = false;
  private invCap = 60;
  private invFilter = -1; // -1 = all, else a weapon base index
  private hudMinimal = false;
  private shrineMode = false; // inventory window is in "offering" (sacrifice) mode
  private shrineSel = new Set<number>(); // selected inventory indices to sacrifice

  constructor(
    private onEquip: (invIdx: number, slot: number) => void,
    private onDrop: (invIdx: number) => void,
    private onDropBelow: (invIdx: number) => void,
    private onOffer: (invIndices: number[]) => void,
  ) {
    this.hud = document.getElementById("hud")!;
    this.inv = document.getElementById("inventory")!;
    this.celebrate = document.getElementById("celebrate")!;
    this.activeItem = document.getElementById("activeitem")!;
  }

  /** Top-right box: the active weapon slot + a visual hint of its effect. */
  updateActiveItem(s: Snapshot) {
    const e = s.equipped[s.slot];
    if (!e.present) {
      this.activeItem.style.borderColor = "#4a4a70";
      this.activeItem.style.boxShadow = "0 0 0 2px #0a0a12";
      this.activeItem.innerHTML = `<div class="anum">${s.slot + 1}</div><div class="none">empty</div>`;
      return;
    }
    const col = ELEM_COLORS[e.dtype] ?? "#ccc";
    this.activeItem.style.borderColor = col;
    this.activeItem.style.boxShadow = `0 0 12px ${col}66, 0 0 0 2px #0a0a12`;
    let h = `<div class="anum" style="color:${col}">${s.slot + 1}</div>`;
    h += `<div class="abody">`;
    h += `<div class="aeff"><span class="adot" style="background:${col}"></span>${TYPE_NAMES[e.dtype]}</div>`;
    h += `<div class="aname" style="color:${RARITY_COLORS[e.rarity]}">${esc(e.name)}</div>`;
    h += `<div class="adur" style="color:${durColor(e.durability)}">dur ${e.durability}%</div>`;
    h += `</div>`;
    this.activeItem.innerHTML = h;
  }

  /** Show/refresh (or hide) the 100,000 celebration stats overlay. */
  updateCelebration(s: Snapshot) {
    if (!s.celebrating) {
      if (this.celebrateShown) {
        this.celebrate.classList.add("hidden");
        this.celebrateShown = false;
      }
      return;
    }
    const st = s.stats;
    let html = `<h1>&#127881; 100,000 &#127881;</h1>`;
    html += `<div class="sub">You walked to the edge of the world.</div>`;
    html += `<div class="stats">`;
    html += `<span>distance record</span><b>${Math.round(s.maxDist).toLocaleString()}</b>`;
    html += `<span>steps taken</span><b>${Math.round(st.steps).toLocaleString()}</b>`;
    html += `<span>play time</span><b>${fmtTime(st.playSecs)}</b>`;
    html += `<span>monsters slain</span><b>${st.kills.toLocaleString()}</b>`;
    html += `<span>bosses slain</span><b>${st.bossKills.toLocaleString()}</b>`;
    html += `<span>deaths</span><b>${st.deaths.toLocaleString()}</b>`;
    html += `<span>chests opened</span><b>${st.chests.toLocaleString()}</b>`;
    html += `<span>fountains used</span><b>${st.fountains.toLocaleString()}</b>`;
    html += `</div>`;
    html += `<div class="count">The wilds swarm again in ${Math.ceil(Math.max(0, s.celebrateT))}s&hellip;</div>`;
    this.celebrate.innerHTML = html;
    this.celebrate.classList.remove("hidden");
    this.celebrateShown = true;
  }

  // Open/close the pack. At a shrine it opens in "offering" mode (multi-select
  // to sacrifice) instead of the normal inventory.
  toggleInventory(shrineNear = false) {
    this.invOpen = !this.invOpen;
    this.shrineMode = this.invOpen && shrineNear;
    this.shrineSel.clear();
    this.inv.classList.toggle("hidden", !this.invOpen);
    this.inv.classList.toggle("shrine", this.shrineMode);
    document.body.classList.toggle("inv-open", this.invOpen); // disables touch joysticks on mobile
    this.invSig = ""; // force a rebuild next frame when reopened
  }

  setHudMinimal(on: boolean) {
    this.hudMinimal = on;
  }

  setInventoryCap(n: number) {
    this.invCap = n;
  }

  isInventoryOpen(): boolean {
    return this.invOpen;
  }

  closeInventory() {
    if (this.invOpen) this.toggleInventory();
  }

  private shrineLine(s: Snapshot): string {
    if (!s.shrine || this.invOpen) return "";
    return `<span style="color:#7fd0ff;font-weight:bold">&#9962; Offering shrine</span>` +
      ` <span style="color:#c8c8d8">— open your pack to sacrifice items for a reward</span>\n`;
  }

  private restLine(s: Snapshot): string {
    if (!s.rest.active) return "";
    if (s.rest.safe) {
      return `<span style="color:#ff8a3a;font-weight:bold">&#128293; Resting</span>` +
        ` <span style="color:#7ee04a">healing — safe up to 50%</span>\n`;
    }
    // Above half HP: ambush risk climbs the higher you heal.
    const over = Math.max(0, s.hp / s.maxhp - 0.5) / 0.5; // 0..1
    const segs = 10;
    const filled = Math.min(segs, Math.round(over * segs));
    const bar = "▓".repeat(filled) + "░".repeat(segs - filled);
    return `<span style="color:#ff8a3a;font-weight:bold">&#128293; Resting</span>` +
      ` <span style="color:#7ee04a">healing</span>` +
      ` <span style="color:#ff5050;font-weight:bold">&#9888; ambush risk ${bar}</span>\n`;
  }

  private relicLine(s: Snapshot): string {
    const r = s.relic;
    if (!r.active) return "";
    const remain = Math.max(0, r.stepsMax - r.steps);
    const segs = 12;
    const filled = r.shieldMax > 0 ? Math.round((r.shield / r.shieldMax) * segs) : 0;
    const bar = "▓".repeat(filled) + "░".repeat(Math.max(0, segs - filled));
    return `<span style="color:#c060ff;font-weight:bold">&#9760; CURSED RELIC</span> ` +
      `<span style="color:#c8c8d8">${esc(r.weapon)}</span> ` +
      `<span style="color:#9a9ab0">${remain} steps left</span>  ` +
      `<span style="color:#8fd0ff">&#128737; ${bar}</span>\n`;
  }

  private arenaLine(s: Snapshot): string {
    // Entry telegraph: approaching an idle ring (not yet committed).
    if (!s.arena.active && s.arena.near) {
      return `<span style="color:#ffd86b;font-weight:bold">&#9876; Arena ahead</span>` +
        ` <span style="color:#c8c8d8">— step inside the ring to begin (no pausing once you do)</span>\n`;
    }
    if (!s.arena.active) return "";
    // Ready-steady-go: a big countdown before the wave spawns.
    if (s.arena.countdown > 0) {
      const next = s.arena.wave + 1;
      return `<span style="color:#ff8a5c;font-weight:bold">&#9876; Arena — wave ${next}/${s.arena.waves} in</span>` +
        ` <span style="color:#ffd86b;font-weight:bold">${s.arena.countdown}…</span> get ready!\n`;
    }
    let line = `<span style="color:#ff8a5c;font-weight:bold">&#9876; Arena — Wave ${s.arena.wave}/${s.arena.waves}</span>`;
    line += s.arena.rot
      ? ` <span style="color:#ff5050;font-weight:bold">&#9888; in the apron — health rotting!</span>`
      : ` <span style="color:#9a9ab0">(${TouchControls.isTouch() ? "Menu" : "Esc"} forfeits)</span>`;
    return line + `\n`;
  }

  private targetLine(s: Snapshot): string {
    if (!s.target) return "\n";
    const mega = s.target.name.startsWith("Colossal");
    const champ = s.target.name.startsWith("Champion");
    const tag = mega ? `<b style="color:#ff4040">&#9760; MEGA</b> `
      : champ ? `<b style="color:#ffd24a">&#9819; CHAMPION</b> `
      : `<b>Target</b> `;
    const nameColor = mega ? "#ff6060" : champ ? "#ffe08a" : "#e8e8f0";
    return `${tag}<span style="color:${nameColor}">${esc(s.target.name)}</span> Lv${s.target.level}  ` +
      `<span style="color:#7ee04a">use:${TYPE_NAMES[s.target.weak]}</span> ` +
      `<span style="color:#ff9060">resist:${TYPE_NAMES[s.target.resist]}</span>\n`;
  }

  // A blue ward from a shield shrine: shown next to HP while it holds.
  private shieldTag(s: Snapshot): string {
    if (s.shield <= 0.5) return "";
    return `<b style="color:#7fd0ff">&#9670; Shield</b> <span style="color:#7fd0ff">${Math.ceil(s.shield)}</span>   `;
  }

  updateHud(s: Snapshot) {
    const hp = `${Math.max(0, Math.ceil(s.hp))}/${Math.round(s.maxhp)}`;
    const ammoColor = s.ammo === 0 ? "#ff6060" : "#e0b24a";
    const best = Math.round(s.maxDist);

    // Minimal HUD: just the HP/status line and the Target line.
    if (this.hudMinimal) {
      let m = this.relicLine(s) + this.restLine(s) + this.shrineLine(s) + this.arenaLine(s);
      m += `<b>HP</b> ${hp}   ${this.shieldTag(s)}<b>Ammo</b> <span style="color:${ammoColor}">${s.ammo}</span>   `;
      m += `<b>Dist</b> ${Math.round(s.dist)}   <b>Danger</b> Lv ${s.difficulty}\n`;
      m += this.targetLine(s);
      this.hud.innerHTML = m;
      if (this.invOpen) { this.shrineMode ? this.renderShrine(s) : this.renderInventory(s); }
      return;
    }

    // Actively setting a new record when current distance is at/above the best.
    const record = s.dist >= s.maxDist - 0.5 && best > 0;
    let html = this.relicLine(s) + this.restLine(s) + this.shrineLine(s) + this.arenaLine(s);
    html += `<b>HP</b> ${hp}   `;
    html += this.shieldTag(s);
    html += `<b>Ammo</b> <span style="color:${ammoColor}">${s.ammo}</span>   `;
    html += `<b>Dist</b> ${Math.round(s.dist)}   <b>Danger</b> Lv ${s.difficulty}\n`;
    html += `<b style="color:#ffd86b">&#9733; Best</b> <span style="color:#ffd86b">${best}</span>`;
    if (s.checkpointDist >= 1) {
      html += `   <b style="color:#8fd0ff">&#9873; Checkpoint</b> <span style="color:#8fd0ff">${Math.round(s.checkpointDist)}</span>`;
    }
    html += record ? ` <span style="color:#7ee04a">&mdash; new record!</span>` : "";
    html += `\n`;

    // Weapon slots.
    for (let i = 0; i < 4; i++) {
      const e = s.equipped[i];
      const active = i === s.slot;
      const label = e.present ? e.name : "(empty)";
      const color = e.present ? RARITY_COLORS[e.rarity] : "#666";
      const mark = active ? "&#9654;" : " ";
      const dur = e.present ? ` <span style="color:${durColor(e.durability)}">${e.durability}%</span>` : "";
      html += `${mark}<span style="color:${color}">[${i + 1}] ${esc(label)}</span>${dur}\n`;
    }

    html += this.targetLine(s);

    html += `<span style="color:#9a9ab0">Skills</span> `;
    html += s.skillLevels.map((lv, i) => `${SKILL_NAMES[i].slice(0, 2)}${lv}`).join(" ");

    if (s.message) {
      // Its own wrapping block so a long respawn note isn't clipped at the edge
      // (the rest of the HUD is pre-formatted / non-wrapping).
      html += `\n<span class="hudmsg">${esc(s.message)}</span>`;
    }
    this.hud.innerHTML = html;

    if (this.invOpen) { this.shrineMode ? this.renderShrine(s) : this.renderInventory(s); }
  }

  private renderInventory(s: Snapshot) {
    // Sort by dps (high -> low), keeping each item's real inventory index so
    // equip/drop actions still target the right underlying weapon, then apply
    // the active weapon-type filter (for finding e.g. a bow fast).
    const all = s.inventory
      .map((it, idx) => ({ it, idx, dps: it.damage / it.cooldown }))
      .sort((a, b) => b.dps - a.dps);
    const view = this.invFilter < 0 ? all : all.filter((v) => v.it.base === this.invFilter);

    // Per-type counts drive the filter buttons (dim the empty ones).
    const counts = [0, 0, 0, 0, 0, 0];
    for (const it of s.inventory) counts[it.base]++;

    // Only touch the DOM when something actually changed, otherwise rebuilding
    // every frame destroys the row between mousedown and mouseup and clicks
    // never register.
    const sig = `${s.slot}|${s.inventory.length}|${this.invFilter}|` +
      view.map((v) => `${v.it.name}:${v.dps.toFixed(1)}:${v.it.equippedSlot}:${v.it.durability}`).join(",");
    if (sig === this.invSig) return;
    this.invSig = sig;

    const n = s.inventory.length;
    const full = n >= this.invCap;
    const countColor = full ? "#ff6060" : n >= this.invCap * 0.8 ? "#e0b24a" : "#9a9ab0";
    let html = `<h2>Inventory &mdash; active slot [${s.slot + 1}] `;
    html += `<span style="color:${countColor};font-size:13px">(${n}/${this.invCap}${full ? " — FULL" : ""})</span></h2>`;

    // Weapon-type filter row.
    html += `<div class="ifilter">`;
    html += `<button class="fbtn${this.invFilter < 0 ? " on" : ""}" data-f="-1">All</button>`;
    BASE_NAMES.forEach((name, b) => {
      const c = counts[b];
      html += `<button class="fbtn${this.invFilter === b ? " on" : ""}${c === 0 ? " off" : ""}" data-f="${b}">${name}${c ? ` ${c}` : ""}</button>`;
    });
    html += `</div>`;

    if (view.length === 0) {
      html += `<div class="ifempty">${this.invFilter < 0 ? "(empty)" : `no ${BASE_NAMES[this.invFilter]}s`}</div>`;
    }
    view.forEach((v) => {
      const it = v.it;
      const color = RARITY_COLORS[it.rarity];
      const eq = it.equippedSlot !== 255 ? ` <span class="equipped">[slot ${it.equippedSlot + 1}]</span>` : "";
      const uniq = it.unique ? ` <span style="color:#ffd86b">&#9733;unique</span>` : "";
      const dur = ` <span style="color:${durColor(it.durability)}">${it.durability}%</span>`;
      html += `<div class="row">`;
      html += `<span class="equip" data-idx="${v.idx}">`;
      html += `<span style="color:${color}">${esc(it.name)}</span>${uniq} `;
      html += `<span style="color:#9a9ab0">${RARITY_NAMES[it.rarity]} ${BASE_NAMES[it.base]} &middot; ${TYPE_NAMES[it.dtype]} &middot; ${v.dps.toFixed(1)} dps</span>`;
      html += dur + eq + `</span>`;
      html += `<button class="below" data-idx="${v.idx}" title="Delete all weaker items (keeps equipped)">v</button>`;
      html += `<button class="drop" data-idx="${v.idx}" title="Drop/trash this item">&#10005;</button>`;
      html += `</div>`;
    });
    const touch = TouchControls.isTouch();
    const changeHint = touch ? "tap a weapon button to change" : `${keyName("slot1")}-${keyName("slot4")} to change`;
    const closeHint = touch ? "Bag to close" : `${keyName("inventory")} to close`;
    html += `<div class="hintline">Sorted by dps &middot; click to equip to active slot (${changeHint}) &middot; v deletes everything weaker &middot; &#10005; drops one &middot; ${closeHint}.</div>`;
    this.inv.innerHTML = html;

    this.inv.querySelectorAll<HTMLElement>(".equip").forEach((el) => {
      el.onclick = () => this.onEquip(parseInt(el.dataset.idx!, 10), s.slot);
    });
    this.inv.querySelectorAll<HTMLElement>(".drop").forEach((el) => {
      el.onclick = (ev) => {
        ev.stopPropagation();
        this.onDrop(parseInt(el.dataset.idx!, 10));
      };
    });
    this.inv.querySelectorAll<HTMLElement>(".below").forEach((el) => {
      el.onclick = (ev) => {
        ev.stopPropagation();
        this.onDropBelow(parseInt(el.dataset.idx!, 10));
      };
    });
    this.inv.querySelectorAll<HTMLElement>(".fbtn").forEach((el) => {
      el.onclick = () => {
        this.invFilter = parseInt(el.dataset.f!, 10);
        this.invSig = ""; // force a rebuild with the new filter
      };
    });
  }

  /** Offering shrine: pick unequipped items to sacrifice for a reward. */
  private renderShrine(s: Snapshot) {
    // Unequipped items only (you can't sacrifice your active gear), by dps.
    const items = s.inventory
      .map((it, idx) => ({ it, idx, dps: it.damage / it.cooldown }))
      .filter((v) => v.it.equippedSlot === 255)
      .sort((a, b) => b.dps - a.dps);
    // Drop any stale selections (e.g. after an offering shrinks the list).
    for (const i of [...this.shrineSel]) if (i >= s.inventory.length) this.shrineSel.delete(i);

    const sel = this.shrineSel;
    const anyAncient = items.some((v) => sel.has(v.idx) && v.it.unique);
    const sig = `shrine|${sel.size}|${[...sel].sort().join(",")}|` +
      items.map((v) => `${v.it.name}:${v.idx}`).join(",");
    if (sig === this.invSig) return;
    this.invSig = sig;

    let html = `<h2>&#9962; Offering Shrine <span style="color:#9a9ab0;font-size:13px">— sacrifice junk for a reward</span></h2>`;
    html += `<div class="ifilter">`;
    html += `<button class="sbtn selall">Select all</button>`;
    html += `<button class="sbtn clear">Clear</button>`;
    html += `<button class="sbtn offer${sel.size ? "" : " off"}">Offer ${sel.size} &#128293;</button>`;
    html += `</div>`;
    if (items.length === 0) {
      html += `<div class="ifempty">No spare items to offer (equipped gear is safe).</div>`;
    }
    items.forEach((v) => {
      const it = v.it;
      const on = sel.has(v.idx);
      const color = RARITY_COLORS[it.rarity];
      const uniq = it.unique ? ` <span style="color:#ffd86b">&#9733;ancient</span>` : "";
      html += `<div class="row srow${on ? " sel" : ""}" data-idx="${v.idx}">`;
      html += `<span class="mark">${on ? "&#10003;" : "&#9744;"}</span> `;
      html += `<span style="color:${color}">${esc(it.name)}</span>${uniq} `;
      html += `<span style="color:#9a9ab0">${RARITY_NAMES[it.rarity]} ${BASE_NAMES[it.base]} &middot; ${v.dps.toFixed(1)} dps</span>`;
      html += `</div>`;
    });
    const warn = anyAncient
      ? `<span style="color:#ff8a5c">&#9888; offering an ancient: a double blessing — or an enraged boss.</span>`
      : `More items = a better reward (60 = a great one). Ancients are a gamble.`;
    const closes = TouchControls.isTouch() ? "Bag closes" : `${keyName("inventory")} closes`;
    html += `<div class="hintline">Click to toggle &middot; ${warn} &middot; ${closes}.</div>`;
    this.inv.innerHTML = html;

    this.inv.querySelectorAll<HTMLElement>(".srow").forEach((el) => {
      el.onclick = () => {
        const i = parseInt(el.dataset.idx!, 10);
        if (sel.has(i)) sel.delete(i); else sel.add(i);
        this.invSig = ""; // rebuild to reflect the toggle
      };
    });
    this.inv.querySelector<HTMLElement>(".selall")!.onclick = () => {
      items.forEach((v) => sel.add(v.idx));
      this.invSig = "";
    };
    this.inv.querySelector<HTMLElement>(".clear")!.onclick = () => {
      sel.clear();
      this.invSig = "";
    };
    this.inv.querySelector<HTMLElement>(".offer")!.onclick = () => {
      if (sel.size === 0) return;
      this.onOffer([...sel]);
      sel.clear();
      this.invSig = "";
    };
  }
}
