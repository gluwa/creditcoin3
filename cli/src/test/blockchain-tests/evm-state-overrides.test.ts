import { WebSocketProvider, ethers } from 'ethers';

// `eth_call` state overrides (the third parameter) must apply to contract-creation calls, and a
// balance or nonce override must take effect for an address the chain has never seen. Both were
// silently ignored before.
describe('eth_call state overrides', (): void => {
    let provider: WebSocketProvider;

    // Runtime: SSTORE(0, GASPRICE), then return GASPRICE as a 32-byte word. Any runtime would do;
    // the creation call just has to return something recognisable.
    //   3a GASPRICE · 80 DUP1 · 6000 PUSH1 0 · 55 SSTORE · 6000 PUSH1 0 · 52 MSTORE ·
    //   6020 PUSH1 32 · 6000 PUSH1 0 · f3 RETURN
    const RUNTIME = '0x3a8060005560005260206000f3';
    // Init: CODECOPY the 13-byte runtime that follows these 11 bytes to memory, then return it.
    //   600d PUSH1 13 · 80 DUP1 · 600b PUSH1 11 · 6000 PUSH1 0 · 39 CODECOPY · 6000 PUSH1 0 · f3 RETURN
    const INIT = `0x600d80600b6000396000f3${RUNTIME.slice(2)}`;

    // Runtime: CREATE an empty contract and return its address as a 32-byte word. The address is
    // derived from this contract's nonce, which makes the nonce observable.
    //   6000 PUSH1 0 · 6000 PUSH1 0 · 6000 PUSH1 0 · f0 CREATE · 6000 PUSH1 0 · 52 MSTORE ·
    //   6020 PUSH1 32 · 6000 PUSH1 0 · f3 RETURN
    const FACTORY = '0x600060006000f060005260206000f3';

    const ONE_CTC = 10n ** 18n;

    // A random address: no `System.Account` entry exists for it.
    const unusedAddress = () => ethers.Wallet.createRandom().address;

    const ethCall = (request: Record<string, string>, overrides?: Record<string, Record<string, string>>) =>
        provider.send('eth_call', overrides ? [request, 'latest', overrides] : [request, 'latest']);

    beforeAll(() => {
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
    });

    afterAll(async () => {
        await provider.destroy();
    });

    describe('contract creation', () => {
        test('applies a balance override to the creating account', async () => {
            const from = unusedAddress();
            const request = { from, data: INIT, value: ethers.toQuantity(ONE_CTC) };

            // The sender can't fund the value it sends...
            await expect(ethCall(request)).rejects.toThrow();
            // ...until its balance is overridden.
            const deployed = await ethCall(request, { [from]: { balance: ethers.toQuantity(10n * ONE_CTC) } });
            expect(deployed).toEqual(RUNTIME);
        }, 60_000);
    });

    describe('an account the chain has never seen', () => {
        test('takes a balance override', async () => {
            const from = unusedAddress();
            const request = { from, to: unusedAddress(), value: ethers.toQuantity(ONE_CTC) };

            await expect(ethCall(request)).rejects.toThrow();
            expect(await ethCall(request, { [from]: { balance: ethers.toQuantity(10n * ONE_CTC) } })).toEqual('0x');
        }, 60_000);

        test('takes a nonce override', async () => {
            // The factory exists only through the code override, so it has no account entry
            // either; the address its CREATE returns shows which nonce it ran with.
            const factory = unusedAddress();
            const nonce = 42;

            const result = await ethCall(
                { to: factory, gas: ethers.toQuantity(1_000_000n) },
                { [factory]: { code: FACTORY, nonce: ethers.toQuantity(nonce) } },
            );
            const created = ethers.getAddress(ethers.dataSlice(result, 12));

            expect(created).toEqual(ethers.getCreateAddress({ from: factory, nonce }));
            expect(created).not.toEqual(ethers.getCreateAddress({ from: factory, nonce: 0 }));
        }, 60_000);
    });
});
