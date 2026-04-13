//! Attest-coin precompile: `accrued(bytes32)` view and `claim(...)` with sr25519 + ERC-20 `transfer` from treasury.

#![cfg_attr(not(feature = "std"), no_std)]

use core::marker::PhantomData;
use fp_evm::{
    Context, ExitError, ExitReason, ExitRevert, ExitSucceed, Precompile, PrecompileFailure,
    PrecompileHandle, PrecompileOutput, PrecompileResult,
};
use pallet_attest_coin_rewards::Pallet as Rewards;
use pallet_evm::AddressMapping;
use pallet_staking::Bonded;
use parity_scale_codec::Encode;
use sp_core::{sr25519, H160, U256};
use sp_io::crypto::sr25519_verify;
use sp_std::vec::Vec;

/// `accrued(bytes32)`
const SEL_ACCRUED: [u8; 4] = [0xf9, 0x2f, 0x23, 0xa7];
/// `claim(bytes32,uint256,uint256,uint256,address,bytes32,bytes32)` — see [`claim_selector`].
const SEL_CLAIM: [u8; 4] = [0x1f, 0xfb, 0x7a, 0x3d];
/// ERC-20 `transfer(address,uint256)`
const SEL_TRANSFER: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];

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
        + pallet_attest_coin_rewards::Config
        + pallet_attestation::Config
        + pallet_staking::Config,
    Runtime::AccountId: From<[u8; 32]> + Encode,
    <Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
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
            _ => Err(PrecompileFailure::Error {
                exit_status: ExitError::Other("unknown selector".into()),
            }),
        }
    }
}

impl<Runtime> AttestCoinPrecompile<Runtime>
where
    Runtime: pallet_evm::Config
        + pallet_attest_coin_rewards::Config
        + pallet_attestation::Config
        + pallet_staking::Config,
    Runtime::AccountId: From<[u8; 32]> + Encode,
    <Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
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
        sig[..32].copy_from_slice(&rest[160..192]);
        sig[32..].copy_from_slice(&rest[192..224]);

        let caller_h160 = handle.context().caller;
        if caller_h160 != evm_recipient {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"evm recipient must be caller".to_vec(),
            });
        }

        let nonce_u64 = u256_to_u64(nonce_u256)?;
        let chain_key = u256_to_u64(chain_u256)?;
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

        Rewards::<Runtime>::commit_claim(&stash, nonce_u64, amount_pts).map_err(|_| {
            PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"commit claim failed".to_vec(),
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

fn encode_u256(u: U256) -> Vec<u8> {
    let mut out = [0u8; 32];
    u.to_big_endian(&mut out);
    out.to_vec()
}

fn encode_address(addr: &[u8; 20]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[12..32].copy_from_slice(addr.as_slice());
    out
}

fn u256_to_u64(u: U256) -> Result<u64, PrecompileFailure> {
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
