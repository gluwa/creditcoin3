//! Solidity ↔ Rust ABI-surface drift detector.
//!
//! `src/abi.rs` is a hand-trimmed `sol!` mirror of the asc-contracts surface the attestor calls.
//! Nothing enforces that the mirror stays byte-identical with the contracts: creditcoin3↔relayer
//! agreement is policed by the shared golden vectors, but Solidity↔Rust drift was only caught
//! implicitly, three layers deep, when the e2e's delivery reverted (or worse, when a topic filter
//! went silent — an event signature change moves topic0 and a subscription just stops matching).
//!
//! This test closes that leg: for every mirrored function and event, recompute the selector /
//! topic0 from the Rust `sol!` types and assert an identical one exists in the compiled hardhat
//! artifact. It runs wherever `ASC_CONTRACTS_DIR` points at a compiled asc-contracts checkout —
//! locally, and in the `write-ability-e2e` workflow, which exports exactly that variable — so a
//! asc-contracts pin bump that moves a mirrored surface fails HERE, naming the drifted item,
//! instead of somewhere downstream. Without the env var the test is a no-op (unit-test runs and
//! CI jobs that don't check out asc-contracts stay green).
//!
//! `IChainInfo` is deliberately NOT covered: it mirrors this repo's own chain-info precompile
//! (`precompiles/chain-info`), not a usc-contracts artifact, and drift there is caught by the
//! in-repo precompile integration tests.

use alloy::primitives::keccak256;
use alloy::sol_types::{SolCall, SolEvent};
use write_ability::abi::{IOutbox, IOutboxFactory, IVoteValidator};

/// Canonical signature string ("name(type1,type2,…)") for an ABI item in a hardhat artifact.
/// Elementary and array types are already canonical in the artifact's `type` field; tuples would
/// need component expansion, which the mirrored surface deliberately has none of (asserted).
fn artifact_signatures(artifact_path: &std::path::Path, kind: &str) -> Vec<String> {
    let raw = std::fs::read_to_string(artifact_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", artifact_path.display()));
    let artifact: serde_json::Value = serde_json::from_str(&raw).expect("artifact is JSON");
    artifact["abi"]
        .as_array()
        .expect("artifact has an abi array")
        .iter()
        .filter(|entry| entry["type"] == kind)
        .map(|entry| {
            let name = entry["name"].as_str().expect("abi entry has a name");
            let params: Vec<&str> = entry["inputs"]
                .as_array()
                .expect("abi entry has inputs")
                .iter()
                .map(|input| {
                    let ty = input["type"].as_str().expect("input has a type");
                    assert!(
                        !ty.starts_with("tuple"),
                        "{name} takes a tuple — extend this helper with component expansion \
                         before mirroring it"
                    );
                    ty
                })
                .collect();
            format!("{name}({})", params.join(","))
        })
        .collect()
}

fn assert_event_mirrored(artifact: &std::path::Path, rust_signature: &str, topic0: [u8; 32]) {
    let on_chain = artifact_signatures(artifact, "event");
    let found = on_chain
        .iter()
        .any(|sig| keccak256(sig.as_bytes()).0 == topic0);
    assert!(
        found,
        "event mirror drifted from {}: Rust binds `{rust_signature}` (topic0 {}), the artifact \
         declares {:?}. A changed signature moves topic0 and silently blinds every subscription — \
         update src/abi.rs (and the relayer's mirror) to match the contract.",
        artifact.display(),
        alloy::primitives::B256::from(topic0),
        on_chain,
    );
}

fn assert_function_mirrored(artifact: &std::path::Path, rust_signature: &str, selector: [u8; 4]) {
    let on_chain = artifact_signatures(artifact, "function");
    let found = on_chain
        .iter()
        .any(|sig| keccak256(sig.as_bytes()).0[..4] == selector);
    assert!(
        found,
        "function mirror drifted from {}: Rust binds `{rust_signature}` (selector 0x{}), no \
         function in the artifact has that selector. Calls will revert with empty returndata — \
         update src/abi.rs (and the relayer's mirror) to match the contract.",
        artifact.display(),
        alloy::hex::encode(selector),
    );
}

#[test]
fn mirrored_abi_surface_matches_compiled_contracts() {
    // `ASC_CONTRACTS_DIR` since the repository was renamed usc-contracts -> asc-contracts;
    // `USC_CONTRACTS_DIR` stays accepted so existing local setups keep working.
    let dir = std::env::var("ASC_CONTRACTS_DIR")
        .or_else(|_| std::env::var("USC_CONTRACTS_DIR"))
        .ok();
    // Set by CI alongside the artifacts dir. Without it an unset dir skips, which is what a plain
    // `cargo test` on a dev machine wants — but it also silently no-opped this gate in CI for
    // weeks, so any environment that means to enforce the gate sets this and gets a failure
    // instead of a pass.
    let strict = std::env::var("ABI_GATE_STRICT").is_ok();
    let Some(dir) = dir else {
        assert!(
            !strict,
            "ABI_GATE_STRICT is set but neither ASC_CONTRACTS_DIR nor USC_CONTRACTS_DIR is — the \
             drift gate would have silently passed without checking anything"
        );
        eprintln!(
            "ASC_CONTRACTS_DIR not set — skipping ABI-surface check (point it at a compiled \
             asc-contracts checkout to enable; the write-ability-e2e workflow does)"
        );
        return;
    };
    let contracts = std::path::Path::new(&dir).join("artifacts/contracts/write-ability");
    assert!(
        contracts.is_dir(),
        "contracts dir is set but {} does not exist — run `npx hardhat compile` there first",
        contracts.display()
    );

    let outbox = contracts.join("Outbox.sol/Outbox.json");
    assert_event_mirrored(
        &outbox,
        IOutbox::MessagePublished::SIGNATURE,
        IOutbox::MessagePublished::SIGNATURE_HASH.0,
    );

    let factory = contracts.join("deployer/OutboxFactory.sol/OutboxFactory.json");
    assert_event_mirrored(
        &factory,
        IOutboxFactory::OutboxCreated::SIGNATURE,
        IOutboxFactory::OutboxCreated::SIGNATURE_HASH.0,
    );

    let validator = contracts.join("EOAValidator.sol/EOAValidator.json");
    assert_function_mirrored(
        &validator,
        "attestors()",
        IVoteValidator::attestorsCall::SELECTOR,
    );
    assert_function_mirrored(
        &validator,
        "threshold()",
        IVoteValidator::thresholdCall::SELECTOR,
    );
    assert_function_mirrored(
        &validator,
        "attestorSetUpdateNonce()",
        IVoteValidator::attestorSetUpdateNonceCall::SELECTOR,
    );
    assert_function_mirrored(
        &validator,
        "submitAttestorSetUpdate(address[],bytes)",
        IVoteValidator::submitAttestorSetUpdateCall::SELECTOR,
    );
}
