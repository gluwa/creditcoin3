/**
 * Query factory using cc-next-query-builder SDK
 *
 * Builds structured queries with selectable transaction fields
 * for proof verification. The SDK helps compose layout segments
 * that define which parts of the transaction/receipt to include.
 */

import { QueryBuilder, QueryableFields, EncodingVersion } from '@gluwa/cc-next-query-builder';
import { JsonRpcProvider } from 'ethers';
import type { QueryMode } from '../types.ts';

/**
 * ERC20 Transfer event ABI for event parsing
 */
const ERC20_TRANSFER_ABI = [
  {
    anonymous: false,
    inputs: [
      { indexed: true, name: 'from', type: 'address' },
      { indexed: true, name: 'to', type: 'address' },
      { indexed: false, name: 'value', type: 'uint256' },
    ],
    name: 'Transfer',
    type: 'event',
  },
];

/**
 * Layout segment describing which bytes to include in the proof
 */
export interface LayoutSegment {
  offset: number;
  size: number;
}

/**
 * Result of query building
 */
export interface QueryResult {
  /** The built query layout segments */
  layout: LayoutSegment[];
  /** Block number */
  blockNumber: number;
  /** Transaction index in block */
  txIndex: number;
  /** Query mode used */
  mode: QueryMode;
}

/**
 * Build a query for a transaction using the specified mode
 */
export async function buildQuery(
  provider: JsonRpcProvider,
  txHash: string,
  mode: QueryMode,
): Promise<QueryResult> {
  // Fetch transaction and receipt
  const tx = await provider.getTransaction(txHash);
  if (!tx) {
    throw new Error(`Transaction not found: ${txHash}`);
  }

  const receipt = await provider.getTransactionReceipt(txHash);
  if (!receipt) {
    throw new Error(`Transaction receipt not found: ${txHash}`);
  }

  // Create query builder from transaction
  const builder = QueryBuilder.createFromTransaction(
    tx,
    receipt,
    EncodingVersion.V1,
  );

  // Add fields based on mode
  switch (mode) {
    case 'minimal':
      addMinimalFields(builder);
      break;
    case 'transfer':
      addTransferFields(builder);
      break;
    case 'full':
      addFullFields(builder);
      break;
    case 'erc20':
      await addErc20Fields(builder);
      break;
  }

  // Build the query layout
  const layout = builder.build();

  return {
    layout,
    blockNumber: receipt.blockNumber,
    txIndex: receipt.index,
    mode,
  };
}

/**
 * Add minimal fields (just status)
 */
function addMinimalFields(builder: QueryBuilder): void {
  builder.addStaticField(QueryableFields.RxStatus);
}

/**
 * Add fields for native token transfer verification
 */
function addTransferFields(builder: QueryBuilder): void {
  builder
    .addStaticField(QueryableFields.RxStatus)
    .addStaticField(QueryableFields.TxFrom)
    .addStaticField(QueryableFields.TxTo)
    .addStaticField(QueryableFields.TxValue);
}

/**
 * Add all available transaction fields
 */
function addFullFields(builder: QueryBuilder): void {
  // Transaction fields
  builder
    .addStaticField(QueryableFields.Type)
    .addStaticField(QueryableFields.TxChainId)
    .addStaticField(QueryableFields.TxNonce)
    .addStaticField(QueryableFields.TxGasPrice)
    .addStaticField(QueryableFields.TxGasLimit)
    .addStaticField(QueryableFields.TxFrom)
    .addStaticField(QueryableFields.TxTo)
    .addStaticField(QueryableFields.TxValue)
    .addStaticField(QueryableFields.TxData);

  // Receipt fields
  builder
    .addStaticField(QueryableFields.RxStatus)
    .addStaticField(QueryableFields.RxGasUsed);
}

/**
 * Add ERC20 Transfer event fields
 */
async function addErc20Fields(builder: QueryBuilder): Promise<void> {
  // Set up ABI provider for ERC20
  builder.setAbiProvider((_contractAddress: string) => {
    return Promise.resolve(JSON.stringify(ERC20_TRANSFER_ABI));
  });

  // Add the Transfer event with all arguments
  // Filter function accepts any Transfer event
  await builder.eventBuilder(
    'Transfer',
    (_log, logDescription, _index) => {
      // Accept all Transfer events
      return logDescription.name === 'Transfer';
    },
    (eventBuilder) => {
      eventBuilder
        .addSignature()
        .addArgument('from')
        .addArgument('to')
        .addArgument('value');
    },
  );

  // Also add basic receipt status
  builder.addStaticField(QueryableFields.RxStatus);
}

/**
 * Get a description of what fields a query mode includes
 */
export function describeQueryMode(mode: QueryMode): string {
  switch (mode) {
    case 'minimal':
      return 'Receipt status only';
    case 'transfer':
      return 'From, To, Value, Status';
    case 'full':
      return 'All transaction and receipt fields';
    case 'erc20':
      return 'ERC20 Transfer event (from, to, value)';
  }
}

/**
 * Create a provider for querying the source chain
 */
export function createSourceProvider(httpUrl: string): JsonRpcProvider {
  return new JsonRpcProvider(httpUrl);
}

/**
 * Format layout segments for logging
 */
export function formatLayout(layout: LayoutSegment[]): string {
  return layout
    .map((seg) => `[${seg.offset}:${seg.offset + seg.size}]`)
    .join(', ');
}
