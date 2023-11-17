import { initEthKeyringPair } from "../../lib/account/keyring";

describe('keyring', () => {
    test('from mnemonic or pk should create the same account', () =>
    {
        const fromMnemonic = initEthKeyringPair(
            'bottom drive obey lake curtain smoke basket hold race lonely fit walk'
        ).address;

        const fromPk = initEthKeyringPair(
            '0x5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133'
        ).address;

        expect(fromMnemonic).toBe(fromPk);
    });
});