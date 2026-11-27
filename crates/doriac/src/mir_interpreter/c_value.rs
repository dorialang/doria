//! Values in the target's C layout on the host (Decision 0148 sections 6
//! and 23). A foreign struct or union lives on the host as its bytes in
//! the C layout the shared `crate::c_layout` gives it, and every FFI-safe
//! value converts to and from its C bytes here: a field of an aggregate,
//! an element of a region over C-layout storage, and a native slot, buffer
//! or record of the companion library all use this one codec.

use super::{InterpreterError, LocalValue};
use crate::mir;
use crate::native_closure_abi::{nullable_scalar_form, type_layout, NullableScalarForm};

/// The host's pointer width, which is the companion's and the target's.
pub(super) const POINTER_SIZE: u32 = std::mem::size_of::<usize>() as u32;

/// A foreign struct or union: its bytes in the target's C layout. A copy
/// of the value is a copy of the bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ForeignAggregateValue {
    pub ty: mir::ForeignAggregateType,
    pub bytes: Vec<u8>,
}

impl ForeignAggregateValue {
    /// Every byte zero: `new Name()` of a union, and the start of every
    /// construction.
    pub fn zeroed(ty: mir::ForeignAggregateType) -> Self {
        Self {
            ty,
            bytes: vec![0; ty.size as usize],
        }
    }

    pub fn from_constant(constant: &mir::ForeignAggregateConstant) -> Self {
        Self {
            ty: constant.ty,
            bytes: constant.bytes(),
        }
    }
}

/// The size of a value of `ty` in its C layout.
pub(super) fn size_of(ty: mir::Type) -> usize {
    type_layout(ty, POINTER_SIZE).size as usize
}

/// Writes the C-layout bytes of `value`, of type `ty`, into `slot`, which
/// holds at least [`size_of`] bytes. An absent value, or a local that
/// holds none yet, leaves the slot zero.
pub(super) fn encode(value: Option<&LocalValue>, ty: mir::Type, slot: &mut [u8]) {
    let scalar = |value: mir::ScalarValue, slot: &mut [u8]| {
        let bytes = value.data_bytes();
        slot[..bytes.len()].copy_from_slice(&bytes);
    };
    match (ty, value) {
        (mir::Type::Scalar(_), Some(LocalValue::Scalar(value))) => scalar(*value, slot),
        (
            mir::Type::NullableScalar(stored),
            Some(LocalValue::NullableScalar {
                value: Some(value), ..
            }),
        ) => {
            let form = nullable_scalar_form(stored, POINTER_SIZE);
            if form != NullableScalarForm::ZeroIsNull {
                slot[..POINTER_SIZE as usize].copy_from_slice(&1_usize.to_ne_bytes());
            }
            scalar(*value, &mut slot[form.payload_offset() as usize..]);
        }
        (mir::Type::ForeignAggregate(_), Some(LocalValue::ForeignAggregate(value))) => {
            slot[..value.bytes.len()].copy_from_slice(&value.bytes);
        }
        (
            mir::Type::NullableForeignAggregate(stored),
            Some(LocalValue::NullableForeignAggregate {
                value: Some(value), ..
            }),
        ) => {
            // The shared nullable inline layout: a presence byte at offset
            // zero and the value at its payload offset.
            slot[0] = 1;
            let start = stored.nullable_payload_offset as usize;
            slot[start..start + value.bytes.len()].copy_from_slice(&value.bytes);
        }
        _ => {}
    }
}

/// The value of type `ty` whose C-layout bytes `slot` holds.
pub(super) fn decode(ty: mir::Type, slot: &[u8]) -> Result<LocalValue, InterpreterError> {
    match ty {
        mir::Type::Scalar(stored) => Ok(LocalValue::Scalar(scalar(stored, slot))),
        mir::Type::NullableScalar(stored) => {
            let form = nullable_scalar_form(stored, POINTER_SIZE);
            let payload = &slot[form.payload_offset() as usize..];
            let present = match form {
                NullableScalarForm::ZeroIsNull => payload[..POINTER_SIZE as usize]
                    .iter()
                    .any(|byte| *byte != 0),
                NullableScalarForm::Tagged { .. } => {
                    slot[..POINTER_SIZE as usize].iter().any(|byte| *byte != 0)
                }
            };
            Ok(LocalValue::NullableScalar {
                ty: stored,
                value: present.then(|| scalar(stored, payload)),
            })
        }
        mir::Type::ForeignAggregate(stored) => {
            Ok(LocalValue::ForeignAggregate(ForeignAggregateValue {
                ty: stored,
                bytes: slot[..stored.size as usize].to_vec(),
            }))
        }
        mir::Type::NullableForeignAggregate(stored) => {
            let start = stored.nullable_payload_offset as usize;
            Ok(LocalValue::NullableForeignAggregate {
                ty: stored,
                value: (slot[0] != 0).then(|| ForeignAggregateValue {
                    ty: stored,
                    bytes: slot[start..start + stored.size as usize].to_vec(),
                }),
            })
        }
        _ => Err(InterpreterError::new(format!(
            "a value of type {ty:?} has no C layout"
        ))),
    }
}

fn scalar(ty: mir::ScalarType, slot: &[u8]) -> mir::ScalarValue {
    let word = |size: usize| {
        let mut bytes = [0_u8; 16];
        bytes[..size].copy_from_slice(&slot[..size]);
        u128::from_ne_bytes(bytes)
    };
    match ty {
        mir::ScalarType::Integer(integer) => {
            mir::ScalarValue::Integer(crate::numeric::IntegerValue::from_bits(
                integer,
                word(integer.storage_bytes() as usize) as u64,
            ))
        }
        mir::ScalarType::Float(float) => mir::ScalarValue::Float(crate::numeric::FloatValue {
            ty: float,
            bits: word(float.storage_bytes() as usize) as u64,
        }),
        mir::ScalarType::Bool => mir::ScalarValue::Bool(slot[0] != 0),
        mir::ScalarType::Enum(enum_id) => mir::ScalarValue::Enum(crate::enums::EnumValue {
            enum_id,
            case_id: crate::enums::EnumCaseId {
                enum_id,
                index: word(4) as usize,
            },
        }),
        mir::ScalarType::Inline(inline) => {
            mir::ScalarValue::Inline(crate::inline_value::InlineValue {
                ty: inline,
                bits: word(inline.storage_bytes() as usize),
            })
        }
    }
}

/// The elements `values` of type `element` in their C layout, in storage
/// with at least one word, so its address is never null.
pub(super) fn buffer(values: &[LocalValue], element: mir::Type) -> Vec<u128> {
    let stride = size_of(element);
    let mut buffer = vec![0_u128; (values.len() * stride).div_ceil(16).max(1)];
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), values.len() * stride)
    };
    for (index, value) in values.iter().enumerate() {
        encode(
            Some(value),
            element,
            &mut bytes[index * stride..(index + 1) * stride],
        );
    }
    buffer
}

/// The `length` elements of type `element` at `data`.
///
/// # Safety
///
/// `data` holds `length` elements in their C layout.
pub(super) unsafe fn elements(
    data: *const u8,
    length: usize,
    element: mir::Type,
) -> Result<Vec<LocalValue>, InterpreterError> {
    let stride = size_of(element);
    (0..length)
        .map(|index| {
            decode(
                element,
                std::slice::from_raw_parts(data.add(index * stride), stride),
            )
        })
        .collect()
}

/// Whether a value of `ty` crosses by value in its C layout: a scalar, a
/// nullable scalar, a foreign struct or union, or its nullable form.
pub(super) const fn has_c_layout(ty: mir::Type) -> bool {
    matches!(
        ty,
        mir::Type::Scalar(_)
            | mir::Type::NullableScalar(_)
            | mir::Type::ForeignAggregate(_)
            | mir::Type::NullableForeignAggregate(_)
    )
}
