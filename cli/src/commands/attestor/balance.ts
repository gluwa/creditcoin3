import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../../lib';
import { evmAddressOption } from '../options';
import { getBalancesAll, toCTCString } from '../../lib/balance';
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

    const balanceAll = await getBalancesAll(stashAccountId, api);

    const table = new Table({});

    table.push(
        ['Transferable', toCTCString(balanceAll.availableBalance, 4)],
        ['Locked', toCTCString(balanceAll.lockedBalance, 4)],
        ['Total', toCTCString(balanceAll.freeBalance.add(balanceAll.reservedBalance), 4)],
        ['TotalStake', toCTCString(totalStaked, 4)],
        ['ActiveStake', toCTCString(active, 4)],
        ['UnlockingChunks', unlockingChunks.toString()],
    );

    console.log(`Address: ${evmAddress}`);
    console.log(table.toString());

    await api.disconnect();
    process.exit(0);
}
