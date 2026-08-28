# Decision 0124: Baton Bootstrap, Doria-Native Transition, And Toolchain Release Gate

- **Status:** Accepted
- **Accepted:** 2026-08-27
- **Date:** 2026-08-27
- **Implementation Status:** PHP Bootstrap Active; Stage 33 Slices 1 And 2
  Implemented; Stage 33 Slice 3 Next; Pre-Stage-45 Transition Scheduled
- **Amends:** Decision 0118's implementation ownership and the Stage 33 plan
- **Preserves:** Decisions 0117 and 0118's public manifest, package-graph, resolver, lockfile, workspace, cache, processor, and offline contracts

## Context

`dorialang/baton-php` exists to put the project workflow in users' hands early
and expose mistakes in Baton's commands, diagnostics, manifests, layouts,
packaging, installation, and compiler boundary. It is a disposable
developer-experience bootstrap, not the permanent Baton implementation.

The accepted long-term architecture instead places Baton in a clean
`dorialang/baton` repository and implements it in Doria. Earlier documentation
recorded both facts but failed to put the rewrite and installed-toolchain
cutover in the binding stage sequence. Stage 33 listed only product features,
while a separate supporting exit strategy listed rewrite prerequisites without
an owning stage or release gate. Read literally, the plan could promote the PHP
bootstrap into the official permanent implementation and leave the Doria-native
rewrite indefinitely unscheduled.

That divergence is rejected. The bootstrap is the UX oracle and temporary
distribution vehicle. The Doria implementation is the shipping product before
the first unsuffixed toolchain release.

## Decision

### Bootstrap role

`dorialang/baton-php` remains the active Baton implementation through Stage 33.
It may implement the complete accepted Stage 33 project, package, build, test,
dependency, workspace, cache, lockfile, processor, and offline workflow. This
work is not an investment in permanent PHP internals: it validates and freezes
the durable user experience against real compiler and editor integrations.

Decision 0126 records Stage 33 Slice 1 Complete: schema compatibility,
schema-2 local/scoped identity, targets and selectors, deterministic autoload
discovery, and single-package build plans execute in the bootstrap. Decision
0127 records Stage 33 Slice 2 Complete: normal path/Git dependency resolution,
strict lockfiles, dependency commands, global caching, offline operation, and
multi-package plans execute there as well. Slice 3 is next, Stage 33 remains in
progress and not complete, and none of that changes the required Doria-native
cutover.

Bootstrap implementation choices are never public contracts. The durable
contract comprises observable behavior:

- command names, options, argument forwarding, exit codes, and signal behavior;
- `Baton.toml`, `Baton.lock`, build-plan, build-receipt, and machine-output schemas;
- project discovery, source inventory, dependency resolution, cache, offline,
  workspace, processor, and test behavior;
- diagnostic identities, wording conventions, structured payloads, and help;
- generated projects, build locations, installed layouts, and release behavior;
- compiler and language-server component discovery and verification.

Stage 33 therefore completes and validates the Baton product contract in the
PHP bootstrap. It does not make PHP Baton's permanent implementation language,
does not erase the clean-repository requirement, and does not satisfy the
Doria-native transition.

### Doria-native transition sequence

A binding **Pre-Stage-45 Doria-Native Baton Transition** runs after Stage 44 and
before Stage 45 compiler self-hosting begins. The transition creates the clean
`dorialang/baton` repository and ports the accepted behavior to Doria without
carrying forward the PHP repository's implementation structure or history.

This placement consumes:

- Stage 31 multi-file compilation, namespaces, package graphs, and build plans;
- Stage 32 attributes and compiler-owned test metadata;
- Stage 33's exercised and frozen Baton product contract;
- Stage 36a filesystem, standard I/O, path-facing, and child-process foundations;
- Stage 40 FFI and native-library integration where a portable library needs it;
- Stage 44 network and HTTP foundations for Git and remote package workflows.

Environment access, path manipulation, TOML, JSON, cryptographic hashing,
archive handling, and any remaining implementation necessities are explicit
transition entry gates. They must be supplied through reviewed Doria standard
library or package APIs. The port may not obtain them through hidden
Baton-specific compiler intrinsics, PHP subprocesses, a bundled PHP runtime, or
host-language semantic shortcuts. Pure-Doria tooling libraries may live with or
be consumed by `dorialang/baton`; platform authority remains in accepted Doria
runtime, standard-library, or FFI boundaries.

### Compatibility and cutover gate

The PHP and Doria implementations run one shared, implementation-neutral
compatibility suite during the transition. It compares at least:

- exit status and signal behavior;
- compiler and language-server argument vectors;
- manifest and lockfile parsing, validation, and canonical serialization;
- deterministic source inventories, build plans, receipts, and cache keys;
- dependency and workspace graphs, conflict diagnostics, and offline behavior;
- human, concise, and machine-readable diagnostics;
- generated project contents and build/install paths;
- test discovery, execution order, reporting, and failure behavior;
- clean-machine toolchain installation and relocation on every supported host.

Cutover requires the Doria implementation to satisfy every Stage 33 acceptance
criterion and the shared compatibility suite. The new implementation must also
build, check, test, and package its own repository through the installed Baton
workflow, using `doriac` as the compiler rather than becoming another semantic
authority.

After cutover:

- `dorialang/baton` owns production Baton source, releases, templates, and
  complete toolchain assembly;
- installed `bin/baton` is the native Doria executable;
- public toolchain archives contain no Baton PHAR, Composer dependency, private
  PHP runtime, or PHP launcher;
- `dorialang/baton-php` is frozen for historical reference, compatibility
  fixtures, and bootstrap archaeology rather than production release assembly;
- users do not migrate manifests, lockfiles, commands, build layouts, or other
  durable contracts merely because the implementation language changed.

### Release blocker

The unsuffixed `2026.03.1` toolchain release must not ship until the
Pre-Stage-45 transition and native distribution cutover are complete. A canary
or other prerelease may continue to carry the PHP bootstrap while transition
work is visibly incomplete; prerelease availability is not evidence that the
shipping gate has passed.

The 1.0 gate inherits this requirement. It validates a toolchain whose Baton is
the Doria-native executable from `dorialang/baton`, not the PHP bootstrap.

## Consequences

- Stage 33 remains useful prototype work rather than throwaway work without a
  purpose: it freezes behavior before the clean-room implementation port.
- Baton's Doria rewrite is scheduled late enough to consume real tooling APIs
  and early enough to exercise the language before compiler self-hosting.
- Stage 45 begins with a substantial Doria-authored tool already shipping in
  the toolchain.
- The PHP implementation may favor direct, disposable internals, but its tests
  must describe public behavior in a form the Doria repository can reuse.
- Release assembly ownership transfers from `dorialang/baton-php` to
  `dorialang/baton` only at the gated cutover.
- No future planning summary may use "Baton is complete" without distinguishing
  the Stage 33 product-contract milestone from the Doria-native transition.

## Invalidated Elsewhere

- `docs/doria-end-to-end-plan.md`: Stage 33 ownership, Phase J sequence,
  decision catalogue, dependency notes, summary, and release gate.
- `README.md`, `SPEC.md`, and `docs/notes/current-pipeline.md`: Baton status and
  implementation timeline.
- Decisions 0117 and 0118: Stage 33 implementation home and later native port.
- `dorialang/baton-php`: agent guidance, architecture, development plan,
  manifest/package model, toolchain, release, contributor, and public status
  documentation.
- Language-server documentation remains correct about Stage 33 project
  inventory; it gains no implementation-language ownership and requires no
  semantic change from this decision.
- Website target-state documentation already presents Baton as the completed
  product and must not expose the temporary PHP bootstrap as product identity.

## Verification

Planning checks must fail if they cannot find all of these facts together:

1. Stage 33 completes the product contract in `dorialang/baton-php`.
2. The Pre-Stage-45 transition ports Baton to Doria in `dorialang/baton`.
3. The port is parity-gated against shared observable fixtures.
4. Production archives drop PHP, Composer, the PHAR, and the private runtime.
5. The unsuffixed `2026.03.1` release is blocked on the native cutover.
