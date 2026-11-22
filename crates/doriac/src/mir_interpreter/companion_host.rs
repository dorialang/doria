//! The interpreter as host of the native companion library (Decision 0148
//! sections 22 and 23, entry ABI version 1).
//!
//! A program with unsafe regions runs each region as one call into its
//! companion: the interpreter builds the library into the compiler cache,
//! loads the shared runtime and then the library with the platform loader,
//! and attaches its host record before the first region runs. A region
//! entry receives the interpreter's frames as a native frame chain, so the
//! Call Path matches the native one. Every host operation the companion
//! calls back runs inside the frame suspended at the region entry, through
//! the interpreter's own statement and terminator execution. A panic,
//! whether the companion reports it to the panic sink or Doria code raises
//! it during a host operation, renders as a native panic does and ends the
//! process with status 101: neither side returns or runs cleanup.

use std::ffi::c_void;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

use super::{
    block_in, function_in, local_in, EvaluationTask, Interpreter, InterpreterError,
    InterpreterOutput, LocalValue, StepOutcome,
};
use crate::diagnostics::{
    DiagnosticSource, RuntimeFact, RuntimeFactValue, RuntimeOutcomeDetails, RuntimeOutcomeFrame,
    RuntimeOutcomeOrigin, TerminationBehavior,
};
use crate::mir;
use crate::source::Span;

/// One shared runtime holds one attached host and one Doria thread, so the
/// interpretations of one process that run a companion take turns.
static COMPANION_TURN: Mutex<()> = Mutex::new(());

/// `DrStackFrameV2`, the native frame a region entry links to.
#[repr(C)]
struct StackFrame {
    parent: *const StackFrame,
    function_name: *const u8,
    function_name_length: usize,
    source_path: *const u8,
    source_path_length: usize,
    source_text: *const u8,
    source_text_length: usize,
    function_span_start: usize,
    function_span_end: usize,
    active_span_start: usize,
    active_span_end: usize,
}

/// `DrRuntimeFactV2`, one fact of a panic the companion reports.
#[repr(C)]
struct RuntimeFactRecord {
    name: *const u8,
    name_length: usize,
    kind: usize,
    value: u64,
    value_length: usize,
}

type HostCall = unsafe extern "C" fn(*mut HostRecord, u32, *mut u8, *mut u8) -> i32;
type PanicSink = unsafe extern "C" fn(
    *mut HostRecord,
    *const StackFrame,
    *const u8,
    usize,
    *const u8,
    usize,
    *const RuntimeFactRecord,
    usize,
);
type RegionEntry =
    unsafe extern "C" fn(*mut HostRecord, *const StackFrame, *mut u8, *mut u8) -> i32;

/// `DrRegionHostV1` at offset 0, which is all the companion reads, then the
/// interpreter the record serves.
#[repr(C)]
struct HostRecord {
    call: HostCall,
    panic: PanicSink,
    interpreter: *mut c_void,
}

/// A companion the interpreter has loaded and attached its host to.
pub(super) struct LoadedCompanion {
    host: Box<HostRecord>,
    /// The entry of each region of the companion plan, in plan order.
    entries: Vec<RegionEntry>,
    /// The entry of each binding of the companion plan, in plan order.
    bindings: Vec<RegionEntry>,
    /// The address of each callback's C entry, in plan order: the value of
    /// its C-callable function's name.
    callbacks: Vec<u64>,
    /// The shared runtime's `dr_v1_last_system_code`.
    last_system_code: unsafe extern "C" fn() -> i64,
    /// The shared runtime's record of the outbound foreign call in
    /// progress, `dr_v1_foreign_call_frame`.
    foreign_call_record: *const *const StackFrame,
    _turn: MutexGuard<'static, ()>,
}

/// A region call in progress: the companion region, the depth of the host
/// frame suspended at its entry, its `outputs` record, the host values
/// operation 0 accepted, and the host content lent to the companion.
pub(super) struct ActiveRegionCall {
    region: usize,
    depth: usize,
    outputs: *mut u8,
    accepted: Vec<Option<LocalValue>>,
    lent: Lent,
}

/// Host storage lent to the companion for the length of a region call:
/// strings, native copies of buffers, and the native copies of regions
/// over host storage, which the companion views in place.
#[derive(Default)]
pub(super) struct Lent {
    text: Vec<Rc<str>>,
    buffers: Vec<Vec<u128>>,
    regions: Vec<LentRegion>,
}

/// The native copy of a region over host storage: where it starts, its
/// elements and their stride, and the region it copies.
struct LentRegion {
    data: usize,
    length: usize,
    stride: usize,
    element: mir::Type,
    region: super::byte_region::ByteRegionValue,
    buffer: usize,
}

impl Lent {
    fn extend(&mut self, other: Lent) {
        let base = self.buffers.len();
        self.text.extend(other.text);
        self.buffers.extend(other.buffers);
        self.regions
            .extend(other.regions.into_iter().map(|region| LentRegion {
                buffer: region.buffer + base,
                ..region
            }));
    }
}

/// The size of one element of type `element` in its C layout.
fn element_stride(element: mir::Type) -> usize {
    crate::native_closure_abi::type_layout(element, 8).size as usize
}

/// The elements `values` of type `element` in their C layout, in storage
/// with at least one word, so its address is never null.
fn native_buffer(values: &[LocalValue], element: mir::Type) -> Vec<u128> {
    let stride = element_stride(element);
    let mut buffer = vec![0_u128; (values.len() * stride).div_ceil(16).max(1)];
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), values.len() * stride)
    };
    for (index, value) in values.iter().enumerate() {
        encode_native(
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
unsafe fn native_elements(
    data: *const u8,
    length: usize,
    element: mir::Type,
) -> Result<Vec<LocalValue>, InterpreterError> {
    let stride = element_stride(element);
    (0..length)
        .map(|index| {
            decode_native(
                element,
                std::slice::from_raw_parts(data.add(index * stride), stride),
            )
        })
        .collect()
}

/// What finishes a pointer operation the host runs once its operand is
/// evaluated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PointerTask {
    FromAddress {
        target: Option<mir::LocalId>,
    },
    Offset {
        target: Option<mir::LocalId>,
        pointer: u64,
        scale: u64,
    },
    CreateHandle {
        target: Option<mir::LocalId>,
    },
}

fn pointer_value(bits: u64) -> mir::ScalarValue {
    mir::ScalarValue::Inline(crate::inline_value::InlineValue {
        ty: crate::inline_value::InlineValueType::Pointer,
        bits: u128::from(bits),
    })
}

fn pointer_bits(value: &LocalValue) -> Result<u64, InterpreterError> {
    match value {
        LocalValue::Scalar(mir::ScalarValue::Inline(inline))
        | LocalValue::NullableScalar {
            value: Some(mir::ScalarValue::Inline(inline)),
            ..
        } => Ok(inline.bits as u64),
        LocalValue::NullableScalar { value: None, .. } => Ok(0),
        _ => Err(InterpreterError::new("a pointer operation reads a pointer")),
    }
}

/// A slot record the host allocates zero-filled, aligned for every slot.
struct Record(Vec<u128>);

impl Record {
    fn zeroed(size: u64) -> Self {
        Self(vec![0; (size as usize).div_ceil(16).max(1)])
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.0.as_mut_ptr().cast()
    }
}

impl LoadedCompanion {
    /// Builds the companion of `program` into the compiler cache, loads the
    /// shared runtime and then the library, and attaches the host.
    pub(super) fn load(
        program: &mir::Program,
        linking: &crate::native_link::NativeLinking,
    ) -> Result<Self, InterpreterError> {
        let turn = COMPANION_TURN
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let published = crate::native_companion::cached(
            program,
            crate::backend::NativeProfile::Fast,
            linking,
            "program",
        )
        .map_err(InterpreterError::backend)?;
        let directory = published
            .library
            .parent()
            .ok_or_else(|| InterpreterError::new("the companion library has a directory"))?;
        let loading = |error: String| {
            InterpreterError::new(format!(
                "the native companion library could not be loaded: {error}"
            ))
        };
        let (library, runtime) =
            platform::open_companion(&published.library, &directory.join(&published.runtime))
                .map_err(loading)?;
        let mut entries = Vec::with_capacity(program.companion.regions.len());
        for region in &program.companion.regions {
            let address = platform::symbol(library, &region.entry_symbol()).map_err(loading)?;
            // SAFETY: the companion exports each region entry with the
            // version-1 entry signature.
            entries.push(unsafe { std::mem::transmute::<*mut c_void, RegionEntry>(address) });
        }
        let mut bindings = Vec::with_capacity(program.companion.bindings.len());
        for binding in &program.companion.bindings {
            let address = platform::symbol(library, &binding.entry_symbol()).map_err(loading)?;
            // SAFETY: a binding entry has the version-1 entry signature.
            bindings.push(unsafe { std::mem::transmute::<*mut c_void, RegionEntry>(address) });
        }
        let callbacks = program
            .companion
            .callbacks
            .iter()
            .map(|callback| {
                platform::symbol(library, &callback.entry_symbol()).map(|address| address as u64)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(loading)?;
        let attach = platform::symbol(runtime, "dr_v1_attach_region_host").map_err(loading)?;
        let last_system_code =
            platform::symbol(runtime, "dr_v1_last_system_code").map_err(loading)?;
        let foreign_call_record =
            platform::symbol(runtime, crate::native_abi::FOREIGN_CALL_FRAME).map_err(loading)?;
        let mut host = Box::new(HostRecord {
            call: host_call,
            panic: host_panic,
            interpreter: std::ptr::null_mut(),
        });
        // SAFETY: the shared runtime exports the attachment with this
        // signature; the record outlives every region call of this
        // interpretation, which holds the process's companion turn.
        unsafe {
            let attach =
                std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut HostRecord)>(attach);
            attach(&mut *host);
        }
        Ok(Self {
            host,
            entries,
            bindings,
            callbacks,
            // SAFETY: the shared runtime exports it with this signature.
            last_system_code: unsafe {
                std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> i64>(last_system_code)
            },
            foreign_call_record: foreign_call_record.cast_const().cast(),
            _turn: turn,
        })
    }

    /// The frame of the outbound foreign call in progress, if any.
    fn foreign_call_frame(&self) -> *const StackFrame {
        // SAFETY: the record is the runtime's atomic pointer, which only
        // the thread running Doria code writes, and this is that thread.
        unsafe { self.foreign_call_record.read_volatile() }
    }

    /// The code the program's last capturing foreign call stored (Decision
    /// 0148 section 16), which the shared runtime holds.
    pub(super) fn last_system_code(&self) -> i64 {
        // SAFETY: the runtime's accessor takes no arguments.
        unsafe { (self.last_system_code)() }
    }
}

impl InterpreterError {
    fn backend(error: crate::backend::BackendError) -> Self {
        let diagnostics = error.diagnostics.unwrap_or_default();
        let mut interpreter_error = Self::new(format!(
            "the native companion library could not be built: {}",
            error.message
        ));
        interpreter_error.diagnostics = diagnostics;
        interpreter_error
    }
}

/// The C-layout value of a native slot of type `ty`.
fn encode_native(value: Option<&LocalValue>, ty: mir::Type, slot: &mut [u8]) {
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
            let form = crate::native_closure_abi::nullable_scalar_form(stored, 8);
            if form != crate::native_closure_abi::NullableScalarForm::ZeroIsNull {
                slot[..8].copy_from_slice(&1_u64.to_ne_bytes());
            }
            scalar(*value, &mut slot[form.payload_offset() as usize..]);
        }
        // An absent value, or a local that holds none yet, is zero.
        _ => {}
    }
}

/// The value a native slot of type `ty` holds.
fn decode_native(ty: mir::Type, slot: &[u8]) -> Result<LocalValue, InterpreterError> {
    fn scalar(ty: mir::ScalarType, slot: &[u8]) -> Result<mir::ScalarValue, InterpreterError> {
        let word = |size: usize| {
            let mut bytes = [0_u8; 16];
            bytes[..size].copy_from_slice(&slot[..size]);
            u128::from_ne_bytes(bytes)
        };
        Ok(match ty {
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
        })
    }
    match ty {
        mir::Type::Scalar(stored) => Ok(LocalValue::Scalar(scalar(stored, slot)?)),
        mir::Type::NullableScalar(stored) => {
            let form = crate::native_closure_abi::nullable_scalar_form(stored, 8);
            let payload = &slot[form.payload_offset() as usize..];
            let present = match form {
                crate::native_closure_abi::NullableScalarForm::ZeroIsNull => {
                    payload[..8].iter().any(|byte| *byte != 0)
                }
                crate::native_closure_abi::NullableScalarForm::Tagged { .. } => {
                    slot[..8].iter().any(|byte| *byte != 0)
                }
            };
            Ok(LocalValue::NullableScalar {
                ty: stored,
                value: present.then(|| scalar(stored, payload)).transpose()?,
            })
        }
        _ => Err(InterpreterError::new(format!(
            "a native companion slot of type {ty:?} has no host form yet"
        ))),
    }
}

/// The text of a content slot, or `None` when it holds no value.
///
/// # Safety
///
/// `slot` is a content slot whose data, when present, holds `length`
/// readable bytes.
unsafe fn slot_text(slot: *const u8) -> Result<Option<Rc<str>>, InterpreterError> {
    let data = slot.cast::<*const u8>().read_unaligned();
    if data.is_null() {
        return Ok(None);
    }
    let length = slot.add(8).cast::<usize>().read_unaligned();
    let bytes = std::slice::from_raw_parts(data, length);
    std::str::from_utf8(bytes)
        .map(|text| Some(Rc::from(text)))
        .map_err(|_| InterpreterError::new("the companion returned text that is not UTF-8"))
}

/// Writes `text` into a content slot as lent storage, or no value.
///
/// # Safety
///
/// `slot` points to a writable content slot.
unsafe fn write_slot_text(slot: *mut u8, text: Option<&Rc<str>>) {
    let (data, length) = text.map_or((std::ptr::null(), 0), |text| (text.as_ptr(), text.len()));
    slot.cast::<*const u8>().write_unaligned(data);
    slot.add(8).cast::<usize>().write_unaligned(length);
}

impl<'program> Interpreter<'program> {
    /// Assigns `value` to `target` as the statement's result, after the
    /// tasks the statement already queued.
    fn queue_result(
        &mut self,
        target: Option<mir::LocalId>,
        value: LocalValue,
    ) -> Result<(), InterpreterError> {
        let Some(target) = target else {
            return Ok(());
        };
        self.push_local_value(value)?;
        self.current_frame_mut()?
            .tasks
            .push(EvaluationTask::Assign(target));
        Ok(())
    }

    /// A pointer operation the host runs itself: address arithmetic, the
    /// value of a C-callable function's name, and the handle table. Every
    /// operation that reads or writes memory is inside an unsafe region,
    /// which runs in the companion.
    pub(super) fn execute_host_pointer(
        &mut self,
        target: Option<mir::LocalId>,
        operation: mir::PointerOperation,
    ) -> Result<StepOutcome, InterpreterError> {
        let integer = |expression: mir::IntegerExpression| {
            EvaluationTask::Value(mir::ValueExpression::Integer(expression))
        };
        match operation {
            mir::PointerOperation::Address { pointer } => {
                let bits = pointer_bits(&self.read_local_value(pointer)?)?;
                self.queue_result(
                    target,
                    LocalValue::Scalar(mir::ScalarValue::Integer(
                        crate::numeric::IntegerValue::from_bits(
                            crate::numeric::IntegerType::UInt64,
                            bits,
                        ),
                    )),
                )?;
            }
            mir::PointerOperation::FromAddress { address } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::HostPointer(PointerTask::FromAddress {
                        target,
                    }));
                frame.tasks.push(integer(address));
            }
            mir::PointerOperation::Offset {
                pointer,
                count,
                scale,
            } => {
                let pointer = pointer_bits(&self.read_local_value(pointer)?)?;
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::HostPointer(PointerTask::Offset {
                        target,
                        pointer,
                        scale,
                    }));
                frame.tasks.push(integer(count));
            }
            mir::PointerOperation::CEntryAddress { function } => {
                let position = self
                    .program
                    .companion
                    .callbacks
                    .iter()
                    .position(|callback| callback.function == function)
                    .ok_or_else(|| {
                        InterpreterError::new("a C-callable function has its callback")
                    })?;
                let address = self
                    .companion
                    .as_ref()
                    .ok_or_else(|| {
                        InterpreterError::new("a program with callbacks has its companion")
                    })?
                    .callbacks[position];
                self.queue_result(target, LocalValue::Scalar(pointer_value(address)))?;
            }
            mir::PointerOperation::CreateHandle { value } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::HostPointer(PointerTask::CreateHandle {
                        target,
                    }));
                frame.tasks.push(EvaluationTask::Rvalue(value));
            }
            mir::PointerOperation::FromHandle {
                handle, reclaim, ..
            } => {
                let token = pointer_bits(&self.read_local_value(handle)?)? as usize;
                let entry = token
                    .checked_sub(1)
                    .and_then(|index| self.handles.get_mut(index))
                    .ok_or_else(|| InterpreterError::new("a handle names a host handle"))?;
                let value = if reclaim { entry.take() } else { entry.clone() }
                    .ok_or_else(|| InterpreterError::new("a handle holds its value"))?;
                self.queue_result(target, value)?;
            }
            _ => {
                return Err(InterpreterError::new(
                    "a pointer operation that reaches memory runs in the native companion library",
                ))
            }
        }
        Ok(StepOutcome::Continue)
    }

    pub(super) fn finish_host_pointer(
        &mut self,
        task: PointerTask,
    ) -> Result<StepOutcome, InterpreterError> {
        match task {
            PointerTask::FromAddress { target } => {
                let address = self.pop_integer()?.bits;
                let value = LocalValue::NullableScalar {
                    ty: mir::ScalarType::Inline(crate::inline_value::InlineValueType::Pointer),
                    value: (address != 0).then(|| pointer_value(address)),
                };
                self.queue_result(target, value)?;
            }
            PointerTask::Offset {
                target,
                pointer,
                scale,
            } => {
                let count = self.pop_integer()?.bits;
                let bits = pointer.wrapping_add(count.wrapping_mul(scale));
                self.queue_result(target, LocalValue::Scalar(pointer_value(bits)))?;
            }
            PointerTask::CreateHandle { target } => {
                let value = self.pop_local_value()?;
                self.handles.push(Some(value));
                let token = self.handles.len() as u64;
                self.queue_result(target, LocalValue::Scalar(pointer_value(token)))?;
            }
        }
        Ok(StepOutcome::Continue)
    }

    /// Whether the current frame is about to enter one of its function's
    /// regions, which then runs in the companion. A frame suspended at a
    /// region entry runs that region's host operations itself.
    pub(super) fn region_at_current_block(
        &self,
    ) -> Result<Option<(&'program mir::Function, usize)>, InterpreterError> {
        let frame = self.current_frame()?;
        if frame.entered_block
            || self
                .region_calls
                .last()
                .is_some_and(|call| call.depth == self.frames.len())
        {
            return Ok(None);
        }
        let function = function_in(self.program, frame.function)?;
        Ok(function
            .unsafe_regions
            .iter()
            .position(|region| region.entry == frame.block)
            .map(|index| (function, index)))
    }

    /// The extern declaration's function the current frame has just
    /// entered, which runs through its binding entry in the companion.
    pub(super) fn binding_at_current_frame(
        &self,
    ) -> Result<Option<&'program mir::Function>, InterpreterError> {
        let frame = self.current_frame()?;
        if frame.entered_block || frame.statement_index != 0 || self.companion.is_none() {
            return Ok(None);
        }
        let function = function_in(self.program, frame.function)?;
        Ok(
            (frame.block == function.entry_block && function.is_foreign_binding())
                .then_some(function),
        )
    }

    /// Calls the extern declaration's function of the current frame, with
    /// the arguments its parameters hold, through its binding entry, and
    /// returns its result from the frame (Decision 0148 section 23).
    pub(super) fn run_binding(
        &mut self,
        function: &'program mir::Function,
    ) -> Result<StepOutcome, InterpreterError> {
        let program = self.program;
        let position = program
            .companion
            .bindings
            .iter()
            .position(|binding| binding.function == function.id)
            .ok_or_else(|| {
                InterpreterError::new(format!(
                    "`{}` is called from a host operation and has its binding entry",
                    function.name
                ))
            })?;
        let binding = &program.companion.bindings[position];
        let (host, entry) = {
            let companion = self.companion.as_mut().ok_or_else(|| {
                InterpreterError::new("a program with unsafe regions has its companion")
            })?;
            (
                &mut *companion.host as *mut HostRecord,
                companion.bindings[position],
            )
        };
        // The entry's parameters follow its host, frame and two records.
        let mut arguments = Record::zeroed(binding.arguments.size);
        for (slot, parameter) in binding.arguments.slots.iter().zip(&function.params) {
            let ty = binding.entry.locals[slot.value.local.0].ty;
            let value = self.read_local_value(*parameter)?;
            let size = crate::native_abi::companion_slot_layout(slot.value, ty)
                .map_or(0, |(size, _)| size as usize);
            let target = unsafe {
                std::slice::from_raw_parts_mut(
                    arguments.as_mut_ptr().add(slot.offset as usize),
                    size,
                )
            };
            if !matches!(ty, mir::Type::Scalar(_) | mir::Type::NullableScalar(_)) {
                return Err(InterpreterError::new(format!(
                    "a native companion slot of type {ty:?} has no host form yet"
                )));
            }
            encode_native(Some(&value), ty, target);
        }
        let mut results = Record::zeroed(binding.results.size);
        // The binding's own frame is not yet native; the entry runs at its
        // caller's.
        let (frames, _names) = self.native_frames(self.frames.len() - 1);
        let caller = frames
            .last()
            .map_or(std::ptr::null(), |frame| frame as *const _);
        // SAFETY: as for a region call; the records live until it returns.
        unsafe {
            (*host).interpreter = (self as *mut Self).cast();
            entry(host, caller, arguments.as_mut_ptr(), results.as_mut_ptr());
        }
        drop(frames);
        let frame = self.current_frame_mut()?;
        match (function.return_type, binding.results.slots.first()) {
            (mir::ReturnType::Value(ty), Some(slot)) => {
                let size = crate::native_abi::companion_slot_layout(slot.value, ty)
                    .map_or(0, |(size, _)| size as usize);
                let value = decode_native(ty, unsafe {
                    std::slice::from_raw_parts(results.as_mut_ptr().add(slot.offset as usize), size)
                })?;
                frame.entered_block = true;
                frame
                    .tasks
                    .push(EvaluationTask::ReturnValue { ty, place: None });
                frame.tasks.push(EvaluationTask::CleanupFrame);
                self.push_local_value(value)?;
            }
            (mir::ReturnType::Void, None) => {
                frame.entered_block = true;
                frame.tasks.push(EvaluationTask::ReturnVoid);
                frame.tasks.push(EvaluationTask::CleanupFrame);
            }
            _ => {
                return Err(InterpreterError::new(
                    "a binding entry's result matches its function's return",
                ))
            }
        }
        Ok(StepOutcome::Continue)
    }

    /// Runs region `index` of `function`, the current frame's function,
    /// through its companion entry, then resumes where the region left.
    pub(super) fn run_region(
        &mut self,
        function: &'program mir::Function,
        index: usize,
    ) -> Result<StepOutcome, InterpreterError> {
        let program = self.program;
        let region = &function.unsafe_regions[index];
        let position = program
            .companion
            .regions
            .iter()
            .position(|plan| plan.id == region.id)
            .ok_or_else(|| InterpreterError::new("an unsafe region has its companion entry"))?;
        let plan = &program.companion.regions[position];
        let canonical = function_in(program, plan.lowerings[0].function)?;
        let canonical_region = &canonical.unsafe_regions[plan.lowerings[0].region];
        if canonical_region.inputs != region.inputs
            || canonical_region.outputs != region.outputs
            || canonical_region.blocks != region.blocks
            || canonical_region.exits != region.exits
        {
            return Err(InterpreterError::new(format!(
                "the lowerings of region {} number their locals or blocks differently",
                region.id
            )));
        }
        let (host, entry) = {
            let companion = self.companion.as_mut().ok_or_else(|| {
                InterpreterError::new("a program with unsafe regions has its companion")
            })?;
            (
                &mut *companion.host as *mut HostRecord,
                companion.entries[position],
            )
        };

        let mut lent = Lent::default();
        let mut inputs = Record::zeroed(plan.inputs.size);
        for slot in &plan.inputs.slots {
            let target = unsafe { inputs.as_mut_ptr().add(slot.offset as usize) };
            self.write_slot(function, slot.value, target, &mut lent)?;
        }
        let mut outputs = Record::zeroed(plan.outputs.size);
        let (frames, _names) = self.native_frames(self.frames.len());
        let caller = frames
            .last()
            .map_or(std::ptr::null(), |frame| frame as *const _);
        self.region_calls.push(ActiveRegionCall {
            region: position,
            depth: self.frames.len(),
            outputs: outputs.as_mut_ptr(),
            accepted: Vec::new(),
            lent,
        });
        // SAFETY: the record serves this interpreter for the call; nothing
        // else touches the interpreter until the entry returns, and every
        // record the entry reads lives until then.
        let status = unsafe {
            (*host).interpreter = (self as *mut Self).cast();
            entry(host, caller, inputs.as_mut_ptr(), outputs.as_mut_ptr())
        };
        let mut call = self
            .region_calls
            .pop()
            .ok_or_else(|| InterpreterError::new("a region call is in progress"))?;
        drop(frames);
        // The companion viewed each writable region over host storage in
        // place: its changes go back to the host's storage.
        for region in &call.lent.regions {
            if region.region.kind.access == crate::types::ByteRegionKind::Writable {
                self.copy_back(region)?;
            }
        }

        // Writable places cross back before the host resumes, on every exit.
        let mut writes = Vec::new();
        for slot in &plan.outputs.slots {
            let source = unsafe { outputs.as_mut_ptr().add(slot.offset as usize) };
            let ty = local_in(function, slot.value.local)?.ty;
            let value = match slot.value.kind {
                mir::UnsafeRegionValueKind::Native | mir::UnsafeRegionValueKind::Region => {
                    let _ = ty;
                    self.read_slot(function, slot.value, source, &call.lent.regions)?
                }
                _ => {
                    let token = unsafe { source.cast::<usize>().read_unaligned() };
                    match token {
                        0 => None,
                        token => call
                            .accepted
                            .get_mut(token - 1)
                            .and_then(Option::take)
                            .map(Some)
                            .ok_or_else(|| {
                                InterpreterError::new("a region output names a host value")
                            })?,
                    }
                }
            };
            writes.push((slot.value.local, value));
        }
        self.assign_crossed(writes)?;

        let exit = (status != 0)
            .then(|| {
                let index = plan
                    .outputs
                    .index
                    .ok_or_else(|| InterpreterError::new("a region's outputs end with its exit"))?;
                let taken = unsafe {
                    outputs
                        .as_mut_ptr()
                        .add(index as usize)
                        .cast::<i32>()
                        .read()
                };
                region
                    .exits
                    .get(taken as usize)
                    .filter(|exit| exit.kind.status() == status)
                    .ok_or_else(|| {
                        InterpreterError::new(format!(
                            "region {} left through exit {taken} with status {status}",
                            region.id
                        ))
                    })
            })
            .transpose()?;
        match exit {
            None if status == 0 => self.move_to_block(function, region.continuation)?,
            None => {
                return Err(InterpreterError::new(format!(
                    "region {} returned status {status}",
                    region.id
                )))
            }
            Some(exit) => match exit.target {
                Some(target) => self.move_to_block(function, target)?,
                // The exit's own terminator leaves the function, in the host
                // frame after the writeback.
                None => {
                    let source = block_in(function, exit.source)?;
                    let frame = self.current_frame_mut()?;
                    frame.block = exit.source;
                    frame.statement_index = source.statements.len();
                    frame.entered_block = true;
                }
            },
        }
        Ok(StepOutcome::Continue)
    }

    /// Assigns each crossed value to its local as an ordinary assignment
    /// does, in order. A value that crosses with none leaves its local
    /// without one, its content having moved.
    fn assign_crossed(
        &mut self,
        writes: Vec<(mir::LocalId, Option<LocalValue>)>,
    ) -> Result<(), InterpreterError> {
        let depth = self.frames.len();
        let mut assignments = Vec::new();
        for (local, value) in writes {
            match value {
                Some(value) => assignments.push((local, value)),
                None => {
                    if let Some(slot) = self.current_frame_mut()?.locals.get_mut(local.0) {
                        *slot = None;
                    }
                }
            }
        }
        for (_, value) in &assignments {
            self.push_local_value(value.clone())?;
        }
        for (local, _) in &assignments {
            self.current_frame_mut()?
                .tasks
                .push(EvaluationTask::Assign(*local));
        }
        self.run_host_site(depth, |_| true)
    }

    /// Writes the current value of `value`'s local into the slot at
    /// `target`, lending host content for the length of the call.
    fn write_slot(
        &self,
        function: &mir::Function,
        value: mir::UnsafeRegionValue,
        target: *mut u8,
        lent: &mut Lent,
    ) -> Result<(), InterpreterError> {
        let ty = local_in(function, value.local)?.ty;
        let held = self.current_frame()?.locals[value.local.0].is_some();
        let current = held
            .then(|| self.read_local_value(value.local))
            .transpose()?;
        let write_content = |data: *const u8, length: usize| unsafe {
            target.cast::<*const u8>().write_unaligned(data);
            target.add(8).cast::<usize>().write_unaligned(length);
        };
        match value.kind {
            mir::UnsafeRegionValueKind::Native => {
                let size = crate::native_abi::companion_slot_layout(value, ty)
                    .map_or(0, |(size, _)| size as usize);
                if !matches!(ty, mir::Type::Scalar(_) | mir::Type::NullableScalar(_)) {
                    return Err(InterpreterError::new(format!(
                        "a native companion slot of type {ty:?} has no host form yet"
                    )));
                }
                let slot = unsafe { std::slice::from_raw_parts_mut(target, size) };
                encode_native(current.as_ref(), ty, slot);
            }
            mir::UnsafeRegionValueKind::Text => {
                let text = match current {
                    Some(LocalValue::String(text))
                    | Some(LocalValue::NullableString(Some(text))) => Some(text),
                    _ => None,
                };
                unsafe { write_slot_text(target, text.as_ref()) };
                lent.text.extend(text);
            }
            mir::UnsafeRegionValueKind::Buffer | mir::UnsafeRegionValueKind::ForeignString => {
                let Some(LocalValue::Collection(collection)) = current else {
                    return Ok(());
                };
                if !collection.present {
                    return Ok(());
                }
                let element = self.collection_element_type(ty)?;
                let values = collection
                    .entries()
                    .iter()
                    .map(|(_, value)| value.clone())
                    .collect::<Vec<_>>();
                let buffer = native_buffer(&values, element);
                write_content(buffer.as_ptr().cast(), values.len());
                lent.buffers.push(buffer);
            }
            mir::UnsafeRegionValueKind::Region => {
                let Some(LocalValue::ByteRegion(region)) = current else {
                    return Ok(());
                };
                let Some(source) = region.source else {
                    return Ok(());
                };
                let element = self.region_element_type(region.kind)?;
                let (entries, index) = self.collection_element_storage(source)?;
                let values = entries
                    .borrow()
                    .get(index..index + region.length)
                    .ok_or_else(|| InterpreterError::new("a region lies inside its storage"))?
                    .iter()
                    .map(|(_, value)| value.clone())
                    .collect::<Vec<_>>();
                let buffer = native_buffer(&values, element);
                let data = buffer.as_ptr() as usize;
                write_content(data as *const u8, region.length);
                lent.regions.push(LentRegion {
                    data,
                    length: region.length,
                    stride: element_stride(element),
                    element,
                    region,
                    buffer: lent.buffers.len(),
                });
                lent.buffers.push(buffer);
            }
            mir::UnsafeRegionValueKind::HostToken => {}
        }
        Ok(())
    }

    /// The element type of the collection type `ty`.
    fn collection_element_type(&self, ty: mir::Type) -> Result<mir::Type, InterpreterError> {
        match ty {
            mir::Type::Collection(id) => self
                .program
                .collection_types
                .get(id.0)
                .map(|collection| collection.value)
                .ok_or_else(|| InterpreterError::new("a collection type exists")),
            _ => Err(InterpreterError::new("a buffer slot holds a collection")),
        }
    }

    /// The element type a region of `kind` views: a byte, or the element
    /// of its typed array.
    fn region_element_type(&self, kind: mir::RegionType) -> Result<mir::Type, InterpreterError> {
        match kind.array {
            Some(array) => self.collection_element_type(mir::Type::Collection(array)),
            None => Ok(mir::Type::Scalar(mir::ScalarType::Integer(
                crate::numeric::IntegerType::UInt8,
            ))),
        }
    }

    /// Writes the elements of the native copy of a writable region back
    /// into the host storage it copies.
    fn copy_back(&self, region: &LentRegion) -> Result<(), InterpreterError> {
        let Some(source) = region.region.source else {
            return Ok(());
        };
        let values =
            unsafe { native_elements(region.data as *const u8, region.length, region.element)? };
        let (entries, index) = self.collection_element_storage(source)?;
        let mut entries = entries.borrow_mut();
        for (offset, value) in values.into_iter().enumerate() {
            let entry = entries
                .get_mut(index + offset)
                .ok_or_else(|| InterpreterError::new("a region lies inside its storage"))?;
            entry.1 = value;
        }
        Ok(())
    }

    /// The value a slot of `value`'s local holds, for the local's type. A
    /// region crosses back as a view of the host storage whose native copy
    /// it lies in.
    fn read_slot(
        &self,
        function: &mir::Function,
        value: mir::UnsafeRegionValue,
        source: *const u8,
        regions: &[LentRegion],
    ) -> Result<Option<LocalValue>, InterpreterError> {
        let ty = local_in(function, value.local)?.ty;
        let content = || unsafe {
            let data = source.cast::<*const u8>().read_unaligned();
            let length = source.add(8).cast::<usize>().read_unaligned();
            (data, length)
        };
        match value.kind {
            mir::UnsafeRegionValueKind::Native => {
                let size = crate::native_abi::companion_slot_layout(value, ty)
                    .map_or(0, |(size, _)| size as usize);
                decode_native(ty, unsafe { std::slice::from_raw_parts(source, size) }).map(Some)
            }
            mir::UnsafeRegionValueKind::Text => {
                let text = unsafe { slot_text(source)? };
                Ok(match (ty, text) {
                    (mir::Type::NullableString, text) => Some(LocalValue::NullableString(text)),
                    (_, Some(text)) => Some(LocalValue::String(text)),
                    (_, None) => None,
                })
            }
            mir::UnsafeRegionValueKind::Buffer | mir::UnsafeRegionValueKind::ForeignString => {
                let (data, length) = content();
                if data.is_null() {
                    return Ok(None);
                }
                let mir::Type::Collection(id) = ty else {
                    return Err(InterpreterError::new("a buffer slot holds a collection"));
                };
                let element = self.collection_element_type(ty)?;
                let values = unsafe { native_elements(data, length, element)? };
                Ok(Some(LocalValue::Collection(super::CollectionValue::new(
                    id,
                    values.into_iter().map(|value| (None, value)).collect(),
                ))))
            }
            mir::UnsafeRegionValueKind::Region => {
                let (data, length) = content();
                let kind = match ty {
                    mir::Type::Region(kind) | mir::Type::NullableRegion(kind) => kind,
                    _ => return Err(InterpreterError::new("a region slot holds a region")),
                };
                let nullable = matches!(ty, mir::Type::NullableRegion(_));
                if data.is_null() {
                    return Ok(nullable.then(|| {
                        LocalValue::ByteRegion(super::byte_region::ByteRegionValue {
                            nullable: true,
                            ..super::byte_region::ByteRegionValue::absent(kind)
                        })
                    }));
                }
                let data = data as usize;
                let lent = regions.iter().find(|lent| {
                    data >= lent.data
                        && data + length * lent.stride <= lent.data + lent.length * lent.stride
                        && (data - lent.data).is_multiple_of(lent.stride.max(1))
                });
                let Some(lent) = lent else {
                    return Err(InterpreterError::new(
                        "a region over memory the companion or foreign code owns has no host form yet",
                    ));
                };
                let mut source = lent
                    .region
                    .source
                    .ok_or_else(|| InterpreterError::new("a lent region has its storage"))?;
                source.index += (data - lent.data) / lent.stride.max(1);
                Ok(Some(LocalValue::ByteRegion(
                    super::byte_region::ByteRegionValue {
                        kind,
                        nullable,
                        source: Some(source),
                        length,
                    },
                )))
            }
            mir::UnsafeRegionValueKind::HostToken => Ok(None),
        }
    }

    /// The interpreter's frames as the native frame chain a region entry
    /// links to, from the outermost: each frame of an authored callable,
    /// active at its current call. The names the chain points into come
    /// with it.
    fn native_frames(&self, depth: usize) -> (Vec<StackFrame>, Vec<String>) {
        let program = self.program;
        let owned = self.frames[..depth]
            .iter()
            .enumerate()
            .filter(|(_, frame)| program.functions[frame.function.0].owns_diagnostic_frame())
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let names = owned
            .iter()
            .map(|index| {
                let function = &program.functions[self.frames[*index].function.0];
                program.diagnostic_callable_name(function).into_owned()
            })
            .collect::<Vec<_>>();
        let mut frames: Vec<StackFrame> = Vec::with_capacity(owned.len());
        for (position, index) in owned.iter().enumerate() {
            let function = &program.functions[self.frames[*index].function.0];
            let source = program
                .source_for_span(function.source_span)
                .unwrap_or(&program.source);
            let active = self
                .frames
                .get(index + 1)
                .and_then(|next| next.entered_from)
                .unwrap_or(self.frames[*index].active_panic_site);
            let parent = frames
                .last()
                .map_or(std::ptr::null(), |frame| frame as *const StackFrame);
            frames.push(StackFrame {
                parent,
                function_name: names[position].as_ptr(),
                function_name_length: names[position].len(),
                source_path: source.path.as_ptr(),
                source_path_length: source.path.len(),
                source_text: source.text.as_ptr(),
                source_text_length: source.text.len(),
                function_span_start: function.source_span.start,
                function_span_end: function.source_span.end,
                active_span_start: active.start,
                active_span_end: active.end,
            });
        }
        // Parents point into the vector, which reserved its whole length
        // first and so never moved.
        (frames, names)
    }

    /// Runs host operation `operation` for the innermost region call.
    fn host_operation(
        &mut self,
        operation: u32,
        arguments: *mut u8,
        results: *mut u8,
    ) -> Result<(), InterpreterError> {
        let program = self.program;
        let call = self
            .region_calls
            .last()
            .ok_or_else(|| InterpreterError::new("a host operation runs inside a region call"))?;
        let plan = &program.companion.regions[call.region];
        let depth = call.depth;
        let function = function_in(program, plan.lowerings[0].function)?;
        if operation == mir::ACCEPT_OUTPUT_OPERATION {
            let offset = (arguments as usize).wrapping_sub(call.outputs as usize) as u64;
            let slot = plan
                .outputs
                .slots
                .iter()
                .find(|slot| slot.offset == offset)
                .ok_or_else(|| InterpreterError::new("operation 0 receives an output slot"))?;
            let value = self.read_slot(function, slot.value, arguments, &call.lent.regions)?;
            let call = self
                .region_calls
                .last_mut()
                .ok_or_else(|| InterpreterError::new("a region call is in progress"))?;
            let token = match value {
                Some(value) => {
                    call.accepted.push(Some(value));
                    call.accepted.len()
                }
                None => 0,
            };
            unsafe { results.cast::<usize>().write_unaligned(token) };
            return Ok(());
        }
        if let Some(callback) = program
            .companion
            .callbacks
            .iter()
            .find(|callback| callback.operation == operation)
        {
            return self.run_callback(callback, arguments, results);
        }
        let host_operation = program
            .companion
            .host_operations
            .get(operation as usize - 1)
            .filter(|host_operation| host_operation.id == operation)
            .ok_or_else(|| {
                InterpreterError::new(format!("the companion called host operation {operation}"))
            })?;
        if self.frames.len() != depth {
            return Err(InterpreterError::new(
                "a host operation runs in the frame suspended at its region",
            ));
        }
        let mut writes = Vec::new();
        for slot in &host_operation.arguments.slots {
            let source = unsafe { arguments.add(slot.offset as usize) };
            let regions = &self
                .region_calls
                .last()
                .ok_or_else(|| InterpreterError::new("a region call is in progress"))?
                .lent
                .regions;
            writes.push((
                slot.value.local,
                self.read_slot(function, slot.value, source, regions)?,
            ));
        }
        self.assign_crossed(writes)?;

        let block = host_operation.block;
        match host_operation.site {
            mir::HostOperationSite::Statement(position) => {
                let frame = self.current_frame_mut()?;
                frame.block = block;
                frame.statement_index = position;
                frame.entered_block = true;
                self.run_host_site(depth, |frame| {
                    frame.block == block && frame.statement_index == position + 1
                })?;
            }
            mir::HostOperationSite::Terminator => {
                let statements = block_in(function, block)?.statements.len();
                let frame = self.current_frame_mut()?;
                frame.block = block;
                frame.statement_index = statements;
                frame.entered_block = true;
                self.run_host_site(depth, |frame| !frame.entered_block)?;
            }
        }

        let mut lent = Lent::default();
        for slot in &host_operation.results.slots {
            let target = unsafe { results.add(slot.offset as usize) };
            self.write_slot(function, slot.value, target, &mut lent)?;
        }
        if let Some(index) = host_operation.results.index {
            let taken = self.current_frame()?.block;
            let successor = host_operation
                .successors
                .iter()
                .position(|successor| *successor == taken)
                .ok_or_else(|| {
                    InterpreterError::new("a host terminator continues at one of its targets")
                })?;
            unsafe {
                results
                    .add(index as usize)
                    .cast::<i32>()
                    .write_unaligned(successor as i32)
            };
        }
        if let Some(call) = self.region_calls.last_mut() {
            call.lent.extend(lent);
        }
        Ok(())
    }

    /// Runs the C-callable function of `callback`, which C called, with the
    /// arguments its forwarder stored, and stores its return value.
    fn run_callback(
        &mut self,
        callback: &'program mir::CompanionCallback,
        arguments: *mut u8,
        results: *mut u8,
    ) -> Result<(), InterpreterError> {
        let forwarder = &callback.forwarder;
        let mut values = Vec::with_capacity(callback.arguments.slots.len());
        for slot in &callback.arguments.slots {
            let source = unsafe { arguments.add(slot.offset as usize) };
            values.push(
                self.read_slot(forwarder, slot.value, source, &[])?
                    .ok_or_else(|| InterpreterError::new("a callback argument holds a value"))?,
            );
        }
        let site = self.foreign_call_site();
        let expectation = match forwarder.return_type {
            mir::ReturnType::Value(ty) => super::ReturnExpectation::Value(ty),
            mir::ReturnType::Void => super::ReturnExpectation::Void,
        };
        let depth = self.frames.len();
        let places = vec![None; values.len()];
        self.push_frame(callback.function, &values, &places, Some(expectation), site)?;
        self.current_frame_mut()?.foreign_caller = Some(site);
        self.run_host_site(depth, |_| true)?;
        if let Some(slot) = callback.results.slots.first() {
            let value = self.pop_local_value()?;
            let ty = forwarder.locals[slot.value.local.0].ty;
            let size = crate::native_abi::companion_slot_layout(slot.value, ty)
                .map_or(0, |(size, _)| size as usize);
            let target =
                unsafe { std::slice::from_raw_parts_mut(results.add(slot.offset as usize), size) };
            encode_native(Some(&value), ty, target);
        }
        Ok(())
    }

    /// The site of the outbound foreign call in progress, as the runtime's
    /// record holds it: the frame making the call is current there.
    fn foreign_call_site(&self) -> Option<Span> {
        let frame = self.companion.as_ref()?.foreign_call_frame();
        if frame.is_null() {
            return None;
        }
        // SAFETY: the record holds a live frame for the length of the call.
        let frame = unsafe { &*frame };
        let path = if frame.source_path.is_null() {
            ""
        } else {
            std::str::from_utf8(unsafe {
                std::slice::from_raw_parts(frame.source_path, frame.source_path_length)
            })
            .unwrap_or("")
        };
        let program = self.program;
        let source = program
            .sources
            .iter()
            .find(|unit| unit.source.path == path)
            .map(|unit| unit.source.id)
            .unwrap_or(program.source.id);
        Some(Span::in_source(
            source,
            frame.active_span_start,
            frame.active_span_end,
        ))
    }

    /// Steps the interpreter until the frame at `depth` has no tasks left
    /// and `done` holds for it. Nothing inside a region call can return to
    /// the companion with a failure, so a panic or an exit ends the process.
    fn run_host_site(
        &mut self,
        depth: usize,
        done: impl Fn(&super::CallFrame) -> bool,
    ) -> Result<(), InterpreterError> {
        loop {
            if self.frames.len() == depth {
                let frame = self.current_frame()?;
                if frame.tasks.is_empty() && done(frame) {
                    return Ok(());
                }
            }
            match self.step()? {
                StepOutcome::Continue => {}
                StepOutcome::RuntimePanic(event) => {
                    let output = self.runtime_panic_output(event);
                    end_process(output);
                }
                StepOutcome::CleanExit => {
                    let output = InterpreterOutput {
                        stdout: self.stdout.clone(),
                        stderr: self.stderr.clone(),
                        exit_status: 0,
                        runtime_diagnostic: None,
                    };
                    end_process(output);
                }
                StepOutcome::EntryReturned(_) => {
                    return Err(InterpreterError::new(
                        "the program's entry returned inside a region call",
                    ))
                }
            }
        }
    }

    /// The report of a panic the companion raised in the frame chain that
    /// ends at `frame`.
    fn companion_panic_output(
        &self,
        frame: *const StackFrame,
        code: &str,
        message: Option<String>,
        facts: Vec<RuntimeFact>,
    ) -> InterpreterOutput {
        let program = self.program;
        let mut path = Vec::new();
        let mut current = frame;
        while !current.is_null() {
            // SAFETY: the runtime validated the frame chain it reports.
            let frame = unsafe { &*current };
            let text = |data: *const u8, length: usize| {
                if data.is_null() {
                    String::new()
                } else {
                    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(data, length) })
                        .into_owned()
                }
            };
            let function = text(frame.function_name, frame.function_name_length);
            let source_path = text(frame.source_path, frame.source_path_length);
            let source = program
                .sources
                .iter()
                .find(|unit| unit.source.path == source_path && !source_path.is_empty())
                .map(|unit| unit.source.id)
                .or_else(|| (program.source.path == source_path).then_some(program.source.id));
            path.push(RuntimeOutcomeFrame {
                function,
                source: if source.is_some() {
                    DiagnosticSource::Current
                } else {
                    DiagnosticSource::Unavailable
                },
                span: Span::in_source(
                    source.unwrap_or_default(),
                    frame.active_span_start,
                    frame.active_span_end,
                ),
            });
            current = frame.parent;
        }
        let entry = doria_diagnostic_catalogue::runtime_entry(code)
            .or_else(|| doria_diagnostic_catalogue::runtime_entry("P1001"))
            .expect("the runtime catalogue has its internal panic");
        let primary_span = path.first().map_or(Span::default(), |frame| frame.span);
        let mut facts = facts;
        if let Some(message) = message.filter(|_| entry.code == "P1000") {
            facts.insert(
                0,
                RuntimeFact {
                    name: "message".to_string(),
                    value: RuntimeFactValue::StaticString(message),
                },
            );
        }
        let outcome = RuntimeOutcomeDetails {
            process_status: 101,
            termination_behavior: TerminationBehavior::AbortWithoutCleanup,
            origin: RuntimeOutcomeOrigin {
                source: path
                    .first()
                    .map_or(DiagnosticSource::Unavailable, |frame| frame.source.clone()),
                span: primary_span,
                function: path.first().map(|frame| frame.function.clone()),
            },
            path,
            facts,
            error_type: None,
        };
        self.render_runtime_panic(entry.code, primary_span, None, outcome)
    }
}

/// Writes the program's output, then its report, to the process's streams
/// and ends the process with the output's status.
fn end_process(output: InterpreterOutput) -> ! {
    use std::io::Write;
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(&output.stdout);
    let _ = stdout.flush();
    let mut stderr = std::io::stderr();
    let _ = stderr.write_all(&output.stderr);
    let _ = stderr.flush();
    std::process::exit(output.exit_status)
}

/// Ends the process after an interpreter failure inside a region call,
/// which cannot return to the companion.
fn end_with_failure(error: InterpreterError) -> ! {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "MIR interpreter failure: {error}");
    std::process::abort()
}

/// `host->call`: runs a host operation for the interpreter the record serves.
unsafe extern "C" fn host_call(
    host: *mut HostRecord,
    operation: u32,
    arguments: *mut u8,
    results: *mut u8,
) -> i32 {
    // SAFETY: a region call stored its interpreter in the record and does
    // not touch it until the entry returns.
    let interpreter = &mut *(*host).interpreter.cast::<Interpreter<'static>>();
    if let Err(error) = interpreter.host_operation(operation, arguments, results) {
        end_with_failure(error);
    }
    0
}

/// The panic sink: renders the companion's panic and ends the process.
unsafe extern "C" fn host_panic(
    host: *mut HostRecord,
    frame: *const StackFrame,
    code: *const u8,
    code_length: usize,
    message: *const u8,
    message_length: usize,
    facts: *const RuntimeFactRecord,
    fact_count: usize,
) {
    let interpreter = &*(*host).interpreter.cast::<Interpreter<'static>>();
    let text = |data: *const u8, length: usize| {
        if data.is_null() || length == 0 {
            String::new()
        } else {
            String::from_utf8_lossy(std::slice::from_raw_parts(data, length)).into_owned()
        }
    };
    let code = text(code, code_length);
    let message = (!message.is_null()).then(|| text(message, message_length));
    let facts = (0..fact_count)
        .map(|index| {
            let fact = &*facts.add(index);
            RuntimeFact {
                name: text(fact.name, fact.name_length),
                value: match fact.kind {
                    1 => RuntimeFactValue::Signed(fact.value as i64),
                    3 => RuntimeFactValue::Boolean(fact.value != 0),
                    4 => RuntimeFactValue::StaticString(text(
                        fact.value as usize as *const u8,
                        fact.value_length,
                    )),
                    _ => RuntimeFactValue::Unsigned(fact.value),
                },
            }
        })
        .collect();
    let output = interpreter.companion_panic_output(frame, &code, message, facts);
    end_process(output)
}

#[cfg(unix)]
mod platform {
    use std::ffi::{c_char, c_int, c_void, CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    #[cfg_attr(all(target_os = "linux", target_env = "gnu"), link(name = "dl"))]
    extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlerror() -> *mut c_char;
    }

    const RTLD_NOW: c_int = 2;

    fn last_error() -> String {
        // SAFETY: dlerror returns null or a NUL-terminated message.
        let message = unsafe { dlerror() };
        if message.is_null() {
            "the platform loader reported no reason".to_string()
        } else {
            unsafe { CStr::from_ptr(message) }
                .to_string_lossy()
                .into_owned()
        }
    }

    /// Loads the library at `path` for the life of the process. Its
    /// dependencies resolve through its own search path (an rpath).
    pub(super) fn open(path: &Path) -> Result<*mut c_void, String> {
        let name = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| format!("`{}` holds a NUL byte", path.display()))?;
        // SAFETY: `name` is a NUL-terminated path.
        let handle = unsafe { dlopen(name.as_ptr(), RTLD_NOW) };
        if handle.is_null() {
            Err(last_error())
        } else {
            Ok(handle)
        }
    }

    /// Loads the companion `library`, which brings the shared runtime it
    /// was linked against through its rpath, and returns it twice: a
    /// lookup through the library's handle searches its dependencies, so
    /// the host reaches the very runtime the library binds, which the
    /// dynamic linker shares among every library that names it.
    pub(super) fn open_companion(
        library: &Path,
        _runtime: &Path,
    ) -> Result<(*mut c_void, *mut c_void), String> {
        let handle = open(library)?;
        Ok((handle, handle))
    }

    pub(super) fn symbol(library: *mut c_void, name: &str) -> Result<*mut c_void, String> {
        let symbol = CString::new(name).map_err(|_| format!("`{name}` holds a NUL byte"))?;
        // SAFETY: `library` came from `open`.
        let address = unsafe { dlsym(library, symbol.as_ptr()) };
        if address.is_null() {
            Err(format!("`{name}`: {}", last_error()))
        } else {
            Ok(address)
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::{c_char, c_void, CString};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        fn GetLastError() -> u32;
    }

    /// The library's own directory first, then the default directories.
    const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: u32 = 0x100;
    const LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: u32 = 0x1000;

    /// Loads the library at `path` for the life of the process, finding its
    /// dependencies in its own directory first. A library the process
    /// loaded already (the shared runtime) satisfies a dependency by name.
    pub(super) fn open(path: &Path) -> Result<*mut c_void, String> {
        let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
        let name = absolute
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>();
        // SAFETY: `name` is a NUL-terminated absolute path.
        let handle = unsafe {
            LoadLibraryExW(
                name.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        };
        if handle.is_null() {
            Err(format!("Windows error {}", unsafe { GetLastError() }))
        } else {
            Ok(handle)
        }
    }

    /// Loads the shared runtime and then the companion `library`, whose
    /// import of the runtime resolves to the loaded module of that name. A
    /// runtime module this process loaded already is the one every library
    /// that names it binds, so the host uses that one.
    pub(super) fn open_companion(
        library: &Path,
        runtime: &Path,
    ) -> Result<(*mut c_void, *mut c_void), String> {
        let name = runtime
            .file_name()
            .ok_or_else(|| format!("`{}` names a file", runtime.display()))?
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>();
        // SAFETY: `name` is a NUL-terminated module name.
        let loaded = unsafe { GetModuleHandleW(name.as_ptr()) };
        let runtime = if loaded.is_null() {
            open(runtime)?
        } else {
            loaded
        };
        Ok((open(library)?, runtime))
    }

    pub(super) fn symbol(library: *mut c_void, name: &str) -> Result<*mut c_void, String> {
        let symbol = CString::new(name).map_err(|_| format!("`{name}` holds a NUL byte"))?;
        // SAFETY: `library` came from `open`.
        let address = unsafe { GetProcAddress(library, symbol.as_ptr()) };
        if address.is_null() {
            Err(format!("`{name}`: Windows error {}", unsafe {
                GetLastError()
            }))
        } else {
            Ok(address)
        }
    }
}
