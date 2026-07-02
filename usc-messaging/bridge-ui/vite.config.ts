import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";

// Dev-server proxies: the browser talks to same-origin paths, Vite forwards to the target services.
// This sidesteps CORS on the RPCs and the proof-gen API (none set CORS headers for a browser origin).
// Targets come from .env so you can point at local (default) or a live network (e.g. usc-dev/Sepolia):
//   VITE_RPC_CC        Creditcoin EVM RPC        (default http://127.0.0.1:9944)
//   VITE_RPC_DEST      destination chain RPC     (default http://127.0.0.1:8545)
//   VITE_PROOFGEN      proof-gen API base URL    (default http://127.0.0.1:3100)
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const ccTarget = env.VITE_RPC_CC || "http://127.0.0.1:9944";
  const destTarget = env.VITE_RPC_DEST || "http://127.0.0.1:8545";
  const proofTarget = env.VITE_PROOFGEN || "http://127.0.0.1:3100";
  return {
    plugins: [react()],
    server: {
      port: 5174,
      proxy: {
        "/rpc/dest": {
          target: destTarget,
          changeOrigin: true,
          secure: false,
          rewrite: (p) => p.replace(/^\/rpc\/dest/, ""),
        },
        "/rpc/cc": {
          target: ccTarget,
          changeOrigin: true,
          secure: false,
          rewrite: (p) => p.replace(/^\/rpc\/cc/, ""),
        },
        "/proofgen": {
          target: proofTarget,
          changeOrigin: true,
          secure: false,
          rewrite: (p) => p.replace(/^\/proofgen/, ""),
        },
      },
    },
  };
});
