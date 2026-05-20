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
//! [`run_pre_execution_hook`] at the appropriate point.
//!
//! Design locked 2026-05-20 (see CLAUDE.md "Pre-execution Hook Design"):
//! - **Agent tx identification (Q1)**: `code(tx.to)` is an EIP-7702 delegation pointing
//!   at `SCI_AGENT_DELEGATOR_ADDRESS`, AND `keys[tx.to][tx.from]` is active.
//! - **Per-call check placement (Q2)**: Rust hook decodes `execute(Call[])`, validates
//!   each call, and aborts the whole batch on any failure.
//! - **`CircuitBreaker` state (Q3)**: in a separate [`crate::SciAgentState`] precompile.
//! - **Refund semantics (Q4)**: R1 — hook writes go through journaled state, revm
//!   auto-rolls back on revert. Pessimistic deduction for `approve`.

pub mod decode;
mod hook;

pub use hook::{HookOutcome, apply_post_execution_deductions, run_pre_execution_hook};
