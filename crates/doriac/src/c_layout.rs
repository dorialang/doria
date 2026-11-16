//! The C data model of a compilation target (Decision 0148 sections 5 and 6).
//!
//! One module owns the width and signedness of every C boundary type and the
//! layout of every FFI-safe type, so checking (`Name::SIZE`, enum payload
//! layouts), the C ABI classification (`native_abi::c_abi`) and both native
//! backends read the same facts. Native code generation targets the host, so
//! [`CTarget::host`] is the target every consumer passes today; nothing here
//! assumes it.

use std::sync::Arc;

use crate::ast::ForeignAggregateKind;
use crate::foreign::CBoundaryType;

/// The operating systems of the tier-1 targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetOperatingSystem {
    Linux,
    MacOs,
    Windows,
}

/// The architectures of the tier-1 targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetArchitecture {
    X8664,
    Aarch64,
}

/// The C calling convention `extern "C"` means on a target (section 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CCallingConvention {
    /// System V x86_64: Linux and macOS on x86_64.
    SystemV,
    /// Windows x64.
    Windows64,
    /// AAPCS64: Linux on aarch64.
    Aapcs64,
    /// Apple arm64: macOS on aarch64.
    AppleArm64,
}

/// A tier-1 compilation target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CTarget {
    pub operating_system: TargetOperatingSystem,
    pub architecture: TargetArchitecture,
}

impl CTarget {
    /// The target `doriac` itself runs on, which native code generation
    /// targets.
    pub const fn host() -> Self {
        let operating_system = if cfg!(target_os = "windows") {
            TargetOperatingSystem::Windows
        } else if cfg!(target_os = "macos") {
            TargetOperatingSystem::MacOs
        } else {
            TargetOperatingSystem::Linux
        };
        let architecture = if cfg!(target_arch = "aarch64") {
            TargetArchitecture::Aarch64
        } else {
            TargetArchitecture::X8664
        };
        Self {
            operating_system,
            architecture,
        }
    }

    pub const fn calling_convention(self) -> CCallingConvention {
        match (self.operating_system, self.architecture) {
            (TargetOperatingSystem::Windows, _) => CCallingConvention::Windows64,
            (_, TargetArchitecture::X8664) => CCallingConvention::SystemV,
            (TargetOperatingSystem::MacOs, TargetArchitecture::Aarch64) => {
                CCallingConvention::AppleArm64
            }
            (TargetOperatingSystem::Linux, TargetArchitecture::Aarch64) => {
                CCallingConvention::Aapcs64
            }
        }
    }

    /// Every tier-1 target has 64-bit pointers.
    pub const fn pointer_bytes(self) -> u64 {
        8
    }

    /// The C type a boundary type stands for on this target (section 5):
    /// `long` and `unsigned long` are 32-bit on Windows and 64-bit
    /// elsewhere; `char` is unsigned on Linux aarch64 and signed on the other
    /// targets; `wchar_t` is unsigned 16-bit on Windows, unsigned 32-bit on
    /// Linux aarch64 and signed 32-bit elsewhere; `size_t` and `ptrdiff_t`
    /// are pointer-width.
    pub const fn boundary(self, boundary: CBoundaryType) -> CScalar {
        let windows = matches!(self.operating_system, TargetOperatingSystem::Windows);
        let linux_aarch64 = matches!(
            (self.operating_system, self.architecture),
            (TargetOperatingSystem::Linux, TargetArchitecture::Aarch64)
        );
        match boundary {
            CBoundaryType::Char => CScalar::Integer {
                bytes: 1,
                signed: !linux_aarch64,
            },
            CBoundaryType::Long => CScalar::Integer {
                bytes: if windows { 4 } else { 8 },
                signed: true,
            },
            CBoundaryType::UnsignedLong => CScalar::Integer {
                bytes: if windows { 4 } else { 8 },
                signed: false,
            },
            CBoundaryType::Size => CScalar::Integer {
                bytes: 8,
                signed: false,
            },
            CBoundaryType::PointerDifference => CScalar::Integer {
                bytes: 8,
                signed: true,
            },
            CBoundaryType::WideChar => {
                if windows {
                    CScalar::Integer {
                        bytes: 2,
                        signed: false,
                    }
                } else {
                    CScalar::Integer {
                        bytes: 4,
                        signed: !linux_aarch64,
                    }
                }
            }
        }
    }
}

/// A C scalar: what one FFI-safe value that is not an aggregate occupies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CScalar {
    Integer {
        bytes: u8,
        signed: bool,
    },
    Float {
        bytes: u8,
    },
    /// `_Bool`.
    Bool,
    /// A data or function pointer.
    Pointer,
}

impl CScalar {
    /// On every tier-1 target a scalar's alignment equals its size.
    pub const fn size(self) -> u64 {
        match self {
            Self::Integer { bytes, .. } | Self::Float { bytes } => bytes as u64,
            Self::Bool => 1,
            Self::Pointer => 8,
        }
    }

    pub const fn is_float(self) -> bool {
        matches!(self, Self::Float { .. })
    }

    /// The Doria integer type with this scalar's width and signedness;
    /// `None` for a float, `_Bool` or a pointer.
    pub const fn integer_type(self) -> Option<crate::numeric::IntegerType> {
        use crate::numeric::IntegerType;
        match self {
            Self::Integer { bytes, signed } => Some(match (bytes, signed) {
                (1, true) => IntegerType::Int8,
                (1, false) => IntegerType::UInt8,
                (2, true) => IntegerType::Int16,
                (2, false) => IntegerType::UInt16,
                (4, true) => IntegerType::Int32,
                (4, false) => IntegerType::UInt32,
                (_, true) => IntegerType::Int64,
                (_, false) => IntegerType::UInt64,
            }),
            Self::Float { .. } | Self::Bool | Self::Pointer => None,
        }
    }
}

impl CTarget {
    /// The integer a boundary value is on this target: what MIR, the
    /// interpreter and both native backends carry for it.
    pub const fn boundary_integer(self, boundary: CBoundaryType) -> crate::numeric::IntegerType {
        match self.boundary(boundary).integer_type() {
            Some(ty) => ty,
            None => panic!("every C boundary type is an integer"),
        }
    }
}

/// The C layout of an FFI-safe type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CType {
    Scalar(CScalar),
    Record(Arc<CRecord>),
    /// A fixed-length array field, `T[N]`.
    Array {
        element: Box<CType>,
        length: u64,
    },
}

impl CType {
    pub fn size(&self) -> u64 {
        match self {
            Self::Scalar(scalar) => scalar.size(),
            Self::Record(record) => record.size,
            Self::Array { element, length } => element.size() * length,
        }
    }

    pub fn align(&self) -> u64 {
        match self {
            Self::Scalar(scalar) => scalar.size(),
            Self::Record(record) => record.align,
            Self::Array { element, .. } => element.align(),
        }
    }

    /// Every scalar this type contains with its byte offset, in memory
    /// order; union members all start at offset zero. The ABI
    /// classification reads aggregates through this.
    pub fn scalars(&self) -> Vec<(u64, CScalar)> {
        let mut scalars = Vec::new();
        self.collect_scalars(0, &mut scalars);
        scalars
    }

    fn collect_scalars(&self, base: u64, scalars: &mut Vec<(u64, CScalar)>) {
        match self {
            Self::Scalar(scalar) => scalars.push((base, *scalar)),
            Self::Record(record) => {
                for field in &record.fields {
                    field.ty.collect_scalars(base + field.offset, scalars);
                }
            }
            Self::Array { element, length } => {
                let stride = element.size();
                for index in 0..*length {
                    element.collect_scalars(base + index * stride, scalars);
                }
            }
        }
    }
}

/// A foreign struct or union laid out by the target C rules.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CRecord {
    pub kind: ForeignAggregateKind,
    pub fields: Vec<CField>,
    pub size: u64,
    pub align: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CField {
    pub offset: u64,
    pub ty: CType,
}

impl CRecord {
    /// Lays out `fields` in declaration order: a struct places each field at
    /// the next offset aligned for it, a union places every member at
    /// offset zero, and both round the size up to the largest member
    /// alignment. `None` when the size does not fit in 64 bits.
    pub fn new(kind: ForeignAggregateKind, fields: Vec<CType>) -> Option<Self> {
        let mut offset = 0u64;
        let mut size = 0u64;
        let mut align = 1u64;
        let mut laid_out = Vec::with_capacity(fields.len());
        for ty in fields {
            let field_size = checked_size(&ty)?;
            let field_align = ty.align();
            align = align.max(field_align);
            let field_offset = match kind {
                ForeignAggregateKind::Struct => {
                    let start = align_up(offset, field_align)?;
                    offset = start.checked_add(field_size)?;
                    start
                }
                ForeignAggregateKind::Union => 0,
            };
            size = size.max(field_offset.checked_add(field_size)?);
            laid_out.push(CField {
                offset: field_offset,
                ty,
            });
        }
        Some(Self {
            kind,
            fields: laid_out,
            size: align_up(size, align)?,
            align,
        })
    }
}

/// The size of `ty`, or `None` when an array's size overflows.
fn checked_size(ty: &CType) -> Option<u64> {
    match ty {
        CType::Scalar(scalar) => Some(scalar.size()),
        CType::Record(record) => Some(record.size),
        CType::Array { element, length } => checked_size(element)?.checked_mul(*length),
    }
}

pub(crate) fn align_up(value: u64, align: u64) -> Option<u64> {
    debug_assert!(align.is_power_of_two());
    value.checked_add(align - 1).map(|sum| sum & !(align - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn target(
        operating_system: TargetOperatingSystem,
        architecture: TargetArchitecture,
    ) -> CTarget {
        CTarget {
            operating_system,
            architecture,
        }
    }

    const LINUX_X86: CTarget = target(TargetOperatingSystem::Linux, TargetArchitecture::X8664);
    const LINUX_ARM: CTarget = target(TargetOperatingSystem::Linux, TargetArchitecture::Aarch64);
    const MAC_ARM: CTarget = target(TargetOperatingSystem::MacOs, TargetArchitecture::Aarch64);
    const MAC_X86: CTarget = target(TargetOperatingSystem::MacOs, TargetArchitecture::X8664);
    const WINDOWS: CTarget = target(TargetOperatingSystem::Windows, TargetArchitecture::X8664);

    fn int(bytes: u8, signed: bool) -> CType {
        CType::Scalar(CScalar::Integer { bytes, signed })
    }

    #[test]
    fn boundary_types_follow_each_target_data_model() {
        let long = |target: CTarget| target.boundary(CBoundaryType::Long).size();
        assert_eq!(
            [LINUX_X86, LINUX_ARM, MAC_ARM, MAC_X86, WINDOWS].map(long),
            [8, 8, 8, 8, 4]
        );
        let char_signed = |target: CTarget| {
            matches!(
                target.boundary(CBoundaryType::Char),
                CScalar::Integer { signed: true, .. }
            )
        };
        assert_eq!(
            [LINUX_X86, LINUX_ARM, MAC_ARM, MAC_X86, WINDOWS].map(char_signed),
            [true, false, true, true, true]
        );
        assert_eq!(
            WINDOWS.boundary(CBoundaryType::WideChar),
            CScalar::Integer {
                bytes: 2,
                signed: false
            }
        );
        assert_eq!(
            LINUX_ARM.boundary(CBoundaryType::WideChar),
            CScalar::Integer {
                bytes: 4,
                signed: false
            }
        );
        assert_eq!(
            MAC_ARM.boundary(CBoundaryType::WideChar),
            CScalar::Integer {
                bytes: 4,
                signed: true
            }
        );
        for target in [LINUX_X86, LINUX_ARM, MAC_ARM, MAC_X86, WINDOWS] {
            assert_eq!(target.boundary(CBoundaryType::Size).size(), 8);
            assert_eq!(target.boundary(CBoundaryType::PointerDifference).size(), 8);
        }
        assert_eq!(
            [LINUX_X86, LINUX_ARM, MAC_ARM, MAC_X86, WINDOWS].map(CTarget::calling_convention),
            [
                CCallingConvention::SystemV,
                CCallingConvention::Aapcs64,
                CCallingConvention::AppleArm64,
                CCallingConvention::SystemV,
                CCallingConvention::Windows64,
            ]
        );
    }

    #[test]
    fn structs_pad_each_field_and_round_to_their_alignment() {
        // struct { char tag; int64_t value; uint16_t extra; }
        let record = CRecord::new(
            ForeignAggregateKind::Struct,
            vec![int(1, true), int(8, true), int(2, false)],
        )
        .expect("fits");
        assert_eq!(
            record
                .fields
                .iter()
                .map(|field| field.offset)
                .collect::<Vec<_>>(),
            [0, 8, 16]
        );
        assert_eq!((record.size, record.align), (24, 8));
        // struct { uint8_t red, green, blue, alpha; }
        let color = CRecord::new(ForeignAggregateKind::Struct, vec![int(1, false); 4]).unwrap();
        assert_eq!((color.size, color.align), (4, 1));
    }

    #[test]
    fn unions_overlay_members_and_take_the_largest() {
        // union { int32_t key; uint16_t tip[3]; double value; }
        let union = CRecord::new(
            ForeignAggregateKind::Union,
            vec![
                int(4, true),
                CType::Array {
                    element: Box::new(int(2, false)),
                    length: 3,
                },
                CType::Scalar(CScalar::Float { bytes: 8 }),
            ],
        )
        .unwrap();
        assert!(union.fields.iter().all(|field| field.offset == 0));
        assert_eq!((union.size, union.align), (8, 8));
        // union { char bytes[5]; int32_t word; } rounds 5 up to 8.
        let rounded = CRecord::new(
            ForeignAggregateKind::Union,
            vec![
                CType::Array {
                    element: Box::new(int(1, true)),
                    length: 5,
                },
                int(4, true),
            ],
        )
        .unwrap();
        assert_eq!((rounded.size, rounded.align), (8, 4));
    }

    #[test]
    fn nested_records_and_arrays_list_their_scalars_in_memory_order() {
        let point = Arc::new(
            CRecord::new(
                ForeignAggregateKind::Struct,
                vec![
                    CType::Scalar(CScalar::Float { bytes: 4 }),
                    CType::Scalar(CScalar::Float { bytes: 4 }),
                ],
            )
            .unwrap(),
        );
        let segment = CType::Record(Arc::new(
            CRecord::new(
                ForeignAggregateKind::Struct,
                vec![
                    CType::Record(point.clone()),
                    CType::Array {
                        element: Box::new(CType::Record(point)),
                        length: 2,
                    },
                ],
            )
            .unwrap(),
        ));
        assert_eq!(segment.size(), 24);
        assert_eq!(
            segment
                .scalars()
                .iter()
                .map(|(offset, _)| *offset)
                .collect::<Vec<_>>(),
            [0, 4, 8, 12, 16, 20]
        );
        assert!(CRecord::new(
            ForeignAggregateKind::Struct,
            vec![CType::Array {
                element: Box::new(int(8, true)),
                length: u64::MAX / 4,
            }],
        )
        .is_none());
    }
}
