use std::cmp::Ordering;

use doriac::numeric::{IntegerPanic, IntegerType, IntegerValue};

fn signed_value(ty: IntegerType, value: i128) -> IntegerValue {
    IntegerValue::from_i128(ty, value).expect("test signed value should fit")
}

fn unsigned_value(ty: IntegerType, value: u128) -> IntegerValue {
    IntegerValue::from_u128(ty, value).expect("test unsigned value should fit")
}

#[test]
fn exhaustively_checks_int8_comparison_and_bitwise_operations() {
    for left in i8::MIN..=i8::MAX {
        let left_value = signed_value(IntegerType::Int8, left as i128);
        assert_eq!(left_value.bitwise_not().signed_value(), (!left) as i128);

        for right in i8::MIN..=i8::MAX {
            let right_value = signed_value(IntegerType::Int8, right as i128);
            assert_eq!(left_value.compare(right_value), left.cmp(&right));
            assert_eq!(
                left_value.bitwise_and(right_value).signed_value(),
                (left & right) as i128
            );
            assert_eq!(
                left_value.bitwise_or(right_value).signed_value(),
                (left | right) as i128
            );
            assert_eq!(
                left_value.bitwise_xor(right_value).signed_value(),
                (left ^ right) as i128
            );
        }
    }
}

#[test]
fn exhaustively_checks_uint8_comparison_and_bitwise_operations() {
    for left in u8::MIN..=u8::MAX {
        let left_value = unsigned_value(IntegerType::UInt8, left as u128);
        assert_eq!(left_value.bitwise_not().unsigned_value(), (!left) as u128);

        for right in u8::MIN..=u8::MAX {
            let right_value = unsigned_value(IntegerType::UInt8, right as u128);
            assert_eq!(left_value.compare(right_value), left.cmp(&right));
            assert_eq!(
                left_value.bitwise_and(right_value).unsigned_value(),
                (left & right) as u128
            );
            assert_eq!(
                left_value.bitwise_or(right_value).unsigned_value(),
                (left | right) as u128
            );
            assert_eq!(
                left_value.bitwise_xor(right_value).unsigned_value(),
                (left ^ right) as u128
            );
        }
    }
}

#[test]
fn exhaustively_checks_int8_division_and_remainder() {
    for left in i8::MIN..=i8::MAX {
        for right in i8::MIN..=i8::MAX {
            let left_value = signed_value(IntegerType::Int8, left as i128);
            let right_value = signed_value(IntegerType::Int8, right as i128);

            if right == 0 {
                assert_eq!(
                    left_value.divide(right_value),
                    Err(IntegerPanic::DivisionByZero)
                );
                assert_eq!(
                    left_value.remainder(right_value),
                    Err(IntegerPanic::RemainderByZero)
                );
            } else if left == i8::MIN && right == -1 {
                assert_eq!(
                    left_value.divide(right_value),
                    Err(IntegerPanic::DivisionOverflow)
                );
                assert_eq!(left_value.remainder(right_value).unwrap().signed_value(), 0);
            } else {
                assert_eq!(
                    left_value.divide(right_value).unwrap().signed_value(),
                    (left / right) as i128
                );
                assert_eq!(
                    left_value.remainder(right_value).unwrap().signed_value(),
                    (left % right) as i128
                );
            }
        }
    }
}

#[test]
fn exhaustively_checks_uint8_division_and_remainder() {
    for left in u8::MIN..=u8::MAX {
        for right in u8::MIN..=u8::MAX {
            let left_value = unsigned_value(IntegerType::UInt8, left as u128);
            let right_value = unsigned_value(IntegerType::UInt8, right as u128);

            if let Some(quotient) = left.checked_div(right) {
                let remainder = left
                    .checked_rem(right)
                    .expect("a valid unsigned divisor has a remainder");
                assert_eq!(
                    left_value.divide(right_value).unwrap().unsigned_value(),
                    quotient as u128
                );
                assert_eq!(
                    left_value.remainder(right_value).unwrap().unsigned_value(),
                    remainder as u128
                );
            } else {
                assert_eq!(
                    left_value.divide(right_value),
                    Err(IntegerPanic::DivisionByZero)
                );
                assert_eq!(
                    left_value.remainder(right_value),
                    Err(IntegerPanic::RemainderByZero)
                );
            }
        }
    }
}

#[test]
fn checks_arithmetic_and_shift_boundaries_for_every_integer_type() {
    for ty in IntegerType::ALL {
        let zero = IntegerValue::zero(ty);
        let one = IntegerValue::one(ty);
        let max = IntegerValue::from_i128(ty, ty.max_value()).unwrap();

        assert_eq!(max.checked_add(one), Err(IntegerPanic::OverflowAddition));
        assert_eq!(
            max.checked_mul(max),
            Err(IntegerPanic::OverflowMultiplication)
        );
        assert_eq!(
            one.shift_left(IntegerValue::from_i128(ty, ty.bit_width() as i128).unwrap()),
            Err(IntegerPanic::ShiftCountOutOfRange)
        );

        if ty.is_signed() {
            let min = IntegerValue::from_i128(ty, ty.min_value()).unwrap();
            assert_eq!(min.checked_sub(one), Err(IntegerPanic::OverflowSubtraction));
            assert_eq!(min.checked_neg(), Err(IntegerPanic::OverflowNegation));
            assert_eq!(
                one.shift_left(IntegerValue::from_i128(ty, -1).unwrap()),
                Err(IntegerPanic::ShiftCountOutOfRange)
            );
        } else {
            assert_eq!(
                zero.checked_sub(one),
                Err(IntegerPanic::OverflowSubtraction)
            );
        }
    }
}

#[test]
fn checks_boundary_table_arithmetic_for_every_integer_type() {
    for ty in IntegerType::ALL {
        let mut values = vec![0, 1, ty.max_value() - 1, ty.max_value()];
        if ty.is_signed() {
            values.extend([ty.min_value(), ty.min_value() + 1, -1]);
        }
        values.sort_unstable();
        values.dedup();

        for left in &values {
            for right in &values {
                let left_value = IntegerValue::from_i128(ty, *left).unwrap();
                let right_value = IntegerValue::from_i128(ty, *right).unwrap();
                let in_range = |value: i128| value >= ty.min_value() && value <= ty.max_value();
                let add_ok = if ty.is_signed() {
                    in_range(*left + *right)
                } else {
                    (*left as u128)
                        .checked_add(*right as u128)
                        .is_some_and(|value| value <= ty.max_value() as u128)
                };
                let sub_ok = if ty.is_signed() {
                    in_range(*left - *right)
                } else {
                    (*left as u128)
                        .checked_sub(*right as u128)
                        .is_some_and(|value| value <= ty.max_value() as u128)
                };
                let mul_ok = if ty.is_signed() {
                    in_range(*left * *right)
                } else {
                    (*left as u128)
                        .checked_mul(*right as u128)
                        .is_some_and(|value| value <= ty.max_value() as u128)
                };

                assert_eq!(
                    left_value.checked_add(right_value).is_ok(),
                    add_ok,
                    "addition: {left} and {right} as {ty}"
                );
                assert_eq!(
                    left_value.checked_sub(right_value).is_ok(),
                    sub_ok,
                    "subtraction: {left} and {right} as {ty}"
                );
                assert_eq!(
                    left_value.checked_mul(right_value).is_ok(),
                    mul_ok,
                    "multiplication: {left} and {right} as {ty}"
                );
            }
        }
    }
}

#[test]
fn shifts_use_fixed_width_signed_and_unsigned_rules() {
    let signed = signed_value(IntegerType::Int8, -2);
    let one_signed = signed_value(IntegerType::Int8, 1);
    assert_eq!(signed.shift_right(one_signed).unwrap().signed_value(), -1);
    assert_eq!(signed.shift_left(one_signed).unwrap().signed_value(), -4);
    assert_eq!(
        signed_value(IntegerType::Int8, 64)
            .shift_left(signed_value(IntegerType::Int8, 2))
            .unwrap()
            .signed_value(),
        0,
        "left shift discards bits beyond the fixed width without arithmetic overflow"
    );

    let unsigned = unsigned_value(IntegerType::UInt8, 254);
    let one_unsigned = unsigned_value(IntegerType::UInt8, 1);
    assert_eq!(
        unsigned.shift_right(one_unsigned).unwrap().unsigned_value(),
        127
    );
    assert_eq!(
        unsigned.shift_left(one_unsigned).unwrap().unsigned_value(),
        252
    );
}

#[test]
fn integer_panics_have_the_stable_stage_13_messages() {
    let cases = [
        (
            IntegerPanic::OverflowAddition,
            "integer overflow during addition",
        ),
        (
            IntegerPanic::OverflowSubtraction,
            "integer overflow during subtraction",
        ),
        (
            IntegerPanic::OverflowMultiplication,
            "integer overflow during multiplication",
        ),
        (
            IntegerPanic::OverflowNegation,
            "integer overflow during negation",
        ),
        (IntegerPanic::DivisionByZero, "integer division by zero"),
        (IntegerPanic::DivisionOverflow, "integer division overflow"),
        (IntegerPanic::RemainderByZero, "integer remainder by zero"),
        (
            IntegerPanic::ShiftCountOutOfRange,
            "integer shift count out of range",
        ),
        (
            IntegerPanic::ConversionOutOfRange,
            "integer conversion out of range",
        ),
    ];

    for (panic, message) in cases {
        assert_eq!(panic.message(), message);
    }
}

#[test]
fn checks_every_integer_conversion_pair_at_boundaries() {
    for source in IntegerType::ALL {
        let mut source_values = vec![0_i128, 1, source.max_value() - 1, source.max_value()];
        if source.is_signed() {
            source_values.extend([source.min_value(), source.min_value() + 1, -1]);
        }
        source_values.sort_unstable();
        source_values.dedup();

        for source_value in source_values {
            let value = IntegerValue::from_i128(source, source_value).unwrap();
            for target in IntegerType::ALL {
                let expected =
                    source_value >= target.min_value() && source_value <= target.max_value();
                let converted = value.convert(target);
                assert_eq!(
                    converted.is_ok(),
                    expected,
                    "{source_value}: {source} -> {target}"
                );
                if let Ok(converted) = converted {
                    assert_eq!(converted.mathematical_value(), source_value);
                } else {
                    assert_eq!(converted, Err(IntegerPanic::ConversionOutOfRange));
                }
            }
        }
    }
}

#[test]
fn preserves_uint64_maximum_without_signed_reinterpretation() {
    let value = unsigned_value(IntegerType::UInt64, u64::MAX as u128);
    assert_eq!(value.bits, u64::MAX);
    assert_eq!(value.unsigned_value(), u64::MAX as u128);
    assert_eq!(
        value.compare(IntegerValue::zero(IntegerType::UInt64)),
        Ordering::Greater
    );
    assert_eq!(value.to_string(), u64::MAX.to_string());
}
