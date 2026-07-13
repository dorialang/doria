#![cfg(feature = "llvm-backend")]

use doriac::mir::{
    BasicBlock, BlockId, FloatBinaryOp, FloatExpression, Function, FunctionId, Program, ReturnType,
    Rvalue, ScalarType, Terminator, Type, ValueExpression,
};
use doriac::numeric::{FloatType, FloatValue};

fn assert_object(source: &str) {
    let program =
        doriac::lower_source_to_mir("llvm-test.doria", source).expect("source should lower to MIR");
    let object = doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("verified MIR should lower to an optimized LLVM object");
    assert!(!object.is_empty());
}

#[test]
fn lowers_complete_stage_14_mir_shapes_to_verified_objects() {
    for source in [
        include_str!("../../../examples/native/main_return_42.doria"),
        include_str!("../../../examples/native/main_void_empty.doria"),
        include_str!("../../../examples/native/main_function_add_42.doria"),
        include_str!("../../../examples/native/main_recursive_fibonacci_55.doria"),
        include_str!("../../../examples/native/main_narrow_recursive_42.doria"),
        include_str!("../../../examples/native/main_fixed_width_arithmetic_42.doria"),
        include_str!("../../../examples/native/main_uint64_boundary_42.doria"),
        include_str!("../../../examples/native/main_add_overflow_panic.doria"),
        include_str!("../../../examples/native/main_divide_by_zero_panic.doria"),
        include_str!("../../../examples/native/main_shift_count_panic.doria"),
        include_str!("../../../examples/native/main_integer_conversion_panic.doria"),
        include_str!("../../../examples/native/main_float32_rounding_42.doria"),
        include_str!("../../../examples/native/main_float64_arithmetic_42.doria"),
        include_str!("../../../examples/native/main_float_nan_comparison_42.doria"),
        include_str!("../../../examples/native/main_float_signed_zero_42.doria"),
        include_str!("../../../examples/native/main_bool_short_circuit_42.doria"),
        include_str!("../../../examples/native/main_bool_xor_42.doria"),
        include_str!("../../../examples/native/main_float_to_int_42.doria"),
        include_str!("../../../examples/native/main_float_to_int_nan_panic.doria"),
        include_str!("../../../examples/native/main_float_to_int_infinity_panic.doria"),
        include_str!("../../../examples/native/main_float_to_int_range_panic.doria"),
        include_str!("../../../examples/native/main_string_concat_hello.doria"),
        include_str!("../../../examples/native/main_invalid_status_panic.doria"),
        include_str!("../../../examples/native/main_release_profile_42.doria"),
    ] {
        assert_object(source);
    }
}

#[test]
fn rejects_malformed_mixed_width_float_mir_before_llvm_emission() {
    let program = Program {
        functions: vec![
            Function {
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
            },
            Function {
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
            },
        ],
        entry: FunctionId(0),
    };

    let error = doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect_err("malformed MIR should be rejected before LLVM construction");
    assert!(error
        .message
        .contains("float binary expression has float32 and float operands"));
}

#[test]
fn lowers_complete_stage17_io_and_format_mir_to_verified_objects() {
    for source in [
        include_str!("../../../examples/native/main_read_line_echo.doria"),
        include_str!("../../../examples/native/main_file_copy.doria"),
        include_str!("../../../examples/native/main_sprintf_matrix.doria"),
        include_str!("../../../examples/native/main_printf_42.doria"),
        include_str!("../../../examples/native/main_write_stderr.doria"),
        include_str!("../../../examples/native/main_missing_file_panic.doria"),
        r#"
function identity(?string $value): ?string { return $value; }
function main(): void
{
    let $line = identity(read_line());
    if ($line != null) { echo $line; }
}

#[test]
fn lowers_stage_18_expression_interpolation_to_verified_objects() {
    for source in [
        include_str!("../../../examples/native/main_expression_interpolation.doria"),
        include_str!("../../../examples/native/main_expression_interpolation_order.doria"),
    ] {
        assert_object(source);
    }
}
"#,
    ] {
        assert_object(source);
    }
}
