# GitMesh Coordination

Immutable Git objects do not require global consensus. Mutable repository state
does require serialization and auditable history.

## Repository Identity

The canonical identity is a cryptographic `RepoId`, not `username/repository`.
Human paths such as `farzeen/project` are aliases resolved by naming services.
If naming services disappear, `gitmesh://repo/<RepoId>` access must continue.

## Coordinated State

Repository-specific coordinators serialize:

- branch refs
- mutable tags
- ownership changes
- ACL changes
- protected branch policy
- key epochs
- coordinator-set changes

Source content itself must not depend on coordinators for survival.

## Ref Updates

Ref mutation is compare-and-swap:

```text
RefUpdate {
  repo_id
  ref_name
  expected_old_oid
  new_oid
  force
  policy_epoch
  transaction_id
  signer
  signature
}
```

A push succeeds only if `current_ref == expected_old_oid` and policy permits the
update. Conflicting simultaneous pushes produce normal Git non-fast-forward
behavior. Force pushes are signed, policy-checked, and auditable.

## Push Transaction

```text
receive Git objects
  |
  v
validate
  |
  v
produce missing immutable segments
  |
  v
compress, encrypt when private, erasure-code
  |
  v
store enough shards
  |
  v
receive independent storage leases
  |
  v
verify durability policy
  |
  v
submit RefUpdate CAS
  |
  v
publish signed checkpoint
  |
  v
return push success
```

A branch must never point to data that has not already met repository durability
policy. A coordinator verifies this against current availability-provider evidence
for the target object before applying the CAS transaction; local object presence
alone is not sufficient.

## Ref History And Checkpoints

Ref mutation is append-only:

```text
R100 main A -> B force=false
R101 main B -> C force=false
R102 main C -> Z force=true
```

Periodic signed `RefCheckpoint` records chain to prior checkpoints and commit to
the current ref map plus mutation history root. Clients verify the checkpoint
CID, parent sequence, coordinator device certificate, and signature before using
a checkpoint to detect stale state, rollback, replay, coordinator equivocation,
and invalid force pushes.

## Coordinator Modes

### Standard Mode

Standard mode favors simplicity and availability. A small repository
CoordinatorSet serializes mutations using mature replicated-state machinery or a
single active coordinator with signed checkpoints and recovery paths during V1/V3
development.

### High-Assurance Mode

High-assurance mode uses multiple independent coordinators with stronger
equivocation resistance and threshold or BFT-backed commitments. GitMesh must not
invent a custom BFT algorithm. Candidate mature systems must be evaluated before
use, such as Narwhal/Bullshark-family protocols, HotStuff-family systems,
CometBFT, or mature threshold-signature-based append logs where the operational
model fits.

## Recovery And Replacement

Coordinator membership is replaceable through signed repository administration
and recovery mechanisms. Recovery must be possible without GitMesh-operated
coordinators when sufficient repository authority keys and latest verifiable
checkpoints are available.

## Availability Tradeoff

More independent coordinators increase censorship and equivocation resistance but
can reduce availability under partitions. Standard repositories should default to
a pragmatic highly available mode; critical repositories can pay operational cost
for high-assurance coordination.
