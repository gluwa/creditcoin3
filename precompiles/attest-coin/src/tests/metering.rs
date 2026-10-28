use super::*;
use fp_evm::{ExitError, Precompile as _, PrecompileHandle, Transfer};
use sp_core::H256;

/// MockHandle models EVM gas/subcalls but ignores external costs. Enforce independent proof and
/// storage limits here so a fixed gas charge cannot accidentally stand in for either resource.
struct MeteredHandle {
    inner: MockHandle,
    proof_limit: u64,
    proof_used: u64,
    growth_limit: u64,
    growth_used: u64,
}

impl MeteredHandle {
    fn new(inner: MockHandle) -> Self {
        Self {
            inner,
            proof_limit: u64::MAX,
            proof_used: 0,
            growth_limit: u64::MAX,
            growth_used: 0,
        }
    }

    fn execute(&mut self) -> fp_evm::PrecompileResult {
        crate::AttestCoinPrecompile::<Runtime>::execute(self)
    }
}

impl PrecompileHandle for MeteredHandle {
    fn call(
        &mut self,
        address: H160,
        transfer: Option<Transfer>,
        input: Vec<u8>,
        target_gas: Option<u64>,
        is_static: bool,
        context: &Context,
    ) -> (ExitReason, Vec<u8>) {
        self.inner
            .call(address, transfer, input, target_gas, is_static, context)
    }
    fn record_cost(&mut self, cost: u64) -> Result<(), ExitError> {
        self.inner.record_cost(cost)
    }
    fn remaining_gas(&self) -> u64 {
        self.inner.remaining_gas()
    }
    fn log(&mut self, address: H160, topics: Vec<H256>, data: Vec<u8>) -> Result<(), ExitError> {
        self.inner.log(address, topics, data)
    }
    fn code_address(&self) -> H160 {
        self.inner.code_address()
    }
    fn input(&self) -> &[u8] {
        self.inner.input()
    }
    fn context(&self) -> &Context {
        self.inner.context()
    }
    fn is_static(&self) -> bool {
        self.inner.is_static()
    }
    fn gas_limit(&self) -> Option<u64> {
        self.inner.gas_limit()
    }
    fn record_external_cost(
        &mut self,
        _ref_time: Option<u64>,
        proof: Option<u64>,
        growth: Option<u64>,
    ) -> Result<(), ExitError> {
        let proof = proof.unwrap_or(0);
        let growth = growth.unwrap_or(0);
        if proof > self.proof_limit - self.proof_used
            || growth > self.growth_limit - self.growth_used
        {
            return Err(ExitError::OutOfGas);
        }
        self.proof_used += proof;
        self.growth_used += growth;
        Ok(())
    }
    fn refund_external_cost(&mut self, _ref_time: Option<u64>, proof: Option<u64>) {
        self.proof_used = self.proof_used.saturating_sub(proof.unwrap_or(0));
    }
    fn origin(&self) -> H160 {
        self.inner.origin()
    }
    fn is_contract_being_constructed(&self, address: H160) -> bool {
        self.inner.is_contract_being_constructed(address)
    }
}

#[test]
fn claim_cannot_create_nonce_without_storage_budget_but_can_update_existing_entries() {
    ExtBuilder::default().build().execute_with(|| {
        pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(ERC20_ADDRESS);
        pallet_attest_coin_rewards::RewardVault::<Runtime>::put(VAULT_ADDRESS);
        let (input, stash, recipient) = signed_claim(50, 1_000);
        let mut limited = MeteredHandle::new(make_handle(recipient, input.clone()));
        limited.growth_limit = 0;
        // No subcall mock: reaching the token before reserving storage would panic.
        assert!(limited.execute().is_err());
        assert_eq!(Accrued::<Runtime>::get(&stash), 1_000);
        assert!(!pallet_attest_coin_rewards::ClaimNonce::<Runtime>::contains_key(&stash));

        pallet_attest_coin_rewards::ClaimNonce::<Runtime>::insert(&stash, 0);
        let mut existing = MeteredHandle::new(make_handle(recipient, input));
        existing.growth_limit = 0;
        attach_mock_vault(&mut existing.inner, true, u128::MAX, u128::MAX);
        assert!(existing.execute().is_ok());
        assert_eq!(Accrued::<Runtime>::get(&stash), 950);
        assert_eq!(
            pallet_attest_coin_rewards::ClaimNonce::<Runtime>::get(stash),
            1
        );
        assert_eq!(existing.growth_used, 0);
    });
}

#[test]
fn deposit_requires_space_for_a_new_account_but_not_for_a_top_up() {
    let caller = H160::repeat_byte(0xCC);
    let beneficiary = <Runtime as pallet_evm::Config>::AddressMapping::into_account_id(caller);
    ExtBuilder::default()
        .with_balances(vec![(beneficiary.clone(), 1_000)])
        .build()
        .execute_with(|| {
            pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(ERC20_ADDRESS);
            let mut limited = MeteredHandle::new(make_handle(caller, deposit_input(100)));
            limited.growth_limit = 0;
            attach_mock_deposit_sequence(&mut limited.inner, 100);
            assert!(limited.execute().is_err());
            assert!(!pallet_assets::Account::<Runtime>::contains_key(
                1,
                &beneficiary
            ));

            let mut first = MeteredHandle::new(make_handle(caller, deposit_input(100)));
            attach_mock_deposit_sequence(&mut first.inner, 100);
            assert!(first.execute().is_ok());
            let key = pallet_assets::Account::<Runtime>::hashed_key_for(1, &beneficiary);
            let value = frame_support::storage::unhashed::get_raw(&key).unwrap();
            assert!(first.growth_used >= (key.len() + value.len()) as u64);

            let mut top_up = MeteredHandle::new(make_handle(caller, deposit_input(100)));
            top_up.growth_limit = 0;
            attach_mock_deposit_sequence(&mut top_up.inner, 100);
            assert!(top_up.execute().is_ok());
            assert_eq!(
                pallet_assets::Pallet::<Runtime>::balance(1, beneficiary),
                200
            );
            assert_eq!(top_up.growth_used, 0);
        });
}
