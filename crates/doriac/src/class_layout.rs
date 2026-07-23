//! Backend-independent native class layout.
//!
//! The layout is private compiler/runtime ABI. Class values themselves are
//! pointer-sized; this module lays out only the headerless property payload.

use crate::numeric::{FloatType, IntegerType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClassId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PropertyId {
    pub class: ClassId,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Integer(IntegerType),
    Float(FloatType),
    Bool,
    String,
    NullableInteger(IntegerType),
    NullableFloat(FloatType),
    NullableBool,
    NullableString,
    Mixed,
    NullableMixed,
    Class(ClassId),
    NullableClass(ClassId),
    Collection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyLayout {
    pub id: PropertyId,
    pub ty: FieldType,
    pub offset: u32,
    pub size: u32,
    pub align: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassLayout {
    pub class: ClassId,
    pub properties: Vec<PropertyLayout>,
    pub size: u32,
    pub align: u32,
}

pub fn compute_class_layout(
    class: ClassId,
    properties: impl IntoIterator<Item = (PropertyId, FieldType)>,
    pointer_size: u32,
) -> ClassLayout {
    assert!(pointer_size.is_power_of_two());
    let mut offset = 0_u32;
    let mut payload_align = 1_u32;
    let mut layouts = Vec::new();

    for (id, ty) in properties {
        assert_eq!(id.class, class);
        let (size, align) = field_size_align(ty, pointer_size);
        offset = align_up(offset, align);
        layouts.push(PropertyLayout {
            id,
            ty,
            offset,
            size,
            align,
        });
        offset = offset
            .checked_add(size)
            .expect("class payload size exceeds the private u32 layout limit");
        payload_align = payload_align.max(align);
    }

    ClassLayout {
        class,
        properties: layouts,
        size: align_up(offset, payload_align),
        align: payload_align,
    }
}

pub const fn field_size_align(ty: FieldType, pointer_size: u32) -> (u32, u32) {
    match ty {
        FieldType::Integer(integer) => {
            let bytes = match integer {
                IntegerType::Int8 | IntegerType::UInt8 => 1,
                IntegerType::Int16 | IntegerType::UInt16 => 2,
                IntegerType::Int32 | IntegerType::UInt32 => 4,
                IntegerType::Int64 | IntegerType::UInt64 => 8,
            };
            (bytes, bytes)
        }
        FieldType::Float(FloatType::Float32) => (4, 4),
        FieldType::Float(FloatType::Float64) => (8, 8),
        FieldType::Bool => (1, 1),
        FieldType::NullableInteger(_)
        | FieldType::NullableFloat(_)
        | FieldType::NullableBool
        | FieldType::NullableString => (pointer_size * 2, pointer_size),
        FieldType::String
        | FieldType::Mixed
        | FieldType::NullableMixed
        | FieldType::Class(_)
        | FieldType::NullableClass(_)
        | FieldType::Collection => (pointer_size, pointer_size),
    }
}

const fn align_up(value: u32, alignment: u32) -> u32 {
    (value + alignment - 1) & !(alignment - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_padded_layouts_are_deterministic() {
        let empty = compute_class_layout(ClassId(0), [], 8);
        assert_eq!((empty.size, empty.align), (0, 1));

        let class = ClassId(1);
        let layout = compute_class_layout(
            class,
            [
                (
                    PropertyId { class, index: 0 },
                    FieldType::Integer(IntegerType::Int8),
                ),
                (PropertyId { class, index: 1 }, FieldType::Class(ClassId(0))),
                (PropertyId { class, index: 2 }, FieldType::Bool),
            ],
            8,
        );
        assert_eq!(layout.properties[0].offset, 0);
        assert_eq!(layout.properties[1].offset, 8);
        assert_eq!(layout.properties[2].offset, 16);
        assert_eq!((layout.size, layout.align), (24, 8));
    }
}
