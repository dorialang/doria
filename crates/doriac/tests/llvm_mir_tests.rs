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
        source: doriac::source::SourceFile::new("llvm-test.doria", ""),
        classes: vec![],
        collection_types: vec![],
        statics: vec![],
        functions: vec![
            Function {
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
            },
            Function {
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
"#,
    ] {
        assert_object(source);
    }
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

/// Where each `alloca` in a module was emitted.
///
/// `blocks` and `in_entry` exist so a caller can prove the scan actually
/// happened. An earlier version of this walk silently matched nothing, and an
/// empty `escaped` looked identical to a clean module.
#[derive(Default)]
struct AllocaPlacement {
    blocks: usize,
    in_entry: usize,
    escaped: Vec<String>,
}

/// Scans printed LLVM IR for allocations emitted outside their function's
/// entry block.
///
/// The IR is read as text because the property under test is exactly what the
/// printed module says: which basic block each allocation landed in.
fn scan_alloca_placement(ir: &str) -> AllocaPlacement {
    let mut placement = AllocaPlacement::default();
    let mut function = String::new();
    let mut entry = String::new();
    let mut block = String::new();

    for line in ir.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("define ") {
            function = rest
                .split_once('@')
                .map(|(_, name)| name.split('(').next().unwrap_or(name).to_string())
                .unwrap_or_default();
            entry.clear();
            block.clear();
            continue;
        }
        if trimmed == "}" {
            function.clear();
            continue;
        }
        if function.is_empty() || trimmed.is_empty() {
            continue;
        }
        // A block label is unindented and is either bare or followed by a
        // `; preds = ...` comment. Matching on a trailing colon alone misses
        // every block that has a predecessor, which is nearly all of them.
        if !line.starts_with(char::is_whitespace) {
            if let Some((label, rest)) = trimmed.split_once(':') {
                let rest = rest.trim();
                if rest.is_empty() || rest.starts_with(';') {
                    block = label.to_string();
                    placement.blocks += 1;
                    if entry.is_empty() {
                        entry = block.clone();
                    }
                    continue;
                }
            }
        }
        if trimmed.contains(" = alloca ") {
            if block == entry {
                placement.in_entry += 1;
            } else {
                placement
                    .escaped
                    .push(format!("{function}: {trimmed}  (in block '{block}')"));
            }
        }
    }
    placement
}

/// A scratch slot emitted outside the entry block is a dynamic stack
/// allocation. LLVM moves the stack pointer when it executes and does not
/// reclaim it until the function returns, so one emitted inside a loop grows
/// the frame every iteration until the program hits its guard page and dies
/// with no diagnostic. Emitting into the entry block instead makes the slot
/// part of the fixed frame, which costs one prologue instruction regardless of
/// how many times the surrounding code runs.
#[test]
fn allocates_every_scratch_slot_in_the_entry_block() {
    let sources: [&str; 6] = [
        // Dictionary get, set, index, and remove: the shape that first failed.
        r#"
function main(): void
{
    writable Dictionary<string, int> $values = [];
    writable List<string> $keys = [];
    for (let writable $index = 0; $index < 4; $index++) {
        let $key = "key{$index}";
        $values->set($key, $index);
        $keys->add($key);
    }
    let writable $total = 0;
    for (let writable $index = 0; $index < 4; $index++) {
        let $key = $keys[$index];
        $total = $total + ($values->get($key) ?? 0);
        $total = $total + $values[$key];
    }
    for (let writable $index = 0; $index < 4; $index++) {
        $values->remove($keys[$index]);
    }
    echo "{$total}:{$values->count}\n";
}
"#,
        // Set construction, membership, and removal inside a loop.
        r#"
function main(): void
{
    writable Set<int> $seen = Set::from([]);
    let writable $hits = 0;
    for (let writable $index = 0; $index < 8; $index++) {
        $seen->add($index % 4);
        if ($seen->contains($index % 4)) { $hits = $hits + 1; }
    }
    for (let writable $index = 0; $index < 4; $index++) {
        $seen->remove($index);
    }
    echo "{$hits}:{$seen->count}\n";
}
"#,
        // List access and removal, which drive the collection drop loops.
        r#"
function main(): void
{
    writable List<string> $items = [];
    for (let writable $index = 0; $index < 8; $index++) {
        $items->add("item{$index}");
    }
    let writable $count = 0;
    for (let writable $index = 0; $index < 4; $index++) {
        let $removed = $items->removeAt(0);
        $count = $count + $removed->length;
    }
    echo "{$count}:{$items->count}\n";
}
"#,
        // String search and parse, each of which allocates an out-parameter.
        r#"
function main(): void
{
    let writable $found = 0;
    for (let writable $index = 0; $index < 8; $index++) {
        let $text = "value{$index}";
        if (String::contains($text, "value")) { $found = $found + 1; }
        $found = $found + (Int::parse("{$index}") ?? 0);
    }
    echo "{$found}\n";
}
"#,
        // Sorted collections, which take the ordered runtime paths.
        r#"
function main(): void
{
    writable SortedSet<int> $ordered = SortedSet::from([]);
    writable SortedDictionary<string, int> $indexed = SortedDictionary::from([]);
    for (let writable $index = 0; $index < 8; $index++) {
        $ordered->add($index);
        $indexed->set("key{$index}", $index);
    }
    let writable $total = 0;
    for (let writable $index = 0; $index < 8; $index++) {
        if ($ordered->contains($index)) { $total = $total + 1; }
        $total = $total + ($indexed->get("key{$index}") ?? 0);
    }
    echo "{$total}\n";
}
"#,
        // Class temporaries allocated in a loop body.
        r#"
class Point
{
    function __construct(int $x, int $y)
    {
    }
}

function main(): void
{
    let writable $total = 0;
    for (let writable $index = 0; $index < 8; $index++) {
        let $point = new Point($index, $index * 2);
        $total = ($total + $point->x + $point->y) % 1000;
    }
    echo "{$total}\n";
}
"#,
    ];

    for source in sources {
        let program = doriac::lower_source_to_mir("llvm-test.doria", source)
            .expect("source should lower to MIR");
        let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program)
            .expect("verified MIR should lower to LLVM IR");
        let placement = scan_alloca_placement(&ir);
        // Prove the scan saw a real module before trusting that it found
        // nothing: a walk that matches no blocks reports a clean result too.
        assert!(
            placement.blocks > 1,
            "expected a multi-block module, saw {} blocks",
            placement.blocks
        );
        assert!(
            placement.in_entry > 0,
            "expected entry-block allocations, saw none"
        );
        assert!(
            placement.escaped.is_empty(),
            "these allocations leak stack on every pass through their block:\n{}",
            placement.escaped.join("\n")
        );
    }
}

/// Guards the scanner itself. The fixture carries the `; preds = ...` comments
/// LLVM actually prints after a label, because a scanner that only recognises
/// bare `label:` lines silently treats a whole function as one block and then
/// reports every module clean.
#[test]
fn detects_an_allocation_emitted_outside_the_entry_block() {
    let ir = "\
define internal void @sample(ptr %0) {
prologue:
  %slot = alloca i64, align 8
  br label %body

body:                                             ; preds = %prologue, %body
  %leaked = alloca i64, align 8
  br i1 true, label %body, label %done

done:                                             ; preds = %body
  ret void
}
";
    let placement = scan_alloca_placement(ir);
    assert_eq!(placement.blocks, 3, "expected three blocks");
    assert_eq!(placement.in_entry, 1, "expected one entry allocation");
    assert_eq!(
        placement.escaped.len(),
        1,
        "expected exactly one escaped allocation, got {:?}",
        placement.escaped
    );
    assert!(
        placement.escaped[0].contains("%leaked"),
        "{}",
        placement.escaped[0]
    );
    assert!(
        placement.escaped[0].contains("'body'"),
        "{}",
        placement.escaped[0]
    );
}
