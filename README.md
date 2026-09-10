![Base](docs/assets/logo.png)

# SCI Chain

SCI Chain is the blockchain base layer of **OPENSCI**, the global open-science
initiative of the **World Laureates Association (WLA)**. OPENSCI's mission is
**scientist-led governance**: making research contributions — data, hypotheses,
results — *attributable, traceable, and rewardable*, and shortening the basic-science
discovery cycle with AI and blockchain.

It is implemented today as an **agent-native Ethereum L2** — a fork of Base
(`base/base`, Azul v0.9) whose execution layer enforces agent permissions (keys,
authorization, spending limits) itself rather than through smart contracts. Its
consensus layer targets **PoSR (Proof of Scientific Research)** as the long-term
mechanism for on-chain trust in scientific contributions.

Chain ID `42001` (testnet on Sepolia).

## Where SCI Chain fits in OPENSCI

OPENSCI has three layers; SCI Chain is the bottom one:

1. **SCI Chain** — the on-chain trust base (this repository).
2. **ASCI** — a general-purpose scientific agent (autonomous reasoning, literature
   retrieval, experiment optimization) that runs on SCI Chain.
3. **Application layer** — OpenGrants (decentralized research funding), OpenWorld
   (AI-driven collaboration), IDA Layer (result assetization), and SCI Network
   (collaboration and impact tracking).

## Why agent-native

Scientific agents must hold keys, spend money, and be stopped instantly when
something goes wrong. A plain EVM L2 cannot express any of this as a protocol
guarantee — agent keys end up in contracts (readable by anyone), permissions are
app-level, and spending limits are advisory. To let agents like ASCI do research
autonomously while staying governable, SCI Chain moves these primitives into the
execution layer so they are enforced, metered, and provable:

1. **Protocol-level permissions.** Keys, authorization, and spending limits live in
   the chain (keychain precompile + pre-execution hook), not in contracts.
2. **Native gas.** SCI is the chain's native gas token, so fees, agent-call costs,
   and gas settle in one token and flow back to the treasury.

## Architecture

```
Agent (session key) ──► SCI Chain (Base Azul v0.9 fork)
                        │  AA tx type 0x76: calls[] + fee_payer
                        ├─ Pre-execution hook:
                        │    CircuitBreaker → Scope → SpendingLimit
                        ├─ Precompile 0xAAAA…00: AccountKeychain
                        ├─ Precompile 0xAAAA…01: SciAgentState
                        └─ Predeploys 0xBBBB…01/02/03: Registry / Budget / Breaker
```

## The three core pieces

### Keychain precompile — why native, not a contract

The `AccountKeychain` precompile (`0xAAAA…00`) is a stateful precompile storing
`keys[root][session_key]` bindings with per-token spending limits and call scopes
(target → selector → recipient). Agent keys live at the protocol layer because a
contract cannot keep keys secret, verify P256/WebAuthn signatures cheaply, or make
authorization and revocation unbypassable. Packing a key's expiry, limits, and scope
into a single storage slot also saves gas versus a multi-mapping contract.

### revm-shim — bridging revm 34 ↔ 38

Base Azul v0.9 pins revm 34; the ported Tempo source targets revm 38 (the EIP-8037
state-gas release). Rather than rewrite Tempo's code, a 484-line additive shim
(`sci/crates/revm-shim`) re-exports revm 34 verbatim and shadows only the two modules
revm 38 changed (`precompile`, `interpreter::gas`), folding outputs back at the
boundary via `to_revm34`. This keeps the ported business source close to upstream and
isolates SCI from Tempo's later revm upgrades. See `sci/docs/porting-notes.md`.

### Pre-execution hook — the sandbox gate

A Rust hook (`SciHandler` wrapping `BaseHandler`) intercepts every agent transaction
before execution and enforces three checks: **CircuitBreaker** (session key not
tripped), **Scope** (each call within the key's target/selector/recipient rules), and
**SpendingLimit** (a read-only pre-flight check; deduction is deferred until the body
succeeds, so a reverting transaction spends no quota).

## Borrowed from Tempo

When SCI Chain was built (mid-2026), Base had not yet shipped native account
abstraction (EIP-8130) or a native token standard (B20). Tempo (`tempoxyz/tempo`) had
already implemented the equivalents, so SCI borrowed rather than re-implemented:

- **Keychain precompile** — adapted from Tempo v1.7.1 via `revm-shim` (see
  [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) for the derived-file list and
  license attribution).
- **Token interface** — Tempo's TIP-20 mapped to standard ERC-20 (SCI does not ship a
  TIP-20 factory).

## SCI-specific changes

- **Native account abstraction** — AA transaction type `0x76` (Plan A) with batched
  `calls[]` and `fee_payer` sponsored gas (gas metered into the keychain limit).
- **Native gas** — SCI as the native gas token via OP-Stack CGT v2.
- **Agent contracts** — `sci/contracts`: `AgentAccessKeyRegistry`,
  `AgentCircuitBreaker`, `AgentBudgetController`.
- **MPP (Machine Payments Protocol)** — agent access layer (planned).

## Build

```bash
cargo build --release -p base
```

See [`sci/README.md`](sci/README.md) for the SCI extension layout and
[`sci/docs/porting-notes.md`](sci/docs/porting-notes.md) for a deep technical writeup.

## Learn More

- [docs.base.org](https://docs.base.org) — Base platform docs (wallets, running a node, deploying).
- [specs.base.org](https://specs.base.org) — protocol overview and upgrades.

## License

Licensed under [MIT](LICENSE).
