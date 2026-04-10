// Auto-generated via `yarn polkadot-types-from-defs`, do not edit
/* eslint-disable */

// import type lookup before we augment - in some environments
// this is required to allow for ambient/previous definitions
import '@polkadot/types/types/registry';

import type {
    AttestorPrimitivesAttestationCheckpoint,
    AttestorPrimitivesAttestationData,
    AttestorPrimitivesAttestor,
    AttestorPrimitivesAttestorStatus,
    AttestorPrimitivesBlockContinuityProof,
    AttestorPrimitivesChainEncodingVersion,
    AttestorPrimitivesSignedAttestation,
    Creditcoin3RuntimeOpaqueSessionKeys,
    Creditcoin3RuntimeOriginCaller,
    Creditcoin3RuntimeProxyFilter,
    Creditcoin3RuntimeRuntime,
    Creditcoin3RuntimeRuntimeFreezeReason,
    Creditcoin3RuntimeRuntimeHoldReason,
    EthbloomBloom,
    EthereumBlock,
    EthereumHeader,
    EthereumLog,
    EthereumReceiptEip658ReceiptData,
    EthereumReceiptReceiptV4,
    EthereumTransactionEip1559Eip1559Transaction,
    EthereumTransactionEip2930AccessListItem,
    EthereumTransactionEip2930Eip2930Transaction,
    EthereumTransactionEip2930MalleableTransactionSignature,
    EthereumTransactionEip2930TransactionSignature,
    EthereumTransactionEip7702AuthorizationListItem,
    EthereumTransactionEip7702Eip7702Transaction,
    EthereumTransactionLegacyLegacyTransaction,
    EthereumTransactionLegacyTransactionAction,
    EthereumTransactionLegacyTransactionSignature,
    EthereumTransactionTransactionV3,
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
    FrameSupportDispatchPays,
    FrameSupportDispatchPerDispatchClassU32,
    FrameSupportDispatchPerDispatchClassWeight,
    FrameSupportDispatchPerDispatchClassWeightsPerClass,
    FrameSupportDispatchRawOrigin,
    FrameSupportPalletId,
    FrameSupportStorageNoDrop,
    FrameSupportTokensFungibleImbalance,
    FrameSupportTokensMiscBalanceStatus,
    FrameSupportTokensMiscIdAmountRuntimeFreezeReason,
    FrameSupportTokensMiscIdAmountRuntimeHoldReason,
    FrameSystemAccountInfo,
    FrameSystemCall,
    FrameSystemCodeUpgradeAuthorization,
    FrameSystemDispatchEventInfo,
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
    PalletAttestationPocAttestorElectionPolicy,
    PalletAttestationPocCall,
    PalletAttestationPocClearOrRevertCheckpointPruningState,
    PalletAttestationPocError,
    PalletAttestationPocEvent,
    PalletAttestationPocLedgerAttestorLedger,
    PalletAttestationPocLedgerUnlockChunk,
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
    PalletBalancesUnexpectedKind,
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
    PalletIdentityProvider,
    PalletIdentityRegistrarInfo,
    PalletIdentityRegistration,
    PalletIdentityUsernameInformation,
    PalletImOnlineCall,
    PalletImOnlineError,
    PalletImOnlineEvent,
    PalletImOnlineHeartbeat,
    PalletImOnlineSr25519AppSr25519Public,
    PalletImOnlineSr25519AppSr25519Signature,
    PalletMembershipCall,
    PalletMembershipError,
    PalletMembershipEvent,
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
    PalletProxyDepositKind,
    PalletProxyError,
    PalletProxyEvent,
    PalletProxyProxyDefinition,
    PalletRandomnessCall,
    PalletRandomnessError,
    PalletRandomnessEvent,
    PalletSessionCall,
    PalletSessionError,
    PalletSessionEvent,
    PalletSessionHistoricalPalletEvent,
    PalletSessionHoldReason,
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
    PalletStakingPalletHoldReason,
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
    PalletSupportedChainsCall,
    PalletSupportedChainsError,
    PalletSupportedChainsEvent,
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
    SpRuntimeProvingTrieTrieError,
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
    SupportedChainsPrimitivesSupportedChain,
} from '@polkadot/types/lookup';

declare module '@polkadot/types/types/registry' {
    interface InterfaceTypes {
        AttestorPrimitivesAttestationCheckpoint: AttestorPrimitivesAttestationCheckpoint;
        AttestorPrimitivesAttestationData: AttestorPrimitivesAttestationData;
        AttestorPrimitivesAttestor: AttestorPrimitivesAttestor;
        AttestorPrimitivesAttestorStatus: AttestorPrimitivesAttestorStatus;
        AttestorPrimitivesBlockContinuityProof: AttestorPrimitivesBlockContinuityProof;
        AttestorPrimitivesChainEncodingVersion: AttestorPrimitivesChainEncodingVersion;
        AttestorPrimitivesSignedAttestation: AttestorPrimitivesSignedAttestation;
        Creditcoin3RuntimeOpaqueSessionKeys: Creditcoin3RuntimeOpaqueSessionKeys;
        Creditcoin3RuntimeOriginCaller: Creditcoin3RuntimeOriginCaller;
        Creditcoin3RuntimeProxyFilter: Creditcoin3RuntimeProxyFilter;
        Creditcoin3RuntimeRuntime: Creditcoin3RuntimeRuntime;
        Creditcoin3RuntimeRuntimeFreezeReason: Creditcoin3RuntimeRuntimeFreezeReason;
        Creditcoin3RuntimeRuntimeHoldReason: Creditcoin3RuntimeRuntimeHoldReason;
        EthbloomBloom: EthbloomBloom;
        EthereumBlock: EthereumBlock;
        EthereumHeader: EthereumHeader;
        EthereumLog: EthereumLog;
        EthereumReceiptEip658ReceiptData: EthereumReceiptEip658ReceiptData;
        EthereumReceiptReceiptV4: EthereumReceiptReceiptV4;
        EthereumTransactionEip1559Eip1559Transaction: EthereumTransactionEip1559Eip1559Transaction;
        EthereumTransactionEip2930AccessListItem: EthereumTransactionEip2930AccessListItem;
        EthereumTransactionEip2930Eip2930Transaction: EthereumTransactionEip2930Eip2930Transaction;
        EthereumTransactionEip2930MalleableTransactionSignature: EthereumTransactionEip2930MalleableTransactionSignature;
        EthereumTransactionEip2930TransactionSignature: EthereumTransactionEip2930TransactionSignature;
        EthereumTransactionEip7702AuthorizationListItem: EthereumTransactionEip7702AuthorizationListItem;
        EthereumTransactionEip7702Eip7702Transaction: EthereumTransactionEip7702Eip7702Transaction;
        EthereumTransactionLegacyLegacyTransaction: EthereumTransactionLegacyLegacyTransaction;
        EthereumTransactionLegacyTransactionAction: EthereumTransactionLegacyTransactionAction;
        EthereumTransactionLegacyTransactionSignature: EthereumTransactionLegacyTransactionSignature;
        EthereumTransactionTransactionV3: EthereumTransactionTransactionV3;
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
        FrameSupportDispatchPays: FrameSupportDispatchPays;
        FrameSupportDispatchPerDispatchClassU32: FrameSupportDispatchPerDispatchClassU32;
        FrameSupportDispatchPerDispatchClassWeight: FrameSupportDispatchPerDispatchClassWeight;
        FrameSupportDispatchPerDispatchClassWeightsPerClass: FrameSupportDispatchPerDispatchClassWeightsPerClass;
        FrameSupportDispatchRawOrigin: FrameSupportDispatchRawOrigin;
        FrameSupportPalletId: FrameSupportPalletId;
        FrameSupportStorageNoDrop: FrameSupportStorageNoDrop;
        FrameSupportTokensFungibleImbalance: FrameSupportTokensFungibleImbalance;
        FrameSupportTokensMiscBalanceStatus: FrameSupportTokensMiscBalanceStatus;
        FrameSupportTokensMiscIdAmountRuntimeFreezeReason: FrameSupportTokensMiscIdAmountRuntimeFreezeReason;
        FrameSupportTokensMiscIdAmountRuntimeHoldReason: FrameSupportTokensMiscIdAmountRuntimeHoldReason;
        FrameSystemAccountInfo: FrameSystemAccountInfo;
        FrameSystemCall: FrameSystemCall;
        FrameSystemCodeUpgradeAuthorization: FrameSystemCodeUpgradeAuthorization;
        FrameSystemDispatchEventInfo: FrameSystemDispatchEventInfo;
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
        PalletAttestationPocAttestorElectionPolicy: PalletAttestationPocAttestorElectionPolicy;
        PalletAttestationPocCall: PalletAttestationPocCall;
        PalletAttestationPocClearOrRevertCheckpointPruningState: PalletAttestationPocClearOrRevertCheckpointPruningState;
        PalletAttestationPocError: PalletAttestationPocError;
        PalletAttestationPocEvent: PalletAttestationPocEvent;
        PalletAttestationPocLedgerAttestorLedger: PalletAttestationPocLedgerAttestorLedger;
        PalletAttestationPocLedgerUnlockChunk: PalletAttestationPocLedgerUnlockChunk;
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
        PalletBalancesUnexpectedKind: PalletBalancesUnexpectedKind;
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
        PalletIdentityProvider: PalletIdentityProvider;
        PalletIdentityRegistrarInfo: PalletIdentityRegistrarInfo;
        PalletIdentityRegistration: PalletIdentityRegistration;
        PalletIdentityUsernameInformation: PalletIdentityUsernameInformation;
        PalletImOnlineCall: PalletImOnlineCall;
        PalletImOnlineError: PalletImOnlineError;
        PalletImOnlineEvent: PalletImOnlineEvent;
        PalletImOnlineHeartbeat: PalletImOnlineHeartbeat;
        PalletImOnlineSr25519AppSr25519Public: PalletImOnlineSr25519AppSr25519Public;
        PalletImOnlineSr25519AppSr25519Signature: PalletImOnlineSr25519AppSr25519Signature;
        PalletMembershipCall: PalletMembershipCall;
        PalletMembershipError: PalletMembershipError;
        PalletMembershipEvent: PalletMembershipEvent;
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
        PalletProxyDepositKind: PalletProxyDepositKind;
        PalletProxyError: PalletProxyError;
        PalletProxyEvent: PalletProxyEvent;
        PalletProxyProxyDefinition: PalletProxyProxyDefinition;
        PalletRandomnessCall: PalletRandomnessCall;
        PalletRandomnessError: PalletRandomnessError;
        PalletRandomnessEvent: PalletRandomnessEvent;
        PalletSessionCall: PalletSessionCall;
        PalletSessionError: PalletSessionError;
        PalletSessionEvent: PalletSessionEvent;
        PalletSessionHistoricalPalletEvent: PalletSessionHistoricalPalletEvent;
        PalletSessionHoldReason: PalletSessionHoldReason;
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
        PalletStakingPalletHoldReason: PalletStakingPalletHoldReason;
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
        PalletSupportedChainsCall: PalletSupportedChainsCall;
        PalletSupportedChainsError: PalletSupportedChainsError;
        PalletSupportedChainsEvent: PalletSupportedChainsEvent;
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
        SpRuntimeProvingTrieTrieError: SpRuntimeProvingTrieTrieError;
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
        SupportedChainsPrimitivesSupportedChain: SupportedChainsPrimitivesSupportedChain;
    } // InterfaceTypes
} // declare module
