/**
 * Attestations GraphQL client
 */

import { fetchWithTimeout } from "./fetch.ts";

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

export async function queryAttestation(
  graphqlUrl: string,
  chainKey: number,
  headerNumber: number,
  checkpointNumber: number,
): Promise<GraphQLAttestationResult> {
  const query = `
    query AttestationData {
      attestations(
        orderBy: HEADER_NUMBER_ASC,
        last: 1,
        filter: { chainKey: { equalTo: "${chainKey}" }, headerNumber: { equalTo: "${headerNumber}" } }
      ) {
        nodes { chainKey, headerNumber, headerHash, root, prevDigest, digest }
      }
      attestationChainData(
        orderBy: CHAIN_KEY_ASC,
        last: 1,
        filter: { chainKey: { equalTo: "${chainKey}" }, lastCheckpointHeaderNumber: { equalTo: "${checkpointNumber}" } }
      ) {
        nodes { chainKey, lastAttestedDigest, lastCheckpointHeaderNumber }
      }
    }
  `;

  const res = await fetchWithTimeout(graphqlUrl, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ query }),
  });

  if (!res.ok) {
    throw new Error(`GraphQL request failed: ${res.status}`);
  }

  const json = (await res.json()) as {
    data?: {
      attestations?: { nodes: AttestationNode[] };
      attestationChainData?: { nodes: CheckpointNode[] };
    };
  };

  const attNodes = json.data?.attestations?.nodes ?? [];
  const cpNodes = json.data?.attestationChainData?.nodes ?? [];

  return {
    attestation: attNodes[0] ?? null,
    checkpoint: cpNodes[0] ?? null,
  };
}
