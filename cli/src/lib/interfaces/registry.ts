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
    Creditcoin3RuntimeRuntimeFreezeReason,
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
    FrameMetadataHashExtensionCheckMetadataHash,
    FrameMetadataHashExtensionMode,
    FrameSupportDispatchDispatchClass,
    FrameSupportDispatchDispatchInfo,
    FrameSupportDispatchPays,
    FrameSupportDispatchPerDispatchClassU32,
    FrameSupportDispatchPerDispatchClassWeight,
    FrameSupportDispatchPerDispatchClassWeightsPerClass,
    FrameSupportDispatchRawOrigin,
    FrameSupportPalletId,
    FrameSupportTokensMiscBalanceStatus,
    FrameSupportTokensMiscIdAmount,
    FrameSystemAccountInfo,
    FrameSystemCall,
    FrameSystemCodeUpgradeAuthorization,
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
    PalletBalancesAdjustmentDirection,
    PalletBalancesBalanceLock,
    PalletBalancesCall,
    PalletBalancesError,
    PalletBalancesEvent,
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
    PalletIdentityAuthorityProperties,
    PalletIdentityCall,
    PalletIdentityError,
    PalletIdentityEvent,
    PalletIdentityJudgement,
    PalletIdentityLegacyIdentityInfo,
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
    PalletNominationPoolsCommissionClaimPermission,
    PalletNominationPoolsConfigOpAccountId32,
    PalletNominationPoolsConfigOpPerbill,
    PalletNominationPoolsConfigOpU128,
    PalletNominationPoolsConfigOpU32,
    PalletNominationPoolsDefensiveError,
    PalletNominationPoolsError,
    PalletNominationPoolsEvent,
    PalletNominationPoolsFreezeReason,
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
    PalletStakingForcing,
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
    SpCoreSr25519VrfVrfSignature,
    SpRuntimeDigest,
    SpRuntimeDigestDigestItem,
    SpRuntimeDispatchError,
    SpRuntimeHeader,
    SpRuntimeModuleError,
    SpRuntimeMultiSignature,
    SpRuntimeTokenError,
    SpRuntimeTransactionalError,
    SpSessionMembershipProof,
    SpStakingExposure,
    SpStakingExposurePage,
    SpStakingIndividualExposure,
    SpStakingOffenceOffenceDetails,
    SpStakingPagedExposureMetadata,
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
        Creditcoin3RuntimeRuntimeFreezeReason: Creditcoin3RuntimeRuntimeFreezeReason;
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
        FrameMetadataHashExtensionCheckMetadataHash: FrameMetadataHashExtensionCheckMetadataHash;
        FrameMetadataHashExtensionMode: FrameMetadataHashExtensionMode;
        FrameSupportDispatchDispatchClass: FrameSupportDispatchDispatchClass;
        FrameSupportDispatchDispatchInfo: FrameSupportDispatchDispatchInfo;
        FrameSupportDispatchPays: FrameSupportDispatchPays;
        FrameSupportDispatchPerDispatchClassU32: FrameSupportDispatchPerDispatchClassU32;
        FrameSupportDispatchPerDispatchClassWeight: FrameSupportDispatchPerDispatchClassWeight;
        FrameSupportDispatchPerDispatchClassWeightsPerClass: FrameSupportDispatchPerDispatchClassWeightsPerClass;
        FrameSupportDispatchRawOrigin: FrameSupportDispatchRawOrigin;
        FrameSupportPalletId: FrameSupportPalletId;
        FrameSupportTokensMiscBalanceStatus: FrameSupportTokensMiscBalanceStatus;
        FrameSupportTokensMiscIdAmount: FrameSupportTokensMiscIdAmount;
        FrameSystemAccountInfo: FrameSystemAccountInfo;
        FrameSystemCall: FrameSystemCall;
        FrameSystemCodeUpgradeAuthorization: FrameSystemCodeUpgradeAuthorization;
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
        PalletBalancesAdjustmentDirection: PalletBalancesAdjustmentDirection;
        PalletBalancesBalanceLock: PalletBalancesBalanceLock;
        PalletBalancesCall: PalletBalancesCall;
        PalletBalancesError: PalletBalancesError;
        PalletBalancesEvent: PalletBalancesEvent;
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
        PalletIdentityAuthorityProperties: PalletIdentityAuthorityProperties;
        PalletIdentityCall: PalletIdentityCall;
        PalletIdentityError: PalletIdentityError;
        PalletIdentityEvent: PalletIdentityEvent;
        PalletIdentityJudgement: PalletIdentityJudgement;
        PalletIdentityLegacyIdentityInfo: PalletIdentityLegacyIdentityInfo;
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
        PalletNominationPoolsCommissionClaimPermission: PalletNominationPoolsCommissionClaimPermission;
        PalletNominationPoolsConfigOpAccountId32: PalletNominationPoolsConfigOpAccountId32;
        PalletNominationPoolsConfigOpPerbill: PalletNominationPoolsConfigOpPerbill;
        PalletNominationPoolsConfigOpU128: PalletNominationPoolsConfigOpU128;
        PalletNominationPoolsConfigOpU32: PalletNominationPoolsConfigOpU32;
        PalletNominationPoolsDefensiveError: PalletNominationPoolsDefensiveError;
        PalletNominationPoolsError: PalletNominationPoolsError;
        PalletNominationPoolsEvent: PalletNominationPoolsEvent;
        PalletNominationPoolsFreezeReason: PalletNominationPoolsFreezeReason;
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
        PalletStakingForcing: PalletStakingForcing;
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
        SpCoreSr25519VrfVrfSignature: SpCoreSr25519VrfVrfSignature;
        SpRuntimeDigest: SpRuntimeDigest;
        SpRuntimeDigestDigestItem: SpRuntimeDigestDigestItem;
        SpRuntimeDispatchError: SpRuntimeDispatchError;
        SpRuntimeHeader: SpRuntimeHeader;
        SpRuntimeModuleError: SpRuntimeModuleError;
        SpRuntimeMultiSignature: SpRuntimeMultiSignature;
        SpRuntimeTokenError: SpRuntimeTokenError;
        SpRuntimeTransactionalError: SpRuntimeTransactionalError;
        SpSessionMembershipProof: SpSessionMembershipProof;
        SpStakingExposure: SpStakingExposure;
        SpStakingExposurePage: SpStakingExposurePage;
        SpStakingIndividualExposure: SpStakingIndividualExposure;
        SpStakingOffenceOffenceDetails: SpStakingOffenceOffenceDetails;
        SpStakingPagedExposureMetadata: SpStakingPagedExposureMetadata;
        SpVersionRuntimeVersion: SpVersionRuntimeVersion;
        SpWeightsRuntimeDbWeight: SpWeightsRuntimeDbWeight;
        SpWeightsWeightV2Weight: SpWeightsWeightV2Weight;
    } // InterfaceTypes
} // declare module
