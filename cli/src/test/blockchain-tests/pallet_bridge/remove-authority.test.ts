import {
    expectNoEventError,
    expectNoDispatchError,
    newApi,
    AccountId,
    ApiPromise,
    Balance,
    KeyringPair,
} from '../../../lib';
import { describeIf, testIf } from '../../utils';

describeIf((global as any).CREDITCOIN_HAS_SUDO, 'removeAuthority', (): void => {
    let accountId: AccountId;
    let api: ApiPromise;
    let sudoSigner: KeyringPair;

    beforeAll(async () => {
        api = (await newApi((global as any).CREDITCOIN_API_URL)).api;
        sudoSigner = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        // insert an authority in order to be able to remove it later
        const randomAccount = (global as any).CREDITCOIN_CREATE_SIGNER('random');
        accountId = randomAccount.address;

        await api.tx.sudo.sudo(api.tx.bridge.addAuthority(accountId)).signAndSend(sudoSigner);
    });

    afterAll(async () => {
        await api.disconnect();
    });

    testIf((global as any).CREDITCOIN_HAS_SUDO, 'fee is 0', async (): Promise<void> => {
        const sudoCall = api.tx.sudo.sudo(api.tx.bridge.removeAuthority(accountId));
        const predicate = (fee: unknown) => expect(fee).toEqual(BigInt(0));

        return new Promise((resolve, _reject) => {
            const unsubscribe = sudoCall.signAndSend(
                sudoSigner,
                { nonce: -1 },
                async ({ dispatchError, events, status }) => {
                    expectNoDispatchError(api, dispatchError);
                    if (!status.isInBlock) return;
                    (await unsubscribe)();

                    events.forEach((event) => expectNoEventError(api, event));
                    const netFee = events
                        .filter(({ event: { section } }) => {
                            return section === 'balances';
                        })
                        .map(({ event: { method, data } }) => {
                            const transform = (x: any) => (x[1] as Balance).toBigInt();
                            if (method === 'Withdraw') return -transform(data);
                            else if (method === 'Deposit') return transform(data);
                            else throw new Error('Unhandled balances event');
                        })
                        .reduce((prev, curr, _index, _array) => {
                            return prev + curr;
                        }, BigInt(0));

                    resolve(netFee);
                },
            );
        }).then(predicate);
    });
});
