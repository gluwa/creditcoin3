import { commandSync } from 'execa';
import { CLI_PATH } from './helpers';

describe('Convert Address command', () => {
    it('should convert a valid Substrate address', () => {
        const result = commandSync(
            `node ${CLI_PATH} convert-address --address 5HDRB6edmWwwh6aCDKrRSbisV8iFHdP7jDy18U2mt9w2wEkq`,
        );

        expect(result.stdout).toContain('0x');
    }, 60000);

    it('should convert a valid EVM address', () => {
        const result = commandSync(
            `node ${CLI_PATH} convert-address --address 0xCb5705FbB64F1336c7cBaC018FB251930A753333`,
        );

        expect(result.stdout).toContain('5');
    }, 60000);

    it('should NOT convert an invalid address', () => {
        // Test that command fails with an invalid address
        const result = commandSync(`node ${CLI_PATH} convert-address --address 0x123`, { reject: false });

        expect(result.stderr).toContain('Not a valid Substrate or EVM address.');
    }, 60000);

    // Test a known convertion from Substrate to EVM works
    it('should convert a known Substrate address to a known EVM address', () => {
        const result = commandSync(
            `node ${CLI_PATH} convert-address --address 5HDRB6edmWwwh6aCDKrRSbisV8iFHdP7jDy18U2mt9w2wEkq`,
        );

        expect(result.stdout.toLowerCase()).toContain('0xe3d237ebd67e011dbf48c34e5e4e936c5debe205');
    }, 60000);

    // Test a known convertion from EVM to Substrate works
    it('should convert a known EVM address to a known Substrate address', () => {
        const result = commandSync(
            `node ${CLI_PATH} convert-address --address 0xCb5705FbB64F1336c7cBaC018FB251930A753333`,
        );

        expect(result.stdout).toContain('5EaVMBYdtn7jwhmtF17YYkYaju74hA5FZZuCWjHmwaWeMbK1');
    }, 60000);
});
