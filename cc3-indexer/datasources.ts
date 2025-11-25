import { SubstrateDatasourceKind, SubstrateHandlerKind, SubstrateRuntimeDatasource } from '@subql/types';
import { FrontierEvmDatasource } from '@subql/frontier-evm-processor';

export const genesisDatasource: SubstrateRuntimeDatasource = {
    kind: SubstrateDatasourceKind.Runtime,
    startBlock: 1,
    endBlock: 1,
    mapping: {
        file: './dist/index.js',
        handlers: [
            {
                kind: SubstrateHandlerKind.Block,
                handler: 'initiateStoreAndDatabase',
            },
        ],
    },
};

export const attestationDatasources: SubstrateRuntimeDatasource = {
    kind: SubstrateDatasourceKind.Runtime,
    startBlock: 1,
    mapping: {
        file: './dist/index.js',
        handlers: [
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventAttestorsElected',
                filter: {
                    module: 'attestation',
                    method: 'AttestorsElected',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventAttestorRegistered',
                filter: {
                    module: 'attestation',
                    method: 'AttestorRegistered',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventAttestorUnregistered',
                filter: {
                    module: 'attestation',
                    method: 'AttestorUnregistered',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventInvulnerableRegistered',
                filter: {
                    module: 'attestation',
                    method: 'InvulnerableRegistered',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventInvulnerableUnregistered',
                filter: {
                    module: 'attestation',
                    method: 'InvulnerableUnregistered',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventCheckpointReached',
                filter: {
                    module: 'attestation',
                    method: 'CheckpointReached',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventTargetSampleSizeChanged',
                filter: {
                    module: 'attestation',
                    method: 'TargetSampleSizeChanged',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventPendingTargetSampleSizeSet',
                filter: {
                    module: 'attestation',
                    method: 'PendingTargetSampleSizeSet',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventBonded',
                filter: {
                    module: 'attestation',
                    method: 'Bonded',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventUnbonded',
                filter: {
                    module: 'attestation',
                    method: 'Unbonded',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventWithdrawn',
                filter: {
                    module: 'attestation',
                    method: 'Withdrawn',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventAttestorActivated',
                filter: {
                    module: 'attestation',
                    method: 'AttestorActivated',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventAttestorChilled',
                filter: {
                    module: 'attestation',
                    method: 'AttestorChilled',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventMinBondRequirementUpdated',
                filter: {
                    module: 'attestation',
                    method: 'MinBondRequirementUpdated',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventCheckpointsCleared',
                filter: {
                    module: 'attestation',
                    method: 'CheckpointsCleared',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventClearedStorageForRemovedChain',
                filter: {
                    module: 'attestation',
                    method: 'ClearedStorageForRemovedChain',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventAttestationIntervalChanged',
                filter: {
                    module: 'attestation',
                    method: 'AttestationIntervalChanged',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventPendingAttestationIntervalSet',
                filter: {
                    module: 'attestation',
                    method: 'PendingAttestationIntervalSet',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventBlockAttested',
                filter: {
                    module: 'attestation',
                    method: 'BlockAttested',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleCheckpointIntervalChanged',
                filter: {
                    module: 'attestation',
                    method: 'CheckpointIntervalChanged',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleSupportedChainRegistered',
                filter: {
                    module: 'supportedChains',
                    method: 'ChainRegistered',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleSupportedChainRemoved',
                filter: {
                    module: 'supportedChains',
                    method: 'ChainRemoved',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleMaturityStrategySet',
                filter: {
                    module: 'supportedChains',
                    method: 'MaturityStrategySet',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleMaxAttestorsChanged',
                filter: {
                    module: 'attestation',
                    method: 'MaxAttestorsChanged',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventVoteAcceptanceWindowChanged',
                filter: {
                    module: 'attestation',
                    method: 'VoteAcceptanceWindowChanged',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleAttestorElectionPolicyChanged',
                filter: {
                    module: 'attestation',
                    method: 'ChangedElectionPolicy',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleAuthorizedAttestorAdded',
                filter: {
                    module: 'attestation',
                    method: 'AuthorizedAttestorAdded',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleAuthorizedAttestorRemoved',
                filter: {
                    module: 'attestation',
                    method: 'AuthorizedAttestorRemoved',
                },
            },
        ],
    },
};

export const nativeQueryVerifierDatasource: FrontierEvmDatasource = {
    // Frontier EVM Processor for Native Query Verifier Precompile
    kind: 'substrate/FrontierEvm',
    startBlock: 1,
    processor: {
        file: './node_modules/@subql/frontier-evm-processor/dist/bundle.js',
        options: {
            abi: 'nativeQueryVerifier',
            // The precompile is at address 0x0FD2
            address: '0x0000000000000000000000000000000000000fd2',
        },
    },
    assets: new Map([['nativeQueryVerifier', { file: './abis/nativeQueryVerifier.abi.json' }]]),
    mapping: {
        file: './dist/index.js',
        handlers: [
            {
                handler: 'handleTransactionVerified',
                kind: 'substrate/FrontierEvmEvent',
                filter: {
                    topics: ['TransactionVerified(uint64,uint64,uint64)'],
                },
            },
        ],
    },
};
