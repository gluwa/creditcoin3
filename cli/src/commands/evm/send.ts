import { Command, OptionValues } from 'commander';
import { ethers } from 'ethers';
import { initEVMCallerWallet } from '../../lib/evm/wallet';
import { parseAmountOrExit, parseEVMAddressOrExit, requiredInput } from '../../lib/parsing';
import { getEvmUrl } from '../../lib/evm/rpc';
import { getEVMBalanceOf, getTransferFeeEstimation } from '../../lib/evm/balance';
import { toCTCString } from '../../lib/balance';
import { BN } from '@polkadot/util';
import { amountOption, ecdsaOption, recipientOption } from '../options';

export function makeEvmSendCommand() {
    const cmd = new Command('send');
    cmd.description('Send funds from an EVM account to another EVM account');
    cmd.addOption(amountOption);
    cmd.addOption(ecdsaOption);
    cmd.addOption(recipientOption);
    cmd.action(evmSendAction);
    return cmd;
}

async function evmSendAction(options: OptionValues) {
    const wallet = await initEVMCallerWallet(options);
    const { amount, recipient } = parseOptions(options);
    const signer = wallet.connect(new ethers.JsonRpcProvider(getEvmUrl(options)));

    const tx = {
        to: recipient,
        value: amount.toString(),
    };

    await checkIfEnoughBalance(wallet.address, amount, options);

    const result = await signer.sendTransaction(tx);

    const txReceipt = await result.wait();

    // Check if txReceipt is not null and then log information
    if (txReceipt) {
        console.log(`Transaction hash: ${result.hash}`);
        console.log(`Transaction included in block: ${txReceipt.blockNumber}`);
        console.log(`Gas used: ${txReceipt.gasUsed.toString()}`);
    } else {
        console.log(`Transaction failed`);
    }
    process.exit(0);
}

function parseOptions(options: OptionValues) {
    const amount = parseAmountOrExit(requiredInput(options.amount, 'Failed to send CTC: Must specify an amount'));
    const recipient = parseEVMAddressOrExit(
        requiredInput(options.recipient, 'Failed to send CTC: Must specify a recipient'),
    );
    return { amount, recipient };
}

async function checkIfEnoughBalance(address: string, amount: BN, options: OptionValues) {
    const balance = await getEVMBalanceOf(address, getEvmUrl(options));

    const fees = await getTransferFeeEstimation(getEvmUrl(options));

    if (balance.ctc < BigInt(amount.toString()) + fees) {
        console.log(`Insufficient balance to send ${toCTCString(amount)}`);
        console.log(`This CC3 CLI considers the transfer fee to be at least twice the base fee`);
        process.exit(1);
    }
}
