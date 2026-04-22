import { Command, OptionValues } from 'commander';
import { attestorAddressOption, chainKeyOption } from '../options';
import {
    getAttestorContractWithSigner,
    substrateAddressToBytes32,
    extractEvmError,
    ATTESTOR_STATUS_ACTIVE,
} from '../../lib/attestor/precompile';
import { getStringFromEnvVar } from '../../lib/account/keyring';

export function makeUnregisterAttestorCommand() {
    const cmd = new Command('unregister');
    cmd.description('Unregister attestor and unbond funds from a stash account');
    cmd.addOption(attestorAddressOption.makeOptionMandatory());
    cmd.addOption(chainKeyOption.makeOptionMandatory());
    cmd.action(unregisterAttestorAction);
    return cmd;
}

async function unregisterAttestorAction(options: OptionValues) {
    const chainKey = options.chain as string;
    const attestorSs58 = options.attestor as string;
    const attestorId32 = substrateAddressToBytes32(attestorSs58);

    const secret = getStringFromEnvVar(process.env.CC_SECRET);
    const { contract } = getAttestorContractWithSigner(secret, options);

    // Pre-call validation via view function
    const attestorInfo = await contract.getAttestor(BigInt(chainKey), attestorId32);
    if (!attestorInfo.exists) {
        console.log(`Address ${attestorSs58} is not an attestor`);
        process.exit(1);
    }

    if (attestorInfo.status === ATTESTOR_STATUS_ACTIVE) {
        console.log(`Address ${attestorSs58} status is Active. Please chill the attestor first`);
        process.exit(1);
    }
    console.log(`Address ${attestorSs58} status is Chill`);
    console.log(`Calling unregister attestor for ${attestorSs58} on chain ${chainKey}`);

    try {
        const tx = await contract.unregisterAttestor(BigInt(chainKey), attestorId32);
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
