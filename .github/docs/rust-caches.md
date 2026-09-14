# Rust CI caches

`rust.yml` and `cache-native.yml` populate dependency caches from `usc-dev` after
manifest, lockfile, toolchain or workflow changes. Weekday maintenance runs refill
evicted entries and account for hosted runner image updates. Both workflows can
also be dispatched manually on `usc-dev`.

Caches created by `pull_request` runs belong to `refs/pull/<number>/merge`; another
PR cannot read them. Base-branch producers are necessary even when the cache key
is shared. A dependency-changing PR can still save its own cache for subsequent
runs before it merges.

## Configurations

| Producer | Cache | Consumers |
| --- | --- | --- |
| `rust.yml` / cargo-fmt, cargo-audit | Separate tools-only caches | Formatting and audit tools |
| `rust.yml` / cargo-check | `rust-check` | Default and benchmarking checks |
| `rust.yml` / cargo-clippy | `rust-clippy` | All-features Clippy |
| `rust.yml` / cargo-test | `rust-test-coverage` | Workspace coverage and simulations |
| `cache-native.yml` / hosted | Native fast-runtime, LFS on | Attestor and validator compatibility |
| `cache-native.yml` / linode | Native fast-runtime, LFS off | Main CI and proof-generator native builds |

The native namespace includes the caller's cache name (with a consistent default)
and a hash of build options, LFS mode and metadata-patching mode. Release,
benchmarking and migration configurations therefore cannot win the same immutable
entry as a fast-runtime build. Swatinem additionally keys by compiler versions,
environment and dependencies.

Hosted and Linode producers are intentional: their Cargo home and checkout paths
and installed toolchains differ. Giving them the same textual shared key does not
make their archives interchangeable. Keep each producer on its consumer's runner
family, and update both sides when changing build options or checkout settings.

Formatting and auditing have separate tool caches without `target/`. Machete uses
a prebuilt tool without a Cargo cache. The benchmark workflow retains its cache:
it also recompiles the generated weights after running the downloaded binary.

## Execution and measurement

Normal PR and manually dispatched Rust checks always run. Only background Rust
warmers, and native calls explicitly opting into `warm-cache-only`, skip work on
an exact cache hit. Cache misses still build the real configuration before saving.
Background coverage runs do not push simulation regression files to `usc-dev`.
Native warmers do not upload binaries, and their Linode runner is cleaned up after
success, failure or cancellation.

After merge:

1. Run both producers on `usc-dev` and confirm the cache-save steps succeed.
2. Open a fresh source-only PR with unchanged dependencies. Check the cache restore
   logs in Rust and native consumers; a rerun of the producer's own PR does not
   demonstrate cross-PR reuse.
3. Compare restore time, Cargo compilation time and total job time separately.
   Native jobs also publish exact-hit status in their job summary.
4. Monitor cache sizes and eviction. Each configuration needs a separate archive;
   warmers skip compilation on exact hits but still incur setup and restore costs.

Swatinem caches dependencies, not the finished workspace binaries. This change
reduces cold dependency builds; each consumer still compiles the current checkout.
Sharing one completed build among independent workflows is a separate artifact
orchestration change. Docker's build cache is also separate from these Cargo caches.

References:

- [GitHub cache access restrictions](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching#restrictions-for-accessing-a-cache)
- [Swatinem cache behavior](https://github.com/Swatinem/rust-cache#cache-details)
