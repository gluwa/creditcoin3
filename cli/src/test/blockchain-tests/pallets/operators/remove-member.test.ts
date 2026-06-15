import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { describeIf, forElapsedBlocks } from '../../../utils';

describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'removeMember', (): void => {
    let root: KeyringPair;
    let operator: KeyringPair;
    let api: ApiPromise;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        operator = (global as any).CREDITCOIN_CREATE_SIGNER('random');

        let members = await api.query.operators.members();
        // todo: the operators pallet doesn't currently expose this as a constant
        expect(members.length).toBeLessThan(5);
        // note: using .toJSON() b/c when treated as an array the toContain() method
        // will fail even when the address is actually contained in result
        expect(members.toJSON()).not.toContain(operator.address);

        await api.tx.sudo
            .sudo(api.tx.operators.addMember(operator.address))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        members = await api.query.operators.members();
        // note: using .toJSON() b/c when treated as an array the toContain() method
        // will fail even when the address is actually contained in result
        expect(members.toJSON()).toContain(operator.address);
    }, 60_000);

    afterAll(async () => {
        await api.tx.sudo
            .sudo(api.tx.operators.resetMembers([]))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        await api.disconnect();
    });

    test('should remove a registered member', async (): Promise<void> => {
        await api.tx.sudo
            .sudo(api.tx.operators.removeMember(operator.address))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        const members = await api.query.operators.members();
        // note: using .toJSON() b/c when treated as an array the toContain() method
        // will fail even when the address is actually contained in result
        expect(members.toJSON()).not.toContain(operator.address);
    }, 60_000);
});
