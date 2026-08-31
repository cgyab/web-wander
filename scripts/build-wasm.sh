#!/usr/bin/env bash
# Build the Rust simulation to a raw wasm32 module and place it in public/.
# No wasm-bindgen / wasm-pack: we use the bare wasm ABI + shared linear memory.
set -euo pipefail
cd "$(dirname "$0")/.."

# Make cargo available even in minimal shells.
if ! command -v cargo >/dev/null 2>&1; then
  # shellcheck disable=SC1090
  [ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
fi

cargo build --release --target wasm32-unknown-unknown

mkdir -p public
cp target/wasm32-unknown-unknown/release/webwander.wasm public/game.wasm
echo "wasm -> public/game.wasm ($(wc -c < public/game.wasm) bytes)"
