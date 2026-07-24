import express, { type Request, type Response } from "express";
import helmet from "helmet";
import { ethers } from "ethers";
import type { QuoterConfig } from "./config.js";
import { priceFee } from "./pricing.js";
import { signQuote, type QuoteFields } from "./quote.js";

interface QuoteRequestBody {
  destinationChain?: number;
  targetContract?: string;
  /** Either the raw payload (hex) — the quoter hashes it — or a precomputed payloadHash. */
  payload?: string;
  payloadHash?: string;
  gasLimit?: string | number;
  requiresAck?: boolean;
}

export function buildServer(cfg: QuoterConfig): express.Express {
  const wallet = new ethers.Wallet(cfg.privateKey);
  const app = express();
  app.use(helmet());
  app.use(express.json());

  app.get("/health", (_req: Request, res: Response) => {
    res.json({
      status: "ok",
      quoter: wallet.address,
      attestUsd: cfg.attestUsd,
      chains: [...cfg.chains.keys()],
    });
  });

  app.post("/quote", async (req: Request, res: Response) => {
    try {
      const body = req.body as QuoteRequestBody;

      const chainId = Number(body.destinationChain);
      const chain = cfg.chains.get(chainId);
      if (!chain)
        return res
          .status(400)
          .json({ error: `unsupported destinationChain ${chainId}` });

      if (!body.targetContract || !ethers.isAddress(body.targetContract))
        return res
          .status(400)
          .json({ error: "targetContract must be a valid address" });

      let payloadHash: string;
      if (body.payloadHash) {
        payloadHash = body.payloadHash;
      } else if (body.payload) {
        payloadHash = ethers.keccak256(body.payload);
      } else {
        return res
          .status(400)
          .json({ error: "provide payload or payloadHash" });
      }
      if (!ethers.isHexString(payloadHash, 32))
        return res
          .status(400)
          .json({ error: "payloadHash must be 32-byte hex" });

      const gasLimit = BigInt(body.gasLimit ?? 0);
      if (gasLimit <= 0n)
        return res.status(400).json({ error: "gasLimit must be > 0" });

      const requiresAck = Boolean(body.requiresAck);
      const fee = await priceFee(chain, cfg, gasLimit, requiresAck);

      const now = BigInt(Math.floor(Date.now() / 1000));
      const fields: QuoteFields = {
        coreFee: fee.coreFee,
        relayPrice: fee.relayPrice,
        acknowledgmentPrice: fee.acknowledgmentPrice,
        gasLimit,
        destinationChain: chainId,
        requiresAck,
        payloadHash,
        targetContract: ethers.getAddress(body.targetContract),
        expectedCompletion: now + BigInt(cfg.estimatedDeliverySecs),
        expiry: now + BigInt(cfg.quoteTtlSecs),
      };

      const signed = await signQuote(wallet, fields, {
        sourceChainId: cfg.sourceChainId,
        verifyingContract: cfg.relayerContract,
      });
      const total = fee.coreFee + fee.relayPrice + fee.acknowledgmentPrice;

      // Serialize bigints as strings (JSON can't hold them).
      res.json({
        quoter: wallet.address,
        signedQuote: signed.signedQuote,
        quote: {
          coreFee: fee.coreFee.toString(),
          relayPrice: fee.relayPrice.toString(),
          acknowledgmentPrice: fee.acknowledgmentPrice.toString(),
          gasLimit: gasLimit.toString(),
          destinationChain: chainId,
          requiresAck,
          payloadHash,
          targetContract: fields.targetContract,
          expectedCompletion: fields.expectedCompletion.toString(),
          expiry: fields.expiry.toString(),
          signature: signed.signature,
        },
        totalAttest: total.toString(),
        gasPriceWei: fee.gasPriceWei.toString(),
      });
    } catch (err) {
      console.error("quote error:", err);
      res.status(500).json({ error: "internal error building quote" });
    }
  });

  return app;
}
