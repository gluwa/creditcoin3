// Polls proof-gen-api-server's GET /api/v1/attested-height/{chainKey} directly from the browser —
// mirrors the poll shape of @gluwa/usc-sdk's ProofBuilder.waitUntilHeightAttested (used server-side
// by the claimer bot), just exposed as a query instead of an awaited promise so the UI can render
// progress rather than block on it.
import { useQuery } from "@tanstack/react-query";

import { PROOF_GEN_URL } from "@/lib/contracts/config";

interface AttestedHeightResponse {
  attestedHeight: number | null;
}

async function fetchAttestedHeight(chainKey: number): Promise<number | null> {
  const res = await fetch(
    `${PROOF_GEN_URL}/api/v1/attested-height/${chainKey}`,
  );
  if (!res.ok) throw new Error(`attested-height ${res.status}`);
  const body = (await res.json()) as AttestedHeightResponse;
  return body.attestedHeight;
}

/** True once proof-gen's cached attested height for `chainKey` reaches `targetHeight`. */
export function useAttestedHeight(
  chainKey: number | undefined,
  targetHeight: bigint | undefined,
) {
  const query = useQuery({
    queryKey: ["attested-height", chainKey],
    queryFn: () => fetchAttestedHeight(chainKey as number),
    enabled:
      Boolean(PROOF_GEN_URL) &&
      chainKey !== undefined &&
      targetHeight !== undefined,
    refetchInterval: (q) => {
      const height = q.state.data;
      if (
        targetHeight !== undefined &&
        height !== null &&
        height !== undefined &&
        BigInt(height) >= targetHeight
      ) {
        return false; // reached target, stop polling
      }
      return 15_000; // matches ProofBuilder.waitUntilHeightAttested's default pollIntervalMs
    },
  });

  const attestedHeight = query.data;
  const isAttested =
    targetHeight !== undefined &&
    attestedHeight !== null &&
    attestedHeight !== undefined &&
    BigInt(attestedHeight) >= targetHeight;

  return {
    attestedHeight,
    isAttested,
    isLoading: query.isLoading,
    error: query.error,
  };
}
