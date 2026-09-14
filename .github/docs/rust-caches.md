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
and a hash of build options, LFS mode, metadata-patching mode, the runner family
and `$HOME`. Release, benchmarking and migration configurations therefore cannot
win the same immutable entry as a fast-runtime build, and a hosted archive cannot
win the entry a self-hosted consumer needs. Swatinem additionally keys by compiler
versions, environment and dependencies.

The runner family and `$HOME` are in the hash on purpose. `target/` fingerprints
and `.d` files record absolute paths, so an archive restored across runner families
fails every fingerprint check and rebuilds anyway after paying for the download.
The other inputs must not be relied on to separate them: today every hosted caller
passes `git-lfs-checkout: true` and every Linode one `false`, which separates them
by coincidence rather than by design.

Keep each producer on its consumer's runner family, and update both sides when
changing build options or checkout settings.

Formatting and auditing have separate tool caches without `target/`. Machete uses
a prebuilt tool without a Cargo cache. The benchmark workflow retains its cache:
it also recompiles the generated weights after running the downloaded binary.

## Execution and measurement

Normal PR and manually dispatched Rust checks always run. Only background Rust
warmers, and native calls explicitly opting into `warm-cache-only`, skip work on
an exact cache hit. Cache misses still build the real configuration before saving.

The coverage warmer is the one exception to "build the real thing": it runs
`cargo llvm-cov --no-report -- --list`, which compiles every target under
llvm-cov's own instrumentation flags and then exits through libtest's `--list`
instead of executing the suite and the 10 000-case simulations. Plain `cargo test`
would not do: it sets different RUSTFLAGS and would populate artifacts the real
run cannot reuse. (`cargo llvm-cov --no-run` is the opposite of what the name
suggests — it reports from existing profile data without building.) Warm runs
therefore publish no coverage report and push no simulation regression files.

Native warmers do not upload binaries, and their Linode runner is cleaned up after
success, failure or cancellation. The self-hosted leg is skipped on `pull_request`:
a PR touching these files only needs to prove they still parse and that the hosted
leg works, and deploying a VM for a full release build per PR is the most expensive
and least reliable path in CI.

Every cached job writes an "Exact cache hit" line into its job summary. Without it
the only way to tell whether any of this is working is to open a job log and search
for the restore line, which is how the previous single-key setup stayed broken
unnoticed.

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
