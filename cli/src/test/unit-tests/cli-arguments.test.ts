import { commandSync } from 'execa';

// Usage errors are raised by commander (or by the command itself) before any connection
// is made, so these run against the built CLI without a node. Nothing listens on the URL
// below: a command that got past validation would fail to connect instead.
const CLI_PATH = 'dist/cli.js';
const NO_NODE_URL = 'ws://127.0.0.1:1';

function runCli(cmd: string) {
    return commandSync(`node ${CLI_PATH} ${cmd} --url ${NO_NODE_URL}`, { reject: false, timeout: 20_000 });
}

describe('CLI argument validation', () => {
    test.each([
        ['balance', "error: required option '--substrate-address [address]' not specified"],
        ['status', "error: required option '--substrate-address [address]' not specified"],
        [
            'attestor register --attestor 5EACfEfYjfg5ZHpzp9uoMCR68UNGBUAu5AYjvZdM5aLYaojx',
            "error: required option '-c, --chain [chain]' not specified",
        ],
        ['attestor register --chain 2', "-a, --attestor [attestor]' not specified"],
        [
            'attestor unregister --attestor 5EACfEfYjfg5ZHpzp9uoMCR68UNGBUAu5AYjvZdM5aLYaojx',
            "error: required option '-c, --chain [chain]' not specified",
        ],
        ['attestor unregister --chain 2', "-a, --attestor [attestor]' not specified"],
        ['attestor chill', "error: required option '-c, --chain [chain]' not specified"],
        ['attestor chill --chain 2', "-a, --attestor [attestor]' not specified"],
        ['attestor show-status --chain 2', "error: required option '--evm-address [address]' not specified"],
        [
            'attestor show-status --evm-address 0xf24FF3a9CF04c71Dbc94D0b566f7A27B94566cac',
            "error: required option '-c, --chain [chain]' not specified",
        ],
        ['attestor show-stash-balance', "error: required option '--evm-address [address]' not specified"],
    ])('`%s` reports a missing required option', (cmd, message) => {
        const result = runCli(cmd);
        expect(result.exitCode).toEqual(1);
        expect(result.stderr).toContain(message);
    });

    test('set-keys without --keys or --rotate is rejected', () => {
        const result = runCli('set-keys');
        expect(result.exitCode).toEqual(1);
        expect(result.stdout).toContain('Must specify keys to set or generate new ones using the --rotate flag');
    });

    test('set-keys with both --keys and --rotate is rejected', () => {
        const result = runCli('set-keys --rotate --keys "test-me"');
        expect(result.exitCode).toEqual(1);
        expect(result.stderr).toContain('Must either specify keys or rotate to generate new ones, can not do both');
    });
});
