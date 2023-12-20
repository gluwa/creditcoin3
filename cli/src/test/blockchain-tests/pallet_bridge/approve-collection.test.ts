import { newApi, ApiPromise, KeyringPair } from '../../../lib';
import { parseAmount } from '../../../commands/options';
import { describeIf, testIf, extractFee, forElapsedBlocks } from '../../utils';

describeIf((global as any).CREDITCOIN_HAS_SUDO, 'approveCollection', (): void => {
    let api: ApiPromise;
    let authoritySigner: KeyringPair;

    beforeAll(async () => {
        api = (await newApi((global as any).CREDITCOIN_API_URL)).api;

        // insert an authority in order to be able to use it later
        const sudoSigner = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        authoritySigner = (global as any).CREDITCOIN_CREATE_SIGNER('random');
        await api.tx.sudo
            .sudo(api.tx.bridge.addAuthority(authoritySigner.address))
            .signAndSend(sudoSigner, { nonce: -1 });

        // give funds
        const amount = parseAmount('1000000');
        await api.tx.sudo
            .sudo(api.tx.balances.forceSetBalance(authoritySigner.address, amount))
            .signAndSend(sudoSigner, { nonce: -1 });
        await forElapsedBlocks(api);
    }, 100_000);

    afterAll(async () => {
        await api.disconnect();
    });

    testIf(
        (global as any).CREDITCOIN_HAS_SUDO,
        'fee is min 0.01 CTC',
        async (): Promise<void> => {
            // note1: Cc2BurnId is just a wrapper around u64
            // note2: use a random value so we can execute the tests repeatedly w/o restarting the blockchain
            const burnId = Math.floor(Math.random() * Number.MAX_SAFE_INTEGER);
            const collector = (global as any).CREDITCOIN_CREATE_SIGNER('random');
            const collectorId = collector.address;
            const amount = parseAmount('999');

            // collector starts with 0 CTC
            let balance = await api.derive.balances.all(collector.address);
            expect(balance.freeBalance.isZero()).toBe(true);

            return new Promise((resolve, reject): void => {
                const unsubscribe = api.tx.bridge
                    .approveCollection(burnId, collectorId, amount)
                    .signAndSend(authoritySigner, { nonce: -1 }, async ({ dispatchError, events, status }) => {
                        await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                    })
                    .catch((error) => reject(error));
            }).then((fee) => {
                expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
            });

            await forElapsedBlocks(api);

            // collector sends up with 999 CTC
            balance = await api.derive.balances.all(collector.address);
            expect(balance.freeBalance).toBe(amount);
        },
        200_000,
    );
});
