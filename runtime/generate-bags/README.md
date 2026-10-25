# Usage

The bag thresholds shouldn't really need to be regenerated, the only situation where it would make sense to do so
is if the distribution of stakers across bags becomes very unbalanced (ideally, stakers would be
rougly evenly divided across the bags).

To generate the bag thresholds, run (from the root of the repo):

```bash

cargo run --release --bin generate-bags -- --total-issuance 100 --output ./runtime/src/output.rs

```
