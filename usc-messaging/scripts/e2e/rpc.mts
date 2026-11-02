import { createServer, type Server } from "node:http";
import { createServer as createTcpServer, type AddressInfo } from "node:net";

export async function assertPortFree(port: number): Promise<void> {
  const server = createTcpServer();
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () =>
      server.close((error) => (error ? reject(error) : resolve())),
    );
  });
}

interface Request {
  jsonrpc: string;
  id: unknown;
  method: string;
  params?: unknown[];
}

export class Rpc {
  private id = 0;
  constructor(readonly url: string) {}
  async call<T>(method: string, params: unknown[] = []): Promise<T> {
    const response = await fetch(this.url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: ++this.id, method, params }),
      signal: AbortSignal.timeout(10_000),
    });
    if (!response.ok) throw new Error(`${method}: HTTP ${response.status}`);
    const body = (await response.json()) as { result?: T; error?: unknown };
    if (body.error || !("result" in body))
      throw new Error(`${method}: ${JSON.stringify(body)}`);
    return body.result as T;
  }
}

export interface Scan {
  phase: string;
  filter: Record<string, unknown>;
}

/** A local fault boundary used only by the relayer. Attestors and assertions use upstream RPCs. */
export class RpcProxy {
  private readonly server: Server;
  private blocked = false;
  phase = "startup";
  readonly scans: Scan[] = [];
  readonly stats = { forwarded: 0, rejected: 0 };
  url = "";

  constructor(readonly upstream: string) {
    this.server = createServer(async (request, response) => {
      try {
        let bytes = 0;
        const chunks: Buffer[] = [];
        for await (const chunk of request) {
          bytes += chunk.length;
          if (bytes > 4 * 1024 * 1024)
            throw new Error("RPC request exceeds 4 MiB");
          chunks.push(Buffer.from(chunk));
        }
        const raw = Buffer.concat(chunks).toString("utf8");
        const input = JSON.parse(raw) as Request | Request[];
        const calls = Array.isArray(input) ? input : [input];
        for (const call of calls) {
          if (call.method === "eth_getLogs") {
            if (this.scans.length >= 10_000)
              throw new Error("RPC scan evidence limit exceeded");
            this.scans.push({
              phase: this.phase,
              filter: call.params?.[0] as Record<string, unknown>,
            });
          }
        }
        response.setHeader("content-type", "application/json");
        if (this.blocked) {
          this.stats.rejected += calls.length;
          response.statusCode = 503;
          const errors = calls.map((call) => ({
            jsonrpc: "2.0",
            id: call.id,
            error: { code: -32099, message: "E2E destination RPC gate closed" },
          }));
          response.end(
            JSON.stringify(Array.isArray(input) ? errors : errors[0]),
          );
          return;
        }
        this.stats.forwarded += calls.length;
        const upstream = await fetch(this.upstream, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: raw,
          signal: AbortSignal.timeout(10_000),
        });
        response.statusCode = upstream.status;
        response.end(await upstream.text());
      } catch (error) {
        console.error("E2E RPC proxy request failed:", error);
        response.statusCode = 502;
        response.setHeader("content-type", "application/json");
        response.end(JSON.stringify({ error: "E2E RPC proxy request failed" }));
      }
    });
  }

  async start(): Promise<void> {
    await new Promise<void>((resolve, reject) => {
      this.server.once("error", reject);
      this.server.listen(0, "127.0.0.1", resolve);
    });
    this.url = `http://127.0.0.1:${(this.server.address() as AddressInfo).port}`;
  }

  gate(blocked: boolean): void {
    this.blocked = blocked;
  }

  async close(): Promise<void> {
    this.server.closeAllConnections();
    await new Promise<void>((resolve, reject) =>
      this.server.close((error) => (error ? reject(error) : resolve())),
    );
  }
}
