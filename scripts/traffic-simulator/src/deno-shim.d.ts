// Editor-only shim for non-Deno TypeScript servers.
// Deno itself provides these globals; this avoids "Cannot find name 'Deno'" in editors.
declare namespace Deno {
  const args: string[];
  const env: {
    get(key: string): string | undefined;
  };
  function exit(code?: number): never;
  function addSignalListener(signal: string, handler: () => void): void;
  function serve(
    options: {
      port?: number;
      signal?: AbortSignal;
      onListen?: (params: { hostname: string; port: number }) => void;
    },
    handler: (req: Request) => Response | Promise<Response>,
  ): void;
}
