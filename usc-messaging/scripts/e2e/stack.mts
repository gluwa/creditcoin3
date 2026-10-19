import { appendFile, mkdir, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  ManagedProcess,
  FatalError,
  json,
  readJson,
  waitFor,
} from "./runtime.mjs";
import { assertPortFree, Rpc, RpcProxy } from "./rpc.mjs";

export const SOURCE_KEY =
  "0x8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b";
export const ANVIL_KEY =
  "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

export interface Deployment {
  source: {
    chainKey: number;
    outbox: string;
    ackValidator: string;
    relayerContract: string;
    attest: string;
  };
  dest: { inbox: string; voteValidator: string; dapp: string };
}

export class Stack {
  readonly repo = resolve(fileURLToPath(new URL("../../../", import.meta.url)));
  readonly usc = resolve(this.repo, "usc-messaging");
  readonly state: string;
  readonly checkpoint: string;
  readonly deployPath: string;
  readonly source: Rpc;
  readonly destination: Rpc;
  readonly sourceProxy: RpcProxy;
  readonly destinationProxy: RpcProxy;
  readonly processes: ManagedProcess[] = [];
  readonly env: NodeJS.ProcessEnv;
  readonly sourcePort = Number(process.env.E2E_SOURCE_PORT ?? 9944);
  readonly destinationPort = Number(process.env.E2E_DESTINATION_PORT ?? 8545);
  readonly proofPort = Number(process.env.E2E_PROOF_PORT ?? 3100);
  readonly quoterPort = Number(process.env.E2E_QUOTER_PORT ?? 3010);
  readonly relayerPort = Number(process.env.E2E_RELAYER_PORT ?? 3200);
  readonly sourceWs: string;
  readonly destinationWs: string;
  readonly configPath: string;
  deployment!: Deployment;
  relayer?: ManagedProcess;
  private generation = 0;

  constructor(
    readonly output: string,
    readonly lookback: number,
    readonly signal: AbortSignal,
  ) {
    this.state = resolve(output, "state");
    this.checkpoint = resolve(this.state, "relayer-checkpoints.json");
    this.deployPath = resolve(this.state, "deploy.json");
    this.configPath = resolve(this.state, "relayer.yaml");
    this.source = new Rpc(`http://127.0.0.1:${this.sourcePort}`);
    this.destination = new Rpc(`http://127.0.0.1:${this.destinationPort}`);
    this.sourceWs = `ws://127.0.0.1:${this.sourcePort}`;
    this.destinationWs = `ws://127.0.0.1:${this.destinationPort}`;
    this.sourceProxy = new RpcProxy(this.source.url);
    this.destinationProxy = new RpcProxy(this.destination.url);
    this.env = {
      ...process.env,
      PATH: `${homedir()}/.foundry/bin:${process.env.PATH ?? ""}`,
      E2E_STATE_DIR: this.state,
      E2E_DEPLOY_PATH: this.deployPath,
      E2E_ENV_FILE: resolve(this.state, "stack.env"),
      E2E_ATTESTOR_CONFIG: resolve(this.state, "attestor.yaml"),
      E2E_ATTESTOR_LOGS: resolve(this.state, "attestor-logs"),
      E2E_ATTESTOR_SET_FILE: resolve(this.state, "attestor-set"),
      E2E_ATTESTOR_READY_PATH: resolve(this.state, "attestor-ready.json"),
      E2E_ATTESTOR_BIN:
        process.env.E2E_ATTESTOR_BIN ??
        resolve(this.repo, "target/release/attestor"),
      E2E_ZOMBIENET_BIN:
        process.env.E2E_ZOMBIENET_BIN ??
        resolve(this.repo, "target/release/attestor_zombienet"),
      CREDITCOIN_RPC_URL: this.source.url,
      CREDITCOIN_SUBSTRATE_WS_URL: this.sourceWs,
      CREDITCOIN_CHAIN_PRIVATE_KEY: SOURCE_KEY,
      DESTINATION_CHAIN_RPC_URL: this.destination.url,
      DESTINATION_CHAIN_WS_URL: this.destinationWs,
      DESTINATION_CHAIN_PRIVATE_KEY: ANVIL_KEY,
      DESTINATION_CHAIN_KEY: "2",
      QUOTER_URL: `http://127.0.0.1:${this.quoterPort}`,
      E2E_QUOTER_URL: `http://127.0.0.1:${this.quoterPort}`,
      RUST_LOG: process.env.RUST_LOG ?? "info",
    };
  }

  async event(name: string, details: unknown = {}): Promise<void> {
    console.log(`[e2e] ${name}: ${JSON.stringify(details)}`);
    await appendFile(
      resolve(this.output, "events.jsonl"),
      `${JSON.stringify({ at: new Date().toISOString(), name, details })}\n`,
    );
  }

  assertAlive = (): void => {
    this.signal.throwIfAborted();
    for (const child of this.processes) child.assertAlive();
  };

  wait<T>(
    label: string,
    probe: () => Promise<T | undefined>,
    timeoutMs = 240_000,
  ): Promise<T> {
    return waitFor(label, probe, {
      timeoutMs,
      signal: this.signal,
      check: this.assertAlive,
    });
  }

  start(
    name: string,
    command: string,
    args: string[],
    env = this.env,
    cwd = this.usc,
  ): ManagedProcess {
    this.assertAlive();
    const child = new ManagedProcess(
      name,
      command,
      args,
      resolve(this.output, `${name}.log`),
      { cwd, env },
    );
    this.processes.push(child);
    return child;
  }

  async script(
    name: string,
    path: string,
    args: string[] = [],
    env = this.env,
    timeoutMs = 300_000,
  ): Promise<void> {
    const child = this.start(
      name,
      process.execPath,
      ["--import", "tsx", path, ...args],
      env,
    );
    await child.runToCompletion(timeoutMs, this.signal);
  }

  async httpReady(
    label: string,
    url: string,
    timeoutMs = 180_000,
  ): Promise<void> {
    await this.wait(
      label,
      async () => {
        const response = await fetch(url, {
          signal: AbortSignal.timeout(10_000),
        });
        return response.ok ? true : undefined;
      },
      timeoutMs,
    );
  }

  async setup(): Promise<void> {
    await mkdir(this.output, { recursive: true });
    // Refuse a stale run's state instead of deleting evidence or silently reusing its chains.
    await mkdir(this.state);
    await mkdir(resolve(this.state, "proof-gen"));
    // Never attach the deployer/assertions to another local stack while our child fails to bind.
    const ports = [
      this.sourcePort,
      this.destinationPort,
      this.proofPort,
      this.quoterPort,
      this.relayerPort,
    ];
    if (new Set(ports).size !== ports.length)
      throw new Error("E2E service ports must be distinct");
    for (const port of ports) await assertPortFree(port);
    await this.event("stack-start", {
      state: this.state,
      lookback: this.lookback,
    });
    this.start(
      "anvil",
      process.env.ANVIL_BIN ?? resolve(homedir(), ".foundry/bin/anvil"),
      [
        "--block-time",
        "2",
        "--chain-id",
        "31337",
        "--port",
        String(this.destinationPort),
      ],
    );
    this.start(
      "creditcoin",
      process.env.CREDITCOIN_BIN ??
        resolve(this.repo, "target/release/creditcoin3-node"),
      [
        "--dev",
        "--base-path",
        resolve(this.state, "creditcoin"),
        "--rpc-port",
        String(this.sourcePort),
        "--port",
        String(process.env.E2E_SOURCE_P2P_PORT ?? 30333),
        "--no-prometheus",
      ],
    );
    await this.wait("source RPC", async () =>
      (await this.source.call("eth_chainId")) === "0x2a" ? true : undefined,
    );
    await this.wait("destination RPC", async () =>
      (await this.destination.call("eth_chainId")) === "0x7a69"
        ? true
        : undefined,
    );
    await this.script("deploy-destination", "scripts/deploy-dest-ethers.mts");
    await this.script(
      "deploy-source",
      "scripts/deploy-source-ethers.mts",
      [],
      this.env,
      600_000,
    );
    this.deployment = await readJson<Deployment>(this.deployPath);
    if (!this.deployment.source?.outbox || !this.deployment.dest?.inbox)
      throw new Error("incomplete deployment manifest");
    const sourceStart = Number(
      BigInt(await this.source.call<string>("eth_blockNumber")),
    );
    const destinationStart = Number(
      BigInt(await this.destination.call<string>("eth_blockNumber")),
    );
    const addresses = {
      VOTE_VALIDATOR_ADDR: this.deployment.dest.voteValidator,
      INBOX_ADDR: this.deployment.dest.inbox,
      OUTBOX_ADDR: this.deployment.source.outbox,
      ACK_VALIDATOR_ADDR: this.deployment.source.ackValidator,
      RELAYER_CONTRACT_ADDR: this.deployment.source.relayerContract,
      DAPP_CONTRACT_ADDR: this.deployment.dest.dapp,
    };
    Object.assign(this.env, addresses);
    const shellEnv = {
      ...addresses,
      DESTINATION_CHAIN_KEY: "2",
      DESTINATION_CHAIN_RPC_URL: this.destination.url,
      DESTINATION_CHAIN_WS_URL: this.destinationWs,
      DESTINATION_CHAIN_PRIVATE_KEY: ANVIL_KEY,
      CREDITCOIN_RPC_URL: this.source.url,
      CREDITCOIN_CHAIN_PRIVATE_KEY: SOURCE_KEY,
    };
    await writeFile(
      this.env.E2E_ENV_FILE!,
      Object.entries(shellEnv)
        .map(([key, value]) => `${key}=${JSON.stringify(value)}`)
        .join("\n") + "\n",
    );
    this.start(
      "quoter",
      process.execPath,
      ["--import", "tsx", "src/quoter/index.ts"],
      {
        ...this.env,
        QUOTER_PRIVATE_KEY:
          "0x2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6",
        QUOTER_PORT: String(this.quoterPort),
        QUOTER_SOURCE_CHAIN_ID: "42",
        QUOTER_RELAYER_CONTRACT: this.deployment.source.relayerContract,
        QUOTER_EST_DELIVERY_SECS: "900",
        QUOTER_QUOTE_TTL_SECS: "600",
        QUOTER_CHAINS: JSON.stringify([
          {
            chainKey: 2,
            nativeUsd: "3000",
            gasPriceGwei: "1",
            coreFeeAttest: "1",
            ackGas: 500000,
          },
        ]),
      },
    );
    await this.httpReady("quoter", `${this.env.QUOTER_URL}/health`);
    this.start("attestors", "bash", ["scripts/launch-attestors.sh", "3"]);
    const ready = await this.wait(
      "attestor readiness and explicit boot nodes",
      async () => {
        const state = await readJson<{
          addresses: string[];
          bootNodes: string[];
        }>(this.env.E2E_ATTESTOR_READY_PATH!);
        return state.addresses?.length === 3 && state.bootNodes?.length === 3
          ? state
          : undefined;
      },
      360_000,
    );
    this.start(
      "proof-gen",
      process.env.PROOF_GEN_BIN ??
        resolve(this.repo, "target/release/proof-gen-api-server"),
      [
        "--cc3-key",
        "//Alice",
        "--cc3-rpc-url",
        this.sourceWs,
        "--eth-rpc-url",
        this.destination.url,
        "--bind-host",
        "127.0.0.1",
        "--bind-port",
        String(this.proofPort),
      ],
      { ...this.env, CHAIN_KEY: "2" },
      resolve(this.state, "proof-gen"),
    );
    await this.httpReady(
      "proof generator readiness",
      `http://127.0.0.1:${this.proofPort}/readyz`,
    );
    await this.sourceProxy.start();
    await this.destinationProxy.start();
    // JSON is YAML. Keeping this structured avoids a new serializer dependency or quoting bugs.
    await writeFile(
      this.configPath,
      json({
        bind_host: "127.0.0.1",
        bind_port: this.relayerPort,
        scan_lookback_blocks: this.lookback,
        p2p: {
          port: Number(process.env.E2E_RELAYER_P2P_PORT ?? 9300),
          boot_nodes: ready.bootNodes,
          no_mdns: true,
        },
        routes: [
          {
            chain_key: 2,
            creditcoin_chain_id: 42,
            destination_rpc_url: this.destinationProxy.url,
            inbox_address: this.deployment.dest.inbox,
            signer_key: ANVIL_KEY,
            relayer_contract_address: this.deployment.source.relayerContract,
            start_block: sourceStart,
            block_confirmation_depth: 2,
            attestor_set: {
              kind: "evm_contract",
              address: this.deployment.dest.voteValidator,
            },
            ack: {
              proof_gen_url: `http://127.0.0.1:${this.proofPort}`,
              validator_address: this.deployment.source.ackValidator,
              signer_key: SOURCE_KEY,
              start_block: destinationStart,
              confirmation_depth: 2,
            },
          },
        ],
      }),
    );
    await this.startRelayer();
  }

  async startRelayer(): Promise<void> {
    if (this.relayer && !this.relayer.exit)
      throw new FatalError("refusing to start a second relayer");
    this.relayer = this.start(
      `relayer-${++this.generation}`,
      process.env.RELAYER_BIN ??
        resolve(this.repo, "target/release/message-relayer"),
      [
        "--config",
        this.configPath,
        "--cc3-rpc-url",
        this.sourceWs,
        "--creditcoin-eth-rpc-url",
        this.sourceProxy.url,
        "--checkpoint-path",
        this.checkpoint,
      ],
    );
    await this.httpReady(
      "relayer health",
      `http://127.0.0.1:${this.relayerPort}/health`,
    );
    await this.wait(
      "relayer gossip peers",
      async () => {
        const response = await fetch(
          `http://127.0.0.1:${this.relayerPort}/metrics`,
          { signal: AbortSignal.timeout(5000) },
        );
        const text = await response.text();
        return /relayer_p2p_peer_count\{[^}]*chain_key="2"[^}]*\}\s+[1-9]\d*/.test(
          text,
        )
          ? true
          : undefined;
      },
      120_000,
    );
    await this.event("relayer-started", {
      generation: this.generation,
      pid: this.relayer.child.pid,
    });
  }

  async close(): Promise<void> {
    const errors: string[] = [];
    for (const child of [...this.processes].reverse()) {
      try {
        await child.stop();
      } catch (error) {
        errors.push(`cleanup ${child.name}: ${String(error)}`);
      }
    }
    if (this.sourceProxy.url) await this.sourceProxy.close();
    if (this.destinationProxy.url) await this.destinationProxy.close();
    if (errors.length) throw new Error(errors.join("\n"));
  }

  diagnostics(): unknown {
    return {
      processes: this.processes.map((child) => ({
        name: child.name,
        pid: child.child.pid,
        exit: child.exit,
        log: child.log,
      })),
      sourceScans: this.sourceProxy.scans,
      destinationRpc: this.destinationProxy.stats,
    };
  }
}
