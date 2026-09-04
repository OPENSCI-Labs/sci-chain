//! # sci-revm-shim
//!
//! Compat shim that maps Tempo v1.7.1's revm 38 API surface (introduced by
//! EIP-8037 / TIP-1016 "state gas + reservoir") onto Base v0.9's revm 34.
//!
//! ## How it is wired
//!
//! Consumed by `sci-precompiles` via a Cargo `package = ...` rename:
//!
//! ```toml
//! # sci/crates/precompiles/Cargo.toml
//! revm = { workspace = true, package = "sci-revm-shim" }
//! ```
//!
//! With that alias in place, every `use revm::...` inside `sci-precompiles`
//! resolves through this crate. Verbatim Tempo source files that import
//! `revm::precompile::PrecompileHalt`, construct
//! `PrecompileOutput::halt(reason, reservoir)`, etc. compile unmodified.
//!
//! ## What the shim does
//!
//! - The [`precompile`] submodule **shadows** revm 34's `precompile` module:
//!   it re-exports everything verbatim *except* `PrecompileOutput` and
//!   `PrecompileResult`, and replaces those with SCI newtypes that carry the
//!   v38-shape fields (`state_gas_used`, `reservoir`, `status`). A new
//!   [`precompile::PrecompileHalt`] enum is added — there is no equivalent in
//!   revm 34 (OOG was signalled by `Err(PrecompileError::OutOfGas)`).
//! - All other revm 34 submodules (`context`, `handler`, `primitives`, etc.)
//!   are re-exported verbatim, so any path the verbatim Tempo source touches
//!   outside `precompile::` keeps working without any wrapper.
//! - [`precompile::to_revm34`] is the boundary function: invoked inside
//!   `sci-precompiles::install` it converts the shim's
//!   [`precompile::PrecompileResult`] back into revm 34's native
//!   `Result<revm::precompile::PrecompileOutput, PrecompileError>`. The halt
//!   variants fold into `Err(...)`; success / revert preserve the underlying
//!   four fields verbatim.
//!
//! ## Scope
//!
//! Only `sci-precompiles` consumes the shim. Base crates and other SCI crates
//! continue to depend on real revm 34 directly. The shim is *additive* — it
//! never removes any revm 34 item. Adding v38 surface here does not perturb
//! anything downstream that still binds to real `revm::precompile::PrecompileOutput`.

pub mod gas_params_ext;
pub mod interpreter;
pub mod precompile;

// Re-export every revm 34 top-level submodule except `precompile` and
// `interpreter` (both shadowed above). Tempo verbatim source like
// `revm::handler::EthPrecompiles` or `revm::context::CfgEnv` resolves through
// these.
// Re-export the v38 state-gas extension trait at the crate root so verbatim
// Tempo source can bring it into scope with `use revm::GasParamsExt;`.
pub use gas_params_ext::GasParamsExt;
pub use precompile::to_revm34;
// Top-level item re-exports (mirror revm 34's `lib.rs`).
pub use revm::{
    Context, Database, DatabaseCommit, DatabaseRef, ExecuteCommitEvm, ExecuteEvm, InspectCommitEvm,
    InspectEvm, InspectSystemCallEvm, Inspector, Journal, JournalEntry, MainBuilder, MainContext,
    MainnetEvm, SystemCallCommitEvm, SystemCallEvm,
};
pub use revm::{
    bytecode, context, context_interface, database, database_interface, handler, inspector,
    primitives, state,
};
