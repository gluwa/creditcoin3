//! Attest-coin precompile: `accrued(bytes32)` view and `claim(uint256)` → nested `CALL` to ERC-20 `mint`.

#![cfg_attr(not(feature = "std"), no_std)]

use core::marker::PhantomData;
use fp_evm::{
    Context, ExitError, ExitReason, ExitRevert, ExitSucceed, Precompile, PrecompileFailure,
    PrecompileHandle, PrecompileOutput, PrecompileResult,
};
use pallet_attest_coin_rewards::Pallet as Rewards;
use pallet_evm::AddressMapping;
use sp_core::U256;
use sp_std::vec::Vec;

/// `accrued(bytes32)`
const SEL_ACCRUED: [u8; 4] = [0xf9, 0x2f, 0x23, 0xa7];
/// `claim(uint256)`
const SEL_CLAIM: [u8; 4] = [0x37, 0x96, 0x07, 0xf5];
/// `mint(address,uint256)` on the ERC-20
const SEL_MINT: [u8; 4] = [0x40, 0xc1, 0x0f, 0x19];

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
    Runtime: pallet_evm::Config + pallet_attest_coin_rewards::Config + pallet_attestation::Config,
    Runtime::AccountId: From<[u8; 32]>,
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
    Runtime: pallet_evm::Config + pallet_attest_coin_rewards::Config + pallet_attestation::Config,
    Runtime::AccountId: From<[u8; 32]>,
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
        handle.record_cost(50_000)?;
        if rest.len() < 32 {
            return Err(bad_input());
        }
        let amount_u256 = U256::from_big_endian(&rest[..32]);
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
        let stash: Runtime::AccountId = Runtime::AddressMapping::into_account_id(caller_h160);
        if !pallet_attestation::Ledger::<Runtime>::contains_key(&stash) {
            return Err(PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"not a stash".to_vec(),
            });
        }

        let amount_balance = u256_to_reward_points::<Runtime>(amount_u256)?;
        Rewards::<Runtime>::take_accrued_for_claim(&stash, amount_balance).map_err(|_| {
            PrecompileFailure::Revert {
                exit_status: ExitRevert::Reverted,
                output: b"insufficient accrued".to_vec(),
            }
        })?;

        let mut mint_data = Vec::with_capacity(4 + 32 + 32);
        mint_data.extend_from_slice(&SEL_MINT);
        mint_data.extend_from_slice(&encode_address(caller_h160.as_fixed_bytes()));
        mint_data.extend_from_slice(&encode_u256(amount_u256));

        let sub_context = Context {
            caller: handle.code_address(),
            address: token,
            apparent_value: U256::zero(),
        };

        let gas = handle.remaining_gas().saturating_mul(9) / 10;
        let (reason, ret) = handle.call(token, None, mint_data, Some(gas), false, &sub_context);

        if !matches!(reason, ExitReason::Succeed(_)) {
            Rewards::<Runtime>::restore_accrued(&stash, amount_balance);
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
