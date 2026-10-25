import { Command, OptionValues } from 'commander';
import { attestorAddressOption, chainKeyOption } from '../options';
import {
    getAttestorContractWithSigner,
    substrateAddressToBytes32,
    extractEvmError,
} from '../../lib/attestor/precompile';
import { getSecretFromEnvOrPrompt } from '../../lib/account/keyring';

export function makeRegisterAttestorCommand() {
    const cmd = new Command('register');
    cmd.description('Register an attestor and bond funds from a stash account');
    cmd.addOption(attestorAddressOption.makeOptionMandatory());
    cmd.addOption(chainKeyOption.makeOptionMandatory());
    cmd.action(registerAttestorAction);
    return cmd;
}

async function registerAttestorAction(options: OptionValues) {
    const chainKey = options.chain as string;
    const attestorSs58 = options.attestor as string;
    const attestorId32 = substrateAddressToBytes32(attestorSs58);

    const secret = await getSecretFromEnvOrPrompt('CC_SECRET', 'caller', options);
    const { contract } = getAttestorContractWithSigner(secret, options);

    try {
        const tx = await contract.registerAttestor(BigInt(chainKey), attestorId32);
        const receipt = await tx.wait();
        if (receipt.status === 1) {
            console.log(`Transaction included at block (hash: ${receipt.blockHash})`);
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
