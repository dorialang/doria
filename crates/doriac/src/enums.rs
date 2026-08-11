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

impl EnumType {
    pub fn new(id: EnumId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
        }
    }
}
