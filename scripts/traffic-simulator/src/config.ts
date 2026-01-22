/**
 * Configuration module for the proof traffic simulator
 *
 * Loads configuration from CLI arguments and environment variables.
 * CLI arguments take precedence over environment variables.
 */

import { parseArgs } from '@std/cli/parse-args';
import type { QueryMode, SimulatorConfig } from './types.ts';

/**
 * Default configuration values
 */
const DEFAULTS = {
  chainKey: 1, // Sepolia
  cc3WsUrl: 'ws://localhost:9944',
  cc3HttpUrl: 'http://localhost:9944',
  proofApiUrl: 'http://localhost:3100',
  maxQueueSize: 100,
  txPerBlock: 2,
  batchSize: 3,
  batchProbability: 0.3,
  queryMode: 'transfer' as QueryMode,
  enableQueryBuilder: true,
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
      'query-mode',
      'chain-key',
      'max-queue-size',
      'tx-per-block',
      'batch-size',
      'batch-probability',
      'health-port',
    ],
    boolean: [
      'enable-query-builder',
      'disable-query-builder',
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
  
      --query-mode <MODE>   Query complexity mode
                            Options: minimal, transfer, full, erc20
                            Env: QUERY_MODE
                            Default: transfer
  
      --chain-key <NUM>     Source chain key (Sepolia: 1)
                            Env: CHAIN_KEY
                            Default: 1
  
      --max-queue-size <N>  Max blocks to track in queue
                            Env: MAX_QUEUE_SIZE
                            Default: 100
  
      --tx-per-block <N>    Transactions per block to submit
                            Env: TX_PER_BLOCK
                            Default: 2
  
      --batch-size <N>      Transactions per batch (max: 10)
                            Env: BATCH_SIZE
                            Default: 3
  
      --batch-probability <P>  Probability of batch mode (0.0-1.0)
                              Env: BATCH_PROBABILITY
                              Default: 0.3
  
      --health-port <PORT>  Health check server port
                            Env: HEALTH_PORT
                            Default: 8080
  
      --enable-query-builder   Enable query builder logging
                               Env: ENABLE_QUERY_BUILDER
                               Default: true
  
      --disable-query-builder  Disable query builder logging
                               Env: ENABLE_QUERY_BUILDER
                               Default: true
  
  -v, --verbose               Enable verbose debug logging
                               Env: LOG_VERBOSE
                               Default: false

ENVIRONMENT VARIABLES:
  CHAIN_KEY              Source chain key (default: 1 for Sepolia)
  MAX_QUEUE_SIZE         Max blocks to track (default: 100)
  TX_PER_BLOCK           Transactions per block to submit (default: 2)
  BATCH_SIZE             Transactions per batch (default: 3)
  BATCH_PROBABILITY      Probability of batch mode, 0.0-1.0 (default: 0.3)
  ENABLE_QUERY_BUILDER   Build/log query layouts (default: true)
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

  // Get query mode and validate
  const queryModeStr = getString(args, 'query-mode', 'QUERY_MODE', DEFAULTS.queryMode);
  const validModes: QueryMode[] = ['minimal', 'transfer', 'full', 'erc20'];
  if (!validModes.includes(queryModeStr as QueryMode)) {
    throw new Error(`Invalid query mode: ${queryModeStr}. Must be one of: ${validModes.join(', ')}`);
  }

  // Query builder flag (default true)
  const disableQueryBuilder = args['disable-query-builder'] === true;
  const enableQueryBuilderFlag = args['enable-query-builder'] === true;
  let enableQueryBuilder = DEFAULTS.enableQueryBuilder;
  if (disableQueryBuilder) {
    enableQueryBuilder = false;
  } else if (enableQueryBuilderFlag) {
    enableQueryBuilder = true;
  } else {
    const envValue = Deno.env.get('ENABLE_QUERY_BUILDER');
    if (envValue !== undefined) {
      const parsed = parseBoolean(envValue);
      if (parsed !== undefined) {
        enableQueryBuilder = parsed;
      }
    }
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
    txPerBlock: getNumber(args, 'tx-per-block', 'TX_PER_BLOCK', DEFAULTS.txPerBlock),
    batchSize: getNumber(args, 'batch-size', 'BATCH_SIZE', DEFAULTS.batchSize),
    batchProbability: getNumber(
      args,
      'batch-probability',
      'BATCH_PROBABILITY',
      DEFAULTS.batchProbability,
    ),
    queryMode: queryModeStr as QueryMode,
    enableQueryBuilder,
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

  if (config.txPerBlock < 1) {
    throw new Error('Transactions per block must be at least 1');
  }

  if (config.batchSize < 1) {
    throw new Error('Batch size must be at least 1');
  }
  if (config.batchSize > 10) {
    throw new Error('Batch size must be at most 10 (precompile limit)');
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
  console.log(`    Txs per block: ${masked.txPerBlock}`);
  console.log(`    Batch size: ${masked.batchSize}`);
  console.log(`    Batch probability: ${masked.batchProbability}`);
  console.log(`    Query mode: ${masked.queryMode}`);
  console.log(`    Query builder: ${masked.enableQueryBuilder ? 'enabled' : 'disabled'}`);
  console.log(`    Verbose logging: ${masked.logVerbose ? 'enabled' : 'disabled'}`);
  console.log('  Server:');
  console.log(`    Health port: ${masked.healthPort}`);
  console.log('');
}
