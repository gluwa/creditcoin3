# EVM block weight accounting

Ethereum transaction admission reserves execution weight plus the cost of assembling its
transaction, receipt, status and bloom into the final block. The reservation uses the gas
limit before dispatch. After execution, unused gas is reclaimed while the assembly cost
for consumed gas remains charged. Execution's measured proof usage does not reclaim the
assembly proof reservation. The EVM runner receives its original execution-only budget.

`on_initialize` reserves the fixed assembly cost, including empty blocks. Imported Ethereum
pre-log batches also check execution plus assembly weight before each transaction.

Opcode gas costs and the advertised 75M block gas limit stay the same. The
additional weight reduces how much Ethereum execution fits inside the normal dispatch
budget, particularly on devnet and fast-runtime. Other transactions still use their
existing weight rules. The base-fee formula is unchanged; when elasticity is enabled,
it observes the additional block weight and can increase fees at lower gas volumes.
Ship this as runtime **3.140.0 / spec 140**.

## Measurements and model

The committed coefficients are from the successful [benchmark run on 13 October 2026](https://github.com/gluwa/creditcoin3/actions/runs/38385540862),
using compiled Wasm on AMD EPYC 7713. These are later, more conservative measurements than
the September run. The originals are available in its `pallet-ethereum-weights` artifact.
The files under `runtime/src/weights/ethereum/` retain the CLI's times, database operations,
proof sizes and component ranges. Their methods are adapted to the runtime's benchmark
adapter rather than a Frontier weight trait.

The model takes the largest marginal reference time per gas across transaction count,
logs, topics, log data, calldata and dirty storage. It includes the maximum 20% gas refund
and adds 25% measurement headroom. The dirty-storage fixture's explicit database writes
are subtracted because EVM execution already pays for those writes. Minimum gas prices
are documented alongside the model. Fixed cost uses the largest measured intercept with
25% headroom. Proof size reserves half a byte per consumed gas, above the measured slopes
after the same refund and margin factors.

Reference time remains a hardware calibration. The state-root fixture uses a shallow test
trie; remeasure against representative deployed state before reducing the margin. The
runtime remains a solo chain with its existing permissive global proof-size limit; this
change records assembly proof weight without introducing a new global limit.

## Reproduce

Build with `WASM_BUILD_WORKSPACE_HINT="$PWD" cargo build --locked --release --features runtime-benchmarks`.
The workspace hint keeps the Wasm build pinned when using an external target directory.
Generate individual
coefficients with:

```sh
./target/release/creditcoin3-node benchmark pallet --chain dev \
  --pallet pallet_ethereum --wasm-execution compiled --heap-pages 2048 \
  --steps 50 --repeat 20 \
  --extrinsic on_finalize_transactions,on_finalize_logs,on_finalize_topics,on_finalize_log_data,on_finalize_transaction_size,on_finalize_state_root \
  --template .github/ethereum-weights.hbs \
  --output runtime/src/weights/ethereum/single_component.rs
```

Run `on_finalize_gas_block` with `--steps 10 --repeat 5` to exercise a combined workload
sharing one gas budget. Run `on_finalize_full_block` separately for the larger synthetic
stress case. Its independent maxima cost several blocks' worth of gas and must not be
treated as one admissible block. Individual dimensions and `on_finalize_gas_block` use
2048 heap pages, the default 128 MiB import heap. The larger diagnostic retains 10000 pages.

The archived runtime completed the gas-bounded combined workload with that heap. Its
independent-maxima fixture exhausted the heap in an aggregate-vector clone. Frontier's
assembly path now borrows receipts and stored aggregates and moves transactions into the
block, avoiding those unnecessary copies. The independent-maxima fixture still exceeds
128 MiB during receipt-trie construction after that change. Keep its larger-heap diagnostic
separate from the production-heap checks; do not infer an admissible block's memory demand
from that synthetic workload.

CI regenerates the committed coefficients using the inherent-method template, checks that
they compile, and preserves the outputs as an artifact. Review regenerated coefficients
and rerun the weight-envelope tests before merging any calibration update.

## Validation

The Frontier regression suite checks admission rejection, preservation of reference time
and proof size after refunds, the fixed reservation, and pre-log admission. Runtime tests
check that its configured reservation covers the committed individual and combined
measurements and that the runner cannot spend the assembly allowance on EVM execution.

Run the runtime checks under production, devnet and fast-runtime profiles. Run the eight
benchmark fixtures with `--features runtime-benchmarks`, then execute the Wasm heap tests;
native unit tests alone do not enforce the import heap limit.
