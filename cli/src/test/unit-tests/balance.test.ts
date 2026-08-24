import { BN } from '@polkadot/util';
import type { DeriveBalancesAll } from '@polkadot/api-derive/types';
import { getTransferable } from '../../lib/balance';

/**
 * `getTransferable` must follow pallet-balances `reducible_balance()`:
 *   free - max(maybeEd, frozen - reserved)
 * and must not fall back to `availableBalance` (`free - lockedBalance`), which ignores holds
 * and double-counts freezes already covered by a reserve.
 */
const derived = (transferable: BN | null, availableBalance: BN) =>
    ({ transferable, availableBalance }) as unknown as DeriveBalancesAll;

describe('getTransferable', () => {
    test('prefers `transferable` over the legacy `availableBalance`', () => {
        // Staker with bonded funds in a HoldReason::Staking hold plus an unrelated 100 freeze
        // that the reserve already covers: spendable is the full free balance, but
        // availableBalance would wrongly subtract the freeze.
        const result = getTransferable(derived(new BN(4004), new BN(3904)));
        expect(result.toString()).toBe('4004');
    });

    test('falls back to `availableBalance` when the chain reports no `transferable`', () => {
        const result = getTransferable(derived(null, new BN(3904)));
        expect(result.toString()).toBe('3904');
    });

    test('returns zero rather than negative when everything is encumbered', () => {
        const result = getTransferable(derived(new BN(0), new BN(0)));
        expect(result.toString()).toBe('0');
    });
});
