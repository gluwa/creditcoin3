/**
 * Configuration module for the proof traffic simulator
 *
 * Loads configuration from CLI arguments and environment variables.
 * CLI arguments take precedence over environment variables.
 */

import { parseArgs } from '@std/cli/parse-args';
import type { SimulatorConfig } from './types.ts';

/**
 * Default configuration values
 */
const DEFAULTS = {
  chainKey: 1, // Sepolia
  cc3WsUrl: 'ws://localhost:9944',
  cc3HttpUrl: 'http://localhost:9944',
  proofApiUrl: 'http://localhost:3100',
  maxQueueSize: 100,
  batchSize: 10,
  batchProbability: 0.3,
  singleEveryBlocks: 1,
  logVerbose: false,
  healthPort: 8080,
};

/**
 * Parsed CLI arguments type
 */
type ParsedArgs = Record<string, string | number | boolean | string[] | number[] | undefined>;

/**
 * Parse CLI arguments
 */
function parseCliArgs(): ParsedArgs {
  const args = parseArgs(Deno.args, {
    string: [
      'source-rpc',
      'cc3-ws',
      'cc3-http',
      'private-key',
      'api-url',
      'chain-key',
      'max-queue-size',
      'batch-size',
      'batch-probability',
      'single-every',
      'health-port',
    ],
    boolean: [
      'verbose',
    ],
    default: {},
    alias: {
      s: 'source-rpc',
      w: 'cc3-ws',
      k: 'private-key',
      a: 'api-url',
      h: 'help',
      v: 'verbose',
    },
  });

  if (args.help) {
    printHelp();
    Deno.exit(0);
  }

  // Convert to our expected type
  const result: ParsedArgs = {};
  for (const [key, value] of Object.entries(args)) {
    if (key !== '_') {
      result[key] = value as string | number | boolean | undefined;
    }
  }

  return result;
}

/**
 * Print help message
 */
function printHelp(): void {
  console.log(`
Proof Traffic Simulator

Simulates proof query traffic by streaming source chain blocks,
tracking attestations, and submitting proofs for random transactions.

USAGE:
  deno task start -- [OPTIONS]

OPTIONS:
  -s, --source-rpc <URL>    Source chain WebSocket RPC URL (required)
                            Env: SOURCE_RPC_URL
  
  -w, --cc3-ws <URL>        Creditcoin3 WebSocket URL
                            Env: CC3_WS_URL
                            Default: ws://localhost:9944
  
      --cc3-http <URL>      Creditcoin3 HTTP URL (derived from ws if not set)
                            Env: CC3_HTTP_URL
  
  -k, --private-key <KEY>   Private key for signing submissions (required)
                            Env: CC3_PRIVATE_KEY
  
  -a, --api-url <URL>       Proof generation API URL
                            Env: PROOF_API_URL
                            Default: http://localhost:3100

      --chain-key <NUM>     Source chain key (Sepolia: 1)
                            Env: CHAIN_KEY
                            Default: 1
  
      --max-queue-size <N>  Max blocks to track in queue
                            Env: MAX_QUEUE_SIZE
                            Default: 100
  
      --batch-size <N>      Max transactions per batch (random 1..N, max: 10)
                            Env: BATCH_SIZE
                            Default: 10
  
      --batch-probability <P>  Probability of batch mode (0.0-1.0)
                              Env: BATCH_PROBABILITY
                              Default: 0.3

      --single-every <N>    Submit a single proof once every N blocks
                            Env: SINGLE_EVERY_BLOCKS
                            Default: 1
  
      --health-port <PORT>  Health check server port
                            Env: HEALTH_PORT
                            Default: 8080

  -v, --verbose             Enable verbose debug logging
                               Env: LOG_VERBOSE
                               Default: false

ENVIRONMENT VARIABLES:
  CHAIN_KEY              Source chain key (default: 1 for Sepolia)
  MAX_QUEUE_SIZE         Max blocks to track (default: 100)
  BATCH_SIZE             Max txs per batch (random 1..N, default: 10)
  BATCH_PROBABILITY      Probability of batch mode, 0.0-1.0 (default: 0.3)
  SINGLE_EVERY_BLOCKS    Submit a single proof once every N blocks (default: 1)
  LOG_VERBOSE            Enable verbose debug logging (default: false)
  HEALTH_PORT            Health check server port (default: 8080)

EXAMPLES:
  # Development with local services
  deno task dev -- -s wss://sepolia.infura.io/ws/v3/YOUR_KEY -k 0x...

  # Production with environment variables
  SOURCE_RPC_URL=wss://... CC3_PRIVATE_KEY=0x... deno task start
`);
}

/**
 * Get string value from CLI args or environment
 */
function getString(
  args: Record<string, unknown>,
  argName: string,
  envName: string,
  defaultValue?: string,
): string {
  const argValue = args[argName];
  if (typeof argValue === 'string' && argValue.length > 0) {
    return argValue;
  }

  const envValue = Deno.env.get(envName);
  if (envValue && envValue.length > 0) {
    return envValue;
  }

  if (defaultValue !== undefined) {
    return defaultValue;
  }

  throw new Error(
    `Missing required configuration: --${argName} or ${envName} environment variable`,
  );
}

/**
 * Get number value from CLI args or environment
 */
function getNumber(
  args: Record<string, unknown>,
  argName: string,
  envName: string,
  defaultValue: number,
): number {
  const argValue = args[argName];
  if (typeof argValue === 'string' || typeof argValue === 'number') {
    const num = Number(argValue);
    if (!isNaN(num)) {
      return num;
    }
  }

  const envValue = Deno.env.get(envName);
  if (envValue) {
    const num = Number(envValue);
    if (!isNaN(num)) {
      return num;
    }
  }

  return defaultValue;
}

/**
 * Parse boolean environment/argument values
 */
function parseBoolean(value: string): boolean | undefined {
  const normalized = value.trim().toLowerCase();
  if (['true', '1', 'yes', 'y', 'on'].includes(normalized)) {
    return true;
  }
  if (['false', '0', 'no', 'n', 'off'].includes(normalized)) {
    return false;
  }
  return undefined;
}

/**
 * Load and validate simulator configuration
 */
export function loadConfig(): SimulatorConfig {
  const args = parseCliArgs();

  // Get CC3 WebSocket URL
  const cc3WsUrl = getString(args, 'cc3-ws', 'CC3_WS_URL', DEFAULTS.cc3WsUrl);

  // Derive HTTP URL from WS URL if not provided
  let cc3HttpUrl = getString(args, 'cc3-http', 'CC3_HTTP_URL', '');
  if (!cc3HttpUrl) {
    cc3HttpUrl = cc3WsUrl.replace(/^ws/, 'http');
  }

  // Verbose logging flag
  let logVerbose = DEFAULTS.logVerbose;
  if (args.verbose === true) {
    logVerbose = true;
  } else if (typeof args.verbose === 'string') {
    const parsed = parseBoolean(args.verbose);
    if (parsed !== undefined) {
      logVerbose = parsed;
    }
  } else {
    const envValue = Deno.env.get('LOG_VERBOSE');
    if (envValue !== undefined) {
      const parsed = parseBoolean(envValue);
      if (parsed !== undefined) {
        logVerbose = parsed;
      }
    }
  }

  const config: SimulatorConfig = {
    // Source chain
    sourceRpcUrl: getString(args, 'source-rpc', 'SOURCE_RPC_URL'),
    chainKey: getNumber(args, 'chain-key', 'CHAIN_KEY', DEFAULTS.chainKey),

    // Creditcoin3
    cc3WsUrl,
    cc3HttpUrl,
    cc3PrivateKey: getString(args, 'private-key', 'CC3_PRIVATE_KEY'),

    // Proof API
    proofApiUrl: getString(args, 'api-url', 'PROOF_API_URL', DEFAULTS.proofApiUrl),

    // Simulation parameters
    maxQueueSize: getNumber(args, 'max-queue-size', 'MAX_QUEUE_SIZE', DEFAULTS.maxQueueSize),
    batchSize: getNumber(args, 'batch-size', 'BATCH_SIZE', DEFAULTS.batchSize),
    batchProbability: getNumber(
      args,
      'batch-probability',
      'BATCH_PROBABILITY',
      DEFAULTS.batchProbability,
    ),
    singleEveryBlocks: getNumber(
      args,
      'single-every',
      'SINGLE_EVERY_BLOCKS',
      DEFAULTS.singleEveryBlocks,
    ),
    logVerbose,

    // Server
    healthPort: getNumber(args, 'health-port', 'HEALTH_PORT', DEFAULTS.healthPort),
  };

  // Validate configuration
  validateConfig(config);

  return config;
}

/**
 * Validate configuration values
 */
function validateConfig(config: SimulatorConfig): void {
  // Validate URLs
  if (!config.sourceRpcUrl.startsWith('ws://') && !config.sourceRpcUrl.startsWith('wss://')) {
    throw new Error('Source RPC URL must be a WebSocket URL (ws:// or wss://)');
  }

  if (!config.cc3WsUrl.startsWith('ws://') && !config.cc3WsUrl.startsWith('wss://')) {
    throw new Error('CC3 WebSocket URL must start with ws:// or wss://');
  }

  // Validate private key format
  if (!config.cc3PrivateKey.startsWith('0x') || config.cc3PrivateKey.length !== 66) {
    throw new Error('Private key must be a 32-byte hex string starting with 0x');
  }

  // Validate numeric ranges
  if (config.batchProbability < 0 || config.batchProbability > 1) {
    throw new Error('Batch probability must be between 0.0 and 1.0');
  }

  if (config.batchSize < 1) {
    throw new Error('Batch size must be at least 1');
  }
  if (config.batchSize > 10) {
    throw new Error('Batch size must be at most 10 (precompile limit)');
  }

  if (!Number.isInteger(config.singleEveryBlocks) || config.singleEveryBlocks < 1) {
    throw new Error('Single submission interval must be an integer >= 1');
  }

  if (config.healthPort < 1 || config.healthPort > 65535) {
    throw new Error('Health port must be between 1 and 65535');
  }
}

/**
 * Log configuration (with sensitive values masked)
 */
export function logConfig(config: SimulatorConfig): void {
  const masked = {
    ...config,
    cc3PrivateKey: config.cc3PrivateKey.slice(0, 6) + '...' + config.cc3PrivateKey.slice(-4),
  };

  console.log('\n📋 Configuration:');
  console.log('  Source chain:');
  console.log(`    RPC URL: ${masked.sourceRpcUrl}`);
  console.log(`    Chain key: ${masked.chainKey}`);
  console.log('  Creditcoin3:');
  console.log(`    WS URL: ${masked.cc3WsUrl}`);
  console.log(`    HTTP URL: ${masked.cc3HttpUrl}`);
  console.log(`    Private key: ${masked.cc3PrivateKey}`);
  console.log('  Proof API:');
  console.log(`    URL: ${masked.proofApiUrl}`);
  console.log('  Simulation:');
  console.log(`    Max queue size: ${masked.maxQueueSize}`);
  console.log(`    Max batch size: ${masked.batchSize}`);
  console.log(`    Batch probability: ${masked.batchProbability}`);
  console.log(`    Single every blocks: ${masked.singleEveryBlocks}`);
  console.log(`    Verbose logging: ${masked.logVerbose ? 'enabled' : 'disabled'}`);
  console.log('  Server:');
  console.log(`    Health port: ${masked.healthPort}`);
  console.log('');
}
