import assert from "node:assert/strict";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import test from "node:test";
import { assertPortFree, Rpc, RpcProxy } from "./rpc.mjs";

test("RPC gate rejects single and batch calls without reaching upstream, then recovers", async () => {
  let forwarded = 0;
  const upstream = createServer(async (request, response) => {
    let input = "";
    for await (const chunk of request) input += String(chunk);
    const parsed = JSON.parse(input) as
      | { id: number; method: string }
      | { id: number; method: string }[];
    const calls = Array.isArray(parsed) ? parsed : [parsed];
    forwarded += calls.length;
    const replies = calls.map((call) =>
      call.method === "fail"
        ? {
            jsonrpc: "2.0",
            id: call.id,
            error: { code: -32000, message: "upstream unavailable" },
          }
        : {
            jsonrpc: "2.0",
            id: call.id,
            result: call.method === "eth_getLogs" ? [] : "0x2a",
          },
    );
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify(Array.isArray(parsed) ? replies : replies[0]));
  });
  await new Promise<void>((resolve) =>
    upstream.listen(0, "127.0.0.1", resolve),
  );
  const proxy = new RpcProxy(
    `http://127.0.0.1:${(upstream.address() as AddressInfo).port}`,
  );
  await proxy.start();
  const rpc = new Rpc(proxy.url);
  try {
    await assert.rejects(
      assertPortFree((upstream.address() as AddressInfo).port),
      /EADDRINUSE/,
    );
    assert.equal(await rpc.call("eth_chainId"), "0x2a");
    proxy.gate(true);
    await assert.rejects(rpc.call("eth_chainId"), /HTTP 503/);
    const response = await fetch(proxy.url, {
      method: "POST",
      body: JSON.stringify([
        { jsonrpc: "2.0", id: 11, method: "eth_chainId" },
        {
          jsonrpc: "2.0",
          id: 12,
          method: "eth_sendRawTransaction",
          params: ["0x1234"],
        },
      ]),
    });
    assert.equal(response.status, 503);
    const errors = (await response.json()) as {
      id: number;
      error: { code: number };
    }[];
    assert.deepEqual(
      errors.map((error) => [error.id, error.error.code]),
      [
        [11, -32099],
        [12, -32099],
      ],
    );
    assert.equal(
      forwarded,
      1,
      "blocked transaction must never reach destination RPC",
    );
    assert.equal(proxy.stats.rejected, 3);
    proxy.gate(false);
    proxy.phase = "restarted";
    const filter = { fromBlock: "0x32", toBlock: "0x33", topics: ["0xabc"] };
    assert.deepEqual(await rpc.call("eth_getLogs", [filter]), []);
    assert.deepEqual(proxy.scans, [{ phase: "restarted", filter }]);
    assert.equal(forwarded, 2);
    const invalid = await fetch(proxy.url, { method: "POST", body: "{" });
    assert.equal(invalid.status, 502);
    assert.deepEqual(await invalid.json(), {
      error: "E2E RPC proxy request failed",
    });
    assert.equal(
      await rpc.call("eth_chainId"),
      "0x2a",
      "malformed request must not crash proxy",
    );
    await assert.rejects(
      rpc.call("fail"),
      /upstream unavailable/,
      "upstream JSON-RPC errors must not be read as successful empty data",
    );
  } finally {
    await proxy.close();
    upstream.closeAllConnections();
    await new Promise<void>((resolve, reject) =>
      upstream.close((error) => (error ? reject(error) : resolve())),
    );
  }
});
