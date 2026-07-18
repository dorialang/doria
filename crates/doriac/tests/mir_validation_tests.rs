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
        ty: Type::String,
        writable: false,
        promoted: false,
    });
    program.classes[0].layout = compute_class_layout(ClassId(0), [(label, FieldType::String)], 8);
    program.functions[0].locals.push(class_local(0, ClassId(0)));
    program.functions[0].blocks[0]
        .statements
        .push(Statement::CallVoid {
            function: FunctionId(1),
            args: vec![
                Rvalue::Class(ClassExpression::Local {
                    class: ClassId(0),
                    local: LocalId(0),
                    transfer: true,
                }),
                Rvalue::String(StringExpression::Property {
                    object: LocalId(0),
                    property: label,
                }),
            ],
        });
    program.functions.push(Function {
        id: FunctionId(1),
        name: "takeWithLabel".to_string(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0), LocalId(1)],
        return_type: ReturnType::Void,
        locals: vec![
            class_local(0, ClassId(0)),
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

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("a call cannot read a property after transferring its object");
    assert!(error
        .message
        .contains("both borrows and transfers class local local0"));
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
        .contains("uses class local local1 after transferring it"));
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
                args: vec![],
            }),
        });
    let mut receiver = class_local(0, ClassId(0));
    receiver.owned = false;
    program.functions.push(Function {
        id: FunctionId(1),
        name: "Message::__construct".to_string(),
        method: None,
        receiver_mode: None,
        params: vec![LocalId(0)],
        return_type: ReturnType::Void,
        locals: vec![receiver],
        blocks: vec![
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
        .contains("assignment targets borrowed local local0"));
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
        statics: vec![],
        functions: vec![Function {
            id: FunctionId(0),
            name: "main".to_string(),
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
