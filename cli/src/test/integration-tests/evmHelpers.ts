import { Mnemonic, Wallet } from "ethers";

export function randomEvmAccount ()
{
    const mnemonic = Mnemonic.entropyToPhrase(Wallet.createRandom().privateKey);
    console.log(mnemonic);
    const wallet = Wallet.fromPhrase(mnemonic);
    console.log(wallet.address);
    const privateKey = wallet.privateKey;
    console.log(privateKey);
    return {
        mnemonic,
        address: wallet.address,
        privateKey,
    }
}