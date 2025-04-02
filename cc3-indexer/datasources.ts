import { SubstrateDatasourceKind, SubstrateHandlerKind, SubstrateRuntimeDatasource } from '@subql/types';
import { FrontierEvmDatasource } from "@subql/frontier-evm-processor";

export const genesisDatasource: SubstrateRuntimeDatasource = {
    kind: SubstrateDatasourceKind.Runtime,
    startBlock: 1,
    endBlock: 1,
    mapping: {
        file: './dist/index.js',
        handlers: [{
            kind: SubstrateHandlerKind.Block,
            handler: "initiateStoreAndDatabase"
        }]
    }
}

export const attestationDatasources: SubstrateRuntimeDatasource =
{
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
                handler: 'handleEventRewardClaimed',
                filter: {
                    module: 'attestation',
                    method: 'RewardClaimed',
                },
            },
            {
                kind: SubstrateHandlerKind.Event,
                handler: 'handleEventRewardPaid',
                filter: {
                    module: 'attestation',
                    method: 'RewardPaid',
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
                handler: 'handleEventChainRewardUpdated',
                filter: {
                    module: 'attestation',
                    method: 'ChainRewardUpdated',
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
            }
        ],
    },
}

export const proverDatasource: FrontierEvmDatasource = {
    // Frontier EVM Processor
    kind: "substrate/FrontierEvm",
    startBlock: 1,
    processor: {
        file: "./node_modules/@subql/frontier-evm-processor/dist/bundle.js",
        options: {
            abi: "prover",
        },
    },
    assets: new Map([["prover", { file: "./abis/prover.abi.json" }]]),
    mapping: {
        file: "./dist/index.js",
        handlers: [
            {
                handler: "handleProverDeployed",
                kind: "substrate/FrontierEvmEvent",
                filter: {
                    topics: [
                        "ProverDeployed(address indexed contractAddress,address indexed owner,address proceedsAccount,uint256 costPerByte,uint256 baseFee,uint64 chainKey,string displayName,uint64 timeout)",
                    ],
                },
            },
            {
                handler: "handleQuerySubmitted",
                kind: "substrate/FrontierEvmEvent",
                filter: {
                    topics: [
                        "QuerySubmitted(bytes32,uint256,uint256,(uint64,uint64,uint64,(uint64,uint64)[]))",
                    ],
                }
            },
            {
                handler: "handleQueryProofVerified",
                kind: "substrate/FrontierEvmEvent",
                filter: {
                    topics: [
                        "QueryProofVerified(bytes32,(uint256,bytes)[],uint8)",
                    ],
                }
            },
            {
                handler: "handleEscrowedPaymentReclaimed",
                kind: "substrate/FrontierEvmEvent",
                filter: {
                    topics: [
                        "EscrowedPaymentReclaimed(bytes32,uint256)",
                    ],
                }
            },
            {
                handler: "handleProceedsWithdrawn",
                kind: "substrate/FrontierEvmEvent",
                filter: {
                    topics: [
                        "ProceedsWithdrawn(address indexed proceedsAccount,uint256 amount)",
                    ],
                }
            },
            {
                handler: "handleUpdateCostPerByte",
                kind: "substrate/FrontierEvmEvent",
                filter: {
                    topics: [
                        "CostPerByteUpdated(uint256 newCostPerByte)",
                    ],
                }
            },
            {
                handler: "handleUpdateBaseFee",
                kind: "substrate/FrontierEvmEvent",
                filter: {
                    topics: [
                        "BaseFeeUpdated(uint256 newBaseFee)",
                    ],
                }
            },
        ]
    },
}
