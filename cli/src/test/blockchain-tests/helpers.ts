import { ethers } from 'ethers';
import { readFile } from 'fs/promises';
import path from 'path';

// matches artifacts/proof_example_erc20.json
export const validQuery = {
    chainId: 2, // not checked by Cairo
    height: 23,
    index: 0,
    // note: must be `layout` when sending to verify() precompile
    layoutSegments: [
        {
            offset: 448,
            size: 32,
        },
        {
            offset: 192,
            size: 32,
        },
        {
            offset: 224,
            size: 32,
        },
        {
            offset: 800,
            size: 32,
        },
        {
            offset: 928,
            size: 32,
        },
        {
            offset: 960,
            size: 32,
        },
        {
            offset: 992,
            size: 32,
        },
        {
            offset: 1056,
            size: 32,
        },
    ],
};

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
