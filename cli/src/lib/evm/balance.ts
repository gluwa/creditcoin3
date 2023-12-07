import { JsonRpcProvider } from 'ethers';

export async function getEVMBalanceOf(address: string, rpcUrl: string): Promise<bigint> {
    // Create an ethers provider and get balance of address
    // Return balance as a bigint
    // NOTE: Seems like the EVM side cannot access the existential deposit amount
    const provider = new JsonRpcProvider(rpcUrl);
    const balance = await provider.getBalance(address);
    return balance;
}
