import { ethers } from 'ethers';
import { readFile } from 'fs/promises';
import path from 'path';

export const deployContract = async (
    contractName: string,
    // deno-lint-ignore no-explicit-any
    args: any[],
    wallet: ethers.Wallet,
): Promise<ethers.Contract> => {
    console.log(`deploying ${contractName}`);

    const artifactsPath = path.resolve(__dirname, `./artifacts/${contractName}.json`);

    const contents = await readFile(artifactsPath);
    const metadata = JSON.parse(contents.toString());

    const factory = new ethers.ContractFactory(metadata.abi, metadata.data.bytecode.object, wallet);

    const contract = await factory.deploy(...args);

    // The contract is NOT deployed yet; we must wait until it is mined
    const deployed = await contract.waitForDeployment();
    console.log(`done at ${await deployed.getAddress()}`);
    return contract as ethers.Contract;
};
