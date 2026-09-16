import { ClaimView } from "./claim-view";

export default async function ClaimPage({
  params,
}: {
  params: Promise<{ depositTxHash: string }>;
}) {
  const { depositTxHash } = await params;
  return <ClaimView depositTxHash={depositTxHash as `0x${string}`} />;
}
