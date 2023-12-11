import { JsonRpcProvider } from 'ethers';
import { toCTCString } from '../balance';
import Table from 'cli-table3';
import { BN } from '..';

interface EVMBalance {
    address: string;
    ctc: bigint;
}

export async function getEVMBalanceOf(address: string, rpcUrl: string): Promise<EVMBalance> {
    // Create an ethers provider and get balance of address
    // Return balance as a bigint
    // NOTE: Seems like the EVM side cannot access the existential deposit amount
    const provider = new JsonRpcProvider(rpcUrl);
    const balance = await provider.getBalance(address);
    return { address, ctc: balance } as EVMBalance;
}

export async function getTransferFeeEstimation(rpcUrl: string): Promise<bigint> {
    const provider = new JsonRpcProvider(rpcUrl);
    const data = await provider.getFeeData();

    console.log(data);
    const fee = data.maxFeePerGas;
    const priority = data.maxPriorityFeePerGas ?? BigInt(0);
    if (!fee) {
        throw new Error('Could not get fee data from RPC at ' + rpcUrl);
    }
    const base = fee - priority;
    const maxFeePerGas = base * BigInt(2) + priority;
    const feesInCredo = BigInt(21000) * maxFeePerGas; // 21000 is the expected gas limit for a native currency transfer
    return feesInCredo;
}

export function logEVMBalance(balance: EVMBalance, human = true) {
    if (human) {
        printEVMBalance(balance);
    } else {
        printEVMJsonBalance(balance);
    }
}

export function printEVMBalance(balance: EVMBalance) {
    const table = new Table({});

    table.push(['CTC Balance', toCTCString(new BN(balance.ctc.toString()), 4)]);

    console.log(`Address: ${balance.address}`);
    console.log(table.toString());
}

export function printEVMJsonBalance(balance: EVMBalance) {
    const jsonBalance = {
        balance: {
            address: balance.address,
            ctc: balance.ctc.toString(),
        },
    };
    console.log(JSON.stringify(jsonBalance, null, 2));
}
