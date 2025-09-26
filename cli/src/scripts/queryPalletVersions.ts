import { creditcoinApi } from '../lib/api';

function camelCase(str: string) {
    return str
        .replace(/\s(.)/g, function (a) {
            return a.toUpperCase();
        })
        .replace(/\s/g, '')
        .replace(/^(.)/, function (b) {
            return b.toLowerCase();
        })
        .replace(/eVM/, function (c) {
            return c.toLowerCase();
        });
}

/**
 * @param wsUrl The URL of the node. Should be a websocket URL, like `ws://127.0.0.1:9944`
 */
async function doCollectPalletVersions(wsUrl: string): Promise<void> {
    // init the api client
    const { api } = await creditcoinApi(wsUrl);
    try {
        const metaData = JSON.parse((await api.rpc.state.getMetadata()).toString());
        // eslint-disable-next-line guard-for-in
        for (const version in metaData.metadata) {
            // eslint-disable-next-line guard-for-in
            for (const palletKey in metaData.metadata[version].pallets) {
                const pallet = metaData.metadata[version].pallets[palletKey];
                const palletName = camelCase(pallet.name);

                if (['hotfixSufficients', 'utility'].includes(palletName)) {
                    // doesn't seem to have palletVersion()
                    continue;
                }

                const storageVersion = (await api.query[palletName].palletVersion()).toString();
                console.log(`${palletName} -> ${storageVersion}`);
            }
        }
    } finally {
        await api.disconnect();
    }
}

if (process.argv.length < 3) {
    console.error('collectPalletVersions.ts <wsUrl> [optional-block-hash]');
    process.exit(1);
}

const inputWsUrl = process.argv[2];

doCollectPalletVersions(inputWsUrl).catch((reason) => {
    console.error(reason);
    process.exit(1);
});
