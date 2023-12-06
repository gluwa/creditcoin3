// Create the send command for the EVM subcommand
//
// Path: cli/src/commands/evm/send.ts

import { Command, OptionValues } from 'commander';
import { ethers } from 'ethers';
import { initEVMCallerWallet } from '../../lib/evm/wallet';
import { parseAmountOrExit, parseEVMAddressOrExit, requiredInput } from '../../lib/parsing';
import { getEvmUrl } from '../../lib/evm/rpc';

export function makeEvmSendCommand() {
    const cmd = new Command('send');
    cmd.description('Send funds from an EVM account to another EVM account');
    cmd.option('-a, --amount [amount]', 'Amount to send');
    cmd.option('-t, --to [to]', 'Specify recipient address');
    cmd.option('--use-ecdsa', 'Use ECDSA private key instead of seed phrase');
    cmd.action(evmSendAction);
    return cmd;
}

async function evmSendAction(options: OptionValues) {
    const wallet = await initEVMCallerWallet(options);
    const { amount, recipient } = parseOptions(options);
    const signer = wallet.connect(new ethers.JsonRpcProvider(getEvmUrl(options)));
    const tx = await signer.sendTransaction({
        to: recipient,
        value: amount.toString(),
    });

    const txReceipt = await tx.wait();
    // Check if txReceipt is not null and then log information
    if (txReceipt) {
        console.log(`Transaction hash: ${tx.hash}`);
        console.log(`Transaction included in block: ${txReceipt.blockNumber}`);
        console.log(`Gas used: ${txReceipt.gasUsed.toString()}`);
    } else {
        console.log(`Transaction failed`);
    }
    process.exit(0);
}

function parseOptions(options: OptionValues) {
    const amount = parseAmountOrExit(requiredInput(options.amount, 'Failed to send CTC: Must specify an amount'));
    const recipient = parseEVMAddressOrExit(requiredInput(options.to, 'Failed to send CTC: Must specify a recipient'));
    return { amount, recipient };
}
