/**
 * Quoter HTTP server.
 * GET /quote?destinationChainId=X&requiresAck=true
 *
 * Start with: node server.js [--payee-address 0x...] [--payment-token 0x...] [--rpc-url https://...]
 */

import cors from "cors";
import express from "express";
import helmet from "helmet";
import morgan from "morgan";
import { createQuote } from "./quote.js";
import { loadQuoterConfig } from "./config.js";
import type { QuoterConfig } from "./config.js";
import type { QuoteRequest, SignedQuote } from "./types.js";
import { isValidBytes32 } from "../utils.js";

const app = express();

app.use(helmet());
app.use(cors());
app.use(morgan(process.env.NODE_ENV === "production" ? "combined" : "dev"));

app.get("/quote", async (req, res) => {
  const config = await getConfig();
  try {
    const messageId = req.query.messageId as string | undefined;
    const destinationChainIdParam = req.query.destinationChainId as
      | string
      | undefined;
    const requiresAck =
      (req.query.requiresAck as string)?.toLowerCase() === "true";
    const gasLimitParam = req.query.gasLimit as string | undefined;

    if (!isValidBytes32(messageId)) {
      res.status(400).json({
        error:
          "Missing or invalid messageId. Must be a 0x-prefixed bytes32 hex string.",
      });
      return;
    }

    const destinationChainId = destinationChainIdParam
      ? parseInt(destinationChainIdParam, 10)
      : config.destinationChainId;

    if (destinationChainId === undefined || isNaN(destinationChainId)) {
      res.status(400).json({
        error:
          "Missing destinationChainId. Pass it as a query param or start the server with --rpc-url to derive it.",
      });
      return;
    }

    if (
      config.destinationChainId !== undefined &&
      destinationChainId !== config.destinationChainId
    ) {
      res.status(400).json({
        error: `destinationChainId ${destinationChainId} does not match RPC chain ${config.destinationChainId}`,
      });
      return;
    }

    const request: QuoteRequest = {
      messageId,
      destinationChainId,
      requiresAck,
      gasLimit: gasLimitParam ? BigInt(gasLimitParam) : undefined,
    };

    const quote: SignedQuote = await createQuote(request, config);

    res.json({
      messageId: quote.messageId,
      relayPrice: quote.relayPrice.toString(),
      acknowledgmentPrice: quote.acknowledgmentPrice.toString(),
      payeeAddress: quote.payeeAddress,
      paymentToken: quote.paymentToken,
      expiry: quote.expiry,
      signature: quote.signature,
    });
  } catch (err) {
    console.error("Quote error:", err);
    res.status(500).json({
      error: err instanceof Error ? err.message : "Internal server error",
    });
  }
});

app.get("/health", (_req, res) => {
  res.json({ status: "ok" });
});

let configCache: QuoterConfig | null = null;
async function getConfig(): Promise<QuoterConfig> {
  if (!configCache) configCache = await loadQuoterConfig();
  return configCache;
}

const config = await getConfig();
const port = config.port;
app.listen(port, () => {
  console.log(`Quoter listening on http://localhost:${port}`);
  console.log(
    `  GET /quote?messageId=0x<bytes32>&destinationChainId=31337&requiresAck=false`,
  );
  if (config.destinationChainRpcUrl) {
    console.log(
      `  RPC: ${config.destinationChainRpcUrl} (chainId: ${config.destinationChainId})`,
    );
    console.log(`  destinationChainId optional when using --rpc-url`);
  }
  console.log(`  GET /health`);
  console.log(`  payeeAddress: ${config.payeeAddress}`);
});
