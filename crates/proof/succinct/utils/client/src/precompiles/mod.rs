//! [`PrecompileProvider`] for FPVM-accelerated rollup precompiles.

use alloc::string::String;

use alloy_evm::precompiles::Precompile as _;
use alloy_primitives::{Address, Bytes};
use base_common_evm::{BasePrecompiles, BaseSpecId};
use revm::{
    context::{Cfg, ContextTr},
    handler::{EthPrecompiles, PrecompileProvider},
    interpreter::{CallInput, CallInputs, Gas, InstructionResult, InterpreterResult},
    precompile::{PrecompileError, Precompiles},
    primitives::hardfork::SpecId,
};
#[cfg(any(test, target_os = "zkvm"))]
use revm_precompile::PrecompileId;

mod custom;
pub use custom::CustomCrypto;

mod factory;
pub use factory::ZkvmBaseEvmFactory;

/// Tracker names for accelerated precompiles.
/// These names are used in cycle-tracker-report events and must match
/// the keys expected by stats.rs and validity/src/types.rs.
pub mod cycle_tracker {
    /// Prefix for all precompile cycle tracker keys.
    pub const PREFIX: &str = "precompile-";

    /// Individual tracker names (without prefix).
    pub mod names {
        /// BN254 addition.
        pub const BN_ADD: &str = "bn-add";
        /// BN254 scalar multiplication.
        pub const BN_MUL: &str = "bn-mul";
        /// BN254 pairing check.
        pub const BN_PAIR: &str = "bn-pair";
        /// ECDSA recovery.
        pub const EC_RECOVER: &str = "ec-recover";
        /// P-256 signature verification.
        pub const P256_VERIFY: &str = "p256-verify";
        /// KZG point evaluation.
        pub const KZG_EVAL: &str = "kzg-eval";
    }

    /// Full cycle tracker keys (with "precompile-" prefix).
    /// These match the keys in `ExecutionReport.cycle_tracker`.
    pub mod keys {
        /// BN254 addition (prefixed).
        pub const BN_ADD: &str = "precompile-bn-add";
        /// BN254 scalar multiplication (prefixed).
        pub const BN_MUL: &str = "precompile-bn-mul";
        /// BN254 pairing check (prefixed).
        pub const BN_PAIR: &str = "precompile-bn-pair";
        /// ECDSA recovery (prefixed).
        pub const EC_RECOVER: &str = "precompile-ec-recover";
        /// P-256 signature verification (prefixed).
        pub const P256_VERIFY: &str = "precompile-p256-verify";
        /// KZG point evaluation (prefixed).
        pub const KZG_EVAL: &str = "precompile-kzg-eval";
    }
}

fn get_or_create_precompiles(spec: BaseSpecId) -> &'static Precompiles {
    BasePrecompiles::new_with_spec(spec).precompiles()
}

/// Get the cycle tracker name for a precompile by its ID.
/// Returns None if the precompile is not accelerated/tracked.
#[cfg(any(test, target_os = "zkvm"))]
#[inline]
const fn get_precompile_tracker_name(id: &PrecompileId) -> Option<&'static str> {
    match id {
        PrecompileId::Bn254Add => Some(cycle_tracker::names::BN_ADD),
        PrecompileId::Bn254Mul => Some(cycle_tracker::names::BN_MUL),
        PrecompileId::Bn254Pairing => Some(cycle_tracker::names::BN_PAIR),
        PrecompileId::EcRec => Some(cycle_tracker::names::EC_RECOVER),
        PrecompileId::P256Verify => Some(cycle_tracker::names::P256_VERIFY),
        PrecompileId::KzgPointEvaluation => Some(cycle_tracker::names::KZG_EVAL),
        _ => None,
    }
}

/// The ZKVM-cycle-tracking precompiles.
#[derive(Debug)]
pub struct BaseZkvmPrecompiles {
    /// The default [`EthPrecompiles`] provider.
    inner: EthPrecompiles,
    /// The [`BaseSpecId`] of the precompiles.
    spec: BaseSpecId,
}

impl BaseZkvmPrecompiles {
    /// Create a new precompile provider with the given [`BaseSpecId`].
    #[inline]
    pub fn new_with_spec(spec: BaseSpecId) -> Self {
        let precompiles = get_or_create_precompiles(spec);
        Self { inner: EthPrecompiles { precompiles, spec: SpecId::default() }, spec }
    }
}

impl<CTX> PrecompileProvider<CTX> for BaseZkvmPrecompiles
where
    CTX: ContextTr<
            Cfg: Cfg<Spec = BaseSpecId>,
            Db: alloy_evm::Database,
            Journal: revm::context_interface::JournalTr<Database: alloy_evm::Database>
                         + core::fmt::Debug,
        >,
{
    type Output = InterpreterResult;

    #[inline]
    fn set_spec(&mut self, spec: <CTX::Cfg as Cfg>::Spec) -> bool {
        if spec == self.spec {
            return false;
        }
        *self = Self::new_with_spec(spec);
        true
    }

    #[inline]
    fn run(
        &mut self,
        context: &mut CTX,
        inputs: &CallInputs,
    ) -> Result<Option<Self::Output>, String> {
        let mut result = InterpreterResult {
            result: InstructionResult::Return,
            gas: Gas::new(inputs.gas_limit),
            output: Bytes::new(),
        };

        use revm::context::LocalContextTr;

        // SCI parity: the EL resolves the AccountKeychain / SciAgentState precompiles
        // through the `PrecompilesMap` lookup installed by `sci_precompiles::install`;
        // the zkVM provider must resolve the same addresses to the same implementations
        // or verifier execution diverges from the sequencer on any direct call to them
        // (a call would hit the `0xef` genesis placeholder code instead — audit finding
        // "Caveat B"). This branch mirrors `PrecompilesMap::run` exactly: lookup-based
        // addresses stay COLD (deliberately NOT added to `warm_addresses`, matching the
        // EL's documented lookup semantics), and reverted outputs carry their bytes.
        if sci_precompiles::is_sci_precompile_address(&inputs.bytecode_address) {
            let gas_params = context.cfg().gas_params().clone();
            let precompile =
                sci_precompiles::lookup_precompile(&inputs.bytecode_address, &gas_params)
                    .expect("is_sci_precompile_address implies a lookup hit");

            let (block, tx, cfg, journaled_state, _, local) = context.all_mut();
            let r;
            let input_bytes = match &inputs.input {
                CallInput::SharedBuffer(range) => {
                    #[allow(clippy::option_if_let_else)]
                    if let Some(slice) = local.shared_memory_buffer_slice(range.clone()) {
                        r = slice;
                        &*r
                    } else {
                        &[]
                    }
                }
                CallInput::Bytes(bytes) => bytes.as_ref(),
            };

            let precompile_result = precompile.call(alloy_evm::precompiles::PrecompileInput {
                data: input_bytes,
                gas: inputs.gas_limit,
                caller: inputs.caller,
                value: inputs.call_value(),
                is_static: inputs.is_static,
                internals: alloy_evm::EvmInternals::new(journaled_state, block, cfg, tx),
                target_address: inputs.target_address,
                bytecode_address: inputs.bytecode_address,
            });

            match precompile_result {
                Ok(output) => {
                    let underflow = result.gas.record_cost(output.gas_used);
                    assert!(underflow, "Gas underflow is not possible");
                    result.result = if output.reverted {
                        InstructionResult::Revert
                    } else {
                        InstructionResult::Return
                    };
                    result.output = output.bytes;
                }
                Err(PrecompileError::Fatal(e)) => return Err(e),
                Err(e) => {
                    result.result = if e.is_oog() {
                        InstructionResult::PrecompileOOG
                    } else {
                        InstructionResult::PrecompileError
                    };
                }
            }

            return Ok(Some(result));
        }
        // NOTE: this snippet is refactored from the revm source code.
        // See https://github.com/bluealloy/revm/blob/9bc0c04fda0891e0e8d2e2a6dfd0af81c2af18c4/crates/handler/src/precompile_provider.rs#L111-L122.
        let shared_buffer;
        let input_bytes = match &inputs.input {
            CallInput::SharedBuffer(range) => {
                shared_buffer = context.local().shared_memory_buffer_slice(range.clone());
                shared_buffer.as_deref().unwrap_or(&[])
            }
            CallInput::Bytes(bytes) => bytes.0.iter().as_slice(),
        };

        // Priority:
        // 1. If the precompile has an accelerated version, use that.
        // 2. If the precompile is not accelerated, use the default version.
        // 3. If the precompile is not found, return None.
        let output = if let Some(precompile) = self.inner.precompiles.get(&inputs.bytecode_address)
        {
            // Track cycles for accelerated precompiles
            #[cfg(target_os = "zkvm")]
            let tracker_name = get_precompile_tracker_name(precompile.id());

            #[cfg(target_os = "zkvm")]
            if let Some(name) = tracker_name {
                println!("cycle-tracker-report-start: precompile-{}", name);
            }

            let result = precompile.execute(input_bytes, inputs.gas_limit);

            #[cfg(target_os = "zkvm")]
            if let Some(name) = tracker_name {
                println!("cycle-tracker-report-end: precompile-{}", name);
            }

            result
        } else {
            return Ok(None);
        };

        match output {
            Ok(output) => {
                let underflow = result.gas.record_cost(output.gas_used);
                assert!(underflow, "Gas underflow is not possible");
                result.result = InstructionResult::Return;
                result.output = output.bytes;
            }
            Err(PrecompileError::Fatal(e)) => return Err(e),
            Err(e) => {
                result.result = if e.is_oog() {
                    InstructionResult::PrecompileOOG
                } else {
                    InstructionResult::PrecompileError
                };
            }
        }

        Ok(Some(result))
    }

    #[inline]
    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        self.inner.warm_addresses()
    }

    #[inline]
    fn contains(&self, address: &Address) -> bool {
        // SCI parity: the EL's `PrecompilesMap::contains` consults the SCI lookup, so a
        // call to these addresses is treated as a precompile call there — mirror that.
        sci_precompiles::is_sci_precompile_address(address) || self.inner.contains(address)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use alloy_primitives::U256;
    use base_common_evm::{BaseContext, BaseUpgrade, DefaultBase as _};
    use revm::{
        Context,
        database::EmptyDB,
        handler::PrecompileProvider,
        interpreter::{CallInput, CallScheme, CallValue},
    };
    use revm_precompile::{PrecompileId, secp256r1};

    use super::*;

    type TestContext = BaseContext<EmptyDB>;

    /// Creates a [`CallInputs`] with `bytecode_address` set to the given address
    /// and `target_address` set to zero, simulating a DELEGATECALL scenario.
    fn create_call_inputs(address: Address, input: Bytes, gas_limit: u64) -> CallInputs {
        CallInputs {
            input: CallInput::Bytes(input),
            gas_limit,
            bytecode_address: address,
            target_address: Address::ZERO, // Simulates DELEGATECALL context
            caller: Address::ZERO,
            value: CallValue::Transfer(U256::ZERO),
            scheme: CallScheme::Call,
            is_static: false,
            return_memory_offset: 0..0,
            known_bytecode: None,
        }
    }

    fn create_test_context() -> TestContext {
        Context::base().with_db(EmptyDB::new())
    }

    // ===== Precompile Provider Functional Tests =====

    /// Test that precompiles are looked up by `bytecode_address`, not `target_address`.
    /// This is critical for DELEGATECALL scenarios where these addresses differ.
    #[test]
    fn test_precompile_lookup_uses_bytecode_address() {
        let mut ctx = create_test_context();
        let mut precompiles =
            BaseZkvmPrecompiles::new_with_spec(BaseSpecId::new(BaseUpgrade::Bedrock));

        // SHA256 precompile at address 0x02
        let sha256_addr = revm::precompile::u64_to_address(2);

        // Create inputs where bytecode_address != target_address (DELEGATECALL scenario)
        let call_inputs = create_call_inputs(sha256_addr, Bytes::from_static(b"test"), u64::MAX);

        // Verify target_address is different from bytecode_address
        assert_ne!(call_inputs.bytecode_address, call_inputs.target_address);

        // Should find the precompile via bytecode_address
        let result = precompiles.run(&mut ctx, &call_inputs).unwrap();
        assert!(result.is_some(), "Precompile should be found via bytecode_address");

        let interpreter_result = result.unwrap();
        assert_eq!(interpreter_result.result, InstructionResult::Return);
        assert!(!interpreter_result.output.is_empty());
    }

    /// Test that a non-existent precompile returns None.
    #[test]
    fn test_run_nonexistent_precompile() {
        let mut ctx = create_test_context();
        let mut precompiles =
            BaseZkvmPrecompiles::new_with_spec(BaseSpecId::new(BaseUpgrade::Bedrock));

        let fake_addr = Address::from_slice(&[0xFFu8; 20]);
        let call_inputs = create_call_inputs(fake_addr, Bytes::new(), u64::MAX);

        let result = precompiles.run(&mut ctx, &call_inputs).unwrap();
        assert!(result.is_none());
    }

    /// Test out-of-gas handling for precompiles.
    #[test]
    fn test_run_out_of_gas() {
        let mut ctx = create_test_context();
        let mut precompiles =
            BaseZkvmPrecompiles::new_with_spec(BaseSpecId::new(BaseUpgrade::Bedrock));

        let sha256_addr = revm::precompile::u64_to_address(2);
        let call_inputs = create_call_inputs(sha256_addr, Bytes::from_static(b"test"), 0);

        let result = precompiles.run(&mut ctx, &call_inputs).unwrap();
        assert!(result.is_some());

        let interpreter_result = result.unwrap();
        assert_eq!(interpreter_result.result, InstructionResult::PrecompileOOG);
    }

    /// Test `SharedBuffer` input handling.
    #[test]
    fn test_run_with_shared_buffer_empty() {
        let mut ctx = create_test_context();
        let mut precompiles =
            BaseZkvmPrecompiles::new_with_spec(BaseSpecId::new(BaseUpgrade::Bedrock));

        let sha256_addr = revm::precompile::u64_to_address(2);
        let call_inputs = CallInputs {
            input: CallInput::SharedBuffer(0..0),
            gas_limit: u64::MAX,
            bytecode_address: sha256_addr,
            target_address: Address::ZERO,
            caller: Address::ZERO,
            value: CallValue::Transfer(U256::ZERO),
            scheme: CallScheme::Call,
            is_static: false,
            return_memory_offset: 0..0,
            known_bytecode: None,
        };

        let result = precompiles.run(&mut ctx, &call_inputs).unwrap();
        assert!(result.is_some());
    }

    // ===== Cycle Tracker Name Tests =====

    #[test]
    fn test_precompile_tracker_name_bn_add() {
        assert_eq!(
            get_precompile_tracker_name(&PrecompileId::Bn254Add),
            Some(cycle_tracker::names::BN_ADD)
        );
    }

    #[test]
    fn test_precompile_tracker_name_bn_mul() {
        assert_eq!(
            get_precompile_tracker_name(&PrecompileId::Bn254Mul),
            Some(cycle_tracker::names::BN_MUL)
        );
    }

    #[test]
    fn test_precompile_tracker_name_bn_pair() {
        assert_eq!(
            get_precompile_tracker_name(&PrecompileId::Bn254Pairing),
            Some(cycle_tracker::names::BN_PAIR)
        );
    }

    #[test]
    fn test_precompile_tracker_name_ecrecover() {
        assert_eq!(
            get_precompile_tracker_name(&PrecompileId::EcRec),
            Some(cycle_tracker::names::EC_RECOVER)
        );
    }

    #[test]
    fn test_precompile_tracker_name_p256verify() {
        assert_eq!(
            get_precompile_tracker_name(&PrecompileId::P256Verify),
            Some(cycle_tracker::names::P256_VERIFY)
        );
    }

    #[test]
    fn test_precompile_tracker_name_kzg_eval() {
        assert_eq!(
            get_precompile_tracker_name(&PrecompileId::KzgPointEvaluation),
            Some(cycle_tracker::names::KZG_EVAL)
        );
    }

    #[test]
    fn test_unknown_precompile_returns_none() {
        // SHA256 is a precompile but not accelerated/tracked
        assert_eq!(get_precompile_tracker_name(&PrecompileId::Sha256), None);
        assert_eq!(get_precompile_tracker_name(&PrecompileId::Identity), None);
    }

    // ===== Consistency Tests =====

    #[test]
    fn test_zkvm_precompiles_match_base_evm_precompiles() {
        for spec in BaseUpgrade::VARIANTS.iter().copied().map(BaseSpecId::new) {
            let base_precompiles = BasePrecompiles::new_with_spec(spec);
            let zkvm_precompiles = BaseZkvmPrecompiles::new_with_spec(spec);

            let base_addresses: Vec<_> =
                <BasePrecompiles as PrecompileProvider<TestContext>>::warm_addresses(
                    &base_precompiles,
                )
                .collect();
            let zkvm_addresses: Vec<_> =
                <BaseZkvmPrecompiles as PrecompileProvider<TestContext>>::warm_addresses(
                    &zkvm_precompiles,
                )
                .collect();

            assert_eq!(
                zkvm_addresses.len(),
                base_addresses.len(),
                "ZKVM and Base EVM precompile counts must match for {spec:?}",
            );

            for address in &base_addresses {
                assert!(
                    <BaseZkvmPrecompiles as PrecompileProvider<TestContext>>::contains(
                        &zkvm_precompiles,
                        address,
                    ),
                    "ZKVM precompiles missing Base EVM precompile {address:?} for {spec:?}",
                );
            }

            for address in &zkvm_addresses {
                assert!(
                    <BasePrecompiles as PrecompileProvider<TestContext>>::contains(
                        &base_precompiles,
                        address,
                    ),
                    "ZKVM precompiles contain non-Base EVM precompile {address:?} for {spec:?}",
                );
            }
        }
    }

    #[test]
    fn test_tracker_keys_match_expected_format() {
        let expected_keys = [
            cycle_tracker::keys::BN_ADD,
            cycle_tracker::keys::BN_MUL,
            cycle_tracker::keys::BN_PAIR,
            cycle_tracker::keys::EC_RECOVER,
            cycle_tracker::keys::P256_VERIFY,
            cycle_tracker::keys::KZG_EVAL,
        ];

        for key in &expected_keys {
            assert!(
                key.starts_with(cycle_tracker::PREFIX),
                "Key '{}' should start with '{}'",
                key,
                cycle_tracker::PREFIX
            );
            assert!(!key.contains(' '), "Key '{key}' contains spaces");
            assert!(
                !key[cycle_tracker::PREFIX.len()..].contains('_'),
                "Key '{key}' contains underscores (should use dashes)"
            );
        }
    }

    #[test]
    fn test_azul_uses_osaka_p256verify() {
        let p256_addr = *secp256r1::P256VERIFY.address();

        let jovian_set = get_or_create_precompiles(BaseSpecId::new(BaseUpgrade::Jovian));
        let azul_set = get_or_create_precompiles(BaseSpecId::new(BaseUpgrade::Azul));

        let jovian_p256 = jovian_set.get(&p256_addr).expect("JOVIAN must have P256VERIFY");
        let azul_p256 = azul_set.get(&p256_addr).expect("AZUL must have P256VERIFY");

        // Legacy P256VERIFY costs 3,450 gas. With 5,000 gas it should succeed.
        assert!(
            jovian_p256.execute(&[], 5_000).is_ok(),
            "JOVIAN P256VERIFY must succeed with 5,000 gas (legacy pricing, 3,450 base fee)",
        );

        // Osaka P256VERIFY costs 6,900 gas. With 5,000 gas it must fail with OOG.
        assert!(
            matches!(azul_p256.execute(&[], 5_000), Err(PrecompileError::OutOfGas)),
            "AZUL P256VERIFY must fail with 5,000 gas (Osaka pricing, 6,900 base fee)",
        );
    }

    #[test]
    fn test_names_and_keys_are_consistent() {
        assert_eq!(
            cycle_tracker::keys::BN_ADD,
            format!("{}{}", cycle_tracker::PREFIX, cycle_tracker::names::BN_ADD)
        );
        assert_eq!(
            cycle_tracker::keys::BN_MUL,
            format!("{}{}", cycle_tracker::PREFIX, cycle_tracker::names::BN_MUL)
        );
        assert_eq!(
            cycle_tracker::keys::BN_PAIR,
            format!("{}{}", cycle_tracker::PREFIX, cycle_tracker::names::BN_PAIR)
        );
        assert_eq!(
            cycle_tracker::keys::EC_RECOVER,
            format!("{}{}", cycle_tracker::PREFIX, cycle_tracker::names::EC_RECOVER)
        );
        assert_eq!(
            cycle_tracker::keys::P256_VERIFY,
            format!("{}{}", cycle_tracker::PREFIX, cycle_tracker::names::P256_VERIFY)
        );
        assert_eq!(
            cycle_tracker::keys::KZG_EVAL,
            format!("{}{}", cycle_tracker::PREFIX, cycle_tracker::names::KZG_EVAL)
        );
    }

    // ===== SCI precompile parity (audit finding "Caveat B") =====

    alloy_sol_types::sol! {
        function getKey(address account, address keyId);
        function isTripped(address sessionKey);
    }

    /// Builds direct-call inputs (`target == bytecode`, unlike the DELEGATECALL helper
    /// above) — SCI precompiles reject non-direct calls.
    fn direct_call_inputs(address: Address, input: Bytes, gas_limit: u64) -> CallInputs {
        CallInputs {
            input: CallInput::Bytes(input),
            gas_limit,
            bytecode_address: address,
            target_address: address,
            caller: Address::ZERO,
            value: CallValue::Transfer(U256::ZERO),
            scheme: CallScheme::Call,
            is_static: false,
            return_memory_offset: 0..0,
            known_bytecode: None,
        }
    }

    /// The zkVM provider must resolve the AccountKeychain precompile exactly like the
    /// EL's `PrecompilesMap` lookup does — a direct `getKey` call executes the keychain
    /// (returning a zeroed KeyInfo on empty state) instead of falling through to the
    /// `0xef` placeholder account code.
    #[test]
    fn test_sci_keychain_call_resolves() {
        use alloy_sol_types::SolCall;
        let mut ctx = create_test_context();
        let mut precompiles =
            BaseZkvmPrecompiles::new_with_spec(BaseSpecId::new(BaseUpgrade::Bedrock));

        let data =
            getKeyCall { account: Address::ZERO, keyId: Address::from([0x11; 20]) }.abi_encode();
        let inputs = direct_call_inputs(
            sci_precompiles::ACCOUNT_KEYCHAIN_ADDRESS,
            Bytes::from(data),
            1_000_000,
        );

        let result = precompiles
            .run(&mut ctx, &inputs)
            .expect("no fatal error")
            .expect("keychain address must resolve to a precompile");
        assert_eq!(result.result, InstructionResult::Return, "getKey on empty state returns Ok");
        assert!(!result.output.is_empty(), "getKey returns an ABI-encoded KeyInfo");
    }

    /// Same for the SciAgentState precompile (`isTripped` view).
    #[test]
    fn test_sci_agent_state_call_resolves() {
        use alloy_sol_types::SolCall;
        let mut ctx = create_test_context();
        let mut precompiles =
            BaseZkvmPrecompiles::new_with_spec(BaseSpecId::new(BaseUpgrade::Bedrock));

        let data = isTrippedCall { sessionKey: Address::from([0x22; 20]) }.abi_encode();
        let inputs = direct_call_inputs(
            sci_precompiles::SCI_AGENT_STATE_ADDRESS,
            Bytes::from(data),
            1_000_000,
        );

        let result = precompiles
            .run(&mut ctx, &inputs)
            .expect("no fatal error")
            .expect("agent-state address must resolve to a precompile");
        assert_eq!(result.result, InstructionResult::Return);
        assert_eq!(result.output.len(), 32, "isTripped returns one ABI word");
    }

    /// Parity details with the EL's `PrecompilesMap`: `contains` reports the SCI
    /// addresses, but `warm_addresses` does NOT include them (lookup-resolved
    /// precompiles are documented as always-cold on the EL side — warming them only
    /// in the zkVM would shift gas and fork the state root).
    #[test]
    fn test_sci_addresses_contained_but_cold() {
        let precompiles = BaseZkvmPrecompiles::new_with_spec(BaseSpecId::new(BaseUpgrade::Bedrock));

        let kc = sci_precompiles::ACCOUNT_KEYCHAIN_ADDRESS;
        let st = sci_precompiles::SCI_AGENT_STATE_ADDRESS;
        assert!(PrecompileProvider::<TestContext>::contains(&precompiles, &kc));
        assert!(PrecompileProvider::<TestContext>::contains(&precompiles, &st));

        let warm: Vec<Address> =
            PrecompileProvider::<TestContext>::warm_addresses(&precompiles).collect();
        assert!(!warm.contains(&kc), "lookup-resolved SCI precompiles must stay cold");
        assert!(!warm.contains(&st), "lookup-resolved SCI precompiles must stay cold");
    }

    /// A DELEGATECALL to the keychain reverts with `DelegateCallNotAllowed` — same as
    /// on the EL (the `sci_precompile!` boundary rejects non-direct calls).
    #[test]
    fn test_sci_keychain_delegatecall_reverts() {
        use alloy_sol_types::SolCall;
        let mut ctx = create_test_context();
        let mut precompiles =
            BaseZkvmPrecompiles::new_with_spec(BaseSpecId::new(BaseUpgrade::Bedrock));

        let data = getKeyCall { account: Address::ZERO, keyId: Address::ZERO }.abi_encode();
        // create_call_inputs sets target_address = ZERO != bytecode_address (DELEGATECALL).
        let inputs = create_call_inputs(
            sci_precompiles::ACCOUNT_KEYCHAIN_ADDRESS,
            Bytes::from(data),
            1_000_000,
        );

        let result = precompiles
            .run(&mut ctx, &inputs)
            .expect("no fatal error")
            .expect("keychain address must resolve to a precompile");
        assert_eq!(
            result.result,
            InstructionResult::Revert,
            "delegatecall to an SCI precompile must revert, not execute"
        );
    }
}
