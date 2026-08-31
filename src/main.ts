// Entry point: load WASM, wire input/render/UI, run the fixed game loop.
// Supports 4 save slots so one browser can host a 4-player distance challenge.

import { loadWasm } from "./wasm";
import { parseSnapshot } from "./snapshot";
import { Input } from "./input";
import { Renderer, LOGICAL_W, LOGICAL_H } from "./render";
import { UI } from "./ui";
import { AudioEngine } from "./audio";
import { TouchControls } from "./touch";
import { Fishing } from "./fishing";
import { Vault } from "./vault";
import { keybinds, controls, keyLabel, isBindableKey, isReservedKey, remapList, type Action, type AimMode } from "./keybinds";
import { SLOTS, saveSlot, loadSlot, resetSlot, slotMeta } from "./save";

// iOS/Safari touch hardening. These `gesture*` events exist only in WebKit, so
// on Android/Chrome the listeners never fire — a true no-op there. Double-tap
// zoom and the tap delay are handled purely in CSS (`touch-action: manipulation`
// in style.css), which — unlike a JS double-tap timer — never swallows a
// legitimate fast double-tap, important for an action game.
function hardenTouchGestures() {
  const stop = (e: Event) => e.preventDefault();
  // Safari pinch-zoom (also fired when two fingers land): cancel it.
  for (const t of ["gesturestart", "gesturechange", "gestureend"]) {
    document.addEventListener(t, stop, { passive: false });
  }
  // Long-press context menu / callout anywhere (the canvas already guards its
  // own; this covers HUD buttons, the menu, etc.).
  document.addEventListener("contextmenu", stop);
}

async function main() {
  hardenTouchGestures();
  const canvas = document.getElementById("screen") as HTMLCanvasElement;
  const menuEl = document.getElementById("menu")!;
  const wasm = await loadWasm();
  const renderer = new Renderer(canvas);
  const audio = new AudioEngine();
  const input = new Input();
  input.attach(canvas);
  const ui = new UI(
    (idx, slot) => wasm.equip(idx, slot),
    (idx) => wasm.drop_item(idx),
    (idx) => wasm.drop_below(idx),
    (indices) => makeOffering(indices),
  );
  ui.setInventoryCap(wasm.inventory_cap());

  // Fishing mini-game: pauses the world; on resolve, apply the outcome in wasm.
  const fishing = new Fishing((n) => audio.sfx(n as Parameters<typeof audio.sfx>[0]));
  let fishingActive = false;
  function openFishing() {
    if (fishingActive || vaultActive || !playing || ui.isInventoryOpen()) return;
    if (!menuEl.classList.contains("hidden")) return;
    fishingActive = true;
    audio.ensureStarted();
    fishing.open((q) => {
      fishingActive = false;
      wasm.fish(q);
    });
  }
  // Test hook: resolve a catch deterministically via the same wasm path the
  // overlay uses, so e2e can exercise the reward without playing the reel.
  (window as unknown as { __ww_fish?: (q: number) => void }).__ww_fish = (q) => wasm.fish(q);

  // Rune vault puzzle: unlike fishing, the world KEEPS running while you solve
  // (monsters wander in), so we only freeze the player's own input.
  const vault = new Vault();
  let vaultActive = false;
  let vaultBailed = false; // bailed this approach; re-arm after walking away
  function openVault(difficulty: number) {
    if (vaultActive || !playing || fishingActive || ui.isInventoryOpen()) return;
    if (!menuEl.classList.contains("hidden")) return;
    vaultActive = true;
    audio.ensureStarted();
    const target = 3 + Math.min(3, Math.floor(difficulty / 8)); // 3..6 by region
    vault.open(target, (solved) => {
      vaultActive = false;
      if (solved) { wasm.open_vault(); audio.sfx("chest"); }
      // Re-arm on either outcome: don't auto-re-open until the player steps off
      // the vault footprint. Belt-and-suspenders against a lingering `atVault`
      // (e.g. an overlapping vault) instantly relaunching the puzzle in a loop.
      vaultBailed = true;
    });
  }
  // Test hook: solve the vault deterministically (bypasses the puzzle).
  (window as unknown as { __ww_vault?: () => void }).__ww_vault = () => {
    if (vaultActive) vault.close();
    wasm.open_vault();
  };
  // Test hook: clear monsters (so a path to a distant milestone can be walked).
  (window as unknown as { __ww_clear?: () => void }).__ww_clear = () => wasm.debug_clear();
  // Test hook: kill the player (exercise the death/respawn flow).
  (window as unknown as { __ww_kill?: () => void }).__ww_kill = () => wasm.debug_kill();
  // Test hooks: inspect audio state and simulate a tab-hide (to check that
  // joining/resuming un-sticks the sound).
  (window as unknown as { __ww_audio?: () => unknown }).__ww_audio = () => audio.state();
  (window as unknown as { __ww_audioHide?: () => void }).__ww_audioHide = () => audio.setActive(false);

  // Sacrifice the selected inventory items at a shrine: pass the indices to wasm
  // via the IO buffer, then autosave so the reward (or boss) can't be reloaded.
  function makeOffering(indices: number[]) {
    const dv = new DataView(wasm.memory.buffer, wasm.io_ptr(), wasm.io_cap());
    const n = Math.min(indices.length, (wasm.io_cap() - 2) >> 1);
    dv.setUint16(0, n, true);
    for (let i = 0; i < n; i++) dv.setUint16(2 + i * 2, indices[i], true);
    wasm.offer();
    ui.closeInventory();
    persist();
  }

  let activeSlot = -1; // -1 = no persistent slot (dev/?seed mode)
  let playing = false;
  let lastSnap: ReturnType<typeof parseSnapshot> | null = null;
  let lastSlot = parseInt(localStorage.getItem("webwander.lastslot") ?? "-1", 10);
  let hudMinimal = localStorage.getItem("webwander.hudmin") === "1";
  let rebinding = false; // true while Settings is capturing a key for a rebind
  function applyHud() {
    ui.setHudMinimal(hudMinimal);
    localStorage.setItem("webwander.hudmin", hudMinimal ? "1" : "0");
    const hb = document.getElementById("hudToggle");
    if (hb) hb.textContent = `HUD: ${hudMinimal ? "Minimal" : "Full"}`;
    const th = document.getElementById("thud");
    if (th) th.classList.toggle("on", hudMinimal);
  }
  function toggleHud() { hudMinimal = !hudMinimal; applyHud(); }
  applyHud();

  const randomSeed = () => (Date.now() ^ (Math.random() * 0xffffffff)) >>> 0;

  function persist() {
    if (activeSlot >= 0 && lastSnap) {
      saveSlot(wasm, activeSlot, { best: lastSnap.maxDist, dist: lastSnap.dist });
    }
  }

  // --- slot menu ---------------------------------------------------------

  function openMenu() {
    persist();
    playing = false;
    menuView = "players"; // hand-off always starts on the slot picker
    renderMenu();
    menuEl.classList.remove("hidden");
  }

  // Esc / the mobile Menu button: during an active arena the first press cancels
  // the event (and keeps you in the game — it was consumed on entry, so this is
  // clean, no reward or save-scum); a second press (nothing active) opens the
  // menu. You can never pause your way through an event.
  function menuOrCancelArena() {
    if (lastSnap?.arena.active) {
      wasm.abort_arena();
      return;
    }
    openMenu();
  }

  function chooseSlot(slot: number) {
    persist(); // save whoever was playing before we swap
    activeSlot = slot;
    lastSlot = slot;
    localStorage.setItem("webwander.lastslot", String(slot));
    lastSnap = null;
    if (!loadSlot(wasm, slot)) wasm.init(randomSeed());
    wasm.set_view_h(logicalH); // init/load reset it; re-apply the device aspect
    playing = true;
    havePrev = false;
    menuEl.classList.add("hidden");
    audio.start(); // the click is our user gesture to unlock Web Audio
    canvas.focus();
  }

  function doReset(slot: number) {
    resetSlot(slot);
    if (slot === activeSlot) activeSlot = -1;
    renderMenu();
  }

  function renderMenu() {
    const touch = TouchControls.isTouch();
    const c = controls(touch);
    const metas = Array.from({ length: SLOTS }, (_, i) => slotMeta(i));
    const best = Math.max(0, ...metas.map((m) => (m ? m.best : 0)));
    let html = `<div class="panel">`;
    html += `<h1>WebWander</h1><p class="subtitle">a &ldquo;Walking Around&rdquo; game</p>`;
    html += `<p class="tag">Distance Challenge &mdash; how far from origin can you get?</p>`;
    // Two tabs that slide the focus between the slot picker and settings, so the
    // header always stays on-screen (mobile can't afford both stacked).
    html += `<div class="mtabs" role="tablist">`;
    html += `<button class="mtab active" data-view="players">Players</button>`;
    html += `<button class="mtab" data-view="settings">Settings</button>`;
    html += `</div>`;
    html += `<div class="views"><div class="track">`;
    html += `<section class="view vplayers">`;
    html += `<div class="slots">`;
    for (let i = 0; i < SLOTS; i++) {
      const m = metas[i];
      const leader = m && m.best > 0 && m.best === best;
      const current = i === lastSlot;
      html += `<div class="slot${leader ? " leader" : ""}${current ? " current" : ""}">`;
      html += `<div class="slotname">${leader ? "&#9733; " : ""}Player ${i + 1}${current ? " &#9679;" : ""}</div>`;
      if (m) {
        html += `<div class="slotstat"><span>best</span> <b>${Math.round(m.best)}</b></div>`;
        html += `<div class="slotstat"><span>at</span> ${Math.round(m.dist)}</div>`;
      } else {
        html += `<div class="slotstat empty">empty</div><div class="slotstat">&nbsp;</div>`;
      }
      html += `<button class="play" data-slot="${i}">${m ? "Continue" : "New game"}</button>`;
      html += `<button class="reset" data-slot="${i}"${m ? "" : " disabled"}>Reset</button>`;
      html += `</div>`;
    }
    html += `</div>`; // .slots
    html += `<p class="hintline">${touch ? `Tap ${c.menu} in-game` : `Press ${c.menu} in-game`} to return here and pass to the next player.</p>`;
    html += `</section>`; // .vplayers
    // Settings view: sound (with live preview), HUD toggle, and the help link.
    html += `<section class="view vsettings">`;
    html += `<div class="sound">`;
    html += `<div class="srow"><span>Music</span><input id="volMusic" class="vol" type="range" min="0" max="100" value="${Math.round(audio.getMusicVolume() * 100)}"><button class="stest" data-k="music" title="Preview">&#9654;</button></div>`;
    html += `<div class="srow"><span>Sound&nbsp;FX</span><input id="volSfx" class="vol" type="range" min="0" max="100" value="${Math.round(audio.getSfxVolume() * 100)}"><button class="stest" data-k="sfx" title="Preview">&#9654;</button></div>`;
    html += `<button class="mute${audio.isMuted() ? " muted" : ""}" id="menuMute">${audio.isMuted() ? "&#128263; Muted" : "&#128266; Sound On"}</button>`;
    html += `<button class="hudtoggle" id="hudToggle">HUD: ${hudMinimal ? "Minimal" : "Full"}</button>`;
    html += `</div>`; // .sound
    // Controls: remap keys on a keyboard; on touch the controls are on-screen.
    html += `<div class="controls">`;
    html += `<div class="ctitle">Controls</div>`;
    if (touch) {
      html += `<p class="cnote">Play with the on-screen controls: drag the <b>left stick</b> to move, the <b>right stick</b> to aim &amp; fire, and tap the weapon, <b>Bag</b>, <b>HUD</b>, <b>Menu</b>, and <b>Sound</b> buttons.</p>`;
    } else {
      // Keyboard aiming: 0 mouse, 1 aim-with-movement + fire key, 2 aim cluster.
      const am = keybinds.aimMode;
      const seg = (v: AimMode, t: string, sub: string) =>
        `<button class="aimopt${am === v ? " on" : ""}" data-aim="${v}"><b>${t}</b><span>${sub}</span></button>`;
      html += `<div class="aimmode">`;
      html += `<div class="ctitle">Keyboard aiming</div>`;
      html += `<div class="aimseg">`;
      html += seg(0, "Mouse", "point &amp; click");
      html += seg(1, "Move-aim", "shoot where you move");
      html += seg(2, "Aim keys", "IJKL aim + fire");
      html += `</div></div>`;
      html += `<div class="keylist">`;
      for (const { action, label } of remapList(am)) {
        html += `<div class="crow"><span>${label}</span><button class="keybtn" data-action="${action}">${keyLabel(keybinds.get(action))}</button></div>`;
      }
      html += `</div>`; // .keylist
      html += `<p class="cnote">Click a key, then press the new one. Movement also works with the arrow keys. (M mute and Esc menu are fixed.)</p>`;
      html += `<button class="ckreset">Reset to defaults</button>`;
    }
    html += `</div>`; // .controls
    html += `<button class="help">? &nbsp;How to play</button>`;
    html += `</section>`; // .vsettings
    html += `</div></div>`; // .track, .views
    html += `</div>`; // .panel
    menuEl.innerHTML = html;

    // Sound controls (start audio on first interaction — a valid user gesture).
    const vm = menuEl.querySelector<HTMLInputElement>("#volMusic")!;
    vm.oninput = () => { audio.ensureStarted(); audio.setMusicVolume(vm.valueAsNumber / 100); };
    const vs = menuEl.querySelector<HTMLInputElement>("#volSfx")!;
    vs.oninput = () => { audio.ensureStarted(); audio.setSfxVolume(vs.valueAsNumber / 100); };
    menuEl.querySelectorAll<HTMLElement>(".stest").forEach((el) => {
      el.onclick = () => {
        audio.ensureStarted();
        if (el.dataset.k === "music") audio.previewMusic();
        else audio.previewSfx();
      };
    });
    menuEl.querySelector<HTMLElement>("#menuMute")!.onclick = () => toggleMute();
    menuEl.querySelector<HTMLElement>("#hudToggle")!.onclick = () => toggleHud();

    // Keyboard-aim mode selector: switch, then re-render so the remap list shows
    // that mode's keys (and the Bag relocates off `i` in the aim-cluster mode).
    menuEl.querySelectorAll<HTMLButtonElement>(".aimopt").forEach((btn) => {
      btn.onclick = () => {
        keybinds.setAimMode(parseInt(btn.dataset.aim!, 10) as AimMode);
        refreshHint();
        renderMenu();
      };
    });

    // Key remapping: click a row, then press the next key (it's captured before
    // the game/menu handlers). A key already in use swaps with this action.
    menuEl.querySelectorAll<HTMLButtonElement>(".keybtn").forEach((btn) => {
      btn.onclick = () => startRebind(btn);
    });
    menuEl.querySelector<HTMLElement>(".ckreset")?.addEventListener("click", () => {
      keybinds.reset();
      refreshHint();
      renderMenu();
    });

    menuEl.querySelector<HTMLElement>(".help")!.onclick = showHelp;
    menuEl.querySelectorAll<HTMLElement>(".play").forEach((el) => {
      el.onclick = () => chooseSlot(parseInt(el.dataset.slot!, 10));
    });
    menuEl.querySelectorAll<HTMLElement>(".reset").forEach((el) => {
      el.onclick = async () => {
        const s = parseInt(el.dataset.slot!, 10);
        if (await askConfirm(`Reset Player ${s + 1}?`, "This permanently erases their world and all progress.", "Reset")) {
          doReset(s);
        }
      };
    });

    // Slide the view track between Players and Settings; size the (clipping)
    // window to the active pane so the shorter one doesn't leave a big gap.
    const track = menuEl.querySelector<HTMLElement>(".track")!;
    const views = menuEl.querySelector<HTMLElement>(".views")!;
    const panes = Array.from(menuEl.querySelectorAll<HTMLElement>(".view"));
    const tabs = Array.from(menuEl.querySelectorAll<HTMLElement>(".mtab"));
    const apply = (animate: boolean) => {
      const idx = menuView === "settings" ? 1 : 0;
      tabs.forEach((t) => t.classList.toggle("active", t.dataset.view === menuView));
      if (!animate) { track.style.transition = "none"; views.style.transition = "none"; }
      track.style.transform = `translateX(-${idx * 100}%)`;
      // Size to the ACTIVE pane so each tab is exactly as tall as its content —
      // the shorter Players pane no longer scrolls into blank space left over
      // from the taller Settings pane. The height transition keeps it smooth.
      const h = panes[idx].scrollHeight;
      if (h > 0) views.style.height = `${h}px`;
      if (!animate) requestAnimationFrame(() => { track.style.transition = ""; views.style.transition = ""; });
    };
    tabs.forEach((t) => { t.onclick = () => { menuView = t.dataset.view!; apply(true); }; });
    relayoutMenu = () => apply(false);
    // Measure after the menu is actually visible (openMenu unhides after render).
    requestAnimationFrame(() => apply(false));
  }
  // Which menu pane is in focus, and a hook so resize() can re-measure it.
  let menuView = "players";
  let relayoutMenu: (() => void) | null = null;

  // Capture the next key press and bind it to `action` (Esc cancels). The
  // `rebinding` flag suspends the game/HUD key handlers meanwhile.
  function startRebind(btn: HTMLButtonElement) {
    if (rebinding) return;
    rebinding = true;
    const action = btn.dataset.action as Action;
    btn.classList.add("listening");
    btn.textContent = "Press a key…";
    const cleanup = () => { rebinding = false; window.removeEventListener("keydown", onKey, true); };
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      cleanup();
      // Reject Esc (cancel), non-bindable keys, and keys reserved by fixed
      // handlers (M mute) so we never create a collision the swap can't see.
      if (e.key !== "Escape" && isBindableKey(e.key) && !isReservedKey(e.key)) {
        keybinds.rebind(action, e.key);
        refreshHint();
      }
      renderMenu(); // reflect the new binding (a swap may change another row)
    };
    window.addEventListener("keydown", onKey, true);
  }

  // The on-screen desktop hint line (hidden on touch) reflects the live binds.
  const hintEl = document.getElementById("hint");
  function refreshHint() {
    if (!hintEl) return;
    const b = (t: string) => `<span class="k">${t}</span>`;
    const key = (a: Action) => b(keyLabel(keybinds.get(a)));
    const move = b(keyLabel(keybinds.get("up")) + keyLabel(keybinds.get("left")) +
      keyLabel(keybinds.get("down")) + keyLabel(keybinds.get("right")));
    hintEl.innerHTML =
      `${move} move &middot; Mouse aim &middot; Click attack &middot; ` +
      `${key("slot1")}-${key("slot4")} weapons &middot; ${key("inventory")} inventory &middot; ` +
      `${key("fish")} fish (at water) &middot; ${b("Esc")} menu &middot; ${key("hud")} hud &middot; ${b("M")} mute`;
  }
  refreshHint();

  // "How to play" reference overlay, opened from the slot menu.
  const helpEl = document.getElementById("help")!;
  function showHelp() {
    const row = (a: string, b: string) => `<tr><td>${a}</td><td>${b}</td></tr>`;
    const c = controls(TouchControls.isTouch());
    helpEl.innerHTML =
      `<div class="hpanel"><h1>WebWander</h1><div class="hsub">How far from the origin can you walk?</div>` +
      `<p>Move with ${c.move}, aim with ${c.aim}, ${c.attack} to attack. ${c.weapons} switch weapons, ${c.pack} opens your pack, ${c.menu} pauses, ${c.hud} toggles the HUD, ${c.mute} mutes. Fish at the water's edge with ${c.fish}.</p>` +

      `<h2>Live status (top of the screen)</h2><table>` +
      row("HP", "Health. You die at 0 and respawn at your last checkpoint. Max HP grows with your <b>Defense</b> skill.") +
      row("Ammo", "Shared pool for ranged weapons (1 per shot). At 0, ranged can't fire — use melee. Melee costs none.") +
      row("Dist", "Your distance from the origin, in tiles. This is the whole game — go as far as you can.") +
      row("Danger", "The threat level where you're standing. Higher = tougher, denser monsters and better loot. It rises with Dist.") +
      row("★ Best", "The farthest you've ever reached. Your record to beat.") +
      row("⚑ Checkpoint", "Where you respawn on death (banked every 250 distance), so you don't restart from the origin.") +
      row("[1]-[4]", "Equipped weapons and their durability %. Gear loses 10% per death and breaks at 0%.") +
      `</table>` +

      `<h2>Skills — you don't level up, you get better at what you use (+6% per level)</h2><table>` +
      row("Sword", "Swords, daggers, spears: more damage and faster attacks. Trained by using them.") +
      row("Bow", "Bows and staves (ranged): more damage, faster shots.") +
      row("Axe", "Axes: more damage, faster swings.") +
      row("Fire / Cold / Poison", "Extra damage when your weapon deals that element — and elements matter: 2× on a monster's weakness, ½× on its resistance (3× / almost nothing vs bosses). Watch the target's <b>use:</b> hint.") +
      row("Defense", "Trained by taking hits: reduces incoming damage (up to −75%) and raises your max HP (+8 per level).") +
      row("Move", "Trained by walking: increases movement speed (capped so you stay in control).") +
      `</table>` +
      `<p style="margin-top:8px;color:#c8c8d8"><b style="color:#ffd86b">Which skill trains which weapon:</b><br>` +
      `&#9670; <b>Sword</b> &rarr; Swords, Daggers, Spears &nbsp;&middot;&nbsp; <b>Bow</b> &rarr; Bows, Staves &nbsp;&middot;&nbsp; <b>Axe</b> &rarr; Axes<br>` +
      `&#9670; <b>Fire / Cold / Poison</b> stack on top for <i>any</i> weapon that deals that element (Physical &amp; Pierce have no element skill).</p>` +
      `<p style="margin-top:8px;color:#9a9ab0">Tip: chests hold overpowered <b>unique</b> weapons, fountains fully heal, and rare <b>Colossal</b> bosses drop a real haul — but bring the right element or you won't scratch them.</p>` +
      `<button class="close">Got it</button></div>`;
    helpEl.classList.remove("hidden");
    const close = () => { helpEl.classList.add("hidden"); window.removeEventListener("keydown", onKey, true); };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); } };
    window.addEventListener("keydown", onKey, true);
    helpEl.querySelector<HTMLElement>(".close")!.onclick = close;
    helpEl.onclick = (e) => { if (e.target === helpEl) close(); };
  }

  // Themed in-app confirmation (replaces the native confirm() dialog).
  const confirmEl = document.getElementById("confirm")!;
  function askConfirm(title: string, body: string, confirmLabel: string): Promise<boolean> {
    return new Promise((resolve) => {
      confirmEl.innerHTML =
        `<div class="cpanel"><h2>${title}</h2><p>${body}</p>` +
        `<div class="cbtns"><button class="cancel">Cancel</button>` +
        `<button class="danger">${confirmLabel}</button></div></div>`;
      confirmEl.classList.remove("hidden");
      const done = (v: boolean) => {
        confirmEl.classList.add("hidden");
        window.removeEventListener("keydown", onKey, true);
        resolve(v);
      };
      const onKey = (e: KeyboardEvent) => {
        if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); done(false); }
        else if (e.key === "Enter") { e.preventDefault(); done(true); }
      };
      window.addEventListener("keydown", onKey, true); // capture, before the game's Esc handler
      confirmEl.querySelector<HTMLElement>(".cancel")!.onclick = () => done(false);
      confirmEl.querySelector<HTMLElement>(".danger")!.onclick = () => done(true);
      confirmEl.onclick = (e) => { if (e.target === confirmEl) done(false); }; // backdrop cancels
    });
  }

  // --- viewport ----------------------------------------------------------
  // Fixed logical width; the height tracks the device aspect ratio so the game
  // fills the screen in any orientation (no letterboxing).
  let logicalH = LOGICAL_H;
  function resize() {
    const iw = window.innerWidth, ih = window.innerHeight;
    logicalH = Math.round(Math.max(150, Math.min(1200, (LOGICAL_W * ih) / iw)));
    renderer.setLogicalSize(LOGICAL_W, logicalH);
    wasm.set_view_h(logicalH); // no-op until a game exists
    // The canvas fills the whole viewport; aspect matches so there's no stretch.
    canvas.style.width = `${iw}px`;
    canvas.style.height = `${ih}px`;
    canvas.style.left = "0px";
    canvas.style.top = "0px";
    canvas.style.right = "auto";
    canvas.style.bottom = "auto";
    input.setViewport(iw / LOGICAL_W, 0, 0); // uniform scale, no offset
    if (!menuEl.classList.contains("hidden")) relayoutMenu?.(); // re-measure the open menu
    applyBottomInset(); // keep the touch buttons clear of browser chrome
  }
  window.addEventListener("resize", resize);
  window.addEventListener("orientationchange", resize);

  // Keep the bottom row of touch buttons above the mobile browser chrome.
  // position:fixed anchors to the *layout* viewport, but the URL bar shrinks the
  // *visual* viewport, so a `bottom:12px` button can end up below the visible
  // area (it looks like the buttons get "pushed off" when the bar reappears).
  // --kb is that gap; the button offsets add it so they stay on-screen.
  function applyBottomInset() {
    const vv = window.visualViewport;
    if (!vv) return;
    const gap = Math.max(0, Math.round(document.documentElement.clientHeight - vv.height - vv.offsetTop));
    document.documentElement.style.setProperty("--kb", `${gap}px`);
  }
  if (window.visualViewport) {
    window.visualViewport.addEventListener("resize", applyBottomInset);
    window.visualViewport.addEventListener("scroll", applyBottomInset);
  }
  applyBottomInset();

  resize();

  // Esc closes the inventory if it's open; otherwise it pauses and opens the
  // slot menu (hand-off to the next player). This stops Esc-from-inventory from
  // yanking players back to the slot screen.
  window.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || !playing) return;
    e.preventDefault();
    if (ui.isInventoryOpen()) ui.closeInventory();
    else menuOrCancelArena();
  });

  // Central mute so every control (M key, touch button, menu toggle) stays in
  // sync and shows the state.
  function updateMuteButtons(muted: boolean) {
    const tm = document.getElementById("tmute");
    if (tm) { tm.textContent = muted ? "Muted" : "Sound"; tm.classList.toggle("muted", muted); }
    const mm = document.getElementById("menuMute");
    if (mm) { mm.innerHTML = muted ? "&#128263; Muted" : "&#128266; Sound On"; mm.classList.toggle("muted", muted); }
  }
  function toggleMute() {
    updateMuteButtons(audio.toggleMute());
  }

  // M toggles mute (fixed); the HUD/fish keys are remappable.
  window.addEventListener("keydown", (e) => {
    if (fishingActive) return; // the fishing overlay owns input while open
    if (rebinding) return; // Settings is capturing the next key for a rebind
    if (e.key === "m" || e.key === "M") { toggleMute(); return; }
    const action = keybinds.actionFor(e.key);
    if (action === "hud") toggleHud();
    else if (action === "fish" && lastSnap?.canFish) openFishing();
  });

  // Mouse wheel scrolls through equipped weapon slots (skips empty ones) so you
  // can swap fast for a tough encounter; the top-right box shows the new pick.
  canvas.addEventListener("wheel", (e) => {
    if (!playing || !lastSnap) return;
    e.preventDefault();
    const present = [0, 1, 2, 3].some((i) => lastSnap!.equipped[i].present);
    if (!present) return;
    const dir = e.deltaY > 0 ? 1 : -1;
    let cur = input.slot;
    for (let step = 0; step < 4; step++) {
      cur = (cur + dir + 4) % 4;
      if (lastSnap.equipped[cur].present) break;
    }
    input.slot = cur;
  }, { passive: false });

  // Touch devices get on-screen twin-stick controls.
  if (TouchControls.isTouch()) {
    new TouchControls().attach(input, {
      toggleInventory: () => ui.toggleInventory(lastSnap?.shrine ?? false),
      openMenu: () => { if (ui.isInventoryOpen()) ui.closeInventory(); else menuOrCancelArena(); },
      toggleMute: () => toggleMute(),
      // The HUD button doubles as the Fish button when you're at water.
      toggleHud: () => { if (lastSnap?.canFish && !fishingActive) openFishing(); else toggleHud(); },
    });
    updateMuteButtons(audio.isMuted()); // set the touch mute button's initial label
    applyHud(); // set the touch HUD button's initial state
  }

  // Persist periodically and when leaving. Silence/suspend audio when the tab is
  // hidden or the app is closing so teardown doesn't emit a click.
  setInterval(persist, 2000);
  window.addEventListener("pagehide", () => { persist(); audio.setActive(false); });
  window.addEventListener("beforeunload", () => { persist(); audio.setActive(false); });
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) { persist(); audio.setActive(false); }
    else audio.setActive(true);
  });

  // --- startup: dev ?seed mode bypasses slots; otherwise show the menu ----

  const params = new URLSearchParams(location.search);
  if (params.has("seed")) {
    wasm.init((parseInt(params.get("seed")!, 10) >>> 0) || 1);
    wasm.set_view_h(logicalH);
    // Dev-only teleport, e.g. ?seed=1&warp=100000 to reach the celebration.
    if (params.has("warp")) wasm.debug_warp(parseFloat(params.get("warp")!) || 0);
    // Dev-only: ?arena=1 drops an arena ring on the player. Add &approach=1 to
    // place it a short walk east so the entry telegraph can be tested.
    if (params.has("arena")) wasm.debug_arena(params.has("approach") ? 160 : 0);
    // Dev-only: ?relic=1 seizes a cursed relic to test the sprint.
    if (params.has("relic")) wasm.debug_relic();
    // Dev-only: ?campfire=1 drops a campfire on the player to test resting.
    if (params.has("campfire")) wasm.debug_campfire();
    // Dev-only: ?shrine=1 drops an offering shrine on the player.
    if (params.has("shrine")) wasm.debug_shrine();
    // Dev-only: ?fog=1 drops a cursed-fog patch (with cache) next to the player.
    if (params.has("fog")) wasm.debug_fog();
    // Dev-only: ?shield=1 grants a shield-shrine ward.
    if (params.has("shield")) wasm.debug_shield();
    // Dev-only: ?champion=1 spawns a champion (with ambient mobs) to the east.
    if (params.has("champion")) wasm.debug_champion();
    // Dev-only: ?vault=1 drops a rune vault on the player.
    if (params.has("vault")) wasm.debug_vault();
    // Dev-only: ?rift=1 drops a rift just east of the player.
    if (params.has("rift")) wasm.debug_rift();
    // Dev-only: ?god=1 grants damage immunity (reach distant milestones).
    if (params.has("god")) wasm.debug_god();
    // Dev-only: ?fish=1 moves the player to a calm water's edge to test fishing.
    if (params.has("fish")) wasm.debug_fish();
    activeSlot = -1;
    playing = true;
  } else {
    openMenu();
  }

  // --- audio driver: derive SFX + music intensity from snapshot diffs ----

  type Snap = ReturnType<typeof parseSnapshot>;
  let a = { ammo: 0, hp: 0, mon: 0, mega: 0, inv: 0, hpSum: 0, celeb: false, milestone: 0, aWave: 0, aCount: 0, aMsg: "", aNear: false, relic: false, init: false };

  function biomeUnderPlayer(s: Snap): number {
    const tx = Math.floor(s.px / 16) - s.tx0;
    const ty = Math.floor(s.py / 16) - s.ty0;
    if (tx < 0 || ty < 0 || tx >= s.cols || ty >= s.rows) return 3;
    return s.tiles[ty * s.cols + tx];
  }

  function driveAudio(s: Snap, respawned: boolean) {
    const monsters = s.entities.filter((e) => e.kind === 1);
    const mon = monsters.length;
    const mega = monsters.filter((e) => (e.shape & 0x80) !== 0).length;
    const hpSum = monsters.reduce((acc, e) => acc + e.hpFrac, 0);
    const near = monsters.some((e) => Math.hypot(e.x - s.px, e.y - s.py) < 72);

    const tension = s.celebrating ? 3 : mega > 0 ? 2 : near || s.hp < a.hp - 0.5 ? 1 : 0;
    audio.setEnvironment(biomeUnderPlayer(s), tension);

    if (a.init && !respawned) {
      if (s.ammo < a.ammo) audio.sfx("shoot");
      else if (s.ammo > a.ammo) audio.sfx("ammo");
      if (s.hp < a.hp - 0.5) audio.sfx("hurt");
      else if (s.hp > a.hp + 0.5) audio.sfx("health");
      if (mon < a.mon) audio.sfx("death");
      else if (hpSum < a.hpSum - 0.02) audio.sfx("hit");
      if (mega > a.mega) audio.sfx("mega");
      if (s.inventory.length > a.inv) {
        audio.sfx(/chest|ancient|colossal|claimed/i.test(s.message) ? "chest" : "item");
      }
      if (s.celebrating && !a.celeb) audio.sfx("cheer");
      if (s.milestoneT > 0 && a.milestone <= 0) audio.sfx("milestone");

      // Arena: ready-steady-go ticks, the "go" on wave spawn, wave-clear chime,
      // and a cheer for the whole arena cleared.
      const ar = s.arena;
      if (ar.near && !a.aNear) audio.sfx("arenanear"); // approaching an idle ring
      if (s.relic.active && !a.relic) audio.sfx("relic"); // cursed relic seized
      if (/^Ambush!/i.test(s.message) && !/^Ambush!/i.test(a.aMsg)) audio.sfx("ambush");
      const waveJustFell = ar.countdown > 0 && a.aCount === 0 && ar.wave >= 1;
      if (waveJustFell) audio.sfx("waveclear");
      if (ar.countdown > 0 && ar.countdown !== a.aCount && !waveJustFell) audio.sfx("count"); // 3… 2… 1…
      if (ar.wave > a.aWave) audio.sfx("go"); // spawn!
      if (/arena cleared/i.test(s.message) && !/arena cleared/i.test(a.aMsg)) audio.sfx("cheer");
    }
    a = { ammo: s.ammo, hp: s.hp, mon, mega, inv: s.inventory.length, hpSum, celeb: s.celebrating, milestone: s.milestoneT,
      aWave: s.arena.wave, aCount: s.arena.countdown, aMsg: s.message, aNear: s.arena.near, relic: s.relic.active, init: true };
  }

  // --- game loop ---------------------------------------------------------

  const isTouch = TouchControls.isTouch();
  const keysVec = (k: number): [number, number] => {
    let x = 0, y = 0;
    if (k & 1) y -= 1;
    if (k & 2) y += 1;
    if (k & 4) x -= 1;
    if (k & 8) x += 1;
    return [x, y];
  };

  let last = performance.now();
  let prevX = 0, prevY = 0, havePrev = false;
  let prevDeaths = 0; // last death count, to detect a respawn (any death)
  let prevCheckpoint = 0; // last banked checkpoint distance, to save when it grows
  let prevRelic = false; // last relic-active state, to autosave on pickup
  let prevCanFish = false; // last fishing eligibility, to swap the mobile button
  const thudBtn = () => document.getElementById("thud");
  let curAimX = 1, curAimY = 0; // persists so touch idle keeps the last facing

  function frame(now: number) {
    const dt = now - last;
    last = now;

    // Drive the rune-vault puzzle from the animation frame (not setTimeout, which
    // mobile Chrome throttles/drops). Runs even while the world is paused, so the
    // puzzle can't stall waiting on a lost timer. No-op when the vault is closed.
    vault.tick(now);

    // The rune vault pauses the world (like fishing), so the puzzle is a calm
    // brain-break — no monsters can attack you mid-solve.
    if (playing && !fishingActive && !vaultActive) {
      if (input.consumeInventoryToggle()) ui.toggleInventory(lastSnap?.shrine ?? false);

      // Aim source, in priority order:
      //  - a direct stick/aim-cluster vector (touch right stick, or mode-2 keys);
      //  - "face the movement direction" for touch idle OR keyboard aim mode 1
      //    (keeping the last facing when standing still);
      //  - mode-2 with no aim key held keeps the last facing (do nothing);
      //  - otherwise (mouse mode) the mouse offset from the centred player.
      const aimMode = keybinds.aimMode;
      if (input.aimActive) {
        curAimX = input.aimDX;
        curAimY = input.aimDY;
      } else if (isTouch || aimMode === 1) {
        const [mx, my] = keysVec(input.keys);
        if (mx !== 0 || my !== 0) { curAimX = mx; curAimY = my; }
      } else if (aimMode === 2) {
        /* aim keys released — hold the last facing */
      } else {
        curAimX = input.mouseX - LOGICAL_W / 2;
        curAimY = input.mouseY - logicalH / 2;
      }
      const aimX = curAimX, aimY = curAimY;
      wasm.set_input(input.keys, aimX, aimY, input.attack ? 1 : 0, input.slot);
      wasm.update(dt);

      const snap = parseSnapshot(wasm.memory.buffer, wasm.snapshot_ptr(), wasm.snapshot_len());

      // A death (the deaths counter ticked up) means the player respawned. This
      // is more reliable than a teleport-distance heuristic: it catches deaths
      // at a nearby checkpoint too, and doesn't misfire on a rift's big jump.
      const respawned = havePrev && snap.stats.deaths > prevDeaths;
      // Any big one-frame jump (a death respawn OR a rift leap) should mute the
      // per-frame audio diff so it doesn't fire spurious hurt/death cues.
      const jumped = havePrev && Math.hypot(snap.px - prevX, snap.py - prevY) > 96;
      if (respawned) {
        renderer.flash();
        audio.sfx("respawn");
        // Dying ends any open event first: close the pack / offering shrine so it
        // doesn't linger over the respawned player.
        if (ui.isInventoryOpen()) ui.closeInventory();
      }
      // Banking a new checkpoint pushes checkpointDist further out.
      const bankedCheckpoint = havePrev && snap.checkpointDist > prevCheckpoint + 0.5;
      // Seizing a cursed relic — save immediately so the sprint survives a reload.
      const relicSeized = havePrev && snap.relic.active && !prevRelic;
      prevX = snap.px;
      prevY = snap.py;
      prevDeaths = snap.stats.deaths;
      prevCheckpoint = snap.checkpointDist;
      prevRelic = snap.relic.active;
      havePrev = true;

      driveAudio(snap, respawned || jumped);

      renderer.draw(snap, aimX, aimY);
      ui.updateHud(snap);
      ui.updateActiveItem(snap);
      ui.updateCelebration(snap);
      lastSnap = snap;

      // Save right away on the moments that matter — death/respawn, each new
      // checkpoint, and a relic pickup — rather than waiting for the 2s autosave.
      if (respawned || bankedCheckpoint || relicSeized) persist();

      // Rune vault: step onto one to start the puzzle (pauses the world).
      // Bailing re-arms only after you walk away.
      if (snap.atVault && !vaultBailed) openVault(snap.difficulty);
      if (!snap.atVault) vaultBailed = false;

      // Swap the mobile HUD button to "Fish" when you can fish here.
      if (snap.canFish !== prevCanFish) {
        prevCanFish = snap.canFish;
        const b = thudBtn();
        if (b) b.textContent = snap.canFish ? "🎣" : "HUD";
      }

      // Debug hook (harmless): lets tooling inspect live state.
      (window as unknown as { __ww?: unknown }).__ww = snap;
    }

    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
}

main().catch((e) => {
  // textContent (not innerHTML) so nothing in the error/stack can be interpreted
  // as markup — defense-in-depth for the one place a non-constant string is shown.
  const pre = document.createElement("pre");
  pre.style.cssText = "color:#f88;padding:20px;white-space:pre-wrap";
  pre.textContent = `Failed to start: ${e}\n${e?.stack ?? ""}`;
  document.body.replaceChildren(pre);
  console.error(e);
});
