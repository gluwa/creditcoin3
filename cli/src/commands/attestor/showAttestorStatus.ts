import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { evmAddressOption, chainKeyOption } from '../options';
import { evmAddressToSubstrateAddress } from '../../lib/evm/address';

export function makeShowAttestorStatusCommand() {
    const cmd = new Command('show-status');
    cmd.description(
        'Show the status of every attestor registered under a stash (identified by its EVM address) on a given chain',
    );
    cmd.addOption(evmAddressOption.makeOptionMandatory());
    cmd.addOption(chainKeyOption.makeOptionMandatory());
    cmd.action(showAttestorStatus);
    return cmd;
}

async function showAttestorStatus(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    // The stash is recorded as the AccountId derived from the caller's EVM
    // address via HashedAddressMapping. The user passes the EVM `0x…`; we map
    // it to the stored stash AccountId before scanning the attestation map.
    const evmAddress = options.evmAddress as string;
    const stashAccountId = evmAddressToSubstrateAddress(evmAddress);
    const chainKey = BigInt(options.chain);

    const attestorsKeys = await api.query.attestation.attestors.keys();
    let found = 0;
    for (const key of attestorsKeys) {
        const chain = (key.args[0] as any).toBigInt();
        if (chain !== chainKey) {
            continue;
        }
        const attestor: any = await api.query.attestation.attestors(key.args[0], key.args[1]);
        if (attestor.isNone) {
            continue;
        }
        const value = attestor.unwrap();
        if (value.stash.toString() !== stashAccountId) {
            continue;
        }

        const statusEnum: any = value.status;
        let statusLabel: string;
        if (statusEnum.isActive) {
            statusLabel = 'Active';
        } else if (statusEnum.isIdle) {
            statusLabel = 'Idle';
        } else if (statusEnum.isWaiting) {
            statusLabel = 'Waiting';
        } else {
            statusLabel = `Unknown(${statusEnum.toString()})`;
        }

        console.log(`Address ${key.args[1].toString()} status is ${statusLabel}`);
        found++;
    }

    if (found === 0) {
        console.log(`No attestors registered for stash ${evmAddress} on chain ${chainKey}`);
    }

    await api.disconnect();
    process.exit(0);
}
