#!/usr/bin/env python3
"""Generate the Anvil state that the attestor-network job loads as its Anvil-1 chain.

The job needs a backlog of ~10k blocks for the attestors to catch up on before they follow
the tip. It does not need many transactions in them: each one makes the archiver, the
checkpoint builder and the verifier fetch and hash more data, and the previous fixture's
~50 transactions per block made a 4.8 GB state. Two per block keep the transaction and
receipt roots non-trivial.

Usage, with the same Anvil version the job pins (see "Install Foundry" in ci.yml):

    .github/generate-anvil-fixture.py --anvil ~/.foundry/bin/anvil --out anvil10k_blocks.json
    xz -9 -T0 anvil10k_blocks.json

then upload anvil10k_blocks.json.xz as a release asset and point ci.yml at it.
"""

import argparse
import hashlib
import http.client
import json
import signal
import subprocess
import sys
import time

CHAIN_ID = 31337
# Same genesis time as the previous fixture, so a regenerated state is reproducible.
GENESIS_TIMESTAMP = 1776157459
SINK = "0x000000000000000000000000000000000000dEaD"


class Rpc:
    def __init__(self, port):
        self.conn = http.client.HTTPConnection("127.0.0.1", port, timeout=60)
        self.next_id = 0

    def call(self, method, *params):
        self.next_id += 1
        body = json.dumps({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": list(params)})
        self.conn.request("POST", "/", body, {"Content-Type": "application/json"})
        reply = json.loads(self.conn.getresponse().read())
        if "error" in reply:
            raise RuntimeError(f"{method}: {reply['error']}")
        return reply["result"]


def wait_for_anvil(port):
    for _ in range(100):
        try:
            Rpc(port).call("eth_chainId")
            return
        except OSError:
            time.sleep(0.1)
    sys.exit("anvil did not start")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--anvil", default="anvil", help="anvil binary")
    parser.add_argument("--out", default="anvil10k_blocks.json", help="state file to write")
    parser.add_argument("--blocks", type=int, default=10005, help="blocks to mine after genesis")
    parser.add_argument("--txs-per-block", type=int, default=2)
    parser.add_argument("--port", type=int, default=18545)
    args = parser.parse_args()

    anvil = subprocess.Popen(
        [
            args.anvil,
            "--chain-id", str(CHAIN_ID),
            "--port", str(args.port),
            "--no-mining",
            "--timestamp", str(GENESIS_TIMESTAMP),
            "--dump-state", args.out,
        ],
        stdout=subprocess.DEVNULL,
    )
    try:
        wait_for_anvil(args.port)
        rpc = Rpc(args.port)
        accounts = rpc.call("eth_accounts")
        rpc.call("anvil_setBlockTimestampInterval", 1)

        for block in range(1, args.blocks + 1):
            for i in range(args.txs_per_block):
                sender = accounts[(block * args.txs_per_block + i) % len(accounts)]
                payload = hashlib.sha256(f"{block}:{i}".encode()).hexdigest()
                rpc.call("eth_sendTransaction", {"from": sender, "to": SINK, "value": "0x0", "data": "0x" + payload})
            rpc.call("evm_mine")
            if block % 1000 == 0:
                print(f"mined {block}/{args.blocks}", flush=True)

        head = int(rpc.call("eth_blockNumber"), 16)
        txs = int(rpc.call("eth_getBlockTransactionCountByNumber", hex(head)), 16)
        if head != args.blocks or txs != args.txs_per_block:
            sys.exit(f"unexpected head {head} with {txs} transactions")
    finally:
        # Anvil writes --dump-state on a graceful exit.
        anvil.send_signal(signal.SIGTERM)
        anvil.wait(timeout=600)
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
