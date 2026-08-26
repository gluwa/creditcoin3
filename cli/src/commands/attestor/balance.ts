import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../../lib';
import { evmAddressOption } from '../options';
import { getBalance, toCTCString } from '../../lib/balance';
import { getAttestorContractReadOnly, substrateAddressToBytes32 } from '../../lib/attestor/precompile';
import { evmAddressToSubstrateAddress } from '../../lib/evm/address';
import Table from 'cli-table3';

export function showAttestorBalanceActionCommand() {
    const cmd = new Command('show-stash-balance');
    cmd.description('Show balance of the attestor stash account (identified by its EVM address)');
    cmd.addOption(evmAddressOption.makeOptionMandatory());
    cmd.action(showStashBalanceAction);
    return cmd;
}

async function showStashBalanceAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    // The stash is recorded as the AccountId derived from the caller's EVM
    // address via HashedAddressMapping. The user-facing input is the EVM
    // `0x…`; we map it to the stored AccountId for ledger + balance lookups.
    const evmAddress = options.evmAddress as string;
    const stashAccountId = evmAddressToSubstrateAddress(evmAddress);
    const stashBytes32 = substrateAddressToBytes32(stashAccountId);

    const contract = getAttestorContractReadOnly(options);
    const ledgerInfo = await contract.getLedger(stashBytes32);

    if (!ledgerInfo.exists) {
        console.error(`No ledger found for ${evmAddress}`);
        await api.disconnect();
        process.exit(1);
    }

    const totalStaked = new BN(ledgerInfo.totalStaked.toString());
    const active = new BN(ledgerInfo.active.toString());
    const unlockingChunks: number = ledgerInfo.unlockingChunks;

    // Reuse the shared helper rather than reading derive fields directly. `locked` has to sum
    // two disjoint mechanisms: the attestor bond, which the attestation pallet still keeps as a
    // `b0ndl0ck` lock (see `pallets/attestation/src/ledger.rs`) and so lands in
    // `lockedBalance`, and any pallet-staking stake, which now sits in a HoldReason::Staking
    // hold and is invisible to `lockedBalance`. Reading the derive directly reported the bond
    // but silently dropped the held stake, so both terms are needed.
    const balance = await getBalance(stashAccountId, api);

    const table = new Table({});

    table.push(
        ['Transferable', toCTCString(balance.transferable, 4)],
        ['Locked', toCTCString(balance.locked, 4)],
        ['Total', toCTCString(balance.total, 4)],
        ['TotalStake', toCTCString(totalStaked, 4)],
        ['ActiveStake', toCTCString(active, 4)],
        ['UnlockingChunks', unlockingChunks.toString()],
    );

    console.log(`Address: ${evmAddress}`);
    console.log(table.toString());

    await api.disconnect();
    process.exit(0);
}
