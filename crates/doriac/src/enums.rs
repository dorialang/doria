use crate::numeric::IntegerValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EnumId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EnumCaseId {
    pub enum_id: EnumId,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnumBackingType {
    Int,
    String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EnumBackingValue {
    Int(IntegerValue),
    String(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnumValue {
    pub enum_id: EnumId,
    pub case_id: EnumCaseId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnumType {
    pub id: EnumId,
    pub name: String,
}

/// Physical facts shared by every compiler consumer of an inline enum.
///
/// These are private compiler/runtime ABI facts, not Doria reflection data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnumCapabilities {
    pub copy: bool,
    pub trivial_copy: bool,
    pub needs_drop: bool,
    pub equality: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayoutShape {
    pub size: u32,
    pub align: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnumLayout {
    pub enum_id: EnumId,
    pub tag_width: u32,
    pub tag_offset: u32,
    pub payload_offset: u32,
    pub size: u32,
    pub align: u32,
    pub cases: Vec<EnumCaseLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NullableEnumLayout {
    pub presence_offset: u32,
    pub payload_offset: u32,
    pub size: u32,
    pub align: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnumCaseLayout {
    pub case_id: EnumCaseId,
    pub fields: Vec<EnumPayloadFieldLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnumPayloadFieldLayout {
    pub index: usize,
    /// Absolute byte offset from the beginning of the enum value.
    pub offset: u32,
    pub size: u32,
    pub align: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumLayoutError {
    TooManyCases,
    InvalidAlignment,
    SizeOverflow,
}

pub fn compute_enum_layout(
    enum_id: EnumId,
    case_fields: &[Vec<LayoutShape>],
) -> Result<EnumLayout, EnumLayoutError> {
    let tag_width = tag_width(case_fields.len())?;
    let mut payload_align = 1_u32;
    let mut largest_payload = 0_u32;
    let mut relative_cases = Vec::with_capacity(case_fields.len());

    for (case_index, fields) in case_fields.iter().enumerate() {
        let mut offset = 0_u32;
        let mut layouts = Vec::with_capacity(fields.len());
        for (index, shape) in fields.iter().copied().enumerate() {
            if shape.align == 0 || !shape.align.is_power_of_two() {
                return Err(EnumLayoutError::InvalidAlignment);
            }
            offset = checked_align_up(offset, shape.align)?;
            layouts.push(EnumPayloadFieldLayout {
                index,
                offset,
                size: shape.size,
                align: shape.align,
            });
            offset = offset
                .checked_add(shape.size)
                .ok_or(EnumLayoutError::SizeOverflow)?;
            payload_align = payload_align.max(shape.align);
        }
        largest_payload = largest_payload.max(checked_align_up(offset, payload_align)?);
        relative_cases.push(EnumCaseLayout {
            case_id: EnumCaseId {
                enum_id,
                index: case_index,
            },
            fields: layouts,
        });
    }

    let align = tag_width.max(payload_align);
    let payload_offset = checked_align_up(tag_width, payload_align)?;
    let size = checked_align_up(
        payload_offset
            .checked_add(largest_payload)
            .ok_or(EnumLayoutError::SizeOverflow)?,
        align,
    )?;
    for case in &mut relative_cases {
        for field in &mut case.fields {
            field.offset = payload_offset
                .checked_add(field.offset)
                .ok_or(EnumLayoutError::SizeOverflow)?;
        }
    }

    Ok(EnumLayout {
        enum_id,
        tag_width,
        tag_offset: 0,
        payload_offset,
        size,
        align,
        cases: relative_cases,
    })
}

pub fn nullable_enum_layout(payload: &EnumLayout) -> Result<NullableEnumLayout, EnumLayoutError> {
    let payload_offset = checked_align_up(1, payload.align)?;
    let size = checked_align_up(
        payload_offset
            .checked_add(payload.size)
            .ok_or(EnumLayoutError::SizeOverflow)?,
        payload.align,
    )?;
    Ok(NullableEnumLayout {
        presence_offset: 0,
        payload_offset,
        size,
        align: payload.align,
    })
}

pub fn tag_width(case_count: usize) -> Result<u32, EnumLayoutError> {
    if case_count <= usize::from(u8::MAX) + 1 {
        Ok(1)
    } else if case_count <= usize::from(u16::MAX) + 1 {
        Ok(2)
    } else if case_count <= u32::MAX as usize {
        Ok(4)
    } else {
        Err(EnumLayoutError::TooManyCases)
    }
}

fn checked_align_up(value: u32, alignment: u32) -> Result<u32, EnumLayoutError> {
    value
        .checked_add(alignment - 1)
        .map(|sum| sum & !(alignment - 1))
        .ok_or(EnumLayoutError::SizeOverflow)
}

impl EnumType {
    pub fn new(id: EnumId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_layout_uses_the_largest_case_and_central_tag_width() {
        let enum_id = EnumId(2);
        let layout = compute_enum_layout(
            enum_id,
            &[
                vec![LayoutShape { size: 1, align: 1 }],
                vec![
                    LayoutShape { size: 4, align: 4 },
                    LayoutShape { size: 8, align: 8 },
                ],
            ],
        )
        .expect("finite enum layout");

        assert_eq!(layout.tag_width, 1);
        assert_eq!(layout.payload_offset, 8);
        assert_eq!((layout.size, layout.align), (24, 8));
        assert_eq!(layout.cases[0].fields[0].offset, 8);
        assert_eq!(layout.cases[1].fields[0].offset, 8);
        assert_eq!(layout.cases[1].fields[1].offset, 16);
    }

    #[test]
    fn tag_width_grows_only_when_case_identity_requires_it() {
        assert_eq!(tag_width(256), Ok(1));
        assert_eq!(tag_width(257), Ok(2));
        assert_eq!(tag_width(65_536), Ok(2));
        assert_eq!(tag_width(65_537), Ok(4));
    }

    #[test]
    fn nullable_layout_aligns_the_inline_payload_after_presence() {
        let payload = compute_enum_layout(EnumId(0), &[vec![LayoutShape { size: 8, align: 8 }]])
            .expect("payload layout");
        let nullable = nullable_enum_layout(&payload).expect("nullable layout");

        assert_eq!(nullable.presence_offset, 0);
        assert_eq!(nullable.payload_offset, 8);
        assert_eq!(nullable.size, 24);
        assert_eq!(nullable.align, 8);
    }
}
