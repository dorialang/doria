use doriac::mir::{self, ByteRegionKind, Rvalue, Statement, Type};
use doriac::numeric::{IntegerType, IntegerValue};

fn lower(body: &str) -> mir::Program {
    let source =
        format!("use Doria\\Std\\Io\\ByteRegion;\nuse Doria\\Std\\Io\\WritableByteRegion;\n{body}");
    doriac::lower_source_to_mir("region-mir.doria", &source).unwrap_or_else(|errors| {
        let hir = doriac::lower_source("region-mir.doria", &source).unwrap();
        let mir = doriac::mir_lowering::lower_program(&hir).unwrap();
        panic!("{source}\n{errors:#?}\n{mir}")
    })
}

fn main(program: &mut mir::Program) -> &mut mir::Function {
    program
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .unwrap()
}

fn local(function: &mir::Function, name: &str) -> mir::LocalId {
    function
        .locals
        .iter()
        .find(|local| local.name == name)
        .unwrap()
        .id
}

fn insert_after_assignment(
    function: &mut mir::Function,
    target: mir::LocalId,
    added: Vec<Statement>,
) {
    for block in &mut function.blocks {
        if let Some(index) = block.statements.iter().position(|statement|
            matches!(statement, Statement::AssignLocal { target: found, .. } if *found == target)) {
            block.statements.splice(index + 1..index + 1, added);
            return;
        }
    }
    panic!("fixture local has no assignment");
}

fn reject(program: &mir::Program, expected: &str) {
    let error = doriac::mir_validation::validate_program(program)
        .expect_err("forged MIR must be rejected before execution");
    assert!(
        error.message.contains(expected),
        "expected {expected:?}: {error:?}"
    );
}

fn integer(ty: IntegerType, bits: u64) -> Rvalue {
    Rvalue::Value(mir::ValueExpression::Integer(
        mir::IntegerExpression::constant(IntegerValue::from_bits(ty, bits)),
    ))
}

fn byte_write(bytes: mir::LocalId) -> Statement {
    Statement::AssignCollectionIndex {
        collection: bytes,
        positional: true,
        index: integer(IntegerType::Int64, 0),
        value: integer(IntegerType::UInt8, 7),
        index_span: doriac::source::Span::new(0, 0),
    }
}

#[test]
fn collection_region_projections_cannot_forge_storage_lifetimes_or_capabilities() {
    let source = r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(1);
    {
        List<ByteRegion> $items = [$bytes->getRegion(0, 1)];
        let $selected = $items[0];
        echo $selected[0];
    }
    $bytes[0] = 9;
}
"#;
    let mut program = lower(source);
    let function = main(&mut program);
    let bytes = local(function, "bytes");
    let selected = local(function, "selected");
    insert_after_assignment(function, selected, vec![byte_write(bytes)]);
    reject(&program, "retains it");

    let mut program = lower(source);
    let function = main(&mut program);
    let selected = local(function, "selected");
    let mut changed = false;
    for statement in function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
    {
        if let Statement::AssignLocal {
            target,
            value: Rvalue::ByteRegion(mir::ByteRegionExpression::CollectionIndex { kind, .. }),
        } = statement
        {
            if *target == selected {
                *kind = ByteRegionKind::Writable;
                changed = true;
            }
        }
    }
    assert!(changed);
    reject(&program, "capability");
}

#[test]
fn region_returns_preserve_view_loans_not_owned_container_contents() {
    for ty in [
        "ByteRegion",
        "?ByteRegion",
        "WritableByteRegion",
        "?WritableByteRegion",
    ] {
        for expression in ["$view", "$call($view)"] {
            let mut program = lower(&format!(
                "function forward(function(take {ty}): {ty} $call, take {ty} $view): {ty} {{ return {expression}; }} function main(): void {{}}"
            ));
            doriac::mir_validation::validate_program(&program).unwrap();
            let function = program
                .functions
                .iter_mut()
                .find(|f| f.name == "forward")
                .unwrap();
            let input = local(function, "view");
            let plan = function
                .blocks
                .iter_mut()
                .flat_map(|b| &mut b.statements)
                .find_map(|statement| match statement {
                    Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) => {
                        Some(plan)
                    }
                    _ => None,
                })
                .unwrap();
            assert!(
                plan.returned_storage
                    .as_ref()
                    .is_none_or(|storage| storage.inputs.is_empty()),
                "{ty}: {plan:?}"
            );
            assert!(plan
                .returns
                .iter()
                .any(|source| source.local == input && source.inherited));
            plan.returned_storage
                .get_or_insert_with(Default::default)
                .inputs
                .push(input);
            reject(&program, "returned storage");
        }
    }
}

#[test]
fn finite_repeated_retained_paths_and_recursive_contracts_validate_independently() {
    let program = lower(
        r#"
class Link {
    writable Link $next;
    writable Bytes $left = Bytes::allocate(4);
    writable Bytes $right = Bytes::allocate(4);
    function __construct(parameter take Link $next) { $this->next = $next; }
}

function one(writable Link $link): WritableByteRegion {
    return $link->next->left->getWritableRegion(0, 4);
}
function two(writable Link $link): WritableByteRegion { return one($link->next); }
function recursive(writable Link $link, bool $stop): WritableByteRegion {
    if ($stop) { return $link->left->getWritableRegion(0, 4); }
    return recursive($link->next, true);
}
function mutual(writable Link $link, bool $stop): WritableByteRegion {
    if ($stop) { return $link->left->getWritableRegion(0, 4); }
    return forward($link->next, true);
}
function forward(writable Link $link, bool $stop): WritableByteRegion {
    return mutual($link, $stop);
}
function finite(writable Link $link): void {
    let writable $view = two($link);
    $link->next->next->right[0] = 1;
    echo $view[0];
}
function main(): void {}
"#,
    );
    doriac::mir_validation::validate_program(&program).unwrap();
    let two = program
        .functions
        .iter()
        .find(|function| function.name == "two")
        .unwrap();
    let plan = two
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| {
            if let Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) =
                statement
            {
                Some(plan)
            } else {
                None
            }
        })
        .unwrap();
    assert!(plan.returns.iter().any(|source| {
        source.owned_path.len() == 3 && source.owned_path[0] == source.owned_path[1]
    }));
}

#[test]
fn erased_receivers_preserve_inherited_retained_field_paths() {
    let program = lower(
        r#"
interface Reader { function(): uint8 $callback { get; } }
function make(Bytes $buffer): function(): uint8 { return fn() with ($buffer) => $buffer[0]; }
open class Base {
    Bytes $buffer = Bytes::allocate(4);
    function(): uint8 $callback { get => make($this->buffer); }
}
class Owner extends Base implements Reader {}
function forward(Reader $owner): function(): uint8 {
    return $owner->callback;
}
function main(): void {
    let $owner = new Owner();
    let $callback = forward($owner);
    echo $callback();
}
"#,
    );
    doriac::mir_validation::validate_program(&program).unwrap();
    let base = program
        .classes
        .iter()
        .find(|class| class.name == "Base")
        .unwrap()
        .id;
    assert!(program.functions.iter().any(|function| {
        function.name.contains("interface-entry") && function.blocks.iter().flat_map(|block| &block.statements).any(|statement| {
            matches!(statement, Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan))
                if plan.returns.iter().any(|source| source.owned_path.first().is_some_and(|field| field.class == base)))
        })
    }), "erased entry must transport the resolved inherited storage path");
}

#[test]
fn forged_field_traversal_on_a_cfg_cycle_terminates_without_a_depth_cap() {
    let mut program = lower(
        r#"
class Link {
    Link $next;
    function __construct(parameter take Link $next) { $this->next = $next; }
}
function identity(Link $link): Link { return $link; }
function walk(Link $link, bool $keep): void {
    let $cursor = identity($link);
    let $step = identity($link);
    while ($keep) {}
}
function main(): void {}
"#,
    );
    let class = program
        .classes
        .iter()
        .find(|class| class.name == "Link")
        .unwrap();
    let next = class
        .properties
        .iter()
        .find(|property| property.name == "next")
        .unwrap()
        .id;
    let class = class.id;
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "walk")
        .unwrap();
    let cursor = local(function, "cursor");
    let step = local(function, "step");
    let link = function.params[0];
    for statement in function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
    {
        if let Statement::AssignLocal { target, value } = statement {
            if *target == cursor || *target == step {
                *value = Rvalue::Class(mir::ClassExpression::Local {
                    class,
                    local: link,
                    transfer: false,
                });
            }
        }
    }
    let header = function
        .blocks
        .iter()
        .find_map(|block| {
            matches!(block.terminator, mir::Terminator::Branch { .. }).then_some(block.id)
        })
        .unwrap();
    let body = function
        .blocks
        .iter_mut()
        .find(|block| {
            block.id != function.entry_block && block.terminator == mir::Terminator::Jump(header)
        })
        .expect("loop has an executable backedge");
    body.statements.push(Statement::AssignLocal {
        target: step,
        value: Rvalue::Class(mir::ClassExpression::Property {
            class,
            object: cursor,
            property: next,
        }),
    });
    body.statements.push(Statement::AssignLocal {
        target: cursor,
        value: Rvalue::Class(mir::ClassExpression::Local {
            class,
            local: step,
            transfer: false,
        }),
    });
    // Readonly user bindings cannot be reassigned; retain that structural
    // rejection before testing legal compiler-generated control-flow slots.
    reject(&program, "must have exactly one reachable initializer");
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "walk")
        .unwrap();
    function.locals[cursor.0].synthetic = true;
    function.locals[step.0].synthetic = true;
    // Omit a source ownership summary: the independent proof must converge
    // from actual cyclic MIR, not trust frontend rejection or a depth limit.
    doriac::mir_validation::validate_program(&program).unwrap();
}

fn owned_field_fixture() -> mir::Program {
    lower(
        r#"
class Buffers {
    writable Bytes $left = Bytes::allocate(4);
    writable Bytes $right = Bytes::allocate(4);
}
class Owner { writable Buffers $buffers = new Buffers(); }
function region(writable Owner $owner): WritableByteRegion {
    return $owner->buffers->left->getWritableRegion(0, 4);
}
function main(): void {
    let writable $owner = new Owner();
    let writable $view = region($owner);
    $owner->buffers->right[0] = 1;
    echo $view[0];
}
"#,
    )
}

#[test]
fn independently_owned_nested_fields_validate_without_source_effect_exemptions() {
    let program = owned_field_fixture();
    doriac::mir_validation::validate_program(&program).unwrap();
    let region = program
        .functions
        .iter()
        .find(|function| function.name == "region")
        .unwrap();
    let plan = region
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| {
            if let Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) =
                statement
            {
                Some(plan)
            } else {
                None
            }
        })
        .unwrap();
    assert!(plan
        .returns
        .iter()
        .any(|source| source.owned_path.len() == 2));
}

#[test]
fn forged_retained_contract_cannot_claim_an_unrelated_sibling_field() {
    let mut program = owned_field_fixture();
    let right = program
        .classes
        .iter()
        .find(|class| class.name == "Buffers")
        .unwrap()
        .properties
        .iter()
        .find(|property| property.name == "right")
        .unwrap()
        .id;
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "region")
        .unwrap();
    let mut changed = 0;
    for statement in function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
    {
        if let Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) = statement {
            for source in &mut plan.returns {
                assert_eq!(source.owned_path.len(), 2);
                source.owned_path[1] = right;
                changed += 1;
            }
        }
    }
    assert_eq!(changed, 1);
    reject(&program, "return loses or outlives a retained source loan");
}

#[test]
fn forged_nested_alias_cannot_write_the_same_selected_field() {
    let mut program = owned_field_fixture();
    let left = program
        .classes
        .iter()
        .find(|class| class.name == "Buffers")
        .unwrap()
        .properties
        .iter()
        .find(|property| property.name == "left")
        .unwrap()
        .id;
    let function = main(&mut program);
    let mut changed = 0;
    for statement in function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
    {
        if let Statement::AssignLocal {
            value: Rvalue::Collection(mir::CollectionExpression::Property { property, .. }),
            ..
        } = statement
        {
            *property = left;
            changed += 1;
        }
    }
    assert_eq!(changed, 1);
    reject(&program, "retains it");
}

#[test]
fn forged_parent_drop_remains_blocked_during_a_nested_field_loan() {
    let mut program = owned_field_fixture();
    let function = main(&mut program);
    let owner = local(function, "owner");
    let view = local(function, "view");
    let drop = function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find(
            |statement| matches!(statement, Statement::DropClass { local, .. } if *local == owner),
        )
        .expect("owning parent has ordinary cleanup")
        .clone();
    insert_after_assignment(function, view, vec![drop]);
    reject(&program, "after its ownership ended");
}

#[test]
fn nested_sibling_arguments_and_materialized_aliases_use_physical_paths() {
    for body in [
        "touch($tree->first->left, $tree->first->right);",
        "touch($tree->first->left, $tree->second->left);",
        "let $left = identity($tree->first->left); observe($left, $tree->first->right); echo $left->value;",
        "$tree->first->left->value = $tree->first->right->value;",
    ] {
        let program = lower(&format!(
            r#"
class Cell {{ writable int $value = 0; }}
class Branch {{
    writable Cell $left = new Cell();
    writable Cell $right = new Cell();
}}
class Tree {{
    writable Branch $first = new Branch();
    writable Branch $second = new Branch();
}}
function touch(writable Cell $left, writable Cell $right): void {{}}
function observe(Cell $left, writable Cell $right): void {{}}
function identity(Cell $cell): Cell {{ return $cell; }}
function route(writable Tree $tree): void {{ {body} }}
function main(): void {{}}
"#
        ));
        doriac::mir_validation::validate_program(&program)
            .unwrap_or_else(|error| panic!("{body}: {error:?}"));
    }
}

fn copy_fixture() -> mir::Program {
    lower(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    {
        ByteRegion $view = $bytes->getRegion(0, 2);
        ByteRegion $copy = $view;
        let $used = $copy[0];
    }
    $bytes[0] = 7;
}
"#,
    )
}

fn writable_fixture() -> mir::Program {
    lower(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    {
        writable WritableByteRegion $view = $bytes->getWritableRegion(0, 2);
        $view[0] = 3;
    }
    $bytes[0] = 7;
}
"#,
    )
}

#[test]
fn readonly_region_snapshots_keep_storage_without_borrowing_the_descriptor() {
    for nullable in [false, true] {
        let ty = if nullable {
            "?ByteRegion"
        } else {
            "ByteRegion"
        };
        let subregion = if nullable {
            "$view?->getRegion(0, 1)"
        } else {
            "$view->getRegion(0, 1)"
        };
        for expression in [subregion, "$call($view)", "through($call, $view)"] {
            let read = if nullable {
                "if ($snapshot != null) { echo $snapshot[0]; } if ($view != null) { echo $view[0]; }"
            } else {
                "echo $snapshot[0]; echo $view[0];"
            };
            let source = format!(
                r#"
function through(function({ty}): {ty} $call, {ty} $value): {ty} {{ return $call($value); }}
function main(): void {{
    writable Bytes $first = Bytes::allocate(1);
    writable Bytes $second = Bytes::allocate(1);
    let $call = fn({ty} $value) => $value;
    writable {ty} $view = $first->getRegion(0, 1);
    let $snapshot = {expression};
    $view = $second->getRegion(0, 1);
    {read}
}}
"#
            );
            let mut program = lower(&source);
            doriac::mir_validation::validate_program(&program).unwrap();
            let function = main(&mut program);
            let view = local(function, "view");
            let snapshot = local(function, "snapshot");
            let first = local(function, "first");
            insert_after_assignment(
                function,
                snapshot,
                vec![Statement::EndBorrow { local: view }, byte_write(first)],
            );
            reject(&program, "retains it");
        }
    }
}

#[test]
fn explicit_readonly_region_capture_still_borrows_its_descriptor() {
    let mut program = lower(
        r#"
function main(): void {
    let $bytes = Bytes::allocate(1);
    writable ByteRegion $view = $bytes->getRegion(0, 1);
    let $call = fn() with ($view) => $view[0];
    echo $call();
}
"#,
    );
    let function = main(&mut program);
    let view = local(function, "view");
    let call = local(function, "call");
    let replacement = function.blocks.iter().flat_map(|block| &block.statements).find(|statement| matches!(statement, Statement::AssignLocal { target, .. } if *target == view)).unwrap().clone();
    insert_after_assignment(function, call, vec![replacement]);
    reject(&program, "parent is accessed while its reborrow is live");
}

#[test]
fn readonly_callback_snapshots_preserve_the_writable_ancestor_loan() {
    let mut program = lower(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(1);
    writable WritableByteRegion $parent = $bytes->getWritableRegion(0, 1);
    let $child = $parent->getRegion(0, 1);
    let $call = fn(ByteRegion $value) => $value;
    let $snapshot = $call($child);
    echo $snapshot[0];
}
"#,
    );
    let function = main(&mut program);
    let parent = local(function, "parent");
    let child = local(function, "child");
    let snapshot = local(function, "snapshot");
    insert_after_assignment(
        function,
        snapshot,
        vec![
            Statement::EndBorrow { local: child },
            Statement::AssignByteRegionIndex {
                region: parent,
                index: integer(IntegerType::Int64, 0),
                value: integer(IntegerType::UInt8, 9),
                index_span: doriac::source::Span::new(0, 0),
            },
        ],
    );
    reject(&program, "parent is accessed while its reborrow is live");
}

#[test]
fn readonly_region_copy_keeps_its_own_loan_after_original_carrier_ends() {
    let mut program = copy_fixture();
    let function = main(&mut program);
    let view = local(function, "view");
    let copy = local(function, "copy");
    insert_after_assignment(function, copy, vec![Statement::EndBorrow { local: view }]);
    doriac::mir_validation::validate_program(&program)
        .expect("ending original carrier must not invalidate its readonly copy");
    let function = main(&mut program);
    let bytes = local(function, "bytes");
    insert_after_assignment(
        function,
        copy,
        vec![Statement::EndBorrow { local: view }, byte_write(bytes)],
    );
    reject(&program, "retains it");
}

#[test]
fn region_carrier_cannot_be_read_after_end_borrow() {
    let mut program = copy_fixture();
    let function = main(&mut program);
    let copy = local(function, "copy");
    insert_after_assignment(function, copy, vec![Statement::EndBorrow { local: copy }]);
    reject(&program, "after its source ownership ended");
}

#[test]
fn resource_reads_follow_nested_transfer_order() {
    for (ty, setup) in [
        ("Bytes", "let $value = Bytes::allocate(2);"),
        (
            "WritableByteRegion",
            "writable Bytes $bytes = Bytes::allocate(2); let writable $value = $bytes->getWritableRegion(0, 2);",
        ),
    ] {
        let original = lower(&format!(
            "function consume(take {ty} $value): int {{ return $value->length; }}\n\
             function main(): void {{ {setup} let $result = consume($value); echo $result; }}"
        ));
        doriac::mir_validation::validate_program(&original)
            .unwrap_or_else(|error| panic!("{ty} baseline: {error:?}"));
        for consume_first in [false, true] {
            let mut program = original.clone();
            let function = main(&mut program);
            let result = local(function, "result");
            let span = function.source_span;
            let expression = function
                .blocks
                .iter_mut()
                .flat_map(|block| &mut block.statements)
                .find_map(|statement| match statement {
                    Statement::AssignLocal {
                        target,
                        value: Rvalue::Value(mir::ValueExpression::Integer(expression)),
                    } if *target == result => Some(expression),
                    _ => None,
                })
                .expect("fixture must retain the nested consuming call");
            let call = expression.clone();
            let mir::IntegerExpression::Call { args, .. } = &call else {
                panic!("{ty}: consuming call was materialized separately");
            };
            let value = args[0]
                .direct_place_local()
                .expect("consume must transfer its materialized argument owner");
            let operand = if ty == "Bytes" {
                mir::Operand::CollectionLength(value)
            } else {
                mir::Operand::ByteRegionLength(value)
            };
            let read = mir::IntegerExpression::Use {
                ty: IntegerType::Int64,
                operand,
            };
            let (left, right) = if consume_first {
                (call, read)
            } else {
                (read, call)
            };
            *expression = mir::IntegerExpression::Binary {
                ty: IntegerType::Int64,
                op: mir::IntegerBinaryOp::Add,
                left: Box::new(left),
                right: Box::new(right),
                span,
                right_span: span,
            };
            if consume_first {
                reject(&program, "after its source ownership ended");
            } else {
                doriac::mir_validation::validate_program(&program)
                    .unwrap_or_else(|error| panic!("{ty} before transfer: {error:?}"));
            }
        }
    }
}

#[test]
fn temporary_region_loans_follow_nested_call_lifetimes() {
    let original = lower(
        r#"
function consume(take WritableByteRegion $view): int { return $view->length; }
function consumeWithByte(take WritableByteRegion $view, uint8 $byte): int { return $view->length; }
function getView(writable Bytes $bytes): WritableByteRegion { return $bytes->getWritableRegion(0, 2); }
function evaluate(writable Bytes $bytes): int { return $bytes->length; }
function main(): void {}
"#,
    );
    let consume = original
        .functions
        .iter()
        .find(|function| function.name == "consume")
        .unwrap()
        .id;
    let consume_with_byte = original
        .functions
        .iter()
        .find(|function| function.name == "consumeWithByte")
        .unwrap()
        .id;
    let helper = original
        .functions
        .iter()
        .find(|function| function.name == "getView")
        .unwrap();
    for use_helper in [false, true] {
        for overlapping_read in [false, true] {
            let mut program = original.clone();
            let function = program
                .functions
                .iter_mut()
                .find(|function| function.name == "evaluate")
                .unwrap();
            let bytes = function.params[0];
            let Type::Collection(collection) = function.locals[bytes.0].ty else {
                panic!("fixture parameter must be Bytes");
            };
            let span = function.source_span;
            let source = Rvalue::Collection(mir::CollectionExpression::Local {
                collection,
                local: bytes,
                transfer: false,
                assume_non_null: false,
            });
            let region = if use_helper {
                Rvalue::ByteRegion(mir::ByteRegionExpression::Call {
                    kind: ByteRegionKind::Writable,
                    function: helper.id,
                    args: vec![source],
                    span,
                    return_borrow: helper.return_borrow,
                })
            } else {
                Rvalue::ByteRegion(mir::ByteRegionExpression::Region {
                    kind: ByteRegionKind::Writable,
                    source: Box::new(source),
                    offset: Box::new(integer(IntegerType::Int64, 0)),
                    length: Box::new(integer(IntegerType::Int64, 2)),
                    span,
                })
            };
            let call = if overlapping_read {
                mir::IntegerExpression::Call {
                    ty: IntegerType::Int64,
                    function: consume_with_byte,
                    args: vec![
                        region,
                        Rvalue::Value(mir::ValueExpression::Integer(mir::IntegerExpression::Use {
                            ty: IntegerType::UInt8,
                            operand: mir::Operand::CollectionIndex {
                                collection: bytes,
                                index: Box::new(integer(IntegerType::Int64, 0)),
                                positional: true,
                                index_span: span,
                                remove: false,
                            },
                        })),
                    ],
                    span,
                }
            } else {
                let call = mir::IntegerExpression::Call {
                    ty: IntegerType::Int64,
                    function: consume,
                    args: vec![region],
                    span,
                };
                mir::IntegerExpression::Binary {
                    ty: IntegerType::Int64,
                    op: mir::IntegerBinaryOp::Add,
                    left: Box::new(call.clone()),
                    right: Box::new(call),
                    span,
                    right_span: span,
                }
            };
            let terminator = function
                .blocks
                .iter_mut()
                .find_map(|block| {
                    matches!(block.terminator, mir::Terminator::Return(_))
                        .then_some(&mut block.terminator)
                })
                .expect("fixture must have a return expression");
            *terminator =
                mir::Terminator::Return(Rvalue::Value(mir::ValueExpression::Integer(call)));
            if overlapping_read {
                reject(
                    &program,
                    "temporary value retains conflicting source access",
                );
            } else {
                doriac::mir_validation::validate_program(&program).unwrap_or_else(|error| {
                    panic!("sequential reuse (helper={use_helper}): {error:?}")
                });
            }
        }
    }
}

#[test]
fn grouped_region_alias_retains_storage_after_its_sibling_ends() {
    let mut program = lower(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    {
        ByteRegion $left, $right = $bytes->getRegion(0, 2);
        let $used = $right[0];
    }
    $bytes[0] = 7;
}
"#,
    );
    let function = main(&mut program);
    let left = local(function, "left");
    let right = local(function, "right");
    let bytes = local(function, "bytes");
    let (block, index) = function
        .blocks
        .iter_mut()
        .find_map(|block| {
            block
                .statements
                .iter()
                .position(|statement| {
                    matches!(statement, Statement::AssignLocalGroup { targets, .. }
                        if targets.as_slice() == [left, right])
                })
                .map(|index| (block, index))
        })
        .expect("fixture must lower to one grouped descriptor assignment");
    block
        .statements
        .insert(index + 1, Statement::EndBorrow { local: left });
    doriac::mir_validation::validate_program(&program)
        .expect("ending one grouped descriptor must leave its sibling usable");

    let function = main(&mut program);
    let block = function
        .blocks
        .iter_mut()
        .find(|block| {
            block.statements.iter().any(|statement| {
                matches!(statement, Statement::AssignLocalGroup { targets, .. }
                    if targets.contains(&right))
            })
        })
        .unwrap();
    let index = block
        .statements
        .iter()
        .position(|statement| matches!(statement, Statement::EndBorrow { local } if *local == left))
        .unwrap();
    block.statements.insert(index + 1, byte_write(bytes));
    reject(&program, "retains it");
}

#[test]
fn missing_copy_region_scope_end_keeps_the_source_frozen() {
    let mut program = copy_fixture();
    let function = main(&mut program);
    let copy = local(function, "copy");
    for block in &mut function.blocks {
        block.statements.retain(
            |statement| !matches!(statement, Statement::EndBorrow { local } if *local == copy),
        );
    }
    doriac::mir_validation::validate_program(&program)
        .expect("dead region copies do not require lexical EndBorrow to release their loans");
    let function = main(&mut program);
    let bytes = local(function, "bytes");
    let used = local(function, "used");
    let block = function.blocks.iter_mut().find(|block| block.statements.iter().any(|statement| {
        matches!(statement, Statement::AssignCollectionIndex { collection, .. } if *collection == bytes)
    })).unwrap();
    let index = block.statements.iter().position(|statement| {
        matches!(statement, Statement::AssignCollectionIndex { collection, .. } if *collection == bytes)
    }).unwrap();
    // The missing EndBorrow is only unsafe when the alias really remains live.
    block.statements.insert(
        index + 1,
        Statement::AssignLocal {
            target: used,
            value: Rvalue::Value(mir::ValueExpression::Integer(mir::IntegerExpression::Use {
                ty: IntegerType::UInt8,
                operand: mir::Operand::ByteRegionIndex {
                    region: copy,
                    index: Box::new(integer(IntegerType::Int64, 0)),
                    index_span: doriac::source::Span::new(0, 0),
                },
            })),
        },
    );
    reject(&program, "retains it");
}

#[test]
fn writable_region_cannot_be_forged_from_readonly_storage_access() {
    let mut program = writable_fixture();
    let function = main(&mut program);
    let bytes = local(function, "bytes");
    function.locals[bytes.0].writable = false;
    reject(&program, "readonly source binding");
}

#[test]
fn writable_region_excludes_source_reads_and_mutation() {
    for write in [false, true] {
        let mut program = writable_fixture();
        let function = main(&mut program);
        let bytes = local(function, "bytes");
        let view = local(function, "view");
        let statement = if write {
            byte_write(bytes)
        } else {
            let target = mir::LocalId(function.locals.len());
            function.locals.push(mir::Local {
                id: target,
                name: "forged_read".into(),
                ty: Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
                writable: false,
                owned: false,
                synthetic: true,
            });
            Statement::AssignLocal {
                target,
                value: Rvalue::Value(mir::ValueExpression::Integer(mir::IntegerExpression::Use {
                    ty: IntegerType::Int64,
                    operand: mir::Operand::CollectionLength(bytes),
                })),
            }
        };
        insert_after_assignment(function, view, vec![statement]);
        reject(&program, "retains it");
    }
}

#[test]
fn writable_region_descriptor_cannot_be_duplicated_as_a_copy() {
    let mut program = lower(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    {
        let $view = $bytes->getWritableRegion(0, 2);
        let writable $moved = $view;
        $moved[0] = 1;
    }
}
"#,
    );
    let function = main(&mut program);
    let moved = local(function, "moved");
    let assignment = function.blocks.iter_mut().flat_map(|block| &mut block.statements)
        .find(|statement| matches!(statement, Statement::AssignLocal { target, .. } if *target == moved)).unwrap();
    let Statement::AssignLocal {
        value: Rvalue::ByteRegion(mir::ByteRegionExpression::Local { transfer, .. }),
        ..
    } = assignment
    else {
        panic!("move fixture must transport the region descriptor directly");
    };
    assert!(*transfer);
    *transfer = false;
    reject(&program, "inconsistent descriptor ownership");
}

#[test]
fn live_subregion_suspends_parent_but_child_end_releases_it() {
    let mut program = lower(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    {
        writable WritableByteRegion $view = $bytes->getWritableRegion(0, 2);
        {
            writable WritableByteRegion $child = $view->getWritableRegion(0, 1);
            $child[0] = 3;
        }
        $view[0] = 4;
    }
    $bytes[0] = 7;
}
"#,
    );
    let function = main(&mut program);
    let view = local(function, "view");
    let child = local(function, "child");
    insert_after_assignment(
        function,
        child,
        vec![Statement::AssignByteRegionIndex {
            region: view,
            index: integer(IntegerType::Int64, 0),
            value: integer(IntegerType::UInt8, 5),
            index_span: doriac::source::Span::new(0, 0),
        }],
    );
    reject(&program, "parent is accessed while its reborrow is live");
}

#[test]
fn helper_return_cannot_erase_its_retained_source_contract() {
    let original = lower(
        r#"
function window(Bytes $bytes): ByteRegion { return $bytes->getRegion(0, 1); }
function main(): void {
    let $bytes = Bytes::allocate(2);
    let $view = window($bytes);
    let $used = $view[0];
}
"#,
    );
    let mut program = original;
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "window")
        .unwrap();
    for statement in function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
    {
        if let Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) = statement {
            plan.returns.clear();
        }
    }
    reject(&program, "loses its retained source plan");
}

#[test]
fn readonly_region_carrier_cannot_claim_move_ownership() {
    let mut program = copy_fixture();
    let function = main(&mut program);
    let copy = local(function, "copy");
    assert_eq!(
        function.locals[copy.0].ty,
        Type::ByteRegion(ByteRegionKind::Readonly)
    );
    function.locals[copy.0].owned = true;
    reject(&program, "Copy byte region cannot own a descriptor");
}

#[test]
fn callable_region_return_cannot_erase_its_retained_parameter_contract() {
    let mut program = lower(
        r#"
function apply(function(ByteRegion): ByteRegion $call, ByteRegion $view): ByteRegion {
    return $call($view);
}
function main(): void {
    let $bytes = Bytes::allocate(1);
    let $forward = fn(ByteRegion $view) => $view;
    let $result = apply($forward, $bytes->getRegion(0, 1));
    let $used = $result[0];
}
"#,
    );
    let mut erased = 0;
    for signature in &mut program.function_types {
        if signature.return_type
            == mir::ReturnType::Value(Type::ByteRegion(ByteRegionKind::Readonly))
        {
            assert_eq!(
                signature.return_borrow.unwrap().kind,
                mir::ReturnBorrowKind::Retained
            );
            signature.return_borrow = None;
            erased += 1;
        }
    }
    assert!(
        erased > 0,
        "fixture must include a structural region return contract"
    );
    // Structural validation precedes executable-origin solving, so the known
    // closure cannot disguise disagreement with its original descriptor.
    reject(&program, "disagrees with its descriptor or structural type");

    // An opaque callback has no descriptor to supply its missing contract.
    // Independently check that erasing the bound still loses the region loan.
    let mut opaque = lower(
        r#"
function apply(function(ByteRegion): ByteRegion $call, ByteRegion $view): ByteRegion {
    return $call($view);
}
function main(): void {}
"#,
    );
    for signature in &mut opaque.function_types {
        if signature.return_type
            == mir::ReturnType::Value(Type::ByteRegion(ByteRegionKind::Readonly))
        {
            signature.return_borrow = None;
        }
    }
    reject(
        &opaque,
        "byte region return has no retained source provenance",
    );
}

#[test]
fn consuming_receiver_cannot_be_forged_as_any_retained_return_source() {
    for (result, body) in [
        ("ByteRegion", "return $source->getRegion(0, 1);"),
        ("?ByteRegion", "return $source->getRegion(0, 1);"),
        (
            "function(): int",
            "let $view = $source->getRegion(0, 1); return fn() with (take $view) => $view->length;",
        ),
    ] {
        let original = lower(&format!(
            "class Packet {{ consuming function finish(Bytes $source): {result} {{ {body} }} }} function main(): void {{}}"
        ));
        for inherited in [false, true] {
            let mut program = original.clone();
            let function = program
                .functions
                .iter_mut()
                .find(|function| {
                    function
                        .method
                        .as_ref()
                        .is_some_and(|method| method.name == "finish")
                })
                .expect("fixture must include the consuming method");
            assert_eq!(function.receiver_mode, Some(mir::ReceiverMode::Consuming));
            let receiver = function.params[0];
            assert!(function.locals[receiver.0].owned);
            // A missing single-source header must not let a forged complete
            // retained plan name a source destroyed by this consuming call.
            function.return_borrow = None;
            let plan = function.blocks[function.entry_block.0]
                .statements
                .iter_mut()
                .find_map(|statement| match statement {
                    Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) => {
                        Some(plan)
                    }
                    _ => None,
                })
                .expect("the caller argument must carry a retained return plan");
            assert!(!plan.returns.is_empty());
            plan.returns = vec![mir::RetainedSource {
                local: receiver,
                inherited,
                writable: false,
                lifetime: doriac::types::RetainedLifetime::LastUse,
                owned_path: Vec::new(),
            }];
            reject(
                &program,
                "retained return cannot borrow from a consuming receiver",
            );
        }
    }
}
