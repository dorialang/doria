//! C entries of C-ABI Doria functions (Decision 0148 sections 12 and 13).
//!
//! Each C entry has the function's C signature under the target's C
//! convention, the inverse of an outbound foreign call: it receives every
//! value where the shared classification (`native_abi::c_abi`) places it,
//! rebuilds the Doria values, and calls the function as Doria code does.
//! Before the call, `dr_v1_enter_c_function` checks the thread (P1706) and
//! links the function's frame to the outbound foreign call in progress.

use super::*;
use crate::c_layout::CScalar;
use crate::native_abi::c_abi::{
    self, Extension, PartClass, PartType, PhysicalParameter, PhysicalReturn, PhysicalSignature,
};
use inkwell::attributes::Attribute;

/// The symbol of the C entry of `function`.
pub(super) fn symbol(function: mir::FunctionId) -> String {
    format!("__doria_c_entry_{}", function.0)
}

/// The C layout of a C entry's parameters and result, and their placement.
fn physical(
    program: &mir::Program,
    function: &mir::Function,
) -> Result<(Vec<mir::Type>, PhysicalSignature), BackendError> {
    let target = crate::c_layout::CTarget::host();
    let c_form = |ty| {
        mir::foreign_c_type(ty, &program.foreign_aggregates, target)
            .ok_or_else(|| malformed_mir("a C entry passes only values with a C form"))
    };
    let types = function
        .params
        .iter()
        .map(|local| local_in(function, *local).map(|local| local.ty))
        .collect::<Result<Vec<_>, _>>()?;
    let parameters = types
        .iter()
        .map(|ty| c_form(*ty))
        .collect::<Result<Vec<_>, _>>()?;
    let result = match function.return_type {
        mir::ReturnType::Value(ty) => Some(c_form(ty)?),
        mir::ReturnType::Void => None,
    };
    let classification = c_abi::classify(target, &parameters, result.as_ref(), None);
    Ok((
        types,
        c_abi::physical_signature(target, &parameters, &classification),
    ))
}

fn scalar_type(context: &Context, scalar: CScalar) -> BasicTypeEnum<'_> {
    match scalar {
        CScalar::Integer { bytes: 1, .. } | CScalar::Bool => context.i8_type().into(),
        CScalar::Integer { bytes: 2, .. } => context.i16_type().into(),
        CScalar::Integer { bytes: 4, .. } => context.i32_type().into(),
        CScalar::Integer { .. } => context.i64_type().into(),
        CScalar::Float { bytes: 4 } => context.f32_type().into(),
        CScalar::Float { .. } => context.f64_type().into(),
        CScalar::Pointer => context.ptr_type(AddressSpace::default()).into(),
    }
}

pub(super) fn part_type(context: &Context, ty: PartType) -> BasicTypeEnum<'_> {
    match ty {
        PartType::Integer64 => context.i64_type().into(),
        PartType::Float32 => context.f32_type().into(),
        PartType::Float64 => context.f64_type().into(),
    }
}

/// The LLVM type of a C function the classification placed as `physical`,
/// with the parameter attributes C expects: `sret` and `byval` images of
/// their sizes, `align` for stack images, and the extension of narrow
/// integers. `result_size` is the size of an aggregate result. Outbound
/// foreign calls and C entries share it.
pub(super) fn c_function_type<'ctx>(
    context: &'ctx Context,
    physical: &PhysicalSignature,
    result_size: u64,
) -> (inkwell::types::FunctionType<'ctx>, Vec<(u32, Attribute)>) {
    let pointer = context.ptr_type(AddressSpace::default());
    let word = context.i64_type();
    let attribute = |name: &str, value: u64| {
        context.create_enum_attribute(Attribute::get_named_enum_kind_id(name), value)
    };
    let image = |size: u64| context.i8_type().array_type(size as u32);
    let type_attribute = |name: &str, size: u64| {
        context.create_type_attribute(
            Attribute::get_named_enum_kind_id(name),
            image(size).as_any_type_enum(),
        )
    };
    let mut parameter_types = Vec::with_capacity(physical.parameters.len());
    let mut attributes = Vec::new();
    for (index, parameter) in physical.parameters.iter().enumerate() {
        let index = index as u32;
        parameter_types.push(match parameter {
            PhysicalParameter::ResultAddress => {
                attributes.push((index, type_attribute("sret", result_size)));
                pointer.into()
            }
            PhysicalParameter::Scalar { scalar, .. } => {
                if let Some(extension) = c_abi::extension(*scalar) {
                    let name = match extension {
                        Extension::Sign => "signext",
                        Extension::Zero => "zeroext",
                    };
                    attributes.push((index, attribute(name, 0)));
                }
                scalar_type(context, *scalar)
            }
            PhysicalParameter::Part { ty, .. } => part_type(context, *ty),
            PhysicalParameter::Padding(PartClass::Integer) => word.into(),
            PhysicalParameter::Padding(PartClass::Float) => context.f64_type().into(),
            PhysicalParameter::StackImage { size, .. } => {
                attributes.push((index, type_attribute("byval", *size)));
                attributes.push((index, attribute("align", 8)));
                pointer.into()
            }
            PhysicalParameter::CopyAddress { .. } => pointer.into(),
        });
    }
    // A variadic function (Decision 0148 section 14) declares its fixed
    // parameters; LLVM places the variadic ones as the convention requires.
    let variadic = physical.fixed_parameters.is_some();
    let parameter_types = parameter_types
        .into_iter()
        .take(physical.fixed_parameters.unwrap_or(usize::MAX))
        .map(BasicMetadataTypeEnum::from)
        .collect::<Vec<_>>();
    // The bits above a narrow result are not relied on.
    let function_type = match &physical.return_value {
        PhysicalReturn::Scalar(scalar) => {
            scalar_type(context, *scalar).fn_type(&parameter_types, variadic)
        }
        PhysicalReturn::Parts(parts) if parts.len() == 1 => {
            part_type(context, parts[0].1).fn_type(&parameter_types, variadic)
        }
        PhysicalReturn::Parts(parts) => context
            .struct_type(
                &parts
                    .iter()
                    .map(|(_, ty)| part_type(context, *ty))
                    .collect::<Vec<_>>(),
                false,
            )
            .fn_type(&parameter_types, variadic),
        PhysicalReturn::Void | PhysicalReturn::Indirect => {
            context.void_type().fn_type(&parameter_types, variadic)
        }
    };
    (function_type, attributes)
}

fn aggregate_size(function: &mir::Function) -> u64 {
    match function.return_type {
        mir::ReturnType::Value(mir::Type::ForeignAggregate(aggregate)) => u64::from(aggregate.size),
        _ => 0,
    }
}

/// Declares the C entry of every C-ABI function of `program`.
pub(super) fn declare<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    program: &mir::Program,
) -> Result<Vec<(mir::FunctionId, FunctionValue<'ctx>)>, BackendError> {
    program
        .c_entries
        .iter()
        .map(|id| {
            let function = function_in(program, *id)?;
            let (_, physical) = physical(program, function)?;
            let (ty, attributes) = c_function_type(context, &physical, aggregate_size(function));
            let entry = module.add_function(&symbol(*id), ty, Some(Linkage::Internal));
            for (index, attribute) in attributes {
                entry.add_attribute(AttributeLoc::Param(index), attribute);
            }
            Ok((*id, entry))
        })
        .collect()
}

/// The runtime's record of the outbound foreign call in progress
/// (`dr_v1_foreign_call_frame`).
pub(super) fn foreign_call_record<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
) -> PointerValue<'ctx> {
    let name = crate::native_abi::FOREIGN_CALL_FRAME;
    module
        .get_global(name)
        .unwrap_or_else(|| {
            let global = module.add_global(context.ptr_type(AddressSpace::default()), None, name);
            global.set_linkage(Linkage::External);
            global
        })
        .as_pointer_value()
}

/// The frame that describes `function` for a P1706 report: its name and
/// declaration, with no parent, over the name and source data the
/// function's own frame refers to.
fn entry_frame<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    program: &mir::Program,
    function: &mir::Function,
) -> Result<PointerValue<'ctx>, BackendError> {
    let pointer = context.ptr_type(AddressSpace::default());
    let usize_type = context.ptr_sized_int_type(target_data, None);
    let callable_name = program.diagnostic_callable_name(function);
    let source = program
        .source_for_span(function.source_span)
        .unwrap_or(&program.source);
    let data = |name: String| {
        module
            .get_global(&name)
            .map(|global| global.as_pointer_value())
            .ok_or_else(|| {
                malformed_mir("a C entry's function defines its own diagnostic frame data")
            })
    };
    let symbols = frame_data_symbols(function.id);
    let word = |value: usize| usize_type.const_int(value as u64, false).into();
    let fields: [BasicValueEnum<'ctx>; 11] = [
        pointer.const_null().into(),
        data(symbols.name)?.into(),
        word(callable_name.len()),
        data(symbols.path)?.into(),
        word(source.path.len()),
        data(symbols.text)?.into(),
        word(source.text.len()),
        word(function.source_span.start),
        word(function.source_span.end),
        word(function.source_span.start),
        word(function.source_span.end),
    ];
    let value = context.const_struct(&fields, false);
    let global = module.add_global(
        value.get_type(),
        None,
        &format!("__doria_c_entry_frame_{}", function.id.0),
    );
    global.set_initializer(&value);
    global.set_constant(true);
    global.set_linkage(Linkage::Private);
    Ok(global.as_pointer_value())
}

/// Defines every declared C entry, after the functions they call.
pub(super) fn define<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    program: &mir::Program,
    entries: &[(mir::FunctionId, FunctionValue<'ctx>)],
    declarations: &DeclaredProgram<'ctx>,
) -> Result<(), BackendError> {
    for (id, entry) in entries {
        let function = function_in(program, *id)?;
        let frame = entry_frame(context, module, target_data, program, function)?;
        let callee = *declarations
            .functions
            .get(id.0)
            .ok_or_else(|| malformed_mir("a C entry's function was not declared"))?;
        emit(context, module, program, function, *entry, frame, callee)?;
    }
    Ok(())
}

fn emit<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    program: &mir::Program,
    function: &mir::Function,
    entry: FunctionValue<'ctx>,
    frame: PointerValue<'ctx>,
    callee: FunctionValue<'ctx>,
) -> Result<(), BackendError> {
    let (types, physical) = physical(program, function)?;
    let pointer = context.ptr_type(AddressSpace::default());
    let word = context.i64_type();
    let builder = context.create_builder();
    let block = context.append_basic_block(entry, "entry");
    builder.position_at_end(block);
    let received = entry.get_params();

    // The thread check and the frame link (section 13).
    let usize_type = word;
    let frame_type = context.struct_type(&[usize_type.into(); 11], false);
    let marker = build(builder.build_alloca(frame_type, "c.entry.marker"))?;
    let enter = module
        .get_function(crate::native_abi::ENTER_C_FUNCTION)
        .unwrap_or_else(|| {
            module.add_function(
                crate::native_abi::ENTER_C_FUNCTION,
                pointer.fn_type(&[pointer.into(), pointer.into()], false),
                Some(Linkage::External),
            )
        });
    let parent =
        build(builder.build_call(enter, &[marker.into(), frame.into()], "c.entry.parent"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| backend_failure("the C entry runtime call returns the parent frame"))?;

    let mut aggregates: HashMap<usize, PointerValue<'ctx>> = HashMap::new();
    let mut scalars: HashMap<usize, BasicValueEnum<'ctx>> = HashMap::new();
    let mut result_address = None;
    for (parameter, value) in physical.parameters.iter().zip(&received) {
        match parameter {
            PhysicalParameter::ResultAddress => result_address = Some(value.into_pointer_value()),
            PhysicalParameter::Scalar { argument, .. } => {
                scalars.insert(*argument, *value);
            }
            PhysicalParameter::Part {
                argument, offset, ..
            } => {
                let mir::Type::ForeignAggregate(aggregate) = types[*argument] else {
                    return Err(malformed_mir("register parts carry a foreign aggregate"));
                };
                let image = match aggregates.get(argument) {
                    Some(image) => *image,
                    None => {
                        let size = c_abi::padded_image_size(u64::from(aggregate.size));
                        let image = build(builder.build_alloca(
                            context.i8_type().array_type(size as u32),
                            "c.entry.image",
                        ))?;
                        set_alloca_alignment(image, 8)?;
                        aggregates.insert(*argument, image);
                        image
                    }
                };
                let address = unsafe {
                    build(builder.build_gep(
                        context.i8_type(),
                        image,
                        &[word.const_int(*offset, false)],
                        "c.entry.part",
                    ))?
                };
                build(builder.build_store(address, *value))?;
            }
            PhysicalParameter::Padding(_) => {}
            // C passed the address of its own copy.
            PhysicalParameter::StackImage { argument, .. }
            | PhysicalParameter::CopyAddress { argument } => {
                aggregates.insert(*argument, value.into_pointer_value());
            }
        }
    }

    let plan = native_closure_abi::NativeCallableSignaturePlan::direct(function);
    let mut values: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
    let mut result_slot = None;
    for input in &plan.hidden_inputs {
        values.push(match input {
            native_closure_abi::NativeCallableHiddenInput::CurrentFrame => parent.into(),
            native_closure_abi::NativeCallableHiddenInput::ResultOut => {
                let mir::ReturnType::Value(mir::Type::ForeignAggregate(aggregate)) =
                    function.return_type
                else {
                    return Err(malformed_mir("a C entry's result slot holds an aggregate"));
                };
                let slot = build(builder.build_alloca(
                    context.i8_type().array_type(aggregate.size.max(1)),
                    "c.entry.result",
                ))?;
                set_alloca_alignment(slot, aggregate.align.max(1))?;
                result_slot = Some((slot, aggregate));
                slot.into()
            }
            _ => {
                return Err(malformed_mir(
                    "a C entry's function takes only its frame and result slot",
                ))
            }
        });
    }
    for (index, ty) in types.iter().enumerate() {
        if let mir::Type::ForeignAggregate(_) = ty {
            values.push(
                (*aggregates
                    .get(&index)
                    .ok_or_else(|| malformed_mir("C passed every aggregate argument"))?)
                .into(),
            );
            continue;
        }
        let value = *scalars
            .get(&index)
            .ok_or_else(|| malformed_mir("C passed every scalar argument"))?;
        let value: BasicValueEnum<'ctx> = match ty {
            // C `_Bool` is 0 or 1; any other byte still reads as `true`.
            mir::Type::Scalar(mir::ScalarType::Bool) => {
                let set = build(builder.build_int_compare(
                    IntPredicate::NE,
                    value.into_int_value(),
                    context.i8_type().const_zero(),
                    "c.entry.bool",
                ))?;
                build(builder.build_int_z_extend(set, context.i8_type(), "c.entry.bool.byte"))?
                    .into()
            }
            // A pointer, nullable or not, is its address word on both sides;
            // a null `?Pointer<T>` is address zero.
            mir::Type::Scalar(mir::ScalarType::Inline(_)) | mir::Type::NullableScalar(_) => build(
                builder.build_ptr_to_int(value.into_pointer_value(), word, "c.entry.address"),
            )?
            .into(),
            _ => value,
        };
        values.push(value.into());
    }

    let call = build(builder.build_call(callee, &values, "c.entry.call"))?;
    let returned = call.try_as_basic_value().basic();
    match (&physical.return_value, function.return_type) {
        (_, mir::ReturnType::Void) => {
            build(builder.build_return(None))?;
        }
        (PhysicalReturn::Indirect, _) => {
            let (source, aggregate) =
                result_slot.ok_or_else(|| malformed_mir("an aggregate result has a slot"))?;
            let destination =
                result_address.ok_or_else(|| malformed_mir("C passed the result address"))?;
            build(builder.build_memcpy(
                destination,
                1,
                source,
                aggregate.align.max(1),
                word.const_int(u64::from(aggregate.size), false),
            ))?;
            build(builder.build_return(None))?;
        }
        (PhysicalReturn::Parts(parts), _) => {
            let (source, aggregate) =
                result_slot.ok_or_else(|| malformed_mir("an aggregate result has a slot"))?;
            let size = c_abi::padded_image_size(u64::from(aggregate.size));
            let image_type = context.i8_type().array_type(size as u32);
            let image = build(builder.build_alloca(image_type, "c.entry.result.image"))?;
            set_alloca_alignment(image, 8)?;
            build(builder.build_store(image, image_type.const_zero()))?;
            build(builder.build_memcpy(
                image,
                8,
                source,
                aggregate.align.max(1),
                word.const_int(u64::from(aggregate.size), false),
            ))?;
            let mut loaded = Vec::with_capacity(parts.len());
            for (offset, ty) in parts {
                let address = unsafe {
                    build(builder.build_gep(
                        context.i8_type(),
                        image,
                        &[word.const_int(*offset, false)],
                        "c.entry.result.part",
                    ))?
                };
                loaded.push(build(builder.build_load(
                    part_type(context, *ty),
                    address,
                    "c.entry.result.part",
                ))?);
            }
            if loaded.len() == 1 {
                build(builder.build_return(Some(&loaded[0])))?;
            } else {
                build(builder.build_aggregate_return(&loaded))?;
            }
        }
        (PhysicalReturn::Scalar(_), mir::ReturnType::Value(ty)) => {
            let value =
                returned.ok_or_else(|| malformed_mir("a C entry's function returns its value"))?;
            let value: BasicValueEnum<'ctx> = match ty {
                // An absent `?Pointer<T>` is already address zero.
                mir::Type::Scalar(mir::ScalarType::Inline(_)) | mir::Type::NullableScalar(_) => {
                    build(builder.build_int_to_ptr(
                        value.into_int_value(),
                        pointer,
                        "c.entry.result.address",
                    ))?
                    .into()
                }
                _ => value,
            };
            build(builder.build_return(Some(&value)))?;
        }
        (PhysicalReturn::Void, mir::ReturnType::Value(_)) => {
            return Err(malformed_mir("a C entry with a result returns it"));
        }
    }
    Ok(())
}
