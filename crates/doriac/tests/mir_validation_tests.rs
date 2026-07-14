use doriac::class_layout::{compute_class_layout, ClassId, FieldType, PropertyId};
use doriac::format_string::{FormatConversion, FormatPiece, FormatSpec};
use doriac::mir::{
    BasicBlock, BlockId, Class, ClassExpression, FloatBinaryOp, FloatExpression, FormatArgument,
    FormatExpression, Function, FunctionId, Local, LocalId, NullableStringExpression, Operand,
    Program, Property, PropertyValue, PropertyValueSource, ReturnType, Rvalue, ScalarType,
    ScalarValue, Statement, StringExpression, Terminator, Type, ValueExpression,
};
use doriac::numeric::{FloatType, FloatValue, IntegerType, IntegerValue};

#[test]
fn shared_validator_rejects_mixed_width_float_binary_operands() {
    let mut program = valid_void_program();
    program.functions.push(Function {
        id: FunctionId(1),
        name: "mixedWidth".to_string(),
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
        .contains("bool expression has an incompatible operand"));
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
fn shared_validator_rejects_scalar_string_assignment_mixing() {
    let mut program = valid_void_program();
    program.functions[0].locals.push(Local {
        id: LocalId(0),
        name: "value".to_string(),
        ty: Type::String,
        writable: true,
        synthetic: false,
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
            }),
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "makeOther".to_string(),
        params: vec![],
        return_type: ReturnType::Value(Type::Class(ClassId(1))),
        locals: vec![class_local(0, ClassId(1))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::Return(Rvalue::Class(ClassExpression::Local {
                class: ClassId(1),
                local: LocalId(0),
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
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![
            class_local(0, ClassId(0)),
            Local {
                id: LocalId(1),
                name: "text".to_string(),
                ty: Type::String,
                writable: false,
                synthetic: false,
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
                })],
            }),
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "Pair::__construct".to_string(),
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![class_local(0, ClassId(0)), class_local(1, ClassId(1))],
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
                            },
                        )),
                    },
                    PropertyValue {
                        property: second,
                        source: PropertyValueSource::Expression(Rvalue::Class(
                            ClassExpression::Local {
                                class: ClassId(1),
                                local: LocalId(1),
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
        .contains("gives class local local1 to more than one property"));
}

#[test]
fn shared_validator_rejects_passing_a_promoted_class_owner_to_the_constructor() {
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
                })],
            }),
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "Parent::__construct".to_string(),
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![class_local(0, ClassId(0)), class_local(1, ClassId(1))],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: vec![],
            terminator: Terminator::ReturnVoid,
        }],
        entry_block: BlockId(0),
    });

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a promoted class owner cannot also enter the constructor body");
    assert!(error.message.contains(
        "gives constructor argument 0 to a property and also passes it to Parent::__construct"
    ));
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
    valid.functions.push(Function {
        id: FunctionId(1),
        name: "Class0::__destruct".to_string(),
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

    let mut wrong_return = valid;
    wrong_return.functions[1].return_type = ReturnType::Value(Type::String);
    let error = doriac::mir_validation::validate_program(&wrong_return)
        .expect_err("lifecycle functions must return void");
    assert!(error.message.contains("does not return void"));
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

fn valid_void_program() -> Program {
    Program {
        classes: vec![],
        functions: vec![Function {
            id: FunctionId(0),
            name: "main".to_string(),
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
    }
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
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![
            class_local(0, ClassId(0)),
            Local {
                id: LocalId(1),
                name: "text".to_string(),
                ty: Type::String,
                writable: false,
                synthetic: false,
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
