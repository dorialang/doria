# Performance and Benchmarking Plan

> Documentation role: supporting design note.
> Source-of-truth hierarchy: `docs/doria-end-to-end-plan.md` owns future sequencing; accepted `docs/decisions/*.md` files own topic-level decisions. This note is subordinate to both.

Doria's long-term goal is native machine code and standalone executables. Performance should be measured honestly from early development onward, especially because Doria is intended for native CLI tools, desktop applications, game tooling, game engines, graphics/media work, C-library bindings, and future raylib bindings.

This document records the benchmark direction. It is not a performance claim.

---

## Stage 26 collection representations

`SortedDictionary` and `SortedSet` use compact contiguous storage, one bulk sort,
and binary search (`O(log n)` lookup; `O(n)` insertion/removal; `O(n)` ordered
iteration). `PriorityQueue` is a binary min-heap (`O(1)` peek, `O(log n)` push and
pop, `O(n)` bulk heapify). `Deque` is a circular buffer with amortized `O(1)`
pushes at both ends and `O(1)` pops/peeks. Ordinary scalar and handle slots are
stored inline without per-element allocation. Deterministic bounded runtime tests
compare these structures with `BTreeMap`, `BTreeSet`, `BinaryHeap<Reverse<_>>`,
and `VecDeque`; those Rust types are test oracles, not public semantics.

## Stage 26a grouped-local contract

Grouped local declarations are syntax sugar with no runtime grouping
abstraction. The initializer evaluates once; scalar copies match separate locals
initialized from that result; string bindings retain the same immutable runtime
handle without duplicating contents. Grouping alone creates no tuple,
collection, heap allocation, dynamic dispatch, or async-runtime interaction.
The canonical MIR node records ordered initialization for validation and is
eligible for ordinary optimization; no group value survives in generated code.

## Stage 26b performance baseline foundation

Stage 26b is in progress. Decision 0112 accepts the repository-owned measurement,
provenance, and regression contract. Slices 1 and 2 are complete: the sibling benchmark
repository has a strict manifest, versioned JSON schema, committed correctness
fixtures, round-robin sampling, explicit toolchain selection, complete available
provenance, and the first three diagnostic pairs. `doriac compile
--performance-report <file>` produces opt-in compiler phase and structural
evidence without changing the ordinary compile path. Slice 2 records the initial
compiler/generated/runtime/diagnostic matrix, deterministic source scaling,
process resource counters, separate optional Callgrind/DHAT evidence, candidate
evidence, and an exact structural baseline without timing thresholds. Slice 3
adds peers for the new cases and proposes controlled timing thresholds for a
separate review. This is not an unlimited optimization campaign and does not
displace Stage 36a's stream gate.

Slice 3 Part 1 is delivered. C, Rust, and PHP peers cover the seventeen
generated-program and runtime-subsystem cases, and every comparative case that
ranks Doria against another language now carries a peer equivalence record that
the manifest loader enforces: what each peer does, every known semantic
difference, and which side it favours. Peers use idiomatic constructs and are not
handicapped to match stricter Doria defaults, so differences favouring the peer
stay visible.

Two controlled candidate sessions across five targets, covering both the
Cranelift and LLVM backends, produced a timing threshold proposal that accepts
nothing. Its finding is that seventy-nine of eighty case and target pairs are
dominated by process startup rather than by their workload, so those cases
cannot carry a timing threshold until their workloads are scaled. The one pair
above the floor threshold clears it by about 2.6 ms, which is too small a margin
to build a threshold on. Neither session is timing baseline eligible because CPU
affinity cannot be verified on macOS; both are structurally eligible.

The slice also produced a provenance finding that outlives its numbers. An
earlier write-up reported `string_search` on Cranelift at 6.6 times its startup
floor; that result did not survive a compiler rebuild and is withdrawn. Two
builds of the same compiler revision bundled materially different runtime
archives, and the benchmark report records the compiler revision, commands, and
driver but not the identity of the linked runtime archive, so the substitution
was invisible in the evidence. Timing results are therefore not comparable
across compiler rebuilds until the runtime archive is part of recorded
provenance. Separately, the release profile prefers a workspace
`target/release/libdoria_rt.a` over the runtime the compiler bundled itself, so
a stale archive can shadow the correct runtime and fail the link with a generic
backend error.

The runner has three separate tracks:

```text
Compiler Performance
- parse time
- semantic-analysis time
- MIR-lowering time
- Cranelift code-generation time
- LLVM code-generation time
- link time
- total compile time
- compiler peak RSS
- generic specialization count

Generated-Program Performance
- cold startup
- hot throughput and latency
- wall, user, and system time
- peak RSS
- allocation count where available
- binary and stripped-binary size
- compressed artifact size where useful
- correctness output or hash

Runtime-Subsystem Performance
- strings, objects, and methods
- ownership and collections
- generics and shared ownership
- streams, async, and FFI when their owning stages land
```

The Cranelift development profile prioritizes fast compilation, fast linking,
responsive iteration, and acceptable runtime performance. The LLVM release
profile prioritizes runtime performance, low memory use, strong optimization,
and reasonable artifact size. The interpreter remains the semantic oracle and
regression target, not a native performance competitor.

Initial executable cases are `hello_world`, `startup`, `fibonacci`, `primes`,
`integer_arithmetic`, `string_interpolation`, `string_search`,
`list_operations`, `dictionary_lookup`, `set_membership`,
`sorted_dictionary`, `sorted_set`, `priority_queue`, `deque`,
`object_construction`, `method_calls`, `generic_specialization`,
`shared_reference`, and `writable_shared_access`. Cases requiring closures,
JSON, routing, templating, raylib, async, networking, or streams join only when
their owning stages land.

The initial comparison set is C, Rust, and PHP: C and Rust are native baselines;
PHP is the central adoption comparison. C++, Java, C#, JavaScript, and Python may
join after the runner and fairness rules stabilize. The project-owned runner
uses the existing sibling `benchmarks` repository's `bench.php` entry point and
flat peer-source case layout; a future `baton bench` orchestrates the same
benchmark engine instead of creating another one.

Correctness passes against committed exact stdout, stderr, and status fixtures
before timing is accepted; no target supplies an implicit reference. Every report records versions,
flags, machine, inputs, runner identity, and profile. Cold startup remains
separate from hot throughput, compile time remains separate from runtime, and
unfavorable results remain visible. Controlled runs default to five warmups and
at least ten measured rounds, interleaved across targets with rotating order.
Quick reports are explicitly baseline-ineligible. Curated reports may be committed; raw
generated results normally are not. Shared CI owns deterministic structural
checks; controlled runners own timing thresholds. Public claims remain specific
to the measured workload.

## Continuous performance impact rule

Every later stage that changes runtime representation, allocation, ownership,
dispatch, code generation, control flow, I/O, concurrency, or FFI records a
`Performance Impact` section covering expected cost; allocation, copying,
dispatch, memory, and code-size changes; benchmark cases added or updated; and
measured evidence where material. “No measurable impact expected” is permitted
only as a checkable claim. Ordinary shared CI does not use brittle wall-clock
thresholds.

---

## 1. Performance expectation

Performance is a design pillar, not an outcome to be discovered after the fact. Suboptimal performance is a defect to be diagnosed, not a characteristic to be documented.

Target:

```text
- Doria aims to match C, C++, and Rust on comparable native workloads. Parity is the floor, not the ceiling.
- Where Doria can safely go faster, it should. Safety and correctness guarantees are never traded for speed.
- Doria should be consistently optimized across workload shapes. An unexplained outlier is a defect, not a data point.
- Doria should be far faster than PHP and Python for CPU-bound userland code, and must never treat that comparison as evidence of good performance.
- Performance work is continuous. As the language matures, keep looking for headroom rather than settling at a benchmark ranking.
```

The shared-backend limit is real and should be reasoned about rather than wished away. Doria, Clang, and rustc all lower through LLVM, so on scalar code LLVM already optimizes well, parity is the realistic ceiling. Beating C therefore means emitting information a C compiler cannot derive: aliasing facts implied by ownership, whole-program monomorphization and devirtualization, guaranteed alignment and dereferenceability, escape analysis into stack or arena allocation, and profile-guided layout by default. Those are the sanctioned routes to a win, and each is a compiler capability rather than a benchmark trick.

Ambition and claims are governed separately, and conflating them is an error in both directions. The target above is unbounded; published claims are bound by measured evidence.

Avoid broad claims like:

```text
Doria is faster than C.
Doria is faster than Rust.
Doria is always faster than PHP.
```

Prefer benchmark-specific claims:

```text
On benchmark X, with compiler version Y and flags Z, Doria performed N% faster/slower than language/runtime R on machine M.
```

---

## 2. Comparison set

The benchmark suite should eventually compare Doria against:

```text
- C
- C++
- Rust
- Java
- C#
- PHP
- JavaScript
- Python
```

Do not treat these languages as a single performance class. Compare by workload.

Example expectations:

```text
C/C++/Rust:
  hardest to beat; useful upper-bound/native baseline.

Java/C#:
  excellent hot performance; useful service/runtime comparison.

JavaScript:
  V8 can be very fast for hot code; useful dynamic/JIT comparison.

PHP/Python:
  important adoption comparisons for PHP developers and scripting workloads.
```

---

## 3. Metrics to collect

Collect more than runtime speed.

```text
- compile time
- cold startup time
- hot execution time
- wall/user/system time
- peak RSS memory
- allocation count, if available
- binary size
- stripped binary size
- compressed artifact size
- container image size later
- output correctness hash
```

Executable size should be measured carefully:

```text
source file size != deploy artifact size
binary size != total runtime footprint
PHP script size assumes a PHP runtime already exists
native binary size may include more runtime support
```

---

## 4. Benchmark cases

Start with small cases, but avoid only toy benchmarks.

Suggested cases:

```text
hello_world
startup
fibonacci
primes
json_parse
json_encode
string_interpolation
list_map_filter
dictionary_lookup
object_construction
method_dispatch
generics later
router
template_render
lexer
parser
type_checker
small_game_loop later
raylib_binding_smoke_test later
```

The most meaningful future benchmark is:

```text
doriac compiling part of doriac
```

That aligns performance measurement with the self-hosting goal.

---

## 5. Repository structure

The suite lives in the `benchmarks` repository, a sibling checkout of `doria`:

```text
benchmarks/
  bench.php
  <case>/
    README.md
    Baton.toml
    <case>.doria
    <case>.c
    <case>.cpp
    <case>.rs
    <case>.go
    <case>.js
    <case>.php
    <case>.py
    <Case>.java
    csharp/
      Program.cs
      <case>.csproj
```

Peers sit beside the Doria source rather than in per-language subdirectories. Only C# needs its own directory, for the project file; Java's filename is PascalCase to match its public class. `bench.php` is the single harness: it builds every target available on the current platform, verifies each one's output against the first target, then times best-of-N.

Repository tooling here is PHP, never Python or shell. AGENTS.md assigns small repository text/JSON helpers to PHP and compiler/project tooling to Rust, and CI fails the build when any `.py` appears under `scripts/`.

Generated benchmark results should usually not be committed, except for curated release reports and any committed regression baselines a performance gate depends on.

---

## 6. Benchmark rules

```text
1. Always verify output correctness.
2. Record compiler/runtime versions and flags.
3. Separate cold startup from hot throughput.
4. Separate compile time from runtime.
5. Run enough iterations for stable measurements.
6. Use the same input data across languages.
7. Avoid unfairly using native libraries in one language but not another.
8. Include real Doria-relevant workloads such as lexing/parsing/type-checking.
9. Publish bad results too.
10. Never claim broad superiority from one benchmark.
```

---

## 7. Possible tooling

Possible benchmark tools:

```text
- hyperfine for command-level benchmarks
- language-native microbenchmark tools where useful
- stat/size/strip for binary size data on Unix-like systems
- platform-specific tools later for Windows/macOS packaging
```

Example command shape:

```bash
hyperfine \
  --warmup 5 \
  --runs 30 \
  --export-json benchmarks/results/fibonacci.json \
  './build/doria/fibonacci' \
  './build/c/fibonacci' \
  './build/rust/fibonacci' \
  'php ../benchmarks/fibonacci/fibonacci.php' \
  'node ../benchmarks/fibonacci/fibonacci.js' \
  'python3 ../benchmarks/fibonacci/fibonacci.py'
```

---

## 8. Desktop, game, and raylib implications

Because Doria may eventually target native desktop apps, a game engine, and raylib bindings, benchmarks should eventually include:

```text
- event loop overhead
- FFI call overhead
- frame-loop timing stability
- allocation pressure per frame
- vector/math operations
- buffer/array access
- image/audio data movement
- native library binding smoke tests
```

Do not start raylib binding work before the native backend, FFI model, and basic runtime are ready. But keep these use cases visible when designing representation, memory, and ABI choices.

---

## 9. Settled direction

Settled:

```text
- Doria should develop a benchmark suite.
- Benchmarks should include runtime speed, memory, compile time, and artifact size.
- Doria should avoid unsupported performance marketing.
- Native desktop/game/FFI use cases should influence future benchmark design.
- The native backend direction is a staged Cranelift/LLVM route: Cranelift first for native smoke/backend iteration, LLVM later as the longer-term optimizing backend path.
- Decision 0110 makes the stream semantic and performance/memory contract
  binding. Stage 36a, not Stage 43, owns its initial regression gate.
```

Open:

```text
- Exact Cranelift object/linking integration and LLVM adoption milestone.
- Exact runtime and memory model.
- Exact benchmark runner implementation.
- Whether benchmark results are published per release.
```

---

## 10. Stage 36a stream performance gate

Decision 0110 requires stream implementation choices to preserve efficient
steady-state operation, not merely semantic correctness. Stage 36a therefore
ships the first cross-platform stream benchmark and memory-regression gate.
Stage 43 continues and broadens this suite for engine workloads.

The gate records these metrics where the host platform can measure them
reliably:

```text
- compile time
- cold startup
- throughput and latency distribution
- wall, user, and system time
- peak RSS
- allocation count and allocated bytes
- syscall count
- bytes copied by the stream/adapter path
- generic specialization count
- runtime library growth
- development binary size
- release binary size
- stripped binary size
- output correctness hash
```

The initial fixture set is binding:

```text
- large streaming file copy
- repeated small writes
- non-blocking pipe transfer
- child stdout and stderr drainage
- incremental UTF-8 line processing across multibyte boundaries and long lines
- first-class versus intrinsic standard-output writes
- many stable readiness registrations
- synchronous startup proving zero executor/task/scheduler initialization
- concrete adapter chain
- erased interface stream
```

Each case has an equivalent direct OS, C, or Rust baseline where meaningful.
Those programs establish comparative evidence; they do not authorize broad
claims that Doria is faster than another language. Correctness hashes and
observable output must agree before timing data is considered.

Ordinary CI enforces deterministic structural invariants: common outcomes and
standard-stream views are allocation-free; reusable-buffer loops do not require
a steady-state allocation; the implementation does not copy whole chunks or
unread suffixes as hidden adapter work; readiness registration and event storage
are reused; synchronous startup initializes no async infrastructure; and bounded
operations remain bounded. Timing thresholds run on curated, controlled runners
because shared CI noise is not a reliable regression signal. Thresholds, runner
identity, compiler revision, build flags, sample count, and raw results are
versioned with the harness before the Stage 36a gate can pass.
