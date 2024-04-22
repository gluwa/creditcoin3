import { WebSocketProvider, ethers, parseEther } from 'ethers';
import contractABI = require('./artifacts/SubstrateTransfer.json');
import { Keyring } from '@polkadot/keyring';
import { mnemonicGenerate } from '@polkadot/util-crypto';

describe('Substrate seamless transfer precompile', (): void => {
    let provider: WebSocketProvider;
    let precompileContractAddress: string;
    let txHash: string;
    let receipt: string;

    beforeAll(async () => {
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        const alith = new ethers.Wallet(privateKey, provider);

        // precompile contract deployed at 4049 to hex, see runtime/src/precompiles.rs for more
        precompileContractAddress = '0x0000000000000000000000000000000000000fd1';

        const contract = new ethers.Contract(precompileContractAddress, contractABI, alith);

        const target = new Keyring();
        const pair = target.addFromMnemonic(mnemonicGenerate());

        const amount = parseEther('10.0');
        const gasPrice = (await provider.getFeeData()).gasPrice;

        const result = await contract.transfer_substrate(pair.addressRaw, amount, {
            gasPrice,
        });

        receipt = await result.wait();
        txHash = result?.hash;
    }, 25000);

    test('substrate_transfer', () => {
        expect(txHash).toBeDefined();
        expect(receipt).toBeDefined();
    });
});
