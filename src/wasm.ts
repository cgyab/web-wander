// Loads the raw wasm module and exposes its exports. No wasm-bindgen: the module
// communicates through shared linear memory and a handful of C-ABI functions.

export interface WasmExports {
  memory: WebAssembly.Memory;
  init(seed: number): void;
  set_input(keys: number, aimx: number, aimy: number, attack: number, slot: number): void;
  update(dt_ms: number): void;
  snapshot_ptr(): number;
  snapshot_len(): number;
  equip(inv_idx: number, slot: number): void;
  drop_item(inv_idx: number): void;
  drop_below(inv_idx: number): void;
  save_ptr(): number;
  save_len(): number;
  io_ptr(): number;
  io_cap(): number;
  load_save(len: number): void;
  debug_warp(tiles: number): void;
  debug_arena(offset: number): void;
  debug_relic(): void;
  debug_campfire(): void;
  debug_shrine(): void;
  debug_fog(): void;
  debug_shield(): void;
  debug_champion(): void;
  open_vault(): void;
  debug_vault(): void;
  debug_rift(): void;
  debug_god(): void;
  debug_clear(): void;
  debug_kill(): void;
  offer(): void;
  fish(quality: number): void;
  debug_fish(): void;
  set_view_h(h: number): void;
  inventory_cap(): number;
  abort_arena(): void;
}

export async function loadWasm(): Promise<WasmExports> {
  const url = `${import.meta.env.BASE_URL}game.wasm`;
  // Provide a no-op env in case the toolchain emits stray imports.
  const imports: WebAssembly.Imports = {
    env: new Proxy({}, { get: () => () => 0 }),
  };
  // Always revalidate so a rebuilt (non-fingerprinted) game.wasm can't be served
  // stale from cache — otherwise a fresh JS bundle can call exports the cached
  // wasm doesn't have.
  const opts: RequestInit = { cache: "no-cache" };
  const res = await fetch(url, opts);
  const { instance } = await WebAssembly.instantiateStreaming(res, imports).catch(async () => {
    // Fallback for servers without wasm MIME support.
    const buf = await (await fetch(url, opts)).arrayBuffer();
    return WebAssembly.instantiate(buf, imports);
  });
  return instance.exports as unknown as WasmExports;
}
