# Decision 0073: Stage 15 LLVM Release Backend

Status: Accepted

## Context

Stages 11 through 14 established one typed MIR consumed by the semantic interpreter and the fast Cranelift native backend. Production-oriented native builds need a more aggressive optimizer without creating a second language model or making backend behavior observable as Doria semantics.

## Decision

Stage 15 adds an LLVM 18 optimized release backend through pinned `inkwell` 0.9.0. The optional Cargo feature is `llvm-backend`, using inkwell's `llvm18-1-prefer-dynamic` integration feature. LLVM 18.1.x is the supported toolchain line; tier-1 CI installs LLVM 18.1.8 where the platform package ecosystem exposes a patch version and otherwise installs the LLVM 18 formula/package line.

Native remains one Doria target with two explicit compiler-owned profiles:

- the fast profile uses Cranelift and remains the default for direct `doriac compile` and `doriac run`;
- the release profile uses LLVM and is selected with `--release`.

The profile is passed explicitly through compiler options. It is not inferred from Rust `debug_assertions`, Cargo's build profile, or artifact paths. A bootstrap compiler built without `llvm-backend` rejects `--release` and never falls back to Cranelift.

LLVM consumes the exact same validated typed MIR as the interpreter and Cranelift. MIR validation is backend-independent and both native lowerers must call it. LLVM does not accept HIR or AST and does not introduce a parallel Doria-native IR.

LLVM emits one host object in memory. The existing native linker path links that object with the same implementation-private `doria-rt` ABI used by Cranelift. Release profile discovery prefers the release runtime artifact; `DORIA_RT_PATH` remains the explicit override.

The LLVM module uses the host triple and data layout, is verified before optimization, runs LLVM's `default<O3>` pipeline, is verified again, and is emitted by the host target machine. The initial implementation uses a generic host CPU rather than host-specific feature specialization. LTO and cross-compilation remain outside Stage 15.

## Semantic constraints

The release profile changes compilation time, optimization strategy, and artifact quality only. It does not change any Doria-visible behavior.

- Checked integer arithmetic, division, remainder, shifts, negation, and conversions perform explicit safety checks before an LLVM instruction that would otherwise be undefined or poison-producing.
- Float lowering uses binary32/binary64 operations without fast-math flags, reassociation, NaN elision, or signed-zero elision.
- Bool values remain canonical false/true scalars and `and`/`or` retain short-circuit control flow in condition and value positions.
- Compile-time-known strings remain exact byte constants passed with explicit lengths; LLVM does not introduce runtime strings, `strlen`, null-terminated discovery, or implicit newlines.
- Panic, output, Doria stack frames, and process entry continue through `doria-rt` with the existing exact status and formatting contracts.

LLVM may not use undefined behavior as an implementation shortcut for a Doria operation with checked or otherwise defined semantics. Function and argument evaluation remains left-to-right where behavior is visible.

## Differential requirement

The MIR interpreter remains the semantic oracle. Every finite native example in the durable manifest executes through the interpreter, Cranelift fast profile, and LLVM release profile. The suite compares exact stdout bytes, exact stderr bytes, and exact process status, including panic fixtures.

NaN payload and sign bits remain non-semantic under decision 0072; parity covers Doria-visible classification and comparisons rather than raw NaN payload identity.

## Consequences

Default contributor builds remain usable without a system LLVM installation. Distributable compiler builds intended to support `--release` include `llvm-backend` and require the pinned LLVM 18 toolchain during bootstrap compilation.

Stage 15 introduces no source syntax or language semantics. Stage 16 runtime strings and canonical display conversion remain next.
