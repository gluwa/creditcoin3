// proof-gen-api-server base URL, polled directly from the browser for attestation status
// (progress/[depositTxHash]/page.tsx) — the service ships CorsLayer::allow_origin(Any), so no
// backend proxy is needed for this POC.
export const PROOF_GEN_URL = process.env.NEXT_PUBLIC_PROOF_GEN_URL ?? "";
