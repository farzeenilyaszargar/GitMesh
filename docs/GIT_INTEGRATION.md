# Git Integration

GitMesh must support ordinary Git workflows:

```bash
git clone gitmesh://farzeen/project
git fetch
git pull
git push origin main
```

Git itself remains standard Git.

## Components

```text
git
  |
  v
git-remote-gitmesh
  |
  v
gitmeshd local socket
  |
  v
gitmesh-git + gitmesh-core
  |
  +--> coordinator protocol
  +--> storage/network protocol
```

`git-remote-gitmesh` communicates with `gitmeshd` over a Unix domain socket on
Unix-like systems and named pipes or equivalent local IPC on Windows.

## Generation 1 Adapter

Generation 1 should ship first. It uses native Git commands, `upload-pack`,
`receive-pack`, and temporary or cached bare repositories internally.

Advantages:

- high compatibility with existing Git behavior
- less risk around pack negotiation edge cases
- faster path to clone/fetch/push acceptance tests
- easier interop testing against standard Git

The Gen 1 adapter remains a compatibility layer. Authoritative data is still
ingested into GitMesh's object CAS, segments, and storage network.

## Generation 2 Adapter

Generation 2 implements a direct Git smart-protocol v2 adapter over GitMesh object
indexes and segment retrieval. It is justified after Gen 1 benchmarks show where
cached bare repos or pack regeneration become bottlenecks.

## Push Flow

1. Git sends a pack to `git-remote-gitmesh`.
2. `gitmeshd` validates objects with standard Git-compatible rules.
3. Missing objects are canonicalized into GitMesh object CAS.
4. Segments are produced, encrypted if private, erasure-coded, and placed.
5. Storage leases are verified against repository policy.
6. `gitmeshd` submits signed `RefUpdate` CAS transactions.
7. On success, Git receives normal push success. On conflict, Git receives normal
   non-fast-forward behavior.

## Clone / Fetch Flow

1. Resolve human name to `RepoId`, or use direct `RepoId`.
2. Fetch latest signed checkpoint from multiple sources.
3. Verify checkpoint, policy, and permissions.
4. Attempt optimized pack/cache retrieval.
5. Fall back to segment availability lookup.
6. Retrieve shards in parallel with hedged requests.
7. Validate shard integrity and reconstruct segments.
8. Decrypt where authorized.
9. Verify resulting Git object IDs.
10. Produce Git packs/objects for native Git.

## Future Git Features

Support should be staged:

- branches and tags
- force push with policy and audit
- shallow clone
- partial clone
- submodules
- Git LFS
- large release/package artifacts
