fn analyze(source: &str) -> doriac::semantics::SemanticAnalysis {
    doriac::analyze_source_for_ide("coherence.doria", source)
        .expect("source parses")
        .1
}

fn assert_accepted(source: &str) {
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{source}: {:?}",
        analysis.diagnostics
    );
}

fn one_conflict(source: &str) -> doriac::semantics::SemanticAnalysis {
    let analysis = analyze(source);
    let failures = analysis.diagnostics.iter().collect::<Vec<_>>();
    assert_eq!(failures.len(), 1, "{source}: {:?}", analysis.diagnostics);
    assert_eq!(failures[0].code, "E0752");
    assert_eq!(
        &source[failures[0].span.start..failures[0].span.end],
        "Root<string>"
    );
    for guard in ["Ready", "Writable"] {
        assert!(
            failures[0]
                .related
                .iter()
                .any(|related| &source[related.span.start..related.span.end] == guard),
            "{:?}",
            failures[0]
        );
    }
    analysis
}

const DECLARATIONS: &str = r#"
interface Ready {}
interface Writable {}
interface BothCaps extends Ready, Writable {}
interface Root<E> {}
class Both implements Ready, Writable {}
class ReadyOnly implements Ready {}
class WritableOnly implements Writable {}
class Neither {}
class Adapter<T> implements Root<int> where (T implements Ready), Root<string> where (T implements Writable) {}
"#;

#[test]
fn concrete_parameter_coherence_is_checked_without_iteration() {
    let caller = "function inspect(Adapter<Both> $value): void {}";
    for source in [
        format!("{DECLARATIONS}{caller}"),
        format!("{caller}{DECLARATIONS}"),
    ] {
        one_conflict(&source);
    }
}

#[test]
fn symbolic_formations_keep_frozen_caller_bounds() {
    let good = "function good<T implements Ready>(Adapter<T> $value): void {}";
    let bad = "function bad<T implements BothCaps>(Adapter<T> $value): void {}";
    assert_accepted(&format!("{DECLARATIONS}{good}"));
    for callers in [format!("{good}{bad}"), format!("{bad}{good}")] {
        for source in [
            format!("{DECLARATIONS}{callers}"),
            format!("{callers}{DECLARATIONS}"),
        ] {
            one_conflict(&source);
        }
    }
}

#[test]
fn symbolic_dependency_closure_uses_only_the_original_callers_proved_bounds() {
    for owner in [
        "class Owner<T> { ?Adapter<T> $value = null; }",
        "class Inner<T> { ?Adapter<T> $value = null; } class Owner<T> { ?Inner<T> $value = null; }",
        "trait Storage<T> { ?Adapter<T> $value = null; } class Owner<T> { uses Storage<T>; }",
        "open class Base<T> { ?Adapter<T> $value = null; } class Owner<T> extends Base<T> {}",
        "interface Uses<E> {} class Owner<T> implements Uses<Adapter<T>> where (T implements Ready) {}",
        "interface Uses<E> {} class Inner<T> { ?Adapter<T> $value = null; } class Owner<T> implements Uses<Inner<T>> where (T implements Ready) {}",
    ] {
        for (parameters, conflict) in [
            ("U", false),
            ("U implements Ready", false),
            ("U implements BothCaps", true),
        ] {
            let caller = format!("function inspect<{parameters}>(Owner<U> $source): void {{}}");
            for source in [
                format!("{DECLARATIONS}{owner}{caller}"),
                format!("{caller}{owner}{DECLARATIONS}"),
            ] {
                if conflict {
                    one_conflict(&source);
                } else {
                    assert_accepted(&source);
                }
            }
        }
    }
}

#[test]
fn symbolic_targets_do_not_assume_their_own_unproved_guard() {
    let owner = "interface Never {} interface Uses<E> {} class Owner<T> implements Uses<Adapter<T>> where (T implements Never) {}";
    let caller = "function inspect<U implements BothCaps>(Owner<U> $source): void {}";
    assert_accepted(&format!("{DECLARATIONS}{owner}{caller}"));
}

#[test]
fn symbolic_dependency_recursion_is_finite_and_preserves_deferred_expansion() {
    for owner in [
        "class Owner<T> { ?Owner<T> $next = null; }",
        "class Owner<T> { ?Owner<int> $next = null; }",
        "interface Uses<E> {} class Owner<T> implements Uses<Owner<T>> where (T implements Ready) {}",
        "interface Uses<E> {} class Inner<T> implements Uses<Owner<T>> {} class Owner<T> { ?Inner<T> $next = null; }",
        "class Owner<T> { ?Owner<List<T>> $next = null; }",
    ] {
        let caller = "function inspect<U>(Owner<U> $source): void {}";
        assert_accepted(&format!("{DECLARATIONS}{owner}{caller}"));
    }
    let owner = "class Owner<T> { ?Owner<List<T>> $next = null; }";
    let caller = "function inspect<U>(Owner<U> $source): void {} function actual(Owner<int> $source): void { inspect($source); }";
    let analysis = analyze(&format!("{DECLARATIONS}{owner}{caller}"));
    let failures = analysis.diagnostics.iter().collect::<Vec<_>>();
    assert_eq!(failures.len(), 1, "{:?}", analysis.diagnostics);
    assert_eq!(failures[0].code, "E0539");
}

#[test]
fn inactive_and_single_capability_specializations_remain_coherent() {
    for argument in ["ReadyOnly", "WritableOnly", "Neither"] {
        let caller = format!("function inspect(Adapter<{argument}> $value): void {{}}");
        for source in [
            format!("{DECLARATIONS}{caller}"),
            format!("{caller}{DECLARATIONS}"),
        ] {
            assert_accepted(&source);
        }
    }
}

#[test]
fn inferred_generic_results_are_checked_at_substitution() {
    let factory = "function make<T>(T $value): ?Adapter<T> { return null; }";
    let caller = "function inspect(Both $value): void { let $result = make($value); }";
    for source in [
        format!("{DECLARATIONS}{factory}{caller}"),
        format!("{caller}{factory}{DECLARATIONS}"),
    ] {
        one_conflict(&source);
    }
}

#[test]
fn callable_local_coherence_obligations_follow_generic_call_chains() {
    let functions = r#"
function leaf<T>(T $source): void { ?Adapter<T> $adapter = null; }
function middle<U>(U $source): void { leaf($source); }
"#;
    for argument in ["ReadyOnly", "WritableOnly", "Neither", "Both"] {
        let caller = format!("function inspect({argument} $source): void {{ middle($source); }}");
        for source in [
            format!("{DECLARATIONS}{functions}{caller}"),
            format!("{caller}{functions}{DECLARATIONS}"),
        ] {
            if argument == "Both" {
                one_conflict(&source);
            } else {
                assert_accepted(&source);
            }
        }
    }
    assert_accepted(&format!("{DECLARATIONS}{functions}"));
}

#[test]
fn symbolic_interface_specializations_may_become_equal_after_substitution() {
    let declarations = r#"
interface Ready {}
interface Writable {}
interface Root<E> {}
class Both implements Ready, Writable {}
class Adapter<T, E> implements Root<E> where (T implements Ready), Root<int> where (T implements Writable) {}
function leaf<T, E>(T $source, E $element): void { ?Adapter<T, E> $adapter = null; }
function middle<U, V>(U $source, V $element): void { leaf($source, $element); }
"#;
    assert_accepted(declarations);
    for (ty, conflict) in [("int", false), ("string", true)] {
        let caller = format!(
            "function inspect(Both $source, {ty} $element): void {{ middle($source, $element); }}"
        );
        for source in [
            format!("{declarations}{caller}"),
            format!("{caller}{declarations}"),
        ] {
            if !conflict {
                assert_accepted(&source);
                continue;
            }
            let analysis = analyze(&source);
            let failures = analysis.diagnostics.iter().collect::<Vec<_>>();
            assert_eq!(failures.len(), 1, "{:?}", analysis.diagnostics);
            assert_eq!(failures[0].code, "E0752");
        }
    }
}

#[test]
fn symbolic_canonical_entry_does_not_hide_concrete_conflicting_paths() {
    let source = r#"
interface Ready {}
interface Root<E> {}
class Adapter<T, E> implements Root<E>, Root<int> where (T implements Ready), Root<string> where (T implements Ready) {}
function inspect<E, T implements Ready>(Adapter<T, E> $source): void {}
"#;
    let analysis = analyze(source);
    let failures = analysis.diagnostics.iter().collect::<Vec<_>>();
    assert_eq!(failures.len(), 1, "{:?}", analysis.diagnostics);
    assert_eq!(failures[0].code, "E0752");
}

#[test]
fn generic_callable_obligations_follow_stored_and_active_target_dependencies() {
    for owner in [
        "class Owner<T> { ?Adapter<T> $value = null; }",
        "interface Uses<E> {} class Owner<T> implements Uses<Adapter<T>> where (T implements Ready) {}",
    ] {
        for argument in ["ReadyOnly", "Neither", "Both"] {
            let functions = format!(r#"
function leaf<T>(T $source): void {{ ?Owner<T> $owner = null; }}
function middle<U>(U $source): void {{ leaf($source); }}
function inspect({argument} $source): void {{ middle($source); }}
"#);
            let source = format!("{DECLARATIONS}{owner}{functions}");
            if argument == "Both" {
                one_conflict(&source);
            } else {
                assert_accepted(&source);
            }
        }
    }
}

#[test]
fn generic_method_and_composed_trait_obligations_use_authored_bindings() {
    for (owner, invoke) in [
        (
            "class Worker { static function run<T>(T $source): void { ?Adapter<T> $adapter = null; } }",
            "Worker::run($source);",
        ),
        (
            "class Worker<T> { function run(T $source): void { ?Adapter<T> $adapter = null; } }",
            "let $worker = new Worker<U>(); $worker->run($source);",
        ),
        (
            "trait Work<T> { function run(T $source): void { ?Adapter<T> $adapter = null; } } class Worker<T> { uses Work<T>; }",
            "let $worker = new Worker<U>(); $worker->run($source);",
        ),
    ] {
        for argument in ["ReadyOnly", "Neither", "Both"] {
            let functions = format!(r#"
function middle<U>(U $source): void {{ {invoke} }}
function inspect({argument} $source): void {{ middle($source); }}
"#);
            for source in [
                format!("{DECLARATIONS}{owner}{functions}"),
                format!("{functions}{owner}{DECLARATIONS}"),
            ] {
                if argument == "Both" {
                    one_conflict(&source);
                } else {
                    assert_accepted(&source);
                }
            }
        }
    }
}

#[test]
fn nested_formed_types_are_checked_without_iteration() {
    for ty in [
        "?Adapter<Both>",
        "List<Adapter<Both>>",
        "Adapter<Both>[]",
        "Dictionary<string, ?Adapter<Both>>",
    ] {
        let caller = format!("function inspect({ty} $value): void {{}}");
        one_conflict(&format!("{DECLARATIONS}{caller}"));
    }
}

#[test]
fn substituted_property_types_are_checked_at_member_access() {
    let owner = "class Owner<T> { ?Adapter<T> $value = null; }";
    let caller = "function inspect(Owner<Both> $owner): void { $owner->value; }";
    for source in [
        format!("{DECLARATIONS}{owner}{caller}"),
        format!("{caller}{owner}{DECLARATIONS}"),
    ] {
        one_conflict(&source);
    }
}

#[test]
fn concrete_stored_types_are_checked_without_member_access() {
    for owner in [
        "class Owner<T> { ?Adapter<T> $value = null; }",
        "class Inner<T> { ?Adapter<T> $value = null; } class Owner<T> { ?Inner<T> $inner = null; }",
        "open class Base<T> { ?Adapter<T> $value = null; } class Owner<T> extends Base<T> {}",
        "trait Storage<T> { ?Adapter<T> $value = null; } class Owner<T> { uses Storage<T>; }",
        "class Owner<T> { ?Adapter<T> $value = null { get => $this->value; } }",
    ] {
        let caller = "function inspect(Owner<Both> $owner): void {}";
        for source in [
            format!("{DECLARATIONS}{owner}{caller}"),
            format!("{caller}{owner}{DECLARATIONS}"),
        ] {
            one_conflict(&source);
        }
    }
}

#[test]
fn concrete_active_conformance_targets_follow_nested_storage_and_inheritance() {
    for owner in [
        "interface Uses<E> {} class Owner<T> implements Uses<Adapter<T>> where (T implements Ready) {}",
        "interface Uses<E> {} interface View<E> extends Uses<E> {} class Owner<T> implements View<Adapter<T>> where (T implements Ready) {}",
        "interface Uses<E> {} open class Base<T> implements Uses<Adapter<T>> where (T implements Ready) {} class Owner<T> extends Base<T> {}",
        "interface Uses<E> {} class Inner<T> implements Uses<Adapter<T>> where (T implements Ready) {} class Owner<T> implements Uses<Inner<T>> where (T implements Ready) {}",
        "interface Uses<E> {} class Inner<T> { ?Adapter<T> $value = null; } class Owner<T> implements Uses<Inner<T>> where (T implements Ready) {}",
        "interface Uses<E> {} class Inner<T> implements Uses<Adapter<T>> where (T implements Ready) {} class Owner<T> { ?Inner<T> $value = null; }",
    ] {
        let caller = "function inspect(Owner<Both> $owner): void {}";
        for source in [format!("{DECLARATIONS}{owner}{caller}"), format!("{caller}{owner}{DECLARATIONS}")] {
            one_conflict(&source);
        }
    }
}

#[test]
fn concrete_active_target_arguments_share_compound_type_traversal() {
    for ty in [
        "?Adapter<T>",
        "List<Adapter<T>>",
        "Adapter<T>[]",
        "Dictionary<string, ?Adapter<T>>",
    ] {
        let owner = format!("interface Uses<E> {{}} class Owner<T> implements Uses<{ty}> where (T implements Ready) {{}}");
        let caller = "function inspect(Owner<Both> $owner): void {}";
        for source in [
            format!("{DECLARATIONS}{owner}{caller}"),
            format!("{caller}{owner}{DECLARATIONS}"),
        ] {
            one_conflict(&source);
        }
    }
}

#[test]
fn concrete_target_closure_coalesces_repeated_dependencies() {
    let owner = "interface Uses<E> {} interface Also<E> {} interface View<E> extends Uses<E>, Also<E> {} \
        class Owner<T> implements View<Adapter<T>> where (T implements Ready), Also<Adapter<T>> where (T implements Ready) {}";
    let caller = "function inspect(Owner<Both> $owner): void {}";
    for source in [
        format!("{DECLARATIONS}{owner}{caller}"),
        format!("{caller}{owner}{DECLARATIONS}"),
    ] {
        one_conflict(&source);
    }
}

#[test]
fn all_active_overlap_arguments_are_checked_independently_of_the_canonical_entry() {
    for entries in [
        "Uses<int>, Uses<Adapter<T>> where (T implements Ready)",
        "Uses<Adapter<T>> where (T implements Ready), Uses<int>",
    ] {
        let owner = format!("interface Uses<E> {{}} class Owner<T> implements {entries} {{}}");
        let caller = "function inspect(Owner<Both> $owner): void {}";
        for source in [
            format!("{DECLARATIONS}{owner}{caller}"),
            format!("{caller}{owner}{DECLARATIONS}"),
        ] {
            let analysis = analyze(&source);
            let failures = analysis.diagnostics.iter().collect::<Vec<_>>();
            assert_eq!(failures.len(), 2, "{source}: {failures:?}");
            for name in ["Root", "Uses"] {
                let failure = failures
                    .iter()
                    .find(|finding| finding.message.contains(&format!("of `{name}`")))
                    .expect("each active overlap retains its own finding");
                assert_eq!(failure.code, "E0752");
            }
        }
    }
}

#[test]
fn checking_only_target_traversal_preserves_unproved_guards_and_coherent_substitutions() {
    let owner = "interface Uses<E> {} class Owner<T> implements Uses<Adapter<T>> where (T implements Ready) {}";
    for argument in ["ReadyOnly", "WritableOnly", "Neither"] {
        let caller = format!("function inspect(Owner<{argument}> $owner): void {{}}");
        for source in [
            format!("{DECLARATIONS}{owner}{caller}"),
            format!("{caller}{owner}{DECLARATIONS}"),
        ] {
            assert_accepted(&source);
        }
    }
    // No executable admission is claimed for an unproved conditional target.
    let owner = "interface Never {} interface Uses<E> {} class Owner<T> implements Uses<Adapter<T>> where (T implements Never) {}";
    let caller = "function inspect(Owner<Both> $owner): void {}";
    for source in [
        format!("{DECLARATIONS}{owner}{caller}"),
        format!("{caller}{owner}{DECLARATIONS}"),
    ] {
        assert_accepted(&source);
    }
}

#[test]
fn concrete_target_closure_preserves_stable_recursion_and_finite_collapse() {
    for owner in [
        "interface Uses<E> {} class Owner<T> implements Uses<Owner<T>> {}",
        "interface Uses<E> {} class Owner<T> implements Uses<Owner<int>> {}",
        "interface Uses<E> {} class Inner<T> implements Uses<Owner<T>> {} class Owner<T> implements Uses<Inner<T>> {}",
    ] {
        let caller = "function inspect(Owner<Both> $owner): void {}";
        for source in [format!("{DECLARATIONS}{owner}{caller}"), format!("{caller}{owner}{DECLARATIONS}")] {
            assert_accepted(&source);
        }
    }
}

#[test]
fn concrete_target_and_storage_expansion_share_cycle_identity() {
    for owner in [
        "interface Uses<E> {} class Owner<T> implements Uses<Owner<List<T>>> {}",
        "interface Uses<E> {} class Inner<T> implements Uses<Owner<T>> {} class Owner<T> implements Uses<Inner<List<T>>> {}",
        "interface Uses<E> {} class Inner<T> implements Uses<Owner<List<T>>> {} class Owner<T> { ?Inner<T> $value = null; }",
    ] {
        let caller = "function inspect(Owner<int> $owner): void {}";
        for source in [format!("{DECLARATIONS}{owner}{caller}"), format!("{caller}{owner}{DECLARATIONS}")] {
            let analysis = analyze(&source);
            let failures = analysis.diagnostics.iter().collect::<Vec<_>>();
            assert_eq!(failures.len(), 1, "{source}: {failures:?}");
            assert_eq!(failures[0].code, "E0539");
        }
    }
}

#[test]
fn stored_coherence_does_not_admit_owned_static_properties() {
    let owner = "class Owner<T> { static ?Adapter<T> $value = null; }";
    let caller = "function inspect(Owner<Both> $owner): void {}";
    for source in [
        format!("{DECLARATIONS}{owner}{caller}"),
        format!("{caller}{owner}{DECLARATIONS}"),
    ] {
        let analysis = analyze(&source);
        let mut codes = analysis
            .diagnostics
            .iter()
            .map(|finding| finding.code)
            .collect::<Vec<_>>();
        codes.sort_unstable();
        assert_eq!(codes, ["E0486", "E0752"], "{source}");
    }
}

#[test]
fn computed_properties_do_not_expand_as_unconditional_storage() {
    let owner =
        "class Owner<T> { ?Adapter<T> $value where (T implements Ready) { get { return null; } } }";
    let caller = "function inspect(Owner<Neither> $owner): void {}";
    for source in [
        format!("{DECLARATIONS}{owner}{caller}"),
        format!("{caller}{owner}{DECLARATIONS}"),
    ] {
        assert_accepted(&source);
    }
}

#[test]
fn concrete_stored_type_checks_preserve_inactive_and_single_capabilities() {
    let owner =
        "class Inner<T> { ?Adapter<T> $value = null; } class Owner<T> { ?Inner<T> $inner = null; }";
    for argument in ["ReadyOnly", "WritableOnly", "Neither"] {
        let caller = format!("function inspect(Owner<{argument}> $owner): void {{}}");
        for source in [
            format!("{DECLARATIONS}{owner}{caller}"),
            format!("{caller}{owner}{DECLARATIONS}"),
        ] {
            assert_accepted(&source);
        }
    }
}

#[test]
fn stored_type_coherence_preserves_stable_recursion_and_finite_collapse() {
    for owner in [
        "class Owner<T> { ?Owner<T> $next = null; ?Adapter<T> $value = null; }",
        "class Inner<T> { ?Owner<T> $next = null; ?Adapter<T> $value = null; } class Owner<T> { ?Inner<T> $inner = null; }",
        "class Owner<T> { ?Owner<Both> $next = null; ?Adapter<T> $value = null; }",
    ] {
        for caller in [
            "function inspect(Owner<Both> $owner): void {}",
            "function inspect(Owner<ReadyOnly> $owner): void {}",
        ] {
            let source = format!("{DECLARATIONS}{owner}{caller}");
            if caller.contains("<Both>") || owner.contains("Owner<Both>") {
                one_conflict(&source);
            } else {
                assert_accepted(&source);
            }
        }
    }
}

#[test]
fn concrete_storage_expansion_retains_the_existing_recursion_diagnostic() {
    for owner in [
        "class Owner<T> { ?Owner<List<T>> $next = null; }",
        "class Inner<T> { ?Owner<T> $next = null; } class Owner<T> { ?Inner<List<T>> $inner = null; }",
    ] {
        let caller = "function inspect(Owner<int> $owner): void {}";
        for source in [
            format!("{DECLARATIONS}{owner}{caller}"),
            format!("{caller}{owner}{DECLARATIONS}"),
        ] {
            let analysis = analyze(&source);
            let failures = analysis
                .diagnostics
                .iter()
                .collect::<Vec<_>>();
            assert_eq!(failures.len(), 1, "{source}: {failures:?}");
            assert_eq!(failures[0].code, "E0539");
        }
    }
}

#[test]
fn incoherent_predicate_argument_types_are_discarded() {
    let gate = "interface Uses<E> {} trait Gate<T> { function inspect(): void where (T implements Uses<Adapter<Both>>) {} }";
    let source = format!("{DECLARATIONS}{gate}");
    let analysis = one_conflict(&source);
    assert!(analysis
        .info
        .contracts
        .conditions
        .values()
        .all(|condition| !source[condition.clause.start..condition.clause.end].contains("Uses<")));
}

#[test]
fn composed_guard_subject_types_are_checked_after_receiver_substitution() {
    let declarations = DECLARATIONS.replace("Adapter", "Bad");
    let gate = "trait Gate<T> { function inspect(): int where (T implements Root<int>) { return 1; } } class Owner<U> { uses Gate<Bad<U>>; }";
    let caller = "function invoke(Owner<Both> $owner): int { return $owner->inspect(); }";
    for source in [
        format!("{declarations}{gate}{caller}"),
        format!("{caller}{gate}{declarations}"),
    ] {
        one_conflict(&source);
    }
}

#[test]
fn deferred_guard_coherence_retains_the_generic_calling_body() {
    let declarations = DECLARATIONS.replace("Adapter", "Bad");
    let gate = "trait Gate<T> { function inspect(): int where (T implements Root<int>) { return 1; } } class Owner<U> { uses Gate<Bad<U>>; }";
    let middle =
        "function middle<U implements Ready>(Owner<U> $owner): int { return $owner->inspect(); }";
    for argument in ["ReadyOnly", "Both"] {
        let caller =
            format!("function invoke(Owner<{argument}> $owner): int {{ return middle($owner); }}");
        for source in [
            format!("{declarations}{gate}{middle}{caller}"),
            format!("{caller}{middle}{gate}{declarations}"),
        ] {
            if argument == "Both" {
                one_conflict(&source);
            } else {
                assert_accepted(&source);
            }
        }
    }
}

#[test]
fn composed_guard_interface_arguments_are_checked_after_receiver_substitution() {
    let declarations = DECLARATIONS.replace("Adapter", "Bad");
    let gate = "interface Uses<E> {} class Source<U> implements Uses<Bad<U>> where (U implements Ready) {} \
        trait Gate<U, T> { function inspect(T $value): int where (T implements Uses<Bad<U>>) { return 1; } } \
        class Owner<U> { uses Gate<U, Source<U>>; }";
    let caller = "function invoke(Owner<Both> $owner, Source<Both> $value): int { return $owner->inspect($value); }";
    for source in [
        format!("{declarations}{gate}{caller}"),
        format!("{caller}{gate}{declarations}"),
    ] {
        one_conflict(&source);
    }
}

#[test]
fn inherited_conformance_coherence_is_checked_without_iteration() {
    let declarations = r#"
interface Ready {}
interface Writable {}
interface Root<E> {}
class Both implements Ready, Writable {}
open class Base<T> implements Root<int> where (T implements Ready) {}
class Child<T> extends Base<T> implements Root<string> where (T implements Writable) {}
"#;
    let caller = "function inspect(Child<Both> $value): void {}";
    for source in [
        format!("{declarations}{caller}"),
        format!("{caller}{declarations}"),
    ] {
        one_conflict(&source);
    }
}
