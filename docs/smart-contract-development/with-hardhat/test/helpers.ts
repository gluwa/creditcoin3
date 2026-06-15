import { mine, time } from '@nomicfoundation/hardhat-toolbox/network-helpers';

export async function skipIfNotCreditcoin() {
    try {
        // only available for Hardhat
        await time.latest();
        // so skip everything
        // @ts-ignore
        this.skip();
    } catch {
        // just ingoring the exception
    }
}

export async function skipIfNotHardhat() {
    try {
        // only available for Hardhat
        await time.latest();
    } catch {
        // so skip everything if we're running against Creditcoin
        // @ts-ignore
        this.skip();
    }
}

// Progress the blockchain by `num` blocks with a blocktime of `blocktime` seconds
export async function progressBlocks(num: number, blocktime: number) {
    await mine(num, { interval: blocktime });
}
