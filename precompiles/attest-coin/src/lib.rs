//! Attest-coin precompile: `accrued(bytes32)`, `claim(...)`, `deposit(uint256)`,
//! `depositTo(uint256,bytes32)`, and `withdraw(uint256)` (`pallet-assets` burn → ERC-20 to caller;
//! inverse of deposit). Requires asset **admin** = precompile account (see runtime migration).

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
use pallet_attest_coin_rewards::Pallet as Rewards;
use pallet_evm::{AddressMapping, GasWeightMapping};
use pallet_staking::Bonded;
use parity_scale_codec::Encode;
use precompile_utils::evm::handle::using_precompile_handle;
use precompile_utils::prelude::RuntimeHelper;
use precompile_utils::substrate::TryDispatchError;
use sp_core::{sr25519, H160, U256};
use sp_io::crypto::sr25519_verify;
use sp_runtime::traits::{Dispatchable, StaticLookup};
use sp_std::vec::Vec;

/// `accrued(bytes32)`
const SEL_ACCRUED: [u8; 4] = [0xf9, 0x2f, 0x23, 0xa7];
/// `claim(bytes32,uint256,uint256,uint256,address,bytes32,bytes32)` — see [`claim_selector`].
const SEL_CLAIM: [u8; 4] = [0x1f, 0xfb, 0x7a, 0x3d];
/// ERC-20 `transfer(address,uint256)`
const SEL_TRANSFER: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
/// ERC-20 `transferFrom(address,address,uint256)`
const SEL_TRANSFER_FROM: [u8; 4] = [0x23, 0xb8, 0x72, 0xdd];
/// `deposit(uint256)` — bridge ERC-20 into Substrate `pallet-assets` (mint to EVM caller’s mapped account).
const SEL_DEPOSIT: [u8; 4] = [0xb6, 0xb5, 0x5f, 0x25];
/// `depositTo(uint256,bytes32)` — same as [`SEL_DEPOSIT`] but mints to an explicit 32-byte `AccountId`.
const SEL_DEPOSIT_TO: [u8; 4] = [0xc6, 0xbc, 0x97, 0x5d];
/// `withdraw(uint256)` — burn Substrate attest coin from caller’s mapped account, send ERC-20 to caller.
pub const SEL_WITHDRAW: [u8; 4] = [0x2e, 0x1a, 0x7d, 0x4d];
/// The asset ID for attest coin in `pallet-assets`.
///
/// **Must match the chain-spec asset ID** used at genesis (and in any runtime upgrade migration).
/// At genesis the asset is created via the chain-spec `pallet_assets` genesis config.
/// During runtime upgrades, if the asset does not yet exist, create it with:
/// `pallet_assets::Pallet::force_create(root, ATTEST_COIN_ASSET_ID.into(), issuer, false, 1)`
/// in a storage migration before any precompile call that mints or transfers the asset.
const ATTEST_COIN_ASSET_ID: u32 = 1;

/// Minimum gas passed to the ERC-20 `transfer` subcall in [`AttestCoinPrecompile::withdraw`].
/// The rest of post-burn gas is reserved for a pallet-assets `mint` rollback if the transfer fails.
const WITHDRAW_ERC20_TRANSFER_MIN_GAS: u64 = 80_000;
/// Buffer on top of weight-derived gas for the restore `mint` so metering / refunds do not OOG rollback.
const WITHDRAW_MINT_RESTORE_GAS_BUFFER: u64 = 120_000;
/// Slack so post-burn remaining gas is still enough for `WITHDRAW_ERC20_TRANSFER_MIN_GAS` after weight estimate error.
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
        + pallet_staking::Config
        + pallet_supported_chains::Config,
    Runtime::AccountId: From<[u8; 32]> + Encode,
    Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    Runtime::RuntimeCall: From<pallet_assets::Call<Runtime>>,
    <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
    <Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
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
        + pallet_staking::Config
        + pallet_supported_chains::Config,
    Runtime::AccountId: From<[u8; 32]> + Encode,
    Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    Runtime::RuntimeCall: From<pallet_assets::Call<Runtime>>,
    <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
    <Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
    <Runtime as pallet_assets::Config>::AssetIdParameter: From<u32>,
    <Runtime as pallet_assets::Config>::Balance: From<u128>,
{
    fn accrued(handle: &mut impl PrecompileHandle, rest: &[u8]) -> PrecompileResult {
        handle.record_cost(3_000)?;
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

        if !pallet_attestation::Ledger::<Runtime>::contains_key(&stash) {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"not a stash".to_vec(),
            });
        }

        let msg = Rewards::<Runtime>::claim_signing_message(
            &stash,
            nonce_u64,
            chain_key,
            amount_u128,
            evm_recipient.0,
        );

        if !verify_sr25519_stash_or_controller::<Runtime>(&stash, &msg, &sig) {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"bad signature".to_vec(),
            });
        }

        Rewards::<Runtime>::commit_claim(&stash, nonce_u64, amount_pts).map_err(|e| {
            use pallet_attest_coin_rewards::Error as RewardErr;
            let msg: &[u8] = match e {
                RewardErr::BadClaimNonce => b"bad nonce",
                RewardErr::InsufficientAccrued => b"insufficient accrued",
                RewardErr::NotStash => b"not a stash",
                _ => b"commit claim failed",
            };
            PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: msg.to_vec(),
            }
        })?;

        let mut transfer_data = Vec::with_capacity(4 + 32 + 32);
        transfer_data.extend_from_slice(&SEL_TRANSFER);
        transfer_data.extend_from_slice(&encode_address(caller_h160.as_fixed_bytes()));
        transfer_data.extend_from_slice(&encode_u256(amount_u256));

        let sub_context = Context {
            caller: handle.code_address(),
            address: token,
            apparent_value: U256::zero(),
        };

        let gas = handle.remaining_gas().saturating_mul(9) / 10;
        let (reason, ret) = handle.call(token, None, transfer_data, Some(gas), false, &sub_context);

        if !matches!(reason, ExitReason::Succeed(_)) {
            Rewards::<Runtime>::undo_claim_commit(&stash, nonce_u64, amount_pts);
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: ret,
            });
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

        let mut transfer_from_data = Vec::with_capacity(4 + 32 + 32 + 32);
        transfer_from_data.extend_from_slice(&SEL_TRANSFER_FROM);
        transfer_from_data.extend_from_slice(&encode_address(caller_h160.as_fixed_bytes()));
        transfer_from_data.extend_from_slice(&encode_address(precompile_h160.as_fixed_bytes()));
        transfer_from_data.extend_from_slice(&encode_u256(amount_u256));

        // `msg.sender` on the token must be the approved spender (`approve(precompile, …)`).
        // This matches `claim`'s `transfer` subcall (`caller = code_address`), not the EOA.
        let sub_context = Context {
            caller: handle.code_address(),
            address: token,
            apparent_value: U256::zero(),
        };
        let gas = handle.remaining_gas().saturating_mul(9) / 10;
        let (reason, ret) = handle.call(
            token,
            None,
            transfer_from_data,
            Some(gas),
            false,
            &sub_context,
        );
        if !matches!(reason, ExitReason::Succeed(_)) {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: ret,
            });
        }

        let issuer = Runtime::AddressMapping::into_account_id(precompile_h160);

        try_dispatch_attest_coin_no_pov::<Runtime, _>(
            handle,
            Some(issuer).into(),
            pallet_assets::Call::<Runtime>::mint {
                id: ATTEST_COIN_ASSET_ID.into(),
                beneficiary: Runtime::Lookup::unlookup(beneficiary),
                amount: amount_u128.into(),
            },
        )
        .map_err(PrecompileFailure::from)?;

        Ok(PrecompileOutput {
            exit_status: ExitSucceed::Returned,
            output: Vec::new(),
        })
    }

    /// Burn liquid attest coin from the EVM caller’s mapped Substrate account, then send ERC-20 to the
    /// caller (inverse of [`Self::deposit`]).
    ///
    /// Uses [`pallet_assets::Call::burn`] as **admin** (`Signed` precompile account); runtime must set
    /// asset admin to the precompile (see attest-coin migration). If the ERC-20 transfer fails, mints
    /// back to restore the Substrate balance.
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

        let burn_call = pallet_assets::Call::<Runtime>::burn {
            id: ATTEST_COIN_ASSET_ID.into(),
            who: Runtime::Lookup::unlookup(beneficiary.clone()),
            amount: amount_u128.into(),
        };
        let mint_restore = pallet_assets::Call::<Runtime>::mint {
            id: ATTEST_COIN_ASSET_ID.into(),
            beneficiary: Runtime::Lookup::unlookup(beneficiary.clone()),
            amount: amount_u128.into(),
        };

        let gas_burn =
            evm_gas_for_dispatch_call::<Runtime>(&Runtime::RuntimeCall::from(burn_call.clone()));
        let gas_mint =
            evm_gas_for_dispatch_call::<Runtime>(&Runtime::RuntimeCall::from(mint_restore.clone()));
        let mint_gas_reserve = gas_mint.saturating_add(WITHDRAW_MINT_RESTORE_GAS_BUFFER);
        let min_before_burn = gas_burn
            .saturating_add(mint_gas_reserve)
            .saturating_add(WITHDRAW_ERC20_TRANSFER_MIN_GAS)
            .saturating_add(WITHDRAW_PRE_BURN_GAS_SLACK);
        if handle.remaining_gas() < min_before_burn {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"withdraw: insufficient gas".to_vec(),
            });
        }

        try_dispatch_attest_coin_no_pov::<Runtime, _>(
            handle,
            Some(admin.clone()).into(),
            burn_call,
        )
        .map_err(PrecompileFailure::from)?;

        let mut transfer_data = Vec::with_capacity(4 + 32 + 32);
        transfer_data.extend_from_slice(&SEL_TRANSFER);
        transfer_data.extend_from_slice(&encode_address(caller_h160.as_fixed_bytes()));
        transfer_data.extend_from_slice(&encode_u256(amount_u256));

        let sub_context = Context {
            caller: handle.code_address(),
            address: token,
            apparent_value: U256::zero(),
        };
        let gas_for_transfer = handle.remaining_gas().saturating_sub(mint_gas_reserve);
        let (reason, ret) = handle.call(
            token,
            None,
            transfer_data,
            Some(gas_for_transfer),
            false,
            &sub_context,
        );

        if !matches!(reason, ExitReason::Succeed(_)) {
            match try_dispatch_attest_coin_no_pov::<Runtime, _>(
                handle,
                Some(admin).into(),
                mint_restore,
            ) {
                Ok(_) => {
                    return Err(PrecompileFailure::Revert {
                        exit_status: ExitRevert::Reverted,
                        output: ret,
                    });
                }
                Err(_) => {
                    log::error!(
                        target: "precompile::attest_coin",
                        "withdraw: ERC-20 transfer failed and pallet_assets mint-restore also failed; attested on-chain burn may diverge from EVM",
                    );
                    return Err(PrecompileFailure::Revert {
                        exit_status: ExitRevert::Reverted,
                        output: b"withdraw: transfer failed; restore mint failed".to_vec(),
                    });
                }
            }
        }

        Ok(PrecompileOutput {
            exit_status: ExitSucceed::Returned,
            output: Vec::new(),
        })
    }
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

fn account_id_to_sr25519_public<AccountId: Encode>(acct: &AccountId) -> Option<sr25519::Public> {
    let enc = acct.encode();
    if enc.len() < 32 {
        return None;
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(&enc[..32]);
    Some(sr25519::Public::from_raw(a))
}

fn verify_sr25519_stash_or_controller<Runtime>(
    stash: &Runtime::AccountId,
    msg: &[u8],
    sig: &[u8; 64],
) -> bool
where
    Runtime: pallet_staking::Config,
    Runtime::AccountId: Encode,
{
    let mut sig_raw = [0u8; 64];
    sig_raw.copy_from_slice(sig);
    let s = sr25519::Signature::from_raw(sig_raw);

    if let Some(pk) = account_id_to_sr25519_public(stash) {
        if sr25519_verify(&s, msg, &pk) {
            return true;
        }
    }

    if let Some(controller) = Bonded::<Runtime>::get(stash) {
        if let Some(pk) = account_id_to_sr25519_public(&controller) {
            return sr25519_verify(&s, msg, &pk);
        }
    }

    false
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
