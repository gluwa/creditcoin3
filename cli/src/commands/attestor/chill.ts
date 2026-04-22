import { Command, OptionValues } from 'commander';
import { chainKeyOption, attestorAddressOption } from '../options';
import {
    getAttestorContractWithSigner,
    substrateAddressToBytes32,
    extractEvmError,
    ATTESTOR_STATUS_IDLE,
} from '../../lib/attestor/precompile';
import { getStringFromEnvVar } from '../../lib/account/keyring';
import { encodeAddress } from '@polkadot/util-crypto';

export function makeChillAttestorCommand() {
    const cmd = new Command('chill');
    cmd.description('Chill attestor');
    cmd.addOption(chainKeyOption.makeOptionMandatory());
    cmd.addOption(attestorAddressOption.makeOptionMandatory());
    cmd.action(chillAction);
    return cmd;
}

async function chillAction(options: OptionValues) {
    const chainKey = options.chain as string;
    const attestorSs58 = options.attestor as string;
    const attestorId32 = substrateAddressToBytes32(attestorSs58);

    const secret = getStringFromEnvVar(process.env.CC_SECRET);
    const { contract, stashAddress } = getAttestorContractWithSigner(secret, options);

    // Pre-call validation via view function
    const attestorInfo = await contract.getAttestor(BigInt(chainKey), attestorId32);
    if (!attestorInfo.exists) {
        console.log(`There is not attestor ${attestorSs58} for chain ${chainKey}`);
        process.exit(1);
    }

    // attestorInfo.stash is bytes32 — convert to SS58 to compare with stashAddress
    const attestorStashHex: string = attestorInfo.stash;
    const attestorStashBytes = Buffer.from(attestorStashHex.replace('0x', ''), 'hex');
    const attestorStashSs58 = encodeAddress(attestorStashBytes);

    if (attestorStashSs58 !== stashAddress) {
        console.log(`Attestor ${attestorSs58} is not owned by the keyring account ${stashAddress}`);
        process.exit(1);
    }

    if (attestorInfo.status === ATTESTOR_STATUS_IDLE) {
        console.log(`Attestor ${attestorSs58} is already chilled`);
        process.exit(1);
    }

    try {
        const tx = await contract.chill(BigInt(chainKey), attestorId32);
        const receipt = await tx.wait();
        if (receipt.status === 1) {
            console.log(`Transaction included at block (hash: ${receipt.blockHash})`);
            process.exit(0);
        } else {
            console.log('Transaction failed');
            process.exit(1);
        }
    } catch (error: unknown) {
        console.log(extractEvmError(error));
        process.exit(1);
    }
}
