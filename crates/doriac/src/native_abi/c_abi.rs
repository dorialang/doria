//! The one C ABI classification (Decision 0148 sections 6 and 14).
//!
//! Given a foreign signature's C layouts, [`classify`] decides where each
//! argument and the return value travel under the target's C convention:
//! System V x86_64, Windows x64, AAPCS64 or Apple arm64. Cranelift and LLVM
//! both consume the result; neither classifies a C signature itself.
//!
//! The classification counts registers as the convention does, so a backend
//! never has to: an argument the convention places on the stack is reported
//! as [`ArgumentPassing::Stack`] even when a backend's own register counter
//! would still have a register free (an AArch64 argument after a composite
//! or homogeneous floating-point aggregate that did not fit, and every
//! variadic argument on Apple arm64).

use crate::c_layout::{CCallingConvention, CScalar, CTarget, CType};

/// The register class of one part of an aggregate passed in registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartClass {
    /// A general-purpose register.
    Integer,
    /// A floating-point or vector register.
    Float,
}

/// One register's worth of an aggregate's memory image: `size` bytes at
/// `offset`. An integer part is loaded into the low bytes of a 64-bit
/// register; a float part of 4 bytes is a `float32`, of 8 bytes a
/// `float64` (on System V, two `float32` fields share one 8-byte part, as
/// the convention packs them into one register).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegisterPart {
    pub offset: u64,
    pub size: u64,
    pub class: PartClass,
}

/// How one argument travels.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArgumentPassing {
    /// A scalar in the next register of its class, or the next stack slot
    /// when that class has none left. An integer narrower than 32 bits, and
    /// `bool`, is extended to 32 bits by the caller ([`extension`]).
    Scalar(CScalar),
    /// An aggregate split into register parts, each loaded from its memory
    /// image.
    Registers(Vec<RegisterPart>),
    /// The value's memory image, rounded up to `size` bytes, in the
    /// argument area on the stack.
    Stack { size: u64, align: u64 },
    /// A pointer to a copy the caller makes, passed as a pointer argument;
    /// `on_stack` places that pointer on the stack (an Apple arm64 variadic
    /// argument).
    IndirectCopy { on_stack: bool },
}

/// How the return value travels.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReturnPassing {
    Void,
    Scalar(CScalar),
    Registers(Vec<RegisterPart>),
    /// The caller passes the address of storage for the result; see
    /// [`indirect_return_takes_first_argument`].
    Indirect,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SignatureClassification {
    pub parameters: Vec<ArgumentPassing>,
    pub return_value: ReturnPassing,
}

/// How a narrow integer or `bool` argument or result is widened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Extension {
    Sign,
    Zero,
}

/// The widening the caller applies to a scalar narrower than 32 bits. Apple
/// arm64 requires it; the other conventions leave the upper bits
/// unspecified, so widening is correct on every target.
pub const fn extension(scalar: CScalar) -> Option<Extension> {
    match scalar {
        CScalar::Integer { bytes, signed } if bytes < 4 => Some(if signed {
            Extension::Sign
        } else {
            Extension::Zero
        }),
        CScalar::Bool => Some(Extension::Zero),
        _ => None,
    }
}

/// Whether the result address of an [`ReturnPassing::Indirect`] return
/// takes the first argument register (System V, Windows x64) rather than a
/// dedicated register (`x8` on AArch64).
pub const fn indirect_return_takes_first_argument(convention: CCallingConvention) -> bool {
    matches!(
        convention,
        CCallingConvention::SystemV | CCallingConvention::Windows64
    )
}

/// Classifies a foreign signature. `return_type` is `None` for `void`;
/// `fixed_parameters` is `Some(n)` for a variadic binding whose first `n`
/// parameters are the fixed ones (section 14).
pub fn classify(
    target: CTarget,
    parameters: &[CType],
    return_type: Option<&CType>,
    fixed_parameters: Option<usize>,
) -> SignatureClassification {
    let convention = target.calling_convention();
    let return_value = match return_type {
        None => ReturnPassing::Void,
        Some(ty) => classify_return(convention, ty),
    };
    let parameters = match convention {
        CCallingConvention::SystemV => system_v_parameters(parameters, &return_value),
        CCallingConvention::Windows64 => parameters.iter().map(windows_parameter).collect(),
        CCallingConvention::Aapcs64 | CCallingConvention::AppleArm64 => aarch64_parameters(
            parameters,
            fixed_parameters.filter(|_| convention == CCallingConvention::AppleArm64),
        ),
    };
    SignatureClassification {
        parameters,
        return_value,
    }
}

fn classify_return(convention: CCallingConvention, ty: &CType) -> ReturnPassing {
    if let CType::Scalar(scalar) = ty {
        return ReturnPassing::Scalar(*scalar);
    }
    match convention {
        CCallingConvention::SystemV => match system_v_parts(ty) {
            Some(parts) => ReturnPassing::Registers(parts),
            None => ReturnPassing::Indirect,
        },
        CCallingConvention::Windows64 => match windows_parts(ty) {
            Some(parts) => ReturnPassing::Registers(parts),
            None => ReturnPassing::Indirect,
        },
        CCallingConvention::Aapcs64 | CCallingConvention::AppleArm64 => {
            if let Some(parts) = homogeneous_float_parts(ty) {
                ReturnPassing::Registers(parts)
            } else if ty.size() <= 16 {
                ReturnPassing::Registers(integer_parts(ty.size()))
            } else {
                ReturnPassing::Indirect
            }
        }
    }
}

// System V x86_64 -----------------------------------------------------------

const SYSTEM_V_INTEGER_REGISTERS: usize = 6;
const SYSTEM_V_FLOAT_REGISTERS: usize = 8;

/// The eightbyte classification of an aggregate of at most 16 bytes: an
/// eightbyte holding any integer is INTEGER, one holding only floats is
/// SSE. `None` is the MEMORY class.
fn system_v_parts(ty: &CType) -> Option<Vec<RegisterPart>> {
    let size = ty.size();
    if size == 0 || size > 16 {
        return None;
    }
    let count = size.div_ceil(8) as usize;
    let mut classes = vec![None; count];
    for (offset, scalar) in ty.scalars() {
        let slot = &mut classes[(offset / 8) as usize];
        *slot = Some(match (*slot, scalar.is_float()) {
            (Some(PartClass::Integer), _) | (_, false) => PartClass::Integer,
            _ => PartClass::Float,
        });
    }
    Some(
        classes
            .into_iter()
            .enumerate()
            .filter_map(|(index, class)| {
                let offset = index as u64 * 8;
                Some(RegisterPart {
                    offset,
                    size: (size - offset).min(8),
                    class: class?,
                })
            })
            .collect(),
    )
}

fn system_v_parameters(parameters: &[CType], return_value: &ReturnPassing) -> Vec<ArgumentPassing> {
    let mut integers = SYSTEM_V_INTEGER_REGISTERS;
    let mut floats = SYSTEM_V_FLOAT_REGISTERS;
    if *return_value == ReturnPassing::Indirect {
        integers -= 1;
    }
    parameters
        .iter()
        .map(|ty| match ty {
            CType::Scalar(scalar) => {
                let available = if scalar.is_float() {
                    &mut floats
                } else {
                    &mut integers
                };
                *available = available.saturating_sub(1);
                ArgumentPassing::Scalar(*scalar)
            }
            aggregate => {
                let stack = ArgumentPassing::Stack {
                    size: aggregate.size().div_ceil(8) * 8,
                    align: aggregate.align().max(8),
                };
                let Some(parts) = system_v_parts(aggregate) else {
                    return stack;
                };
                let needed = |class| parts.iter().filter(|part| part.class == class).count();
                let (needed_integers, needed_floats) =
                    (needed(PartClass::Integer), needed(PartClass::Float));
                // An aggregate that does not fit the registers left goes
                // to memory whole; later arguments may still use them.
                if needed_integers > integers || needed_floats > floats {
                    return stack;
                }
                integers -= needed_integers;
                floats -= needed_floats;
                ArgumentPassing::Registers(parts)
            }
        })
        .collect()
}

// Windows x64 ---------------------------------------------------------------

/// An aggregate of 1, 2, 4 or 8 bytes travels as an integer of that size;
/// any other size travels by reference to a copy.
fn windows_parts(ty: &CType) -> Option<Vec<RegisterPart>> {
    let size = ty.size();
    matches!(size, 1 | 2 | 4 | 8).then(|| {
        vec![RegisterPart {
            offset: 0,
            size,
            class: PartClass::Integer,
        }]
    })
}

fn windows_parameter(ty: &CType) -> ArgumentPassing {
    match ty {
        CType::Scalar(scalar) => ArgumentPassing::Scalar(*scalar),
        aggregate => match windows_parts(aggregate) {
            Some(parts) => ArgumentPassing::Registers(parts),
            None => ArgumentPassing::IndirectCopy { on_stack: false },
        },
    }
}

// AAPCS64 and Apple arm64 ---------------------------------------------------

const AARCH64_REGISTERS: usize = 8;

/// A homogeneous floating-point aggregate: one to four members, every one
/// the same floating-point type.
fn homogeneous_float_parts(ty: &CType) -> Option<Vec<RegisterPart>> {
    let scalars = ty.scalars();
    let (_, first) = *scalars.first()?;
    let CScalar::Float { bytes } = first else {
        return None;
    };
    let homogeneous = scalars.len() <= 4
        && scalars.iter().all(|(_, scalar)| *scalar == first)
        && ty.size() == bytes as u64 * scalars.len() as u64;
    homogeneous.then(|| {
        scalars
            .into_iter()
            .map(|(offset, _)| RegisterPart {
                offset,
                size: bytes as u64,
                class: PartClass::Float,
            })
            .collect()
    })
}

/// A composite of at most 16 bytes in consecutive general-purpose
/// registers, 8 bytes each.
fn integer_parts(size: u64) -> Vec<RegisterPart> {
    (0..size.div_ceil(8))
        .map(|index| RegisterPart {
            offset: index * 8,
            size: (size - index * 8).min(8),
            class: PartClass::Integer,
        })
        .collect()
}

/// The registers of one class, as the convention counts them and as a
/// backend counts the arguments it is given in registers. They differ
/// once an argument that did not fit has closed the class (NGRN or NSRN
/// set to 8): the convention then places every later argument of the class
/// on the stack, which a backend does only when told.
struct RegisterFile {
    convention: usize,
    backend: usize,
}

impl RegisterFile {
    const fn new() -> Self {
        Self {
            convention: AARCH64_REGISTERS,
            backend: AARCH64_REGISTERS,
        }
    }

    fn take(&mut self, count: usize) -> bool {
        if count <= self.convention {
            self.convention -= count;
            self.backend -= count;
            true
        } else {
            self.convention = 0;
            false
        }
    }

    /// Whether a scalar the convention places on the stack would reach a
    /// register in a backend that was not told.
    const fn backend_has_register(&self) -> bool {
        self.backend > 0
    }
}

fn aarch64_parameters(
    parameters: &[CType],
    apple_fixed_parameters: Option<usize>,
) -> Vec<ArgumentPassing> {
    let mut integers = RegisterFile::new();
    let mut floats = RegisterFile::new();
    parameters
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            let slot = |ty: &CType| ArgumentPassing::Stack {
                size: ty.size().div_ceil(8) * 8,
                align: 8,
            };
            // Apple arm64 passes every variadic argument on the stack, one
            // 8-byte slot each; a composite over 16 bytes by reference.
            if apple_fixed_parameters.is_some_and(|fixed| index >= fixed) {
                return if !matches!(ty, CType::Scalar(_)) && ty.size() > 16 {
                    ArgumentPassing::IndirectCopy { on_stack: true }
                } else {
                    slot(ty)
                };
            }
            match ty {
                CType::Scalar(scalar) => {
                    let file = if scalar.is_float() {
                        &mut floats
                    } else {
                        &mut integers
                    };
                    let backend_register = file.backend_has_register();
                    if file.take(1) || !backend_register {
                        ArgumentPassing::Scalar(*scalar)
                    } else {
                        ArgumentPassing::Stack {
                            size: scalar.size(),
                            align: scalar.size(),
                        }
                    }
                }
                aggregate => {
                    if let Some(parts) = homogeneous_float_parts(aggregate) {
                        return if floats.take(parts.len()) {
                            ArgumentPassing::Registers(parts)
                        } else {
                            slot(aggregate)
                        };
                    }
                    if aggregate.size() > 16 {
                        let backend_register = integers.backend_has_register();
                        return ArgumentPassing::IndirectCopy {
                            on_stack: !integers.take(1) && backend_register,
                        };
                    }
                    let parts = integer_parts(aggregate.size());
                    if integers.take(parts.len()) {
                        ArgumentPassing::Registers(parts)
                    } else {
                        slot(aggregate)
                    }
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ast::ForeignAggregateKind;
    use crate::c_layout::{CRecord, TargetArchitecture, TargetOperatingSystem};

    const LINUX_X86: CTarget = CTarget {
        operating_system: TargetOperatingSystem::Linux,
        architecture: TargetArchitecture::X8664,
    };
    const LINUX_ARM: CTarget = CTarget {
        operating_system: TargetOperatingSystem::Linux,
        architecture: TargetArchitecture::Aarch64,
    };
    const MAC_ARM: CTarget = CTarget {
        operating_system: TargetOperatingSystem::MacOs,
        architecture: TargetArchitecture::Aarch64,
    };
    const WINDOWS: CTarget = CTarget {
        operating_system: TargetOperatingSystem::Windows,
        architecture: TargetArchitecture::X8664,
    };

    const INT32: CType = CType::Scalar(CScalar::Integer {
        bytes: 4,
        signed: true,
    });
    const INT64: CType = CType::Scalar(CScalar::Integer {
        bytes: 8,
        signed: true,
    });
    const FLOAT32: CType = CType::Scalar(CScalar::Float { bytes: 4 });
    const FLOAT64: CType = CType::Scalar(CScalar::Float { bytes: 8 });

    fn record(fields: Vec<CType>) -> CType {
        CType::Record(Arc::new(
            CRecord::new(ForeignAggregateKind::Struct, fields).expect("fits"),
        ))
    }

    fn part(offset: u64, size: u64, class: PartClass) -> RegisterPart {
        RegisterPart {
            offset,
            size,
            class,
        }
    }

    #[test]
    fn system_v_splits_small_aggregates_into_eightbytes() {
        // raylib's Vector2 { float x, y; } shares one SSE register.
        let vector2 = record(vec![FLOAT32, FLOAT32]);
        // { int32_t id; float weight; double scale; }
        let mixed = record(vec![INT32, FLOAT32, FLOAT64]);
        let classification = classify(LINUX_X86, &[vector2.clone(), mixed], Some(&vector2), None);
        assert_eq!(
            classification.parameters,
            [
                ArgumentPassing::Registers(vec![part(0, 8, PartClass::Float)]),
                ArgumentPassing::Registers(vec![
                    part(0, 8, PartClass::Integer),
                    part(8, 8, PartClass::Float)
                ]),
            ]
        );
        assert_eq!(
            classification.return_value,
            ReturnPassing::Registers(vec![part(0, 8, PartClass::Float)])
        );
        // { char a, b, c; } is one 3-byte INTEGER part.
        let three = record(vec![
            CType::Scalar(CScalar::Integer {
                bytes: 1,
                signed: true
            });
            3
        ]);
        assert_eq!(
            classify(LINUX_X86, &[three], None, None).parameters,
            [ArgumentPassing::Registers(vec![part(
                0,
                3,
                PartClass::Integer
            )])]
        );
    }

    #[test]
    fn system_v_sends_large_or_unfitting_aggregates_to_memory() {
        let large = record(vec![INT64, INT64, INT64]);
        let pair = record(vec![INT64, INT64]);
        let classification = classify(
            LINUX_X86,
            &[INT64, INT64, INT64, INT64, INT64, pair.clone(), INT64],
            Some(&large),
            None,
        );
        // The hidden result address takes one of six integer registers, the
        // five scalars take the rest, so the pair goes to memory and the
        // last scalar still reaches the stack in order.
        assert_eq!(classification.return_value, ReturnPassing::Indirect);
        assert_eq!(
            classification.parameters[5],
            ArgumentPassing::Stack { size: 16, align: 8 }
        );
        assert_eq!(
            classify(LINUX_X86, &[large], None, None).parameters,
            [ArgumentPassing::Stack { size: 24, align: 8 }]
        );
        assert!(indirect_return_takes_first_argument(
            LINUX_X86.calling_convention()
        ));
    }

    #[test]
    fn windows_passes_power_of_two_aggregates_as_integers() {
        let vector2 = record(vec![FLOAT32, FLOAT32]);
        let color = record(vec![
            CType::Scalar(CScalar::Integer {
                bytes: 1,
                signed: false
            });
            4
        ]);
        let three = record(vec![FLOAT32, FLOAT32, FLOAT32]);
        let classification = classify(
            WINDOWS,
            &[vector2, color, three.clone(), FLOAT64],
            Some(&three),
            None,
        );
        assert_eq!(
            classification.parameters,
            [
                ArgumentPassing::Registers(vec![part(0, 8, PartClass::Integer)]),
                ArgumentPassing::Registers(vec![part(0, 4, PartClass::Integer)]),
                ArgumentPassing::IndirectCopy { on_stack: false },
                ArgumentPassing::Scalar(CScalar::Float { bytes: 8 }),
            ]
        );
        assert_eq!(classification.return_value, ReturnPassing::Indirect);
    }

    #[test]
    fn aarch64_uses_float_registers_for_homogeneous_aggregates() {
        let vector3 = record(vec![FLOAT32, FLOAT32, FLOAT32]);
        let rectangle = record(vec![FLOAT64, FLOAT64, FLOAT64, FLOAT64]);
        let mixed = record(vec![INT32, FLOAT32]);
        for target in [LINUX_ARM, MAC_ARM] {
            let classification = classify(
                target,
                &[vector3.clone(), mixed.clone(), rectangle.clone()],
                Some(&rectangle),
                None,
            );
            assert_eq!(
                classification.parameters,
                [
                    ArgumentPassing::Registers(vec![
                        part(0, 4, PartClass::Float),
                        part(4, 4, PartClass::Float),
                        part(8, 4, PartClass::Float)
                    ]),
                    ArgumentPassing::Registers(vec![part(0, 8, PartClass::Integer)]),
                    ArgumentPassing::Registers(
                        (0..4).map(|i| part(i * 8, 8, PartClass::Float)).collect()
                    ),
                ]
            );
            assert!(matches!(
                classification.return_value,
                ReturnPassing::Registers(ref parts) if parts.len() == 4
            ));
            assert!(!indirect_return_takes_first_argument(
                target.calling_convention()
            ));
        }
        let large = record(vec![INT64, INT64, INT64]);
        let classification = classify(LINUX_ARM, std::slice::from_ref(&large), Some(&large), None);
        assert_eq!(
            classification.parameters,
            [ArgumentPassing::IndirectCopy { on_stack: false }]
        );
        assert_eq!(classification.return_value, ReturnPassing::Indirect);
    }

    #[test]
    fn aarch64_closes_a_register_class_after_an_argument_that_did_not_fit() {
        let pair = record(vec![INT64, INT64]);
        let mut parameters = vec![INT64; 7];
        parameters.push(pair);
        parameters.push(INT32);
        let classification = classify(LINUX_ARM, &parameters, None, None);
        // Seven scalars leave one register; the pair needs two, so it goes
        // to the stack and closes the integer registers: the last scalar is
        // on the stack too, although one register is still free.
        assert_eq!(
            classification.parameters[7],
            ArgumentPassing::Stack { size: 16, align: 8 }
        );
        assert_eq!(
            classification.parameters[8],
            ArgumentPassing::Stack { size: 4, align: 4 }
        );
        // System V keeps the register for later arguments.
        let classification = classify(LINUX_X86, &parameters[2..], None, None);
        assert_eq!(
            classification.parameters[6],
            ArgumentPassing::Scalar(CScalar::Integer {
                bytes: 4,
                signed: true
            })
        );
    }

    #[test]
    fn apple_arm64_places_variadic_arguments_on_the_stack() {
        let pointer = CType::Scalar(CScalar::Pointer);
        let classification = classify(
            MAC_ARM,
            &[pointer.clone(), INT32, FLOAT64],
            Some(&INT32),
            Some(1),
        );
        assert_eq!(
            classification.parameters,
            [
                ArgumentPassing::Scalar(CScalar::Pointer),
                ArgumentPassing::Stack { size: 8, align: 8 },
                ArgumentPassing::Stack { size: 8, align: 8 },
            ]
        );
        // AAPCS64 on Linux passes variadic arguments as fixed ones.
        let classification = classify(LINUX_ARM, &[pointer, INT32], Some(&INT32), Some(1));
        assert!(classification
            .parameters
            .iter()
            .all(|passing| matches!(passing, ArgumentPassing::Scalar(_))));
    }

    #[test]
    fn narrow_integers_and_bools_are_widened() {
        assert_eq!(
            extension(CScalar::Integer {
                bytes: 1,
                signed: true
            }),
            Some(Extension::Sign)
        );
        assert_eq!(
            extension(CScalar::Integer {
                bytes: 2,
                signed: false
            }),
            Some(Extension::Zero)
        );
        assert_eq!(extension(CScalar::Bool), Some(Extension::Zero));
        assert_eq!(
            extension(CScalar::Integer {
                bytes: 4,
                signed: true
            }),
            None
        );
        assert_eq!(extension(CScalar::Pointer), None);
    }
}
