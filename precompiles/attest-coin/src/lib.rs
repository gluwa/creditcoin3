//! Attest-coin precompile: `accrued(bytes32)`, `claim(...)`, `deposit(uint256)`,
//! `depositTo(uint256,bytes32)`, and `withdraw(uint256)` (`pallet-assets` burn → ERC-20 to caller;
//! inverse of deposit). Requires asset **admin** = precompile account (see runtime migration).
//!
//! Governance must configure a standard ERC-20 (no fee-on-transfer / rebasing). Claims require a
//! stash sr25519 signature; staking controllers cannot authorize claims.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

use core::marker::PhantomData;
use fp_evm::{
    Context, ExitError, ExitReason, ExitRevert, ExitSucceed, Precompile, PrecompileFailure,
    PrecompileHandle, PrecompileOutput, PrecompileResult,
};
use frame_support::dispatch::{GetDispatchInfo, PostDispatchInfo};
use frame_support::traits::Get;
use pallet_attest_coin_rewards::Pallet as Rewards;
use pallet_evm::{AddressMapping, GasWeightMapping};
use parity_scale_codec::Encode;
use precompile_utils::evm::handle::using_precompile_handle;
use precompile_utils::prelude::RuntimeHelper;
use precompile_utils::substrate::TryDispatchError;
use sp_core::{sr25519, H160, U256};
use sp_io::crypto::sr25519_verify;
use sp_runtime::traits::{Dispatchable, Saturating, StaticLookup, UniqueSaturatedInto};
use sp_std::vec::Vec;

/// `accrued(bytes32)`
const SEL_ACCRUED: [u8; 4] = [0xf9, 0x2f, 0x23, 0xa7];
/// `claim(bytes32,uint256,uint256,uint256,address,bytes32,bytes32)` — see [`claim_selector`].
const SEL_CLAIM: [u8; 4] = [0x1f, 0xfb, 0x7a, 0x3d];
/// ERC-20 `transfer(address,uint256)`
const SEL_TRANSFER: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
/// ERC-20 `transferFrom(address,address,uint256)`
const SEL_TRANSFER_FROM: [u8; 4] = [0x23, 0xb8, 0x72, 0xdd];
/// ERC-20 `balanceOf(address)`
const SEL_BALANCE_OF: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];
/// `deposit(uint256)` — bridge ERC-20 into Substrate `pallet-assets` (mint to EVM caller’s mapped account).
const SEL_DEPOSIT: [u8; 4] = [0xb6, 0xb5, 0x5f, 0x25];
/// `depositTo(uint256,bytes32)` — same as [`SEL_DEPOSIT`] but mints to an explicit 32-byte `AccountId`.
const SEL_DEPOSIT_TO: [u8; 4] = [0xc6, 0xbc, 0x97, 0x5d];
/// `withdraw(uint256)` — burn Substrate attest coin from caller’s mapped account, send ERC-20 to caller.
pub const SEL_WITHDRAW: [u8; 4] = [0x2e, 0x1a, 0x7d, 0x4d];
// Asset ID for attest coin in `pallet-assets` was previously a magic constant here. It now
// lives on `pallet_attest_coin_rewards::Config::AttestCoinAssetId`; this precompile reads it
// from the runtime so a config change can never silently re-target a different asset. See
// `attest_coin_asset_id::<Runtime>()` below.

/// Minimum gas reserved for the ERC-20 `transfer` subcall in [`AttestCoinPrecompile::withdraw`].
const WITHDRAW_ERC20_TRANSFER_MIN_GAS: u64 = 80_000;
/// Slack on top of weight-derived burn gas so metering error does not OOG the follow-up transfer.
const WITHDRAW_PRE_BURN_GAS_SLACK: u64 = 200_000;

pub struct AttestCoinPrecompile<Runtime>(PhantomData<Runtime>);

impl<Runtime> AttestCoinPrecompile<Runtime> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<Runtime> Default for AttestCoinPrecompile<Runtime> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Runtime> Precompile for AttestCoinPrecompile<Runtime>
where
    Runtime: pallet_evm::Config
        + pallet_assets::Config
        + pallet_attest_coin_rewards::Config
        + pallet_attestation::Config
        + pallet_supported_chains::Config,
    Runtime::AccountId: From<[u8; 32]> + Encode,
    Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    Runtime::RuntimeCall: From<pallet_assets::Call<Runtime>>,
    <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
    <Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
    <Runtime as pallet_assets::Config>::AssetId: From<u32>,
    <Runtime as pallet_assets::Config>::AssetIdParameter: From<u32>,
    <Runtime as pallet_assets::Config>::Balance: From<u128>,
{
    fn execute(handle: &mut impl PrecompileHandle) -> PrecompileResult {
        let input = handle.input().to_vec();
        if input.len() < 4 {
            return Err(PrecompileFailure::Error {
                exit_status: ExitError::OutOfGas,
            });
        }
        let mut sel = [0u8; 4];
        sel.copy_from_slice(&input[..4]);
        match sel {
            SEL_ACCRUED => Self::accrued(handle, &input[4..]),
            SEL_CLAIM => Self::claim(handle, &input[4..]),
            SEL_DEPOSIT => Self::deposit(handle, &input[4..]),
            SEL_DEPOSIT_TO => Self::deposit_to(handle, &input[4..]),
            SEL_WITHDRAW => Self::withdraw(handle, &input[4..]),
            _ => Err(PrecompileFailure::Error {
                exit_status: ExitError::Other("unknown selector".into()),
            }),
        }
    }
}

impl<Runtime> AttestCoinPrecompile<Runtime>
where
    Runtime: pallet_evm::Config
        + pallet_assets::Config
        + pallet_attest_coin_rewards::Config
        + pallet_attestation::Config
        + pallet_supported_chains::Config,
    Runtime::AccountId: From<[u8; 32]> + Encode,
    Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    Runtime::RuntimeCall: From<pallet_assets::Call<Runtime>>,
    <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
    <Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
    <Runtime as pallet_assets::Config>::AssetId: From<u32>,
    <Runtime as pallet_assets::Config>::AssetIdParameter: From<u32>,
    <Runtime as pallet_assets::Config>::Balance: From<u128>,
{
    fn accrued(handle: &mut impl PrecompileHandle, rest: &[u8]) -> PrecompileResult {
        // One Substrate storage read + u128→U256 marshalling. Pricing aligned to the
        // `chain-info` precompile's `GAS_STORAGE_LOOKUP = 2_600` baseline plus headroom for
        // the conversion, so callers can't probe storage at sub-SLOAD prices.
        handle.record_cost(3_500)?;
        if rest.len() < 32 {
            return Err(bad_input());
        }
        let mut raw = [0u8; 32];
        raw.copy_from_slice(&rest[..32]);
        let stash = Runtime::AccountId::from(raw);
        let a = Rewards::<Runtime>::accrued_of(&stash);
        let v: u128 = a.into();
        let u = U256::from(v);
        Ok(PrecompileOutput {
            exit_status: ExitSucceed::Returned,
            output: encode_u256(u),
        })
    }

    /// Transfer accrued attest-coin rewards to the EVM caller.
    ///
    /// The precompile must already hold an ERC-20 balance of the token configured via
    /// [`pallet_attest_coin_rewards::AttestCoinErc20`] (funded by the protocol treasury).
    /// The claim executes `ERC-20.transfer(evm_recipient, amount)` with
    /// `sub_context.caller = code_address` so the transfer is sent from the precompile's own balance.
    ///
    /// Claims may only spend ERC-20 **above** the amount needed to back withdrawable
    /// [`pallet_assets`] attest-coin (total supply minus bond-pool balance). Bonded
    /// attest coin in [`pallet_attestation::Config::BondPoolAccount`] is not redeemable via
    /// `withdraw`, so it does not require ERC-20 headroom during reward claims.
    fn claim(handle: &mut impl PrecompileHandle, rest: &[u8]) -> PrecompileResult {
        // claim(bytes32,uint256,uint256,uint256,address,bytes32,bytes32) — 7 × 32 bytes after selector
        handle.record_cost(120_000)?;
        if rest.len() < 224 {
            return Err(bad_input());
        }

        let mut stash_raw = [0u8; 32];
        stash_raw.copy_from_slice(&rest[0..32]);
        let stash = Runtime::AccountId::from(stash_raw);

        let nonce_u256 = U256::from_big_endian(&rest[32..64]);
        let chain_u256 = U256::from_big_endian(&rest[64..96]);
        let amount_u256 = U256::from_big_endian(&rest[96..128]);

        // `address`: last 20 bytes of the fifth 32-byte word (ABI head).
        let mut evm_recipient = H160::zero();
        evm_recipient.0.copy_from_slice(&rest[140..160]);

        let mut sig = [0u8; 64];
        sig[..].copy_from_slice(&rest[160..224]);

        let caller_h160 = handle.context().caller;
        if caller_h160 != evm_recipient {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"evm recipient must be caller".to_vec(),
            });
        }

        // nonce_u64 is validated against the on-chain counter inside `commit_claim` below.
        let nonce_u64 = u256_to_u64(nonce_u256)?;
        let chain_key = u256_to_u64(chain_u256)?;

        if !pallet_supported_chains::SupportedChains::<Runtime>::contains_key(chain_key) {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"unsupported chain key".to_vec(),
            });
        }
        let amount_pts = u256_to_reward_points::<Runtime>(amount_u256)?;
        let amount_u128: u128 = amount_pts.into();

        let token = match Rewards::<Runtime>::erc20_token() {
            Some(t) => t,
            None => {
                return Err(PrecompileFailure::Revert {
                    exit_status: ExitRevert::Reverted,
                    output: b"token not configured".to_vec(),
                });
            }
        };

        let msg = Rewards::<Runtime>::claim_signing_message(
            &stash,
            nonce_u64,
            chain_key,
            amount_u128,
            evm_recipient.0,
        );

        if !verify_sr25519_stash(&stash, &msg, &sig) {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"bad signature".to_vec(),
            });
        }

        let treasury_balance = erc20_balance_of(handle, token, handle.code_address())?;
        ensure_treasury_covers_claim_and_deposit_backing::<Runtime>(
            treasury_balance,
            amount_u256,
        )?;

        Rewards::<Runtime>::commit_claim(&stash, nonce_u64, amount_pts).map_err(|e| {
            use pallet_attest_coin_rewards::Error as RewardErr;
            let msg: &[u8] = match e {
                RewardErr::BadClaimNonce => b"bad nonce",
                RewardErr::InsufficientAccrued => b"insufficient accrued",
                _ => b"commit claim failed",
            };
            PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: msg.to_vec(),
            }
        })?;

        if let Err(failure) = erc20_transfer(handle, token, caller_h160, amount_u256) {
            Rewards::<Runtime>::undo_claim_commit(&stash, nonce_u64, amount_pts);
            return Err(failure);
        }

        Ok(PrecompileOutput {
            exit_status: ExitSucceed::Returned,
            output: Vec::new(),
        })
    }

    /// Mint attest coin to the Substrate account mapped from the EVM caller (`AddressMapping`).
    ///
    /// **Before calling this function**, the user must call `approve(precompile_address, amount)`
    /// on the ERC-20 contract, where `precompile_address` is this attest-coin precompile's deployed
    /// address. The precompile uses `transferFrom(caller, precompile, amount)` internally, with
    /// `sub_context.caller = code_address` so the precompile is the approved spender.
    fn deposit(handle: &mut impl PrecompileHandle, rest: &[u8]) -> PrecompileResult {
        let caller_h160 = handle.context().caller;
        let beneficiary = Runtime::AddressMapping::into_account_id(caller_h160);
        Self::deposit_with_beneficiary(handle, rest, beneficiary)
    }

    /// Mint attest coin to an explicit 32-byte `AccountId` (e.g. sr25519 stash), still pulling ERC-20 from the caller.
    ///
    /// **Before calling this function**, the user must call `approve(precompile_address, amount)`
    /// on the ERC-20 contract, where `precompile_address` is this attest-coin precompile's deployed
    /// address. The precompile uses `transferFrom(caller, precompile, amount)` with
    /// `sub_context.caller = code_address` (precompile as the approved spender).
    fn deposit_to(handle: &mut impl PrecompileHandle, rest: &[u8]) -> PrecompileResult {
        if rest.len() < 64 {
            return Err(bad_input());
        }
        let mut raw = [0u8; 32];
        raw.copy_from_slice(&rest[32..64]);
        if raw == [0u8; 32] {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"zero beneficiary".to_vec(),
            });
        }
        let beneficiary = Runtime::AccountId::from(raw);
        // NOTE: no attestor-registration gate here, by design. The intended onboarding flow is
        // deposit-then-register: a fresh stash receives attest-coin via `depositTo` *first* and
        // only then calls `register_attestor` (registration is what creates the
        // `pallet_attestation::Ledger` entry, so a Ledger gate here would make onboarding
        // impossible — audit H-3). Minting to an arbitrary 32-byte beneficiary is bounded by
        // the caller's own ERC-20 spend; a typo'd beneficiary loses only the caller's funds.
        Self::deposit_with_beneficiary(handle, &rest[0..32], beneficiary)
    }

    /// Internal implementation for [`Self::deposit`] and [`Self::deposit_to`].
    ///
    /// Executes `ERC-20.transferFrom(caller, precompile, amount)` via a sub-call where
    /// `sub_context.caller = code_address` — making the precompile the approved spender in the
    /// ERC-20 `transferFrom` call. The user must have called `approve(precompile_address, amount)`
    /// on the ERC-20 before invoking `deposit`/`depositTo`.
    fn deposit_with_beneficiary(
        handle: &mut impl PrecompileHandle,
        amount_word: &[u8],
        beneficiary: Runtime::AccountId,
    ) -> PrecompileResult {
        handle.record_cost(120_000)?;
        if amount_word.len() < 32 {
            return Err(bad_input());
        }
        let amount_u256 = U256::from_big_endian(&amount_word[0..32]);
        if amount_u256.is_zero() {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"zero amount".to_vec(),
            });
        }
        let amount_u128 = u256_to_u128_balance(amount_u256)?;

        let token = match Rewards::<Runtime>::erc20_token() {
            Some(t) => t,
            None => {
                return Err(PrecompileFailure::Revert {
                    exit_status: ExitRevert::Reverted,
                    output: b"token not configured".to_vec(),
                });
            }
        };

        let caller_h160 = handle.context().caller;
        let precompile_h160 = handle.code_address();

        // Defence-in-depth against a non-standard ERC-20 being configured by governance
        // (the module doc warns against it, but doesn't enforce). Fee-on-transfer or
        // rebasing tokens would leave the precompile holding fewer ERC-20 than the amount
        // about to be minted as attest-coin, silently breaking 1:1 backing for everyone
        // forever. Bracket `transferFrom` with a balance probe and revert if delta < amount.
        let balance_before = erc20_balance_of(handle, token, precompile_h160)?;
        erc20_transfer_from(handle, token, caller_h160, precompile_h160, amount_u256)?;
        let balance_after = erc20_balance_of(handle, token, precompile_h160)?;
        if balance_after.saturating_sub(balance_before) < amount_u256 {
            // We've already taken less-than-expected ERC-20 from the caller. Refund whatever
            // *did* land in the precompile to preserve value conservation. If the refund
            // itself fails, surface that explicitly (same policy as the mint-failure path).
            let received = balance_after.saturating_sub(balance_before);
            if !received.is_zero() {
                erc20_transfer(handle, token, caller_h160, received)?;
            }
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"non-standard token (fee-on-transfer or rebasing)".to_vec(),
            });
        }

        let issuer = Runtime::AddressMapping::into_account_id(precompile_h160);

        if let Err(failure) = try_dispatch_attest_coin_no_pov::<Runtime, _>(
            handle,
            Some(issuer).into(),
            pallet_assets::Call::<Runtime>::mint {
                id: attest_coin_asset_id::<Runtime>().into(),
                beneficiary: Runtime::Lookup::unlookup(beneficiary),
                amount: amount_u128.into(),
            },
        )
        .map_err(PrecompileFailure::from)
        {
            // Mint failed *after* a successful `transferFrom` already moved ERC-20 into the
            // precompile. We must compensate by transferring it back; if that compensating
            // transfer ALSO fails, we have a value-conservation violation (ERC-20 sitting in
            // the precompile with no attest-coin mint to match) and must surface that failure
            // explicitly rather than silently swallow it and revert with the mint error —
            // which would hide the stuck-funds condition from the caller and any monitor.
            //
            // Returning Err in *either* arm reverts the precompile's EVM call frame, but a
            // failed refund still warrants the explicit signal: the failure mode is louder
            // and observable in any event logs / tracing.
            erc20_transfer(handle, token, caller_h160, amount_u256)?;
            return Err(failure);
        }

        Ok(PrecompileOutput {
            exit_status: ExitSucceed::Returned,
            output: Vec::new(),
        })
    }

    /// Send ERC-20 to the caller, then burn Substrate attest-coin (inverse of [`Self::deposit`]).
    ///
    /// ERC-20 is transferred before any Substrate burn so a failed token transfer does not debit
    /// pallet-assets. Uses [`pallet_assets::Call::burn`] as **admin** (precompile account).
    ///
    /// # Value-conservation invariant
    ///
    /// If the burn dispatch fails *after* the ERC-20 transfer succeeded, returning `Err` reverts
    /// the precompile's EVM call frame — which under Frontier `PrecompileHandle::call` semantics
    /// rolls back the prior `handle.call(...)` sub-call to the ERC-20 contract, undoing the
    /// transfer. **This invariant cannot be exercised in this crate's mock tests** because the
    /// `MockHandle` does not simulate EVM rollback of sub-call state; the symmetric
    /// transfer-fails-before-burn case is covered by
    /// `withdraw_restores_pallet_balance_when_erc20_transfer_fails`, and an end-to-end
    /// burn-failure-after-transfer test belongs in the runtime integration suite (`cli/`) where
    /// the full EVM executor is in scope. The pre-burn substrate balance re-check at line 440
    /// closes the only realistic window where burn could fail post-transfer without an
    /// integration-test harness.
    fn withdraw(handle: &mut impl PrecompileHandle, rest: &[u8]) -> PrecompileResult {
        handle.record_cost(120_000)?;
        if rest.len() < 32 {
            return Err(bad_input());
        }
        let amount_u256 = U256::from_big_endian(&rest[0..32]);
        if amount_u256.is_zero() {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"zero amount".to_vec(),
            });
        }
        let amount_u128 = u256_to_u128_balance(amount_u256)?;

        let token = match Rewards::<Runtime>::erc20_token() {
            Some(t) => t,
            None => {
                return Err(PrecompileFailure::Revert {
                    exit_status: ExitRevert::Reverted,
                    output: b"token not configured".to_vec(),
                });
            }
        };

        let caller_h160 = handle.context().caller;
        let beneficiary = Runtime::AddressMapping::into_account_id(caller_h160);
        let precompile_h160 = handle.code_address();
        let admin = Runtime::AddressMapping::into_account_id(precompile_h160);

        let amount_asset: <Runtime as pallet_assets::Config>::Balance = amount_u128.into();
        let asset_id: <Runtime as pallet_assets::Config>::AssetId =
            attest_coin_asset_id::<Runtime>().into();
        let substrate_balance =
            pallet_assets::Pallet::<Runtime>::balance(asset_id.clone(), &beneficiary);
        if substrate_balance < amount_asset {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"insufficient attest-coin balance".to_vec(),
            });
        }

        let treasury_balance = erc20_balance_of(handle, token, handle.code_address())?;
        if treasury_balance < amount_u256 {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"insufficient treasury balance".to_vec(),
            });
        }

        let burn_call = pallet_assets::Call::<Runtime>::burn {
            id: attest_coin_asset_id::<Runtime>().into(),
            who: Runtime::Lookup::unlookup(beneficiary.clone()),
            amount: amount_asset,
        };
        let gas_burn =
            evm_gas_for_dispatch_call::<Runtime>(&Runtime::RuntimeCall::from(burn_call.clone()));
        let min_before_transfer = gas_burn
            .saturating_add(WITHDRAW_ERC20_TRANSFER_MIN_GAS)
            .saturating_add(WITHDRAW_PRE_BURN_GAS_SLACK);
        if handle.remaining_gas() < min_before_transfer {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"withdraw: insufficient gas".to_vec(),
            });
        }

        erc20_transfer(handle, token, caller_h160, amount_u256)?;

        let balance_after_transfer =
            pallet_assets::Pallet::<Runtime>::balance(asset_id.clone(), &beneficiary);
        if balance_after_transfer < amount_asset {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"insufficient attest-coin balance".to_vec(),
            });
        }

        try_dispatch_attest_coin_no_pov::<Runtime, _>(handle, Some(admin).into(), burn_call)
            .map_err(PrecompileFailure::from)?;

        Ok(PrecompileOutput {
            exit_status: ExitSucceed::Returned,
            output: Vec::new(),
        })
    }
}

/// Read the runtime-configured `pallet_assets` ID for attest coin (was a precompile-side
/// magic constant in earlier revisions).
fn attest_coin_asset_id<Runtime>() -> u32
where
    Runtime: pallet_attest_coin_rewards::Config,
{
    <Runtime as pallet_attest_coin_rewards::Config>::AttestCoinAssetId::get()
}

/// ERC-20 backing required for all non-pool [`pallet_assets`] attest-coin balances.
fn attest_coin_withdrawable_backing_u256<Runtime>() -> U256
where
    Runtime: pallet_assets::Config + pallet_attest_coin_rewards::Config + pallet_attestation::Config,
    <Runtime as pallet_assets::Config>::AssetId: From<u32>,
{
    let asset_id: <Runtime as pallet_assets::Config>::AssetId =
        attest_coin_asset_id::<Runtime>().into();
    let supply = pallet_assets::Pallet::<Runtime>::total_supply(asset_id.clone());
    let pool = <Runtime as pallet_attestation::Config>::BondPoolAccount::get();
    let pool_bal = pallet_assets::Pallet::<Runtime>::balance(asset_id, &pool);
    let withdrawable = supply.saturating_sub(pool_bal);
    let v: u128 = UniqueSaturatedInto::unique_saturated_into(withdrawable);
    U256::from(v)
}

/// Claims and withdraws share one ERC-20 treasury. Withdraw burns matching pallet-assets, so
/// `treasury >= amount` is enough there. Claims do not burn pallet-assets, so they must leave
/// at least [`attest_coin_withdrawable_backing_u256`] in the treasury after payout.
fn ensure_treasury_covers_claim_and_deposit_backing<Runtime>(
    treasury_balance: U256,
    claim_amount: U256,
) -> Result<(), PrecompileFailure>
where
    Runtime: pallet_assets::Config
        + pallet_attest_coin_rewards::Config
        + pallet_attestation::Config,
    <Runtime as pallet_assets::Config>::AssetId: From<u32>,
{
    let deposit_backing = attest_coin_withdrawable_backing_u256::<Runtime>();
    let required = claim_amount.checked_add(deposit_backing).ok_or_else(|| {
        PrecompileFailure::Revert {
            exit_status: ExitRevert::Reverted,
            output: b"amount too large".to_vec(),
        }
    })?;
    if treasury_balance < required {
        return Err(PrecompileFailure::Revert {
            exit_status: ExitRevert::Reverted,
            output: b"claim would impair deposit backing".to_vec(),
        });
    }
    Ok(())
}

/// EVM gas charged by [`try_dispatch_attest_coin_no_pov`] for a runtime call (matches its pre-dispatch check).
fn evm_gas_for_dispatch_call<Runtime>(call: &Runtime::RuntimeCall) -> u64
where
    Runtime: pallet_evm::Config,
    Runtime::RuntimeCall: GetDispatchInfo,
{
    <Runtime as pallet_evm::Config>::GasWeightMapping::weight_to_gas(
        call.get_dispatch_info().weight,
    )
}

/// Like [`RuntimeHelper::try_dispatch`], but does **not** reserve `dispatch_info.weight.proof_size()` on
/// the EVM handle. Otherwise Substrate benchmark PoV for `pallet_assets` dispatch is stacked on top of the
/// same transaction’s Frontier proof meter and exhausts gas (`gasUsed == gasLimit`).
fn try_dispatch_attest_coin_no_pov<Runtime, Call>(
    handle: &mut impl PrecompileHandle,
    origin: <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin,
    call: Call,
) -> Result<PostDispatchInfo, TryDispatchError>
where
    Runtime: pallet_evm::Config,
    Runtime::RuntimeCall: From<Call> + Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
{
    let call = Runtime::RuntimeCall::from(call);
    let dispatch_info = call.get_dispatch_info();

    let remaining_gas = handle.remaining_gas();
    let required_gas =
        <Runtime as pallet_evm::Config>::GasWeightMapping::weight_to_gas(dispatch_info.weight);
    if required_gas > remaining_gas {
        return Err(TryDispatchError::Evm(ExitError::OutOfGas));
    }

    handle
        .record_external_cost(None, None, Some(0u64))
        .map_err(TryDispatchError::Evm)?;

    let post_dispatch_info = using_precompile_handle(handle, || call.dispatch(origin))
        .map_err(|e| TryDispatchError::Substrate(e.error))?;

    RuntimeHelper::<Runtime>::refund_weight_v2_cost(
        handle,
        dispatch_info.weight,
        post_dispatch_info.actual_weight,
    )
    .map_err(TryDispatchError::Evm)?;

    Ok(post_dispatch_info)
}

fn erc20_subcall(
    handle: &mut impl PrecompileHandle,
    token: H160,
    data: Vec<u8>,
) -> Result<Vec<u8>, PrecompileFailure> {
    let sub_context = Context {
        caller: handle.code_address(),
        address: token,
        apparent_value: U256::zero(),
    };
    let gas = handle.remaining_gas().saturating_mul(9) / 10;
    let (reason, ret) = handle.call(token, None, data, Some(gas), false, &sub_context);
    if matches!(reason, ExitReason::Succeed(_)) {
        Ok(ret)
    } else {
        Err(PrecompileFailure::Revert {
            exit_status: ExitRevert::Reverted,
            output: ret,
        })
    }
}

/// SafeERC20-style result decoding (audit M-4): an ERC-20 `transfer`/`transferFrom` succeeds
/// iff the sub-call did not revert AND the returndata is either empty (non-standard tokens
/// like USDT return nothing) or ABI-decodes to `true`. A token returning `false` without
/// reverting would otherwise be treated as success — letting `claim` debit accrued points or
/// `withdraw` burn attest-coin without delivering any tokens.
fn require_erc20_success(ret: &[u8]) -> Result<(), PrecompileFailure> {
    if ret.is_empty() {
        return Ok(());
    }
    if ret.len() >= 32 && U256::from_big_endian(&ret[..32]) == U256::one() {
        return Ok(());
    }
    Err(PrecompileFailure::Revert {
        exit_status: ExitRevert::Reverted,
        output: b"erc20 transfer returned false".to_vec(),
    })
}

fn erc20_transfer(
    handle: &mut impl PrecompileHandle,
    token: H160,
    to: H160,
    amount: U256,
) -> Result<(), PrecompileFailure> {
    let mut data = Vec::with_capacity(4 + 32 + 32);
    data.extend_from_slice(&SEL_TRANSFER);
    data.extend_from_slice(&encode_address(to.as_fixed_bytes()));
    data.extend_from_slice(&encode_u256(amount));
    let ret = erc20_subcall(handle, token, data)?;
    require_erc20_success(&ret)
}

fn erc20_transfer_from(
    handle: &mut impl PrecompileHandle,
    token: H160,
    from: H160,
    to: H160,
    amount: U256,
) -> Result<(), PrecompileFailure> {
    let mut data = Vec::with_capacity(4 + 32 + 32 + 32);
    data.extend_from_slice(&SEL_TRANSFER_FROM);
    data.extend_from_slice(&encode_address(from.as_fixed_bytes()));
    data.extend_from_slice(&encode_address(to.as_fixed_bytes()));
    data.extend_from_slice(&encode_u256(amount));
    let ret = erc20_subcall(handle, token, data)?;
    require_erc20_success(&ret)
}

fn erc20_balance_of(
    handle: &mut impl PrecompileHandle,
    token: H160,
    account: H160,
) -> Result<U256, PrecompileFailure> {
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&SEL_BALANCE_OF);
    data.extend_from_slice(&encode_address(account.as_fixed_bytes()));
    let ret = erc20_subcall(handle, token, data)?;
    if ret.len() < 32 {
        return Err(PrecompileFailure::Revert {
            exit_status: ExitRevert::Reverted,
            output: b"balanceOf: bad return".to_vec(),
        });
    }
    Ok(U256::from_big_endian(&ret[ret.len() - 32..]))
}

fn account_id_to_sr25519_public<AccountId: Encode>(acct: &AccountId) -> Option<sr25519::Public> {
    let enc = acct.encode();
    // Substrate's `AccountId32` is always 32 bytes. A non-32-byte `AccountId` is unsupported
    // by this precompile — returning `None` here lets the caller fail the signature check
    // explicitly rather than panic or silently produce a malformed key.
    if enc.len() != 32 {
        return None;
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(&enc[..32]);
    Some(sr25519::Public::from_raw(a))
}

/// Claim signatures must be from the stash sr25519 key (not a staking controller).
fn verify_sr25519_stash<AccountId: Encode>(stash: &AccountId, msg: &[u8], sig: &[u8; 64]) -> bool {
    let mut sig_raw = [0u8; 64];
    sig_raw.copy_from_slice(sig);
    let s = sr25519::Signature::from_raw(sig_raw);
    account_id_to_sr25519_public(stash)
        .map(|pk| sr25519_verify(&s, msg, &pk))
        .unwrap_or(false)
}

fn bad_input() -> PrecompileFailure {
    PrecompileFailure::Error {
        exit_status: ExitError::Other("invalid input".into()),
    }
}

#[cfg_attr(test, allow(dead_code))]
pub(crate) fn encode_u256(u: U256) -> Vec<u8> {
    let mut out = [0u8; 32];
    u.to_big_endian(&mut out);
    out.to_vec()
}

pub(crate) fn encode_address(addr: &[u8; 20]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[12..32].copy_from_slice(addr.as_slice());
    out
}

pub(crate) fn u256_to_u64(u: U256) -> Result<u64, PrecompileFailure> {
    let mut be = [0u8; 32];
    u.to_big_endian(&mut be);
    if be.iter().take(24).any(|b| *b != 0) {
        return Err(PrecompileFailure::Revert {
            exit_status: ExitRevert::Reverted,
            output: b"value too large".to_vec(),
        });
    }
    Ok(u64::from_be_bytes(be[24..32].try_into().expect("8 bytes")))
}

pub(crate) fn u256_to_u128_balance(u: U256) -> Result<u128, PrecompileFailure> {
    let mut be = [0u8; 32];
    u.to_big_endian(&mut be);
    if be.iter().take(16).any(|b| *b != 0) {
        return Err(PrecompileFailure::Revert {
            exit_status: ExitRevert::Reverted,
            output: b"amount too large".to_vec(),
        });
    }
    Ok(u128::from_be_bytes(
        be[16..32].try_into().expect("16 bytes"),
    ))
}

fn u256_to_reward_points<Runtime: pallet_attest_coin_rewards::Config>(
    u: U256,
) -> Result<Runtime::RewardPoints, PrecompileFailure> {
    let mut be = [0u8; 32];
    u.to_big_endian(&mut be);
    if be.iter().take(16).any(|b| *b != 0) {
        return Err(PrecompileFailure::Revert {
            exit_status: ExitRevert::Reverted,
            output: b"amount too large".to_vec(),
        });
    }
    let v = u128::from_be_bytes(be[16..32].try_into().expect("16 bytes"));
    Ok(v.into())
}
