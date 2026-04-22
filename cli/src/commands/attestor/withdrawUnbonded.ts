import { Command, OptionValues } from 'commander';
import {
    getAttestorContractWithSigner,
    substrateAddressToBytes32,
    deriveEvmKeyFromSecret,
    extractEvmError,
} from '../../lib/attestor/precompile';
import { getStringFromEnvVar } from '../../lib/account/keyring';

export function makeAttestorWithdrawUnbondedCommand() {
    const cmd = new Command('withdraw-unbonded');
    cmd.description(
        'Withdraw unbonded funds from attestor account that become available after calling unregisterAttestor',
    );
    cmd.action(withdrawUnbondedAction);
    return cmd;
}

async function withdrawUnbondedAction(options: OptionValues) {
    const secret = getStringFromEnvVar(process.env.CC_SECRET);
    const { stashAddress } = deriveEvmKeyFromSecret(secret);
    const stashBytes32 = substrateAddressToBytes32(stashAddress);

    const { contract } = getAttestorContractWithSigner(secret, options);

    // Check ledger to determine if there are any unlocking chunks
    const ledgerInfo = await contract.getLedger(stashBytes32);
    if (!ledgerInfo.exists) {
        console.log(`No unbonded funds to withdraw for address ${stashAddress}`);
        process.exit(0);
    }

    if (ledgerInfo.unlockingChunks === 0) {
        console.log('No unbonded funds to withdraw');
        process.exit(0);
    }

    console.log(`Unbonded funds available to withdraw (unlocking chunks: ${ledgerInfo.unlockingChunks})`);

    try {
        const tx = await contract.withdrawUnbonded();
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
