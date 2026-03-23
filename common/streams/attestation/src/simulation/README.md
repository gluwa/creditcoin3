# Attestation Stream Testing Guide

The attestation stream is a core component of attestation generation with many both *implicit* and *explicit* invariants to uphold. While special care has been taken to keep the current implementation as simple as possible, the underlying algorithm of forward sync via backfilling past attestations remains complex and easy to get wrong. As such, ensuring proper test coverage is tricky, as many bugs will manifest only under very specific and rare conditions. This makes manual testing challenging, as most effort is spent in finding edge cases under which core assumptions might break down instead of just ensuring proper functionality in the first place.

To implement correct systems we must understand their problem space. This can prove difficult for systems with many inter-related invariants. *Simulation testing and property testing in particular allow us to automate the exploration of this problem space so that we can focus our efforts on fixing bugs instead of finding them.*

## Property Testing

> [!TIP]
>
> *Property testing* is a system of testing code by checking that certain properties of its output or behavior are fulfilled for all inputs. These inputs are generated automatically, and, critically, when a failing input is found, the input is automatically reduced to a *minimal* test case.
>
> [*source*](https://proptest-rs.github.io/proptest/)

Property testing in the attestation stream relies on the following three components:

- **Determinism:** the system under test (or SUT) must exhibit reproducible behavior so that we can replay and reduce known failures to minimal working example as well as avoid regressions by testing against historical fail states.
- **Strong runtime invariants:** the SUT must be able to validate its own behavior through the use of runtime assertions so that invalid states can be easily detected during the simulation process.
- **Decoupling implementation and testing:** the simulator should not have to rely on specific implementation details of the SUT, nor should it perform manual checking against internal state. By focusing only on input generation and leaving state validation as an implementation detail, we allow our systems to be more maintainable and reusable as long as the input format remains the same.

## Implementation Details

Simulation works by generation a set of legal transitions which will be sequentially applied to the attestations stream. External IO via the tip stream and the root stream is mocked so as to be entirely driven by the simulator. Each simulation run starts by generating a new pseudo-random seed and initial values which it uses to configure the stream's starting state. If the attestation stream detects a runtime invariant violation, the resulting panic will cause the current simulation run to fail. From here, the simulator will keep mutating the stream's start state and re-run the failed test until it can be regressed to a minimal reproducible example composed of as little and as simple state transitions as possible. In addition, known historical fail states contained in `proptest-regressions` are exercised on each run to avoid regressions. The simulation ends either when a pre-configured number of generated tests have passed or if a failure is found and regressed to a minimal working example.

## Testing Process

Simulation testing is gated behind the `simulation` feature flag and can be run as:
```bash
cargo test --release -p stream_attestation --features simulation simulate
```

If the simulator manages to find a bug, it will display the smallest state transition it was able to regress this bug to:

```bash
thread 'simulation::simulate' panicked at common/streams/attestation/src/simulation/mod.rs:5:1:
Test failed: 2 <= 1.
minimal failing input: sim = Simulation {
    steps: [
        Root(Ready),
        Root(Ready),
        Tip(Ready),
        Tip(Ready),
        Tip(Ready),
        Root(Ready),
        Tip(Ready),
        Root(Ready),
        Finalized(Before(2)),
    ],
    start_height: 0,
    attestation_prev: 0,
    attestation_interval: 1,
    attestation_next: 1,
    max_catchup: 2,
}
```

**Do not try and use the simulator for debugging purposes**, instead you should create a new test which reproduces the failing case inside of `tests/mod.rs`:

```rust
#[rstest::rstest]
#[tokio::test]
async fn simulation_failure(
    #[future]
    #[with(
        0, // start height
        nonzero!(1), // attestation interval
        nonzero!(2) // max catchup
    )]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, mut tip, mut stream_attestation) = attestations.await;

    roots.send_ready().await;
    let _ = poll!(stream_attestation);
    roots.send_ready().await;
    let _ = poll!(stream_attestation);
    tip.send_ready().await;
    let _ = poll!(stream_attestation);
    tip.send_ready().await;
    let _ = poll!(stream_attestation);
    tip.send_ready().await;
    let _ = poll!(stream_attestation);
    roots.send_ready().await;
    let _ = poll!(stream_attestation);
    tip.send_ready().await;
    let _ = poll!(stream_attestation);
    roots.send_ready().await;
    let _ = poll!(stream_attestation);
    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 1,
        ..Default::default()
    });
    let _ = poll!(stream_attestation);
}
```

The simulator does its best, however it is often still possible to regress to a simpler case based on your understanding of the system's implementation details:

```rust
#[rstest::rstest]
#[tokio::test]
async fn simulation_failure(
    #[future]
    #[with(0, nonzero!(1), nonzero!(2))]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, mut tip, mut stream_attestation) = attestations.await;

    roots.send_ready().await;
    roots.send_ready().await;
    tip.send_ready().await;
    tip.send_ready().await;
    let _ = poll!(stream_attestation);

    tip.send_ready().await;
    roots.send_ready().await;
    let _ = poll!(stream_attestation);

    tip.send_ready().await;
    roots.send_ready().await;
    let _ = poll!(stream_attestation);

    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 1,
        ..Default::default()
    });
    let _ = poll!(stream_attestation);
}

```

Once you are confident you cannot simplify the failing case anymore, it helps to set some breakpoints and use a debugger to step through each state transition in the stream implementation and write down key variable changes. Regressing to a minimal failure case makes it so it generally takes only a single run of debugging to find the source of the bug this way. 

Once the bug has been addressed, make sure to update the test to something more rigorous which no longer relies on internal assertions to fail, as these might be removed in the future. It also helps to add extra checks and give it a more explicit name now that you know the root cause.

```rust
// If a past attestation finalizes, future attestations have to be regenerated as the prev digest
// has changed.
#[rstest::rstest]
#[tokio::test]
async fn regenerate_attestations(
    #[future]
    #[with(0, nonzero!(1), nonzero!(2))]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, mut tip, mut stream_attestation) = attestations.await;

    roots.send_ready().await; // 0 - skipped, 0 is always ignored
    roots.send_ready().await; // 1
    tip.send_ready().await; // 0
    tip.send_ready().await; // 1

    let std::task::Poll::Ready(Some(Ok(attestation))) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 1);
    assert!(attestation.continuity_proof.is_empty());

    tip.send_ready().await; // 2
    roots.send_ready().await; // 2

    let std::task::Poll::Ready(Some(Ok(attestation))) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 2);
    assert_eq!(attestation.continuity_proof.len(), 1);

    tip.send_ready().await; // 3
    roots.send_ready().await; // 3

    let std::task::Poll::Ready(Some(Ok(attestation))) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 3);
    assert_eq!(attestation.continuity_proof.len(), 2);

    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 1,
        ..Default::default()
    });

    let std::task::Poll::Ready(Some(Ok(attestation))) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    // Attestation 3 is re-generated. Notice that the continuity proof is shorter, as it now
    // attests from block 1 instead of block 0.
    assert_eq!(attestation.header_number(), 3);
    assert_eq!(attestation.continuity_proof.len(), 1);

    // Attestation 2 is regenerated as well
    let std::task::Poll::Ready(Some(Ok(attestation))) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 2);
    assert!(attestation.continuity_proof.is_empty());

    // Attestation 1 is not regenerated as it has already finalized
    assert!(poll!(stream_attestation).is_pending());
}
```

## Limitations

Property testing is a powerful to tool which inverts the effort spent during the testing process: instead of spending most of your time finding bugs which might lead to failing cases, you will be given failing cases and will have to find the source of the bug. A simulation's result are only as good as the simulation itself however, and as such property testing will not be able to detect bugs under conditions which are not part of the state transitions you allow it to generate. Special care must be taken to ensure that the simulation exercises all possible execution paths as might be encountered in production.

> [!CAUTION]
>
> **Make sure you keep the simulator up-to-date with new features.**

In addition, property testing still suffers from needle-in-a-haystack issues. Due to the way in which it explores the problem space, it is possible for it to miss highly specific edge cases if those are statistically improbable to encounter. Property testing therefore works best when the bugs it is checking for occur in a variety of cases and are not just one-off errors. The following property test for example will almost *always* pass:
```rust
proptest! {
    #[test]
    fn i64_abs_is_never_negative(a: i64) {
        // This actually fails if a == i64::MIN, but randomly picking one
        // specific value out of 2⁶⁴ is overwhelmingly unlikely.
        assert!(a.abs() >= 0);
    }
}
```

> [!CAUTION]
>
> **A property test passing is not a guarantee that no bugs exist. Try to limit the range of values you are simulating based on domain knowledge so as to increase the chance of off-by-one errors being detected.**

## Further Resources

- [Proptest Book](https://proptest-rs.github.io/proptest/intro.html)
- [Tigerbeetle presentation](https://youtu.be/sC1B3d9C_sI?t=949)
