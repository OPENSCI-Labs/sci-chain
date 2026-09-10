# Third-Party Notices

## Tempo (tempoxyz/tempo)

This repository includes software adapted from
[Tempo](https://github.com/tempoxyz/tempo) v1.7.1
(commit [`50649558cd13baab559ae6da9f44364e23f165b1`](https://github.com/tempoxyz/tempo/tree/v1.7.1)),
Copyright (c) 2025 Tempo Contributors. Tempo is dual-licensed under the Apache
License, Version 2.0 and the MIT License ("at your option"); SCI Chain
redistributes the derived code below under the MIT License option. The full MIT
license text and copyright notice are retained at the end of this section, as
required by the license.

### Derived files

The following files are adapted from Tempo v1.7.1. Adaptations fall into three
categories: import routing via Cargo `package =` renames (so upstream identifiers
such as `tempo_contracts` resolve to SCI crates), adaptation from Tempo's revm 38
stack to SCI Chain's revm 34 / Base v0.9 execution stack (most extensively in
`storage/`, where `evm.rs` was substantially rewritten against the revm 34 journal
API), and minor formatting / test harness adjustments. A detailed change log is
maintained in [`sci/docs/porting-notes.md`](sci/docs/porting-notes.md).

| SCI Chain file | Upstream source (tempoxyz/tempo v1.7.1) |
| --- | --- |
| `sci/crates/precompiles/src/account_keychain/mod.rs` | `crates/precompiles/src/account_keychain/mod.rs` (plus one added `mod sci_ext;` line) |
| `sci/crates/precompiles/src/account_keychain/dispatch.rs` | `crates/precompiles/src/account_keychain/dispatch.rs` |
| `sci/crates/precompiles/src/storage/**` | `crates/precompiles/src/storage/**` (esp. `evm.rs`, substantially adapted to revm 34) |
| `sci/crates/precompile-abi/src/precompiles/account_keychain.rs` | `crates/contracts/src/precompiles/account_keychain.rs` (plus SCI re-exports and call renames) |
| `sci/crates/precompiles-macros/src/*.rs` | `crates/precompiles-macros/src/*.rs` |
| `sci/contracts/src/interfaces/IAccountKeychain.sol` | `IAccountKeychain` interface definitions in `crates/contracts/src/precompiles/account_keychain.rs` |
| `sci/contracts/test/mocks/MockAccountKeychain.sol` | Same interface definitions (SCI test helper) |

`sci/crates/precompiles/src/account_keychain/sci_ext.rs` and
`sci/crates/precompiles/src/handler/` are SCI-original code, not derived from
Tempo. When citing this work, please describe the Account Keychain precompile
as *adapted from* tempoxyz/tempo v1.7.1 — not copied verbatim, as the source
has been modified as described above.

### MIT License (Tempo)

The MIT License (MIT)

Copyright (c) 2025 Tempo Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
