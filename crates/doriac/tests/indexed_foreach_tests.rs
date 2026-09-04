use doriac::ast::{Item as AstItem, Stmt as AstStmt};
use doriac::diagnostics::{Diagnostic, DiagnosticSeverity, FixApplicability};
use doriac::hir::{Item as HirItem, Stmt as HirStmt};
use doriac::mir::{self, ControlFlowPlan, ForeachIterationKind, Statement};
use doriac::names::{PackageIdentity, SourceIdentity};
use doriac::semantics::{
    ForeachIterableFamily, ForeachIterationKind as SemanticIterationKind, ForeachIterationOrder,
    ForeachValueAccess,
};
use doriac::types::{IntegerType, ResolvedType};

fn analyze(source: &str) -> doriac::semantics::SemanticAnalysis {
    let (_, analysis) = doriac::analyze_source_for_ide("indexed-foreach.doria", source)
        .expect("indexed foreach source should parse");
    analysis
}

fn errors(source: &str) -> Vec<Diagnostic> {
    analyze(source)
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        .collect()
}

fn interpret(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    let mir = doriac::lower_source_to_mir("indexed-foreach.doria", source)
        .unwrap_or_else(|diagnostics| panic!("indexed foreach should lower: {diagnostics:#?}"));
    doriac::mir_interpreter::interpret(&mir).expect("indexed foreach MIR should execute")
}

fn foreach_plans(program: &mir::Program) -> Vec<&mir::ForeachPlan> {
    program
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement {
            Statement::ControlFlowPlan(ControlFlowPlan::Foreach(plan)) => Some(plan),
            _ => None,
        })
        .collect()
}

fn first_foreach_plan_location(program: &mir::Program) -> (usize, usize, usize) {
    program
        .functions
        .iter()
        .enumerate()
        .find_map(|(function_index, function)| {
            function
                .blocks
                .iter()
                .enumerate()
                .find_map(|(block_index, block)| {
                    block
                        .statements
                        .iter()
                        .position(|statement| {
                            matches!(
                                statement,
                                Statement::ControlFlowPlan(ControlFlowPlan::Foreach(_))
                            )
                        })
                        .map(|statement_index| (function_index, block_index, statement_index))
                })
        })
        .expect("foreach plan")
}

fn first_foreach_plan_mut(program: &mut mir::Program) -> &mut mir::ForeachPlan {
    let (function, block, statement) = first_foreach_plan_location(program);
    let Statement::ControlFlowPlan(ControlFlowPlan::Foreach(plan)) =
        &mut program.functions[function].blocks[block].statements[statement]
    else {
        unreachable!("located statement is a foreach plan");
    };
    plan
}

#[test]
fn parser_preserves_neutral_bindings_and_exact_authored_spans() {
    let source = r#"
function main(): void
{
    List<string> $items = ["a"];
    foreach (
        $items as writable int $index => string $item
    ) {
        echo $item;
    }
    echo "after";
}
"#;
    let program = doriac::parse_source("indexed-foreach.doria", source).expect("source parses");
    let AstItem::Function(function) = &program.items[0] else {
        panic!("expected function");
    };
    let AstStmt::Foreach(foreach) = &function.body.statements[1] else {
        panic!("expected foreach");
    };
    let first = foreach.first_binding.as_ref().expect("first binding");
    assert_eq!(
        &source[first.span.start..first.span.end],
        "writable int $index"
    );
    assert_eq!(
        &source[first.writable_span.expect("modifier span").start
            ..first.writable_span.expect("modifier span").end],
        "writable"
    );
    assert_eq!(
        &source[first.type_span.expect("type span").start..first.type_span.expect("type span").end],
        "int"
    );
    assert_eq!(
        &source[first.name_span.start..first.name_span.end],
        "$index"
    );
    assert_eq!(
        &source[foreach.value_binding.span.start..foreach.value_binding.span.end],
        "string $item"
    );
    assert!(matches!(function.body.statements[2], AstStmt::Echo { .. }));
}

#[test]
fn semantic_facts_distinguish_sequence_indexes_dictionary_keys_and_value_only_sources() {
    let source = r#"
function main(): void
{
    List<string> $list = ["a"];
    string[] $array = ["b"];
    Dictionary<string, int> $map = ["answer" => 42];
    Set<int> $set = Set::from([1]);
    foreach ($list as int $i => string $value) {}
    foreach ($array as int $i => string $value) {}
    foreach ($map as string $key => int $value) {}
    foreach ($set as int $value) {}
}
"#;
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let mut facts = analysis.info.foreach_loops.values().collect::<Vec<_>>();
    facts.sort_by_key(|fact| {
        fact.first_binding_span
            .map_or(fact.value_binding_span.start, |s| s.start)
    });
    assert_eq!(facts.len(), 4);
    assert_eq!(facts[0].iterable_family, ForeachIterableFamily::List);
    assert_eq!(
        facts[0].iteration_kind,
        SemanticIterationKind::SequenceIndex
    );
    assert_eq!(
        facts[0].first_binding_type,
        Some(ResolvedType::Integer(IntegerType::Int64))
    );
    assert_eq!(facts[0].value_binding_type, ResolvedType::String);
    assert!(facts[0].first_binding_type_span.is_some());
    assert!(facts[0].value_binding_type_span.is_some());
    assert_eq!(facts[0].value_access, ForeachValueAccess::Readonly);
    assert_eq!(
        facts[0].source,
        SourceIdentity("indexed-foreach.doria".into())
    );
    assert_eq!(facts[0].package, PackageIdentity::Standalone);
    assert_eq!(facts[1].iterable_family, ForeachIterableFamily::TypedArray);
    assert_eq!(
        facts[1].iteration_kind,
        SemanticIterationKind::SequenceIndex
    );
    assert_eq!(facts[2].iterable_family, ForeachIterableFamily::Dictionary);
    assert_eq!(
        facts[2].iteration_kind,
        SemanticIterationKind::DictionaryKey
    );
    assert_eq!(facts[2].first_binding_type, Some(ResolvedType::String));
    assert_eq!(facts[3].iterable_family, ForeachIterableFamily::Set);
    assert_eq!(facts[3].iteration_kind, SemanticIterationKind::ValueOnly);
    assert_eq!(facts[3].first_binding_type, None);
    assert_eq!(facts[3].iteration_order, ForeachIterationOrder::Insertion);
}

#[test]
fn every_foreach_binding_requires_an_explicit_type_with_local_fixes() {
    let source = r#"
function main(): void
{
    List<string> $items = ["a"];
    foreach ($items as $index => $item) {}
    foreach ($items as int $index => $item) {}
    foreach ($items as $item) {}
    foreach (0..<2 as $number) {}
}
"#;
    let analysis = analyze(source);
    let diagnostics = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        .filter(|diagnostic| diagnostic.code == "E0748")
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 5, "{diagnostics:#?}");
    assert!(diagnostics
        .iter()
        .all(|diagnostic| diagnostic.title == "Foreach Binding Type Is Required"));
    assert_eq!(
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.fixes[0].edits[0].replacement.as_str())
            .collect::<Vec<_>>(),
        ["int ", "string ", "string ", "string ", "int "]
    );
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic.fixes[0].applicability == FixApplicability::MachineApplicable
    }));
    assert_eq!(
        analysis
            .info
            .foreach_loops
            .values()
            .filter(|loop_info| loop_info.first_binding_type_span.is_some())
            .count(),
        1
    );
    assert!(analysis
        .info
        .foreach_loops
        .values()
        .all(|loop_info| loop_info.value_binding_type_span.is_none()));

    let lowering = doriac::lower_source_to_mir("indexed-foreach.doria", source)
        .expect_err("missing foreach types must fail before MIR");
    assert!(lowering.iter().all(|diagnostic| diagnostic.code == "E0748"));
}

#[test]
fn explicit_binding_types_cover_every_iterable_family_and_element_shape() {
    for (source, expected_replacements) in [
        (
            r#"
function main(): void
{
    List<string> $list = ["a"];
    string[] $array = ["b"];
    foreach ($list as $index => $value) {}
    foreach ($array as $index => $value) {}
}
"#,
            vec!["int ", "string ", "int ", "string "],
        ),
        (
            r#"
function main(): void
{
    Dictionary<string, int> $dictionary = ["a" => 1];
    SortedDictionary<int, string> $sorted = SortedDictionary::from([1 => "a"]);
    foreach ($dictionary as $key => $value) {}
    foreach ($sorted as $key => $value) {}
}
"#,
            vec!["string ", "int ", "int ", "string "],
        ),
        (
            r#"
function main(): void
{
    Set<int> $set = Set::from([1]);
    SortedSet<int> $sorted = SortedSet::from([1]);
    Deque<string> $deque = Deque::from(["a"]);
    foreach ($set as $value) {}
    foreach ($sorted as $value) {}
    foreach ($deque as $value) {}
}
"#,
            vec!["int ", "int ", "string "],
        ),
        (
            r#"
function main(): void
{
    Dictionary<string, int> $dictionary = ["a" => 1];
    foreach ($dictionary->keys as $key) {}
    foreach ($dictionary->values as $value) {}
}
"#,
            vec!["string ", "int "],
        ),
        (
            r#"
function visit<T>(List<T> $items): void
{
    foreach ($items as $item) {}
}
"#,
            vec!["T "],
        ),
        (
            r#"
function main(): void
{
    List<?int> $nullable = [null];
    writable List<string> $writable = ["a"];
    foreach ($nullable as $value) {}
    foreach ($writable as writable $value) {}
}
"#,
            vec!["?int ", "string "],
        ),
    ] {
        let diagnostics = errors(source);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            vec!["E0748"; expected_replacements.len()],
            "{diagnostics:#?}"
        );
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.fixes[0].edits[0].replacement.as_str())
                .collect::<Vec<_>>(),
            expected_replacements
        );
    }
}

#[test]
fn invalid_first_bindings_are_rejected_with_local_machine_fixes_and_no_backend_error() {
    for (family, declaration) in [
        ("Integer range", "foreach (0..<2 as int $i => int $value) {}"),
        ("Set", "Set<int> $v = Set::from([1]); foreach ($v as int $i => int $value) {}"),
        ("SortedSet", "SortedSet<int> $v = SortedSet::from([1]); foreach ($v as int $i => int $value) {}"),
        ("Deque", "Deque<int> $v = Deque::from([1]); foreach ($v as int $i => int $value) {}"),
        ("Dictionary keys projection", "Dictionary<string, int> $v = [\"a\" => 1]; foreach ($v->keys as int $i => string $value) {}"),
        ("Dictionary values projection", "Dictionary<string, int> $v = [\"a\" => 1]; foreach ($v->values as int $i => int $value) {}"),
    ] {
        let source = format!("function main(): void {{ {declaration} }}");
        let diagnostics = errors(&source);
        assert_eq!(diagnostics.len(), 1, "{family}: {diagnostics:#?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.code, "E0745");
        assert!(
            diagnostic
                .message
                .to_ascii_lowercase()
                .contains(&family.to_ascii_lowercase()),
            "{}",
            diagnostic.message
        );
        assert!(!diagnostic.message.contains("Dictionary collection"));
        assert_eq!(diagnostic.fixes.len(), 1);
        assert_eq!(
            diagnostic.fixes[0].applicability,
            FixApplicability::MachineApplicable
        );
        let edit = &diagnostic.fixes[0].edits[0];
        let mut fixed = source.clone();
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
        doriac::check_source("indexed-foreach.doria", fixed)
            .unwrap_or_else(|found| panic!("{family} fix should be valid: {found:#?}"));
    }

    let commented = errors(
        "function main(): void { Set<int> $v = Set::from([1]); foreach ($v as int $i /* keep */ => int $value) {} }",
    );
    assert_eq!(commented.len(), 1);
    assert!(commented[0].fixes.is_empty());
}

#[test]
fn sequence_index_diagnostics_are_precise_and_noniterable_sources_do_not_cascade() {
    let wrong = errors(
        "function main(): void { List<int> $v = [1]; foreach ($v as string $i => int $value) {} }",
    );
    assert_eq!(wrong.len(), 1);
    assert_eq!(wrong[0].code, "E0746");
    assert_eq!(wrong[0].title, "Sequence Index Binding Must Be Int");
    assert_eq!(wrong[0].fixes[0].edits[0].replacement, "int");

    for broader in ["?int", "mixed"] {
        let diagnostics = errors(&format!(
            "function main(): void {{ List<int> $v = [1]; foreach ($v as {broader} $i => int $value) {{}} }}"
        ));
        assert_eq!(diagnostics.len(), 1, "{broader}: {diagnostics:#?}");
        assert_eq!(diagnostics[0].code, "E0746");
        assert_eq!(diagnostics[0].fixes[0].edits[0].replacement, "int");
    }

    let writable = errors(
        "function main(): void { int[] $v = [1]; foreach ($v as writable int $i => int $value) {} }",
    );
    assert_eq!(writable.len(), 1);
    assert_eq!(writable[0].code, "E0520");
    assert_eq!(writable[0].title, "Sequence Index Binding Is Readonly");
    assert_eq!(writable[0].fixes[0].edits[0].replacement, "");

    for binding in ["int $value", "$value"] {
        let queue = errors(&format!(
            "function main(): void {{ PriorityQueue<int> $v = PriorityQueue::from([1]); foreach ($v as {binding}) {{}} }}"
        ));
        assert_eq!(
            queue
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            ["E0529"],
            "{binding}: {queue:#?}"
        );
    }

    let bytes = errors(
        "function main(): void { Bytes $v = Bytes::fromArray([1]); foreach ($v as int $i => int $value) {} }",
    );
    assert_eq!(
        bytes
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0747")
            .count(),
        1
    );
    assert!(!bytes.iter().any(|diagnostic| diagnostic.code == "E0745"));
}

#[test]
fn hir_and_mir_keep_sequence_and_dictionary_roles_distinct() {
    let source = r#"
function main(): void
{
    List<string> $list = ["a", "b"];
    Dictionary<string, int> $map = ["left" => 1];
    foreach ($list as int $index => string $value) { echo "{$index}{$value}"; }
    foreach ($map as string $key => int $value) { echo "{$key}{$value}"; }
}
"#;
    let hir = doriac::lower_source("indexed-foreach.doria", source).expect("HIR lowers");
    let HirItem::Function(function) = &hir.items[0] else {
        panic!("expected function");
    };
    let loops = function
        .body
        .statements
        .iter()
        .filter_map(|statement| match statement {
            HirStmt::Foreach(foreach) => Some(foreach),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        loops[0].iteration_kind,
        SemanticIterationKind::SequenceIndex
    );
    assert_eq!(
        loops[1].iteration_kind,
        SemanticIterationKind::DictionaryKey
    );
    assert!(format!("{hir:#?}").contains("SequenceIndex"));

    let mir = doriac::lower_source_to_mir("indexed-foreach.doria", source).expect("MIR lowers");
    let plans = foreach_plans(&mir);
    assert_eq!(plans.len(), 2);
    assert_eq!(plans[0].iteration_kind, ForeachIterationKind::SequenceIndex);
    assert_eq!(plans[1].iteration_kind, ForeachIterationKind::DictionaryKey);
    assert!(format!("{}", ControlFlowPlan::Foreach(plans[0].clone())).contains("Sequence Index"));
    assert!(format!("{}", ControlFlowPlan::Foreach(plans[1].clone())).contains("Dictionary Key"));
}

#[test]
fn malformed_foreach_roles_ordinals_and_binding_sources_are_rejected() {
    let source = r#"
function main(): void
{
    List<string> $list = ["a", "b"];
    foreach ($list as int $index => string $value) { echo "{$index}{$value}"; }
}
"#;
    let valid = doriac::lower_source_to_mir("indexed-foreach.doria", source).expect("MIR lowers");
    doriac::mir_validation::validate_program(&valid).expect("valid foreach MIR");

    let mut wrong_role = valid.clone();
    first_foreach_plan_mut(&mut wrong_role).iteration_kind = ForeachIterationKind::DictionaryKey;
    doriac::mir_validation::validate_program(&wrong_role)
        .expect_err("a List cannot claim a Dictionary-key plan");

    let mut writable_ordinal = valid.clone();
    let (function, _, _) = first_foreach_plan_location(&writable_ordinal);
    let ordinal = first_foreach_plan_mut(&mut writable_ordinal).index;
    writable_ordinal.functions[function].locals[ordinal.0].writable = false;
    doriac::mir_validation::validate_program(&writable_ordinal)
        .expect_err("the synthetic traversal ordinal must be writable");

    let mut missing_initialization = valid.clone();
    let (function, _, _) = first_foreach_plan_location(&missing_initialization);
    let plan = first_foreach_plan_mut(&mut missing_initialization).clone();
    missing_initialization.functions[function].blocks[plan.setup.0]
        .statements
        .retain(|statement| {
            !matches!(statement, Statement::AssignLocal { target, .. } if *target == plan.index)
        });
    doriac::mir_validation::validate_program(&missing_initialization)
        .expect_err("the traversal ordinal needs one zero initializer");

    let mut missing_binding_source = valid.clone();
    let (function, _, _) = first_foreach_plan_location(&missing_binding_source);
    let plan = first_foreach_plan_mut(&mut missing_binding_source).clone();
    let first = plan.first_binding.expect("indexed first binding");
    missing_binding_source.functions[function].blocks[plan.body.0]
        .statements
        .retain(|statement| {
            !matches!(statement, Statement::AssignLocal { target, .. } if *target == first)
        });
    doriac::mir_validation::validate_program(&missing_binding_source)
        .expect_err("the first binding must be established before the body");

    let mut misplaced_increment = valid.clone();
    let (function, _, _) = first_foreach_plan_location(&misplaced_increment);
    let plan = first_foreach_plan_mut(&mut misplaced_increment).clone();
    let increment = misplaced_increment.functions[function].blocks[plan.update.0]
        .statements
        .pop()
        .expect("foreach update increment");
    misplaced_increment.functions[function].blocks[plan.body.0]
        .statements
        .push(increment);
    doriac::mir_validation::validate_program(&misplaced_increment)
        .expect_err("the traversal ordinal must advance in the shared update block");

    let mut wrong_value_source = valid.clone();
    let (function, _, _) = first_foreach_plan_location(&wrong_value_source);
    let plan = first_foreach_plan_mut(&mut wrong_value_source).clone();
    let value = wrong_value_source.functions[function].blocks[plan.body.0]
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            Statement::AssignLocal { target, value } if *target == plan.value_binding => {
                Some(value)
            }
            _ => None,
        })
        .expect("foreach value binding assignment");
    *value = mir::Rvalue::String(mir::StringExpression::Literal("wrong".into()));
    doriac::mir_validation::validate_program(&wrong_value_source)
        .expect_err("the value binding must read the planned collection position");
}

#[test]
fn indexed_sequences_execute_with_property_roots_and_control_flow() {
    let source = r#"
class Window
{
    internal writable List<string> $contents = ["alpha", "skip", "gamma"];

    function render(): void
    {
        foreach ($this->contents as int $line => string $content) {
            if ($line == 1) { continue; }
            echo "{$line}:{$content}\n";
        }
        echo "count={$this->contents->count}\n";
    }
}

function main(): void
{
    let $window = new Window();
    $window->render();
    string[] $letters = ["a", "b"];
    foreach ($letters as int $index => string $letter) {
        echo "{$index}{$letter}";
        if ($index == 0) { continue; }
        break;
    }
    Dictionary<string, int> $map = ["left" => 1, "right" => 2];
    foreach ($map as string $key => int $value) { echo " {$key}:{$value}"; }
}
"#;
    let output = interpret(source);
    assert_eq!(
        output.stdout,
        b"0:alpha\n2:gamma\ncount=3\n0a1b left:1 right:2"
    );
    assert_eq!(output.exit_status, 0);
}

#[test]
fn php_uses_a_compiler_owned_sequence_ordinal_and_preserves_dictionary_keys() {
    let source = r#"
function main(): void
{
    List<string> $list = ["a"];
    foreach ($list as int $index => string $value) { echo "{$index}{$value}"; }
    Dictionary<string, int> $map = ["stored" => 1];
    foreach ($map as string $key => int $value) { echo "{$key}{$value}"; }
}
"#;
    let php = doriac::compile_source_to_php("indexed-foreach.doria", source)
        .expect("PHP compatibility output");
    assert!(php.contains("$__doria_sequence_index"));
    assert!(php.contains("= $__doria_sequence_index"));
    assert!(!php.contains("foreach ($list as $index => $value)"));
    let loops = php
        .lines()
        .filter(|line| {
            line.trim_start().starts_with("foreach (")
                && (line.contains("$list") || line.contains("$map"))
        })
        .collect::<Vec<_>>();
    assert_eq!(loops.len(), 2, "{loops:#?}");
    assert!(!loops[0].contains(" => "), "{}", loops[0]);
    assert!(loops[1].contains(" => "), "{}", loops[1]);
    assert!(loops[1].contains("$key"), "{}", loops[1]);
}

#[test]
fn canonical_scalar_display_materializes_ordinary_strings() {
    let source = r#"
function acceptString(string $value): string { return $value; }
function describe(int $line, float $ratio): string { return "{$line}|{$ratio}"; }
function main(): void
{
    int $line = -42;
    uint8 $byte = 255;
    float32 $small = 1.5;
    float $ratio = -0.0;
    bool $enabled = true;
    string $lineText = "{$line}";
    string $combined = acceptString("{$byte}|{$small}|{$enabled}");
    List<string> $items = [$lineText, "" . $ratio, sprintf("%s", $small)];
    string[] $array = [sprintf("%d", $line), sprintf("%.2f", 1.25)];
    echo $combined . "\n" . describe(42, 1.5) . "\n";
    foreach ($items as int $index => string $item) { echo "L{$index}:{$item}\n"; }
    foreach ($array as int $index => string $item) { echo "A{$index}:{$item}\n"; }
}
"#;
    let output = interpret(source);
    assert_eq!(
        output.stdout,
        b"255|1.5|true\n42|1.5\nL0:-42\nL1:-0\nL2:1.5\nA0:-42\nA1:1.25\n"
    );
    for invalid in [
        "function main(): void { string $value = 42; }",
        "function takeString(string $value): void {} function main(): void { takeString(42); }",
        "function main(): void { int $value = 42; let $text = $value->toString(); }",
    ] {
        assert!(
            !errors(invalid).is_empty(),
            "accepted invalid source: {invalid}"
        );
    }
}
