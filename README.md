![Base](docs/assets/logo.png)

# Base

Base is a rollup built on Ethereum.

## SCI Chain

This repository is **SCI Chain** — an agent-native Ethereum L2 forked from Base
(`base/base`, Azul v0.9). Chain ID `42001` (L1 = Sepolia).

### Borrowed from Tempo

At the time of development (May 2026), Base had not yet released its native
account-abstraction (EIP-8130) or native token standard (B20). Tempo
(`tempoxyz/tempo`) had already implemented the equivalent primitives, so SCI Chain
borrowed from Tempo rather than re-implementing them:

- **Keychain precompile** — ported from Tempo v1.7.1 via `revm-shim` compatibility
  crates. Enforces the per-agent permission sandbox (scope, selector, spending limits)
  through a Rust pre-execution hook.
- **TIP-20 token interface** — Tempo's token standard, mapped to standard ERC-20
  (SCI Chain does not ship a TIP-20 factory).

### Other SCI-specific changes

- **Native account abstraction** — custom AA transaction type `0x76` (Plan A) with
  batched `calls[]` and `fee_payer` sponsored gas.
- **Native gas** — SCI enabled as the native gas token via OP-Stack CGT v2.
- **Agent contracts** — `sci/contracts` (Foundry): `AgentAccessKeyRegistry`,
  `AgentCircuitBreaker`, and `AgentBudgetController`.
- **MPP (Machine Payments Protocol)** — planned agent access layer; scaffolded only
  (`sci/gateway/`, not yet implemented).

## Why Base
- **Cheap, fast, and open platform:** Base is a globally available platform that provides 1-second and <1-cent transactions to anyone in the world.
- **Reach more users:** Base is committed to helping developers grow their user base by distributing their apps through official Base channels.
- **A place to earn:** Base has delivered grants to more than 1,000 builders, with plans to continue supporting more.
- **Access to high-quality tooling:** Builders have access to tools to build incredible onchain experiences for AI, social, media, and entertainment.

## Learn More

- Visit the [docs](https://docs.base.org) for information on how to:
    - [Connect your wallet](https://docs.base.org/base-chain/quickstart/connecting-to-base)
    - [Run a node](https://docs.base.org/base-chain/node-operators/run-a-base-node)
    - [Deploy an app](https://docs.base.org/base-chain/quickstart/deploy-on-base)
- The [specs](https://specs.base.org) site has an overview of the protocol, including past and upcoming upgrades.

## Install Binaries

Use [`baseup`](baseup/README.md) to install the GitHub release binaries for this repository:

```bash
curl -fsSL https://raw.githubusercontent.com/base/base/main/baseup/install | bash
```

## License

Licensed under [MIT](LICENSE).
