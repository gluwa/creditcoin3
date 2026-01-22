/**
 * Health check and metrics server
 *
 * Provides endpoints for Kubernetes probes and Prometheus metrics.
 */

import type { HealthStatus, Metrics } from './types.ts';

/**
 * Format metrics in Prometheus text format
 */
function formatPrometheusMetrics(metrics: Metrics): string {
  const lines: string[] = [
    '# HELP simulator_blocks_queued_total Total number of blocks added to queue',
    '# TYPE simulator_blocks_queued_total counter',
    `simulator_blocks_queued_total ${metrics.blocksQueued}`,
    '',
    '# HELP simulator_blocks_processed_total Total number of blocks processed',
    '# TYPE simulator_blocks_processed_total counter',
    `simulator_blocks_processed_total ${metrics.blocksProcessed}`,
    '',
    '# HELP simulator_proofs_submitted_total Total number of proofs submitted',
    '# TYPE simulator_proofs_submitted_total counter',
    `simulator_proofs_submitted_total ${metrics.proofsSubmitted}`,
    '',
    '# HELP simulator_single_submissions_total Total single proof submissions',
    '# TYPE simulator_single_submissions_total counter',
    `simulator_single_submissions_total ${metrics.singleSubmissions}`,
    '',
    '# HELP simulator_batch_submissions_total Total batch proof submissions',
    '# TYPE simulator_batch_submissions_total counter',
    `simulator_batch_submissions_total ${metrics.batchSubmissions}`,
    '',
    '# HELP simulator_proof_errors_total Total proof submission errors',
    '# TYPE simulator_proof_errors_total counter',
    `simulator_proof_errors_total ${metrics.proofErrors}`,
    '',
    '# HELP simulator_queue_size Current number of blocks in queue',
    '# TYPE simulator_queue_size gauge',
    `simulator_queue_size ${metrics.queueSize}`,
    '',
    '# HELP simulator_sepolia_connected Is connected to Sepolia',
    '# TYPE simulator_sepolia_connected gauge',
    `simulator_sepolia_connected ${metrics.sepoliaConnected}`,
    '',
    '# HELP simulator_cc3_connected Is connected to Creditcoin3',
    '# TYPE simulator_cc3_connected gauge',
    `simulator_cc3_connected ${metrics.cc3Connected}`,
    '',
  ];

  return lines.join('\n');
}

/**
 * Start the health check and metrics server
 */
export function startHealthServer(
  port: number,
  getStatus: () => HealthStatus,
  getMetrics: () => Metrics,
): { shutdown: () => void } {
  console.log(`🏥 Starting health server on port ${port}...`);

  const controller = new AbortController();

  Deno.serve(
    {
      port,
      signal: controller.signal,
      onListen: ({ port }) => {
        console.log(`✅ Health server listening on port ${port}`);
        console.log(`   /health  - Liveness probe`);
        console.log(`   /ready   - Readiness probe`);
        console.log(`   /metrics - Prometheus metrics`);
      },
    },
    (req) => {
      const url = new URL(req.url);
      const status = getStatus();

      // Liveness probe - always OK if server is running
      if (url.pathname === '/health') {
        return new Response('OK', { status: 200 });
      }

      // Readiness probe - check connections
      if (url.pathname === '/ready') {
        const ready = status.sepoliaConnected && status.cc3Connected;
        return new Response(
          JSON.stringify({
            ready,
            sepoliaConnected: status.sepoliaConnected,
            cc3Connected: status.cc3Connected,
          }),
          {
            status: ready ? 200 : 503,
            headers: { 'Content-Type': 'application/json' },
          },
        );
      }

      // Prometheus metrics
      if (url.pathname === '/metrics') {
        const metrics = getMetrics();
        return new Response(formatPrometheusMetrics(metrics), {
          status: 200,
          headers: { 'Content-Type': 'text/plain; charset=utf-8' },
        });
      }

      // Status endpoint with full details
      if (url.pathname === '/status') {
        return new Response(JSON.stringify(status, null, 2), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }

      return new Response('Not Found', { status: 404 });
    },
  );

  return {
    shutdown: () => {
      console.log('⏹️  Shutting down health server...');
      controller.abort();
    },
  };
}
