import { WebSocketProvider, ethers, parseEther } from 'ethers';
import contractABI = require('./artifacts/SubstrateTransfer.json');
import { Keyring } from '@polkadot/keyring';
import { mnemonicGenerate } from '@polkadot/util-crypto';

describe('Substrate seamless transfer precompile', (): void => {
    let contract: any;
    let amount: bigint;
    let pair: any;
    let gasPrice: any;

    beforeAll(async () => {
        const provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        // precompile contract deployed at 4049 to hex, see runtime/src/precompiles.rs for more
        const precompileContractAddress = '0x0000000000000000000000000000000000000fd1';

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        const alith = new ethers.Wallet(privateKey, provider);

        contract = new ethers.Contract(precompileContractAddress, contractABI, alith);

        const target = new Keyring();
        pair = target.addFromMnemonic(mnemonicGenerate());

        amount = parseEther('10.0');
        gasPrice = (await provider.getFeeData()).gasPrice;
    }, 25000);

    test('transfer_substrate', async () => {
        const result = await contract.transfer_substrate(pair.addressRaw, amount, {
            gasPrice,
        });
        const receipt = await result.wait();
        const txHash = result?.hash;
        expect(txHash).toBeDefined();
        expect(receipt).toBeDefined();
    }, 25000);
});
