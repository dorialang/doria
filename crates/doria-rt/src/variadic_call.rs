//! The variadic call trampoline of the fast native profile (Decision 0148
//! section 14, amendment of 2026-11-17).
//!
//! Cranelift cannot set `%al` for a System V variadic call or place Apple
//! arm64 variadic arguments in their stack slots, so a Cranelift program
//! calls every variadic C function through `dr_v1_variadic_call`. The
//! compiler's shared C classification (`native_abi::c_abi`) decides where
//! each value goes and fills a [`DrVariadicCallV1`]: the argument register
//! images, the stack argument area, the indirect result register and the
//! System V vector register count. The trampoline loads them exactly,
//! calls the function, and stores the result registers back, so it holds
//! no convention rule of its own. LLVM calls variadic functions directly.

/// One variadic call. The compiler writes the inputs, the trampoline the
/// results. Every field is one 8-byte word; the offsets are part of the
/// runtime ABI (`doriac`'s `native_abi::VARIADIC_CALL_*`).
#[repr(C)]
pub struct DrVariadicCallV1 {
    /// The general-purpose argument registers in convention order.
    pub integer: [u64; 8],
    /// The floating-point argument registers, each a 64-bit image.
    pub float: [u64; 8],
    /// The AArch64 indirect result register `x8`.
    pub indirect_result: u64,
    /// System V's `%al`: the count of vector registers used.
    pub vector_registers: u64,
    /// The bytes of the stack argument area, a multiple of 16.
    pub stack_bytes: u64,
    /// The stack argument area's image, `stack_bytes` long.
    pub stack: *const u64,
    /// The general-purpose result registers.
    pub return_integer: [u64; 2],
    /// The floating-point result registers, each a 64-bit image.
    pub return_float: [u64; 4],
}

const INTEGER: usize = core::mem::offset_of!(DrVariadicCallV1, integer);
const FLOAT: usize = core::mem::offset_of!(DrVariadicCallV1, float);
const INDIRECT_RESULT: usize = core::mem::offset_of!(DrVariadicCallV1, indirect_result);
const VECTOR_REGISTERS: usize = core::mem::offset_of!(DrVariadicCallV1, vector_registers);
const STACK_BYTES: usize = core::mem::offset_of!(DrVariadicCallV1, stack_bytes);
const STACK: usize = core::mem::offset_of!(DrVariadicCallV1, stack);
const RETURN_INTEGER: usize = core::mem::offset_of!(DrVariadicCallV1, return_integer);
const RETURN_FLOAT: usize = core::mem::offset_of!(DrVariadicCallV1, return_float);

const _: () = {
    assert!(INTEGER == 0 && FLOAT == 64 && INDIRECT_RESULT == 128);
    assert!(VECTOR_REGISTERS == 136 && STACK_BYTES == 144 && STACK == 152);
    assert!(RETURN_INTEGER == 160 && RETURN_FLOAT == 176);
    assert!(core::mem::size_of::<DrVariadicCallV1>() == 208);
};

/// Calls the C function at `function` with the registers and stack area
/// `call` describes, then stores its result registers into `call`.
///
/// # Safety
///
/// `function` is a C function the described placement calls correctly, and
/// `call` points to a filled call whose `stack` holds `stack_bytes` bytes.
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn dr_v1_variadic_call(
    function: *const core::ffi::c_void,
    call: *mut DrVariadicCallV1,
) {
    core::arch::naked_asm!(
        "stp x29, x30, [sp, #-32]!",
        "mov x29, sp",
        "str x19, [sp, #16]",
        "mov x19, x1",
        "mov x16, x0",
        // Reserve and fill the stack argument area.
        "ldr x9, [x19, #{stack_bytes}]",
        "sub sp, sp, x9",
        "ldr x10, [x19, #{stack}]",
        "mov x11, #0",
        "2:",
        "cmp x11, x9",
        "b.hs 3f",
        "ldr x12, [x10, x11]",
        "str x12, [sp, x11]",
        "add x11, x11, #8",
        "b 2b",
        "3:",
        "ldp d0, d1, [x19, #{float}]",
        "ldp d2, d3, [x19, #{float_16}]",
        "ldp d4, d5, [x19, #{float_32}]",
        "ldp d6, d7, [x19, #{float_48}]",
        "ldr x8, [x19, #{indirect_result}]",
        "ldp x0, x1, [x19, #{integer}]",
        "ldp x2, x3, [x19, #{integer_16}]",
        "ldp x4, x5, [x19, #{integer_32}]",
        "ldp x6, x7, [x19, #{integer_48}]",
        "blr x16",
        "stp x0, x1, [x19, #{return_integer}]",
        "stp d0, d1, [x19, #{return_float}]",
        "stp d2, d3, [x19, #{return_float_16}]",
        "mov sp, x29",
        "ldr x19, [sp, #16]",
        "ldp x29, x30, [sp], #32",
        "ret",
        integer = const INTEGER,
        float = const FLOAT,
        indirect_result = const INDIRECT_RESULT,
        stack_bytes = const STACK_BYTES,
        stack = const STACK,
        return_integer = const RETURN_INTEGER,
        return_float = const RETURN_FLOAT,
        float_16 = const FLOAT + 16,
        float_32 = const FLOAT + 32,
        float_48 = const FLOAT + 48,
        integer_16 = const INTEGER + 16,
        integer_32 = const INTEGER + 32,
        integer_48 = const INTEGER + 48,
        return_float_16 = const RETURN_FLOAT + 16,
    )
}

/// Calls the C function at `function` with the registers and stack area
/// `call` describes, then stores its result registers into `call`.
///
/// # Safety
///
/// `function` is a C function the described placement calls correctly, and
/// `call` points to a filled call whose `stack` holds `stack_bytes` bytes.
#[cfg(all(target_arch = "x86_64", not(windows)))]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn dr_v1_variadic_call(
    function: *const core::ffi::c_void,
    call: *mut DrVariadicCallV1,
) {
    // System V: the function in `rdi`, the call in `rsi`. Three pushes
    // leave the stack 16-byte aligned, and the area is a multiple of 16.
    core::arch::naked_asm!(
        "push rbp",
        "mov rbp, rsp",
        "push rbx",
        "push r12",
        "mov rbx, rsi",
        "mov r12, rdi",
        "mov rcx, qword ptr [rbx + {stack_bytes}]",
        "sub rsp, rcx",
        "mov rsi, qword ptr [rbx + {stack}]",
        "mov rdi, rsp",
        "cld",
        "rep movsb",
        "movq xmm0, qword ptr [rbx + {float}]",
        "movq xmm1, qword ptr [rbx + {float} + 8]",
        "movq xmm2, qword ptr [rbx + {float} + 16]",
        "movq xmm3, qword ptr [rbx + {float} + 24]",
        "movq xmm4, qword ptr [rbx + {float} + 32]",
        "movq xmm5, qword ptr [rbx + {float} + 40]",
        "movq xmm6, qword ptr [rbx + {float} + 48]",
        "movq xmm7, qword ptr [rbx + {float} + 56]",
        "mov rdi, qword ptr [rbx + {integer}]",
        "mov rsi, qword ptr [rbx + {integer} + 8]",
        "mov rdx, qword ptr [rbx + {integer} + 16]",
        "mov rcx, qword ptr [rbx + {integer} + 24]",
        "mov r8, qword ptr [rbx + {integer} + 32]",
        "mov r9, qword ptr [rbx + {integer} + 40]",
        "mov rax, qword ptr [rbx + {vector_registers}]",
        "call r12",
        "mov qword ptr [rbx + {return_integer}], rax",
        "mov qword ptr [rbx + {return_integer} + 8], rdx",
        "movq qword ptr [rbx + {return_float}], xmm0",
        "movq qword ptr [rbx + {return_float} + 8], xmm1",
        "lea rsp, [rbp - 16]",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
        integer = const INTEGER,
        float = const FLOAT,
        vector_registers = const VECTOR_REGISTERS,
        stack_bytes = const STACK_BYTES,
        stack = const STACK,
        return_integer = const RETURN_INTEGER,
        return_float = const RETURN_FLOAT,
    )
}

/// Calls the C function at `function` with the registers and stack area
/// `call` describes, then stores its result registers into `call`.
///
/// # Safety
///
/// `function` is a C function the described placement calls correctly, and
/// `call` points to a filled call whose `stack` holds `stack_bytes` bytes,
/// the 32-byte shadow area first.
#[cfg(all(target_arch = "x86_64", windows))]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn dr_v1_variadic_call(
    function: *const core::ffi::c_void,
    call: *mut DrVariadicCallV1,
) {
    // Windows x64: the function in `rcx`, the call in `rdx`. `rbx`, `rsi`,
    // `rdi` and `r12` are callee-saved; five pushes leave the stack 16-byte
    // aligned, and the area is a multiple of 16.
    core::arch::naked_asm!(
        "push rbp",
        "mov rbp, rsp",
        "push rbx",
        "push rsi",
        "push rdi",
        "push r12",
        "mov rbx, rdx",
        "mov r12, rcx",
        "mov rcx, qword ptr [rbx + {stack_bytes}]",
        "sub rsp, rcx",
        "mov rsi, qword ptr [rbx + {stack}]",
        "mov rdi, rsp",
        "cld",
        "rep movsb",
        "movq xmm0, qword ptr [rbx + {float}]",
        "movq xmm1, qword ptr [rbx + {float} + 8]",
        "movq xmm2, qword ptr [rbx + {float} + 16]",
        "movq xmm3, qword ptr [rbx + {float} + 24]",
        "mov rcx, qword ptr [rbx + {integer}]",
        "mov rdx, qword ptr [rbx + {integer} + 8]",
        "mov r8, qword ptr [rbx + {integer} + 16]",
        "mov r9, qword ptr [rbx + {integer} + 24]",
        "call r12",
        "mov qword ptr [rbx + {return_integer}], rax",
        "movq qword ptr [rbx + {return_float}], xmm0",
        "lea rsp, [rbp - 32]",
        "pop r12",
        "pop rdi",
        "pop rsi",
        "pop rbx",
        "pop rbp",
        "ret",
        integer = const INTEGER,
        float = const FLOAT,
        stack_bytes = const STACK_BYTES,
        stack = const STACK,
        return_integer = const RETURN_INTEGER,
        return_float = const RETURN_FLOAT,
    )
}
