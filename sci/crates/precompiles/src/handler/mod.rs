//! Pre-execution hook for SCI Chain agent txs.
//!
//! This module is the **logic** side of the hook: it detects whether a tx is an agent
//! tx, runs `CircuitBreaker` / scope / spending-limit checks, and reports its outcome to
//! the caller. It is deliberately decoupled from Base's `OpHandler` so that
//! `sci-precompiles` can stay independent of `base-common-evm` (which already depends on
//! us — wiring it the other way would create a cycle).
//!
//! The matching **wrapper** lives at `crates/common/evm/src/sci_handler.rs`. That Base
//! file implements the `Handler` trait by delegating to `OpHandler` and calling
//! [`run_aa_keychain_hook`] at the appropriate point.
//!
//! Design (Plan A, native AA tx `0x76`):
//! - **Agent tx identification**: the tx is an AA tx (`0x76`) whose `root` is set; the
//!   `SciHandler` passes `(root, session_key, calls)` in, and `keys[root][session_key]`
//!   must be an active access key.
//! - **Per-call check placement**: the hook validates each call's scope and pre-flights
//!   spending limits, aborting the whole batch on any failure.
//! - **`CircuitBreaker` state**: in a separate [`crate::SciAgentState`] precompile.
//! - **Refund semantics (Q4)**: read-only pre-flight; real deductions deferred to
//!   [`apply_aa_post_execution_deductions`] on body success (strong-R1). Pessimistic
//!   deduction for `approve`.

pub mod decode;
mod hook;

pub use hook::{
    AaCall, HookOutcome, apply_aa_post_execution_deductions, run_aa_keychain_hook,
};
