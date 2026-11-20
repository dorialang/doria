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
    /// argument area on the stack. `closed` is the register class the
    /// convention has closed for this and every later argument of the class
    /// (an AArch64 argument that did not fit); `None` when the convention
    /// leaves the registers to later arguments (System V's memory class)
    /// or places every argument on the stack (an Apple arm64 variadic one).
    Stack {
        size: u64,
        align: u64,
        closed: Option<PartClass>,
    },
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
    /// The count of fixed parameters of a variadic binding (section 14).
    pub fixed_parameters: Option<usize>,
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
        fixed_parameters,
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
                    closed: None,
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
            let slot = |ty: &CType, closed| ArgumentPassing::Stack {
                size: ty.size().div_ceil(8) * 8,
                align: 8,
                closed,
            };
            // Apple arm64 passes every variadic argument on the stack, one
            // 8-byte slot each; a composite over 16 bytes by reference.
            if apple_fixed_parameters.is_some_and(|fixed| index >= fixed) {
                return if !matches!(ty, CType::Scalar(_)) && ty.size() > 16 {
                    ArgumentPassing::IndirectCopy { on_stack: true }
                } else {
                    slot(ty, None)
                };
            }
            match ty {
                CType::Scalar(scalar) => {
                    let (file, class) = if scalar.is_float() {
                        (&mut floats, PartClass::Float)
                    } else {
                        (&mut integers, PartClass::Integer)
                    };
                    let backend_register = file.backend_has_register();
                    if file.take(1) || !backend_register {
                        ArgumentPassing::Scalar(*scalar)
                    } else {
                        ArgumentPassing::Stack {
                            size: scalar.size(),
                            align: scalar.size(),
                            closed: Some(class),
                        }
                    }
                }
                aggregate => {
                    if let Some(parts) = homogeneous_float_parts(aggregate) {
                        return if floats.take(parts.len()) {
                            ArgumentPassing::Registers(parts)
                        } else {
                            slot(aggregate, Some(PartClass::Float))
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
                        slot(aggregate, Some(PartClass::Integer))
                    }
                }
            }
        })
        .collect()
}

// The backend signature ------------------------------------------------------

/// How a backend types one register's worth of an aggregate's memory image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartType {
    /// A 64-bit integer register; a part narrower than 8 bytes travels in
    /// its low bytes.
    Integer64,
    Float32,
    Float64,
}

impl RegisterPart {
    pub const fn part_type(self) -> PartType {
        match (self.class, self.size) {
            (PartClass::Integer, _) => PartType::Integer64,
            (PartClass::Float, 4) => PartType::Float32,
            (PartClass::Float, _) => PartType::Float64,
        }
    }
}

/// One value of the signature both native backends declare for a foreign
/// call, in order. Each names the argument it comes from by index.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PhysicalParameter {
    /// The address of the caller's storage for an indirect result: the
    /// first argument on System V and Windows x64, `x8` on AArch64
    /// ([`indirect_return_takes_first_argument`]). It comes first.
    ResultAddress,
    /// A scalar argument, widened as [`extension`] says.
    Scalar { argument: usize, scalar: CScalar },
    /// The register's worth of an argument's memory image at `offset`.
    Part {
        argument: usize,
        offset: u64,
        ty: PartType,
    },
    /// A register of `class` the callee does not read. The convention has
    /// closed that class, so the parameters after it travel on the stack;
    /// padding the registers that remain makes a backend place them there.
    Padding(PartClass),
    /// An argument's memory image, `size` bytes, which the backend copies
    /// into the stack argument area (Cranelift's struct argument, LLVM's
    /// `byval`), leaving the registers to later arguments.
    StackImage { argument: usize, size: u64 },
    /// The address of a copy of the argument the caller makes.
    CopyAddress { argument: usize },
}

/// How a backend receives the result of a foreign call.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PhysicalReturn {
    Void,
    Scalar(CScalar),
    /// One returned register per part, each the part of the result's
    /// memory image at its offset.
    Parts(Vec<(u64, PartType)>),
    /// The callee writes the result to the storage
    /// [`PhysicalParameter::ResultAddress`] names.
    Indirect,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PhysicalSignature {
    pub parameters: Vec<PhysicalParameter>,
    pub return_value: PhysicalReturn,
    /// For a variadic call, the count of leading `parameters` that come
    /// from the fixed arguments (and the result address); the rest are the
    /// variadic arguments.
    pub fixed_parameters: Option<usize>,
}

/// The bytes a backend reserves for an argument's or result's memory image
/// when it moves the image by register parts or stack chunks: the C size
/// rounded up to whole 8-byte registers, zero beyond the value.
pub const fn padded_image_size(size: u64) -> u64 {
    size.div_ceil(8) * 8
}

/// The registers of each class an AArch64 backend fills in argument order.
const AARCH64_ARGUMENT_REGISTERS: usize = 8;

/// The one signature both native backends declare for a classified foreign
/// call (Decision 0148 section 6): which value travels in each backend
/// parameter and how the result returns. Both backends place a parameter in
/// the next register of its class while one is free, and otherwise on the
/// stack in order, so padding the remaining registers of a class the
/// convention has closed places an AArch64 stack argument where the
/// convention does; System V's memory class uses a stack image instead,
/// because later arguments still use the registers.
/// `parameters` are the C types `classification` classified.
pub fn physical_signature(
    target: CTarget,
    parameter_types: &[CType],
    classification: &SignatureClassification,
) -> PhysicalSignature {
    let aarch64 = matches!(
        target.calling_convention(),
        CCallingConvention::Aapcs64 | CCallingConvention::AppleArm64
    );
    let mut parameters = Vec::new();
    if classification.return_value == ReturnPassing::Indirect {
        parameters.push(PhysicalParameter::ResultAddress);
    }
    // Registers a backend has filled, by class, which only AArch64 padding
    // reads; the result address travels in `x8` there.
    let mut used = RegisterUse::default();
    let mut fixed_parameters = None;
    for (argument, passing) in classification.parameters.iter().enumerate() {
        // Apple arm64 places every variadic argument on the stack, so no
        // register needs padding before one.
        let apple_variadic = classification
            .fixed_parameters
            .is_some_and(|fixed| argument >= fixed)
            && target.calling_convention() == CCallingConvention::AppleArm64;
        if classification.fixed_parameters == Some(argument) {
            fixed_parameters = Some(parameters.len());
        }
        match passing {
            ArgumentPassing::Scalar(scalar) => used.push(
                &mut parameters,
                PhysicalParameter::Scalar {
                    argument,
                    scalar: *scalar,
                },
            ),
            ArgumentPassing::Registers(parts) => {
                for part in parts {
                    used.push(
                        &mut parameters,
                        PhysicalParameter::Part {
                            argument,
                            offset: part.offset,
                            ty: part.part_type(),
                        },
                    );
                }
            }
            ArgumentPassing::Stack { size, closed, .. } if aarch64 => {
                if let Some(class) = closed {
                    used.pad(&mut parameters, *class);
                }
                let ty = match closed {
                    Some(PartClass::Float) => PartType::Float64,
                    _ => PartType::Integer64,
                };
                // A scalar keeps its own type, so a backend gives it the
                // convention's slot size; an image moves in 8-byte chunks.
                if let Some(CType::Scalar(scalar)) = parameter_types.get(argument) {
                    let scalar = *scalar;
                    used.push(
                        &mut parameters,
                        PhysicalParameter::Scalar { argument, scalar },
                    );
                } else {
                    for chunk in 0..size / 8 {
                        used.push(
                            &mut parameters,
                            PhysicalParameter::Part {
                                argument,
                                offset: chunk * 8,
                                ty,
                            },
                        );
                    }
                }
            }
            ArgumentPassing::Stack { size, .. } => used.push(
                &mut parameters,
                PhysicalParameter::StackImage {
                    argument,
                    size: *size,
                },
            ),
            ArgumentPassing::IndirectCopy { on_stack } => {
                if *on_stack && !apple_variadic {
                    used.pad(&mut parameters, PartClass::Integer);
                }
                used.push(&mut parameters, PhysicalParameter::CopyAddress { argument });
            }
        }
    }
    let return_value = match &classification.return_value {
        ReturnPassing::Void => PhysicalReturn::Void,
        ReturnPassing::Scalar(scalar) => PhysicalReturn::Scalar(*scalar),
        ReturnPassing::Registers(parts) => PhysicalReturn::Parts(
            parts
                .iter()
                .map(|part| (part.offset, part.part_type()))
                .collect(),
        ),
        ReturnPassing::Indirect => PhysicalReturn::Indirect,
    };
    PhysicalSignature {
        fixed_parameters: classification
            .fixed_parameters
            .map(|_| fixed_parameters.unwrap_or(parameters.len())),
        parameters,
        return_value,
    }
}

/// The registers of each class a backend has filled with parameters.
#[derive(Default)]
struct RegisterUse {
    integers: usize,
    floats: usize,
}

impl RegisterUse {
    fn count(&mut self, class: PartClass) -> &mut usize {
        match class {
            PartClass::Integer => &mut self.integers,
            PartClass::Float => &mut self.floats,
        }
    }

    fn push(&mut self, parameters: &mut Vec<PhysicalParameter>, parameter: PhysicalParameter) {
        let class = match &parameter {
            PhysicalParameter::Scalar { scalar, .. } if scalar.is_float() => Some(PartClass::Float),
            PhysicalParameter::Scalar { .. } | PhysicalParameter::CopyAddress { .. } => {
                Some(PartClass::Integer)
            }
            PhysicalParameter::Part {
                ty: PartType::Integer64,
                ..
            } => Some(PartClass::Integer),
            PhysicalParameter::Part { .. } => Some(PartClass::Float),
            PhysicalParameter::Padding(class) => Some(*class),
            PhysicalParameter::ResultAddress | PhysicalParameter::StackImage { .. } => None,
        };
        if let Some(class) = class {
            let count = self.count(class);
            *count = (*count + 1).min(AARCH64_ARGUMENT_REGISTERS);
        }
        parameters.push(parameter);
    }

    /// Fills every remaining register of `class` with padding.
    fn pad(&mut self, parameters: &mut Vec<PhysicalParameter>, class: PartClass) {
        while *self.count(class) < AARCH64_ARGUMENT_REGISTERS {
            self.push(parameters, PhysicalParameter::Padding(class));
        }
    }
}

// The variadic call trampoline -----------------------------------------------

/// Where the runtime's variadic call trampoline (`dr_v1_variadic_call`,
/// Decision 0148 section 14, amendment of 2026-11-17) places one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VariadicSlot {
    /// The argument register of index `n` among the general-purpose ones:
    /// `rdi rsi rdx rcx r8 r9` on System V, `rcx rdx r8 r9` on Windows x64,
    /// `x0`-`x7` on AArch64.
    Integer(usize),
    /// The floating-point argument register of index `n`: `xmm0`-`xmm7` or
    /// `v0`-`v7`. A `float32` travels in its low four bytes.
    Float(usize),
    /// The AArch64 indirect result register `x8`.
    IndirectResult,
    /// `size` bytes at `offset` in the stack argument area.
    Stack { offset: u64, size: u64 },
}

/// Where every value of a variadic call goes. The compiler fills the
/// trampoline's register images and stack area from it; the trampoline
/// only loads them, so the convention's rules live here alone.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VariadicCallLayout {
    /// The slots of each [`PhysicalSignature`] parameter, in order: one,
    /// two for a Windows x64 variadic float (a float register and the
    /// integer register of the same position), none for padding.
    pub placements: Vec<Vec<VariadicSlot>>,
    /// The stack argument area, a multiple of 16 bytes, including Windows
    /// x64's 32-byte shadow area.
    pub stack_bytes: u64,
    /// The count of vector registers used, which System V passes in `%al`
    /// to a variadic callee; 0 elsewhere.
    pub vector_registers: u64,
}

/// The trampoline layout of a classified variadic call. Fixed arguments
/// travel as in any call; the variadic ones follow the convention's rules
/// for them: System V counts the vector registers in `%al`, Windows x64
/// duplicates a float in the integer register of its position, Apple arm64
/// places each in its own 8-byte stack slot, and AAPCS64 treats them as
/// fixed.
pub fn variadic_call_layout(target: CTarget, physical: &PhysicalSignature) -> VariadicCallLayout {
    let convention = target.calling_convention();
    let fixed = physical
        .fixed_parameters
        .unwrap_or(physical.parameters.len());
    let mut integers = 0;
    let mut floats = 0;
    let mut stack = 0;
    let mut placements = Vec::with_capacity(physical.parameters.len());
    // The next slot of `size` bytes aligned to `align` in the stack area.
    let stack_slot = |stack: &mut u64, size: u64, align: u64| {
        let offset = stack.div_ceil(align) * align;
        *stack = offset + size;
        VariadicSlot::Stack { offset, size }
    };
    for (index, parameter) in physical.parameters.iter().enumerate() {
        let variadic = index >= fixed;
        // Whether the value travels in an integer register, a float
        // register, or neither.
        let class = match parameter {
            PhysicalParameter::Scalar { scalar, .. } if scalar.is_float() => Some(PartClass::Float),
            PhysicalParameter::Part {
                ty: PartType::Float32 | PartType::Float64,
                ..
            } => Some(PartClass::Float),
            PhysicalParameter::Padding(class) => Some(*class),
            PhysicalParameter::StackImage { .. } => None,
            _ => Some(PartClass::Integer),
        };
        // The natural size of a value that reaches the stack.
        let natural = match parameter {
            PhysicalParameter::Scalar { scalar, .. } => scalar.size(),
            _ => 8,
        };
        let slots = match convention {
            CCallingConvention::SystemV => match (parameter, class) {
                (PhysicalParameter::StackImage { size, .. }, _) => {
                    vec![stack_slot(&mut stack, *size, 8)]
                }
                (_, Some(PartClass::Float)) if floats < SYSTEM_V_FLOAT_REGISTERS => {
                    floats += 1;
                    vec![VariadicSlot::Float(floats - 1)]
                }
                (_, Some(PartClass::Integer)) if integers < SYSTEM_V_INTEGER_REGISTERS => {
                    integers += 1;
                    vec![VariadicSlot::Integer(integers - 1)]
                }
                _ => vec![stack_slot(&mut stack, 8, 8)],
            },
            CCallingConvention::Windows64 => {
                // Each value takes the next of four positions, whatever
                // its class; later values take 8-byte stack slots after
                // the shadow area.
                let position = integers;
                integers += 1;
                if position >= 4 {
                    vec![stack_slot(&mut stack, 8, 8)]
                } else if class == Some(PartClass::Float) {
                    let mut slots = vec![VariadicSlot::Float(position)];
                    if variadic {
                        slots.push(VariadicSlot::Integer(position));
                    }
                    slots
                } else {
                    vec![VariadicSlot::Integer(position)]
                }
            }
            CCallingConvention::Aapcs64 | CCallingConvention::AppleArm64 => {
                let apple = convention == CCallingConvention::AppleArm64;
                match (parameter, class) {
                    (PhysicalParameter::ResultAddress, _) => vec![VariadicSlot::IndirectResult],
                    _ if variadic && apple => vec![stack_slot(&mut stack, 8, 8)],
                    (PhysicalParameter::Padding(_), Some(PartClass::Float)) => {
                        floats = (floats + 1).min(AARCH64_REGISTERS);
                        Vec::new()
                    }
                    (PhysicalParameter::Padding(_), _) => {
                        integers = (integers + 1).min(AARCH64_REGISTERS);
                        Vec::new()
                    }
                    (_, Some(PartClass::Float)) if floats < AARCH64_REGISTERS => {
                        floats += 1;
                        vec![VariadicSlot::Float(floats - 1)]
                    }
                    (_, Some(PartClass::Integer)) if integers < AARCH64_REGISTERS => {
                        integers += 1;
                        vec![VariadicSlot::Integer(integers - 1)]
                    }
                    // Apple arm64 packs fixed stack arguments at their
                    // natural size; AAPCS64 gives each an 8-byte slot.
                    _ if apple => vec![stack_slot(&mut stack, natural, natural)],
                    _ => vec![stack_slot(&mut stack, natural.max(8), 8)],
                }
            }
        };
        placements.push(slots);
    }
    let vector_registers = match convention {
        CCallingConvention::SystemV => floats as u64,
        _ => 0,
    };
    let shadow = match convention {
        CCallingConvention::Windows64 => 32,
        _ => 0,
    };
    // Windows x64 stack slots follow the shadow area.
    if shadow > 0 {
        for slot in placements.iter_mut().flatten() {
            if let VariadicSlot::Stack { offset, .. } = slot {
                *offset += shadow;
            }
        }
    }
    VariadicCallLayout {
        placements,
        stack_bytes: (stack + shadow).div_ceil(16) * 16,
        vector_registers,
    }
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
            ArgumentPassing::Stack {
                size: 16,
                align: 8,
                closed: None
            }
        );
        assert_eq!(
            classify(LINUX_X86, &[large], None, None).parameters,
            [ArgumentPassing::Stack {
                size: 24,
                align: 8,
                closed: None
            }]
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
            ArgumentPassing::Stack {
                size: 16,
                align: 8,
                closed: Some(PartClass::Integer)
            }
        );
        assert_eq!(
            classification.parameters[8],
            ArgumentPassing::Stack {
                size: 4,
                align: 4,
                closed: Some(PartClass::Integer)
            }
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
                ArgumentPassing::Stack {
                    size: 8,
                    align: 8,
                    closed: None
                },
                ArgumentPassing::Stack {
                    size: 8,
                    align: 8,
                    closed: None
                },
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

    #[test]
    fn the_backend_signature_pads_closed_aarch64_registers_before_stack_arguments() {
        let pair = record(vec![INT64, INT64]);
        let mut parameters = vec![INT64; 7];
        parameters.push(pair);
        parameters.push(INT32);
        let classification = classify(LINUX_ARM, &parameters, None, None);
        let signature = physical_signature(LINUX_ARM, &parameters, &classification);
        // Seven scalars, one padding register, the pair's image in two
        // stack chunks, and the last scalar on the stack after them.
        assert_eq!(
            signature.parameters[7..],
            [
                PhysicalParameter::Padding(PartClass::Integer),
                PhysicalParameter::Part {
                    argument: 7,
                    offset: 0,
                    ty: PartType::Integer64
                },
                PhysicalParameter::Part {
                    argument: 7,
                    offset: 8,
                    ty: PartType::Integer64
                },
                PhysicalParameter::Scalar {
                    argument: 8,
                    scalar: CScalar::Integer {
                        bytes: 4,
                        signed: true
                    }
                },
            ]
        );
        // A homogeneous float aggregate that does not fit moves as float
        // chunks, leaving the integer registers to later arguments.
        let vector = record(vec![FLOAT64, FLOAT64]);
        let mut parameters = vec![FLOAT64; 7];
        parameters.push(vector);
        parameters.push(INT32);
        let classification = classify(MAC_ARM, &parameters, None, None);
        let signature = physical_signature(MAC_ARM, &parameters, &classification);
        assert_eq!(
            signature.parameters[7..10],
            [
                PhysicalParameter::Padding(PartClass::Float),
                PhysicalParameter::Part {
                    argument: 7,
                    offset: 0,
                    ty: PartType::Float64
                },
                PhysicalParameter::Part {
                    argument: 7,
                    offset: 8,
                    ty: PartType::Float64
                },
            ]
        );
    }

    #[test]
    fn the_backend_signature_places_results_and_large_arguments_by_convention() {
        let large = record(vec![INT64, INT64, INT64]);
        let mixed = record(vec![INT32, FLOAT32, FLOAT64]);
        let parameters = [large.clone(), mixed, INT32];
        for target in [LINUX_X86, LINUX_ARM, WINDOWS] {
            let classification = classify(target, &parameters, Some(&large), None);
            let signature = physical_signature(target, &parameters, &classification);
            assert_eq!(signature.parameters[0], PhysicalParameter::ResultAddress);
            assert_eq!(signature.return_value, PhysicalReturn::Indirect);
            let large_argument = &signature.parameters[1];
            match target.calling_convention() {
                CCallingConvention::SystemV => assert_eq!(
                    *large_argument,
                    PhysicalParameter::StackImage {
                        argument: 0,
                        size: 24
                    }
                ),
                _ => assert_eq!(
                    *large_argument,
                    PhysicalParameter::CopyAddress { argument: 0 }
                ),
            }
        }
        let classification = classify(LINUX_X86, &parameters[1..], Some(&parameters[1]), None);
        let signature = physical_signature(LINUX_X86, &parameters[1..], &classification);
        assert_eq!(
            signature.return_value,
            PhysicalReturn::Parts(vec![(0, PartType::Integer64), (8, PartType::Float64)])
        );
        assert_eq!(padded_image_size(12), 16);
    }

    fn variadic_layout(target: CTarget, parameters: &[CType], fixed: usize) -> VariadicCallLayout {
        let classification = classify(target, parameters, None, Some(fixed));
        let physical = physical_signature(target, parameters, &classification);
        assert_eq!(physical.fixed_parameters, Some(fixed));
        variadic_call_layout(target, &physical)
    }

    #[test]
    fn a_variadic_call_places_each_argument_by_its_convention() {
        use VariadicSlot::{Float, Integer, Stack};
        // printf(const char *format, ...) with an int and a double.
        let printf = [CType::Scalar(CScalar::Pointer), INT32, FLOAT64];
        let system_v = variadic_layout(LINUX_X86, &printf, 1);
        assert_eq!(
            system_v.placements,
            [vec![Integer(0)], vec![Integer(1)], vec![Float(0)]]
        );
        assert_eq!((system_v.stack_bytes, system_v.vector_registers), (0, 1));
        // Windows x64 duplicates a variadic double in the integer register
        // of its position and keeps the 32-byte shadow area.
        let windows = variadic_layout(WINDOWS, &printf, 1);
        assert_eq!(
            windows.placements,
            [
                vec![Integer(0)],
                vec![Integer(1)],
                vec![Float(2), Integer(2)]
            ]
        );
        assert_eq!((windows.stack_bytes, windows.vector_registers), (32, 0));
        // Apple arm64 gives each variadic argument its own 8-byte slot.
        let apple = variadic_layout(MAC_ARM, &printf, 1);
        assert_eq!(
            apple.placements,
            [
                vec![Integer(0)],
                vec![Stack { offset: 0, size: 8 }],
                vec![Stack { offset: 8, size: 8 }]
            ]
        );
        assert_eq!(apple.stack_bytes, 16);
        // AAPCS64 passes variadic arguments as fixed ones.
        let linux_arm = variadic_layout(LINUX_ARM, &printf, 1);
        assert_eq!(
            linux_arm.placements,
            [vec![Integer(0)], vec![Integer(1)], vec![Float(0)]]
        );
        assert_eq!(linux_arm.stack_bytes, 0);
    }

    #[test]
    fn variadic_arguments_past_the_registers_take_stack_slots() {
        use VariadicSlot::{Integer, Stack};
        let integers = [INT64; 8];
        let system_v = variadic_layout(LINUX_X86, &integers, 1);
        assert_eq!(system_v.placements[5], [Integer(5)]);
        assert_eq!(system_v.placements[6], [Stack { offset: 0, size: 8 }]);
        assert_eq!(system_v.placements[7], [Stack { offset: 8, size: 8 }]);
        assert_eq!(system_v.stack_bytes, 16);
        let windows = variadic_layout(WINDOWS, &integers[..5], 1);
        assert_eq!(windows.placements[3], [Integer(3)]);
        assert_eq!(
            windows.placements[4],
            [Stack {
                offset: 32,
                size: 8
            }]
        );
        assert_eq!(windows.stack_bytes, 48);
        // A struct after the fixed arguments on Apple arm64 fills whole
        // 8-byte slots, and one over 16 bytes travels as the address of a
        // copy in a slot, with no register padding before it.
        let pair = record(vec![INT64, INT32]);
        let wide = record(vec![INT64, INT64, INT64]);
        let apple = variadic_layout(MAC_ARM, &[INT32, pair, wide], 1);
        assert_eq!(
            apple.placements,
            [
                vec![Integer(0)],
                vec![Stack { offset: 0, size: 8 }],
                vec![Stack { offset: 8, size: 8 }],
                vec![Stack {
                    offset: 16,
                    size: 8
                }]
            ]
        );
        assert_eq!(apple.stack_bytes, 32);
    }
}
