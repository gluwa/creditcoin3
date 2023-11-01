// Auto-generated via `yarn polkadot-types-from-defs`, do not edit
/* eslint-disable */

// import type lookup before we augment - in some environments
// this is required to allow for ambient/previous definitions
import '@polkadot/types/lookup'

import type { Data } from '@polkadot/types'
import type {
    BTreeMap,
    Bytes,
    Compact,
    Enum,
    Null,
    Option,
    Result,
    Set,
    Struct,
    Text,
    U256,
    U8aFixed,
    Vec,
    bool,
    u128,
    u16,
    u32,
    u64,
    u8,
} from '@polkadot/types-codec'
import type { ITuple } from '@polkadot/types-codec/types'
import type {
    AccountId20,
    Call,
    H160,
    H256,
    Perbill,
    Percent,
    Permill,
} from '@polkadot/types/interfaces/runtime'
import type { Event } from '@polkadot/types/interfaces/system'

declare module '@polkadot/types/lookup' {
    /** @name FrameSystemAccountInfo (3) */
    interface FrameSystemAccountInfo extends Struct {
        readonly nonce: u32
        readonly consumers: u32
        readonly providers: u32
        readonly sufficients: u32
        readonly data: PalletBalancesAccountData
    }

    /** @name PalletBalancesAccountData (5) */
    interface PalletBalancesAccountData extends Struct {
        readonly free: u128
        readonly reserved: u128
        readonly frozen: u128
        readonly flags: u128
    }

    /** @name FrameSupportDispatchPerDispatchClassWeight (8) */
    interface FrameSupportDispatchPerDispatchClassWeight extends Struct {
        readonly normal: SpWeightsWeightV2Weight
        readonly operational: SpWeightsWeightV2Weight
        readonly mandatory: SpWeightsWeightV2Weight
    }

    /** @name SpWeightsWeightV2Weight (9) */
    interface SpWeightsWeightV2Weight extends Struct {
        readonly refTime: Compact<u64>
        readonly proofSize: Compact<u64>
    }

    /** @name SpRuntimeDigest (15) */
    interface SpRuntimeDigest extends Struct {
        readonly logs: Vec<SpRuntimeDigestDigestItem>
    }

    /** @name SpRuntimeDigestDigestItem (17) */
    interface SpRuntimeDigestDigestItem extends Enum {
        readonly isOther: boolean
        readonly asOther: Bytes
        readonly isConsensus: boolean
        readonly asConsensus: ITuple<[U8aFixed, Bytes]>
        readonly isSeal: boolean
        readonly asSeal: ITuple<[U8aFixed, Bytes]>
        readonly isPreRuntime: boolean
        readonly asPreRuntime: ITuple<[U8aFixed, Bytes]>
        readonly isRuntimeEnvironmentUpdated: boolean
        readonly type:
            | 'Other'
            | 'Consensus'
            | 'Seal'
            | 'PreRuntime'
            | 'RuntimeEnvironmentUpdated'
    }

    /** @name FrameSystemEventRecord (20) */
    interface FrameSystemEventRecord extends Struct {
        readonly phase: FrameSystemPhase
        readonly event: Event
        readonly topics: Vec<H256>
    }

    /** @name FrameSystemEvent (22) */
    interface FrameSystemEvent extends Enum {
        readonly isExtrinsicSuccess: boolean
        readonly asExtrinsicSuccess: {
            readonly dispatchInfo: FrameSupportDispatchDispatchInfo
        } & Struct
        readonly isExtrinsicFailed: boolean
        readonly asExtrinsicFailed: {
            readonly dispatchError: SpRuntimeDispatchError
            readonly dispatchInfo: FrameSupportDispatchDispatchInfo
        } & Struct
        readonly isCodeUpdated: boolean
        readonly isNewAccount: boolean
        readonly asNewAccount: {
            readonly account: AccountId20
        } & Struct
        readonly isKilledAccount: boolean
        readonly asKilledAccount: {
            readonly account: AccountId20
        } & Struct
        readonly isRemarked: boolean
        readonly asRemarked: {
            readonly sender: AccountId20
            readonly hash_: H256
        } & Struct
        readonly type:
            | 'ExtrinsicSuccess'
            | 'ExtrinsicFailed'
            | 'CodeUpdated'
            | 'NewAccount'
            | 'KilledAccount'
            | 'Remarked'
    }

    /** @name FrameSupportDispatchDispatchInfo (23) */
    interface FrameSupportDispatchDispatchInfo extends Struct {
        readonly weight: SpWeightsWeightV2Weight
        readonly class: FrameSupportDispatchDispatchClass
        readonly paysFee: FrameSupportDispatchPays
    }

    /** @name FrameSupportDispatchDispatchClass (24) */
    interface FrameSupportDispatchDispatchClass extends Enum {
        readonly isNormal: boolean
        readonly isOperational: boolean
        readonly isMandatory: boolean
        readonly type: 'Normal' | 'Operational' | 'Mandatory'
    }

    /** @name FrameSupportDispatchPays (25) */
    interface FrameSupportDispatchPays extends Enum {
        readonly isYes: boolean
        readonly isNo: boolean
        readonly type: 'Yes' | 'No'
    }

    /** @name SpRuntimeDispatchError (26) */
    interface SpRuntimeDispatchError extends Enum {
        readonly isOther: boolean
        readonly isCannotLookup: boolean
        readonly isBadOrigin: boolean
        readonly isModule: boolean
        readonly asModule: SpRuntimeModuleError
        readonly isConsumerRemaining: boolean
        readonly isNoProviders: boolean
        readonly isTooManyConsumers: boolean
        readonly isToken: boolean
        readonly asToken: SpRuntimeTokenError
        readonly isArithmetic: boolean
        readonly asArithmetic: SpArithmeticArithmeticError
        readonly isTransactional: boolean
        readonly asTransactional: SpRuntimeTransactionalError
        readonly isExhausted: boolean
        readonly isCorruption: boolean
        readonly isUnavailable: boolean
        readonly isRootNotAllowed: boolean
        readonly type:
            | 'Other'
            | 'CannotLookup'
            | 'BadOrigin'
            | 'Module'
            | 'ConsumerRemaining'
            | 'NoProviders'
            | 'TooManyConsumers'
            | 'Token'
            | 'Arithmetic'
            | 'Transactional'
            | 'Exhausted'
            | 'Corruption'
            | 'Unavailable'
            | 'RootNotAllowed'
    }

    /** @name SpRuntimeModuleError (27) */
    interface SpRuntimeModuleError extends Struct {
        readonly index: u8
        readonly error: U8aFixed
    }

    /** @name SpRuntimeTokenError (28) */
    interface SpRuntimeTokenError extends Enum {
        readonly isFundsUnavailable: boolean
        readonly isOnlyProvider: boolean
        readonly isBelowMinimum: boolean
        readonly isCannotCreate: boolean
        readonly isUnknownAsset: boolean
        readonly isFrozen: boolean
        readonly isUnsupported: boolean
        readonly isCannotCreateHold: boolean
        readonly isNotExpendable: boolean
        readonly isBlocked: boolean
        readonly type:
            | 'FundsUnavailable'
            | 'OnlyProvider'
            | 'BelowMinimum'
            | 'CannotCreate'
            | 'UnknownAsset'
            | 'Frozen'
            | 'Unsupported'
            | 'CannotCreateHold'
            | 'NotExpendable'
            | 'Blocked'
    }

    /** @name SpArithmeticArithmeticError (29) */
    interface SpArithmeticArithmeticError extends Enum {
        readonly isUnderflow: boolean
        readonly isOverflow: boolean
        readonly isDivisionByZero: boolean
        readonly type: 'Underflow' | 'Overflow' | 'DivisionByZero'
    }

    /** @name SpRuntimeTransactionalError (30) */
    interface SpRuntimeTransactionalError extends Enum {
        readonly isLimitReached: boolean
        readonly isNoLayer: boolean
        readonly type: 'LimitReached' | 'NoLayer'
    }

    /** @name PalletBalancesEvent (31) */
    interface PalletBalancesEvent extends Enum {
        readonly isEndowed: boolean
        readonly asEndowed: {
            readonly account: AccountId20
            readonly freeBalance: u128
        } & Struct
        readonly isDustLost: boolean
        readonly asDustLost: {
            readonly account: AccountId20
            readonly amount: u128
        } & Struct
        readonly isTransfer: boolean
        readonly asTransfer: {
            readonly from: AccountId20
            readonly to: AccountId20
            readonly amount: u128
        } & Struct
        readonly isBalanceSet: boolean
        readonly asBalanceSet: {
            readonly who: AccountId20
            readonly free: u128
        } & Struct
        readonly isReserved: boolean
        readonly asReserved: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly isUnreserved: boolean
        readonly asUnreserved: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly isReserveRepatriated: boolean
        readonly asReserveRepatriated: {
            readonly from: AccountId20
            readonly to: AccountId20
            readonly amount: u128
            readonly destinationStatus: FrameSupportTokensMiscBalanceStatus
        } & Struct
        readonly isDeposit: boolean
        readonly asDeposit: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly isWithdraw: boolean
        readonly asWithdraw: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly isSlashed: boolean
        readonly asSlashed: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly isMinted: boolean
        readonly asMinted: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly isBurned: boolean
        readonly asBurned: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly isSuspended: boolean
        readonly asSuspended: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly isRestored: boolean
        readonly asRestored: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly isUpgraded: boolean
        readonly asUpgraded: {
            readonly who: AccountId20
        } & Struct
        readonly isIssued: boolean
        readonly asIssued: {
            readonly amount: u128
        } & Struct
        readonly isRescinded: boolean
        readonly asRescinded: {
            readonly amount: u128
        } & Struct
        readonly isLocked: boolean
        readonly asLocked: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly isUnlocked: boolean
        readonly asUnlocked: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly isFrozen: boolean
        readonly asFrozen: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly isThawed: boolean
        readonly asThawed: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly type:
            | 'Endowed'
            | 'DustLost'
            | 'Transfer'
            | 'BalanceSet'
            | 'Reserved'
            | 'Unreserved'
            | 'ReserveRepatriated'
            | 'Deposit'
            | 'Withdraw'
            | 'Slashed'
            | 'Minted'
            | 'Burned'
            | 'Suspended'
            | 'Restored'
            | 'Upgraded'
            | 'Issued'
            | 'Rescinded'
            | 'Locked'
            | 'Unlocked'
            | 'Frozen'
            | 'Thawed'
    }

    /** @name FrameSupportTokensMiscBalanceStatus (32) */
    interface FrameSupportTokensMiscBalanceStatus extends Enum {
        readonly isFree: boolean
        readonly isReserved: boolean
        readonly type: 'Free' | 'Reserved'
    }

    /** @name PalletStakingPalletEvent (33) */
    interface PalletStakingPalletEvent extends Enum {
        readonly isEraPaid: boolean
        readonly asEraPaid: {
            readonly eraIndex: u32
            readonly validatorPayout: u128
            readonly remainder: u128
        } & Struct
        readonly isRewarded: boolean
        readonly asRewarded: {
            readonly stash: AccountId20
            readonly amount: u128
        } & Struct
        readonly isSlashed: boolean
        readonly asSlashed: {
            readonly staker: AccountId20
            readonly amount: u128
        } & Struct
        readonly isSlashReported: boolean
        readonly asSlashReported: {
            readonly validator: AccountId20
            readonly fraction: Perbill
            readonly slashEra: u32
        } & Struct
        readonly isOldSlashingReportDiscarded: boolean
        readonly asOldSlashingReportDiscarded: {
            readonly sessionIndex: u32
        } & Struct
        readonly isStakersElected: boolean
        readonly isBonded: boolean
        readonly asBonded: {
            readonly stash: AccountId20
            readonly amount: u128
        } & Struct
        readonly isUnbonded: boolean
        readonly asUnbonded: {
            readonly stash: AccountId20
            readonly amount: u128
        } & Struct
        readonly isWithdrawn: boolean
        readonly asWithdrawn: {
            readonly stash: AccountId20
            readonly amount: u128
        } & Struct
        readonly isKicked: boolean
        readonly asKicked: {
            readonly nominator: AccountId20
            readonly stash: AccountId20
        } & Struct
        readonly isStakingElectionFailed: boolean
        readonly isChilled: boolean
        readonly asChilled: {
            readonly stash: AccountId20
        } & Struct
        readonly isPayoutStarted: boolean
        readonly asPayoutStarted: {
            readonly eraIndex: u32
            readonly validatorStash: AccountId20
        } & Struct
        readonly isValidatorPrefsSet: boolean
        readonly asValidatorPrefsSet: {
            readonly stash: AccountId20
            readonly prefs: PalletStakingValidatorPrefs
        } & Struct
        readonly isSnapshotVotersSizeExceeded: boolean
        readonly asSnapshotVotersSizeExceeded: {
            readonly size_: u32
        } & Struct
        readonly isSnapshotTargetsSizeExceeded: boolean
        readonly asSnapshotTargetsSizeExceeded: {
            readonly size_: u32
        } & Struct
        readonly isForceEra: boolean
        readonly asForceEra: {
            readonly mode: PalletStakingForcing
        } & Struct
        readonly type:
            | 'EraPaid'
            | 'Rewarded'
            | 'Slashed'
            | 'SlashReported'
            | 'OldSlashingReportDiscarded'
            | 'StakersElected'
            | 'Bonded'
            | 'Unbonded'
            | 'Withdrawn'
            | 'Kicked'
            | 'StakingElectionFailed'
            | 'Chilled'
            | 'PayoutStarted'
            | 'ValidatorPrefsSet'
            | 'SnapshotVotersSizeExceeded'
            | 'SnapshotTargetsSizeExceeded'
            | 'ForceEra'
    }

    /** @name PalletStakingValidatorPrefs (35) */
    interface PalletStakingValidatorPrefs extends Struct {
        readonly commission: Compact<Perbill>
        readonly blocked: bool
    }

    /** @name PalletStakingForcing (38) */
    interface PalletStakingForcing extends Enum {
        readonly isNotForcing: boolean
        readonly isForceNew: boolean
        readonly isForceNone: boolean
        readonly isForceAlways: boolean
        readonly type: 'NotForcing' | 'ForceNew' | 'ForceNone' | 'ForceAlways'
    }

    /** @name PalletOffencesEvent (39) */
    interface PalletOffencesEvent extends Enum {
        readonly isOffence: boolean
        readonly asOffence: {
            readonly kind: U8aFixed
            readonly timeslot: Bytes
        } & Struct
        readonly type: 'Offence'
    }

    /** @name PalletSessionEvent (41) */
    interface PalletSessionEvent extends Enum {
        readonly isNewSession: boolean
        readonly asNewSession: {
            readonly sessionIndex: u32
        } & Struct
        readonly type: 'NewSession'
    }

    /** @name PalletGrandpaEvent (42) */
    interface PalletGrandpaEvent extends Enum {
        readonly isNewAuthorities: boolean
        readonly asNewAuthorities: {
            readonly authoritySet: Vec<
                ITuple<[SpConsensusGrandpaAppPublic, u64]>
            >
        } & Struct
        readonly isPaused: boolean
        readonly isResumed: boolean
        readonly type: 'NewAuthorities' | 'Paused' | 'Resumed'
    }

    /** @name SpConsensusGrandpaAppPublic (45) */
    interface SpConsensusGrandpaAppPublic extends SpCoreEd25519Public {}

    /** @name SpCoreEd25519Public (46) */
    interface SpCoreEd25519Public extends U8aFixed {}

    /** @name PalletImOnlineEvent (47) */
    interface PalletImOnlineEvent extends Enum {
        readonly isHeartbeatReceived: boolean
        readonly asHeartbeatReceived: {
            readonly authorityId: PalletImOnlineSr25519AppSr25519Public
        } & Struct
        readonly isAllGood: boolean
        readonly isSomeOffline: boolean
        readonly asSomeOffline: {
            readonly offline: Vec<ITuple<[AccountId20, PalletStakingExposure]>>
        } & Struct
        readonly type: 'HeartbeatReceived' | 'AllGood' | 'SomeOffline'
    }

    /** @name PalletImOnlineSr25519AppSr25519Public (48) */
    interface PalletImOnlineSr25519AppSr25519Public
        extends SpCoreSr25519Public {}

    /** @name SpCoreSr25519Public (49) */
    interface SpCoreSr25519Public extends U8aFixed {}

    /** @name PalletStakingExposure (52) */
    interface PalletStakingExposure extends Struct {
        readonly total: Compact<u128>
        readonly own: Compact<u128>
        readonly others: Vec<PalletStakingIndividualExposure>
    }

    /** @name PalletStakingIndividualExposure (55) */
    interface PalletStakingIndividualExposure extends Struct {
        readonly who: AccountId20
        readonly value: Compact<u128>
    }

    /** @name PalletBagsListEvent (56) */
    interface PalletBagsListEvent extends Enum {
        readonly isRebagged: boolean
        readonly asRebagged: {
            readonly who: AccountId20
            readonly from: u64
            readonly to: u64
        } & Struct
        readonly isScoreUpdated: boolean
        readonly asScoreUpdated: {
            readonly who: AccountId20
            readonly newScore: u64
        } & Struct
        readonly type: 'Rebagged' | 'ScoreUpdated'
    }

    /** @name PalletTransactionPaymentEvent (57) */
    interface PalletTransactionPaymentEvent extends Enum {
        readonly isTransactionFeePaid: boolean
        readonly asTransactionFeePaid: {
            readonly who: AccountId20
            readonly actualFee: u128
            readonly tip: u128
        } & Struct
        readonly type: 'TransactionFeePaid'
    }

    /** @name PalletSudoEvent (58) */
    interface PalletSudoEvent extends Enum {
        readonly isSudid: boolean
        readonly asSudid: {
            readonly sudoResult: Result<Null, SpRuntimeDispatchError>
        } & Struct
        readonly isKeyChanged: boolean
        readonly asKeyChanged: {
            readonly oldSudoer: Option<AccountId20>
        } & Struct
        readonly isSudoAsDone: boolean
        readonly asSudoAsDone: {
            readonly sudoResult: Result<Null, SpRuntimeDispatchError>
        } & Struct
        readonly type: 'Sudid' | 'KeyChanged' | 'SudoAsDone'
    }

    /** @name PalletUtilityEvent (62) */
    interface PalletUtilityEvent extends Enum {
        readonly isBatchInterrupted: boolean
        readonly asBatchInterrupted: {
            readonly index: u32
            readonly error: SpRuntimeDispatchError
        } & Struct
        readonly isBatchCompleted: boolean
        readonly isBatchCompletedWithErrors: boolean
        readonly isItemCompleted: boolean
        readonly isItemFailed: boolean
        readonly asItemFailed: {
            readonly error: SpRuntimeDispatchError
        } & Struct
        readonly isDispatchedAs: boolean
        readonly asDispatchedAs: {
            readonly result: Result<Null, SpRuntimeDispatchError>
        } & Struct
        readonly type:
            | 'BatchInterrupted'
            | 'BatchCompleted'
            | 'BatchCompletedWithErrors'
            | 'ItemCompleted'
            | 'ItemFailed'
            | 'DispatchedAs'
    }

    /** @name PalletProxyEvent (63) */
    interface PalletProxyEvent extends Enum {
        readonly isProxyExecuted: boolean
        readonly asProxyExecuted: {
            readonly result: Result<Null, SpRuntimeDispatchError>
        } & Struct
        readonly isPureCreated: boolean
        readonly asPureCreated: {
            readonly pure: AccountId20
            readonly who: AccountId20
            readonly proxyType: FrontierTemplateRuntimeProxyFilter
            readonly disambiguationIndex: u16
        } & Struct
        readonly isAnnounced: boolean
        readonly asAnnounced: {
            readonly real: AccountId20
            readonly proxy: AccountId20
            readonly callHash: H256
        } & Struct
        readonly isProxyAdded: boolean
        readonly asProxyAdded: {
            readonly delegator: AccountId20
            readonly delegatee: AccountId20
            readonly proxyType: FrontierTemplateRuntimeProxyFilter
            readonly delay: u32
        } & Struct
        readonly isProxyRemoved: boolean
        readonly asProxyRemoved: {
            readonly delegator: AccountId20
            readonly delegatee: AccountId20
            readonly proxyType: FrontierTemplateRuntimeProxyFilter
            readonly delay: u32
        } & Struct
        readonly type:
            | 'ProxyExecuted'
            | 'PureCreated'
            | 'Announced'
            | 'ProxyAdded'
            | 'ProxyRemoved'
    }

    /** @name FrontierTemplateRuntimeProxyFilter (64) */
    interface FrontierTemplateRuntimeProxyFilter extends Enum {
        readonly isAll: boolean
        readonly isNonTransfer: boolean
        readonly isStaking: boolean
        readonly type: 'All' | 'NonTransfer' | 'Staking'
    }

    /** @name PalletIdentityEvent (66) */
    interface PalletIdentityEvent extends Enum {
        readonly isIdentitySet: boolean
        readonly asIdentitySet: {
            readonly who: AccountId20
        } & Struct
        readonly isIdentityCleared: boolean
        readonly asIdentityCleared: {
            readonly who: AccountId20
            readonly deposit: u128
        } & Struct
        readonly isIdentityKilled: boolean
        readonly asIdentityKilled: {
            readonly who: AccountId20
            readonly deposit: u128
        } & Struct
        readonly isJudgementRequested: boolean
        readonly asJudgementRequested: {
            readonly who: AccountId20
            readonly registrarIndex: u32
        } & Struct
        readonly isJudgementUnrequested: boolean
        readonly asJudgementUnrequested: {
            readonly who: AccountId20
            readonly registrarIndex: u32
        } & Struct
        readonly isJudgementGiven: boolean
        readonly asJudgementGiven: {
            readonly target: AccountId20
            readonly registrarIndex: u32
        } & Struct
        readonly isRegistrarAdded: boolean
        readonly asRegistrarAdded: {
            readonly registrarIndex: u32
        } & Struct
        readonly isSubIdentityAdded: boolean
        readonly asSubIdentityAdded: {
            readonly sub: AccountId20
            readonly main: AccountId20
            readonly deposit: u128
        } & Struct
        readonly isSubIdentityRemoved: boolean
        readonly asSubIdentityRemoved: {
            readonly sub: AccountId20
            readonly main: AccountId20
            readonly deposit: u128
        } & Struct
        readonly isSubIdentityRevoked: boolean
        readonly asSubIdentityRevoked: {
            readonly sub: AccountId20
            readonly main: AccountId20
            readonly deposit: u128
        } & Struct
        readonly type:
            | 'IdentitySet'
            | 'IdentityCleared'
            | 'IdentityKilled'
            | 'JudgementRequested'
            | 'JudgementUnrequested'
            | 'JudgementGiven'
            | 'RegistrarAdded'
            | 'SubIdentityAdded'
            | 'SubIdentityRemoved'
            | 'SubIdentityRevoked'
    }

    /** @name PalletFastUnstakeEvent (67) */
    interface PalletFastUnstakeEvent extends Enum {
        readonly isUnstaked: boolean
        readonly asUnstaked: {
            readonly stash: AccountId20
            readonly result: Result<Null, SpRuntimeDispatchError>
        } & Struct
        readonly isSlashed: boolean
        readonly asSlashed: {
            readonly stash: AccountId20
            readonly amount: u128
        } & Struct
        readonly isBatchChecked: boolean
        readonly asBatchChecked: {
            readonly eras: Vec<u32>
        } & Struct
        readonly isBatchFinished: boolean
        readonly asBatchFinished: {
            readonly size_: u32
        } & Struct
        readonly isInternalError: boolean
        readonly type:
            | 'Unstaked'
            | 'Slashed'
            | 'BatchChecked'
            | 'BatchFinished'
            | 'InternalError'
    }

    /** @name PalletNominationPoolsEvent (69) */
    interface PalletNominationPoolsEvent extends Enum {
        readonly isCreated: boolean
        readonly asCreated: {
            readonly depositor: AccountId20
            readonly poolId: u32
        } & Struct
        readonly isBonded: boolean
        readonly asBonded: {
            readonly member: AccountId20
            readonly poolId: u32
            readonly bonded: u128
            readonly joined: bool
        } & Struct
        readonly isPaidOut: boolean
        readonly asPaidOut: {
            readonly member: AccountId20
            readonly poolId: u32
            readonly payout: u128
        } & Struct
        readonly isUnbonded: boolean
        readonly asUnbonded: {
            readonly member: AccountId20
            readonly poolId: u32
            readonly balance: u128
            readonly points: u128
            readonly era: u32
        } & Struct
        readonly isWithdrawn: boolean
        readonly asWithdrawn: {
            readonly member: AccountId20
            readonly poolId: u32
            readonly balance: u128
            readonly points: u128
        } & Struct
        readonly isDestroyed: boolean
        readonly asDestroyed: {
            readonly poolId: u32
        } & Struct
        readonly isStateChanged: boolean
        readonly asStateChanged: {
            readonly poolId: u32
            readonly newState: PalletNominationPoolsPoolState
        } & Struct
        readonly isMemberRemoved: boolean
        readonly asMemberRemoved: {
            readonly poolId: u32
            readonly member: AccountId20
        } & Struct
        readonly isRolesUpdated: boolean
        readonly asRolesUpdated: {
            readonly root: Option<AccountId20>
            readonly bouncer: Option<AccountId20>
            readonly nominator: Option<AccountId20>
        } & Struct
        readonly isPoolSlashed: boolean
        readonly asPoolSlashed: {
            readonly poolId: u32
            readonly balance: u128
        } & Struct
        readonly isUnbondingPoolSlashed: boolean
        readonly asUnbondingPoolSlashed: {
            readonly poolId: u32
            readonly era: u32
            readonly balance: u128
        } & Struct
        readonly isPoolCommissionUpdated: boolean
        readonly asPoolCommissionUpdated: {
            readonly poolId: u32
            readonly current: Option<ITuple<[Perbill, AccountId20]>>
        } & Struct
        readonly isPoolMaxCommissionUpdated: boolean
        readonly asPoolMaxCommissionUpdated: {
            readonly poolId: u32
            readonly maxCommission: Perbill
        } & Struct
        readonly isPoolCommissionChangeRateUpdated: boolean
        readonly asPoolCommissionChangeRateUpdated: {
            readonly poolId: u32
            readonly changeRate: PalletNominationPoolsCommissionChangeRate
        } & Struct
        readonly isPoolCommissionClaimed: boolean
        readonly asPoolCommissionClaimed: {
            readonly poolId: u32
            readonly commission: u128
        } & Struct
        readonly type:
            | 'Created'
            | 'Bonded'
            | 'PaidOut'
            | 'Unbonded'
            | 'Withdrawn'
            | 'Destroyed'
            | 'StateChanged'
            | 'MemberRemoved'
            | 'RolesUpdated'
            | 'PoolSlashed'
            | 'UnbondingPoolSlashed'
            | 'PoolCommissionUpdated'
            | 'PoolMaxCommissionUpdated'
            | 'PoolCommissionChangeRateUpdated'
            | 'PoolCommissionClaimed'
    }

    /** @name PalletNominationPoolsPoolState (70) */
    interface PalletNominationPoolsPoolState extends Enum {
        readonly isOpen: boolean
        readonly isBlocked: boolean
        readonly isDestroying: boolean
        readonly type: 'Open' | 'Blocked' | 'Destroying'
    }

    /** @name PalletNominationPoolsCommissionChangeRate (73) */
    interface PalletNominationPoolsCommissionChangeRate extends Struct {
        readonly maxIncrease: Perbill
        readonly minDelay: u32
    }

    /** @name PalletEthereumEvent (74) */
    interface PalletEthereumEvent extends Enum {
        readonly isExecuted: boolean
        readonly asExecuted: {
            readonly from: H160
            readonly to: H160
            readonly transactionHash: H256
            readonly exitReason: EvmCoreErrorExitReason
            readonly extraData: Bytes
        } & Struct
        readonly type: 'Executed'
    }

    /** @name EvmCoreErrorExitReason (76) */
    interface EvmCoreErrorExitReason extends Enum {
        readonly isSucceed: boolean
        readonly asSucceed: EvmCoreErrorExitSucceed
        readonly isError: boolean
        readonly asError: EvmCoreErrorExitError
        readonly isRevert: boolean
        readonly asRevert: EvmCoreErrorExitRevert
        readonly isFatal: boolean
        readonly asFatal: EvmCoreErrorExitFatal
        readonly type: 'Succeed' | 'Error' | 'Revert' | 'Fatal'
    }

    /** @name EvmCoreErrorExitSucceed (77) */
    interface EvmCoreErrorExitSucceed extends Enum {
        readonly isStopped: boolean
        readonly isReturned: boolean
        readonly isSuicided: boolean
        readonly type: 'Stopped' | 'Returned' | 'Suicided'
    }

    /** @name EvmCoreErrorExitError (78) */
    interface EvmCoreErrorExitError extends Enum {
        readonly isStackUnderflow: boolean
        readonly isStackOverflow: boolean
        readonly isInvalidJump: boolean
        readonly isInvalidRange: boolean
        readonly isDesignatedInvalid: boolean
        readonly isCallTooDeep: boolean
        readonly isCreateCollision: boolean
        readonly isCreateContractLimit: boolean
        readonly isOutOfOffset: boolean
        readonly isOutOfGas: boolean
        readonly isOutOfFund: boolean
        readonly isPcUnderflow: boolean
        readonly isCreateEmpty: boolean
        readonly isOther: boolean
        readonly asOther: Text
        readonly isMaxNonce: boolean
        readonly isInvalidCode: boolean
        readonly asInvalidCode: u8
        readonly type:
            | 'StackUnderflow'
            | 'StackOverflow'
            | 'InvalidJump'
            | 'InvalidRange'
            | 'DesignatedInvalid'
            | 'CallTooDeep'
            | 'CreateCollision'
            | 'CreateContractLimit'
            | 'OutOfOffset'
            | 'OutOfGas'
            | 'OutOfFund'
            | 'PcUnderflow'
            | 'CreateEmpty'
            | 'Other'
            | 'MaxNonce'
            | 'InvalidCode'
    }

    /** @name EvmCoreErrorExitRevert (82) */
    interface EvmCoreErrorExitRevert extends Enum {
        readonly isReverted: boolean
        readonly type: 'Reverted'
    }

    /** @name EvmCoreErrorExitFatal (83) */
    interface EvmCoreErrorExitFatal extends Enum {
        readonly isNotSupported: boolean
        readonly isUnhandledInterrupt: boolean
        readonly isCallErrorAsFatal: boolean
        readonly asCallErrorAsFatal: EvmCoreErrorExitError
        readonly isOther: boolean
        readonly asOther: Text
        readonly type:
            | 'NotSupported'
            | 'UnhandledInterrupt'
            | 'CallErrorAsFatal'
            | 'Other'
    }

    /** @name PalletEvmEvent (84) */
    interface PalletEvmEvent extends Enum {
        readonly isLog: boolean
        readonly asLog: {
            readonly log: EthereumLog
        } & Struct
        readonly isCreated: boolean
        readonly asCreated: {
            readonly address: H160
        } & Struct
        readonly isCreatedFailed: boolean
        readonly asCreatedFailed: {
            readonly address: H160
        } & Struct
        readonly isExecuted: boolean
        readonly asExecuted: {
            readonly address: H160
        } & Struct
        readonly isExecutedFailed: boolean
        readonly asExecutedFailed: {
            readonly address: H160
        } & Struct
        readonly type:
            | 'Log'
            | 'Created'
            | 'CreatedFailed'
            | 'Executed'
            | 'ExecutedFailed'
    }

    /** @name EthereumLog (85) */
    interface EthereumLog extends Struct {
        readonly address: H160
        readonly topics: Vec<H256>
        readonly data: Bytes
    }

    /** @name PalletBaseFeeEvent (87) */
    interface PalletBaseFeeEvent extends Enum {
        readonly isNewBaseFeePerGas: boolean
        readonly asNewBaseFeePerGas: {
            readonly fee: U256
        } & Struct
        readonly isBaseFeeOverflow: boolean
        readonly isNewElasticity: boolean
        readonly asNewElasticity: {
            readonly elasticity: Permill
        } & Struct
        readonly type: 'NewBaseFeePerGas' | 'BaseFeeOverflow' | 'NewElasticity'
    }

    /** @name FrameSystemPhase (91) */
    interface FrameSystemPhase extends Enum {
        readonly isApplyExtrinsic: boolean
        readonly asApplyExtrinsic: u32
        readonly isFinalization: boolean
        readonly isInitialization: boolean
        readonly type: 'ApplyExtrinsic' | 'Finalization' | 'Initialization'
    }

    /** @name FrameSystemLastRuntimeUpgradeInfo (94) */
    interface FrameSystemLastRuntimeUpgradeInfo extends Struct {
        readonly specVersion: Compact<u32>
        readonly specName: Text
    }

    /** @name FrameSystemCall (96) */
    interface FrameSystemCall extends Enum {
        readonly isRemark: boolean
        readonly asRemark: {
            readonly remark: Bytes
        } & Struct
        readonly isSetHeapPages: boolean
        readonly asSetHeapPages: {
            readonly pages: u64
        } & Struct
        readonly isSetCode: boolean
        readonly asSetCode: {
            readonly code: Bytes
        } & Struct
        readonly isSetCodeWithoutChecks: boolean
        readonly asSetCodeWithoutChecks: {
            readonly code: Bytes
        } & Struct
        readonly isSetStorage: boolean
        readonly asSetStorage: {
            readonly items: Vec<ITuple<[Bytes, Bytes]>>
        } & Struct
        readonly isKillStorage: boolean
        readonly asKillStorage: {
            readonly keys_: Vec<Bytes>
        } & Struct
        readonly isKillPrefix: boolean
        readonly asKillPrefix: {
            readonly prefix: Bytes
            readonly subkeys: u32
        } & Struct
        readonly isRemarkWithEvent: boolean
        readonly asRemarkWithEvent: {
            readonly remark: Bytes
        } & Struct
        readonly type:
            | 'Remark'
            | 'SetHeapPages'
            | 'SetCode'
            | 'SetCodeWithoutChecks'
            | 'SetStorage'
            | 'KillStorage'
            | 'KillPrefix'
            | 'RemarkWithEvent'
    }

    /** @name FrameSystemLimitsBlockWeights (100) */
    interface FrameSystemLimitsBlockWeights extends Struct {
        readonly baseBlock: SpWeightsWeightV2Weight
        readonly maxBlock: SpWeightsWeightV2Weight
        readonly perClass: FrameSupportDispatchPerDispatchClassWeightsPerClass
    }

    /** @name FrameSupportDispatchPerDispatchClassWeightsPerClass (101) */
    interface FrameSupportDispatchPerDispatchClassWeightsPerClass
        extends Struct {
        readonly normal: FrameSystemLimitsWeightsPerClass
        readonly operational: FrameSystemLimitsWeightsPerClass
        readonly mandatory: FrameSystemLimitsWeightsPerClass
    }

    /** @name FrameSystemLimitsWeightsPerClass (102) */
    interface FrameSystemLimitsWeightsPerClass extends Struct {
        readonly baseExtrinsic: SpWeightsWeightV2Weight
        readonly maxExtrinsic: Option<SpWeightsWeightV2Weight>
        readonly maxTotal: Option<SpWeightsWeightV2Weight>
        readonly reserved: Option<SpWeightsWeightV2Weight>
    }

    /** @name FrameSystemLimitsBlockLength (104) */
    interface FrameSystemLimitsBlockLength extends Struct {
        readonly max: FrameSupportDispatchPerDispatchClassU32
    }

    /** @name FrameSupportDispatchPerDispatchClassU32 (105) */
    interface FrameSupportDispatchPerDispatchClassU32 extends Struct {
        readonly normal: u32
        readonly operational: u32
        readonly mandatory: u32
    }

    /** @name SpWeightsRuntimeDbWeight (106) */
    interface SpWeightsRuntimeDbWeight extends Struct {
        readonly read: u64
        readonly write: u64
    }

    /** @name SpVersionRuntimeVersion (107) */
    interface SpVersionRuntimeVersion extends Struct {
        readonly specName: Text
        readonly implName: Text
        readonly authoringVersion: u32
        readonly specVersion: u32
        readonly implVersion: u32
        readonly apis: Vec<ITuple<[U8aFixed, u32]>>
        readonly transactionVersion: u32
        readonly stateVersion: u8
    }

    /** @name FrameSystemError (112) */
    interface FrameSystemError extends Enum {
        readonly isInvalidSpecName: boolean
        readonly isSpecVersionNeedsToIncrease: boolean
        readonly isFailedToExtractRuntimeVersion: boolean
        readonly isNonDefaultComposite: boolean
        readonly isNonZeroRefCount: boolean
        readonly isCallFiltered: boolean
        readonly type:
            | 'InvalidSpecName'
            | 'SpecVersionNeedsToIncrease'
            | 'FailedToExtractRuntimeVersion'
            | 'NonDefaultComposite'
            | 'NonZeroRefCount'
            | 'CallFiltered'
    }

    /** @name SpConsensusBabeAppPublic (115) */
    interface SpConsensusBabeAppPublic extends SpCoreSr25519Public {}

    /** @name SpConsensusBabeDigestsNextConfigDescriptor (118) */
    interface SpConsensusBabeDigestsNextConfigDescriptor extends Enum {
        readonly isV1: boolean
        readonly asV1: {
            readonly c: ITuple<[u64, u64]>
            readonly allowedSlots: SpConsensusBabeAllowedSlots
        } & Struct
        readonly type: 'V1'
    }

    /** @name SpConsensusBabeAllowedSlots (120) */
    interface SpConsensusBabeAllowedSlots extends Enum {
        readonly isPrimarySlots: boolean
        readonly isPrimaryAndSecondaryPlainSlots: boolean
        readonly isPrimaryAndSecondaryVRFSlots: boolean
        readonly type:
            | 'PrimarySlots'
            | 'PrimaryAndSecondaryPlainSlots'
            | 'PrimaryAndSecondaryVRFSlots'
    }

    /** @name SpConsensusBabeDigestsPreDigest (124) */
    interface SpConsensusBabeDigestsPreDigest extends Enum {
        readonly isPrimary: boolean
        readonly asPrimary: SpConsensusBabeDigestsPrimaryPreDigest
        readonly isSecondaryPlain: boolean
        readonly asSecondaryPlain: SpConsensusBabeDigestsSecondaryPlainPreDigest
        readonly isSecondaryVRF: boolean
        readonly asSecondaryVRF: SpConsensusBabeDigestsSecondaryVRFPreDigest
        readonly type: 'Primary' | 'SecondaryPlain' | 'SecondaryVRF'
    }

    /** @name SpConsensusBabeDigestsPrimaryPreDigest (125) */
    interface SpConsensusBabeDigestsPrimaryPreDigest extends Struct {
        readonly authorityIndex: u32
        readonly slot: u64
        readonly vrfSignature: SpCoreSr25519VrfVrfSignature
    }

    /** @name SpCoreSr25519VrfVrfSignature (126) */
    interface SpCoreSr25519VrfVrfSignature extends Struct {
        readonly output: U8aFixed
        readonly proof: U8aFixed
    }

    /** @name SpConsensusBabeDigestsSecondaryPlainPreDigest (128) */
    interface SpConsensusBabeDigestsSecondaryPlainPreDigest extends Struct {
        readonly authorityIndex: u32
        readonly slot: u64
    }

    /** @name SpConsensusBabeDigestsSecondaryVRFPreDigest (129) */
    interface SpConsensusBabeDigestsSecondaryVRFPreDigest extends Struct {
        readonly authorityIndex: u32
        readonly slot: u64
        readonly vrfSignature: SpCoreSr25519VrfVrfSignature
    }

    /** @name SpConsensusBabeBabeEpochConfiguration (131) */
    interface SpConsensusBabeBabeEpochConfiguration extends Struct {
        readonly c: ITuple<[u64, u64]>
        readonly allowedSlots: SpConsensusBabeAllowedSlots
    }

    /** @name PalletBabeCall (135) */
    interface PalletBabeCall extends Enum {
        readonly isReportEquivocation: boolean
        readonly asReportEquivocation: {
            readonly equivocationProof: SpConsensusSlotsEquivocationProof
            readonly keyOwnerProof: SpSessionMembershipProof
        } & Struct
        readonly isReportEquivocationUnsigned: boolean
        readonly asReportEquivocationUnsigned: {
            readonly equivocationProof: SpConsensusSlotsEquivocationProof
            readonly keyOwnerProof: SpSessionMembershipProof
        } & Struct
        readonly isPlanConfigChange: boolean
        readonly asPlanConfigChange: {
            readonly config: SpConsensusBabeDigestsNextConfigDescriptor
        } & Struct
        readonly type:
            | 'ReportEquivocation'
            | 'ReportEquivocationUnsigned'
            | 'PlanConfigChange'
    }

    /** @name SpConsensusSlotsEquivocationProof (136) */
    interface SpConsensusSlotsEquivocationProof extends Struct {
        readonly offender: SpConsensusBabeAppPublic
        readonly slot: u64
        readonly firstHeader: SpRuntimeHeader
        readonly secondHeader: SpRuntimeHeader
    }

    /** @name SpRuntimeHeader (137) */
    interface SpRuntimeHeader extends Struct {
        readonly parentHash: H256
        readonly number: Compact<u32>
        readonly stateRoot: H256
        readonly extrinsicsRoot: H256
        readonly digest: SpRuntimeDigest
    }

    /** @name SpSessionMembershipProof (138) */
    interface SpSessionMembershipProof extends Struct {
        readonly session: u32
        readonly trieNodes: Vec<Bytes>
        readonly validatorCount: u32
    }

    /** @name PalletBabeError (139) */
    interface PalletBabeError extends Enum {
        readonly isInvalidEquivocationProof: boolean
        readonly isInvalidKeyOwnershipProof: boolean
        readonly isDuplicateOffenceReport: boolean
        readonly isInvalidConfiguration: boolean
        readonly type:
            | 'InvalidEquivocationProof'
            | 'InvalidKeyOwnershipProof'
            | 'DuplicateOffenceReport'
            | 'InvalidConfiguration'
    }

    /** @name PalletTimestampCall (140) */
    interface PalletTimestampCall extends Enum {
        readonly isSet: boolean
        readonly asSet: {
            readonly now: Compact<u64>
        } & Struct
        readonly type: 'Set'
    }

    /** @name PalletBalancesBalanceLock (142) */
    interface PalletBalancesBalanceLock extends Struct {
        readonly id: U8aFixed
        readonly amount: u128
        readonly reasons: PalletBalancesReasons
    }

    /** @name PalletBalancesReasons (143) */
    interface PalletBalancesReasons extends Enum {
        readonly isFee: boolean
        readonly isMisc: boolean
        readonly isAll: boolean
        readonly type: 'Fee' | 'Misc' | 'All'
    }

    /** @name PalletBalancesReserveData (146) */
    interface PalletBalancesReserveData extends Struct {
        readonly id: U8aFixed
        readonly amount: u128
    }

    /** @name PalletBalancesIdAmount (149) */
    interface PalletBalancesIdAmount extends Struct {
        readonly id: Null
        readonly amount: u128
    }

    /** @name PalletBalancesCall (151) */
    interface PalletBalancesCall extends Enum {
        readonly isTransferAllowDeath: boolean
        readonly asTransferAllowDeath: {
            readonly dest: AccountId20
            readonly value: Compact<u128>
        } & Struct
        readonly isSetBalanceDeprecated: boolean
        readonly asSetBalanceDeprecated: {
            readonly who: AccountId20
            readonly newFree: Compact<u128>
            readonly oldReserved: Compact<u128>
        } & Struct
        readonly isForceTransfer: boolean
        readonly asForceTransfer: {
            readonly source: AccountId20
            readonly dest: AccountId20
            readonly value: Compact<u128>
        } & Struct
        readonly isTransferKeepAlive: boolean
        readonly asTransferKeepAlive: {
            readonly dest: AccountId20
            readonly value: Compact<u128>
        } & Struct
        readonly isTransferAll: boolean
        readonly asTransferAll: {
            readonly dest: AccountId20
            readonly keepAlive: bool
        } & Struct
        readonly isForceUnreserve: boolean
        readonly asForceUnreserve: {
            readonly who: AccountId20
            readonly amount: u128
        } & Struct
        readonly isUpgradeAccounts: boolean
        readonly asUpgradeAccounts: {
            readonly who: Vec<AccountId20>
        } & Struct
        readonly isTransfer: boolean
        readonly asTransfer: {
            readonly dest: AccountId20
            readonly value: Compact<u128>
        } & Struct
        readonly isForceSetBalance: boolean
        readonly asForceSetBalance: {
            readonly who: AccountId20
            readonly newFree: Compact<u128>
        } & Struct
        readonly type:
            | 'TransferAllowDeath'
            | 'SetBalanceDeprecated'
            | 'ForceTransfer'
            | 'TransferKeepAlive'
            | 'TransferAll'
            | 'ForceUnreserve'
            | 'UpgradeAccounts'
            | 'Transfer'
            | 'ForceSetBalance'
    }

    /** @name PalletBalancesError (153) */
    interface PalletBalancesError extends Enum {
        readonly isVestingBalance: boolean
        readonly isLiquidityRestrictions: boolean
        readonly isInsufficientBalance: boolean
        readonly isExistentialDeposit: boolean
        readonly isExpendability: boolean
        readonly isExistingVestingSchedule: boolean
        readonly isDeadAccount: boolean
        readonly isTooManyReserves: boolean
        readonly isTooManyHolds: boolean
        readonly isTooManyFreezes: boolean
        readonly type:
            | 'VestingBalance'
            | 'LiquidityRestrictions'
            | 'InsufficientBalance'
            | 'ExistentialDeposit'
            | 'Expendability'
            | 'ExistingVestingSchedule'
            | 'DeadAccount'
            | 'TooManyReserves'
            | 'TooManyHolds'
            | 'TooManyFreezes'
    }

    /** @name PalletStakingStakingLedger (154) */
    interface PalletStakingStakingLedger extends Struct {
        readonly stash: AccountId20
        readonly total: Compact<u128>
        readonly active: Compact<u128>
        readonly unlocking: Vec<PalletStakingUnlockChunk>
        readonly claimedRewards: Vec<u32>
    }

    /** @name PalletStakingUnlockChunk (156) */
    interface PalletStakingUnlockChunk extends Struct {
        readonly value: Compact<u128>
        readonly era: Compact<u32>
    }

    /** @name PalletStakingRewardDestination (159) */
    interface PalletStakingRewardDestination extends Enum {
        readonly isStaked: boolean
        readonly isStash: boolean
        readonly isController: boolean
        readonly isAccount: boolean
        readonly asAccount: AccountId20
        readonly isNone: boolean
        readonly type: 'Staked' | 'Stash' | 'Controller' | 'Account' | 'None'
    }

    /** @name PalletStakingNominations (160) */
    interface PalletStakingNominations extends Struct {
        readonly targets: Vec<AccountId20>
        readonly submittedIn: u32
        readonly suppressed: bool
    }

    /** @name PalletStakingActiveEraInfo (162) */
    interface PalletStakingActiveEraInfo extends Struct {
        readonly index: u32
        readonly start: Option<u64>
    }

    /** @name PalletStakingEraRewardPoints (165) */
    interface PalletStakingEraRewardPoints extends Struct {
        readonly total: u32
        readonly individual: BTreeMap<AccountId20, u32>
    }

    /** @name PalletStakingUnappliedSlash (170) */
    interface PalletStakingUnappliedSlash extends Struct {
        readonly validator: AccountId20
        readonly own: u128
        readonly others: Vec<ITuple<[AccountId20, u128]>>
        readonly reporters: Vec<AccountId20>
        readonly payout: u128
    }

    /** @name PalletStakingSlashingSlashingSpans (174) */
    interface PalletStakingSlashingSlashingSpans extends Struct {
        readonly spanIndex: u32
        readonly lastStart: u32
        readonly lastNonzeroSlash: u32
        readonly prior: Vec<u32>
    }

    /** @name PalletStakingSlashingSpanRecord (175) */
    interface PalletStakingSlashingSpanRecord extends Struct {
        readonly slashed: u128
        readonly paidOut: u128
    }

    /** @name PalletStakingPalletCall (179) */
    interface PalletStakingPalletCall extends Enum {
        readonly isBond: boolean
        readonly asBond: {
            readonly value: Compact<u128>
            readonly payee: PalletStakingRewardDestination
        } & Struct
        readonly isBondExtra: boolean
        readonly asBondExtra: {
            readonly maxAdditional: Compact<u128>
        } & Struct
        readonly isUnbond: boolean
        readonly asUnbond: {
            readonly value: Compact<u128>
        } & Struct
        readonly isWithdrawUnbonded: boolean
        readonly asWithdrawUnbonded: {
            readonly numSlashingSpans: u32
        } & Struct
        readonly isValidate: boolean
        readonly asValidate: {
            readonly prefs: PalletStakingValidatorPrefs
        } & Struct
        readonly isNominate: boolean
        readonly asNominate: {
            readonly targets: Vec<AccountId20>
        } & Struct
        readonly isChill: boolean
        readonly isSetPayee: boolean
        readonly asSetPayee: {
            readonly payee: PalletStakingRewardDestination
        } & Struct
        readonly isSetController: boolean
        readonly isSetValidatorCount: boolean
        readonly asSetValidatorCount: {
            readonly new_: Compact<u32>
        } & Struct
        readonly isIncreaseValidatorCount: boolean
        readonly asIncreaseValidatorCount: {
            readonly additional: Compact<u32>
        } & Struct
        readonly isScaleValidatorCount: boolean
        readonly asScaleValidatorCount: {
            readonly factor: Percent
        } & Struct
        readonly isForceNoEras: boolean
        readonly isForceNewEra: boolean
        readonly isSetInvulnerables: boolean
        readonly asSetInvulnerables: {
            readonly invulnerables: Vec<AccountId20>
        } & Struct
        readonly isForceUnstake: boolean
        readonly asForceUnstake: {
            readonly stash: AccountId20
            readonly numSlashingSpans: u32
        } & Struct
        readonly isForceNewEraAlways: boolean
        readonly isCancelDeferredSlash: boolean
        readonly asCancelDeferredSlash: {
            readonly era: u32
            readonly slashIndices: Vec<u32>
        } & Struct
        readonly isPayoutStakers: boolean
        readonly asPayoutStakers: {
            readonly validatorStash: AccountId20
            readonly era: u32
        } & Struct
        readonly isRebond: boolean
        readonly asRebond: {
            readonly value: Compact<u128>
        } & Struct
        readonly isReapStash: boolean
        readonly asReapStash: {
            readonly stash: AccountId20
            readonly numSlashingSpans: u32
        } & Struct
        readonly isKick: boolean
        readonly asKick: {
            readonly who: Vec<AccountId20>
        } & Struct
        readonly isSetStakingConfigs: boolean
        readonly asSetStakingConfigs: {
            readonly minNominatorBond: PalletStakingPalletConfigOpU128
            readonly minValidatorBond: PalletStakingPalletConfigOpU128
            readonly maxNominatorCount: PalletStakingPalletConfigOpU32
            readonly maxValidatorCount: PalletStakingPalletConfigOpU32
            readonly chillThreshold: PalletStakingPalletConfigOpPercent
            readonly minCommission: PalletStakingPalletConfigOpPerbill
        } & Struct
        readonly isChillOther: boolean
        readonly asChillOther: {
            readonly controller: AccountId20
        } & Struct
        readonly isForceApplyMinCommission: boolean
        readonly asForceApplyMinCommission: {
            readonly validatorStash: AccountId20
        } & Struct
        readonly isSetMinCommission: boolean
        readonly asSetMinCommission: {
            readonly new_: Perbill
        } & Struct
        readonly type:
            | 'Bond'
            | 'BondExtra'
            | 'Unbond'
            | 'WithdrawUnbonded'
            | 'Validate'
            | 'Nominate'
            | 'Chill'
            | 'SetPayee'
            | 'SetController'
            | 'SetValidatorCount'
            | 'IncreaseValidatorCount'
            | 'ScaleValidatorCount'
            | 'ForceNoEras'
            | 'ForceNewEra'
            | 'SetInvulnerables'
            | 'ForceUnstake'
            | 'ForceNewEraAlways'
            | 'CancelDeferredSlash'
            | 'PayoutStakers'
            | 'Rebond'
            | 'ReapStash'
            | 'Kick'
            | 'SetStakingConfigs'
            | 'ChillOther'
            | 'ForceApplyMinCommission'
            | 'SetMinCommission'
    }

    /** @name PalletStakingPalletConfigOpU128 (180) */
    interface PalletStakingPalletConfigOpU128 extends Enum {
        readonly isNoop: boolean
        readonly isSet: boolean
        readonly asSet: u128
        readonly isRemove: boolean
        readonly type: 'Noop' | 'Set' | 'Remove'
    }

    /** @name PalletStakingPalletConfigOpU32 (181) */
    interface PalletStakingPalletConfigOpU32 extends Enum {
        readonly isNoop: boolean
        readonly isSet: boolean
        readonly asSet: u32
        readonly isRemove: boolean
        readonly type: 'Noop' | 'Set' | 'Remove'
    }

    /** @name PalletStakingPalletConfigOpPercent (182) */
    interface PalletStakingPalletConfigOpPercent extends Enum {
        readonly isNoop: boolean
        readonly isSet: boolean
        readonly asSet: Percent
        readonly isRemove: boolean
        readonly type: 'Noop' | 'Set' | 'Remove'
    }

    /** @name PalletStakingPalletConfigOpPerbill (183) */
    interface PalletStakingPalletConfigOpPerbill extends Enum {
        readonly isNoop: boolean
        readonly isSet: boolean
        readonly asSet: Perbill
        readonly isRemove: boolean
        readonly type: 'Noop' | 'Set' | 'Remove'
    }

    /** @name PalletStakingPalletError (184) */
    interface PalletStakingPalletError extends Enum {
        readonly isNotController: boolean
        readonly isNotStash: boolean
        readonly isAlreadyBonded: boolean
        readonly isAlreadyPaired: boolean
        readonly isEmptyTargets: boolean
        readonly isDuplicateIndex: boolean
        readonly isInvalidSlashIndex: boolean
        readonly isInsufficientBond: boolean
        readonly isNoMoreChunks: boolean
        readonly isNoUnlockChunk: boolean
        readonly isFundedTarget: boolean
        readonly isInvalidEraToReward: boolean
        readonly isInvalidNumberOfNominations: boolean
        readonly isNotSortedAndUnique: boolean
        readonly isAlreadyClaimed: boolean
        readonly isIncorrectHistoryDepth: boolean
        readonly isIncorrectSlashingSpans: boolean
        readonly isBadState: boolean
        readonly isTooManyTargets: boolean
        readonly isBadTarget: boolean
        readonly isCannotChillOther: boolean
        readonly isTooManyNominators: boolean
        readonly isTooManyValidators: boolean
        readonly isCommissionTooLow: boolean
        readonly isBoundNotMet: boolean
        readonly type:
            | 'NotController'
            | 'NotStash'
            | 'AlreadyBonded'
            | 'AlreadyPaired'
            | 'EmptyTargets'
            | 'DuplicateIndex'
            | 'InvalidSlashIndex'
            | 'InsufficientBond'
            | 'NoMoreChunks'
            | 'NoUnlockChunk'
            | 'FundedTarget'
            | 'InvalidEraToReward'
            | 'InvalidNumberOfNominations'
            | 'NotSortedAndUnique'
            | 'AlreadyClaimed'
            | 'IncorrectHistoryDepth'
            | 'IncorrectSlashingSpans'
            | 'BadState'
            | 'TooManyTargets'
            | 'BadTarget'
            | 'CannotChillOther'
            | 'TooManyNominators'
            | 'TooManyValidators'
            | 'CommissionTooLow'
            | 'BoundNotMet'
    }

    /** @name SpStakingOffenceOffenceDetails (185) */
    interface SpStakingOffenceOffenceDetails extends Struct {
        readonly offender: ITuple<[AccountId20, PalletStakingExposure]>
        readonly reporters: Vec<AccountId20>
    }

    /** @name FrontierTemplateRuntimeOpaqueSessionKeys (189) */
    interface FrontierTemplateRuntimeOpaqueSessionKeys extends Struct {
        readonly grandpa: SpConsensusGrandpaAppPublic
        readonly babe: SpConsensusBabeAppPublic
        readonly imOnline: PalletImOnlineSr25519AppSr25519Public
    }

    /** @name SpCoreCryptoKeyTypeId (191) */
    interface SpCoreCryptoKeyTypeId extends U8aFixed {}

    /** @name PalletSessionCall (192) */
    interface PalletSessionCall extends Enum {
        readonly isSetKeys: boolean
        readonly asSetKeys: {
            readonly keys_: FrontierTemplateRuntimeOpaqueSessionKeys
            readonly proof: Bytes
        } & Struct
        readonly isPurgeKeys: boolean
        readonly type: 'SetKeys' | 'PurgeKeys'
    }

    /** @name PalletSessionError (193) */
    interface PalletSessionError extends Enum {
        readonly isInvalidProof: boolean
        readonly isNoAssociatedValidatorId: boolean
        readonly isDuplicatedKey: boolean
        readonly isNoKeys: boolean
        readonly isNoAccount: boolean
        readonly type:
            | 'InvalidProof'
            | 'NoAssociatedValidatorId'
            | 'DuplicatedKey'
            | 'NoKeys'
            | 'NoAccount'
    }

    /** @name PalletGrandpaStoredState (194) */
    interface PalletGrandpaStoredState extends Enum {
        readonly isLive: boolean
        readonly isPendingPause: boolean
        readonly asPendingPause: {
            readonly scheduledAt: u32
            readonly delay: u32
        } & Struct
        readonly isPaused: boolean
        readonly isPendingResume: boolean
        readonly asPendingResume: {
            readonly scheduledAt: u32
            readonly delay: u32
        } & Struct
        readonly type: 'Live' | 'PendingPause' | 'Paused' | 'PendingResume'
    }

    /** @name PalletGrandpaStoredPendingChange (195) */
    interface PalletGrandpaStoredPendingChange extends Struct {
        readonly scheduledAt: u32
        readonly delay: u32
        readonly nextAuthorities: Vec<
            ITuple<[SpConsensusGrandpaAppPublic, u64]>
        >
        readonly forced: Option<u32>
    }

    /** @name PalletGrandpaCall (198) */
    interface PalletGrandpaCall extends Enum {
        readonly isReportEquivocation: boolean
        readonly asReportEquivocation: {
            readonly equivocationProof: SpConsensusGrandpaEquivocationProof
            readonly keyOwnerProof: SpSessionMembershipProof
        } & Struct
        readonly isReportEquivocationUnsigned: boolean
        readonly asReportEquivocationUnsigned: {
            readonly equivocationProof: SpConsensusGrandpaEquivocationProof
            readonly keyOwnerProof: SpSessionMembershipProof
        } & Struct
        readonly isNoteStalled: boolean
        readonly asNoteStalled: {
            readonly delay: u32
            readonly bestFinalizedBlockNumber: u32
        } & Struct
        readonly type:
            | 'ReportEquivocation'
            | 'ReportEquivocationUnsigned'
            | 'NoteStalled'
    }

    /** @name SpConsensusGrandpaEquivocationProof (199) */
    interface SpConsensusGrandpaEquivocationProof extends Struct {
        readonly setId: u64
        readonly equivocation: SpConsensusGrandpaEquivocation
    }

    /** @name SpConsensusGrandpaEquivocation (200) */
    interface SpConsensusGrandpaEquivocation extends Enum {
        readonly isPrevote: boolean
        readonly asPrevote: FinalityGrandpaEquivocationPrevote
        readonly isPrecommit: boolean
        readonly asPrecommit: FinalityGrandpaEquivocationPrecommit
        readonly type: 'Prevote' | 'Precommit'
    }

    /** @name FinalityGrandpaEquivocationPrevote (201) */
    interface FinalityGrandpaEquivocationPrevote extends Struct {
        readonly roundNumber: u64
        readonly identity: SpConsensusGrandpaAppPublic
        readonly first: ITuple<
            [FinalityGrandpaPrevote, SpConsensusGrandpaAppSignature]
        >
        readonly second: ITuple<
            [FinalityGrandpaPrevote, SpConsensusGrandpaAppSignature]
        >
    }

    /** @name FinalityGrandpaPrevote (202) */
    interface FinalityGrandpaPrevote extends Struct {
        readonly targetHash: H256
        readonly targetNumber: u32
    }

    /** @name SpConsensusGrandpaAppSignature (203) */
    interface SpConsensusGrandpaAppSignature extends SpCoreEd25519Signature {}

    /** @name SpCoreEd25519Signature (204) */
    interface SpCoreEd25519Signature extends U8aFixed {}

    /** @name FinalityGrandpaEquivocationPrecommit (206) */
    interface FinalityGrandpaEquivocationPrecommit extends Struct {
        readonly roundNumber: u64
        readonly identity: SpConsensusGrandpaAppPublic
        readonly first: ITuple<
            [FinalityGrandpaPrecommit, SpConsensusGrandpaAppSignature]
        >
        readonly second: ITuple<
            [FinalityGrandpaPrecommit, SpConsensusGrandpaAppSignature]
        >
    }

    /** @name FinalityGrandpaPrecommit (207) */
    interface FinalityGrandpaPrecommit extends Struct {
        readonly targetHash: H256
        readonly targetNumber: u32
    }

    /** @name PalletGrandpaError (209) */
    interface PalletGrandpaError extends Enum {
        readonly isPauseFailed: boolean
        readonly isResumeFailed: boolean
        readonly isChangePending: boolean
        readonly isTooSoon: boolean
        readonly isInvalidKeyOwnershipProof: boolean
        readonly isInvalidEquivocationProof: boolean
        readonly isDuplicateOffenceReport: boolean
        readonly type:
            | 'PauseFailed'
            | 'ResumeFailed'
            | 'ChangePending'
            | 'TooSoon'
            | 'InvalidKeyOwnershipProof'
            | 'InvalidEquivocationProof'
            | 'DuplicateOffenceReport'
    }

    /** @name PalletImOnlineCall (212) */
    interface PalletImOnlineCall extends Enum {
        readonly isHeartbeat: boolean
        readonly asHeartbeat: {
            readonly heartbeat: PalletImOnlineHeartbeat
            readonly signature: PalletImOnlineSr25519AppSr25519Signature
        } & Struct
        readonly type: 'Heartbeat'
    }

    /** @name PalletImOnlineHeartbeat (213) */
    interface PalletImOnlineHeartbeat extends Struct {
        readonly blockNumber: u32
        readonly sessionIndex: u32
        readonly authorityIndex: u32
        readonly validatorsLen: u32
    }

    /** @name PalletImOnlineSr25519AppSr25519Signature (214) */
    interface PalletImOnlineSr25519AppSr25519Signature
        extends SpCoreSr25519Signature {}

    /** @name SpCoreSr25519Signature (215) */
    interface SpCoreSr25519Signature extends U8aFixed {}

    /** @name PalletImOnlineError (216) */
    interface PalletImOnlineError extends Enum {
        readonly isInvalidKey: boolean
        readonly isDuplicatedHeartbeat: boolean
        readonly type: 'InvalidKey' | 'DuplicatedHeartbeat'
    }

    /** @name PalletBagsListListNode (217) */
    interface PalletBagsListListNode extends Struct {
        readonly id: AccountId20
        readonly prev: Option<AccountId20>
        readonly next: Option<AccountId20>
        readonly bagUpper: u64
        readonly score: u64
    }

    /** @name PalletBagsListListBag (218) */
    interface PalletBagsListListBag extends Struct {
        readonly head: Option<AccountId20>
        readonly tail: Option<AccountId20>
    }

    /** @name PalletBagsListCall (219) */
    interface PalletBagsListCall extends Enum {
        readonly isRebag: boolean
        readonly asRebag: {
            readonly dislocated: AccountId20
        } & Struct
        readonly isPutInFrontOf: boolean
        readonly asPutInFrontOf: {
            readonly lighter: AccountId20
        } & Struct
        readonly isPutInFrontOfOther: boolean
        readonly asPutInFrontOfOther: {
            readonly heavier: AccountId20
            readonly lighter: AccountId20
        } & Struct
        readonly type: 'Rebag' | 'PutInFrontOf' | 'PutInFrontOfOther'
    }

    /** @name PalletBagsListError (221) */
    interface PalletBagsListError extends Enum {
        readonly isList: boolean
        readonly asList: PalletBagsListListListError
        readonly type: 'List'
    }

    /** @name PalletBagsListListListError (222) */
    interface PalletBagsListListListError extends Enum {
        readonly isDuplicate: boolean
        readonly isNotHeavier: boolean
        readonly isNotInSameBag: boolean
        readonly isNodeNotFound: boolean
        readonly type:
            | 'Duplicate'
            | 'NotHeavier'
            | 'NotInSameBag'
            | 'NodeNotFound'
    }

    /** @name PalletTransactionPaymentReleases (225) */
    interface PalletTransactionPaymentReleases extends Enum {
        readonly isV1Ancient: boolean
        readonly isV2: boolean
        readonly type: 'V1Ancient' | 'V2'
    }

    /** @name PalletSudoCall (226) */
    interface PalletSudoCall extends Enum {
        readonly isSudo: boolean
        readonly asSudo: {
            readonly call: Call
        } & Struct
        readonly isSudoUncheckedWeight: boolean
        readonly asSudoUncheckedWeight: {
            readonly call: Call
            readonly weight: SpWeightsWeightV2Weight
        } & Struct
        readonly isSetKey: boolean
        readonly asSetKey: {
            readonly new_: AccountId20
        } & Struct
        readonly isSudoAs: boolean
        readonly asSudoAs: {
            readonly who: AccountId20
            readonly call: Call
        } & Struct
        readonly type: 'Sudo' | 'SudoUncheckedWeight' | 'SetKey' | 'SudoAs'
    }

    /** @name PalletUtilityCall (228) */
    interface PalletUtilityCall extends Enum {
        readonly isBatch: boolean
        readonly asBatch: {
            readonly calls: Vec<Call>
        } & Struct
        readonly isAsDerivative: boolean
        readonly asAsDerivative: {
            readonly index: u16
            readonly call: Call
        } & Struct
        readonly isBatchAll: boolean
        readonly asBatchAll: {
            readonly calls: Vec<Call>
        } & Struct
        readonly isDispatchAs: boolean
        readonly asDispatchAs: {
            readonly asOrigin: FrontierTemplateRuntimeOriginCaller
            readonly call: Call
        } & Struct
        readonly isForceBatch: boolean
        readonly asForceBatch: {
            readonly calls: Vec<Call>
        } & Struct
        readonly isWithWeight: boolean
        readonly asWithWeight: {
            readonly call: Call
            readonly weight: SpWeightsWeightV2Weight
        } & Struct
        readonly type:
            | 'Batch'
            | 'AsDerivative'
            | 'BatchAll'
            | 'DispatchAs'
            | 'ForceBatch'
            | 'WithWeight'
    }

    /** @name FrontierTemplateRuntimeOriginCaller (230) */
    interface FrontierTemplateRuntimeOriginCaller extends Enum {
        readonly isSystem: boolean
        readonly asSystem: FrameSupportDispatchRawOrigin
        readonly isVoid: boolean
        readonly isEthereum: boolean
        readonly asEthereum: PalletEthereumRawOrigin
        readonly type: 'System' | 'Void' | 'Ethereum'
    }

    /** @name FrameSupportDispatchRawOrigin (231) */
    interface FrameSupportDispatchRawOrigin extends Enum {
        readonly isRoot: boolean
        readonly isSigned: boolean
        readonly asSigned: AccountId20
        readonly isNone: boolean
        readonly type: 'Root' | 'Signed' | 'None'
    }

    /** @name PalletEthereumRawOrigin (232) */
    interface PalletEthereumRawOrigin extends Enum {
        readonly isEthereumTransaction: boolean
        readonly asEthereumTransaction: H160
        readonly type: 'EthereumTransaction'
    }

    /** @name SpCoreVoid (233) */
    type SpCoreVoid = Null

    /** @name PalletProxyCall (234) */
    interface PalletProxyCall extends Enum {
        readonly isProxy: boolean
        readonly asProxy: {
            readonly real: AccountId20
            readonly forceProxyType: Option<FrontierTemplateRuntimeProxyFilter>
            readonly call: Call
        } & Struct
        readonly isAddProxy: boolean
        readonly asAddProxy: {
            readonly delegate: AccountId20
            readonly proxyType: FrontierTemplateRuntimeProxyFilter
            readonly delay: u32
        } & Struct
        readonly isRemoveProxy: boolean
        readonly asRemoveProxy: {
            readonly delegate: AccountId20
            readonly proxyType: FrontierTemplateRuntimeProxyFilter
            readonly delay: u32
        } & Struct
        readonly isRemoveProxies: boolean
        readonly isCreatePure: boolean
        readonly asCreatePure: {
            readonly proxyType: FrontierTemplateRuntimeProxyFilter
            readonly delay: u32
            readonly index: u16
        } & Struct
        readonly isKillPure: boolean
        readonly asKillPure: {
            readonly spawner: AccountId20
            readonly proxyType: FrontierTemplateRuntimeProxyFilter
            readonly index: u16
            readonly height: Compact<u32>
            readonly extIndex: Compact<u32>
        } & Struct
        readonly isAnnounce: boolean
        readonly asAnnounce: {
            readonly real: AccountId20
            readonly callHash: H256
        } & Struct
        readonly isRemoveAnnouncement: boolean
        readonly asRemoveAnnouncement: {
            readonly real: AccountId20
            readonly callHash: H256
        } & Struct
        readonly isRejectAnnouncement: boolean
        readonly asRejectAnnouncement: {
            readonly delegate: AccountId20
            readonly callHash: H256
        } & Struct
        readonly isProxyAnnounced: boolean
        readonly asProxyAnnounced: {
            readonly delegate: AccountId20
            readonly real: AccountId20
            readonly forceProxyType: Option<FrontierTemplateRuntimeProxyFilter>
            readonly call: Call
        } & Struct
        readonly type:
            | 'Proxy'
            | 'AddProxy'
            | 'RemoveProxy'
            | 'RemoveProxies'
            | 'CreatePure'
            | 'KillPure'
            | 'Announce'
            | 'RemoveAnnouncement'
            | 'RejectAnnouncement'
            | 'ProxyAnnounced'
    }

    /** @name PalletIdentityCall (236) */
    interface PalletIdentityCall extends Enum {
        readonly isAddRegistrar: boolean
        readonly asAddRegistrar: {
            readonly account: AccountId20
        } & Struct
        readonly isSetIdentity: boolean
        readonly asSetIdentity: {
            readonly info: PalletIdentityIdentityInfo
        } & Struct
        readonly isSetSubs: boolean
        readonly asSetSubs: {
            readonly subs: Vec<ITuple<[AccountId20, Data]>>
        } & Struct
        readonly isClearIdentity: boolean
        readonly isRequestJudgement: boolean
        readonly asRequestJudgement: {
            readonly regIndex: Compact<u32>
            readonly maxFee: Compact<u128>
        } & Struct
        readonly isCancelRequest: boolean
        readonly asCancelRequest: {
            readonly regIndex: u32
        } & Struct
        readonly isSetFee: boolean
        readonly asSetFee: {
            readonly index: Compact<u32>
            readonly fee: Compact<u128>
        } & Struct
        readonly isSetAccountId: boolean
        readonly asSetAccountId: {
            readonly index: Compact<u32>
            readonly new_: AccountId20
        } & Struct
        readonly isSetFields: boolean
        readonly asSetFields: {
            readonly index: Compact<u32>
            readonly fields: PalletIdentityBitFlags
        } & Struct
        readonly isProvideJudgement: boolean
        readonly asProvideJudgement: {
            readonly regIndex: Compact<u32>
            readonly target: AccountId20
            readonly judgement: PalletIdentityJudgement
            readonly identity: H256
        } & Struct
        readonly isKillIdentity: boolean
        readonly asKillIdentity: {
            readonly target: AccountId20
        } & Struct
        readonly isAddSub: boolean
        readonly asAddSub: {
            readonly sub: AccountId20
            readonly data: Data
        } & Struct
        readonly isRenameSub: boolean
        readonly asRenameSub: {
            readonly sub: AccountId20
            readonly data: Data
        } & Struct
        readonly isRemoveSub: boolean
        readonly asRemoveSub: {
            readonly sub: AccountId20
        } & Struct
        readonly isQuitSub: boolean
        readonly type:
            | 'AddRegistrar'
            | 'SetIdentity'
            | 'SetSubs'
            | 'ClearIdentity'
            | 'RequestJudgement'
            | 'CancelRequest'
            | 'SetFee'
            | 'SetAccountId'
            | 'SetFields'
            | 'ProvideJudgement'
            | 'KillIdentity'
            | 'AddSub'
            | 'RenameSub'
            | 'RemoveSub'
            | 'QuitSub'
    }

    /** @name PalletIdentityIdentityInfo (237) */
    interface PalletIdentityIdentityInfo extends Struct {
        readonly additional: Vec<ITuple<[Data, Data]>>
        readonly display: Data
        readonly legal: Data
        readonly web: Data
        readonly riot: Data
        readonly email: Data
        readonly pgpFingerprint: Option<U8aFixed>
        readonly image: Data
        readonly twitter: Data
    }

    /** @name PalletIdentityBitFlags (273) */
    interface PalletIdentityBitFlags extends Set {
        readonly isDisplay: boolean
        readonly isLegal: boolean
        readonly isWeb: boolean
        readonly isRiot: boolean
        readonly isEmail: boolean
        readonly isPgpFingerprint: boolean
        readonly isImage: boolean
        readonly isTwitter: boolean
    }

    /** @name PalletIdentityIdentityField (274) */
    interface PalletIdentityIdentityField extends Enum {
        readonly isDisplay: boolean
        readonly isLegal: boolean
        readonly isWeb: boolean
        readonly isRiot: boolean
        readonly isEmail: boolean
        readonly isPgpFingerprint: boolean
        readonly isImage: boolean
        readonly isTwitter: boolean
        readonly type:
            | 'Display'
            | 'Legal'
            | 'Web'
            | 'Riot'
            | 'Email'
            | 'PgpFingerprint'
            | 'Image'
            | 'Twitter'
    }

    /** @name PalletIdentityJudgement (275) */
    interface PalletIdentityJudgement extends Enum {
        readonly isUnknown: boolean
        readonly isFeePaid: boolean
        readonly asFeePaid: u128
        readonly isReasonable: boolean
        readonly isKnownGood: boolean
        readonly isOutOfDate: boolean
        readonly isLowQuality: boolean
        readonly isErroneous: boolean
        readonly type:
            | 'Unknown'
            | 'FeePaid'
            | 'Reasonable'
            | 'KnownGood'
            | 'OutOfDate'
            | 'LowQuality'
            | 'Erroneous'
    }

    /** @name PalletFastUnstakeCall (276) */
    interface PalletFastUnstakeCall extends Enum {
        readonly isRegisterFastUnstake: boolean
        readonly isDeregister: boolean
        readonly isControl: boolean
        readonly asControl: {
            readonly erasToCheck: u32
        } & Struct
        readonly type: 'RegisterFastUnstake' | 'Deregister' | 'Control'
    }

    /** @name PalletNominationPoolsCall (277) */
    interface PalletNominationPoolsCall extends Enum {
        readonly isJoin: boolean
        readonly asJoin: {
            readonly amount: Compact<u128>
            readonly poolId: u32
        } & Struct
        readonly isBondExtra: boolean
        readonly asBondExtra: {
            readonly extra: PalletNominationPoolsBondExtra
        } & Struct
        readonly isClaimPayout: boolean
        readonly isUnbond: boolean
        readonly asUnbond: {
            readonly memberAccount: AccountId20
            readonly unbondingPoints: Compact<u128>
        } & Struct
        readonly isPoolWithdrawUnbonded: boolean
        readonly asPoolWithdrawUnbonded: {
            readonly poolId: u32
            readonly numSlashingSpans: u32
        } & Struct
        readonly isWithdrawUnbonded: boolean
        readonly asWithdrawUnbonded: {
            readonly memberAccount: AccountId20
            readonly numSlashingSpans: u32
        } & Struct
        readonly isCreate: boolean
        readonly asCreate: {
            readonly amount: Compact<u128>
            readonly root: AccountId20
            readonly nominator: AccountId20
            readonly bouncer: AccountId20
        } & Struct
        readonly isCreateWithPoolId: boolean
        readonly asCreateWithPoolId: {
            readonly amount: Compact<u128>
            readonly root: AccountId20
            readonly nominator: AccountId20
            readonly bouncer: AccountId20
            readonly poolId: u32
        } & Struct
        readonly isNominate: boolean
        readonly asNominate: {
            readonly poolId: u32
            readonly validators: Vec<AccountId20>
        } & Struct
        readonly isSetState: boolean
        readonly asSetState: {
            readonly poolId: u32
            readonly state: PalletNominationPoolsPoolState
        } & Struct
        readonly isSetMetadata: boolean
        readonly asSetMetadata: {
            readonly poolId: u32
            readonly metadata: Bytes
        } & Struct
        readonly isSetConfigs: boolean
        readonly asSetConfigs: {
            readonly minJoinBond: PalletNominationPoolsConfigOpU128
            readonly minCreateBond: PalletNominationPoolsConfigOpU128
            readonly maxPools: PalletNominationPoolsConfigOpU32
            readonly maxMembers: PalletNominationPoolsConfigOpU32
            readonly maxMembersPerPool: PalletNominationPoolsConfigOpU32
            readonly globalMaxCommission: PalletNominationPoolsConfigOpPerbill
        } & Struct
        readonly isUpdateRoles: boolean
        readonly asUpdateRoles: {
            readonly poolId: u32
            readonly newRoot: PalletNominationPoolsConfigOpAccountId20
            readonly newNominator: PalletNominationPoolsConfigOpAccountId20
            readonly newBouncer: PalletNominationPoolsConfigOpAccountId20
        } & Struct
        readonly isChill: boolean
        readonly asChill: {
            readonly poolId: u32
        } & Struct
        readonly isBondExtraOther: boolean
        readonly asBondExtraOther: {
            readonly member: AccountId20
            readonly extra: PalletNominationPoolsBondExtra
        } & Struct
        readonly isSetClaimPermission: boolean
        readonly asSetClaimPermission: {
            readonly permission: PalletNominationPoolsClaimPermission
        } & Struct
        readonly isClaimPayoutOther: boolean
        readonly asClaimPayoutOther: {
            readonly other: AccountId20
        } & Struct
        readonly isSetCommission: boolean
        readonly asSetCommission: {
            readonly poolId: u32
            readonly newCommission: Option<ITuple<[Perbill, AccountId20]>>
        } & Struct
        readonly isSetCommissionMax: boolean
        readonly asSetCommissionMax: {
            readonly poolId: u32
            readonly maxCommission: Perbill
        } & Struct
        readonly isSetCommissionChangeRate: boolean
        readonly asSetCommissionChangeRate: {
            readonly poolId: u32
            readonly changeRate: PalletNominationPoolsCommissionChangeRate
        } & Struct
        readonly isClaimCommission: boolean
        readonly asClaimCommission: {
            readonly poolId: u32
        } & Struct
        readonly type:
            | 'Join'
            | 'BondExtra'
            | 'ClaimPayout'
            | 'Unbond'
            | 'PoolWithdrawUnbonded'
            | 'WithdrawUnbonded'
            | 'Create'
            | 'CreateWithPoolId'
            | 'Nominate'
            | 'SetState'
            | 'SetMetadata'
            | 'SetConfigs'
            | 'UpdateRoles'
            | 'Chill'
            | 'BondExtraOther'
            | 'SetClaimPermission'
            | 'ClaimPayoutOther'
            | 'SetCommission'
            | 'SetCommissionMax'
            | 'SetCommissionChangeRate'
            | 'ClaimCommission'
    }

    /** @name PalletNominationPoolsBondExtra (278) */
    interface PalletNominationPoolsBondExtra extends Enum {
        readonly isFreeBalance: boolean
        readonly asFreeBalance: u128
        readonly isRewards: boolean
        readonly type: 'FreeBalance' | 'Rewards'
    }

    /** @name PalletNominationPoolsConfigOpU128 (279) */
    interface PalletNominationPoolsConfigOpU128 extends Enum {
        readonly isNoop: boolean
        readonly isSet: boolean
        readonly asSet: u128
        readonly isRemove: boolean
        readonly type: 'Noop' | 'Set' | 'Remove'
    }

    /** @name PalletNominationPoolsConfigOpU32 (280) */
    interface PalletNominationPoolsConfigOpU32 extends Enum {
        readonly isNoop: boolean
        readonly isSet: boolean
        readonly asSet: u32
        readonly isRemove: boolean
        readonly type: 'Noop' | 'Set' | 'Remove'
    }

    /** @name PalletNominationPoolsConfigOpPerbill (281) */
    interface PalletNominationPoolsConfigOpPerbill extends Enum {
        readonly isNoop: boolean
        readonly isSet: boolean
        readonly asSet: Perbill
        readonly isRemove: boolean
        readonly type: 'Noop' | 'Set' | 'Remove'
    }

    /** @name PalletNominationPoolsConfigOpAccountId20 (282) */
    interface PalletNominationPoolsConfigOpAccountId20 extends Enum {
        readonly isNoop: boolean
        readonly isSet: boolean
        readonly asSet: AccountId20
        readonly isRemove: boolean
        readonly type: 'Noop' | 'Set' | 'Remove'
    }

    /** @name PalletNominationPoolsClaimPermission (283) */
    interface PalletNominationPoolsClaimPermission extends Enum {
        readonly isPermissioned: boolean
        readonly isPermissionlessCompound: boolean
        readonly isPermissionlessWithdraw: boolean
        readonly isPermissionlessAll: boolean
        readonly type:
            | 'Permissioned'
            | 'PermissionlessCompound'
            | 'PermissionlessWithdraw'
            | 'PermissionlessAll'
    }

    /** @name PalletEthereumCall (284) */
    interface PalletEthereumCall extends Enum {
        readonly isTransact: boolean
        readonly asTransact: {
            readonly transaction: EthereumTransactionTransactionV2
        } & Struct
        readonly type: 'Transact'
    }

    /** @name EthereumTransactionTransactionV2 (285) */
    interface EthereumTransactionTransactionV2 extends Enum {
        readonly isLegacy: boolean
        readonly asLegacy: EthereumTransactionLegacyTransaction
        readonly isEip2930: boolean
        readonly asEip2930: EthereumTransactionEip2930Transaction
        readonly isEip1559: boolean
        readonly asEip1559: EthereumTransactionEip1559Transaction
        readonly type: 'Legacy' | 'Eip2930' | 'Eip1559'
    }

    /** @name EthereumTransactionLegacyTransaction (286) */
    interface EthereumTransactionLegacyTransaction extends Struct {
        readonly nonce: U256
        readonly gasPrice: U256
        readonly gasLimit: U256
        readonly action: EthereumTransactionTransactionAction
        readonly value: U256
        readonly input: Bytes
        readonly signature: EthereumTransactionTransactionSignature
    }

    /** @name EthereumTransactionTransactionAction (287) */
    interface EthereumTransactionTransactionAction extends Enum {
        readonly isCall: boolean
        readonly asCall: H160
        readonly isCreate: boolean
        readonly type: 'Call' | 'Create'
    }

    /** @name EthereumTransactionTransactionSignature (288) */
    interface EthereumTransactionTransactionSignature extends Struct {
        readonly v: u64
        readonly r: H256
        readonly s: H256
    }

    /** @name EthereumTransactionEip2930Transaction (290) */
    interface EthereumTransactionEip2930Transaction extends Struct {
        readonly chainId: u64
        readonly nonce: U256
        readonly gasPrice: U256
        readonly gasLimit: U256
        readonly action: EthereumTransactionTransactionAction
        readonly value: U256
        readonly input: Bytes
        readonly accessList: Vec<EthereumTransactionAccessListItem>
        readonly oddYParity: bool
        readonly r: H256
        readonly s: H256
    }

    /** @name EthereumTransactionAccessListItem (292) */
    interface EthereumTransactionAccessListItem extends Struct {
        readonly address: H160
        readonly storageKeys: Vec<H256>
    }

    /** @name EthereumTransactionEip1559Transaction (293) */
    interface EthereumTransactionEip1559Transaction extends Struct {
        readonly chainId: u64
        readonly nonce: U256
        readonly maxPriorityFeePerGas: U256
        readonly maxFeePerGas: U256
        readonly gasLimit: U256
        readonly action: EthereumTransactionTransactionAction
        readonly value: U256
        readonly input: Bytes
        readonly accessList: Vec<EthereumTransactionAccessListItem>
        readonly oddYParity: bool
        readonly r: H256
        readonly s: H256
    }

    /** @name PalletEvmCall (294) */
    interface PalletEvmCall extends Enum {
        readonly isWithdraw: boolean
        readonly asWithdraw: {
            readonly address: H160
            readonly value: u128
        } & Struct
        readonly isCall: boolean
        readonly asCall: {
            readonly source: H160
            readonly target: H160
            readonly input: Bytes
            readonly value: U256
            readonly gasLimit: u64
            readonly maxFeePerGas: U256
            readonly maxPriorityFeePerGas: Option<U256>
            readonly nonce: Option<U256>
            readonly accessList: Vec<ITuple<[H160, Vec<H256>]>>
        } & Struct
        readonly isCreate: boolean
        readonly asCreate: {
            readonly source: H160
            readonly init: Bytes
            readonly value: U256
            readonly gasLimit: u64
            readonly maxFeePerGas: U256
            readonly maxPriorityFeePerGas: Option<U256>
            readonly nonce: Option<U256>
            readonly accessList: Vec<ITuple<[H160, Vec<H256>]>>
        } & Struct
        readonly isCreate2: boolean
        readonly asCreate2: {
            readonly source: H160
            readonly init: Bytes
            readonly salt: H256
            readonly value: U256
            readonly gasLimit: u64
            readonly maxFeePerGas: U256
            readonly maxPriorityFeePerGas: Option<U256>
            readonly nonce: Option<U256>
            readonly accessList: Vec<ITuple<[H160, Vec<H256>]>>
        } & Struct
        readonly type: 'Withdraw' | 'Call' | 'Create' | 'Create2'
    }

    /** @name PalletDynamicFeeCall (298) */
    interface PalletDynamicFeeCall extends Enum {
        readonly isNoteMinGasPriceTarget: boolean
        readonly asNoteMinGasPriceTarget: {
            readonly target: U256
        } & Struct
        readonly type: 'NoteMinGasPriceTarget'
    }

    /** @name PalletBaseFeeCall (299) */
    interface PalletBaseFeeCall extends Enum {
        readonly isSetBaseFeePerGas: boolean
        readonly asSetBaseFeePerGas: {
            readonly fee: U256
        } & Struct
        readonly isSetElasticity: boolean
        readonly asSetElasticity: {
            readonly elasticity: Permill
        } & Struct
        readonly type: 'SetBaseFeePerGas' | 'SetElasticity'
    }

    /** @name PalletHotfixSufficientsCall (300) */
    interface PalletHotfixSufficientsCall extends Enum {
        readonly isHotfixIncAccountSufficients: boolean
        readonly asHotfixIncAccountSufficients: {
            readonly addresses: Vec<H160>
        } & Struct
        readonly type: 'HotfixIncAccountSufficients'
    }

    /** @name PalletSudoError (302) */
    interface PalletSudoError extends Enum {
        readonly isRequireSudo: boolean
        readonly type: 'RequireSudo'
    }

    /** @name PalletUtilityError (303) */
    interface PalletUtilityError extends Enum {
        readonly isTooManyCalls: boolean
        readonly type: 'TooManyCalls'
    }

    /** @name PalletProxyProxyDefinition (306) */
    interface PalletProxyProxyDefinition extends Struct {
        readonly delegate: AccountId20
        readonly proxyType: FrontierTemplateRuntimeProxyFilter
        readonly delay: u32
    }

    /** @name PalletProxyAnnouncement (310) */
    interface PalletProxyAnnouncement extends Struct {
        readonly real: AccountId20
        readonly callHash: H256
        readonly height: u32
    }

    /** @name PalletProxyError (312) */
    interface PalletProxyError extends Enum {
        readonly isTooMany: boolean
        readonly isNotFound: boolean
        readonly isNotProxy: boolean
        readonly isUnproxyable: boolean
        readonly isDuplicate: boolean
        readonly isNoPermission: boolean
        readonly isUnannounced: boolean
        readonly isNoSelfProxy: boolean
        readonly type:
            | 'TooMany'
            | 'NotFound'
            | 'NotProxy'
            | 'Unproxyable'
            | 'Duplicate'
            | 'NoPermission'
            | 'Unannounced'
            | 'NoSelfProxy'
    }

    /** @name PalletIdentityRegistration (313) */
    interface PalletIdentityRegistration extends Struct {
        readonly judgements: Vec<ITuple<[u32, PalletIdentityJudgement]>>
        readonly deposit: u128
        readonly info: PalletIdentityIdentityInfo
    }

    /** @name PalletIdentityRegistrarInfo (321) */
    interface PalletIdentityRegistrarInfo extends Struct {
        readonly account: AccountId20
        readonly fee: u128
        readonly fields: PalletIdentityBitFlags
    }

    /** @name PalletIdentityError (323) */
    interface PalletIdentityError extends Enum {
        readonly isTooManySubAccounts: boolean
        readonly isNotFound: boolean
        readonly isNotNamed: boolean
        readonly isEmptyIndex: boolean
        readonly isFeeChanged: boolean
        readonly isNoIdentity: boolean
        readonly isStickyJudgement: boolean
        readonly isJudgementGiven: boolean
        readonly isInvalidJudgement: boolean
        readonly isInvalidIndex: boolean
        readonly isInvalidTarget: boolean
        readonly isTooManyFields: boolean
        readonly isTooManyRegistrars: boolean
        readonly isAlreadyClaimed: boolean
        readonly isNotSub: boolean
        readonly isNotOwned: boolean
        readonly isJudgementForDifferentIdentity: boolean
        readonly isJudgementPaymentFailed: boolean
        readonly type:
            | 'TooManySubAccounts'
            | 'NotFound'
            | 'NotNamed'
            | 'EmptyIndex'
            | 'FeeChanged'
            | 'NoIdentity'
            | 'StickyJudgement'
            | 'JudgementGiven'
            | 'InvalidJudgement'
            | 'InvalidIndex'
            | 'InvalidTarget'
            | 'TooManyFields'
            | 'TooManyRegistrars'
            | 'AlreadyClaimed'
            | 'NotSub'
            | 'NotOwned'
            | 'JudgementForDifferentIdentity'
            | 'JudgementPaymentFailed'
    }

    /** @name PalletFastUnstakeUnstakeRequest (324) */
    interface PalletFastUnstakeUnstakeRequest extends Struct {
        readonly stashes: Vec<ITuple<[AccountId20, u128]>>
        readonly checked: Vec<u32>
    }

    /** @name PalletFastUnstakeError (327) */
    interface PalletFastUnstakeError extends Enum {
        readonly isNotController: boolean
        readonly isAlreadyQueued: boolean
        readonly isNotFullyBonded: boolean
        readonly isNotQueued: boolean
        readonly isAlreadyHead: boolean
        readonly isCallNotAllowed: boolean
        readonly type:
            | 'NotController'
            | 'AlreadyQueued'
            | 'NotFullyBonded'
            | 'NotQueued'
            | 'AlreadyHead'
            | 'CallNotAllowed'
    }

    /** @name PalletNominationPoolsPoolMember (328) */
    interface PalletNominationPoolsPoolMember extends Struct {
        readonly poolId: u32
        readonly points: u128
        readonly lastRecordedRewardCounter: u128
        readonly unbondingEras: BTreeMap<u32, u128>
    }

    /** @name PalletNominationPoolsBondedPoolInner (333) */
    interface PalletNominationPoolsBondedPoolInner extends Struct {
        readonly commission: PalletNominationPoolsCommission
        readonly memberCounter: u32
        readonly points: u128
        readonly roles: PalletNominationPoolsPoolRoles
        readonly state: PalletNominationPoolsPoolState
    }

    /** @name PalletNominationPoolsCommission (334) */
    interface PalletNominationPoolsCommission extends Struct {
        readonly current: Option<ITuple<[Perbill, AccountId20]>>
        readonly max: Option<Perbill>
        readonly changeRate: Option<PalletNominationPoolsCommissionChangeRate>
        readonly throttleFrom: Option<u32>
    }

    /** @name PalletNominationPoolsPoolRoles (337) */
    interface PalletNominationPoolsPoolRoles extends Struct {
        readonly depositor: AccountId20
        readonly root: Option<AccountId20>
        readonly nominator: Option<AccountId20>
        readonly bouncer: Option<AccountId20>
    }

    /** @name PalletNominationPoolsRewardPool (338) */
    interface PalletNominationPoolsRewardPool extends Struct {
        readonly lastRecordedRewardCounter: u128
        readonly lastRecordedTotalPayouts: u128
        readonly totalRewardsClaimed: u128
        readonly totalCommissionPending: u128
        readonly totalCommissionClaimed: u128
    }

    /** @name PalletNominationPoolsSubPools (339) */
    interface PalletNominationPoolsSubPools extends Struct {
        readonly noEra: PalletNominationPoolsUnbondPool
        readonly withEra: BTreeMap<u32, PalletNominationPoolsUnbondPool>
    }

    /** @name PalletNominationPoolsUnbondPool (340) */
    interface PalletNominationPoolsUnbondPool extends Struct {
        readonly points: u128
        readonly balance: u128
    }

    /** @name FrameSupportPalletId (346) */
    interface FrameSupportPalletId extends U8aFixed {}

    /** @name PalletNominationPoolsError (347) */
    interface PalletNominationPoolsError extends Enum {
        readonly isPoolNotFound: boolean
        readonly isPoolMemberNotFound: boolean
        readonly isRewardPoolNotFound: boolean
        readonly isSubPoolsNotFound: boolean
        readonly isAccountBelongsToOtherPool: boolean
        readonly isFullyUnbonding: boolean
        readonly isMaxUnbondingLimit: boolean
        readonly isCannotWithdrawAny: boolean
        readonly isMinimumBondNotMet: boolean
        readonly isOverflowRisk: boolean
        readonly isNotDestroying: boolean
        readonly isNotNominator: boolean
        readonly isNotKickerOrDestroying: boolean
        readonly isNotOpen: boolean
        readonly isMaxPools: boolean
        readonly isMaxPoolMembers: boolean
        readonly isCanNotChangeState: boolean
        readonly isDoesNotHavePermission: boolean
        readonly isMetadataExceedsMaxLen: boolean
        readonly isDefensive: boolean
        readonly asDefensive: PalletNominationPoolsDefensiveError
        readonly isPartialUnbondNotAllowedPermissionlessly: boolean
        readonly isMaxCommissionRestricted: boolean
        readonly isCommissionExceedsMaximum: boolean
        readonly isCommissionExceedsGlobalMaximum: boolean
        readonly isCommissionChangeThrottled: boolean
        readonly isCommissionChangeRateNotAllowed: boolean
        readonly isNoPendingCommission: boolean
        readonly isNoCommissionCurrentSet: boolean
        readonly isPoolIdInUse: boolean
        readonly isInvalidPoolId: boolean
        readonly isBondExtraRestricted: boolean
        readonly type:
            | 'PoolNotFound'
            | 'PoolMemberNotFound'
            | 'RewardPoolNotFound'
            | 'SubPoolsNotFound'
            | 'AccountBelongsToOtherPool'
            | 'FullyUnbonding'
            | 'MaxUnbondingLimit'
            | 'CannotWithdrawAny'
            | 'MinimumBondNotMet'
            | 'OverflowRisk'
            | 'NotDestroying'
            | 'NotNominator'
            | 'NotKickerOrDestroying'
            | 'NotOpen'
            | 'MaxPools'
            | 'MaxPoolMembers'
            | 'CanNotChangeState'
            | 'DoesNotHavePermission'
            | 'MetadataExceedsMaxLen'
            | 'Defensive'
            | 'PartialUnbondNotAllowedPermissionlessly'
            | 'MaxCommissionRestricted'
            | 'CommissionExceedsMaximum'
            | 'CommissionExceedsGlobalMaximum'
            | 'CommissionChangeThrottled'
            | 'CommissionChangeRateNotAllowed'
            | 'NoPendingCommission'
            | 'NoCommissionCurrentSet'
            | 'PoolIdInUse'
            | 'InvalidPoolId'
            | 'BondExtraRestricted'
    }

    /** @name PalletNominationPoolsDefensiveError (348) */
    interface PalletNominationPoolsDefensiveError extends Enum {
        readonly isNotEnoughSpaceInUnbondPool: boolean
        readonly isPoolNotFound: boolean
        readonly isRewardPoolNotFound: boolean
        readonly isSubPoolsNotFound: boolean
        readonly isBondedStashKilledPrematurely: boolean
        readonly type:
            | 'NotEnoughSpaceInUnbondPool'
            | 'PoolNotFound'
            | 'RewardPoolNotFound'
            | 'SubPoolsNotFound'
            | 'BondedStashKilledPrematurely'
    }

    /** @name FpRpcTransactionStatus (351) */
    interface FpRpcTransactionStatus extends Struct {
        readonly transactionHash: H256
        readonly transactionIndex: u32
        readonly from: H160
        readonly to: Option<H160>
        readonly contractAddress: Option<H160>
        readonly logs: Vec<EthereumLog>
        readonly logsBloom: EthbloomBloom
    }

    /** @name EthbloomBloom (354) */
    interface EthbloomBloom extends U8aFixed {}

    /** @name EthereumReceiptReceiptV3 (356) */
    interface EthereumReceiptReceiptV3 extends Enum {
        readonly isLegacy: boolean
        readonly asLegacy: EthereumReceiptEip658ReceiptData
        readonly isEip2930: boolean
        readonly asEip2930: EthereumReceiptEip658ReceiptData
        readonly isEip1559: boolean
        readonly asEip1559: EthereumReceiptEip658ReceiptData
        readonly type: 'Legacy' | 'Eip2930' | 'Eip1559'
    }

    /** @name EthereumReceiptEip658ReceiptData (357) */
    interface EthereumReceiptEip658ReceiptData extends Struct {
        readonly statusCode: u8
        readonly usedGas: U256
        readonly logsBloom: EthbloomBloom
        readonly logs: Vec<EthereumLog>
    }

    /** @name EthereumBlock (358) */
    interface EthereumBlock extends Struct {
        readonly header: EthereumHeader
        readonly transactions: Vec<EthereumTransactionTransactionV2>
        readonly ommers: Vec<EthereumHeader>
    }

    /** @name EthereumHeader (359) */
    interface EthereumHeader extends Struct {
        readonly parentHash: H256
        readonly ommersHash: H256
        readonly beneficiary: H160
        readonly stateRoot: H256
        readonly transactionsRoot: H256
        readonly receiptsRoot: H256
        readonly logsBloom: EthbloomBloom
        readonly difficulty: U256
        readonly number: U256
        readonly gasLimit: U256
        readonly gasUsed: U256
        readonly timestamp: u64
        readonly extraData: Bytes
        readonly mixHash: H256
        readonly nonce: EthereumTypesHashH64
    }

    /** @name EthereumTypesHashH64 (360) */
    interface EthereumTypesHashH64 extends U8aFixed {}

    /** @name PalletEthereumError (365) */
    interface PalletEthereumError extends Enum {
        readonly isInvalidSignature: boolean
        readonly isPreLogExists: boolean
        readonly type: 'InvalidSignature' | 'PreLogExists'
    }

    /** @name PalletEvmCodeMetadata (366) */
    interface PalletEvmCodeMetadata extends Struct {
        readonly size_: u64
        readonly hash_: H256
    }

    /** @name PalletEvmError (368) */
    interface PalletEvmError extends Enum {
        readonly isBalanceLow: boolean
        readonly isFeeOverflow: boolean
        readonly isPaymentOverflow: boolean
        readonly isWithdrawFailed: boolean
        readonly isGasPriceTooLow: boolean
        readonly isInvalidNonce: boolean
        readonly isGasLimitTooLow: boolean
        readonly isGasLimitTooHigh: boolean
        readonly isUndefined: boolean
        readonly isReentrancy: boolean
        readonly isTransactionMustComeFromEOA: boolean
        readonly type:
            | 'BalanceLow'
            | 'FeeOverflow'
            | 'PaymentOverflow'
            | 'WithdrawFailed'
            | 'GasPriceTooLow'
            | 'InvalidNonce'
            | 'GasLimitTooLow'
            | 'GasLimitTooHigh'
            | 'Undefined'
            | 'Reentrancy'
            | 'TransactionMustComeFromEOA'
    }

    /** @name PalletHotfixSufficientsError (369) */
    interface PalletHotfixSufficientsError extends Enum {
        readonly isMaxAddressCountExceeded: boolean
        readonly type: 'MaxAddressCountExceeded'
    }

    /** @name FpAccountEthereumSignature (371) */
    interface FpAccountEthereumSignature extends SpCoreEcdsaSignature {}

    /** @name SpCoreEcdsaSignature (372) */
    interface SpCoreEcdsaSignature extends U8aFixed {}

    /** @name FrameSystemExtensionsCheckNonZeroSender (375) */
    type FrameSystemExtensionsCheckNonZeroSender = Null

    /** @name FrameSystemExtensionsCheckSpecVersion (376) */
    type FrameSystemExtensionsCheckSpecVersion = Null

    /** @name FrameSystemExtensionsCheckTxVersion (377) */
    type FrameSystemExtensionsCheckTxVersion = Null

    /** @name FrameSystemExtensionsCheckGenesis (378) */
    type FrameSystemExtensionsCheckGenesis = Null

    /** @name FrameSystemExtensionsCheckNonce (381) */
    interface FrameSystemExtensionsCheckNonce extends Compact<u32> {}

    /** @name FrameSystemExtensionsCheckWeight (382) */
    type FrameSystemExtensionsCheckWeight = Null

    /** @name PalletTransactionPaymentChargeTransactionPayment (383) */
    interface PalletTransactionPaymentChargeTransactionPayment
        extends Compact<u128> {}

    /** @name FrontierTemplateRuntimeRuntime (385) */
    type FrontierTemplateRuntimeRuntime = Null
} // declare module
