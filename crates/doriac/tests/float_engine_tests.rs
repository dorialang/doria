use doriac::numeric::{FloatType, FloatValue};

#[test]
fn float_type_metadata_is_canonical() {
    assert_eq!(
        FloatType::from_source_name("float"),
        Some(FloatType::Float64)
    );
    assert_eq!(
        FloatType::from_source_name("float64"),
        Some(FloatType::Float64)
    );
    assert_eq!(
        FloatType::from_source_name("float32"),
        Some(FloatType::Float32)
    );
    assert_eq!(FloatType::Float32.bit_width(), 32);
    assert_eq!(FloatType::Float64.storage_bytes(), 8);
    assert_eq!(FloatType::Float64.source_name(), "float");
    assert_eq!(FloatType::Float64.explicit_source_name(), "float64");
}

#[test]
fn float32_operations_have_expected_bits() {
    let one = FloatValue::from_f32(1.0);
    let two = FloatValue::from_f32(2.0);
    let four = FloatValue::from_f32(4.0);
    assert_eq!(one.add(two).bits, 3.0_f32.to_bits() as u64);
    assert_eq!(four.subtract(one).bits, 3.0_f32.to_bits() as u64);
    assert_eq!(two.multiply(two).bits, 4.0_f32.to_bits() as u64);
    assert_eq!(one.divide(two).bits, 0.5_f32.to_bits() as u64);
    assert_eq!(one.negate().bits, (-1.0_f32).to_bits() as u64);
}

#[test]
fn float64_operations_have_expected_bits() {
    let one = FloatValue::from_f64(1.0);
    let two = FloatValue::from_f64(2.0);
    let four = FloatValue::from_f64(4.0);
    assert_eq!(one.add(two).bits, 3.0_f64.to_bits());
    assert_eq!(four.subtract(one).bits, 3.0_f64.to_bits());
    assert_eq!(two.multiply(two).bits, 4.0_f64.to_bits());
    assert_eq!(one.divide(two).bits, 0.5_f64.to_bits());
    assert_eq!(one.negate().bits, (-1.0_f64).to_bits());
}

#[test]
fn signed_zero_infinities_subnormals_and_nan_follow_ieee() {
    for ty in [FloatType::Float32, FloatType::Float64] {
        let zero = FloatValue::zero(ty);
        let negative_zero = zero.negate();
        let one = match ty {
            FloatType::Float32 => FloatValue::from_f32(1.0),
            FloatType::Float64 => FloatValue::from_f64(1.0),
        };
        let positive_infinity = one.divide(zero);
        let negative_infinity = one.divide(negative_zero);
        let nan = zero.divide(zero);

        assert!(negative_zero.is_negative_zero());
        assert!(zero.compare_equal(negative_zero));
        assert!(positive_infinity.is_infinite());
        assert!(negative_infinity.is_infinite());
        assert!(nan.is_nan());
        assert!(!nan.compare_equal(nan));
        assert!(nan.compare_not_equal(nan));
        assert!(!nan.compare_less(one));
        assert!(!nan.compare_less_equal(one));
        assert!(!nan.compare_greater(one));
        assert!(!nan.compare_greater_equal(one));
    }

    assert_eq!(FloatValue::from_f32(f32::MIN_POSITIVE).bits, 0x0080_0000);
    assert_eq!(FloatValue::from_f32(f32::from_bits(1)).bits, 1);
    assert_eq!(
        FloatValue::from_f64(f64::MIN_POSITIVE).bits,
        0x0010_0000_0000_0000
    );
    assert_eq!(FloatValue::from_f64(f64::from_bits(1)).bits, 1);
}

#[test]
fn decimal_literals_round_directly_to_the_context_width() {
    assert_eq!(
        FloatValue::parse_decimal(FloatType::Float32, "0.1")
            .unwrap()
            .bits,
        0x3dcc_cccd
    );
    assert_eq!(
        FloatValue::parse_decimal(FloatType::Float64, "0.1")
            .unwrap()
            .bits,
        0x3fb9_9999_9999_999a
    );
}

#[test]
fn float32_rounds_after_each_operation() {
    let value = FloatValue::from_f32(16_777_216.0);
    let one = FloatValue::from_f32(1.0);
    assert_eq!(value.add(one).bits, value.bits);

    let wide = FloatValue::from_f64(16_777_216.0);
    assert_ne!(wide.add(FloatValue::from_f64(1.0)).bits, wide.bits);
}

#[test]
fn float_to_int_checks_ieee_boundaries() {
    assert_eq!(FloatValue::from_f64(42.9).to_i64_checked(), Some(42));
    assert_eq!(FloatValue::from_f64(-42.9).to_i64_checked(), Some(-42));
    assert_eq!(FloatValue::from_f64(-0.0).to_i64_checked(), Some(0));
    assert_eq!(
        FloatValue::from_f64(-9_223_372_036_854_775_808.0).to_i64_checked(),
        Some(i64::MIN)
    );
    assert_eq!(
        FloatValue::from_f64(9_223_372_036_854_775_807.0).to_i64_checked(),
        None
    );
    assert_eq!(FloatValue::from_f64(f64::NAN).to_i64_checked(), None);
    assert_eq!(FloatValue::from_f64(f64::INFINITY).to_i64_checked(), None);
}
