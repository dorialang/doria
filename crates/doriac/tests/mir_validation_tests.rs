use doriac::mir::{
    BasicBlock, BlockId, FloatBinaryOp, FloatExpression, Function, FunctionId, Operand, Program,
    ReturnType, ScalarType, ScalarValue, Terminator, ValueExpression,
};
use doriac::numeric::{FloatType, FloatValue, IntegerType, IntegerValue};

#[test]
fn shared_validator_rejects_mixed_width_float_binary_operands() {
    let mut program = valid_void_program();
    program.functions.push(Function {
        id: FunctionId(1),
        name: "mixedWidth".to_string(),
        params: Vec::new(),
        return_type: ReturnType::Value(ScalarType::Float(FloatType::Float64)),
        locals: Vec::new(),
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: Vec::new(),
            terminator: Terminator::Return(ValueExpression::Float(FloatExpression::Binary {
                ty: FloatType::Float64,
                op: FloatBinaryOp::Add,
                left: Box::new(FloatExpression::constant(FloatValue::from_f32(1.0))),
                right: Box::new(FloatExpression::constant(FloatValue::from_f64(2.0))),
            })),
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
