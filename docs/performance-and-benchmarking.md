# Performance and Benchmarking Plan

Doria's long-term goal is native machine code and standalone executables. Performance should be measured honestly from early development onward, especially because Doria is intended for native CLI tools, desktop applications, game tooling, game engines, graphics/media work, C-library bindings, and future raylib bindings.

This document records the benchmark direction. It is not a performance claim.

---

## 1. Performance expectation

A mature native Doria should aim to be much closer to native compiled languages than to interpreted/dynamic application runtimes.

Honest expectation:

```text
- Doria will probably not consistently beat mature C, C++, or Rust on low-level optimized workloads.
- Doria can plausibly be in the Rust/Go/C# NativeAOT neighborhood for many application workloads if the compiler/runtime are well designed.
- Doria should be much faster than PHP and Python for CPU-bound userland code.
- Doria may be competitive with Java/C#/JavaScript depending on startup, hot-code behavior, runtime design, and workload shape.
```

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

Possible future structure:

```text
benchmarks/
  README.md
  runner/
    bench.py
    report.py
  cases/
    hello_world/
      doria/
      c/
      cpp/
      rust/
      java/
      csharp/
      php/
      javascript/
      python/
    fibonacci/
    json_parse/
    object_construction/
    lexer/
    parser/
  results/
    .gitkeep
```

Generated benchmark results should usually not be committed except for curated release reports.

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
  'php benchmarks/cases/fibonacci/php/fibonacci.php' \
  'node benchmarks/cases/fibonacci/javascript/fibonacci.js' \
  'python3 benchmarks/cases/fibonacci/python/fibonacci.py'
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
```

Open:

```text
- Exact Cranelift object/linking integration and LLVM adoption milestone.
- Exact runtime and memory model.
- Exact benchmark runner implementation.
- Whether benchmark results are published per release.
```
