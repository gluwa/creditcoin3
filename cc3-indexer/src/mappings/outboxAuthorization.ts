import { Interface } from '@ethersproject/abi';

const discoveryAbi = new Interface([
    'function isActiveOutbox(uint32 chainKey, address outbox) view returns (bool)',
    'function activeOutboxes(uint32 chainKey) view returns (address[])',
]);

/** Governance state at the indexed block, including genesis entries and registry rotations. */
export async function discoveryAddress(chainKey: bigint): Promise<string | undefined> {
    // Older runtime versions had no Discovery registry. They cannot authorize an Outbox.
    const query = (api.query as any).supportedChains?.outboxDiscoveries;
    if (!query) return undefined;
    const address = await query(chainKey.toString());
    return address.isSome ? address.unwrap().toHex().toLowerCase() : undefined;
}

async function callDiscovery(address: string, data: string, blockNumber: number): Promise<string> {
    // SubQuery injects a height-scoped api. Supply the indexed height explicitly as well:
    // Frontier's eth.call declares this parameter isHistoric, so the safe API permits it.
    // Never use unsafeApi/latest. Errors must propagate and retry the block, not admit a
    // candidate or permanently skip its message during an RPC outage.
    const result = await api.rpc.eth.call({ to: address, data }, blockNumber);
    return result.toHex();
}

export async function isAuthorizedOutbox(chainKey: bigint, outbox: string, blockNumber: number): Promise<boolean> {
    const address = await discoveryAddress(chainKey);
    if (!address) return false;
    return isActiveInRegistry(address, chainKey, outbox, blockNumber);
}

async function isActiveInRegistry(
    address: string,
    chainKey: bigint,
    outbox: string,
    blockNumber: number,
): Promise<boolean> {
    const result = await callDiscovery(
        address,
        discoveryAbi.encodeFunctionData('isActiveOutbox', [chainKey.toString(), outbox]),
        blockNumber,
    );
    // Reject malformed/non-canonical booleans instead of treating any nonzero response as true.
    if (result !== '0x' + '0'.repeat(64) && result !== '0x' + '0'.repeat(63) + '1') {
        throw new Error(`Invalid isActiveOutbox response from Discovery ${address} at ${blockNumber}`);
    }
    return result.endsWith('1');
}

/** Publications must not depend on factory candidates or same-block registry handler order. */
export async function authorizedOutboxChainKey(outbox: string, blockNumber: number): Promise<bigint | undefined> {
    const query = (api.query as any).supportedChains?.outboxDiscoveries;
    if (!query) return undefined;
    // Only governance-selected registries are queried. An arbitrary emitting contract cannot
    // make the indexer execute its own getters or persist unbounded candidate/message rows.
    for (const [key, value] of await query.entries()) {
        if (!value.isSome) continue;
        const chainKey = BigInt(key.args[0].toString());
        const address = value.unwrap().toHex().toLowerCase();
        if (await isActiveInRegistry(address, chainKey, outbox, blockNumber)) return chainKey;
    }
    return undefined;
}

/** Snapshot members when governance registers a registry that already contains Outboxes. */
export async function registeredOutboxes(
    chainKey: bigint,
    expectedDiscovery: string,
    blockNumber: number,
): Promise<string[]> {
    const address = await discoveryAddress(chainKey);
    if (!address || address !== expectedDiscovery.toLowerCase()) return [];
    const result = await callDiscovery(
        address,
        discoveryAbi.encodeFunctionData('activeOutboxes', [chainKey.toString()]),
        blockNumber,
    );
    return (discoveryAbi.decodeFunctionResult('activeOutboxes', result)[0] as string[]).map((outbox) =>
        outbox.toLowerCase(),
    );
}
