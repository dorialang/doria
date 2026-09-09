use doriac::semantics::CatchCoverage;
use doriac::{
    mir,
    semantics::CallableTarget,
    types::{InterfaceType, ResolvedType},
};

const ERROR_CARRIER_SOURCE: &str =
    include_str!("../../../examples/native/main_stage35_interface_error_carrier.doria");

#[test]
fn comparable_collections_preserve_sorted_order_min_heap_and_sources() {
    let source =
        include_str!("../../../examples/native/main_stage35_interface_ordered_collections.doria");
    let program = doriac::lower_source_to_mir("ordered-collections.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        include_bytes!(
            "fixtures/native_io/main_stage35_interface_ordered_collections/expected_stdout"
        )
    );
}

#[test]
fn hash_sets_compare_collisions_and_preserve_owned_entries() {
    let source = include_str!("../../../examples/native/main_stage35_interface_hash_set.doria");
    let program = doriac::lower_source_to_mir("core-hash-set.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        include_bytes!("fixtures/native_io/main_stage35_interface_hash_set/expected_stdout")
    );
}

#[test]
fn preserving_hash_set_clones_selected_representatives_and_keeps_source() {
    let source = include_str!("../../../examples/native/main_stage35_interface_set_algebra.doria");
    let program = doriac::lower_source_to_mir("core-hash-from.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        include_bytes!("fixtures/native_io/main_stage35_interface_set_algebra/expected_stdout")
    );
}

#[test]
fn hash_dictionaries_use_full_class_keys_and_nullable_values() {
    let source =
        include_str!("../../../examples/native/main_stage35_interface_hash_dictionary.doria");
    let program = doriac::lower_source_to_mir("core-hash-dictionary.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        include_bytes!("fixtures/native_io/main_stage35_interface_hash_dictionary/expected_stdout")
    );
}

#[test]
fn builtin_iterable_erasure_keeps_the_collection_and_borrows_its_elements() {
    let source = r#"
function inspect(Iterable<string> $source): void {
    foreach ($source as string $value) { echo $value; }
}

function main(): void {
    List<string> $words = ["a", "b"];
    inspect($words);
    echo $words[0];
    Iterable<string> $owned = ["c", "d"];
    inspect($owned);
}
"#;
    let program = doriac::lower_source_to_mir("builtin-iterable.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        b"abacd"
    );
}

#[test]
fn builtin_iteration_manifest_fixture_covers_all_public_collection_families() {
    let source =
        include_str!("../../../examples/native/main_stage35_interface_builtin_iteration.doria");
    let program = doriac::lower_source_to_mir("builtin-iterable-matrix.doria", source).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        include_bytes!(
            "fixtures/native_io/main_stage35_interface_builtin_iteration/expected_stdout"
        )
    );
}

#[test]
fn retained_iterator_mir_keeps_source_ownership_and_validates_lifetimes() {
    let source = r#"
class Cursor implements Iterator<int> {
    writable int $position = 0;
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return $this->position < $this->source->count; }
    function getCurrent(): int { return $this->source[$this->position]; }
    writable function advance(): void { $this->position++; }
}

function forward(List<int> $source): Cursor { return new Cursor($source); }
function main(): void {
    let $source = [3, 7];
    {
        let writable $cursor = forward($source);
        echo $cursor->getCurrent();
        $cursor->advance();
        echo $cursor->getCurrent();
    }
    echo $source[0];
}
"#;
    let program = doriac::lower_source_to_mir("iterator-loans.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        b"373"
    );
    let mut invalid = program.clone();
    let function = invalid
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .unwrap();
    let (block, index, source) = function
        .blocks
        .iter()
        .enumerate()
        .find_map(|(block, definition)| {
            definition.statements.iter().enumerate().find_map(
                |(index, statement)| match statement {
                    mir::Statement::AssignLocal {
                        value: mir::Rvalue::Class(mir::ClassExpression::Call { args, .. }),
                        ..
                    } => Some((block, index, args[0].direct_place_local().unwrap())),
                    _ => None,
                },
            )
        })
        .unwrap();
    let collection_type = function.locals[source.0].ty;
    let mir::Type::Collection(collection) = collection_type else {
        panic!("source must be a collection")
    };
    function.blocks[block].statements.insert(
        index + 1,
        mir::Statement::DropCollection {
            local: source,
            collection,
        },
    );
    assert!(doriac::mir_validation::validate_program(&invalid).is_err());
}

#[test]
fn retained_iterator_interface_entries_preserve_sources_across_owned_forwarding() {
    let source = r#"
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
}
class Shelf implements Iterable<int> {
    function __construct(take List<int> $source) {}
    function iterator(): Iterator<int> { return new Cursor($this->source); }
}
function pass(take Iterator<int> $cursor): Iterator<int> { return $cursor; }
function obtain(Iterable<int> $shelf): Iterator<int> { return pass($shelf->iterator()); }
function main(): void {
    Iterable<int> $shelf = new Shelf([13]);
    { let writable $cursor = obtain($shelf); echo $cursor->getCurrent(); $cursor->advance(); }
    { let $cursor = obtain($shelf); echo $cursor->getCurrent(); }
}
"#;
    let program = doriac::lower_source_to_mir("erased-iterator-loans.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        b"1313"
    );
}

#[test]
fn preserving_constructors_clone_directly_into_the_destination() {
    let program = doriac::lower_source_to_mir(
        "preserving-from.doria",
        include_str!("../../../examples/native/main_stage35_interface_preserving_from.doria"),
    )
    .unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        output.stdout,
        include_bytes!("fixtures/native_io/main_stage35_interface_preserving_from/expected_stdout")
    );
}

#[test]
fn primitive_contracts_remain_unboxed_and_preserve_exact_results() {
    let source =
        include_str!("../../../examples/native/main_stage35_interface_primitive_contracts.doria");
    let program = doriac::lower_source_to_mir("primitive-contracts.doria", source).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        include_bytes!(
            "fixtures/native_io/main_stage35_interface_primitive_contracts/expected_stdout"
        )
    );
    assert!(program.interface_vtables.iter().all(|table| {
        program.interface_types[table.interface.0]
            .methods
            .iter()
            .all(|method| {
                doriac::compiler_known_contracts::CoreValueOperation::from_requirement(
                    method.requirement,
                )
                .is_none()
            })
    }));
    for function in &program.functions {
        if ["same", "order", "hashValue"]
            .iter()
            .any(|name| function.name.starts_with(name))
        {
            assert!(function.params.iter().all(|local| matches!(
                function.locals[local.0].ty,
                mir::Type::Scalar(_) | mir::Type::String
            )));
        }
    }
}

#[test]
fn discarded_call_results_use_the_normal_temporary_cleanup_path() {
    let source = r#"
class Item {
    function __construct(int $id) {}
    function __destruct(): void {
        try { echo "drop {$this->id}\n"; }
        catch (Doria\Std\Io\IoError $error) {}
    }
}
class Factory {
    function make(int $id): Item { return new Item($id); }
    static function create(int $id): Item { return new Item($id); }
    function scalar(): int { return 42; }
}
function make(int $id): Item { return new Item($id); }
function optional(bool $present): ?Factory {
    if ($present) { return new Factory(); }
    return null;
}
function main(): void {
    make(1);
    let $factory = new Factory();
    $factory->make(2);
    Factory::create(3);
    ?Factory $present = optional(true);
    $present?->make(4);
    ?Factory $absent = optional(false);
    $absent?->make(5);
    $factory->scalar();
    $present?->scalar();
    echo "done\n";
}
"#;
    let program = doriac::lower_source_to_mir("discarded-results.doria", source).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        b"drop 1\ndrop 2\ndrop 3\ndrop 4\ndone\n"
    );
}

#[test]
fn cursor_captures_preserve_retained_sources_until_closure_cleanup() {
    let source = r#"
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
}
function main(): void {
    writable List<int> $source = [7];
    {
        let $cursor = new Cursor($source);
        let $read = fn() with (take $cursor) => $cursor->getCurrent();
        echo $read();
    }
    $source->add(8);
    echo $source->count;
}
"#;
    let program = doriac::lower_source_to_mir("cursor-capture.doria", source).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        b"72"
    );
    let mut invalid = program.clone();
    let function = invalid
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .unwrap();
    let (block, index, source) = function
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .enumerate()
                .find_map(|(index, statement)| match statement {
                    mir::Statement::AssignLocal {
                        value: mir::Rvalue::Function(mir::FunctionExpression::Create { .. }),
                        ..
                    } => {
                        let source = function
                            .locals
                            .iter()
                            .find(|local| local.name == "source")?
                            .id;
                        Some((block.id, index, source))
                    }
                    _ => None,
                })
        })
        .unwrap();
    let mir::Type::Collection(collection) = function.locals[source.0].ty else {
        panic!("collection source")
    };
    function.blocks[block.0].statements.insert(
        index + 1,
        mir::Statement::DropCollection {
            local: source,
            collection,
        },
    );
    assert!(doriac::mir_validation::validate_program(&invalid).is_err());
}

#[test]
fn current_element_mir_rejects_a_use_after_cursor_mutation() {
    let source = r#"
class Book { function __construct(int $value) {} }
class Cursor implements Iterator<Book> {
    function __construct(borrow List<Book> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): Book { return $this->source[0]; }
    writable function advance(): void {}
}
function main(): void {
    let $source = [new Book(7)];
    let writable $cursor = new Cursor($source);
    let $book = $cursor->getCurrent();
    echo "{$book->value}";
}
"#;
    let program = doriac::lower_source_to_mir("current-borrow.doria", source).unwrap();
    let mut invalid = program.clone();
    let advance = invalid
        .functions
        .iter()
        .find(|function| {
            function
                .method
                .as_ref()
                .is_some_and(|method| method.name == "advance")
        })
        .map(|function| function.id)
        .unwrap();
    let function = invalid
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .unwrap();
    let cursor = function
        .locals
        .iter()
        .find(|local| local.name == "cursor")
        .unwrap();
    let mir::Type::Class(class) = cursor.ty else {
        panic!("class cursor")
    };
    let cursor = cursor.id;
    let (block, index) = function
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .enumerate()
                .find_map(|(index, statement)| {
                    matches!(statement, mir::Statement::AssignLocal { target, .. }
                        if function.locals[target.0].name == "book")
                    .then_some((block.id, index + 1))
                })
        })
        .unwrap();
    function.blocks[block.0].statements.insert(
        index,
        mir::Statement::CallVoid {
            function: advance,
            args: vec![mir::Rvalue::Class(mir::ClassExpression::Local {
                class,
                local: cursor,
                transfer: false,
            })],
            span: doriac::source::Span::new(0, 0),
        },
    );
    assert!(doriac::mir_validation::validate_program(&invalid).is_err());
}

#[test]
fn public_foreach_acquires_once_advances_on_continue_and_preserves_move_elements() {
    let source =
        include_str!("../../../examples/native/main_stage35_interface_public_iteration.doria");
    let program = doriac::lower_source_to_mir("public-iteration.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        include_bytes!(
            "fixtures/native_io/main_stage35_interface_public_iteration/expected_stdout"
        )
    );
}

#[test]
fn owning_iterator_return_preserves_source_and_reverse_element_cleanup() {
    let source =
        include_str!("../../../examples/native/main_stage35_interface_owning_iteration.doria");
    let program = doriac::lower_source_to_mir("owning-iteration.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        include_bytes!(
            "fixtures/native_io/main_stage35_interface_owning_iteration/expected_stdout"
        )
    );
}

#[test]
fn public_iteration_preserves_redeclared_requirement_identity() {
    let source = r#"
interface Values extends Iterable<int> {
    function iterator(): Iterator<int>;
}
interface Position extends Iterator<int> {
    function getCurrent(): int;
}
class Cursor implements Position {
    writable bool $available = true;
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return $this->available; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void { $this->available = false; }
}
class Source implements Values {
    List<int> $items = [42];
    function iterator(): Iterator<int> {
        Position $cursor = new Cursor($this->items);
        return $cursor;
    }
}
function main(): void {
    Values $source = new Source();
    foreach ($source as int $value) { echo $value; }
}
"#;
    let program = doriac::lower_source_to_mir("redeclared-iteration.doria", source).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        b"42"
    );
}

#[test]
fn user_interface_forwarding_preserves_iterator_sources() {
    let declarations = r#"
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
    function fork(): Iterator<int> { return forward($this); }
}
function forward(Cursor $cursor): Iterator<int> {
    return new Cursor($cursor->source);
}
interface Factory {
    function make(List<int> $source): Iterator<int>;
}
class FactoryImpl implements Factory {
    function make(List<int> $source): Iterator<int> { return new Cursor($source); }
}
"#;
    let source = format!(
        r#"{declarations}
function main(): void {{
    writable List<int> $source = [7];
    Factory $factory = new FactoryImpl();
    {{ let $cursor = $factory->make($source); echo $cursor->getCurrent(); }}
    $source->add(8);
    {{ let $cursor = new Cursor($source); let $fork = $cursor->fork(); echo $fork->getCurrent(); }}
}}
"#
    );
    let program = doriac::lower_source_to_mir("iterator-forwarding.doria", &source).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        b"77"
    );
    let escaping = format!(
        r#"{declarations}
function leak(Factory $factory): Iterator<int> {{
    List<int> $source = [7];
    return $factory->make($source);
}}
function main(): void {{}}
"#
    );
    let errors = doriac::lower_source_to_mir("iterator-escape.doria", &escaping).unwrap_err();
    assert!(
        errors.iter().any(|diagnostic| diagnostic.code == "E0762"),
        "{errors:?}"
    );
}

#[test]
fn virtual_overrides_preserve_retained_input_dependencies() {
    let declarations = r#"
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
}
open class Factory {
    List<int> $items = [1];
    open function make(List<int> $source): Iterator<int> {
        return new Cursor($this->items);
    }
}
class BorrowingFactory extends Factory {
    override function make(List<int> $source): Iterator<int> {
        return new Cursor($source);
    }
}
"#;
    let source = format!(
        r#"{declarations}
function main(): void {{
    Factory $factory = new BorrowingFactory();
    writable List<int> $items = [7];
    {{ let $cursor = $factory->make($items); echo $cursor->getCurrent(); }}
    $items->add(8);
    echo $items->count;
}}
"#
    );
    let program = doriac::lower_source_to_mir("virtual-iterator.doria", &source).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        b"72"
    );
    let escaping = format!(
        r#"{declarations}
function leak(Factory $factory): Iterator<int> {{
    List<int> $items = [7];
    return $factory->make($items);
}}
function main(): void {{}}
"#
    );
    let errors =
        doriac::lower_source_to_mir("virtual-iterator-escape.doria", &escaping).unwrap_err();
    assert!(
        errors.iter().any(|error| error.code == "E0762"),
        "{errors:?}"
    );
}

#[test]
fn filter_failures_drop_partial_outputs_and_preserve_sources() {
    let program = doriac::lower_source_to_mir(
        "clone-failure.doria",
        include_str!("../../../examples/native/main_stage35_interface_clone_failure.doria"),
    )
    .unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        output.stdout,
        include_bytes!("fixtures/native_io/main_stage35_interface_clone_failure/expected_stdout")
    );
}

#[test]
fn public_iteration_mir_rejects_changed_protocol_receivers_and_control_flow() {
    let program = doriac::lower_source_to_mir(
        "iteration-proof.doria",
        include_str!("../../../examples/native/main_stage35_interface_public_iteration.doria"),
    )
    .unwrap();
    for mutation in 0..7 {
        let mut invalid = program.clone();
        let function = invalid
            .functions
            .iter_mut()
            .find(|function| function.name == "main")
            .unwrap();
        let (block, statement, mut plan) = function
            .blocks
            .iter()
            .find_map(|block| {
                block
                    .statements
                    .iter()
                    .enumerate()
                    .find_map(|(index, statement)| match statement {
                        mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::PublicForeach(
                            plan,
                        )) => Some((block.id, index, plan.clone())),
                        _ => None,
                    })
            })
            .unwrap();
        match mutation {
            0 => plan.calls[3].receiver = plan.source,
            1 => plan.calls[2].slot = usize::MAX,
            2 => function.locals[plan.value_binding.0].owned = true,
            3 => {
                function.blocks[plan.calls[1].success.0].terminator =
                    mir::Terminator::Jump(plan.body)
            }
            4 => {
                function.blocks[plan.calls[3].success.0].terminator =
                    mir::Terminator::Jump(plan.calls[0].call)
            }
            5 => plan.calls[0].result = Some(plan.source),
            6 => {
                plan.calls[2].operation =
                    doriac::compiler_known_contracts::IterationOperation::Advance
            }
            _ => unreachable!(),
        }
        function.blocks[block.0].statements[statement] =
            mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::PublicForeach(plan));
        assert!(
            doriac::mir_validation::validate_program(&invalid).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn public_iteration_preserves_temporary_sources_nullable_elements_and_checked_exits() {
    let source =
        include_str!("../../../examples/native/main_stage35_interface_iteration_flow.doria");
    let program = doriac::lower_source_to_mir("iteration-flow.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        include_bytes!("fixtures/native_io/main_stage35_interface_iteration_flow/expected_stdout")
    );
}

#[test]
fn generated_core_calls_require_nominal_selection_and_owned_duplication() {
    let program = doriac::lower_source_to_mir(
        "core-proof.doria",
        include_str!("../../../examples/native/main_stage35_interface_fill.doria"),
    )
    .unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    for mutation in 0..5 {
        let mut invalid = program.clone();
        let (function, block, statement) =
            invalid
                .functions
                .iter()
                .enumerate()
                .find_map(|(function, definition)| {
                    definition
                        .blocks
                        .iter()
                        .enumerate()
                        .find_map(|(block, definition)| {
                            definition.statements.iter().enumerate().find_map(
                                |(statement, value)| {
                                    matches!(
                                        value,
                                        mir::Statement::ControlFlowPlan(
                                            mir::ControlFlowPlan::CoreValue(_)
                                        )
                                    )
                                    .then_some((function, block, statement))
                                },
                            )
                        })
                })
                .unwrap();
        let function = &mut invalid.functions[function];
        let mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::CoreValue(plan)) =
            &mut function.blocks[block].statements[statement]
        else {
            unreachable!()
        };
        match mutation {
            0 => plan.operation = doriac::compiler_known_contracts::CoreValueOperation::Hash,
            1 => plan.slot = usize::MAX,
            2 => function.locals[plan.receiver.0].owned = true,
            3 => function.locals[plan.result.0].owned = false,
            4 => plan.success = mir::BlockId(usize::MAX),
            _ => unreachable!(),
        }
        assert!(
            doriac::mir_validation::validate_program(&invalid).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn collection_construction_proofs_prevent_partial_escape_and_duplicate_owners() {
    let program = doriac::lower_source_to_mir(
        "construction-proof.doria",
        include_str!("../../../examples/native/main_stage35_interface_fill.doria"),
    )
    .unwrap();
    for mutation in 0..8 {
        let mut invalid = program.clone();
        let function = invalid
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    block.statements.iter().any(|statement| {
                        matches!(
                            statement,
                            mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::CollectionBuild(
                                _
                            ))
                        )
                    })
                })
            })
            .unwrap();
        let plan = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|statement| match statement {
                mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::CollectionBuild(plan)) => {
                    Some(plan.clone())
                }
                _ => None,
            })
            .unwrap();
        let finish = function.blocks[plan.exit.0].statements[0].clone();
        let mut owner = function.locals[plan.output.0].clone();
        owner.id = mir::LocalId(function.locals.len());
        let new_owner = owner.id;
        function.locals.push(owner);
        match mutation {
            0 => {
                let mut capacity = function.blocks[plan.setup.0].statements.iter().find(|statement|
                    matches!(statement, mir::Statement::AssignLocal { target, .. } if *target == plan.output)).unwrap().clone();
                if let mir::Statement::AssignLocal { target, .. } = &mut capacity {
                    *target = new_owner;
                }
                function.blocks[plan.setup.0].statements.push(capacity);
            }
            1 => function.blocks[plan.body.0].statements.insert(0, finish),
            2 => {
                let mir::Statement::AssignLocal {
                    value:
                        mir::Rvalue::Collection(mir::CollectionExpression::FinishConstruction {
                            collection,
                            ..
                        }),
                    ..
                } = finish
                else {
                    unreachable!()
                };
                function.blocks[plan.body.0].statements.insert(
                    0,
                    mir::Statement::AssignLocal {
                        target: new_owner,
                        value: mir::Rvalue::Collection(mir::CollectionExpression::Local {
                            collection,
                            local: plan.output,
                            transfer: true,
                            assume_non_null: false,
                        }),
                    },
                );
            }
            3 => function.blocks[plan.body.0].statements.insert(
                0,
                mir::Statement::EchoString(mir::StringExpression::Display(
                    mir::ValueExpression::Integer(mir::IntegerExpression::Use {
                        ty: doriac::numeric::IntegerType::Int64,
                        operand: mir::Operand::CollectionLength(plan.output),
                    }),
                )),
            ),
            4 => function.blocks[plan.setup.0].terminator = mir::Terminator::Jump(plan.body),
            5 => {
                let bound_write = function.blocks[plan.setup.0].statements.iter().find(|statement|
                    matches!(statement, mir::Statement::AssignLocal { target, .. } if *target == plan.count)).unwrap().clone();
                function.blocks[plan.body.0]
                    .statements
                    .insert(0, bound_write);
            }
            6 => function.blocks[plan.exit.0].statements.insert(1, finish),
            7 => {
                let mir::Type::Collection(collection) = function.locals[plan.output.0].ty else {
                    unreachable!()
                };
                function.blocks[plan.body.0].statements.insert(
                    0,
                    mir::Statement::DropCollection {
                        local: plan.output,
                        collection,
                    },
                );
            }
            _ => unreachable!(),
        }
        let error = doriac::mir_validation::validate_program(&invalid)
            .expect_err("malformed construction accepted");
        assert!(
            error.message.contains("collection") || error.message.contains("sequence fill"),
            "mutation {mutation}: {error:?}"
        );
    }
}

#[test]
fn nullable_duplication_preserves_absence_and_clones_present_values() {
    let program = doriac::lower_source_to_mir(
        "nullable-duplication.doria",
        include_str!("../../../examples/native/main_stage35_interface_nullable_duplication.doria"),
    )
    .unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        output.stdout,
        include_bytes!(
            "fixtures/native_io/main_stage35_interface_nullable_duplication/expected_stdout"
        )
    );
}

#[test]
fn preserving_filter_clones_only_selected_elements() {
    let program = doriac::lower_source_to_mir(
        "filter.doria",
        include_str!("../../../examples/native/main_stage35_interface_filter.doria"),
    )
    .unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        output.stdout,
        include_bytes!("fixtures/native_io/main_stage35_interface_filter/expected_stdout")
    );
}

#[test]
fn collection_search_uses_checked_equality_without_consuming_the_source() {
    let program = doriac::lower_source_to_mir(
        "membership.doria",
        include_str!("../../../examples/native/main_stage35_interface_membership.doria"),
    )
    .unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        output.stdout,
        include_bytes!("fixtures/native_io/main_stage35_interface_membership/expected_stdout")
    );
}

#[test]
fn sequence_fill_clones_each_destination_and_preserves_the_source() {
    let program = doriac::lower_source_to_mir(
        "fill.doria",
        include_str!("../../../examples/native/main_stage35_interface_fill.doria"),
    )
    .unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        include_str!("fixtures/native_io/main_stage35_interface_fill/expected_stdout")
    );
}

#[test]
fn core_equality_uses_the_static_contract_and_presence_proofs() {
    let program = doriac::lower_source_to_mir(
        "equality.doria",
        include_str!("../../../examples/native/main_stage35_interface_equality.doria"),
    )
    .unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap(),
        "make 1\nmake 1\nequals 1:1\ntrue\nequals 1:2\nfalse\ntrue\ntrue false\nfalse true\nfalse true\nequals 1:2\nequals 1:2\nfalse true\ntrue false\n");
}

#[test]
fn core_contract_calls_preserve_ordering_hash_width_and_owned_clone() {
    let program = doriac::lower_source_to_mir(
        "core-calls.doria",
        include_str!("../../../examples/native/main_stage35_interface_core_calls.doria"),
    )
    .unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        output.stdout,
        include_bytes!("fixtures/native_io/main_stage35_interface_core_calls/expected_stdout")
    );
}

#[test]
fn uint64_transport_preserves_high_bits_and_unsigned_ordering() {
    let program = doriac::lower_source_to_mir(
        "uint64.doria",
        include_str!("../../../examples/native/main_stage35_interface_uint64.doria"),
    )
    .unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        output.stdout,
        include_bytes!("fixtures/native_io/main_stage35_interface_uint64/expected_stdout")
    );
}

#[test]
fn interface_erasure_keeps_headerless_layout_and_constrained_calls_direct() {
    let program = doriac::lower_source_to_mir(
        "interface-structure.doria",
        include_str!("../../../examples/native/main_stage35_interface_structure.doria"),
    )
    .unwrap();
    let number = program
        .classes
        .iter()
        .find(|class| class.name == "Number")
        .unwrap();
    assert_eq!(number.layout.size, 8);
    assert_eq!(number.layout.properties.len(), 1);
    assert_eq!(number.layout.properties[0].offset, 0);
    assert_eq!(
        program
            .interface_vtables
            .iter()
            .filter(|table| table.implementing_type == mir::ImplementingType::Class(number.id))
            .count(),
        1
    );
    assert!(program.closure_descriptors.is_empty());
    for name in ["concrete", "constrained", "erased"] {
        let function = program
            .functions
            .iter()
            .find(|function| function.name.starts_with(name))
            .unwrap();
        let indirect = function.blocks.iter().any(|block| {
            matches!(
                block.terminator,
                mir::Terminator::IndirectCall { .. } | mir::Terminator::CheckedIndirectCall { .. }
            )
        });
        assert_eq!(indirect, name == "erased", "{function}");
    }
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, b"7\n");
}

#[test]
fn collection_core_failures_preserve_existing_owners_and_clean_partial_results() {
    let source =
        include_str!("../../../examples/native/main_stage35_interface_collection_failures.doria");
    let program = doriac::lower_source_to_mir("collection-failures.doria", source).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        include_bytes!(
            "fixtures/native_io/main_stage35_interface_collection_failures/expected_stdout"
        )
    );
}

#[test]
fn local_collection_cursors_use_fixed_frame_storage_without_changing_returned_cursors() {
    let source = include_str!("../../../examples/native/main_stage35_interface_stack_cursor.doria");
    let program = doriac::lower_source_to_mir("stack-cursor.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    let main = program
        .functions
        .iter()
        .find(|function| function.name.starts_with("inspect"))
        .unwrap();
    let returned = program
        .functions
        .iter()
        .find(|function| function.name.starts_with("cursor"))
        .unwrap();
    let plans = doriac::mir_validation::stack_collection_iterators(&program, main);
    assert_eq!(plans.len(), 1, "{main}");
    assert!(doriac::mir_validation::stack_collection_iterators(&program, returned).is_empty());
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        include_bytes!("fixtures/native_io/main_stage35_interface_stack_cursor/expected_stdout")
    );

    let mut repeated = main.clone();
    let last = repeated
        .blocks
        .iter_mut()
        .find(|block| matches!(block.terminator, mir::Terminator::ReturnVoid))
        .unwrap();
    last.terminator = mir::Terminator::Jump(repeated.entry_block);
    assert!(doriac::mir_validation::stack_collection_iterators(&program, &repeated).is_empty());

    let mut escaping = main.clone();
    let last = escaping
        .blocks
        .iter_mut()
        .find(|block| matches!(block.terminator, mir::Terminator::ReturnVoid))
        .unwrap();
    let mir::Type::Interface(interface) = escaping.locals[plans[0].cursor.0].ty else {
        unreachable!()
    };
    last.terminator = mir::Terminator::Return(mir::Rvalue::interface(
        interface,
        mir::InterfaceValue::Local {
            local: plans[0].cursor,
            transfer: true,
        },
    ));
    assert!(doriac::mir_validation::stack_collection_iterators(&program, &escaping).is_empty());
}

#[test]
fn durable_interface_fixtures_preserve_php_execution_and_cleanup() {
    use std::io::Write;
    use std::process::Command;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for path in include_str!("fixtures/native_parity_examples.txt")
        .lines()
        .filter(|path| {
            path.starts_with("examples/native/main_stage35_interface_")
                || path.starts_with("examples/native/main_stage35_trait_")
        })
    {
        let source = std::fs::read_to_string(root.join(path)).unwrap();
        let hir = doriac::lower_source(path, source).unwrap();
        let mir = doriac::mir_lowering::lower_program(&hir).unwrap();
        let php = doriac::codegen_php::generate(&hir, Some(&mir)).unwrap();
        let stem = std::path::Path::new(path).file_stem().unwrap();
        let fixture = root
            .join("crates/doriac/tests/fixtures/native_io")
            .join(stem);
        let file = std::env::temp_dir().join(format!(
            "doria-interface-{}-{}.php",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::File::create_new(&file)
            .unwrap()
            .write_all(php.as_bytes())
            .unwrap();
        let lint = Command::new("php").arg("-l").arg(&file).output();
        let run = Command::new("php").arg(&file).output();
        std::fs::remove_file(&file).unwrap();
        for (lint, output) in [(true, lint), (false, run)] {
            let output = output.expect("PHP is required for interface compatibility parity");
            assert!(output.status.success(), "{path}: {output:?}");
            assert!(output.stderr.is_empty(), "{path}: {output:?}");
            if !lint {
                assert_eq!(
                    output.stdout,
                    std::fs::read(fixture.join("expected_stdout")).unwrap(),
                    "{path}"
                );
            }
        }
    }
}

#[test]
fn php_host_collection_and_shutdown_preserve_strong_cycles() {
    use std::process::Command;
    let source =
        include_str!("../../../examples/native/main_stage35_interface_strong_cycles.doria");
    let hir = doriac::lower_source("shared-cycles.doria", source).unwrap();
    let mir = doriac::mir_lowering::lower_program(&hir).unwrap();
    let expected = doriac::mir_interpreter::interpret(&mir).unwrap().stdout;
    let php = doriac::codegen_php::generate(&hir, Some(&mir)).unwrap();
    for enabled in [true, false] {
        let script = format!(
            "{}\n{}\n__DoriaFunction_6d61696e(); echo \"before host collection\\n\"; gc_collect_cycles(); echo \"after host collection\\n\";
            $control = new __DoriaSharedControl(new stdClass());
            $observed = WeakReference::create($control);
            $owner = new __DoriaSharedHandle($control, 0);
            unset($control);
            __doria_drop_value($owner);
            gc_collect_cycles();
            if ($observed->get() !== null) {{ throw new LogicException(\"released control block retained\"); }}",
            if enabled { "gc_enable();" } else { "gc_disable();" },
            php.strip_prefix("<?php").unwrap(),
        );
        let output = Command::new("php").args(["-r", &script]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(
            output.stdout,
            [
                expected.as_slice(),
                b"before host collection\nafter host collection\n"
            ]
            .concat(),
            "host GC enabled: {enabled}"
        );
    }
}

fn interface_dispatch_mir() -> mir::Program {
    let mut program = doriac::lower_source_to_mir(
        "interface-entry.doria",
        r#"
interface Marker {}
class Box implements Marker { function read(): int { return 42; } }
function main(): int { return 42; }
"#,
    )
    .unwrap();
    let interface = program
        .interface_types
        .iter()
        .find(|ty| ty.name == "Marker")
        .unwrap()
        .id;
    let class = program
        .classes
        .iter()
        .find(|class| class.name == "Box")
        .unwrap()
        .id;
    let constructor = program.classes[class.0].constructor;
    let implementation = program
        .functions
        .iter()
        .find(|function| {
            function
                .method
                .as_ref()
                .is_some_and(|method| method.class == class && method.name == "read")
        })
        .unwrap()
        .id;
    let vtable = program
        .interface_vtable(mir::ImplementingType::Class(class), interface)
        .unwrap();
    let signature = mir::FunctionTypeId(program.function_types.len());
    let integer = mir::Type::Scalar(mir::ScalarType::Integer(
        doriac::numeric::IntegerType::Int64,
    ));
    program.function_types.push(mir::FunctionType {
        id: signature,
        invocation_mode: mir::FunctionInvocationMode::Readonly,
        parameters: vec![mir::FunctionParameter {
            mode: mir::FunctionParameterMode::Readonly,
            ty: mir::Type::Interface(interface),
        }],
        return_type: mir::ReturnType::Value(integer),
        return_borrow: None,
        checked_effects: vec![],
        ambient_checked_effects: vec![],
        test_assertion_checked_effects: vec![],
    });
    program.interface_types[interface.0]
        .methods
        .push(mir::InterfaceMethod {
            requirement: Default::default(),
            name: "read".into(),
            arguments: vec![],
            signature,
            writable_receiver: false,
            exact_dynamic_return: false,
        });
    let receiver = mir::Local {
        id: mir::LocalId(0),
        name: "receiver".into(),
        ty: mir::Type::Interface(interface),
        writable: false,
        owned: false,
        synthetic: true,
    };
    let mut entry = program.functions[program.entry.0].clone();
    entry.id = mir::FunctionId(program.functions.len());
    entry.name = "interface-entry".into();
    entry.params = vec![receiver.id];
    entry.parameter_modes = vec![mir::FunctionParameterMode::Readonly];
    entry.locals = vec![receiver.clone()];
    entry.blocks[0].terminator = mir::Terminator::Return(mir::Rvalue::Value(
        mir::ValueExpression::Integer(mir::IntegerExpression::Call {
            ty: doriac::numeric::IntegerType::Int64,
            function: implementation,
            args: vec![mir::Rvalue::Class(
                mir::ClassExpression::InterfaceReceiver {
                    class,
                    receiver: receiver.id,
                    vtable,
                },
            )],
        }),
    ));
    program.interface_vtables[vtable.0].methods.push(entry.id);
    program.functions.push(entry);

    let main = &mut program.functions[program.entry.0];
    main.locals = vec![
        mir::Local {
            owned: true,
            ..receiver
        },
        mir::Local {
            id: mir::LocalId(1),
            name: "result".into(),
            ty: integer,
            writable: false,
            owned: false,
            synthetic: true,
        },
    ];
    main.blocks = vec![
        mir::BasicBlock {
            id: mir::BlockId(0),
            statements: vec![mir::Statement::AssignLocal {
                target: mir::LocalId(0),
                value: mir::Rvalue::Interface(mir::InterfaceExpression {
                    interface,
                    value: mir::InterfaceValue::FromClass {
                        object: Box::new(mir::ClassExpression::New {
                            class,
                            concrete_class: class,
                            properties: vec![],
                            constructor,
                            args: vec![],
                        }),
                        vtable,
                    },
                }),
            }],
            terminator: mir::Terminator::IndirectCall {
                callee: mir::IndirectCallee::InterfaceMethod {
                    receiver: mir::LocalId(0),
                    interface,
                    slot: 0,
                },
                function_type: signature,
                invocation_mode: mir::FunctionInvocationMode::Readonly,
                args: vec![mir::Rvalue::Interface(mir::InterfaceExpression {
                    interface,
                    value: mir::InterfaceValue::Local {
                        local: mir::LocalId(0),
                        transfer: false,
                    },
                })],
                result: Some(mir::LocalId(1)),
                continuation: mir::BlockId(1),
                span: Default::default(),
            },
        },
        mir::BasicBlock {
            id: mir::BlockId(1),
            statements: vec![mir::Statement::DropError {
                local: mir::LocalId(0),
            }],
            terminator: mir::Terminator::Return(mir::Rvalue::Value(mir::ValueExpression::Integer(
                mir::IntegerExpression::Use {
                    ty: doriac::numeric::IntegerType::Int64,
                    operand: mir::Operand::Local(mir::LocalId(1)),
                },
            ))),
        },
    ];
    main.entry_block = mir::BlockId(0);
    program
}

#[test]
fn interface_dispatch_uses_an_ordinary_entry_without_a_closure_environment() {
    let program = interface_dispatch_mir();
    doriac::mir_validation::validate_program(&program).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.exit_status, 42);
    assert!(output.stdout.is_empty());
    assert!(program.closure_descriptors.is_empty());
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .unwrap()
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .unwrap()
        .is_empty());
}

#[test]
fn interface_dispatch_rejects_slot_receiver_and_entry_abi_mismatches() {
    let program = interface_dispatch_mir();
    let mut bad_slot = program.clone();
    let mir::Terminator::IndirectCall {
        callee: mir::IndirectCallee::InterfaceMethod { slot, .. },
        ..
    } = &mut bad_slot.functions[program.entry.0].blocks[0].terminator
    else {
        unreachable!()
    };
    *slot = 1;
    assert!(doriac::mir_validation::validate_program(&bad_slot).is_err());
    let mut bad_receiver = program.clone();
    let mir::Terminator::IndirectCall { args, .. } =
        &mut bad_receiver.functions[program.entry.0].blocks[0].terminator
    else {
        unreachable!()
    };
    let mir::Rvalue::Interface(mir::InterfaceExpression {
        value: mir::InterfaceValue::Local { transfer, .. },
        ..
    }) = &mut args[0]
    else {
        unreachable!()
    };
    *transfer = true;
    assert!(doriac::mir_validation::validate_program(&bad_receiver).is_err());
    let mut unproved = program.clone();
    let mut unregistered = unproved.functions.last().unwrap().clone();
    unregistered.id = mir::FunctionId(unproved.functions.len());
    unproved.functions.push(unregistered);
    assert!(doriac::mir_validation::validate_program(&unproved)
        .unwrap_err()
        .message
        .contains("exact entry-vtable proof"));
    let mut expired = program.clone();
    expired.functions[program.entry.0].blocks[0]
        .statements
        .push(mir::Statement::DropError {
            local: mir::LocalId(0),
        });
    assert!(doriac::mir_validation::validate_program(&expired)
        .unwrap_err()
        .message
        .contains("after its ownership ended"));
    let mut bad_entry = program;
    bad_entry.functions.last_mut().unwrap().parameter_modes[0] = mir::FunctionParameterMode::Take;
    assert!(doriac::mir_validation::validate_program(&bad_entry).is_err());
}

#[test]
fn error_interface_preserves_dynamic_identity_and_nullable_pairs() {
    let program =
        doriac::lower_source_to_mir("interface-error.doria", ERROR_CARRIER_SOURCE).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        output.stdout,
        include_bytes!("fixtures/native_io/main_stage35_interface_error_carrier/expected_stdout")
    );
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}

#[test]
fn interface_vtable_validation_rejects_incompatible_payload_and_metadata() {
    let program =
        doriac::lower_source_to_mir("interface-error.doria", ERROR_CARRIER_SOURCE).unwrap();
    let mut duplicate = program.clone();
    duplicate
        .interface_vtables
        .push(duplicate.interface_vtables[0].clone());
    assert!(doriac::mir_validation::validate_program(&duplicate).is_err());

    let mut missing = program.clone();
    missing.interface_vtables.clear();
    assert!(doriac::mir_validation::validate_program(&missing).is_err());

    let mut wrong = program.clone();
    wrong.interface_vtables[0].error_descriptor = None;
    assert!(doriac::mir_validation::validate_program(&wrong).is_err());

    let mut premature_collection = program;
    premature_collection.interface_vtables[0].implementing_type =
        mir::ImplementingType::Collection(mir::CollectionTypeId(0));
    assert!(doriac::mir_validation::validate_program(&premature_collection).is_err());
}

#[test]
fn erased_calls_publish_the_requirement_and_specialized_graph() {
    let source = r#"
interface Root<T> { function read(T $fallback): T; }
interface Child extends Root<int> {}
function read(Child $value): int { return $value->read(1); }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("interface-facts.doria", source).unwrap();
    let interface = analysis
        .info
        .contracts
        .interface_specializations
        .iter()
        .find(|fact| fact.specialization.name == "Child")
        .unwrap();
    assert!(interface.valid);
    assert_eq!(interface.ancestors.len(), 1);
    assert_eq!(interface.ancestors[0].name, "Root");
    let requirement = &interface.requirements[0];
    assert_eq!(
        requirement.signature.parameters[0].r#type,
        ResolvedType::Integer(doriac::numeric::IntegerType::Int64)
    );
    assert_eq!(
        requirement.signature.return_type,
        ResolvedType::Integer(doriac::numeric::IntegerType::Int64)
    );
    assert!(analysis.info.call_targets.values().any(|target| {
        matches!(target, CallableTarget::InterfaceMethod { interface, requirement: origin, .. }
            if interface.name == "Child" && *origin == requirement.origins[0].declaration)
    }));
}

#[test]
fn constrained_interface_specialization_stays_an_erased_requirement_call() {
    let target = CallableTarget::ConstrainedMethod {
        receiver: ResolvedType::TypeParameter("T".into()),
        method_name: "read".into(),
        requirement: doriac::source::Span::default(),
        implementations: Vec::new(),
    };
    let specialized =
        target.specialize(|_| ResolvedType::Interface(InterfaceType::new("Readable", vec![])));
    assert!(
        matches!(specialized, Some(CallableTarget::InterfaceMethod { interface, .. }) if interface.name == "Readable")
    );
}

#[test]
fn interface_carrier_validation_rejects_unknown_and_mismatched_views() {
    let program = doriac::lower_source_to_mir(
        "interface-mir.doria",
        "function main(): void { ?Error $value = null; }",
    )
    .unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    for register_view in [false, true] {
        let mut invalid = program.clone();
        if register_view {
            invalid.interface_types.push(mir::InterfaceType {
                id: mir::InterfaceTypeId(1),
                name: "Other".into(),
                ancestors: vec![],
                methods: vec![],
            });
        }
        let function = &mut invalid.functions[invalid.entry.0];
        let value = function
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.statements)
            .find_map(|statement| match statement {
                mir::Statement::AssignLocal {
                    value: mir::Rvalue::NullableInterface(value),
                    ..
                } => Some(value),
                _ => None,
            })
            .unwrap();
        value.interface = mir::InterfaceTypeId(1);
        assert!(doriac::mir_validation::validate_program(&invalid).is_err());
    }
}

const ERROR_HIERARCHY: &str = r#"
open class Failure implements Error {
    function __construct(string $message) {}
}
class Missing extends Failure {
    function __construct() { parent::__construct("missing"); }
}
"#;

#[test]
fn nominal_catch_facts_distinguish_partial_and_complete_coverage() {
    let source = format!(
        r#"{ERROR_HIERARCHY}
function fail(): void throws Failure {{ throw new Missing(); }}
function main(): void {{
    try {{ fail(); }} catch (Missing) {{}} catch (Failure) {{}}
}}
"#
    );
    let hir = doriac::lower_source("catch-coverage.doria", source)
        .expect("a descendant catch may handle part of an open parent effect");
    let mut catches = hir.semantic_info.catch_coverage.iter().collect::<Vec<_>>();
    catches.sort_by_key(|(span, _)| span.start);
    assert_eq!(catches.len(), 2);
    assert_eq!(catches[0].1.len(), 1);
    assert_eq!(catches[1].1.len(), 1);
    assert_eq!(catches[0].1.values().next(), Some(&CatchCoverage::Partial));
    assert_eq!(catches[1].1.values().next(), Some(&CatchCoverage::Complete));
}

#[test]
fn parent_catches_preserve_exceptional_ownership_states() {
    let source = format!(
        r#"{ERROR_HIERARCHY}
class Payload {{}}
function consume(take Payload $payload): void {{}}
function inspect(Payload $payload): void {{}}
function fail(): void throws Missing {{ throw new Missing(); }}
function invalid(): void {{
    let writable $payload = new Payload();
    try {{
        consume($payload);
        fail();
        $payload = new Payload();
    }} catch (Failure) {{}}
    inspect($payload);
}}
"#
    );
    let errors = doriac::check_source("parent-catch.doria", source)
        .expect_err("the parent catch observes the move before the child Error");
    assert!(
        errors.iter().any(|error| error.code == "E0470"),
        "{errors:#?}"
    );
}

#[test]
fn partial_catches_preserve_both_ownership_paths() {
    for effect in ["Failure", "Error"] {
        let source = format!(
            r#"{ERROR_HIERARCHY}
class Payload {{}}
function consume(take Payload $payload): void {{}}
function inspect(Payload $payload): void {{}}
function fail(): void throws {effect} {{ throw new Missing(); }}
function invalid(): void {{
    let $payload = new Payload();
    try {{ fail(); }}
    catch (Missing) {{ consume($payload); }}
    catch (Error) {{}}
    inspect($payload);
}}
"#
        );
        let errors = doriac::check_source("partial-catch.doria", source)
            .expect_err("a partial catch can consume the value before the join");
        assert!(
            errors.iter().any(|error| error.code == "E0470"),
            "{errors:#?}"
        );
    }
}

#[test]
fn nominal_catches_preserve_constructor_definite_initialization() {
    for (effect, catch) in [
        ("Missing", "Failure"),
        ("Failure", "Missing"),
        ("Error", "Missing"),
    ] {
        let source = format!(
            r#"{ERROR_HIERARCHY}
function load(): string throws {effect} {{ throw new Missing(); }}
class NotReady {{
    string $value;
    function __construct() throws {effect} {{
        try {{ $this->value = load(); }} catch ({catch}) {{}}
    }}
}}
"#
        );
        let errors = doriac::check_source("catch-initialization.doria", source)
            .expect_err("a caught RHS failure leaves the property uninitialized");
        assert!(
            errors.iter().any(|error| error.code == "E0500"),
            "{errors:#?}"
        );
    }
}
