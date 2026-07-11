use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FloatType {
    Float32,
    Float64,
}

impl FloatType {
    pub fn from_source_name(name: &str) -> Option<Self> {
        match name {
            "float32" => Some(Self::Float32),
            "float" | "float64" => Some(Self::Float64),
            _ => None,
        }
    }

    /// Canonical diagnostic spelling. `float64` shares the `float` identity.
    pub const fn source_name(self) -> &'static str {
        match self {
            Self::Float32 => "float32",
            Self::Float64 => "float",
        }
    }

    pub const fn is_default_float(self) -> bool {
        matches!(self, Self::Float64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IntegerType {
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
}

impl IntegerType {
    pub const ALL: [Self; 8] = [
        Self::Int8,
        Self::Int16,
        Self::Int32,
        Self::Int64,
        Self::UInt8,
        Self::UInt16,
        Self::UInt32,
        Self::UInt64,
    ];

    pub fn from_source_name(name: &str) -> Option<Self> {
        match name {
            "int8" => Some(Self::Int8),
            "int16" => Some(Self::Int16),
            "int32" => Some(Self::Int32),
            "int" | "int64" => Some(Self::Int64),
            "uint8" => Some(Self::UInt8),
            "uint16" => Some(Self::UInt16),
            "uint32" => Some(Self::UInt32),
            "uint64" => Some(Self::UInt64),
            _ => None,
        }
    }

    pub fn from_companion_name(name: &str) -> Option<Self> {
        match name {
            "Int8" => Some(Self::Int8),
            "Int16" => Some(Self::Int16),
            "Int32" => Some(Self::Int32),
            "Int" | "Int64" => Some(Self::Int64),
            "UInt8" => Some(Self::UInt8),
            "UInt16" => Some(Self::UInt16),
            "UInt32" => Some(Self::UInt32),
            "UInt64" => Some(Self::UInt64),
            _ => None,
        }
    }

    pub const fn bit_width(self) -> u32 {
        match self {
            Self::Int8 | Self::UInt8 => 8,
            Self::Int16 | Self::UInt16 => 16,
            Self::Int32 | Self::UInt32 => 32,
            Self::Int64 | Self::UInt64 => 64,
        }
    }

    pub const fn storage_bytes(self) -> u32 {
        self.bit_width() / 8
    }

    pub const fn is_signed(self) -> bool {
        matches!(self, Self::Int8 | Self::Int16 | Self::Int32 | Self::Int64)
    }

    pub const fn is_default_int(self) -> bool {
        matches!(self, Self::Int64)
    }

    pub const fn min_value(self) -> i128 {
        if self.is_signed() {
            -(1_i128 << (self.bit_width() - 1))
        } else {
            0
        }
    }

    pub const fn max_value(self) -> i128 {
        if self.is_signed() {
            (1_i128 << (self.bit_width() - 1)) - 1
        } else {
            (1_i128 << self.bit_width()) - 1
        }
    }

    pub const fn mask(self) -> u64 {
        if self.bit_width() == 64 {
            u64::MAX
        } else {
            (1_u64 << self.bit_width()) - 1
        }
    }

    /// Canonical diagnostic/MIR spelling. `int64` shares the `int` identity.
    pub const fn source_name(self) -> &'static str {
        match self {
            Self::Int8 => "int8",
            Self::Int16 => "int16",
            Self::Int32 => "int32",
            Self::Int64 => "int",
            Self::UInt8 => "uint8",
            Self::UInt16 => "uint16",
            Self::UInt32 => "uint32",
            Self::UInt64 => "uint64",
        }
    }

    pub const fn explicit_source_name(self) -> &'static str {
        match self {
            Self::Int64 => "int64",
            _ => self.source_name(),
        }
    }

    pub const fn companion_name(self) -> &'static str {
        match self {
            Self::Int8 => "Int8",
            Self::Int16 => "Int16",
            Self::Int32 => "Int32",
            Self::Int64 => "Int",
            Self::UInt8 => "UInt8",
            Self::UInt16 => "UInt16",
            Self::UInt32 => "UInt32",
            Self::UInt64 => "UInt64",
        }
    }
}

impl fmt::Display for IntegerType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.source_name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IntegerValue {
    pub ty: IntegerType,
    pub bits: u64,
}

impl IntegerValue {
    pub const fn from_bits(ty: IntegerType, bits: u64) -> Self {
        Self {
            ty,
            bits: bits & ty.mask(),
        }
    }

    pub fn from_i128(ty: IntegerType, value: i128) -> Option<Self> {
        (value >= ty.min_value() && value <= ty.max_value())
            .then(|| Self::from_bits(ty, value as u64))
    }

    pub fn from_u128(ty: IntegerType, value: u128) -> Option<Self> {
        (value <= ty.max_value() as u128).then(|| Self::from_bits(ty, value as u64))
    }

    pub fn from_literal(ty: IntegerType, magnitude: u128, negative: bool) -> Option<Self> {
        if negative {
            if !ty.is_signed() {
                return None;
            }
            let minimum_magnitude = ty.min_value().unsigned_abs();
            if magnitude > minimum_magnitude {
                return None;
            }
            if magnitude == minimum_magnitude {
                return Self::from_i128(ty, ty.min_value());
            }
            Self::from_i128(ty, -(magnitude as i128))
        } else {
            Self::from_u128(ty, magnitude)
        }
    }

    pub const fn zero(ty: IntegerType) -> Self {
        Self::from_bits(ty, 0)
    }

    pub const fn one(ty: IntegerType) -> Self {
        Self::from_bits(ty, 1)
    }

    pub fn signed_value(self) -> i128 {
        debug_assert!(self.ty.is_signed());
        let shift = 64 - self.ty.bit_width();
        (((self.bits << shift) as i64) >> shift) as i128
    }

    pub fn unsigned_value(self) -> u128 {
        (self.bits & self.ty.mask()) as u128
    }

    pub fn mathematical_value(self) -> i128 {
        if self.ty.is_signed() {
            self.signed_value()
        } else {
            self.unsigned_value() as i128
        }
    }

    pub fn checked_add(self, right: Self) -> Result<Self, IntegerPanic> {
        self.require_same_type(right);
        let result = if self.ty.is_signed() {
            Self::from_i128(self.ty, self.signed_value() + right.signed_value())
        } else {
            Self::from_u128(self.ty, self.unsigned_value() + right.unsigned_value())
        };
        result.ok_or(IntegerPanic::OverflowAddition)
    }

    pub fn checked_sub(self, right: Self) -> Result<Self, IntegerPanic> {
        self.require_same_type(right);
        let result = if self.ty.is_signed() {
            Self::from_i128(self.ty, self.signed_value() - right.signed_value())
        } else {
            self.unsigned_value()
                .checked_sub(right.unsigned_value())
                .and_then(|value| Self::from_u128(self.ty, value))
        };
        result.ok_or(IntegerPanic::OverflowSubtraction)
    }

    pub fn checked_mul(self, right: Self) -> Result<Self, IntegerPanic> {
        self.require_same_type(right);
        let result = if self.ty.is_signed() {
            Self::from_i128(self.ty, self.signed_value() * right.signed_value())
        } else {
            Self::from_u128(self.ty, self.unsigned_value() * right.unsigned_value())
        };
        result.ok_or(IntegerPanic::OverflowMultiplication)
    }

    pub fn checked_neg(self) -> Result<Self, IntegerPanic> {
        debug_assert!(self.ty.is_signed());
        Self::from_i128(self.ty, -self.signed_value()).ok_or(IntegerPanic::OverflowNegation)
    }

    pub fn divide(self, right: Self) -> Result<Self, IntegerPanic> {
        self.require_same_type(right);
        if right.bits == 0 {
            return Err(IntegerPanic::DivisionByZero);
        }
        if self.ty.is_signed() {
            let left = self.signed_value();
            let right = right.signed_value();
            if left == self.ty.min_value() && right == -1 {
                return Err(IntegerPanic::DivisionOverflow);
            }
            Ok(Self::from_i128(self.ty, left / right).expect("signed quotient must fit"))
        } else {
            Ok(
                Self::from_u128(self.ty, self.unsigned_value() / right.unsigned_value())
                    .expect("unsigned quotient must fit"),
            )
        }
    }

    pub fn remainder(self, right: Self) -> Result<Self, IntegerPanic> {
        self.require_same_type(right);
        if right.bits == 0 {
            return Err(IntegerPanic::RemainderByZero);
        }
        if self.ty.is_signed() {
            let left = self.signed_value();
            let right = right.signed_value();
            if left == self.ty.min_value() && right == -1 {
                return Ok(Self::zero(self.ty));
            }
            Ok(Self::from_i128(self.ty, left % right).expect("signed remainder must fit"))
        } else {
            Ok(
                Self::from_u128(self.ty, self.unsigned_value() % right.unsigned_value())
                    .expect("unsigned remainder must fit"),
            )
        }
    }

    fn shift_count(self) -> Result<u32, IntegerPanic> {
        let count = if self.ty.is_signed() {
            let value = self.signed_value();
            if value < 0 {
                return Err(IntegerPanic::ShiftCountOutOfRange);
            }
            value as u128
        } else {
            self.unsigned_value()
        };
        if count >= self.ty.bit_width() as u128 {
            return Err(IntegerPanic::ShiftCountOutOfRange);
        }
        Ok(count as u32)
    }

    pub fn shift_left(self, right: Self) -> Result<Self, IntegerPanic> {
        self.require_same_type(right);
        let count = right.shift_count()?;
        Ok(Self::from_bits(self.ty, self.bits << count))
    }

    pub fn shift_right(self, right: Self) -> Result<Self, IntegerPanic> {
        self.require_same_type(right);
        let count = right.shift_count()?;
        if self.ty.is_signed() {
            Ok(Self::from_i128(self.ty, self.signed_value() >> count)
                .expect("arithmetic shift result must fit"))
        } else {
            Ok(Self::from_bits(self.ty, self.bits >> count))
        }
    }

    pub fn bitwise_and(self, right: Self) -> Self {
        self.require_same_type(right);
        Self::from_bits(self.ty, self.bits & right.bits)
    }

    pub fn bitwise_or(self, right: Self) -> Self {
        self.require_same_type(right);
        Self::from_bits(self.ty, self.bits | right.bits)
    }

    pub fn bitwise_xor(self, right: Self) -> Self {
        self.require_same_type(right);
        Self::from_bits(self.ty, self.bits ^ right.bits)
    }

    pub fn bitwise_not(self) -> Self {
        Self::from_bits(self.ty, !self.bits)
    }

    pub fn compare(self, right: Self) -> Ordering {
        self.require_same_type(right);
        if self.ty.is_signed() {
            self.signed_value().cmp(&right.signed_value())
        } else {
            self.unsigned_value().cmp(&right.unsigned_value())
        }
    }

    pub fn convert(self, target: IntegerType) -> Result<Self, IntegerPanic> {
        let converted = if self.ty.is_signed() {
            Self::from_i128(target, self.signed_value())
        } else {
            Self::from_u128(target, self.unsigned_value())
        };
        converted.ok_or(IntegerPanic::ConversionOutOfRange)
    }

    fn require_same_type(self, right: Self) {
        debug_assert_eq!(
            self.ty, right.ty,
            "integer engine operands must have one type"
        );
    }
}

impl fmt::Display for IntegerValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ty.is_signed() {
            write!(formatter, "{}", self.signed_value())
        } else {
            write!(formatter, "{}", self.unsigned_value())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerPanic {
    OverflowAddition,
    OverflowSubtraction,
    OverflowMultiplication,
    OverflowNegation,
    DivisionByZero,
    DivisionOverflow,
    RemainderByZero,
    ShiftCountOutOfRange,
    ConversionOutOfRange,
}

impl IntegerPanic {
    pub const fn message(self) -> &'static str {
        match self {
            Self::OverflowAddition => "integer overflow during addition",
            Self::OverflowSubtraction => "integer overflow during subtraction",
            Self::OverflowMultiplication => "integer overflow during multiplication",
            Self::OverflowNegation => "integer overflow during negation",
            Self::DivisionByZero => "integer division by zero",
            Self::DivisionOverflow => "integer division overflow",
            Self::RemainderByZero => "integer remainder by zero",
            Self::ShiftCountOutOfRange => "integer shift count out of range",
            Self::ConversionOutOfRange => "integer conversion out of range",
        }
    }
}

pub fn parse_decimal_magnitude(text: &str) -> Option<u128> {
    text.parse::<u128>().ok()
}
