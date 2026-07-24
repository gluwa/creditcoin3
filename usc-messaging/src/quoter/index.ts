// Phase-1 USC write-ability quoter service.
// Serves signed fee quotes (static ATTEST/USD pricing) that RelayerContract accepts.
//   POST /quote  { destinationChain, targetContract, payload|payloadHash, gasLimit, requiresAck }
//   GET  /health
import { loadConfig } from "./config.js";
import { buildServer } from "./server.js";

const cfg = loadConfig();
const app = buildServer(cfg);

app.listen(cfg.port, () => {
  console.log(
    `🧮 USC quoter listening on :${cfg.port} — ATTEST=$${cfg.attestUsd}, chains=[${[
      ...cfg.chains.keys(),
    ].join(", ")}]`,
  );
});
