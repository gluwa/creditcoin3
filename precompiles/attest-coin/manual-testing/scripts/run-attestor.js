'use strict';

// Step 8 — render config.yaml from .env and run the locally built attestor.
//
// `config.yaml` in this directory is the operator guide's template, kept
// verbatim with its `<placeholder>` values. Those placeholders cannot be loaded
// as-is: `chain_key` deserializes as a u64, `eth.url` / `cc3.url` as URLs, and
// `boot_nodes` as libp2p multiaddrs, so the file fails to parse before the
// attestor ever starts. This script reads the template, overrides the
// environment-specific fields from .env, drops the boot-node placeholder, and
// writes attestor-config.generated.yaml — which is what actually gets loaded.
//
// Everything else in config.yaml is passed through untouched, so `name`,
// `api.port`, `p2p.port` and any `attestation.*` overrides you add there survive.
//
// This replaces the guide's `docker run … gluwa/creditcoin3:<release-image>`
// with the binary from ./target/release, so you test the code in your tree. The
// guide's `-v $PWD/data:/data` mount has no equivalent — the attestor reads no
// data directory; only `--logs` matters.
//
// Usage:
//   node scripts/run-attestor.js                 # render, then run in the foreground
//   node scripts/run-attestor.js --config-only   # render and stop
//   node scripts/run-attestor.js -- --start-height 0    # extra attestor args

const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');
const yaml = require('js-yaml');

const HERE = path.resolve(__dirname, '..');
const ENV_PATH = path.join(HERE, '.env');
require('dotenv').config({ path: ENV_PATH, quiet: true });

const REPO_ROOT = path.resolve(HERE, '../../..');
const TEMPLATE = path.join(HERE, 'config.yaml');
const RENDERED = path.join(HERE, 'attestor-config.generated.yaml');
const LOG_DIR = path.join(HERE, 'logs');

const ATTESTOR_BIN = process.env.ATTESTOR_BIN || path.join(REPO_ROOT, 'target/release/attestor');
/** `common::constants::MIN_BALANCE` — below this the attestor refuses to start. */
const MIN_CTC = '1';

function fail(message) {
    console.error(`\nFAILED: ${message}`);
    process.exit(1);
}

function main() {
    const passthrough = process.argv.includes('--') ? process.argv.slice(process.argv.indexOf('--') + 1) : [];
    const configOnly = process.argv.includes('--config-only');

    const { CHAIN_KEY, ATTESTOR_SEED, ATTESTOR_SS58, CC3_WS_URL, ANVIL_WS_URL } = process.env;
    if (!ATTESTOR_SEED) {
        fail('ATTESTOR_SEED is not set — generate the operator account with subkey (step 5.3)');
    }
    if (!fs.existsSync(TEMPLATE)) {
        fail(`no template at ${TEMPLATE}`);
    }

    let mdnsReenabled = false;
    const config = yaml.load(fs.readFileSync(TEMPLATE, 'utf8')) || {};
    config.attestor = config.attestor || {};
    config.eth = config.eth || {};
    config.cc3 = config.cc3 || {};
    config.p2p = config.p2p || {};

    config.attestor.chain_key = Number(CHAIN_KEY || 2);
    config.attestor.secret = ATTESTOR_SEED;
    config.eth.url = ANVIL_WS_URL || 'ws://127.0.0.1:8545';
    config.cc3.url = CC3_WS_URL || 'ws://127.0.0.1:9944';

    // A multiaddr always starts with '/'. Anything else in here is the template's
    // "<attestor-boot-node-addr>" placeholder, which would fail to deserialize.
    // A single local attestor needs no peers, so an empty list is correct.
    const bootNodes = (config.p2p.boot_nodes || []).filter((n) => String(n).startsWith('/'));
    if (bootNodes.length > 0) {
        config.p2p.boot_nodes = bootNodes;
    } else {
        delete config.p2p.boot_nodes;
    }

    // The p2p task refuses to start when it can neither find a peer nor be found
    // as one: no boot nodes, mdns off, and no public_addr. Dropping the template's
    // boot-node placeholder above leaves exactly that hole, and the template also
    // ships `no_mdns: true` — which its own comment scopes to "outside a local
    // network". This is a local network, so re-enable discovery. If you supply real
    // boot nodes or a public_addr, your `no_mdns` setting is left alone.
    if (bootNodes.length === 0 && !config.attestor.public_addr && config.p2p.no_mdns) {
        config.p2p.no_mdns = false;
        mdnsReenabled = true;
    }

    fs.writeFileSync(RENDERED, yaml.dump(config, { lineWidth: 100 }), { mode: 0o600 });
    fs.mkdirSync(LOG_DIR, { recursive: true });

    // Echo the rendered config with the secret masked — the file on disk keeps
    // the real value, but it should not end up in a terminal transcript.
    const shown = yaml.dump(
        { ...config, attestor: { ...config.attestor, secret: '***' } },
        { lineWidth: 100 },
    );
    console.log(`rendered ${path.relative(process.cwd(), RENDERED)} (mode 600):\n`);
    console.log(shown.replace(/^/gm, '  '));
    if (mdnsReenabled) {
        console.log('note: no_mdns forced to false — with no boot nodes and no public_addr,');
        console.log('      the p2p task would refuse to start.\n');
    }
    console.log(`attestor account  ${ATTESTOR_SS58 || '(ATTESTOR_SS58 not set)'}`);
    console.log(`logs              ${LOG_DIR}`);
    console.log(`metrics           http://127.0.0.1:${config.api?.port ?? 9100}/metrics`);

    if (configOnly) {
        console.log(`\nRun it yourself with:\n  ${ATTESTOR_BIN} --config ${RENDERED} --logs ${LOG_DIR}`);
        return;
    }
    if (!fs.existsSync(ATTESTOR_BIN)) {
        console.log(`
No attestor binary at ${ATTESTOR_BIN}. Build it, then re-run:

  cargo build --release -p attestor

Or point ATTESTOR_BIN at an existing build.`);
        process.exit(1);
    }

    console.log(`
Starting ${path.relative(REPO_ROOT, ATTESTOR_BIN)} in the foreground (Ctrl-C to stop).
It needs at least ${MIN_CTC} CTC on the attestor account or it will exit at startup.
`);

    const child = spawn(ATTESTOR_BIN, ['--config', RENDERED, '--logs', LOG_DIR, ...passthrough], {
        stdio: 'inherit',
    });
    child.on('exit', (code, signal) => process.exit(signal ? 1 : (code ?? 0)));
    for (const sig of ['SIGINT', 'SIGTERM']) {
        process.on(sig, () => child.kill(sig));
    }
}

try {
    main();
} catch (error) {
    fail(error.message);
}
