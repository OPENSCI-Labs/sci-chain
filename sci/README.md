# SCI Chain Extensions

Everything SCI adds to Base Azul v0.9 lives under this directory. See the top-level
[`README.md`](../README.md) for the project overview and
[`docs/porting-notes.md`](docs/porting-notes.md) for a deep technical writeup.

## Layout

- `crates/` — Rust
  - `precompiles/` — `AccountKeychain` precompile (ported from Tempo v1.7.1), the
    `SciAgentState` circuit-breaker precompile, the pre-execution hook, and the EVM
    storage abstraction.
  - `precompiles-macros/` — proc macros (`#[contract]`, `#[derive(Storable)]`).
  - `precompile-abi/` — ABI bindings for the precompiles (alloy `sol!`).
  - `revm-shim/` — compat shim exposing revm 38's API on Base v0.9's revm 34.
  - `tempo-chainspec-shim/` — exposes `TempoHardfork` so ported Tempo source compiles.
- `contracts/` — Solidity (Foundry): agent infrastructure contracts.
- `gateway/` — TypeScript: MPP server + REST API (scaffolded).
- `devnet/` — genesis patch + custom allocs.
- `docs/` — project documentation (see `docs/porting-notes.md`).
- `sepolia/` — Sepolia testnet deploy tooling.

## Build

```bash
# Check the SCI crates only (fast)
cargo check -p sci-precompiles -p sci-precompiles-macros \
            -p sci-precompile-abi -p sci-revm-shim -p tempo-chainspec-shim

# Run the SCI tests
cargo test -p sci-precompiles
```
