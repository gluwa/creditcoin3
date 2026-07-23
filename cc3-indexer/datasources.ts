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
                handler: 'handleEventForwardCheckpointPatchApplied',
                filter: {
                    module: 'attestation',
                    method: 'ForwardCheckpointPatchApplied',
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
                kind: SubstrateHandlerKind.Call,
                handler: 'handleCallAttestorChill',
                filter: {
                    module: 'attestation',
                    method: 'chill',
                    success: true,
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
                kind: SubstrateHandlerKind.Call,
                handler: 'handleCallCommitAttestation',
                filter: {
                    module: 'attestation',
                    method: 'commitAttestation',
                    success: true,
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
                // USC write-ability: on-chain factory registration. The handler spins up a dynamic
                // datasource for the registered factory (no address is configured anywhere).
                kind: SubstrateHandlerKind.Event,
                handler: 'handleOutboxFactoryRegistered',
                filter: {
                    module: 'supportedChains',
                    method: 'OutboxFactoryRegistered',
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
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleForcedElection',
                filter: {
                    module: 'attestation',
                    method: 'ForcedElection',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventRevertedAttestationChainTo',
                filter: {
                    module: 'attestation',
                    method: 'RevertedAttestationChainTo',
                },
            },
        ],
    },
};

export const blockProverDatasource: FrontierEvmDatasource = {
    // Frontier EVM Processor for Native Query Verifier Precompile
    kind: 'substrate/FrontierEvm',
    startBlock: 1,
    processor: {
        file: './node_modules/@subql/frontier-evm-processor/dist/bundle.js',
        options: {
            abi: 'block_prover',
            // The precompile is at address 0x0FD2
            address: '0x0000000000000000000000000000000000000fd2',
        },
    },
    assets: new Map([['block_prover', { file: './abis/block_prover.json' }]]),
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

// USC write-ability — fully on-chain discovery, no configured addresses:
//
//   OutboxCreated (EVM, chain-wide topic filter — the static datasource below)
//     └─▶ createDynamicDatasource('Outbox', { address })   // watches each created Outbox
//           └─ MessagePublished / MessageAcknowledged (EVM)
//
// Discovery watches `OutboxCreated` across all contracts by topic (no address), rather than
// following the substrate OutboxFactoryRegistered event to the factory. This is deliberate: the
// deploy flow calls the factory's `createOutbox` (emitting OutboxCreated) *before* it registers the
// factory with the pallet, so a datasource that only started once the factory was registered would
// miss the already-emitted OutboxCreated and index nothing. A chain-wide topic watch from block 1
// is immune to that ordering. (The substrate OutboxFactoryRegistered handler still records the
// OutboxFactory entity for display; it is no longer on the discovery path.)
//
// createDynamicDatasource spreads its `args` into the 'Outbox' template's `processor.options` (see
// @subql/node BlockchainService.updateDynamicDs), so `{ address }` binds each instance to its
// Outbox while inheriting the template's abi + handlers. Only our OutboxFactory emits this exact
// event signature, so the address-less filter yields only real Outbox creations.

type FrontierEvmTemplate = Omit<FrontierEvmDatasource, 'startBlock' | 'endBlock'> & { name: string };

export const outboxDiscoveryDatasource: FrontierEvmDatasource = {
    kind: 'substrate/FrontierEvm',
    startBlock: 1,
    processor: {
        file: './node_modules/@subql/frontier-evm-processor/dist/bundle.js',
        options: {
            // No `address`: match OutboxCreated by topic across all contracts.
            abi: 'outbox_factory',
        },
    },
    assets: new Map([['outbox_factory', { file: './abis/outbox_factory.json' }]]),
    mapping: {
        file: './dist/index.js',
        handlers: [
            {
                handler: 'handleOutboxCreated',
                kind: 'substrate/FrontierEvmEvent',
                filter: {
                    topics: ['OutboxCreated(bytes32,address)'],
                },
            },
        ],
    },
};

export const outboxTemplate: FrontierEvmTemplate = {
    name: 'Outbox',
    kind: 'substrate/FrontierEvm',
    processor: {
        file: './node_modules/@subql/frontier-evm-processor/dist/bundle.js',
        options: {
            abi: 'outbox',
        },
    },
    assets: new Map([['outbox', { file: './abis/outbox.json' }]]),
    mapping: {
        file: './dist/index.js',
        handlers: [
            {
                handler: 'handleMessagePublished',
                kind: 'substrate/FrontierEvmEvent',
                filter: {
                    topics: ['MessagePublished(bytes32,bytes32,bool,bytes)'],
                },
            },
            {
                handler: 'handleMessageAcknowledged',
                kind: 'substrate/FrontierEvmEvent',
                filter: {
                    topics: ['MessageAcknowledged(bytes32)'],
                },
            },
        ],
    },
};
