//! Binary entry point for the headless progression simulator.
//! See `game/sim.rs`. Run: `cargo run --release --bin economy -- [flags]`.
fn main() {
    webwander::sim::run_cli();
}
