/**
 * Attestations GraphQL client
 *
 * Retries when the indexer lags behind chain head: on-chain reads succeed first,
 * but GraphQL may return empty nodes until the indexer catches up.
 */

import { fetchWithTimeout } from "./fetch.ts";

/** Attempts when data is missing or request fails (HTTP / GraphQL errors). */
const GRAPHQL_RETRY_ATTEMPTS = 5;
/** Delay between attempts (indexer catch-up). */
const GRAPHQL_RETRY_DELAY_MS = 5000;

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export interface AttestationNode {
  headerNumber: string;
  root: string;
  prevDigest: string;
  digest: string;
}

export interface CheckpointNode {
  lastCheckpointHeaderNumber: string;
  lastAttestedDigest: string;
}

export interface GraphQLAttestationResult {
  attestation: AttestationNode | null;
  checkpoint: CheckpointNode | null;
}

async function queryAttestationOnce(
  graphqlUrl: string,
  chainKey: number,
  headerNumber: number,
  checkpointNumber: number,
): Promise<GraphQLAttestationResult> {
  const query = `
    query AttestationData($chainKey: BigFloat!, $headerNumber: BigFloat!, $checkpointNumber: BigFloat!) {
      attestations(
        orderBy: HEADER_NUMBER_ASC,
        last: 1,
        filter: { chainKey: { equalTo: $chainKey }, headerNumber: { equalTo: $headerNumber } }
      ) {
        nodes { chainKey, headerNumber, headerHash, root, prevDigest, digest }
      }
      attestationChainData(
        orderBy: CHAIN_KEY_ASC,
        last: 1,
        filter: { chainKey: { equalTo: $chainKey }, lastCheckpointHeaderNumber: { equalTo: $checkpointNumber } }
      ) {
        nodes { chainKey, lastAttestedDigest, lastCheckpointHeaderNumber }
      }
    }
  `;

  const variables = {
    chainKey,
    headerNumber,
    checkpointNumber,
  };

  const res = await fetchWithTimeout(graphqlUrl, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ query, variables }),
  });

  if (!res.ok) {
    throw new Error(`GraphQL request failed: ${res.status}`);
  }

  const json = (await res.json()) as {
    data?: {
      attestations?: { nodes: AttestationNode[] };
      attestationChainData?: { nodes: CheckpointNode[] };
    };
    errors?: { message: string }[];
  };

  if (json.errors?.length) {
    throw new Error(
      `GraphQL errors: ${json.errors.map((e) => e.message).join("; ")}`,
    );
  }

  const attNodes = json.data?.attestations?.nodes ?? [];
  const cpNodes = json.data?.attestationChainData?.nodes ?? [];

  return {
    attestation: attNodes[0] ?? null,
    checkpoint: cpNodes[0] ?? null,
  };
}

/**
 * Fetches attestation + checkpoint rows for the given chain/header/checkpoint numbers.
 * Retries when either row is missing (indexer lag) or the HTTP/GraphQL request fails.
 */
export async function queryAttestation(
  graphqlUrl: string,
  chainKey: number,
  headerNumber: number,
  checkpointNumber: number,
): Promise<GraphQLAttestationResult> {
  let lastResult: GraphQLAttestationResult = {
    attestation: null,
    checkpoint: null,
  };

  for (let attempt = 1; attempt <= GRAPHQL_RETRY_ATTEMPTS; attempt++) {
    try {
      lastResult = await queryAttestationOnce(
        graphqlUrl,
        chainKey,
        headerNumber,
        checkpointNumber,
      );
      if (lastResult.attestation && lastResult.checkpoint) {
        return lastResult;
      }
      if (attempt < GRAPHQL_RETRY_ATTEMPTS) {
        const missing: string[] = [];
        if (!lastResult.attestation) missing.push("attestation");
        if (!lastResult.checkpoint) missing.push("checkpoint");
        console.warn(
          `⚠️ GraphQL indexer: ${missing.join(" and ")} not found yet ` +
          `(attempt ${attempt}/${GRAPHQL_RETRY_ATTEMPTS}), retrying in ${GRAPHQL_RETRY_DELAY_MS}ms...`,
        );
        await sleep(GRAPHQL_RETRY_DELAY_MS);
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (attempt === GRAPHQL_RETRY_ATTEMPTS) {
        throw e;
      }
      console.warn(
        `⚠️ GraphQL request failed (attempt ${attempt}/${GRAPHQL_RETRY_ATTEMPTS}): ${msg} — retrying in ${GRAPHQL_RETRY_DELAY_MS}ms...`,
      );
      await sleep(GRAPHQL_RETRY_DELAY_MS);
    }
  }

  return lastResult;
}
