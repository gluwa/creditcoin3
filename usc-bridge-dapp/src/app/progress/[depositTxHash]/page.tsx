import { ProgressView } from "./progress-view";

export default async function ProgressPage({
  params,
}: {
  params: Promise<{ depositTxHash: string }>;
}) {
  const { depositTxHash } = await params;
  return <ProgressView depositTxHash={depositTxHash as `0x${string}`} />;
}
