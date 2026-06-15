import { Command, OptionValues } from 'commander';
import { chainKeyOption, attestorAddressOption } from '../options';
import {
    getAttestorContractWithSigner,
    substrateAddressToBytes32,
    extractEvmError,
    ATTESTOR_STATUS_IDLE,
    ATTESTOR_STATUS_ACTIVE,
    ATTESTOR_STATUS_LEAVING,
} from '../../lib/attestor/precompile';
import { getSecretFromEnvOrPrompt } from '../../lib/account/keyring';
import { encodeAddress } from '@polkadot/util-crypto';
import { ethers } from 'ethers';

const POLL_MS = 500;
const MAX_WAIT_MS = 600_000;

function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitUntilAttestorIdle(
    contract: ethers.Contract,
    chainKey: string,
    attestorId32: string,
    attestorSs58: string,
): Promise<void> {
    const deadline = Date.now() + MAX_WAIT_MS;
    while (Date.now() < deadline) {
        const attestorInfo = await contract.getAttestor(BigInt(chainKey), attestorId32);
        if (attestorInfo.exists && BigInt(attestorInfo.status) === ATTESTOR_STATUS_IDLE) {
            return;
        }
        await sleep(POLL_MS);
    }
    console.log(
        `Timed out after ${MAX_WAIT_MS / 1000}s waiting for attestor ${attestorSs58} to reach Idle (fully chilled).`,
    );
    process.exit(1);
}

export function makeChillAttestorCommand() {
    const cmd = new Command('chill');
    cmd.description(
        'Chill an attestor: if active, the change applies at the next era boundary (same as activation); if still waiting to activate, chills immediately.',
    );
    cmd.addOption(chainKeyOption.makeOptionMandatory());
    cmd.addOption(attestorAddressOption.makeOptionMandatory());
    cmd.action(chillAction);
    return cmd;
}

async function chillAction(options: OptionValues) {
    const chainKey = options.chain as string;
    const attestorSs58 = options.attestor as string;
    const attestorId32 = substrateAddressToBytes32(attestorSs58);

    const secret = await getSecretFromEnvOrPrompt('CC_SECRET', 'caller', options);
    const { contract, stashAddress } = getAttestorContractWithSigner(secret, options);

    // Pre-call validation via view function
    const attestorInfo = await contract.getAttestor(BigInt(chainKey), attestorId32);
    if (!attestorInfo.exists) {
        console.error(`There is not attestor ${attestorSs58} for chain ${chainKey}`);
        process.exit(1);
    }

    // attestorInfo.stash is bytes32 — convert to SS58 to compare with stashAddress
    const attestorStashHex: string = attestorInfo.stash;
    const attestorStashBytes = Buffer.from(attestorStashHex.replace('0x', ''), 'hex');
    const attestorStashSs58 = encodeAddress(attestorStashBytes);

    if (attestorStashSs58 !== stashAddress) {
        console.error(`Attestor ${attestorSs58} is not owned by the keyring account ${stashAddress}`);
        process.exit(1);
    }

    const attestorStatus = BigInt(attestorInfo.status);

    if (attestorStatus === ATTESTOR_STATUS_IDLE) {
        console.error(`Attestor ${attestorSs58} is already chilled`);
        process.exit(1);
    }
    if (attestorStatus === ATTESTOR_STATUS_LEAVING) {
        console.error(
            `Attestor ${attestorSs58} already has a chill scheduled; wait for the next era boundary to complete.`,
        );
        process.exit(1);
    }

    const shouldWaitForIdle = attestorStatus === ATTESTOR_STATUS_ACTIVE;

    try {
        const tx = await contract.chill(BigInt(chainKey), attestorId32);
        const receipt = await tx.wait();
        if (receipt.status === 1) {
            console.log(`Transaction included at block (hash: ${receipt.blockHash})`);
            if (shouldWaitForIdle) {
                console.log('Waiting for era rotation so the attestor becomes fully chilled (Idle)...');
                await waitUntilAttestorIdle(contract, chainKey, attestorId32, attestorSs58);
                console.log('Attestor is now Idle.');
            }
            process.exit(0);
        } else {
            console.error('Transaction failed');
            process.exit(1);
        }
    } catch (error: unknown) {
        console.error(extractEvmError(error));
        process.exit(1);
    }
}
