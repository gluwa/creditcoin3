import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { describeIf, extractFee, forElapsedBlocks } from '../../../utils';
import { randomFundedAccount } from '../../../integration-tests/helpers';
import { chain_Anvil1_Key } from '../supported-chains/consts';

describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'swapMember', (): void => {
    let root: KeyringPair;
    let operator1: KeyringPair;
    let operator2: KeyringPair;
    let api: ApiPromise;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        operator1 = (await randomFundedAccount(api, root)).keyring;
        operator2 = (await randomFundedAccount(api, root)).keyring;

        let members = await api.query.operators.members();
        // todo: the operators pallet doesn't currently expose this as a constant
        expect(members.length).toBeLessThan(5);
        // note: using .toJSON() b/c when treated as an array the toContain() method
        // will fail even when the address is actually contained in result
        expect(members.toJSON()).not.toContain(operator1.address);
        expect(members.toJSON()).not.toContain(operator2.address);

        await api.tx.sudo
            .sudo(api.tx.operators.addMember(operator1.address))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        members = await api.query.operators.members();
        // note: using .toJSON() b/c when treated as an array the toContain() method
        // will fail even when the address is actually contained in result
        expect(members.toJSON()).toContain(operator1.address);
        expect(members.toJSON()).not.toContain(operator2.address);
    }, 90_000);

    afterAll(async () => {
        await api.tx.sudo
            .sudo(api.tx.operators.resetMembers([]))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        await api.disconnect();
    });

    test('should replace existing member with a new member who can execute privileged extrinsic', async (): Promise<void> => {
        await api.tx.sudo
            .sudo(api.tx.operators.swapMember(operator1.address, operator2.address))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        const members = await api.query.operators.members();
        expect(members.length).toBeGreaterThanOrEqual(1);
        // note: using .toJSON() b/c when treated as an array the toContain() method
        // will fail even when the address is actually contained in result
        expect(members.toJSON()).not.toContain(operator1.address);
        expect(members.toJSON()).toContain(operator2.address);

        // operator2 can exercise privileged extrinsic
        const nonce = await api.rpc.system.accountNextIndex(operator2.address);
        return new Promise((resolve, reject): void => {
            const unsubscribe = api.tx.supportedChains
                .setMaturityStrategy(chain_Anvil1_Key, 'EvmFinalized')
                .signAndSend(operator2, { nonce }, async ({ dispatchError, events, status }) => {
                    // note: also checks for dispatch error(s)
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(new Error(error)));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 60_000);
});
