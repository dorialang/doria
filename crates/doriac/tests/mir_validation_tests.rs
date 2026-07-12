use doriac::format_string::{FormatConversion, FormatPiece, FormatSpec};
use doriac::mir::{
    BasicBlock, BlockId, FloatBinaryOp, FloatExpression, FormatArgument, FormatExpression,
    Function, FunctionId, Local, LocalId, NullableStringExpression, Operand, Program, ReturnType,
    Rvalue, ScalarType, ScalarValue, Statement, StringExpression, Terminator, Type,
    ValueExpression,
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
