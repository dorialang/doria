//! The callback-provenance solver (`semantics::nonblocking::call_contexts`)
//! gives a call a context of its own caller only when a callable value can
//! cross it. A library whose methods call shared helpers from several places,
//! as doria-tui's widgets do, has a number of call paths that grows
//! exponentially with call depth; the solver's work and memory must grow with
//! the number of calls instead. Keying every call by its caller made
//! `doriac metadata` on doria-tui run for 1 h 46 min at 6.2 GB, and the
//! depth-16 library here take 44 s at 1.3 GB.

use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A program in which every level calls the one below from `fan` call
/// sites, down to a class method, so a property hook that calls the top
/// level reaches `fan ^ depth` call paths through `fan * depth` calls.
fn layered_library(depth: usize, fan: usize) -> String {
    let mut source = String::from(
        "class Surface\n{\n    function __construct(int $width) {}\n\n    function put(int $column): int\n    {\n        return $column;\n    }\n}\n\nfunction level0(Surface $surface, int $column): int\n{\n    return $surface->put($column);\n}\n",
    );
    for level in 1..=depth {
        let calls = (0..fan)
            .map(|site| format!("level{}($surface, $column + {site})", level - 1))
            .collect::<Vec<_>>()
            .join(" + ");
        source.push_str(&format!(
            "\nfunction level{level}(Surface $surface, int $column): int\n{{\n    return {calls};\n}}\n"
        ));
    }
    // A property hook must not block, so the solver walks every call the
    // hook reaches.
    source.push_str(&format!(
        "\nclass Gauge\n{{\n    int $reading {{ get {{ return level{depth}(new Surface(1), 0); }} }}\n}}\n\nfunction main(): void\n{{\n    let $surface = new Surface(80);\n    echo level{depth}($surface, 0);\n}}\n"
    ));
    source
}

/// Runs the metadata analysis `doriac metadata --schema-version 3` performs,
/// failing when it does not finish within `limit`.
fn analyze_within(source: String, limit: Duration) -> Duration {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let start = Instant::now();
        let result = doriac::metadata_source_v3("library.doria", source);
        let _ = sender.send((result.map(|_| ()), start.elapsed()));
    });
    let (result, elapsed) = receiver
        .recv_timeout(limit)
        .unwrap_or_else(|_| panic!("the metadata analysis did not finish within {limit:?}"));
    result.unwrap_or_else(|diagnostics| panic!("the library does not check: {diagnostics:#?}"));
    elapsed
}

#[test]
fn call_paths_do_not_multiply_the_solver_work() {
    // 2^24 call paths. With a context per call path this does not finish.
    let shallow = analyze_within(layered_library(12, 2), Duration::from_secs(120));
    let deep = analyze_within(layered_library(24, 2), Duration::from_secs(120));
    // Twice the calls, 4096 times the call paths: the time follows the calls.
    assert!(
        deep <= shallow * 8 + Duration::from_secs(2),
        "depth 12 took {shallow:?} and depth 24 took {deep:?}"
    );
}

#[test]
fn a_wide_fan_of_calls_stays_bounded() {
    // 4^10 call paths through 40 calls.
    analyze_within(layered_library(10, 4), Duration::from_secs(120));
}
