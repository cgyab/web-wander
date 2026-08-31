import { defineConfig } from "vite";

// Minimal config. The Rust build emits game.wasm into /public so Vite serves
// and bundles it as a static asset (fetched at runtime via WebAssembly).
export default defineConfig({
  base: "./",
  server: { port: 5173 },
  build: { target: "es2022" },
});
