import { SpVersionRuntimeVersion } from '@polkadot/types/lookup';
import { creditcoinApi, expectNoDispatchError, expectNoEventError } from '../lib/api';
import { BN } from '../lib/index';
import { initKeyringPair } from '../lib/account/keyring';
import { u8aToHex } from '../lib/common';
import * as fs from 'fs';
import * as child_process from 'child_process';
import { promisify } from 'util';

// From https://github.com/chevdor/subwasm/blob/v0.19.0/lib/src/runtime_info.rs#L9-L21
/* eslint-disable @typescript-eslint/naming-convention */
type WasmRuntimeInfo = {
    size: number;
    compression: {
        size_compressed: number;
        size_decompressed: number;
        compressed: boolean;
    };
    reserved_meta: number[];
    reserved_meta_valid: boolean;
    metadata_version: number;
    core_version: SpVersionRuntimeVersion;
    proposal_hash: string;
    parachain_authorize_upgrade_hash: string;
    ipfs_hash: string;
    blake2_256: string;
};
/* eslint-enable */

// these normally use callbacks, but promises are more convenient
const readFile = promisify(fs.readFile);
const exec = promisify(child_process.exec);
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * Performs an upgrade to the runtime at the provided path.
 * @param wsUrl The URL of the node to send the upgrade transaction to. Should be a websocket URL, like `ws://127.0.0.1:9944`
 * @param wasmBlobPath The path to the wasm blob to upgrade to.
 * @param sudoKeyUri The the secret key (SURI, either a mnemonic or raw secret) of the account to use to send the upgrade transaction.
 * Must be the sudo account.
 * @param hasSubwasm Whether the subwasm CLI tool is installed. If true subwasm is used to get info about the runtime and checks are performed.
 */
async function doRuntimeUpgrade(
    wsUrl: string,
    wasmBlobPath: string,
    sudoKeyUri: string,
    hasSubwasm = false,
    scheduleDelay = 50,
): Promise<void> {
    // init the api client
    const { api } = await creditcoinApi(wsUrl);
    try {
        // make the keyring for the sudo account, see
        // test/integration-tests/helpers.ts::initAlithKeyring() for the devel mnemonic
        const keyring = initKeyringPair(sudoKeyUri);

        if (process.env.NEW_SUDO_BALANCE !== undefined) {
            await api.tx.sudo
                .sudo(api.tx.balances.forceSetBalance(keyring.address, new BN(process.env.NEW_SUDO_BALANCE)))
                .signAndSend(keyring, { nonce: -1 });
            // wait for 60 sec for blocks to finalize
            await sleep(60_000);
        }

        const { specVersion } = api.runtimeVersion;

        let needsUpgrade = true;

        if (hasSubwasm) {
            // subwasm needs to be installed with `cargo install --locked --git https://github.com/chevdor/subwasm --tag v0.19.0`
            const output = await exec(`subwasm info -j ${wasmBlobPath}`);
            if (output.stderr.length > 0) {
                throw new Error(`subwasm info failed: ${output.stderr}`);
            }
            const info = JSON.parse(output.stdout) as WasmRuntimeInfo;
            // should probably do some checks here to see that the runtime is right
            // e.g. the core version is reasonable, it's compressed, etc.
            if (Number(info.core_version.specVersion) <= specVersion.toNumber()) {
                needsUpgrade = false;
            }
        }

        if (!needsUpgrade) {
            console.log('Skipping upgrade because version has not increased');
            return;
        }

        // read the wasm blob from the give path
        const wasmBlob = await readFile(wasmBlobPath);

        const hexBlob = u8aToHex(wasmBlob);
        let callback = api.tx.system.setCode(hexBlob);
        if (scheduleDelay > 0) {
            // TODO: this is currently missing, see CSUB-899
            callback = api.tx.scheduler.scheduleAfter(scheduleDelay, null, 0, callback);
        }
        const overrideWeight = {
            refTime: new BN(1),
            proofSize: new BN(0),
        };

        // schedule the upgrade
        await new Promise<void>((resolve, reject) => {
            const unsubscribe = api.tx.sudo
                .sudoUncheckedWeight(callback, overrideWeight)
                .signAndSend(keyring, { nonce: -1 }, async ({ dispatchError, events, status }) => {
                    const finish = (fn: () => void) => {
                        unsubscribe
                            .then((unsub) => {
                                unsub();
                                fn();
                            })
                            .catch(reject);
                    };

                    // these two will throw exceptions in case of errors
                    try {
                        expectNoDispatchError(api, dispatchError);
                        if (events) events.forEach((event) => expectNoEventError(api, event));
                    } catch (err) {
                        /* eslint-disable */
                        // @ts-expect-error: 'err' is of type 'unknown'
                        const error = new Error(`Failed to schedule runtime upgrade: ${err.toString()}`);
                        /* eslint-enable */
                        finish(() => reject(error));
                    }

                    if (status.isInBlock) {
                        const header = await api.rpc.chain.getHeader(status.asInBlock);
                        const blockNumber = header.number.toNumber();

                        console.log(
                            `Runtime upgrade successfully scheduled at block ${blockNumber}, hash ${status.asInBlock.toString()}`,
                        );
                        finish(resolve);
                    }
                });
        });
    } finally {
        await api.disconnect();
    }
}

if (process.argv.length < 5) {
    console.error('runtimeUpgrade.ts <wsUrl> <wasmBlobPath> <sudoKeyUri>');
    process.exit(1);
}

const inputWsUrl = process.argv[2];
const inputWasmBlobPath = process.argv[3];
const inputSudoKeyUri = process.argv[4];
const explicitDelay = Number(process.argv[5] || 50);

doRuntimeUpgrade(inputWsUrl, inputWasmBlobPath, inputSudoKeyUri, true, explicitDelay).catch((reason) => {
    console.error(reason);
    process.exit(1);
});
