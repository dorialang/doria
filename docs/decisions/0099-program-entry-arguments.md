# Decision 0099: Program entry arguments and process introspection

Status: Accepted

## Context

Doria's flagship product is command-line tools, yet nothing specified how a program reads its command-line arguments. `main` had only `(): int` and `(): void` forms (record 0032); there was no argv access anywhere — not on `main`, not in a stdlib module. A CLI language must define this. This record settles argument access and names the home for the rest of process introspection. (Finding F1 of `docs/notes/plan-open-questions-audit.md`.)

## Decision

### Arguments arrive as an optional `main` parameter
- `main` gains a third, optional parameter form: **`main(List<string> $args): int`** (and the `: void` variant). Both `main()` and `main(List<string> $args)` are valid entry points; a program that ignores its arguments keeps the parameterless form.
- The container is **`List<string>`**, not `string[]`: real CLI code parses arguments (filter/map/partition), and those methods live on `List`; `$args->count` gives the argument count. There is **no separate `argc` parameter** — the list carries its own length, so `main(string[] $argv, int $argc)` is explicitly rejected.
- `$args` is populated by the entry glue at process start (it is not a runtime-mutable global).
- **`$args` contains the program's arguments only — the executable path is NOT element 0** (amended post-acceptance; this supersedes the earlier "follows the platform's argv, settled with the implementation" wording). `$args[0]` is the **first real argument**, so `$args->count` means "how many arguments the user passed" — the intuitive reading, and it removes the C/PHP-era off-by-one where every `count` and every index must be mentally adjusted. A program invoked with no arguments receives an **empty** `List<string>` (`count == 0`), never a one-element list and never null. The executable path is a *process fact*, not an argument, and is therefore reached through `Doria\Std\Process` (below) — the module this record already designates for exactly that class of information. This contract is identical on Linux, macOS, and Windows and across the interpreter, Cranelift, and LLVM; the platform argv's element 0 is stripped by the entry glue.

### Process introspection lives in `Doria\Std\Process`
- The other process facts — exit code, process id, executable path, and similar — live in the **`Doria\Std\Process`** stdlib module, not on `main`. Arguments are the entry point's *input*, so they belong on `main`; ambient process facts are queried where they are needed, so they belong in a module. A hostile edge (`Console`) is rejected as a home: arguments are process state, not terminal capability.
- Where a `Process` member returns data (arguments, executable path), it is spelled as a **property** (`Process::args` would read data, not construct it) rather than a `getX` method, per the §9.1 nouns-are-properties charter — but since arguments are already delivered through `main`, `Process` need not duplicate them.

### Scheduling
- The `main(List<string> $args)` form depends on `List<string>` and therefore lands with the collections tier (Stage 23); the spelling is fixed now so CLI examples and the entry-point contract are settled ahead of implementation. `Doria\Std\Process` module contents follow with the stdlib modules.

## Alternatives considered
- **`string[]` instead of `List<string>`.** `string[]` is the semantically honest type (argv is fixed-length), but arg-parsing wants `List`'s `map`/`filter`, and growability being unused is a smaller cost than pushing parsing onto a type without those methods. `List` wins on ergonomics.
- **`main(string[] $argv, int $argc)`.** Rejected — the container carries its length; a separate `argc` is C-era redundancy.
- **`Doria\Std\Process::args` (or `Env::args`) as the sole access, `main` parameterless.** Rejected as the *primary* path: a `string[]`/`List` value is a move type, and runtime-initialized owned statics are deferred, so a `Process::args` property would need Stage-36 property hooks; the `main` parameter is populated by entry glue and needs nothing new, is more discoverable, and matches `$argv` familiarity. `Process` still owns the non-argument process facts.
- **`Console` as the home.** Rejected — arguments are process input, not terminal capability; miscategorizing them there conflates two concerns.

## Consequences
- CLI programs read arguments through a typed `main(List<string> $args)` — discoverable, no `argc`, no getter-method naming friction (a parameter is a noun binding, not a method).
- Record 0032's entry-point forms gain the optional-args variant; the two return types (`int`/`void`) are unchanged.
- `Doria\Std\Process` is reserved for exit code, pid, executable path, and related process facts.

## Affected components
Record 0032 (entry-point forms — amended by this addition), the end-to-end plan (§9 stdlib / entry point; Stage 23 for the `List` dependency), `SPEC.md` entry-point section, entry glue in `doria-rt`, and CLI examples that should parse arguments.

## Invalidated elsewhere
- Any statement or example implying a Doria program cannot read its command-line arguments, or that `main` takes no parameters.
- This record's original "element `$args[0]` … follows the platform's argv, settled with the implementation" wording — superseded by the amendment above: the executable path is stripped, `$args[0]` is the first real argument, and the executable path is a `Doria\Std\Process` fact.
- Any example or doc that indexes `$args` from 1 to skip a program name, or that treats `$args->count` as one greater than the number of user arguments.
- Record 0032's "two entry forms" framing — there are now two return types across an optional-args parameter.
