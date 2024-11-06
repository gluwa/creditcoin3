// Auto-generated via `yarn polkadot-types-from-defs`, do not edit
/* eslint-disable */

// import type lookup before we augment - in some environments
// this is required to allow for ambient/previous definitions
import '@polkadot/types/types/registry';

import type {
    Creditcoin3RuntimeOpaqueSessionKeys,
    Creditcoin3RuntimeOriginCaller,
    Creditcoin3RuntimeProxyFilter,
    Creditcoin3RuntimeRuntime,
    EthbloomBloom,
    EthereumBlock,
    EthereumHeader,
    EthereumLog,
    EthereumReceiptEip658ReceiptData,
    EthereumReceiptReceiptV3,
    EthereumTransactionAccessListItem,
    EthereumTransactionEip1559Transaction,
    EthereumTransactionEip2930Transaction,
    EthereumTransactionLegacyTransaction,
    EthereumTransactionTransactionAction,
    EthereumTransactionTransactionSignature,
    EthereumTransactionTransactionV2,
    EthereumTypesHashH64,
    EvmCoreErrorExitError,
    EvmCoreErrorExitFatal,
    EvmCoreErrorExitReason,
    EvmCoreErrorExitRevert,
    EvmCoreErrorExitSucceed,
    FinalityGrandpaEquivocationPrecommit,
    FinalityGrandpaEquivocationPrevote,
    FinalityGrandpaPrecommit,
    FinalityGrandpaPrevote,
    FpRpcTransactionStatus,
    FrameSupportDispatchDispatchClass,
    FrameSupportDispatchDispatchInfo,
    FrameSupportDispatchPays,
    FrameSupportDispatchPerDispatchClassU32,
    FrameSupportDispatchPerDispatchClassWeight,
    FrameSupportDispatchPerDispatchClassWeightsPerClass,
    FrameSupportDispatchRawOrigin,
    FrameSupportPalletId,
    FrameSupportTokensMiscBalanceStatus,
    FrameSystemAccountInfo,
    FrameSystemCall,
    FrameSystemError,
    FrameSystemEvent,
    FrameSystemEventRecord,
    FrameSystemExtensionsCheckGenesis,
    FrameSystemExtensionsCheckNonZeroSender,
    FrameSystemExtensionsCheckNonce,
    FrameSystemExtensionsCheckSpecVersion,
    FrameSystemExtensionsCheckTxVersion,
    FrameSystemExtensionsCheckWeight,
    FrameSystemLastRuntimeUpgradeInfo,
    FrameSystemLimitsBlockLength,
    FrameSystemLimitsBlockWeights,
    FrameSystemLimitsWeightsPerClass,
    FrameSystemPhase,
    PalletBabeCall,
    PalletBabeError,
    PalletBagsListCall,
    PalletBagsListError,
    PalletBagsListEvent,
    PalletBagsListListBag,
    PalletBagsListListListError,
    PalletBagsListListNode,
    PalletBalancesAccountData,
    PalletBalancesBalanceLock,
    PalletBalancesCall,
    PalletBalancesError,
    PalletBalancesEvent,
    PalletBalancesIdAmount,
    PalletBalancesReasons,
    PalletBalancesReserveData,
    PalletBaseFeeCall,
    PalletBaseFeeEvent,
    PalletDynamicFeeCall,
    PalletEthereumCall,
    PalletEthereumError,
    PalletEthereumEvent,
    PalletEthereumRawOrigin,
    PalletEvmCall,
    PalletEvmCodeMetadata,
    PalletEvmError,
    PalletEvmEvent,
    PalletFastUnstakeCall,
    PalletFastUnstakeError,
    PalletFastUnstakeEvent,
    PalletFastUnstakeUnstakeRequest,
    PalletGrandpaCall,
    PalletGrandpaError,
    PalletGrandpaEvent,
    PalletGrandpaStoredPendingChange,
    PalletGrandpaStoredState,
    PalletHotfixSufficientsCall,
    PalletHotfixSufficientsError,
    PalletIdentityBitFlags,
    PalletIdentityCall,
    PalletIdentityError,
    PalletIdentityEvent,
    PalletIdentityIdentityField,
    PalletIdentityIdentityInfo,
    PalletIdentityJudgement,
    PalletIdentityRegistrarInfo,
    PalletIdentityRegistration,
    PalletImOnlineCall,
    PalletImOnlineError,
    PalletImOnlineEvent,
    PalletImOnlineHeartbeat,
    PalletImOnlineSr25519AppSr25519Public,
    PalletImOnlineSr25519AppSr25519Signature,
    PalletNominationPoolsBondExtra,
    PalletNominationPoolsBondedPoolInner,
    PalletNominationPoolsCall,
    PalletNominationPoolsClaimPermission,
    PalletNominationPoolsCommission,
    PalletNominationPoolsCommissionChangeRate,
    PalletNominationPoolsConfigOpAccountId32,
    PalletNominationPoolsConfigOpPerbill,
    PalletNominationPoolsConfigOpU128,
    PalletNominationPoolsConfigOpU32,
    PalletNominationPoolsDefensiveError,
    PalletNominationPoolsError,
    PalletNominationPoolsEvent,
    PalletNominationPoolsPoolMember,
    PalletNominationPoolsPoolRoles,
    PalletNominationPoolsPoolState,
    PalletNominationPoolsRewardPool,
    PalletNominationPoolsSubPools,
    PalletNominationPoolsUnbondPool,
    PalletOffencesEvent,
    PalletProxyAnnouncement,
    PalletProxyCall,
    PalletProxyError,
    PalletProxyEvent,
    PalletProxyProxyDefinition,
    PalletSessionCall,
    PalletSessionError,
    PalletSessionEvent,
    PalletStakingActiveEraInfo,
    PalletStakingEraRewardPoints,
    PalletStakingExposure,
    PalletStakingForcing,
    PalletStakingIndividualExposure,
    PalletStakingNominations,
    PalletStakingPalletCall,
    PalletStakingPalletConfigOpPerbill,
    PalletStakingPalletConfigOpPercent,
    PalletStakingPalletConfigOpU128,
    PalletStakingPalletConfigOpU32,
    PalletStakingPalletError,
    PalletStakingPalletEvent,
    PalletStakingRewardDestination,
    PalletStakingSlashingSlashingSpans,
    PalletStakingSlashingSpanRecord,
    PalletStakingStakingLedger,
    PalletStakingUnappliedSlash,
    PalletStakingUnlockChunk,
    PalletStakingValidatorPrefs,
    PalletSudoCall,
    PalletSudoError,
    PalletSudoEvent,
    PalletTimestampCall,
    PalletTransactionPaymentCall,
    PalletTransactionPaymentChargeTransactionPayment,
    PalletTransactionPaymentEvent,
    PalletTransactionPaymentReleases,
    PalletUtilityCall,
    PalletUtilityError,
    PalletUtilityEvent,
    SpArithmeticArithmeticError,
    SpConsensusBabeAllowedSlots,
    SpConsensusBabeAppPublic,
    SpConsensusBabeBabeEpochConfiguration,
    SpConsensusBabeDigestsNextConfigDescriptor,
    SpConsensusBabeDigestsPreDigest,
    SpConsensusBabeDigestsPrimaryPreDigest,
    SpConsensusBabeDigestsSecondaryPlainPreDigest,
    SpConsensusBabeDigestsSecondaryVRFPreDigest,
    SpConsensusGrandpaAppPublic,
    SpConsensusGrandpaAppSignature,
    SpConsensusGrandpaEquivocation,
    SpConsensusGrandpaEquivocationProof,
    SpConsensusSlotsEquivocationProof,
    SpCoreCryptoKeyTypeId,
    SpCoreEcdsaSignature,
    SpCoreEd25519Public,
    SpCoreEd25519Signature,
    SpCoreSr25519Public,
    SpCoreSr25519Signature,
    SpCoreSr25519VrfVrfSignature,
    SpCoreVoid,
    SpRuntimeDigest,
    SpRuntimeDigestDigestItem,
    SpRuntimeDispatchError,
    SpRuntimeHeader,
    SpRuntimeModuleError,
    SpRuntimeMultiSignature,
    SpRuntimeTokenError,
    SpRuntimeTransactionalError,
    SpSessionMembershipProof,
    SpStakingOffenceOffenceDetails,
    SpVersionRuntimeVersion,
    SpWeightsRuntimeDbWeight,
    SpWeightsWeightV2Weight,
} from '@polkadot/types/lookup';

declare module '@polkadot/types/types/registry' {
    interface InterfaceTypes {
        Creditcoin3RuntimeOpaqueSessionKeys: Creditcoin3RuntimeOpaqueSessionKeys;
        Creditcoin3RuntimeOriginCaller: Creditcoin3RuntimeOriginCaller;
        Creditcoin3RuntimeProxyFilter: Creditcoin3RuntimeProxyFilter;
        Creditcoin3RuntimeRuntime: Creditcoin3RuntimeRuntime;
        EthbloomBloom: EthbloomBloom;
        EthereumBlock: EthereumBlock;
        EthereumHeader: EthereumHeader;
        EthereumLog: EthereumLog;
        EthereumReceiptEip658ReceiptData: EthereumReceiptEip658ReceiptData;
        EthereumReceiptReceiptV3: EthereumReceiptReceiptV3;
        EthereumTransactionAccessListItem: EthereumTransactionAccessListItem;
        EthereumTransactionEip1559Transaction: EthereumTransactionEip1559Transaction;
        EthereumTransactionEip2930Transaction: EthereumTransactionEip2930Transaction;
        EthereumTransactionLegacyTransaction: EthereumTransactionLegacyTransaction;
        EthereumTransactionTransactionAction: EthereumTransactionTransactionAction;
        EthereumTransactionTransactionSignature: EthereumTransactionTransactionSignature;
        EthereumTransactionTransactionV2: EthereumTransactionTransactionV2;
        EthereumTypesHashH64: EthereumTypesHashH64;
        EvmCoreErrorExitError: EvmCoreErrorExitError;
        EvmCoreErrorExitFatal: EvmCoreErrorExitFatal;
        EvmCoreErrorExitReason: EvmCoreErrorExitReason;
        EvmCoreErrorExitRevert: EvmCoreErrorExitRevert;
        EvmCoreErrorExitSucceed: EvmCoreErrorExitSucceed;
        FinalityGrandpaEquivocationPrecommit: FinalityGrandpaEquivocationPrecommit;
        FinalityGrandpaEquivocationPrevote: FinalityGrandpaEquivocationPrevote;
        FinalityGrandpaPrecommit: FinalityGrandpaPrecommit;
        FinalityGrandpaPrevote: FinalityGrandpaPrevote;
        FpRpcTransactionStatus: FpRpcTransactionStatus;
        FrameSupportDispatchDispatchClass: FrameSupportDispatchDispatchClass;
        FrameSupportDispatchDispatchInfo: FrameSupportDispatchDispatchInfo;
        FrameSupportDispatchPays: FrameSupportDispatchPays;
        FrameSupportDispatchPerDispatchClassU32: FrameSupportDispatchPerDispatchClassU32;
        FrameSupportDispatchPerDispatchClassWeight: FrameSupportDispatchPerDispatchClassWeight;
        FrameSupportDispatchPerDispatchClassWeightsPerClass: FrameSupportDispatchPerDispatchClassWeightsPerClass;
        FrameSupportDispatchRawOrigin: FrameSupportDispatchRawOrigin;
        FrameSupportPalletId: FrameSupportPalletId;
        FrameSupportTokensMiscBalanceStatus: FrameSupportTokensMiscBalanceStatus;
        FrameSystemAccountInfo: FrameSystemAccountInfo;
        FrameSystemCall: FrameSystemCall;
        FrameSystemError: FrameSystemError;
        FrameSystemEvent: FrameSystemEvent;
        FrameSystemEventRecord: FrameSystemEventRecord;
        FrameSystemExtensionsCheckGenesis: FrameSystemExtensionsCheckGenesis;
        FrameSystemExtensionsCheckNonZeroSender: FrameSystemExtensionsCheckNonZeroSender;
        FrameSystemExtensionsCheckNonce: FrameSystemExtensionsCheckNonce;
        FrameSystemExtensionsCheckSpecVersion: FrameSystemExtensionsCheckSpecVersion;
        FrameSystemExtensionsCheckTxVersion: FrameSystemExtensionsCheckTxVersion;
        FrameSystemExtensionsCheckWeight: FrameSystemExtensionsCheckWeight;
        FrameSystemLastRuntimeUpgradeInfo: FrameSystemLastRuntimeUpgradeInfo;
        FrameSystemLimitsBlockLength: FrameSystemLimitsBlockLength;
        FrameSystemLimitsBlockWeights: FrameSystemLimitsBlockWeights;
        FrameSystemLimitsWeightsPerClass: FrameSystemLimitsWeightsPerClass;
        FrameSystemPhase: FrameSystemPhase;
        PalletBabeCall: PalletBabeCall;
        PalletBabeError: PalletBabeError;
        PalletBagsListCall: PalletBagsListCall;
        PalletBagsListError: PalletBagsListError;
        PalletBagsListEvent: PalletBagsListEvent;
        PalletBagsListListBag: PalletBagsListListBag;
        PalletBagsListListListError: PalletBagsListListListError;
        PalletBagsListListNode: PalletBagsListListNode;
        PalletBalancesAccountData: PalletBalancesAccountData;
        PalletBalancesBalanceLock: PalletBalancesBalanceLock;
        PalletBalancesCall: PalletBalancesCall;
        PalletBalancesError: PalletBalancesError;
        PalletBalancesEvent: PalletBalancesEvent;
        PalletBalancesIdAmount: PalletBalancesIdAmount;
        PalletBalancesReasons: PalletBalancesReasons;
        PalletBalancesReserveData: PalletBalancesReserveData;
        PalletBaseFeeCall: PalletBaseFeeCall;
        PalletBaseFeeEvent: PalletBaseFeeEvent;
        PalletDynamicFeeCall: PalletDynamicFeeCall;
        PalletEthereumCall: PalletEthereumCall;
        PalletEthereumError: PalletEthereumError;
        PalletEthereumEvent: PalletEthereumEvent;
        PalletEthereumRawOrigin: PalletEthereumRawOrigin;
        PalletEvmCall: PalletEvmCall;
        PalletEvmCodeMetadata: PalletEvmCodeMetadata;
        PalletEvmError: PalletEvmError;
        PalletEvmEvent: PalletEvmEvent;
        PalletFastUnstakeCall: PalletFastUnstakeCall;
        PalletFastUnstakeError: PalletFastUnstakeError;
        PalletFastUnstakeEvent: PalletFastUnstakeEvent;
        PalletFastUnstakeUnstakeRequest: PalletFastUnstakeUnstakeRequest;
        PalletGrandpaCall: PalletGrandpaCall;
        PalletGrandpaError: PalletGrandpaError;
        PalletGrandpaEvent: PalletGrandpaEvent;
        PalletGrandpaStoredPendingChange: PalletGrandpaStoredPendingChange;
        PalletGrandpaStoredState: PalletGrandpaStoredState;
        PalletHotfixSufficientsCall: PalletHotfixSufficientsCall;
        PalletHotfixSufficientsError: PalletHotfixSufficientsError;
        PalletIdentityBitFlags: PalletIdentityBitFlags;
        PalletIdentityCall: PalletIdentityCall;
        PalletIdentityError: PalletIdentityError;
        PalletIdentityEvent: PalletIdentityEvent;
        PalletIdentityIdentityField: PalletIdentityIdentityField;
        PalletIdentityIdentityInfo: PalletIdentityIdentityInfo;
        PalletIdentityJudgement: PalletIdentityJudgement;
        PalletIdentityRegistrarInfo: PalletIdentityRegistrarInfo;
        PalletIdentityRegistration: PalletIdentityRegistration;
        PalletImOnlineCall: PalletImOnlineCall;
        PalletImOnlineError: PalletImOnlineError;
        PalletImOnlineEvent: PalletImOnlineEvent;
        PalletImOnlineHeartbeat: PalletImOnlineHeartbeat;
        PalletImOnlineSr25519AppSr25519Public: PalletImOnlineSr25519AppSr25519Public;
        PalletImOnlineSr25519AppSr25519Signature: PalletImOnlineSr25519AppSr25519Signature;
        PalletNominationPoolsBondExtra: PalletNominationPoolsBondExtra;
        PalletNominationPoolsBondedPoolInner: PalletNominationPoolsBondedPoolInner;
        PalletNominationPoolsCall: PalletNominationPoolsCall;
        PalletNominationPoolsClaimPermission: PalletNominationPoolsClaimPermission;
        PalletNominationPoolsCommission: PalletNominationPoolsCommission;
        PalletNominationPoolsCommissionChangeRate: PalletNominationPoolsCommissionChangeRate;
        PalletNominationPoolsConfigOpAccountId32: PalletNominationPoolsConfigOpAccountId32;
        PalletNominationPoolsConfigOpPerbill: PalletNominationPoolsConfigOpPerbill;
        PalletNominationPoolsConfigOpU128: PalletNominationPoolsConfigOpU128;
        PalletNominationPoolsConfigOpU32: PalletNominationPoolsConfigOpU32;
        PalletNominationPoolsDefensiveError: PalletNominationPoolsDefensiveError;
        PalletNominationPoolsError: PalletNominationPoolsError;
        PalletNominationPoolsEvent: PalletNominationPoolsEvent;
        PalletNominationPoolsPoolMember: PalletNominationPoolsPoolMember;
        PalletNominationPoolsPoolRoles: PalletNominationPoolsPoolRoles;
        PalletNominationPoolsPoolState: PalletNominationPoolsPoolState;
        PalletNominationPoolsRewardPool: PalletNominationPoolsRewardPool;
        PalletNominationPoolsSubPools: PalletNominationPoolsSubPools;
        PalletNominationPoolsUnbondPool: PalletNominationPoolsUnbondPool;
        PalletOffencesEvent: PalletOffencesEvent;
        PalletProxyAnnouncement: PalletProxyAnnouncement;
        PalletProxyCall: PalletProxyCall;
        PalletProxyError: PalletProxyError;
        PalletProxyEvent: PalletProxyEvent;
        PalletProxyProxyDefinition: PalletProxyProxyDefinition;
        PalletSessionCall: PalletSessionCall;
        PalletSessionError: PalletSessionError;
        PalletSessionEvent: PalletSessionEvent;
        PalletStakingActiveEraInfo: PalletStakingActiveEraInfo;
        PalletStakingEraRewardPoints: PalletStakingEraRewardPoints;
        PalletStakingExposure: PalletStakingExposure;
        PalletStakingForcing: PalletStakingForcing;
        PalletStakingIndividualExposure: PalletStakingIndividualExposure;
        PalletStakingNominations: PalletStakingNominations;
        PalletStakingPalletCall: PalletStakingPalletCall;
        PalletStakingPalletConfigOpPerbill: PalletStakingPalletConfigOpPerbill;
        PalletStakingPalletConfigOpPercent: PalletStakingPalletConfigOpPercent;
        PalletStakingPalletConfigOpU128: PalletStakingPalletConfigOpU128;
        PalletStakingPalletConfigOpU32: PalletStakingPalletConfigOpU32;
        PalletStakingPalletError: PalletStakingPalletError;
        PalletStakingPalletEvent: PalletStakingPalletEvent;
        PalletStakingRewardDestination: PalletStakingRewardDestination;
        PalletStakingSlashingSlashingSpans: PalletStakingSlashingSlashingSpans;
        PalletStakingSlashingSpanRecord: PalletStakingSlashingSpanRecord;
        PalletStakingStakingLedger: PalletStakingStakingLedger;
        PalletStakingUnappliedSlash: PalletStakingUnappliedSlash;
        PalletStakingUnlockChunk: PalletStakingUnlockChunk;
        PalletStakingValidatorPrefs: PalletStakingValidatorPrefs;
        PalletSudoCall: PalletSudoCall;
        PalletSudoError: PalletSudoError;
        PalletSudoEvent: PalletSudoEvent;
        PalletTimestampCall: PalletTimestampCall;
        PalletTransactionPaymentCall: PalletTransactionPaymentCall;
        PalletTransactionPaymentChargeTransactionPayment: PalletTransactionPaymentChargeTransactionPayment;
        PalletTransactionPaymentEvent: PalletTransactionPaymentEvent;
        PalletTransactionPaymentReleases: PalletTransactionPaymentReleases;
        PalletUtilityCall: PalletUtilityCall;
        PalletUtilityError: PalletUtilityError;
        PalletUtilityEvent: PalletUtilityEvent;
        SpArithmeticArithmeticError: SpArithmeticArithmeticError;
        SpConsensusBabeAllowedSlots: SpConsensusBabeAllowedSlots;
        SpConsensusBabeAppPublic: SpConsensusBabeAppPublic;
        SpConsensusBabeBabeEpochConfiguration: SpConsensusBabeBabeEpochConfiguration;
        SpConsensusBabeDigestsNextConfigDescriptor: SpConsensusBabeDigestsNextConfigDescriptor;
        SpConsensusBabeDigestsPreDigest: SpConsensusBabeDigestsPreDigest;
        SpConsensusBabeDigestsPrimaryPreDigest: SpConsensusBabeDigestsPrimaryPreDigest;
        SpConsensusBabeDigestsSecondaryPlainPreDigest: SpConsensusBabeDigestsSecondaryPlainPreDigest;
        SpConsensusBabeDigestsSecondaryVRFPreDigest: SpConsensusBabeDigestsSecondaryVRFPreDigest;
        SpConsensusGrandpaAppPublic: SpConsensusGrandpaAppPublic;
        SpConsensusGrandpaAppSignature: SpConsensusGrandpaAppSignature;
        SpConsensusGrandpaEquivocation: SpConsensusGrandpaEquivocation;
        SpConsensusGrandpaEquivocationProof: SpConsensusGrandpaEquivocationProof;
        SpConsensusSlotsEquivocationProof: SpConsensusSlotsEquivocationProof;
        SpCoreCryptoKeyTypeId: SpCoreCryptoKeyTypeId;
        SpCoreEcdsaSignature: SpCoreEcdsaSignature;
        SpCoreEd25519Public: SpCoreEd25519Public;
        SpCoreEd25519Signature: SpCoreEd25519Signature;
        SpCoreSr25519Public: SpCoreSr25519Public;
        SpCoreSr25519Signature: SpCoreSr25519Signature;
        SpCoreSr25519VrfVrfSignature: SpCoreSr25519VrfVrfSignature;
        SpCoreVoid: SpCoreVoid;
        SpRuntimeDigest: SpRuntimeDigest;
        SpRuntimeDigestDigestItem: SpRuntimeDigestDigestItem;
        SpRuntimeDispatchError: SpRuntimeDispatchError;
        SpRuntimeHeader: SpRuntimeHeader;
        SpRuntimeModuleError: SpRuntimeModuleError;
        SpRuntimeMultiSignature: SpRuntimeMultiSignature;
        SpRuntimeTokenError: SpRuntimeTokenError;
        SpRuntimeTransactionalError: SpRuntimeTransactionalError;
        SpSessionMembershipProof: SpSessionMembershipProof;
        SpStakingOffenceOffenceDetails: SpStakingOffenceOffenceDetails;
        SpVersionRuntimeVersion: SpVersionRuntimeVersion;
        SpWeightsRuntimeDbWeight: SpWeightsRuntimeDbWeight;
        SpWeightsWeightV2Weight: SpWeightsWeightV2Weight;
    } // InterfaceTypes
} // declare module
