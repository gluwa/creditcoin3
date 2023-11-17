// Auto-generated via `yarn polkadot-types-from-chain`, do not edit
/* eslint-disable */

// import type lookup before we augment - in some environments
// this is required to allow for ambient/previous definitions
import '@polkadot/api-base/types/submittable';

import type {
    ApiTypes,
    AugmentedSubmittable,
    SubmittableExtrinsic,
    SubmittableExtrinsicFunction,
} from '@polkadot/api-base/types';
import type { Data } from '@polkadot/types';
import type { Bytes, Compact, Option, U256, Vec, bool, u128, u16, u32, u64 } from '@polkadot/types-codec';
import type { AnyNumber, IMethod, ITuple } from '@polkadot/types-codec/types';
import type { AccountId32, Call, H160, H256, Perbill, Percent, Permill } from '@polkadot/types/interfaces/runtime';
import {
    SpConsensusBabeDigestsNextConfigDescriptor,
    SpConsensusSlotsEquivocationProof,
    SpSessionMembershipProof,
    EthereumTransactionTransactionV2,
    SpConsensusGrandpaEquivocationProof,
    PalletIdentityJudgement,
    PalletIdentityBitFlags,
    PalletIdentityIdentityInfo,
    PalletImOnlineHeartbeat,
    PalletImOnlineSr25519AppSr25519Signature,
    PalletNominationPoolsBondExtra,
    PalletNominationPoolsClaimPermission,
    PalletNominationPoolsCommissionChangeRate,
    PalletNominationPoolsConfigOpU128,
    PalletNominationPoolsConfigOpU32,
    PalletNominationPoolsConfigOpPerbill,
    PalletNominationPoolsPoolState,
    PalletNominationPoolsConfigOpAccountId32,
    Creditcoin3RuntimeProxyFilter,
    Creditcoin3RuntimeOpaqueSessionKeys,
    PalletStakingRewardDestination,
    PalletStakingPalletConfigOpU128,
    PalletStakingPalletConfigOpU32,
    PalletStakingPalletConfigOpPercent,
    PalletStakingPalletConfigOpPerbill,
    PalletStakingValidatorPrefs,
    SpWeightsWeightV2Weight,
    Creditcoin3RuntimeOriginCaller,
} from '@polkadot/types/lookup';

export type __AugmentedSubmittable = AugmentedSubmittable<() => unknown>;
export type __SubmittableExtrinsic<ApiType extends ApiTypes> = SubmittableExtrinsic<ApiType>;
export type __SubmittableExtrinsicFunction<ApiType extends ApiTypes> = SubmittableExtrinsicFunction<ApiType>;

declare module '@polkadot/api-base/types/submittable' {
    interface AugmentedSubmittables<ApiType extends ApiTypes> {
        babe: {
            /**
             * See [`Pallet::plan_config_change`].
             **/
            planConfigChange: AugmentedSubmittable<
                (
                    config: SpConsensusBabeDigestsNextConfigDescriptor | { V1: any } | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [SpConsensusBabeDigestsNextConfigDescriptor]
            >;
            /**
             * See [`Pallet::report_equivocation`].
             **/
            reportEquivocation: AugmentedSubmittable<
                (
                    equivocationProof:
                        | SpConsensusSlotsEquivocationProof
                        | { offender?: any; slot?: any; firstHeader?: any; secondHeader?: any }
                        | string
                        | Uint8Array,
                    keyOwnerProof:
                        | SpSessionMembershipProof
                        | { session?: any; trieNodes?: any; validatorCount?: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [SpConsensusSlotsEquivocationProof, SpSessionMembershipProof]
            >;
            /**
             * See [`Pallet::report_equivocation_unsigned`].
             **/
            reportEquivocationUnsigned: AugmentedSubmittable<
                (
                    equivocationProof:
                        | SpConsensusSlotsEquivocationProof
                        | { offender?: any; slot?: any; firstHeader?: any; secondHeader?: any }
                        | string
                        | Uint8Array,
                    keyOwnerProof:
                        | SpSessionMembershipProof
                        | { session?: any; trieNodes?: any; validatorCount?: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [SpConsensusSlotsEquivocationProof, SpSessionMembershipProof]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        balances: {
            /**
             * See [`Pallet::force_set_balance`].
             **/
            forceSetBalance: AugmentedSubmittable<
                (
                    who: AccountId32 | string | Uint8Array,
                    newFree: Compact<u128> | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, Compact<u128>]
            >;
            /**
             * See [`Pallet::force_transfer`].
             **/
            forceTransfer: AugmentedSubmittable<
                (
                    source: AccountId32 | string | Uint8Array,
                    dest: AccountId32 | string | Uint8Array,
                    value: Compact<u128> | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, AccountId32, Compact<u128>]
            >;
            /**
             * See [`Pallet::force_unreserve`].
             **/
            forceUnreserve: AugmentedSubmittable<
                (
                    who: AccountId32 | string | Uint8Array,
                    amount: u128 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, u128]
            >;
            /**
             * See [`Pallet::set_balance_deprecated`].
             **/
            setBalanceDeprecated: AugmentedSubmittable<
                (
                    who: AccountId32 | string | Uint8Array,
                    newFree: Compact<u128> | AnyNumber | Uint8Array,
                    oldReserved: Compact<u128> | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, Compact<u128>, Compact<u128>]
            >;
            /**
             * See [`Pallet::transfer`].
             **/
            transfer: AugmentedSubmittable<
                (
                    dest: AccountId32 | string | Uint8Array,
                    value: Compact<u128> | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, Compact<u128>]
            >;
            /**
             * See [`Pallet::transfer_all`].
             **/
            transferAll: AugmentedSubmittable<
                (
                    dest: AccountId32 | string | Uint8Array,
                    keepAlive: bool | boolean | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, bool]
            >;
            /**
             * See [`Pallet::transfer_allow_death`].
             **/
            transferAllowDeath: AugmentedSubmittable<
                (
                    dest: AccountId32 | string | Uint8Array,
                    value: Compact<u128> | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, Compact<u128>]
            >;
            /**
             * See [`Pallet::transfer_keep_alive`].
             **/
            transferKeepAlive: AugmentedSubmittable<
                (
                    dest: AccountId32 | string | Uint8Array,
                    value: Compact<u128> | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, Compact<u128>]
            >;
            /**
             * See [`Pallet::upgrade_accounts`].
             **/
            upgradeAccounts: AugmentedSubmittable<
                (who: Vec<AccountId32> | (AccountId32 | string | Uint8Array)[]) => SubmittableExtrinsic<ApiType>,
                [Vec<AccountId32>]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        baseFee: {
            /**
             * See [`Pallet::set_base_fee_per_gas`].
             **/
            setBaseFeePerGas: AugmentedSubmittable<
                (fee: U256 | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [U256]
            >;
            /**
             * See [`Pallet::set_elasticity`].
             **/
            setElasticity: AugmentedSubmittable<
                (elasticity: Permill | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Permill]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        dynamicFee: {
            /**
             * See [`Pallet::note_min_gas_price_target`].
             **/
            noteMinGasPriceTarget: AugmentedSubmittable<
                (target: U256 | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [U256]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        ethereum: {
            /**
             * See [`Pallet::transact`].
             **/
            transact: AugmentedSubmittable<
                (
                    transaction:
                        | EthereumTransactionTransactionV2
                        | { Legacy: any }
                        | { EIP2930: any }
                        | { EIP1559: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [EthereumTransactionTransactionV2]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        evm: {
            /**
             * See [`Pallet::call`].
             **/
            call: AugmentedSubmittable<
                (
                    source: H160 | string | Uint8Array,
                    target: H160 | string | Uint8Array,
                    input: Bytes | string | Uint8Array,
                    value: U256 | AnyNumber | Uint8Array,
                    gasLimit: u64 | AnyNumber | Uint8Array,
                    maxFeePerGas: U256 | AnyNumber | Uint8Array,
                    maxPriorityFeePerGas: Option<U256> | null | Uint8Array | U256 | AnyNumber,
                    nonce: Option<U256> | null | Uint8Array | U256 | AnyNumber,
                    accessList:
                        | Vec<ITuple<[H160, Vec<H256>]>>
                        | [H160 | string | Uint8Array, Vec<H256> | (H256 | string | Uint8Array)[]][],
                ) => SubmittableExtrinsic<ApiType>,
                [H160, H160, Bytes, U256, u64, U256, Option<U256>, Option<U256>, Vec<ITuple<[H160, Vec<H256>]>>]
            >;
            /**
             * See [`Pallet::create`].
             **/
            create: AugmentedSubmittable<
                (
                    source: H160 | string | Uint8Array,
                    init: Bytes | string | Uint8Array,
                    value: U256 | AnyNumber | Uint8Array,
                    gasLimit: u64 | AnyNumber | Uint8Array,
                    maxFeePerGas: U256 | AnyNumber | Uint8Array,
                    maxPriorityFeePerGas: Option<U256> | null | Uint8Array | U256 | AnyNumber,
                    nonce: Option<U256> | null | Uint8Array | U256 | AnyNumber,
                    accessList:
                        | Vec<ITuple<[H160, Vec<H256>]>>
                        | [H160 | string | Uint8Array, Vec<H256> | (H256 | string | Uint8Array)[]][],
                ) => SubmittableExtrinsic<ApiType>,
                [H160, Bytes, U256, u64, U256, Option<U256>, Option<U256>, Vec<ITuple<[H160, Vec<H256>]>>]
            >;
            /**
             * See [`Pallet::create2`].
             **/
            create2: AugmentedSubmittable<
                (
                    source: H160 | string | Uint8Array,
                    init: Bytes | string | Uint8Array,
                    salt: H256 | string | Uint8Array,
                    value: U256 | AnyNumber | Uint8Array,
                    gasLimit: u64 | AnyNumber | Uint8Array,
                    maxFeePerGas: U256 | AnyNumber | Uint8Array,
                    maxPriorityFeePerGas: Option<U256> | null | Uint8Array | U256 | AnyNumber,
                    nonce: Option<U256> | null | Uint8Array | U256 | AnyNumber,
                    accessList:
                        | Vec<ITuple<[H160, Vec<H256>]>>
                        | [H160 | string | Uint8Array, Vec<H256> | (H256 | string | Uint8Array)[]][],
                ) => SubmittableExtrinsic<ApiType>,
                [H160, Bytes, H256, U256, u64, U256, Option<U256>, Option<U256>, Vec<ITuple<[H160, Vec<H256>]>>]
            >;
            /**
             * See [`Pallet::withdraw`].
             **/
            withdraw: AugmentedSubmittable<
                (
                    address: H160 | string | Uint8Array,
                    value: u128 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [H160, u128]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        fastUnstake: {
            /**
             * See [`Pallet::control`].
             **/
            control: AugmentedSubmittable<
                (erasToCheck: u32 | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [u32]
            >;
            /**
             * See [`Pallet::deregister`].
             **/
            deregister: AugmentedSubmittable<() => SubmittableExtrinsic<ApiType>, []>;
            /**
             * See [`Pallet::register_fast_unstake`].
             **/
            registerFastUnstake: AugmentedSubmittable<() => SubmittableExtrinsic<ApiType>, []>;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        grandpa: {
            /**
             * See [`Pallet::note_stalled`].
             **/
            noteStalled: AugmentedSubmittable<
                (
                    delay: u32 | AnyNumber | Uint8Array,
                    bestFinalizedBlockNumber: u32 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [u32, u32]
            >;
            /**
             * See [`Pallet::report_equivocation`].
             **/
            reportEquivocation: AugmentedSubmittable<
                (
                    equivocationProof:
                        | SpConsensusGrandpaEquivocationProof
                        | { setId?: any; equivocation?: any }
                        | string
                        | Uint8Array,
                    keyOwnerProof:
                        | SpSessionMembershipProof
                        | { session?: any; trieNodes?: any; validatorCount?: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [SpConsensusGrandpaEquivocationProof, SpSessionMembershipProof]
            >;
            /**
             * See [`Pallet::report_equivocation_unsigned`].
             **/
            reportEquivocationUnsigned: AugmentedSubmittable<
                (
                    equivocationProof:
                        | SpConsensusGrandpaEquivocationProof
                        | { setId?: any; equivocation?: any }
                        | string
                        | Uint8Array,
                    keyOwnerProof:
                        | SpSessionMembershipProof
                        | { session?: any; trieNodes?: any; validatorCount?: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [SpConsensusGrandpaEquivocationProof, SpSessionMembershipProof]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        hotfixSufficients: {
            /**
             * See [`Pallet::hotfix_inc_account_sufficients`].
             **/
            hotfixIncAccountSufficients: AugmentedSubmittable<
                (addresses: Vec<H160> | (H160 | string | Uint8Array)[]) => SubmittableExtrinsic<ApiType>,
                [Vec<H160>]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        identity: {
            /**
             * See [`Pallet::add_registrar`].
             **/
            addRegistrar: AugmentedSubmittable<
                (account: AccountId32 | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [AccountId32]
            >;
            /**
             * See [`Pallet::add_sub`].
             **/
            addSub: AugmentedSubmittable<
                (
                    sub: AccountId32 | string | Uint8Array,
                    data:
                        | Data
                        | { None: any }
                        | { Raw: any }
                        | { BlakeTwo256: any }
                        | { Sha256: any }
                        | { Keccak256: any }
                        | { ShaThree256: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, Data]
            >;
            /**
             * See [`Pallet::cancel_request`].
             **/
            cancelRequest: AugmentedSubmittable<
                (regIndex: u32 | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [u32]
            >;
            /**
             * See [`Pallet::clear_identity`].
             **/
            clearIdentity: AugmentedSubmittable<() => SubmittableExtrinsic<ApiType>, []>;
            /**
             * See [`Pallet::kill_identity`].
             **/
            killIdentity: AugmentedSubmittable<
                (target: AccountId32 | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [AccountId32]
            >;
            /**
             * See [`Pallet::provide_judgement`].
             **/
            provideJudgement: AugmentedSubmittable<
                (
                    regIndex: Compact<u32> | AnyNumber | Uint8Array,
                    target: AccountId32 | string | Uint8Array,
                    judgement:
                        | PalletIdentityJudgement
                        | { Unknown: any }
                        | { FeePaid: any }
                        | { Reasonable: any }
                        | { KnownGood: any }
                        | { OutOfDate: any }
                        | { LowQuality: any }
                        | { Erroneous: any }
                        | string
                        | Uint8Array,
                    identity: H256 | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Compact<u32>, AccountId32, PalletIdentityJudgement, H256]
            >;
            /**
             * See [`Pallet::quit_sub`].
             **/
            quitSub: AugmentedSubmittable<() => SubmittableExtrinsic<ApiType>, []>;
            /**
             * See [`Pallet::remove_sub`].
             **/
            removeSub: AugmentedSubmittable<
                (sub: AccountId32 | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [AccountId32]
            >;
            /**
             * See [`Pallet::rename_sub`].
             **/
            renameSub: AugmentedSubmittable<
                (
                    sub: AccountId32 | string | Uint8Array,
                    data:
                        | Data
                        | { None: any }
                        | { Raw: any }
                        | { BlakeTwo256: any }
                        | { Sha256: any }
                        | { Keccak256: any }
                        | { ShaThree256: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, Data]
            >;
            /**
             * See [`Pallet::request_judgement`].
             **/
            requestJudgement: AugmentedSubmittable<
                (
                    regIndex: Compact<u32> | AnyNumber | Uint8Array,
                    maxFee: Compact<u128> | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Compact<u32>, Compact<u128>]
            >;
            /**
             * See [`Pallet::set_account_id`].
             **/
            setAccountId: AugmentedSubmittable<
                (
                    index: Compact<u32> | AnyNumber | Uint8Array,
                    updated: AccountId32 | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Compact<u32>, AccountId32]
            >;
            /**
             * See [`Pallet::set_fee`].
             **/
            setFee: AugmentedSubmittable<
                (
                    index: Compact<u32> | AnyNumber | Uint8Array,
                    fee: Compact<u128> | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Compact<u32>, Compact<u128>]
            >;
            /**
             * See [`Pallet::set_fields`].
             **/
            setFields: AugmentedSubmittable<
                (
                    index: Compact<u32> | AnyNumber | Uint8Array,
                    fields: PalletIdentityBitFlags,
                ) => SubmittableExtrinsic<ApiType>,
                [Compact<u32>, PalletIdentityBitFlags]
            >;
            /**
             * See [`Pallet::set_identity`].
             **/
            setIdentity: AugmentedSubmittable<
                (
                    info:
                        | PalletIdentityIdentityInfo
                        | {
                              additional?: any;
                              display?: any;
                              legal?: any;
                              web?: any;
                              riot?: any;
                              email?: any;
                              pgpFingerprint?: any;
                              image?: any;
                              twitter?: any;
                          }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [PalletIdentityIdentityInfo]
            >;
            /**
             * See [`Pallet::set_subs`].
             **/
            setSubs: AugmentedSubmittable<
                (
                    subs:
                        | Vec<ITuple<[AccountId32, Data]>>
                        | [
                              AccountId32 | string | Uint8Array,
                              (
                                  | Data
                                  | { None: any }
                                  | { Raw: any }
                                  | { BlakeTwo256: any }
                                  | { Sha256: any }
                                  | { Keccak256: any }
                                  | { ShaThree256: any }
                                  | string
                                  | Uint8Array
                              ),
                          ][],
                ) => SubmittableExtrinsic<ApiType>,
                [Vec<ITuple<[AccountId32, Data]>>]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        imOnline: {
            /**
             * See [`Pallet::heartbeat`].
             **/
            heartbeat: AugmentedSubmittable<
                (
                    heartbeat:
                        | PalletImOnlineHeartbeat
                        | { blockNumber?: any; sessionIndex?: any; authorityIndex?: any; validatorsLen?: any }
                        | string
                        | Uint8Array,
                    signature: PalletImOnlineSr25519AppSr25519Signature | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [PalletImOnlineHeartbeat, PalletImOnlineSr25519AppSr25519Signature]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        nominationPools: {
            /**
             * See [`Pallet::bond_extra`].
             **/
            bondExtra: AugmentedSubmittable<
                (
                    extra:
                        | PalletNominationPoolsBondExtra
                        | { FreeBalance: any }
                        | { Rewards: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [PalletNominationPoolsBondExtra]
            >;
            /**
             * See [`Pallet::bond_extra_other`].
             **/
            bondExtraOther: AugmentedSubmittable<
                (
                    member: AccountId32 | string | Uint8Array,
                    extra:
                        | PalletNominationPoolsBondExtra
                        | { FreeBalance: any }
                        | { Rewards: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, PalletNominationPoolsBondExtra]
            >;
            /**
             * See [`Pallet::chill`].
             **/
            chill: AugmentedSubmittable<(poolId: u32 | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>, [u32]>;
            /**
             * See [`Pallet::claim_commission`].
             **/
            claimCommission: AugmentedSubmittable<
                (poolId: u32 | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [u32]
            >;
            /**
             * See [`Pallet::claim_payout`].
             **/
            claimPayout: AugmentedSubmittable<() => SubmittableExtrinsic<ApiType>, []>;
            /**
             * See [`Pallet::claim_payout_other`].
             **/
            claimPayoutOther: AugmentedSubmittable<
                (other: AccountId32 | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [AccountId32]
            >;
            /**
             * See [`Pallet::create`].
             **/
            create: AugmentedSubmittable<
                (
                    amount: Compact<u128> | AnyNumber | Uint8Array,
                    root: AccountId32 | string | Uint8Array,
                    nominator: AccountId32 | string | Uint8Array,
                    bouncer: AccountId32 | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Compact<u128>, AccountId32, AccountId32, AccountId32]
            >;
            /**
             * See [`Pallet::create_with_pool_id`].
             **/
            createWithPoolId: AugmentedSubmittable<
                (
                    amount: Compact<u128> | AnyNumber | Uint8Array,
                    root: AccountId32 | string | Uint8Array,
                    nominator: AccountId32 | string | Uint8Array,
                    bouncer: AccountId32 | string | Uint8Array,
                    poolId: u32 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Compact<u128>, AccountId32, AccountId32, AccountId32, u32]
            >;
            /**
             * See [`Pallet::join`].
             **/
            join: AugmentedSubmittable<
                (
                    amount: Compact<u128> | AnyNumber | Uint8Array,
                    poolId: u32 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Compact<u128>, u32]
            >;
            /**
             * See [`Pallet::nominate`].
             **/
            nominate: AugmentedSubmittable<
                (
                    poolId: u32 | AnyNumber | Uint8Array,
                    validators: Vec<AccountId32> | (AccountId32 | string | Uint8Array)[],
                ) => SubmittableExtrinsic<ApiType>,
                [u32, Vec<AccountId32>]
            >;
            /**
             * See [`Pallet::pool_withdraw_unbonded`].
             **/
            poolWithdrawUnbonded: AugmentedSubmittable<
                (
                    poolId: u32 | AnyNumber | Uint8Array,
                    numSlashingSpans: u32 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [u32, u32]
            >;
            /**
             * See [`Pallet::set_claim_permission`].
             **/
            setClaimPermission: AugmentedSubmittable<
                (
                    permission:
                        | PalletNominationPoolsClaimPermission
                        | 'Permissioned'
                        | 'PermissionlessCompound'
                        | 'PermissionlessWithdraw'
                        | 'PermissionlessAll'
                        | number
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [PalletNominationPoolsClaimPermission]
            >;
            /**
             * See [`Pallet::set_commission`].
             **/
            setCommission: AugmentedSubmittable<
                (
                    poolId: u32 | AnyNumber | Uint8Array,
                    newCommission:
                        | Option<ITuple<[Perbill, AccountId32]>>
                        | null
                        | Uint8Array
                        | ITuple<[Perbill, AccountId32]>
                        | [Perbill | AnyNumber | Uint8Array, AccountId32 | string | Uint8Array],
                ) => SubmittableExtrinsic<ApiType>,
                [u32, Option<ITuple<[Perbill, AccountId32]>>]
            >;
            /**
             * See [`Pallet::set_commission_change_rate`].
             **/
            setCommissionChangeRate: AugmentedSubmittable<
                (
                    poolId: u32 | AnyNumber | Uint8Array,
                    changeRate:
                        | PalletNominationPoolsCommissionChangeRate
                        | { maxIncrease?: any; minDelay?: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [u32, PalletNominationPoolsCommissionChangeRate]
            >;
            /**
             * See [`Pallet::set_commission_max`].
             **/
            setCommissionMax: AugmentedSubmittable<
                (
                    poolId: u32 | AnyNumber | Uint8Array,
                    maxCommission: Perbill | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [u32, Perbill]
            >;
            /**
             * See [`Pallet::set_configs`].
             **/
            setConfigs: AugmentedSubmittable<
                (
                    minJoinBond:
                        | PalletNominationPoolsConfigOpU128
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                    minCreateBond:
                        | PalletNominationPoolsConfigOpU128
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                    maxPools:
                        | PalletNominationPoolsConfigOpU32
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                    maxMembers:
                        | PalletNominationPoolsConfigOpU32
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                    maxMembersPerPool:
                        | PalletNominationPoolsConfigOpU32
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                    globalMaxCommission:
                        | PalletNominationPoolsConfigOpPerbill
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [
                    PalletNominationPoolsConfigOpU128,
                    PalletNominationPoolsConfigOpU128,
                    PalletNominationPoolsConfigOpU32,
                    PalletNominationPoolsConfigOpU32,
                    PalletNominationPoolsConfigOpU32,
                    PalletNominationPoolsConfigOpPerbill,
                ]
            >;
            /**
             * See [`Pallet::set_metadata`].
             **/
            setMetadata: AugmentedSubmittable<
                (
                    poolId: u32 | AnyNumber | Uint8Array,
                    metadata: Bytes | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [u32, Bytes]
            >;
            /**
             * See [`Pallet::set_state`].
             **/
            setState: AugmentedSubmittable<
                (
                    poolId: u32 | AnyNumber | Uint8Array,
                    state: PalletNominationPoolsPoolState | 'Open' | 'Blocked' | 'Destroying' | number | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [u32, PalletNominationPoolsPoolState]
            >;
            /**
             * See [`Pallet::unbond`].
             **/
            unbond: AugmentedSubmittable<
                (
                    memberAccount: AccountId32 | string | Uint8Array,
                    unbondingPoints: Compact<u128> | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, Compact<u128>]
            >;
            /**
             * See [`Pallet::update_roles`].
             **/
            updateRoles: AugmentedSubmittable<
                (
                    poolId: u32 | AnyNumber | Uint8Array,
                    newRoot:
                        | PalletNominationPoolsConfigOpAccountId32
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                    newNominator:
                        | PalletNominationPoolsConfigOpAccountId32
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                    newBouncer:
                        | PalletNominationPoolsConfigOpAccountId32
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [
                    u32,
                    PalletNominationPoolsConfigOpAccountId32,
                    PalletNominationPoolsConfigOpAccountId32,
                    PalletNominationPoolsConfigOpAccountId32,
                ]
            >;
            /**
             * See [`Pallet::withdraw_unbonded`].
             **/
            withdrawUnbonded: AugmentedSubmittable<
                (
                    memberAccount: AccountId32 | string | Uint8Array,
                    numSlashingSpans: u32 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, u32]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        proxy: {
            /**
             * See [`Pallet::add_proxy`].
             **/
            addProxy: AugmentedSubmittable<
                (
                    delegate: AccountId32 | string | Uint8Array,
                    proxyType: Creditcoin3RuntimeProxyFilter | 'All' | 'NonTransfer' | 'Staking' | number | Uint8Array,
                    delay: u32 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, Creditcoin3RuntimeProxyFilter, u32]
            >;
            /**
             * See [`Pallet::announce`].
             **/
            announce: AugmentedSubmittable<
                (
                    real: AccountId32 | string | Uint8Array,
                    callHash: H256 | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, H256]
            >;
            /**
             * See [`Pallet::create_pure`].
             **/
            createPure: AugmentedSubmittable<
                (
                    proxyType: Creditcoin3RuntimeProxyFilter | 'All' | 'NonTransfer' | 'Staking' | number | Uint8Array,
                    delay: u32 | AnyNumber | Uint8Array,
                    index: u16 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Creditcoin3RuntimeProxyFilter, u32, u16]
            >;
            /**
             * See [`Pallet::kill_pure`].
             **/
            killPure: AugmentedSubmittable<
                (
                    spawner: AccountId32 | string | Uint8Array,
                    proxyType: Creditcoin3RuntimeProxyFilter | 'All' | 'NonTransfer' | 'Staking' | number | Uint8Array,
                    index: u16 | AnyNumber | Uint8Array,
                    height: Compact<u32> | AnyNumber | Uint8Array,
                    extIndex: Compact<u32> | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, Creditcoin3RuntimeProxyFilter, u16, Compact<u32>, Compact<u32>]
            >;
            /**
             * See [`Pallet::proxy`].
             **/
            proxy: AugmentedSubmittable<
                (
                    real: AccountId32 | string | Uint8Array,
                    forceProxyType:
                        | Option<Creditcoin3RuntimeProxyFilter>
                        | null
                        | Uint8Array
                        | Creditcoin3RuntimeProxyFilter
                        | 'All'
                        | 'NonTransfer'
                        | 'Staking'
                        | number,
                    call: Call | IMethod | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, Option<Creditcoin3RuntimeProxyFilter>, Call]
            >;
            /**
             * See [`Pallet::proxy_announced`].
             **/
            proxyAnnounced: AugmentedSubmittable<
                (
                    delegate: AccountId32 | string | Uint8Array,
                    real: AccountId32 | string | Uint8Array,
                    forceProxyType:
                        | Option<Creditcoin3RuntimeProxyFilter>
                        | null
                        | Uint8Array
                        | Creditcoin3RuntimeProxyFilter
                        | 'All'
                        | 'NonTransfer'
                        | 'Staking'
                        | number,
                    call: Call | IMethod | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, AccountId32, Option<Creditcoin3RuntimeProxyFilter>, Call]
            >;
            /**
             * See [`Pallet::reject_announcement`].
             **/
            rejectAnnouncement: AugmentedSubmittable<
                (
                    delegate: AccountId32 | string | Uint8Array,
                    callHash: H256 | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, H256]
            >;
            /**
             * See [`Pallet::remove_announcement`].
             **/
            removeAnnouncement: AugmentedSubmittable<
                (
                    real: AccountId32 | string | Uint8Array,
                    callHash: H256 | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, H256]
            >;
            /**
             * See [`Pallet::remove_proxies`].
             **/
            removeProxies: AugmentedSubmittable<() => SubmittableExtrinsic<ApiType>, []>;
            /**
             * See [`Pallet::remove_proxy`].
             **/
            removeProxy: AugmentedSubmittable<
                (
                    delegate: AccountId32 | string | Uint8Array,
                    proxyType: Creditcoin3RuntimeProxyFilter | 'All' | 'NonTransfer' | 'Staking' | number | Uint8Array,
                    delay: u32 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, Creditcoin3RuntimeProxyFilter, u32]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        session: {
            /**
             * See [`Pallet::purge_keys`].
             **/
            purgeKeys: AugmentedSubmittable<() => SubmittableExtrinsic<ApiType>, []>;
            /**
             * See [`Pallet::set_keys`].
             **/
            setKeys: AugmentedSubmittable<
                (
                    keys:
                        | Creditcoin3RuntimeOpaqueSessionKeys
                        | { grandpa?: any; babe?: any; imOnline?: any }
                        | string
                        | Uint8Array,
                    proof: Bytes | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Creditcoin3RuntimeOpaqueSessionKeys, Bytes]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        staking: {
            /**
             * See [`Pallet::bond`].
             **/
            bond: AugmentedSubmittable<
                (
                    value: Compact<u128> | AnyNumber | Uint8Array,
                    payee:
                        | PalletStakingRewardDestination
                        | { Staked: any }
                        | { Stash: any }
                        | { Controller: any }
                        | { Account: any }
                        | { None: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Compact<u128>, PalletStakingRewardDestination]
            >;
            /**
             * See [`Pallet::bond_extra`].
             **/
            bondExtra: AugmentedSubmittable<
                (maxAdditional: Compact<u128> | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Compact<u128>]
            >;
            /**
             * See [`Pallet::cancel_deferred_slash`].
             **/
            cancelDeferredSlash: AugmentedSubmittable<
                (
                    era: u32 | AnyNumber | Uint8Array,
                    slashIndices: Vec<u32> | (u32 | AnyNumber | Uint8Array)[],
                ) => SubmittableExtrinsic<ApiType>,
                [u32, Vec<u32>]
            >;
            /**
             * See [`Pallet::chill`].
             **/
            chill: AugmentedSubmittable<() => SubmittableExtrinsic<ApiType>, []>;
            /**
             * See [`Pallet::chill_other`].
             **/
            chillOther: AugmentedSubmittable<
                (controller: AccountId32 | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [AccountId32]
            >;
            /**
             * See [`Pallet::force_apply_min_commission`].
             **/
            forceApplyMinCommission: AugmentedSubmittable<
                (validatorStash: AccountId32 | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [AccountId32]
            >;
            /**
             * See [`Pallet::force_new_era`].
             **/
            forceNewEra: AugmentedSubmittable<() => SubmittableExtrinsic<ApiType>, []>;
            /**
             * See [`Pallet::force_new_era_always`].
             **/
            forceNewEraAlways: AugmentedSubmittable<() => SubmittableExtrinsic<ApiType>, []>;
            /**
             * See [`Pallet::force_no_eras`].
             **/
            forceNoEras: AugmentedSubmittable<() => SubmittableExtrinsic<ApiType>, []>;
            /**
             * See [`Pallet::force_unstake`].
             **/
            forceUnstake: AugmentedSubmittable<
                (
                    stash: AccountId32 | string | Uint8Array,
                    numSlashingSpans: u32 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, u32]
            >;
            /**
             * See [`Pallet::increase_validator_count`].
             **/
            increaseValidatorCount: AugmentedSubmittable<
                (additional: Compact<u32> | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Compact<u32>]
            >;
            /**
             * See [`Pallet::kick`].
             **/
            kick: AugmentedSubmittable<
                (who: Vec<AccountId32> | (AccountId32 | string | Uint8Array)[]) => SubmittableExtrinsic<ApiType>,
                [Vec<AccountId32>]
            >;
            /**
             * See [`Pallet::nominate`].
             **/
            nominate: AugmentedSubmittable<
                (targets: Vec<AccountId32> | (AccountId32 | string | Uint8Array)[]) => SubmittableExtrinsic<ApiType>,
                [Vec<AccountId32>]
            >;
            /**
             * See [`Pallet::payout_stakers`].
             **/
            payoutStakers: AugmentedSubmittable<
                (
                    validatorStash: AccountId32 | string | Uint8Array,
                    era: u32 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, u32]
            >;
            /**
             * See [`Pallet::reap_stash`].
             **/
            reapStash: AugmentedSubmittable<
                (
                    stash: AccountId32 | string | Uint8Array,
                    numSlashingSpans: u32 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, u32]
            >;
            /**
             * See [`Pallet::rebond`].
             **/
            rebond: AugmentedSubmittable<
                (value: Compact<u128> | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Compact<u128>]
            >;
            /**
             * See [`Pallet::scale_validator_count`].
             **/
            scaleValidatorCount: AugmentedSubmittable<
                (factor: Percent | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Percent]
            >;
            /**
             * See [`Pallet::set_controller`].
             **/
            setController: AugmentedSubmittable<() => SubmittableExtrinsic<ApiType>, []>;
            /**
             * See [`Pallet::set_invulnerables`].
             **/
            setInvulnerables: AugmentedSubmittable<
                (
                    invulnerables: Vec<AccountId32> | (AccountId32 | string | Uint8Array)[],
                ) => SubmittableExtrinsic<ApiType>,
                [Vec<AccountId32>]
            >;
            /**
             * See [`Pallet::set_min_commission`].
             **/
            setMinCommission: AugmentedSubmittable<
                (updated: Perbill | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Perbill]
            >;
            /**
             * See [`Pallet::set_payee`].
             **/
            setPayee: AugmentedSubmittable<
                (
                    payee:
                        | PalletStakingRewardDestination
                        | { Staked: any }
                        | { Stash: any }
                        | { Controller: any }
                        | { Account: any }
                        | { None: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [PalletStakingRewardDestination]
            >;
            /**
             * See [`Pallet::set_staking_configs`].
             **/
            setStakingConfigs: AugmentedSubmittable<
                (
                    minNominatorBond:
                        | PalletStakingPalletConfigOpU128
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                    minValidatorBond:
                        | PalletStakingPalletConfigOpU128
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                    maxNominatorCount:
                        | PalletStakingPalletConfigOpU32
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                    maxValidatorCount:
                        | PalletStakingPalletConfigOpU32
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                    chillThreshold:
                        | PalletStakingPalletConfigOpPercent
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                    minCommission:
                        | PalletStakingPalletConfigOpPerbill
                        | { Noop: any }
                        | { Set: any }
                        | { Remove: any }
                        | string
                        | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [
                    PalletStakingPalletConfigOpU128,
                    PalletStakingPalletConfigOpU128,
                    PalletStakingPalletConfigOpU32,
                    PalletStakingPalletConfigOpU32,
                    PalletStakingPalletConfigOpPercent,
                    PalletStakingPalletConfigOpPerbill,
                ]
            >;
            /**
             * See [`Pallet::set_validator_count`].
             **/
            setValidatorCount: AugmentedSubmittable<
                (updated: Compact<u32> | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Compact<u32>]
            >;
            /**
             * See [`Pallet::unbond`].
             **/
            unbond: AugmentedSubmittable<
                (value: Compact<u128> | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Compact<u128>]
            >;
            /**
             * See [`Pallet::validate`].
             **/
            validate: AugmentedSubmittable<
                (
                    prefs: PalletStakingValidatorPrefs | { commission?: any; blocked?: any } | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [PalletStakingValidatorPrefs]
            >;
            /**
             * See [`Pallet::withdraw_unbonded`].
             **/
            withdrawUnbonded: AugmentedSubmittable<
                (numSlashingSpans: u32 | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [u32]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        sudo: {
            /**
             * See [`Pallet::set_key`].
             **/
            setKey: AugmentedSubmittable<
                (updated: AccountId32 | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [AccountId32]
            >;
            /**
             * See [`Pallet::sudo`].
             **/
            sudo: AugmentedSubmittable<
                (call: Call | IMethod | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Call]
            >;
            /**
             * See [`Pallet::sudo_as`].
             **/
            sudoAs: AugmentedSubmittable<
                (
                    who: AccountId32 | string | Uint8Array,
                    call: Call | IMethod | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, Call]
            >;
            /**
             * See [`Pallet::sudo_unchecked_weight`].
             **/
            sudoUncheckedWeight: AugmentedSubmittable<
                (
                    call: Call | IMethod | string | Uint8Array,
                    weight: SpWeightsWeightV2Weight | { refTime?: any; proofSize?: any } | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Call, SpWeightsWeightV2Weight]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        system: {
            /**
             * See [`Pallet::kill_prefix`].
             **/
            killPrefix: AugmentedSubmittable<
                (
                    prefix: Bytes | string | Uint8Array,
                    subkeys: u32 | AnyNumber | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Bytes, u32]
            >;
            /**
             * See [`Pallet::kill_storage`].
             **/
            killStorage: AugmentedSubmittable<
                (keys: Vec<Bytes> | (Bytes | string | Uint8Array)[]) => SubmittableExtrinsic<ApiType>,
                [Vec<Bytes>]
            >;
            /**
             * See [`Pallet::remark`].
             **/
            remark: AugmentedSubmittable<
                (remark: Bytes | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Bytes]
            >;
            /**
             * See [`Pallet::remark_with_event`].
             **/
            remarkWithEvent: AugmentedSubmittable<
                (remark: Bytes | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Bytes]
            >;
            /**
             * See [`Pallet::set_code`].
             **/
            setCode: AugmentedSubmittable<
                (code: Bytes | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Bytes]
            >;
            /**
             * See [`Pallet::set_code_without_checks`].
             **/
            setCodeWithoutChecks: AugmentedSubmittable<
                (code: Bytes | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Bytes]
            >;
            /**
             * See [`Pallet::set_heap_pages`].
             **/
            setHeapPages: AugmentedSubmittable<
                (pages: u64 | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [u64]
            >;
            /**
             * See [`Pallet::set_storage`].
             **/
            setStorage: AugmentedSubmittable<
                (
                    items: Vec<ITuple<[Bytes, Bytes]>> | [Bytes | string | Uint8Array, Bytes | string | Uint8Array][],
                ) => SubmittableExtrinsic<ApiType>,
                [Vec<ITuple<[Bytes, Bytes]>>]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        timestamp: {
            /**
             * See [`Pallet::set`].
             **/
            set: AugmentedSubmittable<
                (now: Compact<u64> | AnyNumber | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [Compact<u64>]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        utility: {
            /**
             * See [`Pallet::as_derivative`].
             **/
            asDerivative: AugmentedSubmittable<
                (
                    index: u16 | AnyNumber | Uint8Array,
                    call: Call | IMethod | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [u16, Call]
            >;
            /**
             * See [`Pallet::batch`].
             **/
            batch: AugmentedSubmittable<
                (calls: Vec<Call> | (Call | IMethod | string | Uint8Array)[]) => SubmittableExtrinsic<ApiType>,
                [Vec<Call>]
            >;
            /**
             * See [`Pallet::batch_all`].
             **/
            batchAll: AugmentedSubmittable<
                (calls: Vec<Call> | (Call | IMethod | string | Uint8Array)[]) => SubmittableExtrinsic<ApiType>,
                [Vec<Call>]
            >;
            /**
             * See [`Pallet::dispatch_as`].
             **/
            dispatchAs: AugmentedSubmittable<
                (
                    asOrigin:
                        | Creditcoin3RuntimeOriginCaller
                        | { system: any }
                        | { Void: any }
                        | { Ethereum: any }
                        | string
                        | Uint8Array,
                    call: Call | IMethod | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Creditcoin3RuntimeOriginCaller, Call]
            >;
            /**
             * See [`Pallet::force_batch`].
             **/
            forceBatch: AugmentedSubmittable<
                (calls: Vec<Call> | (Call | IMethod | string | Uint8Array)[]) => SubmittableExtrinsic<ApiType>,
                [Vec<Call>]
            >;
            /**
             * See [`Pallet::with_weight`].
             **/
            withWeight: AugmentedSubmittable<
                (
                    call: Call | IMethod | string | Uint8Array,
                    weight: SpWeightsWeightV2Weight | { refTime?: any; proofSize?: any } | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [Call, SpWeightsWeightV2Weight]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
        voterList: {
            /**
             * See [`Pallet::put_in_front_of`].
             **/
            putInFrontOf: AugmentedSubmittable<
                (lighter: AccountId32 | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [AccountId32]
            >;
            /**
             * See [`Pallet::put_in_front_of_other`].
             **/
            putInFrontOfOther: AugmentedSubmittable<
                (
                    heavier: AccountId32 | string | Uint8Array,
                    lighter: AccountId32 | string | Uint8Array,
                ) => SubmittableExtrinsic<ApiType>,
                [AccountId32, AccountId32]
            >;
            /**
             * See [`Pallet::rebag`].
             **/
            rebag: AugmentedSubmittable<
                (dislocated: AccountId32 | string | Uint8Array) => SubmittableExtrinsic<ApiType>,
                [AccountId32]
            >;
            /**
             * Generic tx
             **/
            [key: string]: SubmittableExtrinsicFunction<ApiType>;
        };
    } // AugmentedSubmittables
} // declare module
