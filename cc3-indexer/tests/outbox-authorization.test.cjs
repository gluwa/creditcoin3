const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');
const ts = require('typescript');
const { Interface } = require('@ethersproject/abi');

const factory = '0x' + '11'.repeat(20);
const outbox = '0x' + '22'.repeat(20);
const attacker = '0x' + '33'.repeat(20);
const registry = '0x' + '44'.repeat(20);
const replacement = '0x' + '55'.repeat(20);
const txHash = '0x' + '66'.repeat(32);
const emitter = attacker + '00'.repeat(12);
const messageId = (n) => '0x' + n.toString(16).padStart(64, '0');
const abi = new Interface([
    'function isActiveOutbox(uint32 chainKey, address outbox) view returns (bool)',
    'function activeOutboxes(uint32 chainKey) view returns (address[])',
]);

function model() {
    const rows = new Map();
    return {
        rows,
        get: async (id) => rows.get(id),
        create: (fields) => ({
            ...fields,
            async save() {
                rows.set(this.id, this);
            },
        }),
        remove: async (id) => rows.delete(id),
        getByFields: async (filters, { limit = 100 } = {}) =>
            [...rows.values()]
                .filter((row) => filters.every(([key, op, value]) => op === '=' && row[key] === value))
                .slice(0, limit),
    };
}

async function fixture() {
    const types = Object.fromEntries(
        [
            'OutboxContract',
            'OutboxFactoryRegistration',
            'OutboxMessage',
            'PendingOutbox',
            'QuarantinedMessage',
            'SupportedChain',
            'TransactionVerified',
        ].map((name) => [name, model()]),
    );
    const state = {
        height: 100,
        registry,
        active: () => false,
        members: [],
        calls: [],
        error: undefined,
        rawResult: undefined,
        // Optional second governance entry (chain key 9) whose registry answers with garbage —
        // e.g. a misconfigured address with no code. Only consulted by the unknown-emitter scan.
        brokenRegistry: undefined,
    };
    const registryOption = () => ({ isSome: Boolean(state.registry), unwrap: () => ({ toHex: () => state.registry }) });
    const registryQuery = async (key) => {
        assert.equal(key, '8');
        return registryOption();
    };
    registryQuery.entries = async () => {
        const entries = [];
        if (state.brokenRegistry) {
            entries.push([{ args: [9n] }, { isSome: true, unwrap: () => ({ toHex: () => state.brokenRegistry }) }]);
        }
        if (state.registry) entries.push([{ args: [8n] }, registryOption()]);
        return entries;
    };
    const api = {
        query: { supportedChains: { outboxDiscoveries: registryQuery } },
        rpc: {
            eth: {
                call: async ({ to, data }, height) => {
                    // Exercise the real ABI encoding and require explicit historical height, never latest.
                    assert.equal(height, state.height);
                    if (state.brokenRegistry && to === state.brokenRegistry) {
                        state.calls.push({ to, height, method: 'broken' });
                        return { toHex: () => '0x' };
                    }
                    assert.equal(to, state.registry);
                    const call = abi.parseTransaction({ data });
                    assert.equal(call.args.chainKey, 8);
                    state.calls.push({ to, height, method: call.name });
                    if (state.error) throw state.error;
                    if (state.rawResult) return { toHex: () => state.rawResult };
                    const result =
                        call.name === 'isActiveOutbox'
                            ? state.active(height, call.args.outbox.toLowerCase(), to)
                            : state.members;
                    return { toHex: () => abi.encodeFunctionResult(call.name, [result]) };
                },
            },
        },
    };
    const modules = {};
    const log = Object.fromEntries(['info', 'warn', 'error', 'debug'].map((key) => [key, () => {}]));
    function load(name) {
        if (modules[name]) return modules[name];
        const exports = {};
        modules[name] = exports;
        const filename = path.join(__dirname, '../src/mappings', name + '.ts');
        const compiled = ts.transpileModule(fs.readFileSync(filename, 'utf8'), {
            compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020, esModuleInterop: true },
            fileName: filename,
        }).outputText;
        vm.runInNewContext(
            compiled,
            {
                exports,
                api,
                logger: log,
                require: (specifier) => {
                    if (specifier === '../types') return types;
                    if (specifier === './storeUtils') return { flushStore: async () => {} };
                    if (specifier === './initStore') return {};
                    if (specifier.startsWith('./')) return load(specifier.slice(2));
                    return require(specifier);
                },
            },
            { filename },
        );
        return exports;
    }
    const handlers = load('evmHandlers');
    const mappings = load('mappingHandlers');
    const event = (args, address) => ({
        args,
        address,
        transactionHash: txHash,
        blockNumber: state.height,
        blockTimestamp: new Date(state.height * 1000),
        transactionIndex: 0,
        logIndex: 0,
    });
    await types.SupportedChain.create({ id: '8', chainKey: 8n }).save();
    await types.OutboxFactoryRegistration.create({ id: '8', factoryAddress: factory }).save();
    return {
        types,
        state,
        handlers,
        mappings,
        api,
        created: (address = outbox, origin = factory) =>
            handlers.handleOutboxCreated(event([address, 8n, attacker, attacker, '1'], origin)),
        registered: (address = outbox, origin = registry) =>
            handlers.handleOutboxRegistered(event([8n, address, factory], origin)),
        publish: (id = 1, address = outbox) =>
            handlers.handleMessagePublished(event([messageId(id), emitter, false, '0x1234'], address)),
        ack: (id = 1, address = outbox) => handlers.handleMessageAcknowledged(event([messageId(id)], address)),
    };
}

test('a permissionless deployment from the registered factory never authorizes messages', async () => {
    const f = await fixture();
    await f.created();
    await f.publish();
    assert.equal(f.types.OutboxContract.rows.size, 0);
    assert.equal(f.types.OutboxMessage.rows.size, 0);
    assert.equal(f.types.QuarantinedMessage.rows.size, 0);
    assert.equal(f.types.PendingOutbox.rows.size, 1);
    await f.handlers.promotePendingOutboxes(8n, factory, f.state.height);
    assert.equal(f.types.OutboxContract.rows.size, 0, 'factory registration alone cannot promote a candidate');
});

test('canonical members index at historical height and stop at the removal boundary', async () => {
    const f = await fixture();
    f.state.active = (height) => height < 105;
    await f.created();
    f.state.height = 104;
    await f.publish();
    f.state.height = 105;
    await f.publish(2);
    assert.equal(f.types.OutboxContract.rows.size, 1, 'historical Outbox row is retained');
    assert.equal(f.types.OutboxMessage.rows.size, 1, 'the retained row does not authorize a removed Outbox');
    assert.deepEqual(
        f.state.calls.map((call) => call.height),
        [100, 104, 105],
    );
    await f.ack(1, attacker);
    assert.equal((await f.types.OutboxMessage.get(messageId(1))).acknowledged, false);
    await f.ack();
    assert.equal(
        (await f.types.OutboxMessage.get(messageId(1))).acknowledged,
        true,
        'old messages can complete after removal',
    );
});

test('cancelled removals and old active Outboxes continue without following only the default', async () => {
    const f = await fixture();
    f.state.active = (_height, address) => address === outbox;
    await f.registered();
    f.state.height = 500;
    await f.publish();
    assert.equal(f.types.OutboxMessage.rows.size, 1);
    assert(f.state.calls.every((call) => call.method === 'isActiveOutbox'));
});

test('forged registry events reject; authentic registry admission bypasses a full candidate cap', async () => {
    const f = await fixture();
    f.state.active = (_height, address) => address === outbox;
    await f.registered(outbox, attacker);
    assert.equal(f.types.OutboxContract.rows.size, 0);
    for (let i = 0; i < f.handlers.MAX_PENDING_OUTBOXES_PER_CHAIN_KEY; i++) {
        await f.created('0x' + (100 + i).toString(16).padStart(40, '0'));
    }
    await f.registered();
    await f.publish();
    assert.equal(f.types.PendingOutbox.rows.size, 8);
    assert.equal(f.types.OutboxContract.rows.size, 1);
    assert.equal(f.types.OutboxMessage.rows.size, 1);
});

test('later Discovery registration admits candidates without retroactively authorizing publications', async () => {
    const f = await fixture();
    await f.created();
    await f.publish(1);
    // Compatibility with rows written by the previous indexer version: never backfill them.
    await f.types.QuarantinedMessage.create({ id: messageId(1), outboxAddress: outbox }).save();
    f.state.height = 101;
    f.state.active = () => true;
    await f.registered();
    await f.publish(2);
    assert.equal(f.types.PendingOutbox.rows.size, 0);
    assert.equal(f.types.QuarantinedMessage.rows.size, 0);
    assert.equal(f.types.OutboxMessage.rows.size, 1);
    assert(await f.types.OutboxMessage.get(messageId(2)));
});

test('governance registration snapshots existing registry members without factory events', async () => {
    const f = await fixture();
    f.state.members = [outbox];
    f.state.active = () => true;
    await f.mappings.handleOutboxDiscoveryRegistered({
        event: { data: [8n, registry] },
        block: { block: { header: { number: { toNumber: () => f.state.height } } }, timestamp: new Date(0) },
        extrinsic: { extrinsic: { hash: { toHex: () => txHash } } },
    });
    await f.publish();
    assert.equal(f.types.OutboxContract.rows.size, 1);
    assert.equal(f.types.OutboxMessage.rows.size, 1);
    assert.equal(f.state.calls[0].method, 'activeOutboxes');
});

test('registry rotations and governance removal revoke further publications from stored rows', async () => {
    const f = await fixture();
    f.state.active = (_height, _address, discovery) => discovery === registry;
    await f.registered();
    await f.publish(1);
    f.state.height++;
    f.state.registry = replacement;
    await f.publish(2);
    await f.registered(outbox, registry);
    f.state.registry = undefined;
    await f.publish(3);
    assert.equal(f.types.OutboxMessage.rows.size, 1);
    assert.equal(f.types.OutboxContract.rows.size, 1);
});

test('RPC errors and malformed authorization responses fail the indexed block for retry', async () => {
    const f = await fixture();
    f.state.active = () => true;
    await f.registered();
    f.state.error = new Error('archive RPC unavailable');
    await assert.rejects(() => f.publish(), /archive RPC unavailable/);
    f.state.error = undefined;
    f.state.rawResult = '0x' + '0'.repeat(63) + '2';
    await assert.rejects(() => f.publish(), /Invalid isActiveOutbox response/);
    assert.equal(f.types.OutboxMessage.rows.size, 0);
    f.state.rawResult = undefined;
    await f.publish();
    assert.equal(f.types.OutboxMessage.rows.size, 1);
});

test('a broken registry on another chain key cannot stall first publications elsewhere', async () => {
    const f = await fixture();
    f.state.active = (_height, address) => address === outbox;
    // Chain key 9's governance entry points at an address that returns no ABI bool. The
    // unknown-emitter scan must skip it and still find the emitter in chain key 8's registry.
    f.state.brokenRegistry = replacement;
    await f.publish();
    assert.equal(f.types.OutboxMessage.rows.size, 1, 'healthy registry still authorizes the publication');
    assert.equal(BigInt((await f.types.OutboxContract.get(outbox)).chainKey), 8n);
    assert(
        f.state.calls.some((call) => call.method === 'broken'),
        'the broken registry was consulted first',
    );
    // The known-Outbox path stays strict: a malformed answer from the Outbox's OWN registry is
    // still a failed block, never a silent skip.
    f.state.height++;
    f.state.rawResult = '0x' + '0'.repeat(63) + '2';
    await assert.rejects(() => f.publish(2), /Invalid isActiveOutbox response/);
    assert.equal(f.types.OutboxMessage.rows.size, 1);
});

test('runtimes without Discovery storage and unknown events create no authorized rows', async () => {
    const f = await fixture();
    delete f.api.query.supportedChains.outboxDiscoveries;
    await f.created();
    await f.publish();
    await f.registered();
    assert.equal(f.types.OutboxContract.rows.size, 0);
    assert.equal(f.types.OutboxMessage.rows.size, 0);
});

test('same-block publication precedes registration even when counterfeit candidates filled the cap', async () => {
    const f = await fixture();
    f.state.active = (_height, address) => address === outbox;
    for (let i = 0; i < f.handlers.MAX_PENDING_OUTBOXES_PER_CHAIN_KEY; i++) {
        await f.created('0x' + (100 + i).toString(16).padStart(40, '0'));
    }
    await f.publish();
    assert.equal(f.types.OutboxMessage.rows.size, 1);
    await f.registered();
    assert.equal(f.types.OutboxContract.rows.size, 1);
    assert.equal(f.types.OutboxMessage.rows.size, 1);
});

test('a counterfeit pending row cannot route an authorized publication to the wrong chain', async () => {
    const f = await fixture();
    f.state.active = (_height, address) => address === outbox;
    await f.types.PendingOutbox.create({ id: outbox, chainKey: 9n, factoryAddress: attacker }).save();
    await f.publish();
    const admitted = await f.types.OutboxContract.get(outbox);
    assert.equal(BigInt(admitted.chainKey), 8n);
    assert.equal(f.types.PendingOutbox.rows.size, 0);
    assert.equal(f.types.OutboxMessage.rows.size, 1);
});
