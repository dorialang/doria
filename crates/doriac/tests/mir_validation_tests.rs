use doriac::class_layout::{compute_class_layout, ClassId, FieldType, PropertyId};
use doriac::format_string::{FormatConversion, FormatPiece, FormatSpec};
use doriac::mir::{
    BasicBlock, BlockId, BoolExpression, Class, ClassExpression, CollectionComparator,
    CollectionExpression, CollectionKind, CollectionMembershipOp, CollectionType, CollectionTypeId,
    FloatBinaryOp, FloatExpression, FormatArgument, FormatExpression, Function, FunctionId,
    IntegerExpression, Local, LocalId, NullableClassExpression, NullableScalarExpression,
    NullableSharedReferenceExpression, NullableStringExpression, Operand, Program, Property,
    PropertyValue, PropertyValueSource, ReturnType, Rvalue, ScalarType, ScalarValue,
    SharedReferenceExpression, Statement, StaticId, StaticProperty, StaticValue, StringExpression,
    StringIntrinsicCall, StringIntrinsicKind, Terminator, Type, ValueExpression,
    WeakReferenceExpression,
};
use doriac::numeric::{FloatType, FloatValue, IntegerType, IntegerValue};

#[test]
fn shared_validator_rejects_malformed_collection_clear_shapes() {
    let source = r#"
function main(): void
{
    writable List<int> $values = [1, 2];
    $values->clear();
}
"#;
    let valid = doriac::lower_source_to_mir("clear.doria", source)
        .expect("valid collection clear should lower");
    doriac::mir_validation::validate_program(&valid)
        .expect("valid collection clear MIR should validate");

    for kind in [CollectionKind::TypedArray, CollectionKind::Bytes] {
        let mut malformed = valid.clone();
        malformed.collection_types[0].kind = kind;
        let error = doriac::mir_validation::validate_program(&malformed)
            .expect_err("fixed and byte collections cannot be cleared");
        assert!(
            error.message.contains("collection"),
            "unexpected malformed clear diagnostic: {}",
            error.message
        );
    }

    let mut mismatched = valid.clone();
    mismatched.collection_types.push(CollectionType {
        id: CollectionTypeId(1),
        kind: CollectionKind::Set,
        key: None,
        value: Type::Scalar(ScalarType::Integer(IntegerType::Int64)),
        comparator: None,
    });
    let clear = mismatched.functions[0].blocks[0]
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            Statement::CollectionClear {
                collection_type, ..
            } => Some(collection_type),
            _ => None,
        })
        .expect("clear statement should exist");
    *clear = CollectionTypeId(1);
    assert!(doriac::mir_validation::validate_program(&mismatched)
        .expect_err("clear type identity must match the receiver")
        .message
        .contains("type mismatch"));

    let local = valid.functions[0].blocks[0]
        .statements
        .iter()
        .find_map(|statement| match statement {
            Statement::CollectionClear { collection, .. } => Some(*collection),
            _ => None,
        })
        .expect("clear statement should exist");

    let mut nullable = valid.clone();
    nullable.functions[0].locals[local.0].ty = Type::NullableCollection(CollectionTypeId(0));
    doriac::mir_validation::validate_program(&nullable)
        .expect_err("clear requires a proven-present receiver");

    let mut scalar = valid.clone();
    scalar.functions[0].locals[local.0].ty = Type::Scalar(ScalarType::Integer(IntegerType::Int64));
    doriac::mir_validation::validate_program(&scalar)
        .expect_err("clear requires a collection receiver");

    let mut readonly = valid;
    readonly.functions[0].locals[local.0].writable = false;
    doriac::mir_validation::validate_program(&readonly)
        .expect_err("clear requires a writable local");
}

#[test]
fn shared_validator_rejects_malformed_list_index_of_shapes() {
    let source = r#"
function main(): void
{
    List<int> $values = [1, 2];
    ?int $position = $values->indexOf(2);
}
"#;

    let mut wrong_receiver =
        doriac::lower_source_to_mir("index-of.doria", source).expect("valid indexOf should lower");
    wrong_receiver.collection_types[0].kind = CollectionKind::Set;
    assert!(doriac::mir_validation::validate_program(&wrong_receiver)
        .expect_err("indexOf on a non-list must be malformed")
        .message
        .contains("List::indexOf"));

    let mut wrong_probe =
        doriac::lower_source_to_mir("index-of.doria", source).expect("valid indexOf should lower");
    let entry = wrong_probe.entry.0;
    let expression = wrong_probe.functions[entry].blocks[0]
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            Statement::AssignLocal {
                value:
                    Rvalue::NullableScalar(NullableScalarExpression::CollectionIndexOf {
                        value, ..
                    }),
                ..
            } => Some(value),
            _ => None,
        })
        .expect("indexOf assignment should be present");
    **expression = Rvalue::String(StringExpression::Literal("wrong".to_string()));
    assert!(doriac::mir_validation::validate_program(&wrong_probe)
        .expect_err("indexOf with a wrong probe type must be malformed")
        .message
        .contains("argument type"));

    let mut wrong_result =
        doriac::lower_source_to_mir("index-of.doria", source).expect("valid indexOf should lower");
    let entry = wrong_result.entry.0;
    let target = wrong_result.functions[entry].blocks[0]
        .statements
        .iter()
        .find_map(|statement| match statement {
            Statement::AssignLocal {
                target,
                value: Rvalue::NullableScalar(NullableScalarExpression::CollectionIndexOf { .. }),
            } => Some(*target),
            _ => None,
        })
        .expect("indexOf assignment should be present");
    wrong_result.functions[entry].locals[target.0].ty =
        Type::Scalar(ScalarType::Integer(IntegerType::Int64));
    assert!(doriac::mir_validation::validate_program(&wrong_result)
        .expect_err("indexOf into non-nullable int must be malformed")
        .message
        .contains("nullable"));
}

#[test]
fn shared_validator_rejects_malformed_slice_three_membership_shapes() {
    let source = r#"
function main(): void
{
    Dictionary<string, int> $values = ["one" => 1];
    bool $found = $values->containsValue(1);
}
"#;
    let mut wrong_receiver = doriac::lower_source_to_mir("contains-value.doria", source)
        .expect("valid containsValue should lower");
    let entry = wrong_receiver.entry.0;
    let collection = wrong_receiver.functions[entry].blocks[0]
        .statements
        .iter()
        .find_map(|statement| match statement {
            Statement::AssignLocal {
                value:
                    Rvalue::Value(ValueExpression::Bool(BoolExpression::CollectionHas {
                        collection,
                        op: CollectionMembershipOp::ContainsValue,
                        ..
                    })),
                ..
            } => Some(*collection),
            _ => None,
        })
        .expect("containsValue assignment should be present");
    wrong_receiver.functions[entry].blocks[0]
        .statements
        .retain(|statement| {
            !matches!(statement, Statement::AssignLocal { target, .. } if *target == collection)
        });
    wrong_receiver.collection_types[0].kind = CollectionKind::List;
    wrong_receiver.collection_types[0].key = None;
    assert!(doriac::mir_validation::validate_program(&wrong_receiver)
        .expect_err("containsValue on a non-map must be malformed")
        .message
        .contains("collection kind"));

    let mut wrong_axis_type = doriac::lower_source_to_mir("contains-value.doria", source)
        .expect("valid containsValue should lower");
    let entry = wrong_axis_type.entry.0;
    let value = wrong_axis_type.functions[entry].blocks[0]
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            Statement::AssignLocal {
                value:
                    Rvalue::Value(ValueExpression::Bool(BoolExpression::CollectionHas {
                        value,
                        op: CollectionMembershipOp::ContainsValue,
                        ..
                    })),
                ..
            } => Some(value),
            _ => None,
        })
        .expect("containsValue assignment should be present");
    **value = Rvalue::String(StringExpression::Literal("one".to_string()));
    assert!(doriac::mir_validation::validate_program(&wrong_axis_type)
        .expect_err("containsValue must validate against the value axis")
        .message
        .contains("argument type"));

    let remove_source = r#"
function main(): void
{
    writable List<int> $values = [1, 2];
    bool $removed = $values->remove(1);
}
"#;
    let mut readonly_remove = doriac::lower_source_to_mir("list-remove.doria", remove_source)
        .expect("valid writable List::remove should lower");
    let entry = readonly_remove.entry.0;
    let collection = readonly_remove.functions[entry].blocks[0]
        .statements
        .iter()
        .find_map(|statement| match statement {
            Statement::AssignLocal {
                value:
                    Rvalue::Value(ValueExpression::Bool(BoolExpression::CollectionHas {
                        collection,
                        op: CollectionMembershipOp::Remove,
                        ..
                    })),
                ..
            } => Some(*collection),
            _ => None,
        })
        .expect("List::remove assignment should be present");
    readonly_remove.functions[entry].locals[collection.0].writable = false;
    assert!(doriac::mir_validation::validate_program(&readonly_remove)
        .expect_err("remove against a readonly MIR place must be malformed")
        .message
        .contains("readonly"));
}

#[test]
fn shared_validator_rejects_set_endpoint_access_on_an_invalid_receiver() {
    let source = r#"
function main(): void
{
    Set<int> $values = Set::from([1]);
    ?int $first = $values->first;
}
"#;
    let mut program = doriac::lower_source_to_mir("set-first.doria", source)
        .expect("valid Set::first should lower");
    program.collection_types[0].kind = CollectionKind::TypedArray;
    assert!(doriac::mir_validation::validate_program(&program)
        .expect_err("set endpoint MIR on another receiver must be malformed")
        .message
        .contains("nullable collection access type mismatch"));
}

#[test]
fn shared_validator_rejects_mixed_width_float_binary_operands() {
    let mut program = valid_void_program();
    program.functions.push(Function {
        id: FunctionId(1),
        name: "mixedWidth".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: Vec::new(),
        return_type: ReturnType::Value(Type::Scalar(ScalarType::Float(FloatType::Float64))),
        locals: Vec::new(),
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: Vec::new(),
            terminator: Terminator::Return(Rvalue::Value(ValueExpression::Float(
                FloatExpression::Binary {
                    ty: FloatType::Float64,
                    op: FloatBinaryOp::Add,
                    left: Box::new(FloatExpression::constant(FloatValue::from_f32(1.0))),
                    right: Box::new(FloatExpression::constant(FloatValue::from_f64(2.0))),
                },
            ))),
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("mixed-width float operands must be rejected");
    assert!(error
        .message
        .contains("float binary expression has float32 and float operands"));
}

#[test]
fn shared_validator_rejects_noncanonical_bool_operands() {
    let mut program = valid_void_program();
    program.functions[0].blocks[0].terminator = Terminator::Branch {
        condition: doriac::mir::BoolExpression::Use {
            operand: Operand::Scalar(ScalarValue::Integer(IntegerValue::from_bits(
                IntegerType::Int64,
                1,
            ))),
        },
        then_block: BlockId(0),
        else_block: BlockId(0),
    };

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("integer truthiness must not enter native backends");
    assert!(error
        .message
        .contains("bool expression contains a non-bool constant"));
}

/// The bool and float operand surfaces are validated by exhaustive matches, so
/// every `Operand` variant a scalar element read can produce has to be listed.
/// A catch-all arm here reported well-formed reads as malformed MIR: first for
/// `bool` collection elements, then — identically shaped — for `float`
/// collection elements and `float` statics.
#[test]
fn shared_validator_accepts_every_scalar_element_read_operand() {
    let program = doriac::lower_source_to_mir(
        "scalar-element-reads.doria",
        r#"
class Ratios
{
    static float $wide = 2.5;
    static float32 $narrow = 1.25;
    static bool $on = true;
}

function main(): void
{
    float[] $array = [2.5, 1.5];
    float $bound = $array[0];
    float $sum = $array[0] + 1.0;
    echo "{$bound}{$sum}{$array[1]}\n";
    if ($array[0] > 1.0) {
        echo "ordered\n";
    }

    float32[] $narrow = [2.5, 1.5];
    float32 $narrowBound = $narrow[0];
    echo "{$narrowBound}\n";

    List<float> $list = [3.5];
    Dictionary<int, float> $dictionary = [1 => 6.5];
    echo "{$list[0]}{$dictionary[1]}\n";

    bool[] $flags = [true, false];
    if ($flags[0]) {
        echo "flag\n";
    }

    float $staticWide = Ratios::wide;
    float32 $staticNarrow = Ratios::narrow;
    if (Ratios::on) {
        echo "{$staticWide}{$staticNarrow}\n";
    }
}
"#,
    )
    .expect("scalar element and static reads must lower");

    doriac::mir_validation::validate_program(&program)
        .expect("scalar element and static reads must pass shared MIR validation");
}

#[test]
fn shared_validator_rejects_a_float_element_read_of_another_type() {
    let mut program = valid_void_program();
    let collection = CollectionTypeId(0);
    program.collection_types.push(CollectionType {
        id: collection,
        kind: CollectionKind::TypedArray,
        key: None,
        value: Type::Scalar(ScalarType::Integer(IntegerType::Int64)),
        comparator: None,
    });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "first".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: Vec::new(),
        return_type: ReturnType::Value(Type::Scalar(ScalarType::Float(FloatType::Float64))),
        locals: vec![Local {
            id: LocalId(0),
            name: "numbers".to_string(),
            ty: Type::Collection(collection),
            writable: false,
            owned: true,
            synthetic: false,
        }],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: Vec::new(),
            terminator: Terminator::Return(Rvalue::Value(ValueExpression::Float(
                FloatExpression::Use {
                    ty: FloatType::Float64,
                    operand: Operand::CollectionIndex {
                        positional: true,
                        collection: LocalId(0),
                        index: Box::new(Rvalue::Value(ValueExpression::Integer(
                            IntegerExpression::constant(IntegerValue::from_bits(
                                IntegerType::Int64,
                                0,
                            )),
                        ))),
                        remove: false,
                    },
                },
            ))),
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("an int element must not be read as a float");
    assert!(
        error.message.contains("float index element type mismatch"),
        "unexpected validation message: {}",
        error.message
    );
}

#[test]
fn shared_validator_enforces_grouped_local_invariants() {
    let mut valid = valid_void_program();
    valid.functions[0].locals = (0..2)
        .map(|index| Local {
            id: LocalId(index),
            name: format!("value{index}"),
            ty: Type::Scalar(ScalarType::Integer(IntegerType::Int64)),
            writable: false,
            owned: false,
            synthetic: false,
        })
        .collect();
    valid.functions[0].blocks[0].statements = vec![Statement::AssignLocalGroup {
        targets: vec![LocalId(0), LocalId(1)],
        value: Rvalue::Value(ValueExpression::Integer(IntegerExpression::constant(
            IntegerValue::from_bits(IntegerType::Int64, 1),
        ))),
    }];
    doriac::mir_validation::validate_program(&valid)
        .expect("a canonical Copy-valued local group should validate");

    for (targets, expected) in [
        (vec![LocalId(0)], "at least two targets"),
        (vec![LocalId(0), LocalId(0)], "repeats local0"),
        (vec![LocalId(1), LocalId(0)], "follow declaration order"),
    ] {
        let mut malformed = valid.clone();
        let Statement::AssignLocalGroup {
            targets: actual, ..
        } = &mut malformed.functions[0].blocks[0].statements[0]
        else {
            unreachable!()
        };
        *actual = targets;
        let error = doriac::mir_validation::validate_program(&malformed)
            .expect_err("malformed grouped assignment must be rejected");
        assert!(error.message.contains(expected), "{}", error.message);
    }
}

#[test]
fn shared_validator_rejects_malformed_string_intrinsic_signatures() {
    let malformed = [
        StringIntrinsicCall {
            kind: StringIntrinsicKind::Trim,
            args: vec![],
            result: Type::String,
            span: Default::default(),
            argument_spans: Vec::new(),
        },
        StringIntrinsicCall {
            kind: StringIntrinsicKind::Repeat,
            args: vec![
                Rvalue::String(StringExpression::Literal("x".to_string())),
                Rvalue::String(StringExpression::Literal("2".to_string())),
            ],
            result: Type::String,
            span: Default::default(),
            argument_spans: Vec::new(),
        },
        StringIntrinsicCall {
            kind: StringIntrinsicKind::Upper,
            args: vec![Rvalue::String(StringExpression::Literal("x".to_string()))],
            result: Type::Scalar(ScalarType::Bool),
            span: Default::default(),
            argument_spans: Vec::new(),
        },
        StringIntrinsicCall {
            kind: StringIntrinsicKind::LowerFirst,
            args: vec![Rvalue::String(StringExpression::Literal("x".to_string()))],
            result: Type::Scalar(ScalarType::Bool),
            span: Default::default(),
            argument_spans: Vec::new(),
        },
        StringIntrinsicCall {
            kind: StringIntrinsicKind::ContainsIgnoreCase,
            args: vec![Rvalue::String(StringExpression::Literal("x".to_string()))],
            result: Type::Scalar(ScalarType::Bool),
            span: Default::default(),
            argument_spans: Vec::new(),
        },
        StringIntrinsicCall {
            kind: StringIntrinsicKind::IndexOfIgnoreCase,
            args: vec![
                Rvalue::String(StringExpression::Literal("x".to_string())),
                Rvalue::String(StringExpression::Literal("x".to_string())),
            ],
            result: Type::String,
            span: Default::default(),
            argument_spans: Vec::new(),
        },
        StringIntrinsicCall {
            kind: StringIntrinsicKind::CountOccurrences,
            args: vec![
                Rvalue::String(StringExpression::Literal("x".to_string())),
                Rvalue::String(StringExpression::Literal("x".to_string())),
            ],
            result: Type::Scalar(ScalarType::Bool),
            span: Default::default(),
            argument_spans: Vec::new(),
        },
    ];

    for call in malformed {
        let mut program = valid_void_program();
        program.functions[0].blocks[0]
            .statements
            .push(Statement::EchoString(StringExpression::Intrinsic(
                Box::new(call),
            )));
        let error = doriac::mir_validation::validate_program(&program)
            .expect_err("malformed String intrinsic signatures must be rejected");
        assert!(error.message.contains("String "));
    }
}

#[test]
fn shared_validator_rejects_confused_string_intrinsic_collection_shapes() {
    let mut program = valid_void_program();
    program.collection_types.push(CollectionType {
        id: CollectionTypeId(0),
        kind: CollectionKind::List,
        key: None,
        value: Type::String,
        comparator: None,
    });
    program.functions[0].locals.push(Local {
        id: LocalId(0),
        name: "bytes".to_string(),
        ty: Type::Collection(CollectionTypeId(0)),
        writable: false,
        synthetic: true,
        owned: true,
    });
    program.functions[0].blocks[0].statements = vec![
        Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Collection(CollectionExpression::StringIntrinsic(Box::new(
                StringIntrinsicCall {
                    kind: StringIntrinsicKind::ToBytes,
                    args: vec![Rvalue::String(StringExpression::Literal("x".to_string()))],
                    result: Type::Collection(CollectionTypeId(0)),
                    span: Default::default(),
                    argument_spans: Vec::new(),
                },
            ))),
        },
        Statement::DropCollection {
            local: LocalId(0),
            collection: CollectionTypeId(0),
        },
    ];

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("String bytes must not masquerade as List<string>");
    assert!(error.message.contains("wrong shape"));
}

#[test]
fn shared_validator_rejects_wrong_nullable_string_intrinsic_representation() {
    let mut program = valid_void_program();
    program.functions[0].locals.push(Local {
        id: LocalId(0),
        name: "found".to_string(),
        ty: Type::NullableString,
        writable: false,
        synthetic: true,
        owned: true,
    });
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::NullableString(NullableStringExpression::Intrinsic(Box::new(
                StringIntrinsicCall {
                    kind: StringIntrinsicKind::IndexOf,
                    args: vec![
                        Rvalue::String(StringExpression::Literal("text".to_string())),
                        Rvalue::String(StringExpression::Literal("needle".to_string())),
                    ],
                    result: Type::NullableString,
                    span: Default::default(),
                    argument_spans: Vec::new(),
                },
            ))),
        });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("String search must use the nullable integer representation");
    assert!(error.message.contains("expected ?int"));
}

#[test]
fn shared_validator_rejects_consuming_string_intrinsic_collection_inputs() {
    let mut program = valid_void_program();
    program.collection_types.push(CollectionType {
        id: CollectionTypeId(0),
        kind: CollectionKind::List,
        key: None,
        value: Type::String,
        comparator: None,
    });
    program.functions[0].locals.push(Local {
        id: LocalId(0),
        name: "parts".to_string(),
        ty: Type::Collection(CollectionTypeId(0)),
        writable: false,
        synthetic: false,
        owned: true,
    });
    program.functions[0].blocks[0].statements = vec![
        Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Collection(CollectionExpression::Literal {
                collection: CollectionTypeId(0),
                entries: vec![],
            }),
        },
        Statement::EchoString(StringExpression::Intrinsic(Box::new(StringIntrinsicCall {
            kind: StringIntrinsicKind::Join,
            args: vec![
                Rvalue::String(StringExpression::Literal(",".to_string())),
                Rvalue::Collection(CollectionExpression::Local {
                    collection: CollectionTypeId(0),
                    local: LocalId(0),
                    transfer: true,
                }),
            ],
            result: Type::String,
            span: Default::default(),
            argument_spans: Vec::new(),
        }))),
    ];

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("String::join must borrow rather than consume List<string>");
    assert!(error.message.contains("borrowed collection"));
}

#[test]
fn shared_validator_rejects_writable_access_projection_from_a_strong_handle() {
    let source = r#"
class Value
{
    writable int $number = 0;
}

function main(): void
{
    let $shared = new WritableSharedReference(new Value());
    let writable $access = $shared->acquireWritableAccess();
    $access->number = 1;
}
"#;
    let mut program = doriac::lower_source_to_mir("malformed-writable-access.doria", source)
        .expect("valid writable access should lower");
    let main = program
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .expect("main should exist");
    let access = main
        .locals
        .iter()
        .find(|local| local.name == "access")
        .expect("access local should exist")
        .id;
    let payload = match main
        .locals
        .iter()
        .find(|local| local.id == access)
        .expect("access local should exist")
        .ty
    {
        Type::WritableSharedReferenceAccess(payload) => payload,
        other => panic!("expected writable access, got {other}"),
    };
    main.locals[access.0].ty = Type::WritableSharedReference(payload);

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a strong handle cannot stand in for an access object");
    assert!(error.message.contains("mismatched shared-handle rvalue"));
}

#[test]
fn shared_validator_rejects_writable_operations_over_the_readonly_family() {
    let source = r#"
class Value {}

function main(): void
{
    let $shared = new WritableSharedReference(new Value());
    let $second = $shared->share();
}
"#;
    let mut program = doriac::lower_source_to_mir("malformed-writable-family.doria", source)
        .expect("valid writable sharing should lower");
    let main = program
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .expect("main should exist");
    let strong = main
        .locals
        .iter()
        .find(|local| local.name == "shared")
        .expect("strong local should exist")
        .id;
    let class = match main.locals[strong.0].ty {
        Type::WritableSharedReference(doriac::mir::WritableSharedPayload::Class(class)) => class,
        other => panic!("expected writable class handle, got {other}"),
    };
    main.locals[strong.0].ty = Type::SharedReference(class);

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("writable retain must reject a readonly-family local");
    assert!(error.message.contains("mismatched shared-handle rvalue"));
}

#[test]
fn shared_validator_rejects_nullable_access_family_mismatches() {
    let mut program =
        doriac::lower_source_to_mir("nullable-access-family.doria", nullable_access_source())
            .expect("valid nullable access should lower");
    let main = program
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .expect("main should exist");
    let access = main
        .locals
        .iter_mut()
        .find(|local| local.name == "access")
        .expect("access local should exist");
    let Type::NullableReadonlySharedReferenceAccess(payload) = access.ty else {
        panic!("expected nullable readonly access, got {}", access.ty);
    };
    access.ty = Type::NullableWritableSharedReferenceAccess(payload);

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("nullable access families must match exactly");
    assert!(error.message.contains("mismatched shared-handle rvalue"));
}

#[test]
fn shared_validator_requires_presence_proof_for_nullable_access_unwraps() {
    let mut program =
        doriac::lower_source_to_mir("nullable-access-proof.doria", nullable_access_source())
            .expect("valid nullable access should lower");
    let main = program
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .expect("main should exist");
    let branch =
        main.blocks
            .iter_mut()
            .find(|block| {
                matches!(
                    block.terminator,
                    Terminator::Branch {
                        condition:
                            doriac::mir::BoolExpression::NullableSharedReferenceAccessIsPresent(_),
                        ..
                    }
                )
            })
            .expect("nullable access presence branch should exist");
    let Terminator::Branch { condition, .. } = &mut branch.terminator else {
        unreachable!("branch was selected above");
    };
    *condition = doriac::mir::BoolExpression::Use {
        operand: Operand::Scalar(ScalarValue::Bool(true)),
    };

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("nullable access unwraps require a dominating presence proof");
    assert!(error
        .message
        .contains("without a dominating presence proof"));
}

#[test]
fn shared_validator_rejects_noncanonical_bytes_storage() {
    let mut program = valid_void_program();
    program.collection_types.push(CollectionType {
        id: CollectionTypeId(0),
        kind: CollectionKind::Bytes,
        key: None,
        value: Type::Scalar(ScalarType::Integer(IntegerType::Int64)),
        comparator: None,
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("Bytes must always use the packed uint8 element contract");
    assert!(error.message.contains("Bytes collection"));
    assert!(error.message.contains("packed uint8"));
}

#[test]
fn shared_validator_enforces_stage26_collection_capabilities() {
    for (name, collection, expected) in [
        (
            "missing sorted comparator",
            CollectionType {
                id: CollectionTypeId(0),
                kind: CollectionKind::SortedSet,
                key: None,
                value: Type::Scalar(ScalarType::Integer(IntegerType::Int64)),
                comparator: None,
            },
            "comparator identity",
        ),
        (
            "float sorted comparator",
            CollectionType {
                id: CollectionTypeId(0),
                kind: CollectionKind::PriorityQueue,
                key: None,
                value: Type::Scalar(ScalarType::Float(FloatType::Float64)),
                comparator: Some(CollectionComparator::SignedInteger(64)),
            },
            "comparator identity",
        ),
        (
            "deque comparator",
            CollectionType {
                id: CollectionTypeId(0),
                kind: CollectionKind::Deque,
                key: None,
                value: Type::Scalar(ScalarType::Integer(IntegerType::Int64)),
                comparator: Some(CollectionComparator::SignedInteger(64)),
            },
            "must not carry a comparator",
        ),
    ] {
        let mut program = valid_void_program();
        program.collection_types.push(collection);
        let error = doriac::mir_validation::validate_program(&program)
            .expect_err(&format!("{name} must be rejected"));
        assert!(
            error.message.contains(expected),
            "{name}: {}",
            error.message
        );
    }
}

fn nullable_access_source() -> &'static str {
    r#"
class Value
{
    int $number = 1;
}

function main(): void
{
    ?WritableSharedReference<Value> $source = null;
    ?ReadonlySharedReferenceAccess<Value> $access =
        $source?->acquireReadonlyAccess();
    if ($access != null) {
        echo "{$access->number}";
    }
}
"#
}

#[test]
fn shared_validator_enforces_sequence_fill_shape() {
    let mut program = valid_void_program();
    program.collection_types.push(CollectionType {
        id: CollectionTypeId(0),
        kind: CollectionKind::List,
        key: None,
        value: Type::Scalar(ScalarType::Integer(IntegerType::Int64)),
        comparator: None,
    });
    program.functions[0].locals.push(Local {
        id: LocalId(0),
        name: "values".to_string(),
        ty: Type::Collection(CollectionTypeId(0)),
        writable: false,
        owned: true,
        synthetic: false,
    });
    program.functions[0].blocks[0].statements = vec![
        Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Collection(CollectionExpression::Fill {
                collection: CollectionTypeId(0),
                value: Box::new(Rvalue::Value(ValueExpression::Integer(
                    IntegerExpression::constant(
                        IntegerValue::from_i128(IntegerType::Int64, 7).expect("valid int"),
                    ),
                ))),
                count: Box::new(IntegerExpression::constant(
                    IntegerValue::from_i128(IntegerType::Int64, 3).expect("valid int"),
                )),
                count_span: doriac::source::Span::default(),
            }),
        },
        Statement::DropCollection {
            local: LocalId(0),
            collection: CollectionTypeId(0),
        },
    ];
    doriac::mir_validation::validate_program(&program)
        .expect("well-typed sequence fill MIR should validate");

    let mut keyed = program.clone();
    keyed.collection_types[0].kind = CollectionKind::Dictionary;
    keyed.collection_types[0].key = Some(Type::String);
    let error = doriac::mir_validation::validate_program(&keyed)
        .expect_err("fill MIR must reject keyed destinations");
    assert!(error
        .message
        .contains("collection fill destination is not a sequence"));

    let mut narrow_count = program;
    let Statement::AssignLocal {
        value: Rvalue::Collection(CollectionExpression::Fill { count, .. }),
        ..
    } = &mut narrow_count.functions[0].blocks[0].statements[0]
    else {
        panic!("expected fill assignment");
    };
    **count = IntegerExpression::constant(
        IntegerValue::from_i128(IntegerType::Int32, 3).expect("valid int32"),
    );
    let error = doriac::mir_validation::validate_program(&narrow_count)
        .expect_err("fill MIR must reject non-int counts");
    assert!(error.message.contains("collection fill count is not int"));
}

#[test]
fn shared_validator_requires_exact_is_proof_for_mixed_payload_reads() {
    let mut program = valid_void_program();
    program.functions[0].return_type =
        ReturnType::Value(Type::Scalar(ScalarType::Integer(IntegerType::Int64)));
    program.functions[0].locals.push(Local {
        id: LocalId(0),
        name: "value".to_string(),
        ty: Type::Mixed,
        writable: false,
        owned: true,
        synthetic: false,
    });
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Mixed(doriac::mir::MixedExpression::BoxValue(
                ValueExpression::Integer(doriac::mir::IntegerExpression::Use {
                    ty: IntegerType::Int64,
                    operand: Operand::Scalar(ScalarValue::Integer(IntegerValue::from_bits(
                        IntegerType::Int64,
                        1,
                    ))),
                }),
            )),
        });
    program.functions[0].blocks[0].terminator = Terminator::Return(Rvalue::Value(
        ValueExpression::Integer(doriac::mir::IntegerExpression::Use {
            ty: IntegerType::Int64,
            operand: Operand::MixedPayload {
                mixed: LocalId(0),
                tag: doriac::mir::MixedTag::Integer(IntegerType::Int64),
            },
        }),
    ));

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("mixed payload reads require a dominating exact is proof");
    assert!(error
        .message
        .contains("without a dominating exact `is` proof"));
}

#[test]
fn shared_validator_limits_explicit_string_drops_to_synthetic_temporaries() {
    let mut program = valid_void_program();
    program.functions[0].locals.push(Local {
        id: LocalId(0),
        name: "_string0".to_string(),
        ty: Type::String,
        writable: false,
        synthetic: true,
        owned: false,
    });
    program.functions[0].blocks[0].statements = vec![
        Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::String(StringExpression::Literal("path".to_string())),
        },
        Statement::DropString { local: LocalId(0) },
    ];
    doriac::mir_validation::validate_program(&program)
        .expect("a synthetic string temporary may be released explicitly");

    program.functions[0].locals[0].synthetic = false;
    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("an ordinary Doria string local must not be explicitly dropped");
    assert!(error
        .message
        .contains("string drop must reference a synthetic string local"));
}

#[test]
fn shared_validator_rejects_string_main_return() {
    let mut program = valid_void_program();
    program.functions[0].return_type = ReturnType::Value(Type::String);
    program.functions[0].blocks[0].terminator =
        Terminator::Return(Rvalue::String(StringExpression::Literal("bad".to_string())));

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("main returning string must be rejected");
    assert!(error
        .message
        .contains("entry function must return void or int/int64"));
}

#[test]
fn shared_validator_rejects_owned_nullable_class_statics() {
    let mut program = class_program();
    program.statics.push(StaticProperty {
        id: StaticId(0),
        class: ClassId(0),
        name: "cached".to_string(),
        ty: Type::NullableClass(ClassId(1)),
        writable: false,
        initializer: StaticValue::Null,
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("owned nullable statics must wait for static lifetime support");
    assert!(error
        .message
        .contains("owned class type before owned static lifetime support"));
}

#[test]
fn shared_validator_requires_control_flow_proof_for_nullable_class_unwraps() {
    let mut invalid = class_program();
    invalid.functions[0].locals = vec![
        nullable_class_local(0, ClassId(0)),
        class_local(1, ClassId(0)),
    ];
    invalid.functions[0].blocks[0].statements = vec![
        Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::NullableClass(NullableClassExpression::Null(ClassId(0))),
        },
        Statement::AssignLocal {
            target: LocalId(1),
            value: Rvalue::Class(ClassExpression::NullableLocalAssumeNonNull {
                class: ClassId(0),
                local: LocalId(0),
                transfer: true,
            }),
        },
    ];

    let error = doriac::mir_validation::validate_program(&invalid)
        .expect_err("an unchecked nullable class unwrap must be rejected");
    assert!(error
        .message
        .contains("without a dominating presence proof"));

    let mut valid = class_program();
    valid.functions[0].locals = vec![
        nullable_class_local(0, ClassId(0)),
        class_local(1, ClassId(0)),
    ];
    valid.functions[0].blocks = vec![
        BasicBlock {
            id: BlockId(0),
            statements: vec![Statement::AssignLocal {
                target: LocalId(0),
                value: Rvalue::NullableClass(NullableClassExpression::Null(ClassId(0))),
            }],
            terminator: Terminator::Branch {
                condition: doriac::mir::BoolExpression::NullableClassIsPresent(Box::new(
                    NullableClassExpression::Local {
                        class: ClassId(0),
                        local: LocalId(0),
                        transfer: false,
                    },
                )),
                then_block: BlockId(1),
                else_block: BlockId(2),
            },
        },
        BasicBlock {
            id: BlockId(1),
            statements: vec![Statement::AssignLocal {
                target: LocalId(1),
                value: Rvalue::Class(ClassExpression::NullableLocalAssumeNonNull {
                    class: ClassId(0),
                    local: LocalId(0),
                    transfer: true,
                }),
            }],
            terminator: Terminator::ReturnVoid,
        },
        BasicBlock {
            id: BlockId(2),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        },
    ];

    doriac::mir_validation::validate_program(&valid)
        .expect("a dominating presence branch must authorize the unwrap");
}

#[test]
fn shared_validator_rejects_scalar_string_assignment_mixing() {
    let mut program = valid_void_program();
    program.functions[0].locals.push(Local {
        id: LocalId(0),
        name: "value".to_string(),
        ty: Type::String,
        writable: true,
        synthetic: false,
        owned: false,
    });
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Value(ValueExpression::Integer(
                doriac::mir::IntegerExpression::constant(IntegerValue::from_bits(
                    IntegerType::Int64,
                    1,
                )),
            )),
        });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("scalar assigned to string local must be rejected");
    assert!(error.message.contains("string local local0 receives"));
    assert!(error.message.contains("rvalue"));
}

#[test]
fn shared_validator_rejects_nullable_string_main_return() {
    let mut program = valid_void_program();
    program.functions[0].return_type = ReturnType::Value(Type::NullableString);
    program.functions[0].blocks[0].terminator =
        Terminator::Return(Rvalue::NullableString(NullableStringExpression::Null));
    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("main returning nullable string must be rejected");
    assert!(error
        .message
        .contains("entry function must return void or int/int64"));
}

#[test]
fn shared_validator_rejects_nullable_rvalue_assigned_to_plain_string() {
    let mut program = valid_void_program();
    program.functions[0].locals.push(Local {
        id: LocalId(0),
        name: "value".to_string(),
        ty: Type::String,
        writable: true,
        synthetic: false,
        owned: false,
    });
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::NullableString(NullableStringExpression::Null),
        });
    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("nullable rvalue must not enter a plain string local");
    assert!(error.message.contains("string local"));
    assert!(error.message.contains("nullable-string rvalue"));
}

#[test]
fn shared_validator_rejects_invalid_format_index_and_argument_type() {
    for format in [
        FormatExpression {
            pieces: vec![FormatPiece::Argument {
                index: 1,
                spec: decimal_spec(),
            }],
            arguments: vec![],
        },
        FormatExpression {
            pieces: vec![FormatPiece::Argument {
                index: 0,
                spec: decimal_spec(),
            }],
            arguments: vec![FormatArgument::String(StringExpression::Literal(
                "wrong".to_string(),
            ))],
        },
    ] {
        let mut program = valid_void_program();
        program.functions[0].blocks[0]
            .statements
            .push(Statement::Printf(format));
        doriac::mir_validation::validate_program(&program)
            .expect_err("invalid checked format MIR must be rejected");
    }
}

#[test]
fn shared_validator_preserves_implicit_display_borrows_across_format_arguments() {
    let mut program = class_program();
    let label = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    program.classes[0].properties.push(Property {
        id: label,
        name: "label".to_string(),
        ty: Type::String,
        writable: false,
        promoted: false,
    });
    program.classes[0].layout = compute_class_layout(ClassId(0), [(label, FieldType::String)], 8);
    let mut receiver = class_local(0, ClassId(0));
    receiver.writable = true;
    program.functions[0].locals.push(receiver);
    let receiver_argument = || {
        Rvalue::Class(ClassExpression::Local {
            class: ClassId(0),
            local: LocalId(0),
            transfer: false,
        })
    };
    let display_call = StringExpression::Call {
        function: FunctionId(1),
        args: vec![receiver_argument()],
    };
    let update_call = StringExpression::Call {
        function: FunctionId(2),
        args: vec![receiver_argument()],
    };
    program.functions[0].blocks[0]
        .statements
        .push(Statement::Printf(FormatExpression {
            pieces: vec![
                FormatPiece::Argument {
                    index: 0,
                    spec: display_spec(),
                },
                FormatPiece::Argument {
                    index: 1,
                    spec: display_spec(),
                },
            ],
            arguments: vec![
                FormatArgument::ClassDisplay(display_call.clone()),
                FormatArgument::String(update_call),
            ],
        }));
    for (id, name, writable) in [
        (FunctionId(1), "display", false),
        (FunctionId(2), "update", true),
    ] {
        let mut parameter = borrowed_class_local(0, ClassId(0));
        parameter.writable = writable;
        program.functions.push(Function {
            id,
            name: name.to_string(),
            source_span: Default::default(),
            method: None,
            receiver_mode: None,
            params: vec![LocalId(0)],
            return_type: ReturnType::Value(Type::String),
            locals: vec![parameter],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                statements: vec![],
                terminator: Terminator::Return(Rvalue::String(StringExpression::Literal(
                    name.to_string(),
                ))),
            }],
            entry_block: BlockId(0),
        });
    }

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("class display must remain borrowed through later format arguments");
    assert!(error
        .message
        .contains("takes overlapping writable borrows of class local local0"));

    program.functions[2].locals[0].writable = false;
    doriac::mir_validation::validate_program(&program)
        .expect("multiple readonly format borrows do not conflict");

    program.functions[2].locals[0].writable = true;
    let Statement::Printf(format) = &mut program.functions[0].blocks[0].statements[0] else {
        unreachable!()
    };
    format.arguments[0] = FormatArgument::String(display_call);
    doriac::mir_validation::validate_program(&program).expect(
        "an explicit string-producing call ends its receiver borrow before the next argument",
    );

    let Statement::Printf(format) = &mut program.functions[0].blocks[0].statements[0] else {
        unreachable!()
    };
    format.arguments[0] = FormatArgument::ClassDisplay(StringExpression::Call {
        function: FunctionId(1),
        args: vec![Rvalue::Class(ClassExpression::New {
            class: ClassId(0),
            properties: vec![PropertyValue {
                property: label,
                source: PropertyValueSource::Expression(Rvalue::String(StringExpression::Literal(
                    "temporary".to_string(),
                ))),
            }],
            constructor: None,
            args: vec![],
        })],
    });
    doriac::mir_validation::validate_program(&program)
        .expect("displaying an owned temporary does not borrow the later argument's owner");

    let Statement::Printf(format) = &mut program.functions[0].blocks[0].statements[0] else {
        unreachable!()
    };
    format.arguments[0] = FormatArgument::String(StringExpression::Property {
        object: LocalId(0),
        property: label,
    });
    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a format property read must remain live through later arguments");
    assert!(error
        .message
        .contains("takes overlapping writable borrows of class local local0"));

    let Statement::Printf(format) = &mut program.functions[0].blocks[0].statements[0] else {
        unreachable!()
    };
    format.pieces.swap(0, 1);
    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("noncanonical format evaluation order must be rejected");
    assert!(error.message.contains("canonical evaluation order"));
}

#[test]
fn shared_validator_requires_class_calls_to_return_the_declared_class() {
    let mut program = class_program();
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Class(ClassExpression::Call {
                class: ClassId(0),
                function: FunctionId(1),
                args: vec![],
                return_borrow: None,
            }),
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "makeOther".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![],
        return_type: ReturnType::Value(Type::Class(ClassId(1))),
        locals: vec![class_local(0, ClassId(1))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Return(Rvalue::Class(ClassExpression::Local {
                class: ClassId(1),
                local: LocalId(0),
                transfer: true,
            })),
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("class calls cannot lie about their return class");
    assert!(error
        .message
        .contains("class#0 call targets a function with another return type"));
}

#[test]
fn shared_validator_skips_the_implicit_constructor_receiver() {
    let mut program = class_program();
    program.classes[0].constructor = Some(FunctionId(1));
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Class(ClassExpression::New {
                class: ClassId(0),
                properties: vec![],
                constructor: Some(FunctionId(1)),
                args: vec![Rvalue::String(StringExpression::Literal(
                    "value".to_string(),
                ))],
            }),
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "Message::__construct".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![
            borrowed_class_local(0, ClassId(0)),
            Local {
                id: LocalId(1),
                name: "text".to_string(),
                ty: Type::String,
                writable: false,
                synthetic: false,
                owned: false,
            },
        ],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    doriac::mir_validation::validate_program(&program)
        .expect("source constructor arguments exclude the synthetic receiver");
}

#[test]
fn shared_validator_requires_promoted_class_arguments_to_transfer_ownership() {
    let mut program = class_program();
    let child = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    program.classes[0].properties = vec![Property {
        id: child,
        name: "child".to_string(),
        ty: Type::Class(ClassId(1)),
        writable: false,
        promoted: true,
    }];
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [(child, FieldType::Class(ClassId(1)))],
        std::mem::size_of::<usize>() as u32,
    );
    program.classes[0].constructor = Some(FunctionId(1));
    program.functions[0].locals = vec![class_local(0, ClassId(0)), class_local(1, ClassId(1))];
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Class(ClassExpression::New {
                class: ClassId(0),
                properties: vec![PropertyValue {
                    property: child,
                    source: PropertyValueSource::ConstructorArgument(0),
                }],
                constructor: Some(FunctionId(1)),
                args: vec![Rvalue::Class(ClassExpression::Local {
                    class: ClassId(1),
                    local: LocalId(1),
                    transfer: false,
                })],
            }),
        });
    let mut borrowed_child = class_local(1, ClassId(1));
    borrowed_child.owned = false;
    program.functions.push(Function {
        id: FunctionId(1),
        name: "Parent::__construct".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![borrowed_class_local(0, ClassId(0)), borrowed_child],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a promoted class property must receive ownership");
    assert!(error
        .message
        .contains("argument 1 receives borrowed class local local1"));
}

#[test]
fn shared_validator_rejects_borrowing_and_transferring_one_class_local_in_a_call() {
    let mut program = class_program();
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::CallVoid {
            span: Default::default(),
            function: FunctionId(1),
            args: vec![
                Rvalue::Class(ClassExpression::Local {
                    class: ClassId(0),
                    local: LocalId(0),
                    transfer: false,
                }),
                Rvalue::Class(ClassExpression::Local {
                    class: ClassId(0),
                    local: LocalId(0),
                    transfer: true,
                }),
            ],
        });
    let mut borrowed = class_local(0, ClassId(0));
    borrowed.owned = false;
    program.functions.push(Function {
        id: FunctionId(1),
        name: "borrowAndTake".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![borrowed, class_local(1, ClassId(0))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a class local cannot be borrowed and transferred by one call");
    assert!(error
        .message
        .contains("both borrows and transfers class local local0"));
}

#[test]
fn shared_validator_enforces_writable_class_argument_rules() {
    let mut program = class_program();
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::CallVoid {
            span: Default::default(),
            function: FunctionId(1),
            args: vec![Rvalue::Class(ClassExpression::Local {
                class: ClassId(0),
                local: LocalId(0),
                transfer: false,
            })],
        });
    let mut parameter = borrowed_class_local(0, ClassId(0));
    parameter.writable = true;
    program.functions.push(Function {
        id: FunctionId(1),
        name: "mutate".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Void,
        locals: vec![parameter],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("readonly class arguments cannot satisfy writable parameters");
    assert!(error.message.contains("requires a writable class value"));

    program.functions[0].locals[0].writable = true;
    doriac::mir_validation::validate_program(&program)
        .expect("a writable class argument should satisfy a writable parameter");

    let Statement::CallVoid { args, .. } = &mut program.functions[0].blocks[0].statements[0] else {
        unreachable!("the fixture contains a call")
    };
    args.push(Rvalue::Class(ClassExpression::Local {
        class: ClassId(0),
        local: LocalId(0),
        transfer: false,
    }));
    program.functions[1].params.push(LocalId(1));
    program.functions[1]
        .locals
        .push(borrowed_class_local(1, ClassId(0)));
    for (left_writable, right_writable) in [(true, true), (true, false), (false, true)] {
        program.functions[1].locals[0].writable = left_writable;
        program.functions[1].locals[1].writable = right_writable;
        let error = doriac::mir_validation::validate_program(&program)
            .expect_err("a writable borrow cannot overlap another borrow in one call");
        assert!(error
            .message
            .contains("takes overlapping writable borrows of class local local0"));
    }

    program.functions[1].locals[0].writable = false;
    program.functions[1].locals[1].writable = false;
    doriac::mir_validation::validate_program(&program)
        .expect("multiple readonly borrows of one class local should remain valid");
}

#[test]
fn shared_validator_does_not_keep_nested_argument_borrows_alive() {
    let mut program = class_program();
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::CallVoid {
            span: Default::default(),
            function: FunctionId(2),
            args: vec![
                Rvalue::String(StringExpression::Call {
                    function: FunctionId(1),
                    args: vec![Rvalue::Class(ClassExpression::Local {
                        class: ClassId(0),
                        local: LocalId(0),
                        transfer: false,
                    })],
                }),
                Rvalue::Class(ClassExpression::Local {
                    class: ClassId(0),
                    local: LocalId(0),
                    transfer: true,
                }),
            ],
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "label".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Value(Type::String),
        locals: vec![borrowed_class_local(0, ClassId(0))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Return(Rvalue::String(StringExpression::Literal(
                "box".to_string(),
            ))),
        }],
        entry_block: BlockId(0),
    });
    program.functions.push(Function {
        id: FunctionId(2),
        name: "sink".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![
            Local {
                id: LocalId(0),
                name: "label".to_string(),
                ty: Type::String,
                writable: false,
                owned: false,
                synthetic: false,
            },
            class_local(1, ClassId(0)),
        ],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    doriac::mir_validation::validate_program(&program)
        .expect("a nested borrow should end before the next outer argument");
}

#[test]
fn shared_validator_preserves_constant_boolean_move_reachability() {
    let mut program = class_program();
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks = vec![
        BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Branch {
                condition: doriac::mir::BoolExpression::Binary {
                    op: doriac::mir::BoolBinaryOp::And,
                    left: Box::new(doriac::mir::BoolExpression::Use {
                        operand: Operand::Scalar(ScalarValue::Bool(false)),
                    }),
                    right: Box::new(doriac::mir::BoolExpression::Call {
                        function: FunctionId(1),
                        args: vec![Rvalue::Class(ClassExpression::Local {
                            class: ClassId(0),
                            local: LocalId(0),
                            transfer: true,
                        })],
                    }),
                },
                then_block: BlockId(1),
                else_block: BlockId(2),
            },
        },
        BasicBlock {
            id: BlockId(1),
            statements: vec![Statement::CallVoid {
                span: Default::default(),
                function: FunctionId(2),
                args: vec![Rvalue::Class(ClassExpression::Local {
                    class: ClassId(0),
                    local: LocalId(0),
                    transfer: true,
                })],
            }],
            terminator: Terminator::Jump(BlockId(3)),
        },
        BasicBlock {
            id: BlockId(2),
            statements: vec![],
            terminator: Terminator::Jump(BlockId(3)),
        },
        BasicBlock {
            id: BlockId(3),
            statements: vec![Statement::CallVoid {
                span: Default::default(),
                function: FunctionId(3),
                args: vec![Rvalue::Class(ClassExpression::Local {
                    class: ClassId(0),
                    local: LocalId(0),
                    transfer: false,
                })],
            }],
            terminator: Terminator::ReturnVoid,
        },
    ];
    program.functions.push(Function {
        id: FunctionId(1),
        name: "probe".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Value(Type::Scalar(ScalarType::Bool)),
        locals: vec![class_local(0, ClassId(0))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Return(Rvalue::Value(ValueExpression::Bool(
                doriac::mir::BoolExpression::Use {
                    operand: Operand::Scalar(ScalarValue::Bool(true)),
                },
            ))),
        }],
        entry_block: BlockId(0),
    });
    program.functions.push(Function {
        id: FunctionId(2),
        name: "consume".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Void,
        locals: vec![class_local(0, ClassId(0))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });
    program.functions.push(Function {
        id: FunctionId(3),
        name: "inspect".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Void,
        locals: vec![borrowed_class_local(0, ClassId(0))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    doriac::mir_validation::validate_program(&program)
        .expect("short-circuited and unreachable transfers must not move the class local");
}

#[test]
fn shared_validator_tracks_nested_transfers_across_outer_call_arguments() {
    let mut program = class_program();
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::CallVoid {
            span: Default::default(),
            function: FunctionId(1),
            args: vec![
                Rvalue::Class(ClassExpression::Local {
                    class: ClassId(0),
                    local: LocalId(0),
                    transfer: false,
                }),
                Rvalue::String(StringExpression::Call {
                    function: FunctionId(2),
                    args: vec![Rvalue::Class(ClassExpression::Local {
                        class: ClassId(0),
                        local: LocalId(0),
                        transfer: true,
                    })],
                }),
            ],
        });

    let mut borrowed = class_local(0, ClassId(0));
    borrowed.owned = false;
    program.functions.push(Function {
        id: FunctionId(1),
        name: "inspectWithLabel".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![
            borrowed,
            Local {
                id: LocalId(1),
                name: "label".to_string(),
                ty: Type::String,
                writable: false,
                synthetic: false,
                owned: false,
            },
        ],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });
    program.functions.push(Function {
        id: FunctionId(2),
        name: "consumeAndLabel".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Value(Type::String),
        locals: vec![class_local(0, ClassId(0))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Unreachable,
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("nested transfers must conflict with persistent outer-call borrows");
    assert!(error
        .message
        .contains("both borrows and transfers class local local0"));
}

#[test]
fn shared_validator_tracks_property_borrows_across_outer_call_arguments() {
    let mut program = class_program();
    let label = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    program.classes[0].properties.push(Property {
        id: label,
        name: "label".to_string(),
        ty: Type::Scalar(ScalarType::Integer(IntegerType::Int64)),
        writable: false,
        promoted: false,
    });
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [(label, FieldType::Integer(IntegerType::Int64))],
        8,
    );
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::CallVoid {
            span: Default::default(),
            function: FunctionId(1),
            args: vec![
                Rvalue::Class(ClassExpression::Local {
                    class: ClassId(0),
                    local: LocalId(0),
                    transfer: true,
                }),
                Rvalue::Value(ValueExpression::Integer(
                    doriac::mir::IntegerExpression::Use {
                        ty: IntegerType::Int64,
                        operand: Operand::Property {
                            object: LocalId(0),
                            property: label,
                        },
                    },
                )),
            ],
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "takeWithLabel".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![
            class_local(0, ClassId(0)),
            Local {
                id: LocalId(1),
                name: "label".to_string(),
                ty: Type::Scalar(ScalarType::Integer(IntegerType::Int64)),
                writable: false,
                synthetic: false,
                owned: false,
            },
        ],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a call cannot read a property after transferring its object");
    assert!(error
        .message
        .contains("both borrows and transfers class local local0"));

    program.functions[0].locals[0].writable = true;
    let Statement::CallVoid { args, .. } = &mut program.functions[0].blocks[0].statements[0] else {
        unreachable!("the fixture contains a call")
    };
    args.reverse();
    let Rvalue::Class(ClassExpression::Local { transfer, .. }) = &mut args[1] else {
        unreachable!("the fixture's second argument is the class local")
    };
    *transfer = false;
    program.functions[1].locals = vec![
        Local {
            id: LocalId(0),
            name: "label".to_string(),
            ty: Type::Scalar(ScalarType::Integer(IntegerType::Int64)),
            writable: false,
            synthetic: false,
            owned: false,
        },
        borrowed_class_local(1, ClassId(0)),
    ];
    program.functions[1].locals[1].writable = true;

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a property read cannot overlap a later writable borrow");
    assert!(error
        .message
        .contains("takes overlapping writable borrows of class local local0"));

    program.functions[1].locals[1].writable = false;
    doriac::mir_validation::validate_program(&program)
        .expect("a property read may overlap a later readonly borrow");

    let property_read = doriac::mir::IntegerExpression::Use {
        ty: IntegerType::Int64,
        operand: Operand::Property {
            object: LocalId(0),
            property: label,
        },
    };
    let writable_call = doriac::mir::IntegerExpression::Call {
        ty: IntegerType::Int64,
        function: FunctionId(2),
        args: vec![Rvalue::Class(ClassExpression::Local {
            class: ClassId(0),
            local: LocalId(0),
            transfer: false,
        })],
    };
    program.functions[0].blocks[0].statements[0] = Statement::CallVoid {
        span: Default::default(),
        function: FunctionId(1),
        args: vec![Rvalue::Value(ValueExpression::Integer(
            doriac::mir::IntegerExpression::Binary {
                span: Default::default(),
                right_span: Default::default(),
                ty: IntegerType::Int64,
                op: doriac::mir::IntegerBinaryOp::Add,
                left: Box::new(property_read),
                right: Box::new(writable_call),
            },
        ))],
    };
    program.functions[1].params = vec![LocalId(0)];
    program.functions[1].locals.truncate(1);
    let mut writable = borrowed_class_local(0, ClassId(0));
    writable.writable = true;
    program.functions.push(Function {
        id: FunctionId(2),
        name: "update".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Value(Type::Scalar(ScalarType::Integer(IntegerType::Int64))),
        locals: vec![writable],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Return(Rvalue::Value(ValueExpression::Integer(
                doriac::mir::IntegerExpression::constant(IntegerValue::from_bits(
                    IntegerType::Int64,
                    1,
                )),
            ))),
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a nested writable call cannot overlap an earlier property read");
    assert!(error
        .message
        .contains("takes overlapping writable borrows of class local local0"));
}

#[test]
fn shared_validator_rejects_invalid_class_new_property_sources() {
    let property = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    for value in [
        PropertyValue {
            property: PropertyId {
                class: ClassId(1),
                index: 0,
            },
            source: PropertyValueSource::ConstructorArgument(0),
        },
        PropertyValue {
            property,
            source: PropertyValueSource::Expression(Rvalue::Value(ValueExpression::Integer(
                doriac::mir::IntegerExpression::constant(IntegerValue::from_bits(
                    IntegerType::Int64,
                    1,
                )),
            ))),
        },
        PropertyValue {
            property,
            source: PropertyValueSource::ConstructorArgument(99),
        },
    ] {
        let mut program = class_new_program();
        let Statement::AssignLocal {
            value: Rvalue::Class(ClassExpression::New { properties, .. }),
            ..
        } = &mut program.functions[0].blocks[0].statements[0]
        else {
            panic!("class new fixture");
        };
        properties.push(value);
        doriac::mir_validation::validate_program(&program)
            .expect_err("invalid class property initialization must be rejected");
    }

    let mut valid = class_new_program();
    let Statement::AssignLocal {
        value: Rvalue::Class(ClassExpression::New { properties, .. }),
        ..
    } = &mut valid.functions[0].blocks[0].statements[0]
    else {
        panic!("class new fixture");
    };
    properties.push(PropertyValue {
        property,
        source: PropertyValueSource::ConstructorArgument(0),
    });
    doriac::mir_validation::validate_program(&valid)
        .expect("matching constructor property source should validate");
}

#[test]
fn shared_validator_rejects_reusing_a_moved_constructor_argument() {
    let mut program = class_program();
    let first = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    let second = PropertyId {
        class: ClassId(0),
        index: 1,
    };
    program.classes[0].properties = vec![
        Property {
            id: first,
            name: "first".to_string(),
            ty: Type::Class(ClassId(1)),
            writable: false,
            promoted: true,
        },
        Property {
            id: second,
            name: "second".to_string(),
            ty: Type::Class(ClassId(1)),
            writable: false,
            promoted: true,
        },
    ];
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [
            (first, FieldType::Class(ClassId(1))),
            (second, FieldType::Class(ClassId(1))),
        ],
        std::mem::size_of::<usize>() as u32,
    );
    program.classes[0].constructor = Some(FunctionId(1));
    program.functions[0].locals = vec![class_local(0, ClassId(0)), class_local(1, ClassId(1))];
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Class(ClassExpression::New {
                class: ClassId(0),
                properties: vec![
                    PropertyValue {
                        property: first,
                        source: PropertyValueSource::ConstructorArgument(0),
                    },
                    PropertyValue {
                        property: second,
                        source: PropertyValueSource::ConstructorArgument(0),
                    },
                ],
                constructor: Some(FunctionId(1)),
                args: vec![Rvalue::Class(ClassExpression::Local {
                    class: ClassId(1),
                    local: LocalId(1),
                    transfer: true,
                })],
            }),
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "Pair::__construct".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![
            borrowed_class_local(0, ClassId(0)),
            class_local(1, ClassId(1)),
        ],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("one class owner cannot initialize multiple properties");
    assert!(error
        .message
        .contains("gives constructor argument 0 to more than one property"));
}

#[test]
fn shared_validator_rejects_reusing_a_class_local_for_properties() {
    let mut program = class_program();
    let first = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    let second = PropertyId {
        class: ClassId(0),
        index: 1,
    };
    program.classes[0].properties = vec![
        Property {
            id: first,
            name: "first".to_string(),
            ty: Type::Class(ClassId(1)),
            writable: false,
            promoted: false,
        },
        Property {
            id: second,
            name: "second".to_string(),
            ty: Type::Class(ClassId(1)),
            writable: false,
            promoted: false,
        },
    ];
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [
            (first, FieldType::Class(ClassId(1))),
            (second, FieldType::Class(ClassId(1))),
        ],
        std::mem::size_of::<usize>() as u32,
    );
    program.functions[0].locals = vec![class_local(0, ClassId(0)), class_local(1, ClassId(1))];
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Class(ClassExpression::New {
                class: ClassId(0),
                properties: vec![
                    PropertyValue {
                        property: first,
                        source: PropertyValueSource::Expression(Rvalue::Class(
                            ClassExpression::Local {
                                class: ClassId(1),
                                local: LocalId(1),
                                transfer: true,
                            },
                        )),
                    },
                    PropertyValue {
                        property: second,
                        source: PropertyValueSource::Expression(Rvalue::Class(
                            ClassExpression::Local {
                                class: ClassId(1),
                                local: LocalId(1),
                                transfer: true,
                            },
                        )),
                    },
                ],
                constructor: None,
                args: vec![],
            }),
        });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("one class local cannot initialize multiple properties");
    assert!(error
        .message
        .contains("transfers class local local1 more than once"));
}

#[test]
fn shared_validator_tracks_nested_transfers_across_property_initializers() {
    let mut program = class_program();
    let first = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    let second = PropertyId {
        class: ClassId(0),
        index: 1,
    };
    program.classes[0].properties = vec![
        Property {
            id: first,
            name: "first".to_string(),
            ty: Type::Class(ClassId(1)),
            writable: false,
            promoted: false,
        },
        Property {
            id: second,
            name: "second".to_string(),
            ty: Type::Class(ClassId(1)),
            writable: false,
            promoted: false,
        },
    ];
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [
            (first, FieldType::Class(ClassId(1))),
            (second, FieldType::Class(ClassId(1))),
        ],
        8,
    );
    program.functions[0].locals = vec![class_local(0, ClassId(0)), class_local(1, ClassId(1))];
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Class(ClassExpression::New {
                class: ClassId(0),
                properties: [first, second]
                    .into_iter()
                    .map(|property| PropertyValue {
                        property,
                        source: PropertyValueSource::Expression(Rvalue::Class(
                            ClassExpression::Call {
                                class: ClassId(1),
                                function: FunctionId(1),
                                args: vec![Rvalue::Class(ClassExpression::Local {
                                    class: ClassId(1),
                                    local: LocalId(1),
                                    transfer: true,
                                })],
                                return_borrow: None,
                            },
                        )),
                    })
                    .collect(),
                constructor: None,
                args: vec![],
            }),
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "relay".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Value(Type::Class(ClassId(1))),
        locals: vec![class_local(0, ClassId(1))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Return(Rvalue::Class(ClassExpression::Local {
                class: ClassId(1),
                local: LocalId(0),
                transfer: true,
            })),
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("nested property initializers cannot transfer one owner twice");
    assert!(error
        .message
        .contains("transfers class local local1 more than once"));
}

#[test]
fn shared_validator_rejects_a_promoted_class_owner_also_owned_by_the_constructor_parameter() {
    let mut program = class_program();
    let child = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    program.classes[0].properties = vec![Property {
        id: child,
        name: "child".to_string(),
        ty: Type::Class(ClassId(1)),
        writable: false,
        promoted: true,
    }];
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [(child, FieldType::Class(ClassId(1)))],
        std::mem::size_of::<usize>() as u32,
    );
    program.classes[0].constructor = Some(FunctionId(1));
    program.functions[0].locals = vec![class_local(0, ClassId(0)), class_local(1, ClassId(1))];
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Class(ClassExpression::New {
                class: ClassId(0),
                properties: vec![PropertyValue {
                    property: child,
                    source: PropertyValueSource::ConstructorArgument(0),
                }],
                constructor: Some(FunctionId(1)),
                args: vec![Rvalue::Class(ClassExpression::Local {
                    class: ClassId(1),
                    local: LocalId(1),
                    transfer: true,
                })],
            }),
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "Parent::__construct".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![
            borrowed_class_local(0, ClassId(0)),
            class_local(1, ClassId(1)),
        ],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a promoted class owner cannot also be owned by the constructor parameter");
    assert!(error.message.contains(
        "gives constructor argument 0 to a property and an owning constructor parameter"
    ));
}

#[test]
fn shared_validator_invalidates_promoted_class_aliases_after_property_replacement() {
    let (mut program, child) = promoted_class_alias_program();
    program.functions[1].blocks[0].statements = vec![
        Statement::AssignProperty {
            object: LocalId(0),
            property: child,
            value: Rvalue::Class(ClassExpression::New {
                class: ClassId(1),
                properties: vec![],
                constructor: None,
                args: vec![],
            }),
        },
        Statement::CallVoid {
            span: Default::default(),
            function: FunctionId(2),
            args: vec![Rvalue::Class(ClassExpression::Local {
                class: ClassId(1),
                local: LocalId(1),
                transfer: false,
            })],
        },
    ];

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("replacing a promoted class property must invalidate its parameter alias");
    assert!(error
        .message
        .contains("uses class local local1 after its ownership ended"));
}

#[test]
fn shared_validator_rejects_construction_borrows_after_transfers() {
    let (mut program, _) = promoted_class_alias_program();
    program.classes[0].properties[0].promoted = false;
    let Statement::AssignLocal {
        value: Rvalue::Class(ClassExpression::New {
            properties, args, ..
        }),
        ..
    } = &mut program.functions[0].blocks[0].statements[0]
    else {
        panic!("class new fixture");
    };
    properties[0].source = PropertyValueSource::Expression(Rvalue::Class(ClassExpression::Local {
        class: ClassId(1),
        local: LocalId(1),
        transfer: true,
    }));
    args[0] = Rvalue::Class(ClassExpression::Local {
        class: ClassId(1),
        local: LocalId(1),
        transfer: false,
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("construction cannot borrow an owner after an earlier initializer moved it");
    assert!(error
        .message
        .contains("both borrows and transfers class local local1"));
}

#[test]
fn shared_validator_keeps_initializer_borrows_live_through_constructor_arguments() {
    let mut program = class_new_program();
    let target = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    let source = PropertyId {
        class: ClassId(1),
        index: 0,
    };
    program.classes[0].properties[0].promoted = false;
    program.classes[1].properties = vec![Property {
        id: source,
        name: "value".to_string(),
        ty: Type::String,
        writable: false,
        promoted: false,
    }];
    program.classes[1].layout = compute_class_layout(ClassId(1), [(source, FieldType::String)], 8);
    program.functions[0].locals.push(class_local(1, ClassId(1)));
    let Statement::AssignLocal {
        value: Rvalue::Class(ClassExpression::New {
            properties, args, ..
        }),
        ..
    } = &mut program.functions[0].blocks[0].statements[0]
    else {
        panic!("class new fixture");
    };
    *properties = vec![PropertyValue {
        property: target,
        source: PropertyValueSource::Expression(Rvalue::String(StringExpression::Property {
            object: LocalId(1),
            property: source,
        })),
    }];
    *args = vec![Rvalue::Class(ClassExpression::Local {
        class: ClassId(1),
        local: LocalId(1),
        transfer: true,
    })];
    program.functions[1].locals[1] = class_local(1, ClassId(1));

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("an initializer borrow must prevent a later constructor transfer");
    assert!(error
        .message
        .contains("both borrows and transfers class local local1"));

    let Statement::AssignLocal {
        value: Rvalue::Class(ClassExpression::New { args, .. }),
        ..
    } = &mut program.functions[0].blocks[0].statements[0]
    else {
        panic!("class new fixture");
    };
    let Rvalue::Class(ClassExpression::Local { transfer, .. }) = &mut args[0] else {
        panic!("class argument fixture");
    };
    *transfer = false;
    program.functions[0].locals[1].writable = true;
    program.functions[1].locals[1].owned = false;
    program.functions[1].locals[1].writable = true;

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("an initializer borrow must prevent a later writable constructor borrow");
    assert!(error
        .message
        .contains("takes overlapping writable borrows of class local local1"));
}

#[test]
fn shared_validator_rejects_class_new_with_missing_properties() {
    let program = class_new_program();
    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("class construction must initialize every property");
    assert!(error.message.contains("does not initialize property0"));
}

#[test]
fn shared_validator_requires_class_properties_in_construction_order() {
    let mut program = class_new_program();
    let first = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    let second = PropertyId {
        class: ClassId(0),
        index: 1,
    };
    program.classes[0].properties.push(Property {
        id: second,
        name: "other".to_string(),
        ty: Type::String,
        writable: false,
        promoted: true,
    });
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [(first, FieldType::String), (second, FieldType::String)],
        std::mem::size_of::<usize>() as u32,
    );
    program.functions[1].params.push(LocalId(2));
    program.functions[1].locals.push(Local {
        id: LocalId(2),
        name: "other".to_string(),
        ty: Type::String,
        writable: false,
        synthetic: false,
        owned: false,
    });
    let Statement::AssignLocal {
        value: Rvalue::Class(ClassExpression::New {
            properties, args, ..
        }),
        ..
    } = &mut program.functions[0].blocks[0].statements[0]
    else {
        panic!("class new fixture");
    };
    args.push(Rvalue::String(StringExpression::Literal(
        "other".to_string(),
    )));
    properties.extend([
        PropertyValue {
            property: second,
            source: PropertyValueSource::ConstructorArgument(1),
        },
        PropertyValue {
            property: first,
            source: PropertyValueSource::ConstructorArgument(0),
        },
    ]);

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("property initializers must retain canonical construction order");
    assert!(error.message.contains("out of construction order"));
}

#[test]
fn shared_validator_requires_constructor_body_initializers_on_every_return_path() {
    let mut program = class_program();
    let property = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    program.classes[0].properties.push(Property {
        id: property,
        name: "text".to_string(),
        ty: Type::String,
        writable: false,
        promoted: false,
    });
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [(property, FieldType::String)],
        std::mem::size_of::<usize>() as u32,
    );
    program.classes[0].constructor = Some(FunctionId(1));
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Class(ClassExpression::New {
                class: ClassId(0),
                properties: vec![PropertyValue {
                    property,
                    source: PropertyValueSource::ConstructorBody,
                }],
                constructor: Some(FunctionId(1)),
                args: vec![Rvalue::Value(ValueExpression::Bool(
                    doriac::mir::BoolExpression::Use {
                        operand: Operand::Scalar(ScalarValue::Bool(true)),
                    },
                ))],
            }),
        });
    let mut receiver = class_local(0, ClassId(0));
    receiver.owned = false;
    let condition = Local {
        id: LocalId(1),
        name: "condition".to_string(),
        ty: Type::Scalar(ScalarType::Bool),
        writable: false,
        owned: false,
        synthetic: false,
    };
    program.functions.push(Function {
        id: FunctionId(1),
        name: "Message::__construct".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![receiver, condition],
        blocks: vec![
            BasicBlock {
                id: BlockId(0),
                statements: vec![],
                terminator: Terminator::Branch {
                    condition: doriac::mir::BoolExpression::Use {
                        operand: Operand::Local(LocalId(1)),
                    },
                    then_block: BlockId(1),
                    else_block: BlockId(2),
                },
            },
            BasicBlock {
                id: BlockId(1),
                statements: vec![Statement::AssignProperty {
                    object: LocalId(0),
                    property,
                    value: Rvalue::String(StringExpression::Literal("ready".to_string())),
                }],
                terminator: Terminator::Jump(BlockId(3)),
            },
            BasicBlock {
                id: BlockId(2),
                statements: vec![],
                terminator: Terminator::Jump(BlockId(3)),
            },
            BasicBlock {
                id: BlockId(3),
                statements: vec![],
                terminator: Terminator::ReturnVoid,
            },
        ],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("constructor-body initialization must dominate every normal return");
    assert!(error
        .message
        .contains("can return without initializing property0"));

    program.functions[1].blocks[2]
        .statements
        .push(Statement::AssignProperty {
            object: LocalId(0),
            property,
            value: Rvalue::String(StringExpression::Literal("fallback".to_string())),
        });
    doriac::mir_validation::validate_program(&program)
        .expect("mutually exclusive readonly initialization paths are valid");

    let mut duplicate = program.clone();
    duplicate.functions[1].blocks[1]
        .statements
        .push(Statement::AssignProperty {
            object: LocalId(0),
            property,
            value: Rvalue::String(StringExpression::Literal("twice".to_string())),
        });
    let error = doriac::mir_validation::validate_program(&duplicate)
        .expect_err("readonly initialization twice on one path must be rejected");
    assert!(error.message.contains("more than once on one path"));

    let mut read_before_init = program.clone();
    read_before_init.functions[1].blocks[0]
        .statements
        .push(Statement::EchoString(StringExpression::Property {
            object: LocalId(0),
            property,
        }));
    let error = doriac::mir_validation::validate_program(&read_before_init)
        .expect_err("constructor MIR cannot read an uninitialized property");
    assert!(error.message.contains("reads or exposes property0"));

    let mut unreachable_write = program;
    unreachable_write.functions[1].blocks[2].statements.clear();
    unreachable_write.functions[1].blocks[2].terminator = Terminator::Panic {
        message: StringExpression::Literal("stop".to_string()),
        span: Default::default(),
    };
    unreachable_write.functions[1].blocks.push(BasicBlock {
        id: BlockId(4),
        statements: vec![Statement::AssignProperty {
            object: LocalId(0),
            property,
            value: Rvalue::String(StringExpression::Literal("unreachable".to_string())),
        }],
        terminator: Terminator::ReturnVoid,
    });
    doriac::mir_validation::validate_program(&unreachable_write)
        .expect("an unreachable write neither establishes nor invalidates initialization");
}

#[test]
fn shared_validator_rejects_property_assignments_that_transfer_the_receiver() {
    let mut program = class_program();
    let property = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    program.classes[0].properties.push(Property {
        id: property,
        name: "child".to_string(),
        ty: Type::Class(ClassId(0)),
        writable: true,
        promoted: false,
    });
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [(property, FieldType::Class(ClassId(0)))],
        std::mem::size_of::<usize>() as u32,
    );
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignProperty {
            object: LocalId(0),
            property,
            value: Rvalue::Class(ClassExpression::Local {
                class: ClassId(0),
                local: LocalId(0),
                transfer: true,
            }),
        });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a property assignment cannot consume its receiver before the store");
    assert!(error
        .message
        .contains("assignment to property0 consumes its receiver local0"));
}

#[test]
fn shared_validator_rejects_property_assignment_receiver_borrows_except_the_target() {
    let mut program = class_program();
    let target = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    let other = PropertyId {
        class: ClassId(0),
        index: 1,
    };
    program.classes[0].properties = vec![
        Property {
            id: target,
            name: "target".to_string(),
            ty: Type::String,
            writable: true,
            promoted: false,
        },
        Property {
            id: other,
            name: "other".to_string(),
            ty: Type::String,
            writable: false,
            promoted: false,
        },
    ];
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [(target, FieldType::String), (other, FieldType::String)],
        8,
    );
    let mut receiver = class_local(0, ClassId(0));
    receiver.writable = true;
    program.functions[0].locals.push(receiver);
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignProperty {
            object: LocalId(0),
            property: target,
            value: Rvalue::String(StringExpression::Property {
                object: LocalId(0),
                property: other,
            }),
        });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a property write cannot read another property on its receiver");
    assert!(error.message.contains("borrows its receiver local0"));

    let Statement::AssignProperty { value, .. } = &mut program.functions[0].blocks[0].statements[0]
    else {
        unreachable!("the fixture contains a property assignment")
    };
    *value = Rvalue::String(StringExpression::Property {
        object: LocalId(0),
        property: target,
    });
    doriac::mir_validation::validate_program(&program)
        .expect("an exact-target read remains valid for read-modify-write lowering");

    let mut parameter = borrowed_class_local(0, ClassId(0));
    parameter.writable = true;
    program.functions.push(Function {
        id: FunctionId(1),
        name: "update".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Value(Type::String),
        locals: vec![parameter],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Return(Rvalue::String(StringExpression::Literal(
                "updated".to_string(),
            ))),
        }],
        entry_block: BlockId(0),
    });
    let Statement::AssignProperty { value, .. } = &mut program.functions[0].blocks[0].statements[0]
    else {
        unreachable!("the fixture contains a property assignment")
    };
    *value = Rvalue::String(StringExpression::Call {
        function: FunctionId(1),
        args: vec![Rvalue::Class(ClassExpression::Local {
            class: ClassId(0),
            local: LocalId(0),
            transfer: false,
        })],
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a property write cannot borrow its receiver through a call");
    assert!(error.message.contains("borrows its receiver local0"));
}

#[test]
fn shared_validator_enforces_property_and_receiver_mutability() {
    for (property_writable, receiver_writable, expected) in [
        (false, true, "mutates readonly property0"),
        (true, false, "uses readonly receiver local0"),
    ] {
        let mut program = class_program();
        let property = PropertyId {
            class: ClassId(0),
            index: 0,
        };
        program.classes[0].properties.push(Property {
            id: property,
            name: "text".to_string(),
            ty: Type::String,
            writable: property_writable,
            promoted: false,
        });
        program.classes[0].layout =
            compute_class_layout(ClassId(0), [(property, FieldType::String)], 8);
        let mut receiver = class_local(0, ClassId(0));
        receiver.writable = receiver_writable;
        program.functions[0].locals.push(receiver);
        program.functions[0].blocks[0]
            .statements
            .push(Statement::AssignProperty {
                object: LocalId(0),
                property,
                value: Rvalue::String(StringExpression::Literal("changed".to_string())),
            });

        let error = doriac::mir_validation::validate_program(&program)
            .expect_err("property mutation requires both mutable property and receiver");
        assert!(
            error.message.contains(expected),
            "unexpected error: {error:?}"
        );
    }

    let mut program = class_new_program();
    let property = program.classes[0].properties[0].id;
    let Statement::AssignLocal {
        value: Rvalue::Class(ClassExpression::New { properties, .. }),
        ..
    } = &mut program.functions[0].blocks[0].statements[0]
    else {
        panic!("class new fixture");
    };
    properties.push(PropertyValue {
        property,
        source: PropertyValueSource::ConstructorArgument(0),
    });
    program.functions[1].blocks[0]
        .statements
        .push(Statement::AssignProperty {
            object: LocalId(0),
            property,
            value: Rvalue::String(StringExpression::Local(LocalId(1))),
        });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a readonly promoted property cannot be reassigned by its constructor");
    assert!(error
        .message
        .contains("readonly property0 is initialized before its constructor assigns it"));
}

#[test]
fn shared_validator_rejects_invalid_constructor_successors_without_panicking() {
    let mut program = class_new_program();
    let property = program.classes[0].properties[0].id;
    let Statement::AssignLocal {
        value: Rvalue::Class(ClassExpression::New { properties, .. }),
        ..
    } = &mut program.functions[0].blocks[0].statements[0]
    else {
        panic!("class new fixture");
    };
    properties.push(PropertyValue {
        property,
        source: PropertyValueSource::ConstructorBody,
    });
    program.functions[1].blocks[0].terminator = Terminator::Jump(BlockId(9));

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("an invalid constructor successor must be rejected as malformed MIR");
    assert!(error.message.contains("BlockId block9 does not exist"));
}

#[test]
fn shared_validator_requires_constructors_to_return_void() {
    let mut program = class_new_program();
    program.functions[1].return_type = ReturnType::Value(Type::String);
    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("constructors cannot return values");
    assert!(error.message.contains("constructor") && error.message.contains("return void"));
}

#[test]
fn shared_validator_rejects_inconsistent_class_and_property_tables() {
    let mut wrong_class_slot = class_program();
    wrong_class_slot.classes[0].id = ClassId(1);
    let error = doriac::mir_validation::validate_program(&wrong_class_slot)
        .expect_err("class IDs must match their table slots");
    assert!(error.message.contains("class table slot 0"));

    let mut wrong_property_slot = class_new_program();
    wrong_property_slot.classes[0].properties[0].id.index = 1;
    let error = doriac::mir_validation::validate_program(&wrong_property_slot)
        .expect_err("property IDs must match their table slots");
    assert!(error.message.contains("property slot 0"));

    let mut wrong_layout = class_new_program();
    wrong_layout.classes[0].layout.size += 8;
    let error = doriac::mir_validation::validate_program(&wrong_layout)
        .expect_err("class layouts must be derived from property metadata");
    assert!(error.message.contains("layout does not match"));
}

#[test]
fn shared_validator_rejects_unknown_property_class_references() {
    let mut program = class_program();
    let property = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    program.classes[0].properties.push(Property {
        id: property,
        name: "missing".to_string(),
        ty: Type::Class(ClassId(99)),
        writable: false,
        promoted: false,
    });
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [(property, FieldType::Class(ClassId(99)))],
        std::mem::size_of::<usize>() as u32,
    );

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("property types must reference declared classes");
    assert!(error.message.contains("ClassId class#99 does not exist"));
}

#[test]
fn shared_validator_rejects_unknown_classes_in_function_types() {
    let mut local = valid_void_program();
    local.functions[0].locals.push(class_local(0, ClassId(99)));
    let error = doriac::mir_validation::validate_program(&local)
        .expect_err("local types must reference declared classes");
    assert!(error.message.contains("ClassId class#99 does not exist"));

    let mut parameter = valid_void_program();
    parameter.functions.push(Function {
        id: FunctionId(1),
        name: "missingClassParameter".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Void,
        locals: vec![class_local(0, ClassId(99))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });
    let error = doriac::mir_validation::validate_program(&parameter)
        .expect_err("parameter types must reference declared classes");
    assert!(error.message.contains("ClassId class#99 does not exist"));

    let mut returned = valid_void_program();
    returned.functions.push(Function {
        id: FunctionId(1),
        name: "missingClass".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![],
        return_type: ReturnType::Value(Type::Class(ClassId(99))),
        locals: vec![],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Unreachable,
        }],
        entry_block: BlockId(0),
    });
    let error = doriac::mir_validation::validate_program(&returned)
        .expect_err("return types must reference declared classes");
    assert!(error.message.contains("ClassId class#99 does not exist"));
}

#[test]
fn shared_validator_checks_lifecycle_metadata_even_when_unused() {
    let mut valid = class_program();
    valid.classes[0].destructor = Some(FunctionId(1));
    let mut receiver = class_local(0, ClassId(0));
    receiver.owned = false;
    valid.functions.push(Function {
        id: FunctionId(1),
        name: "Class0::__destruct".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Void,
        locals: vec![receiver],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });
    doriac::mir_validation::validate_program(&valid)
        .expect("well-formed lifecycle metadata should validate");

    let mut missing = valid.clone();
    missing.classes[0].destructor = Some(FunctionId(99));
    doriac::mir_validation::validate_program(&missing)
        .expect_err("lifecycle function IDs must exist");

    let mut wrong_receiver = valid.clone();
    wrong_receiver.functions[1].locals[0].ty = Type::Class(ClassId(1));
    let error = doriac::mir_validation::validate_program(&wrong_receiver)
        .expect_err("lifecycle receivers must use the owning class");
    assert!(error.message.contains("incompatible implicit receiver"));

    let mut owned_receiver = valid.clone();
    owned_receiver.functions[1].locals[0].owned = true;
    let error = doriac::mir_validation::validate_program(&owned_receiver)
        .expect_err("lifecycle receivers must remain borrowed");
    assert!(error.message.contains("implicit receiver as owned"));

    let mut wrong_return = valid;
    wrong_return.functions[1].return_type = ReturnType::Value(Type::String);
    let error = doriac::mir_validation::validate_program(&wrong_return)
        .expect_err("lifecycle functions must return void");
    assert!(error.message.contains("does not return void"));
}

#[test]
fn shared_validator_rejects_transfers_into_borrowed_class_parameters() {
    let mut program = class_program();
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::CallVoid {
            span: Default::default(),
            function: FunctionId(1),
            args: vec![Rvalue::Class(ClassExpression::Local {
                class: ClassId(0),
                local: LocalId(0),
                transfer: true,
            })],
        });
    let mut borrowed = class_local(0, ClassId(0));
    borrowed.owned = false;
    program.functions.push(Function {
        id: FunctionId(1),
        name: "inspect".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Void,
        locals: vec![borrowed],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a transfer cannot masquerade as a borrowed call argument");
    assert!(error
        .message
        .contains("transfers argument 1 into a borrowed parameter"));
}

#[test]
fn shared_validator_rejects_borrows_into_owned_class_parameters() {
    let mut program = class_program();
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::CallVoid {
            span: Default::default(),
            function: FunctionId(1),
            args: vec![Rvalue::Class(ClassExpression::Local {
                class: ClassId(0),
                local: LocalId(0),
                transfer: false,
            })],
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "consume".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Void,
        locals: vec![class_local(0, ClassId(0))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("an owned call parameter cannot receive a borrowed class value");
    assert!(error
        .message
        .contains("borrows argument 1 for an owned parameter"));
}

#[test]
fn shared_validator_rejects_owned_parameters_as_return_borrow_sources() {
    let mut program = class_program();
    program.functions.push(Function {
        id: FunctionId(1),
        name: "invalidBorrowReturn".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Value(Type::Class(ClassId(0))),
        locals: vec![class_local(0, ClassId(0))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Return(Rvalue::Class(ClassExpression::Local {
                class: ClassId(0),
                local: LocalId(0),
                transfer: false,
            })),
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("an owned parameter cannot escape as a borrowed return");
    assert!(error
        .message
        .contains("receives borrowed class local local0"));
}

#[test]
fn shared_validator_tracks_borrow_returning_outer_call_arguments() {
    let mut program = class_program();
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::CallVoid {
            span: Default::default(),
            function: FunctionId(2),
            args: vec![
                Rvalue::Class(ClassExpression::Call {
                    class: ClassId(0),
                    function: FunctionId(1),
                    args: vec![Rvalue::Class(ClassExpression::Local {
                        class: ClassId(0),
                        local: LocalId(0),
                        transfer: false,
                    })],
                    return_borrow: Some(doriac::mir::ReturnBorrow {
                        source: doriac::mir::BorrowSource::Parameter(0),
                        writable: false,
                    }),
                }),
                Rvalue::Class(ClassExpression::Local {
                    class: ClassId(0),
                    local: LocalId(0),
                    transfer: true,
                }),
            ],
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "identity".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Value(Type::Class(ClassId(0))),
        locals: vec![borrowed_class_local(0, ClassId(0))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Return(Rvalue::Class(ClassExpression::Local {
                class: ClassId(0),
                local: LocalId(0),
                transfer: false,
            })),
        }],
        entry_block: BlockId(0),
    });
    program.functions.push(Function {
        id: FunctionId(2),
        name: "observeThenConsume".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![
            borrowed_class_local(0, ClassId(0)),
            class_local(1, ClassId(0)),
        ],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a returned borrow must conflict with a later transfer in the outer call");
    assert!(error
        .message
        .contains("both borrows and transfers class local local0"));
}

#[test]
fn shared_validator_rejects_duplicate_class_local_transfers_in_one_call() {
    let mut program = class_program();
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::CallVoid {
            span: Default::default(),
            function: FunctionId(1),
            args: vec![
                Rvalue::Class(ClassExpression::Local {
                    class: ClassId(0),
                    local: LocalId(0),
                    transfer: true,
                }),
                Rvalue::Class(ClassExpression::Local {
                    class: ClassId(0),
                    local: LocalId(0),
                    transfer: true,
                }),
            ],
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "consumeBoth".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![class_local(0, ClassId(0)), class_local(1, ClassId(0))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("one class local cannot satisfy two ownership-taking arguments");
    assert!(error
        .message
        .contains("transfers class local local0 more than once"));
}

#[test]
fn shared_validator_rejects_class_use_after_a_transfer_on_any_reachable_path() {
    let mut program = class_program();
    let property = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    program.classes[0].properties.push(Property {
        id: property,
        name: "label".to_string(),
        ty: Type::String,
        writable: false,
        promoted: false,
    });
    program.classes[0].layout =
        compute_class_layout(ClassId(0), [(property, FieldType::String)], 8);
    program.functions[0].locals = vec![class_local(0, ClassId(0)), class_local(1, ClassId(0))];
    program.functions[0].blocks = vec![
        BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Branch {
                condition: doriac::mir::BoolExpression::Use {
                    operand: Operand::Scalar(ScalarValue::Bool(true)),
                },
                then_block: BlockId(1),
                else_block: BlockId(2),
            },
        },
        BasicBlock {
            id: BlockId(1),
            statements: vec![Statement::AssignLocal {
                target: LocalId(1),
                value: Rvalue::Class(ClassExpression::Local {
                    class: ClassId(0),
                    local: LocalId(0),
                    transfer: true,
                }),
            }],
            terminator: Terminator::Jump(BlockId(3)),
        },
        BasicBlock {
            id: BlockId(2),
            statements: vec![],
            terminator: Terminator::Jump(BlockId(3)),
        },
        BasicBlock {
            id: BlockId(3),
            statements: vec![Statement::EchoString(StringExpression::Property {
                object: LocalId(0),
                property,
            })],
            terminator: Terminator::ReturnVoid,
        },
    ];

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a class local moved on one predecessor is unavailable at the join");
    assert!(error
        .message
        .contains("uses class local local0 after its ownership ended"));
}

#[test]
fn shared_validator_rejects_borrowed_class_rvalues_in_owning_slots() {
    let mut assignment = class_program();
    assignment.functions[0].locals = vec![class_local(0, ClassId(0)), class_local(1, ClassId(0))];
    assignment.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Class(ClassExpression::Local {
                class: ClassId(0),
                local: LocalId(1),
                transfer: false,
            }),
        });
    let error = doriac::mir_validation::validate_program(&assignment)
        .expect_err("an owned class local cannot receive a borrowed class rvalue");
    assert!(error
        .message
        .contains("class assignment to local0 receives borrowed class local local1"));

    let mut returned = class_program();
    returned.functions.push(Function {
        id: FunctionId(1),
        name: "borrowedReturn".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![],
        return_type: ReturnType::Value(Type::Class(ClassId(0))),
        locals: vec![class_local(0, ClassId(0))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Return(Rvalue::Class(ClassExpression::Local {
                class: ClassId(0),
                local: LocalId(0),
                transfer: false,
            })),
        }],
        entry_block: BlockId(0),
    });
    let error = doriac::mir_validation::validate_program(&returned)
        .expect_err("a class return must transfer ownership");
    assert!(error
        .message
        .contains("return from borrowedReturn receives borrowed class local local0"));
}

#[test]
fn shared_validator_requires_owned_nullable_class_property_values() {
    let mut program = class_program();
    let property = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    program.classes[0].properties.push(Property {
        id: property,
        name: "box".to_string(),
        ty: Type::NullableClass(ClassId(1)),
        writable: true,
        promoted: false,
    });
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [(property, FieldType::NullableClass(ClassId(1)))],
        8,
    );
    let mut receiver = class_local(0, ClassId(0));
    receiver.writable = true;
    program.functions[0].locals = vec![receiver, class_local(1, ClassId(1))];
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignProperty {
            object: LocalId(0),
            property,
            value: Rvalue::NullableClass(NullableClassExpression::Class(ClassExpression::Local {
                class: ClassId(1),
                local: LocalId(1),
                transfer: false,
            })),
        });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("an owning nullable-class property cannot store a borrowed value");
    assert!(error
        .message
        .contains("assignment to property0 receives borrowed class local local1"));
}

#[test]
fn shared_validator_treats_promoted_nullable_class_arguments_as_transfers() {
    let mut program = class_program();
    let property = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    program.classes[0].properties.push(Property {
        id: property,
        name: "box".to_string(),
        ty: Type::NullableClass(ClassId(1)),
        writable: false,
        promoted: true,
    });
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [(property, FieldType::NullableClass(ClassId(1)))],
        8,
    );
    program.classes[0].constructor = Some(FunctionId(1));
    program.functions[0].locals = vec![class_local(0, ClassId(0)), class_local(1, ClassId(1))];
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Class(ClassExpression::New {
                class: ClassId(0),
                properties: vec![PropertyValue {
                    property,
                    source: PropertyValueSource::ConstructorArgument(0),
                }],
                constructor: Some(FunctionId(1)),
                args: vec![Rvalue::NullableClass(NullableClassExpression::Class(
                    ClassExpression::Local {
                        class: ClassId(1),
                        local: LocalId(1),
                        transfer: false,
                    },
                ))],
            }),
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "Holder::__construct".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![
            borrowed_class_local(0, ClassId(0)),
            Local {
                id: LocalId(1),
                name: "box".to_string(),
                ty: Type::NullableClass(ClassId(1)),
                writable: false,
                synthetic: false,
                owned: false,
            },
        ],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a promoted nullable-class property requires an owned argument");
    assert!(error
        .message
        .contains("call to Holder::__construct argument 1 receives borrowed class local local1"));
}

#[test]
fn shared_validator_checks_nullable_class_property_references() {
    let mut program = class_program();
    let property = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    program.classes[0].properties.push(Property {
        id: property,
        name: "missing".to_string(),
        ty: Type::NullableClass(ClassId(99)),
        writable: false,
        promoted: false,
    });
    program.classes[0].layout = compute_class_layout(
        ClassId(0),
        [(property, FieldType::NullableClass(ClassId(99)))],
        8,
    );

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("nullable-class properties must reference declared classes");
    assert!(error.message.contains("class#99 does not exist"));
}

#[test]
fn shared_validator_rejects_cleanup_and_assignment_of_borrowed_class_locals() {
    let mut drop_program = class_program();
    let mut borrowed = class_local(0, ClassId(0));
    borrowed.owned = false;
    drop_program.functions[0].locals.push(borrowed.clone());
    drop_program.functions[0].blocks[0]
        .statements
        .push(Statement::DropClass {
            local: LocalId(0),
            class: ClassId(0),
        });
    let error = doriac::mir_validation::validate_program(&drop_program)
        .expect_err("borrowed locals have no cleanup obligation");
    assert!(error.message.contains("references borrowed local0"));

    let mut assign_program = class_program();
    assign_program.functions[0].locals.push(borrowed);
    assign_program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Class(ClassExpression::New {
                class: ClassId(0),
                properties: vec![],
                constructor: None,
                args: vec![],
            }),
        });
    let error = doriac::mir_validation::validate_program(&assign_program)
        .expect_err("borrowed class slots cannot become owners through assignment");
    assert!(error
        .message
        .contains("borrowed class local0 receives an owning value"));
}

#[test]
fn shared_validator_rejects_mismatched_shared_reference_operations() {
    let mut program = class_program();
    program.functions.push(Function {
        id: FunctionId(1),
        name: "wrongShare".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Value(Type::SharedReference(ClassId(1))),
        locals: vec![Local {
            id: LocalId(0),
            name: "value".to_string(),
            ty: Type::SharedReference(ClassId(0)),
            writable: false,
            synthetic: false,
            owned: false,
        }],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Return(Rvalue::SharedReference(
                SharedReferenceExpression::Share {
                    class: ClassId(1),
                    value: Box::new(SharedReferenceExpression::Local {
                        class: ClassId(0),
                        local: LocalId(0),
                        transfer: false,
                    }),
                },
            )),
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("share must preserve the payload class identity");
    assert!(error.message.contains("share changes payload class"));
}

#[test]
fn shared_validator_rejects_mismatched_weak_acquisition_and_drop() {
    let mut program = class_program();
    program.functions[0].locals.push(Local {
        id: LocalId(0),
        name: "weak".to_string(),
        ty: Type::WeakReference(ClassId(0)),
        writable: false,
        synthetic: false,
        owned: true,
    });
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::WeakReference(WeakReferenceExpression::Create {
                class: ClassId(0),
                value: Box::new(SharedReferenceExpression::New {
                    class: ClassId(0),
                    value: Box::new(ClassExpression::New {
                        class: ClassId(0),
                        properties: vec![],
                        constructor: None,
                        args: vec![],
                    }),
                }),
            }),
        });
    program.functions[0].blocks[0]
        .statements
        .push(Statement::DropWeakReference {
            local: LocalId(0),
            class: ClassId(1),
        });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("weak-reference drop must use its handle's payload class");
    assert!(error.message.contains("weak-reference drop"));

    let mut acquire = class_program();
    acquire.functions.push(Function {
        id: FunctionId(1),
        name: "wrongAcquire".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Value(Type::NullableSharedReference(ClassId(1))),
        locals: vec![Local {
            id: LocalId(0),
            name: "weak".to_string(),
            ty: Type::WeakReference(ClassId(0)),
            writable: false,
            synthetic: false,
            owned: false,
        }],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Return(Rvalue::NullableSharedReference(
                NullableSharedReferenceExpression::Acquire {
                    class: ClassId(1),
                    value: Box::new(WeakReferenceExpression::Local {
                        class: ClassId(0),
                        local: LocalId(0),
                        transfer: false,
                    }),
                },
            )),
        }],
        entry_block: BlockId(0),
    });
    let error = doriac::mir_validation::validate_program(&acquire)
        .expect_err("weak acquisition must preserve the payload class identity");
    assert!(error
        .message
        .contains("weak acquisition changes payload class"));
}

fn decimal_spec() -> FormatSpec {
    FormatSpec {
        conversion: FormatConversion::Decimal,
        width: None,
        precision: None,
        left_align: false,
        zero_pad: false,
    }
}

fn display_spec() -> FormatSpec {
    FormatSpec {
        conversion: FormatConversion::Display,
        width: None,
        precision: None,
        left_align: false,
        zero_pad: false,
    }
}

fn valid_void_program() -> Program {
    Program {
        source: doriac::source::SourceFile::new("<test>", ""),
        classes: vec![],
        collection_types: vec![],
        statics: vec![],
        functions: vec![Function {
            id: FunctionId(0),
            name: "main".to_string(),
            source_span: Default::default(),
            method: None,
            receiver_mode: None,
            params: Vec::new(),
            return_type: ReturnType::Void,
            locals: Vec::new(),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                statements: Vec::new(),
                terminator: Terminator::ReturnVoid,
            }],
            entry_block: BlockId(0),
        }],
        entry: FunctionId(0),
    }
}

fn class_program() -> Program {
    let mut program = valid_void_program();
    program.classes = [ClassId(0), ClassId(1)]
        .into_iter()
        .map(|id| Class {
            id,
            name: format!("Class{}", id.0),
            properties: vec![],
            layout: compute_class_layout(id, [], 8),
            constructor: None,
            destructor: None,
        })
        .collect();
    program
}

fn class_local(index: usize, class: ClassId) -> Local {
    Local {
        id: LocalId(index),
        name: format!("class{index}"),
        ty: Type::Class(class),
        writable: false,
        synthetic: false,
        owned: true,
    }
}

fn nullable_class_local(index: usize, class: ClassId) -> Local {
    let mut local = class_local(index, class);
    local.ty = Type::NullableClass(class);
    local
}

fn borrowed_class_local(index: usize, class: ClassId) -> Local {
    let mut local = class_local(index, class);
    local.owned = false;
    local
}

fn class_new_program() -> Program {
    let mut program = class_program();
    let property = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    program.classes[0].properties.push(Property {
        id: property,
        name: "text".to_string(),
        ty: Type::String,
        writable: false,
        promoted: true,
    });
    program.classes[0].layout =
        compute_class_layout(ClassId(0), [(property, FieldType::String)], 8);
    program.classes[0].constructor = Some(FunctionId(1));
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Class(ClassExpression::New {
                class: ClassId(0),
                properties: vec![],
                constructor: Some(FunctionId(1)),
                args: vec![Rvalue::String(StringExpression::Literal(
                    "value".to_string(),
                ))],
            }),
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "Message::__construct".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![
            borrowed_class_local(0, ClassId(0)),
            Local {
                id: LocalId(1),
                name: "text".to_string(),
                ty: Type::String,
                writable: false,
                synthetic: false,
                owned: false,
            },
        ],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });
    program
}

fn promoted_class_alias_program() -> (Program, PropertyId) {
    let mut program = class_program();
    let child = PropertyId {
        class: ClassId(0),
        index: 0,
    };
    program.classes[0].properties.push(Property {
        id: child,
        name: "child".to_string(),
        ty: Type::Class(ClassId(1)),
        writable: true,
        promoted: true,
    });
    program.classes[0].layout =
        compute_class_layout(ClassId(0), [(child, FieldType::Class(ClassId(1)))], 8);
    program.classes[0].constructor = Some(FunctionId(1));
    program.functions[0].locals = vec![class_local(0, ClassId(0)), class_local(1, ClassId(1))];
    program.functions[0].blocks[0]
        .statements
        .push(Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Class(ClassExpression::New {
                class: ClassId(0),
                properties: vec![PropertyValue {
                    property: child,
                    source: PropertyValueSource::ConstructorArgument(0),
                }],
                constructor: Some(FunctionId(1)),
                args: vec![Rvalue::Class(ClassExpression::Local {
                    class: ClassId(1),
                    local: LocalId(1),
                    transfer: true,
                })],
            }),
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "Parent::__construct".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![
            borrowed_class_local(0, ClassId(0)),
            borrowed_class_local(1, ClassId(1)),
        ],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });
    program.functions.push(Function {
        id: FunctionId(2),
        name: "inspect".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Void,
        locals: vec![borrowed_class_local(0, ClassId(1))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });
    (program, child)
}
