use doriac::diagnostics::{Diagnostic, DiagnosticFormat, FixApplicability, RenderOptions};

fn rejected(source: &str) -> Vec<Diagnostic> {
    doriac::check_source("collection-diagnostics.doria", source)
        .expect_err("source should be rejected")
}

fn diagnostic(source: &str, code: &str) -> Diagnostic {
    let diagnostics = rejected(source);
    diagnostics
        .into_iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}"))
}

fn apply_fix(source: &str, diagnostic: &Diagnostic) -> String {
    let fix = diagnostic
        .fixes
        .first()
        .expect("diagnostic should carry a fix");
    let mut edits = fix.edits.clone();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.span.start));
    let mut result = source.to_string();
    for edit in edits {
        result.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    result
}

#[test]
fn map_membership_spellings_have_receiver_aware_applied_fixes() {
    for family in ["Dictionary", "SortedDictionary"] {
        for written in [
            "has",
            "hasKey",
            "array_key_exists",
            "contains_key",
            "ContainsKey",
        ] {
            let initializer = if family == "Dictionary" {
                "[\"present\" => null]".to_string()
            } else {
                "SortedDictionary::from([\"present\" => null])".to_string()
            };
            let source = format!(
                "function main(): void {{ {family}<string, ?int> $values = {initializer}; echo $values->{written}(\"present\"); }}"
            );
            let error = diagnostic(&source, "E0521");
            assert_eq!(error.span, error.fixes[0].edits[0].span);
            assert_eq!(
                error.fixes[0].applicability,
                FixApplicability::MachineApplicable
            );
            assert_eq!(error.fixes[0].edits[0].replacement, "containsKey");
            let fixed = apply_fix(&source, &error);
            assert!(fixed.contains("->containsKey(\"present\")"));
            doriac::check_source("fixed-map-membership.doria", fixed)
                .expect("the canonical containsKey call should check");
        }
    }
}

#[test]
fn element_membership_and_size_suggestions_follow_receiver_semantics() {
    for declaration in [
        "int[] $values = [1, 2];",
        "List<int> $values = [1, 2];",
        "Set<int> $values = Set::from([1, 2]);",
        "SortedSet<int> $values = SortedSet::from([1, 2]);",
        "PriorityQueue<int> $values = PriorityQueue::from([1, 2]);",
        "Deque<int> $values = Deque::from([1, 2]);",
    ] {
        for written in ["in_array", "includes"] {
            let source =
                format!("function main(): void {{ {declaration} echo $values->{written}(1); }}");
            let error = diagnostic(&source, "E0521");
            assert_eq!(error.fixes[0].edits[0].replacement, "contains");
            doriac::check_source("fixed-element-membership.doria", apply_fix(&source, &error))
                .expect("receiver-supported contains should check");
        }
    }

    let map = r#"function main(): void {
        Dictionary<string, int> $values = ["one" => 1];
        echo $values->includes(1);
    }"#;
    assert!(diagnostic(map, "E0521").fixes.is_empty());

    doriac::check_source(
        "array-length.doria",
        "function main(): void { int[] $values = [1]; echo $values->length; }",
    )
    .expect("typed-array length remains canonical");
    let array_wrong_count = diagnostic(
        "function main(): void { int[] $values = [1]; echo $values->Count; }",
        "E0521",
    );
    assert!(array_wrong_count.fixes.is_empty());
}

#[test]
fn peer_spellings_cover_size_mutation_search_endpoints_and_queue_vocabulary() {
    for (declaration, written, canonical) in [
        ("List<int> $v = [1];", "size", "count"),
        ("Set<int> $v = Set::from([1]);", "len", "count"),
        ("Deque<int> $v = Deque::from([1]);", "length", "count"),
        ("writable List<int> $v = [1];", "append(2)", "add"),
        ("writable Set<int> $v = Set::from([1]);", "push(2)", "add"),
        ("writable List<int> $v = [1];", "delete(1)", "remove"),
        (
            "writable Dictionary<string, int> $v = [];",
            "unset(\"a\")",
            "remove",
        ),
        ("List<int> $v = [1];", "Min", "first"),
        ("List<int> $v = [1];", "Max", "last"),
        (
            "writable Deque<int> $v = Deque::from([1]);",
            "Enqueue(2)",
            "pushBack",
        ),
        (
            "writable Deque<int> $v = Deque::from([1]);",
            "Dequeue()",
            "popFront",
        ),
    ] {
        let source = format!("function main(): void {{ {declaration} $v->{written}; }}");
        let error = diagnostic(&source, "E0521");
        assert_eq!(error.fixes[0].edits[0].replacement, canonical);
    }

    for written in ["array_search(1)", "position(1)", "find(1)"] {
        let source = format!("function main(): void {{ List<int> $v = [1]; echo $v->{written}; }}");
        let error = diagnostic(&source, "E0521");
        assert_eq!(error.fixes[0].edits[0].replacement, "indexOf");
        assert_eq!(
            error.fixes[0].applicability,
            FixApplicability::RequiresReview
        );
    }

    doriac::check_source(
        "priority-queue-push.doria",
        "function main(): void { writable PriorityQueue<int> $v = PriorityQueue::from([]); $v->push(1); }",
    )
    .expect("PriorityQueue push remains canonical");
    let queue_append = diagnostic(
        "function main(): void { writable PriorityQueue<int> $v = PriorityQueue::from([]); $v->append(1); }",
        "E0521",
    );
    assert!(queue_append.fixes.is_empty());
}

#[test]
fn property_invocation_has_safe_and_combined_fixes() {
    let source = "function main(): void { List<int> $values = [1]; echo $values->Count(); }";
    let error = diagnostic(source, "E0557");
    assert_eq!(error.title, "Property Is Not A Method");
    assert_eq!(
        error.fixes[0].applicability,
        FixApplicability::MachineApplicable
    );
    assert_eq!(error.fixes[0].edits.len(), 2);
    let fixed = apply_fix(source, &error);
    assert!(fixed.contains("$values->count"));
    assert!(!fixed.contains("count()"));
    doriac::check_source("fixed-property.doria", fixed)
        .expect("the combined property fix should check");

    let with_argument = diagnostic(
        r#"function makeValue(): int { return 1; }
           function main(): void { List<int> $values = [1]; echo $values->count(makeValue()); }"#,
        "E0557",
    );
    assert!(with_argument.fixes.is_empty());

    for source in [
        "function main(): void { List<int> $v = [1]; echo $v->isEmpty(); }",
        "function main(): void { List<int> $v = [1]; echo $v->first(); echo $v->last(); }",
        "function main(): void { Set<int> $v = Set::from([1]); echo $v->first(); echo $v->last(); }",
        "function main(): void { SortedSet<int> $v = SortedSet::from([1]); echo $v->first(); echo $v->last(); }",
        "function main(): void { Dictionary<string, int> $v = []; echo $v->keys(); echo $v->values(); }",
        "function main(): void { PriorityQueue<int> $v = PriorityQueue::from([1]); echo $v->peek(); }",
        "function main(): void { Deque<int> $v = Deque::from([1]); echo $v->peekFront(); echo $v->peekBack(); }",
        "function main(): void { int[] $v = [1]; echo $v->length(); }",
    ] {
        let errors = rejected(source);
        assert!(errors.iter().any(|error| error.code == "E0557"));
        assert!(!errors.iter().any(|error| error.code == "E0521"));
    }
}

#[test]
fn withdrawn_literal_constructors_preserve_source_and_context() {
    for source in [
        "function main(): void { List<int> $v = List::from([1, /* keep */ 2]); }",
        "function main(): void { Dictionary<string, int> $v = Dictionary::from([\"alpha\" => 1]); }",
        "function main(): void { List<int> $v = List::from([]); }",
        "function main(): void { Dictionary<string, int> $v = Dictionary::from([]); }",
    ] {
        let error = diagnostic(source, "E0558");
        assert!(!error.message.contains("unknown class"));
        assert_eq!(error.fixes[0].applicability, FixApplicability::MachineApplicable);
        let fixed = apply_fix(source, &error);
        assert!(!fixed.contains("::from"));
        if source.contains("keep") {
            assert!(fixed.contains("/* keep */"));
        }
        doriac::check_source("fixed-literal-constructor.doria", fixed)
            .expect("direct literal replacement should check");
    }

    for source in [
        "function main(): void { let $v = List::from([]); }",
        "function main(): void { List<int> $source = [1]; let $v = List::from($source); }",
        "function main(): void { Dictionary<string, int> $source = []; let $v = Dictionary::from($source); }",
        "function main(): void { Dictionary<int, int> $v = List::from([1, 2]); }",
        "function main(): void { List<int> $v = Dictionary::from([1 => 2]); }",
        "function main(): void { List<int> $v = List::from([1, \"two\"]); }",
        "function main(): void { Dictionary<string, int> $v = Dictionary::from([\"one\" => \"two\"]); }",
    ] {
        assert!(diagnostic(source, "E0558").fixes.is_empty());
    }

    doriac::check_source(
        "valid-from-families.doria",
        r#"function main(): void {
            Set<int> $a = Set::from([1]);
            SortedSet<int> $b = SortedSet::from([1]);
            SortedDictionary<string, int> $c = SortedDictionary::from(["a" => 1]);
            PriorityQueue<int> $d = PriorityQueue::from([1]);
            Deque<int> $e = Deque::from([1]);
        }"#,
    )
    .expect("the five non-literal collection families retain ::from");
}

#[test]
fn slice_three_members_execute_and_only_slice_four_remains_pending() {
    let executable = [
        "List<int> $v = [1]; echo $v->indexOf(1) ?? -1;",
        "writable List<int> $v = [1]; echo $v->remove(1);",
        "Dictionary<string, int> $v = []; echo $v->containsValue(1);",
        "SortedDictionary<string, int> $v = SortedDictionary::from([]); echo $v->containsValue(1);",
        "Set<int> $v = Set::from([1]); echo $v->first ?? -1; echo $v->last ?? -1;",
        "SortedSet<int> $v = SortedSet::from([1]); echo $v->first ?? -1; echo $v->last ?? -1;",
    ];
    for statement in executable {
        let source = format!("function main(): void {{ {statement} }}");
        doriac::lower_source_to_mir("slice-three-member.doria", source)
            .expect("Slice 3 collection members should lower");
    }

    let pending_clear = [
        "writable List<int> $v = [1]; $v->clear();",
        "writable Dictionary<string, int> $v = []; $v->clear();",
        "writable Set<int> $v = Set::from([1]); $v->clear();",
        "writable SortedDictionary<string, int> $v = SortedDictionary::from([]); $v->clear();",
        "writable SortedSet<int> $v = SortedSet::from([1]); $v->clear();",
        "writable PriorityQueue<int> $v = PriorityQueue::from([1]); $v->clear();",
        "writable Deque<int> $v = Deque::from([1]); $v->clear();",
    ];
    for statement in pending_clear {
        let source = format!("function main(): void {{ {statement} }}");
        let errors = rejected(&source);
        let pending = errors
            .iter()
            .find(|error| error.code == "E0559")
            .unwrap_or_else(|| panic!("clear must retain an accepted-pending diagnostic"));
        assert!(pending.message.contains("clear"));
        assert!(pending.explanation.as_deref().unwrap().contains("Slice 4"));
        assert!(!errors.iter().any(|error| error.code == "E0521"));
        assert!(doriac::lower_source_to_mir("pending-member.doria", source).is_err());
    }

    let suggested = diagnostic(
        "function main(): void { List<int> $v = [1]; echo $v->find(1) ?? -1; }",
        "E0521",
    );
    assert_eq!(suggested.fixes[0].edits[0].replacement, "indexOf");
    doriac::check_source(
        "fixed-member.doria",
        apply_fix(
            "function main(): void { List<int> $v = [1]; echo $v->find(1) ?? -1; }",
            &suggested,
        ),
    )
    .expect("the safe indexOf migration should now execute");
}

#[test]
fn equality_diagnostics_name_the_actual_collection_operation() {
    for (declaration, operation) in [
        ("List<Token> $v = [];", "List::contains"),
        ("Token[] $v = [];", "T[]::contains"),
        ("Set<Token> $v = Set::from([]);", "Set::contains"),
        (
            "SortedSet<Token> $v = SortedSet::from([]);",
            "SortedSet::contains",
        ),
        (
            "PriorityQueue<Token> $v = PriorityQueue::from([]);",
            "PriorityQueue::contains",
        ),
        ("Deque<Token> $v = Deque::from([]);", "Deque::contains"),
        ("Dictionary<Token, int> $v = [];", "Dictionary::containsKey"),
        (
            "SortedDictionary<Token, int> $v = SortedDictionary::from([]);",
            "SortedDictionary::containsKey",
        ),
    ] {
        let source =
            format!(
            "class Token {{}} function main(): void {{ {declaration} echo $v->{}(new Token()); }}",
            if operation.contains("Dictionary") { "containsKey" } else { "contains" }
        );
        let errors = rejected(&source);
        assert!(
            errors
                .iter()
                .filter(|error| error.code == "E0524")
                .any(|error| error.message.contains(operation)),
            "missing receiver-aware equality diagnostic for {operation}: {errors:#?}"
        );
    }

    for (declaration, member, operation) in [
        ("List<Token> $v = [];", "indexOf", "List::indexOf"),
        ("writable List<Token> $v = [];", "remove", "List::remove"),
        (
            "Dictionary<string, Token> $v = [];",
            "containsValue",
            "Dictionary::containsValue",
        ),
        (
            "SortedDictionary<string, Token> $v = SortedDictionary::from([]);",
            "containsValue",
            "SortedDictionary::containsValue",
        ),
    ] {
        let source = format!(
            "class Token {{}} function main(): void {{ {declaration} echo $v->{member}(new Token()); }}"
        );
        let errors = rejected(&source);
        assert!(
            errors
                .iter()
                .filter(|error| error.code == "E0524")
                .any(|error| error.message.contains(operation)),
            "missing receiver-aware equality diagnostic for {operation}: {errors:#?}"
        );
    }
}

#[test]
fn list_remove_reports_the_readonly_receiver_instead_of_an_unknown_member() {
    let errors = rejected(
        "function main(): void { List<int> $values = [1, 2, 3]; echo $values->remove(2); }",
    );
    assert!(!errors.iter().any(|error| error.code == "E0521"));
    assert!(errors.iter().any(|error| {
        error.code == "E0201"
            && error.message.contains("remove")
            && error.message.contains("readonly")
    }));
}

#[test]
fn collection_fixes_survive_human_concise_and_json_projection() {
    let source = "function main(): void { Dictionary<string, int> $v = []; echo $v->has(\"a\"); }";
    let errors = rejected(source);
    for format in [
        DiagnosticFormat::Human,
        DiagnosticFormat::Concise,
        DiagnosticFormat::Json,
    ] {
        let rendered = doriac::render_diagnostics_with_options(
            "collection-diagnostics.doria",
            source,
            &errors,
            RenderOptions {
                format,
                ..RenderOptions::default()
            },
        );
        assert!(rendered.contains("E0521"));
        assert!(rendered.contains("Unknown Collection Member"));
        if format == DiagnosticFormat::Json {
            let json: serde_json::Value = serde_json::from_str(&rendered).unwrap();
            assert_eq!(
                json["diagnostics"][0]["fixes"][0]["applicability"],
                "machineApplicable"
            );
            assert_eq!(
                json["diagnostics"][0]["fixes"][0]["edits"][0]["replacement"],
                "containsKey"
            );
        }
    }
}
