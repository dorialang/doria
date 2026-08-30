//! Backend-neutral layout authority for the compiler-private native closure ABI.

use crate::backend::BackendError;
use crate::enums::EnumCapabilities;
use crate::mir;

pub const CARRIER_WORDS: u32 = 2;
pub const DESCRIPTOR_WORDS: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCallableHiddenInput {
    CurrentFrame,
    ResultOut,
    ErrorOut,
    BorrowHome,
    Environment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCallableSignaturePlan {
    pub hidden_inputs: Vec<NativeCallableHiddenInput>,
    pub checked: bool,
}

impl NativeCallableSignaturePlan {
    pub fn direct(function: &mir::Function) -> Self {
        Self::new(
            function.return_type,
            !function.checked_effects.is_empty(),
            function.return_borrow,
            false,
        )
    }

    pub fn indirect(function: &mir::FunctionType) -> Self {
        Self::new(
            function.return_type,
            function.has_checked_transport(),
            function.return_borrow,
            true,
        )
    }

    fn new(
        return_type: mir::ReturnType,
        checked: bool,
        return_borrow: Option<mir::ReturnBorrow>,
        environment: bool,
    ) -> Self {
        let mut hidden_inputs = vec![NativeCallableHiddenInput::CurrentFrame];
        if checked {
            if matches!(return_type, mir::ReturnType::Value(_)) {
                hidden_inputs.push(NativeCallableHiddenInput::ResultOut);
            }
            hidden_inputs.push(NativeCallableHiddenInput::ErrorOut);
        } else if matches!(
            return_type,
            mir::ReturnType::Value(mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_))
        ) {
            hidden_inputs.push(NativeCallableHiddenInput::ResultOut);
        }
        if return_borrow.is_some() && returns_function_value(return_type) {
            hidden_inputs.push(NativeCallableHiddenInput::BorrowHome);
        }
        if environment {
            hidden_inputs.push(NativeCallableHiddenInput::Environment);
        }
        Self {
            hidden_inputs,
            checked,
        }
    }

    pub fn index_of(&self, input: NativeCallableHiddenInput) -> Option<usize> {
        self.hidden_inputs
            .iter()
            .position(|candidate| *candidate == input)
    }

    pub fn source_parameter_offset(&self) -> usize {
        self.hidden_inputs.len()
    }
}

pub const fn returns_function_value(return_type: mir::ReturnType) -> bool {
    matches!(
        return_type,
        mir::ReturnType::Value(mir::Type::Function(_) | mir::Type::NullableFunction(_))
    )
}

pub fn return_borrow_source_parameter(
    function: &mir::Function,
) -> Result<Option<mir::LocalId>, BackendError> {
    let Some(return_borrow) = function.return_borrow else {
        return Ok(None);
    };
    if !returns_function_value(function.return_type) {
        return Ok(None);
    }
    let index = match return_borrow.source {
        mir::BorrowSource::Receiver => 0,
        mir::BorrowSource::Parameter(index) => {
            index
                + usize::from(function.receiver_mode.is_some())
                + usize::from(function.closure.is_some())
        }
    };
    function
        .params
        .get(index)
        .copied()
        .map(Some)
        .ok_or_else(|| {
            malformed(format!(
                "function {} return-borrow source parameter does not exist",
                function.name
            ))
        })
}

pub fn return_borrow_argument_index(return_borrow: mir::ReturnBorrow, has_receiver: bool) -> usize {
    match return_borrow.source {
        mir::BorrowSource::Receiver => 0,
        mir::BorrowSource::Parameter(index) => index + usize::from(has_receiver),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeLayout {
    pub size: u32,
    pub align: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeClosureCarrierLayout {
    pub descriptor_offset: u32,
    pub environment_offset: u32,
    pub layout: NativeLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeClosureDescriptorLayout {
    pub entry_offset: u32,
    pub drop_environment_offset: u32,
    pub layout: NativeLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeClosureEnvironmentFieldLayout {
    pub field: mir::ClosureEnvironmentFieldId,
    pub offset: u32,
    pub layout: NativeLayout,
    pub live_bit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeClosureEnvironmentLayout {
    pub logical: mir::ClosureEnvironmentLayoutId,
    pub fields: Vec<NativeClosureEnvironmentFieldLayout>,
    pub live_state_bytes: u32,
    pub layout: NativeLayout,
}

pub const fn carrier_layout(pointer_size: u32) -> NativeClosureCarrierLayout {
    NativeClosureCarrierLayout {
        descriptor_offset: 0,
        environment_offset: pointer_size,
        layout: NativeLayout {
            size: pointer_size * CARRIER_WORDS,
            align: pointer_size,
        },
    }
}

pub const fn descriptor_layout(pointer_size: u32) -> NativeClosureDescriptorLayout {
    NativeClosureDescriptorLayout {
        entry_offset: 0,
        drop_environment_offset: pointer_size,
        layout: NativeLayout {
            size: pointer_size * DESCRIPTOR_WORDS,
            align: pointer_size,
        },
    }
}

pub fn environment_layout(
    program: &mir::Program,
    logical: mir::ClosureEnvironmentLayoutId,
    pointer_size: u32,
) -> Result<NativeClosureEnvironmentLayout, BackendError> {
    let definition = program
        .closure_environment_layouts
        .get(logical.0)
        .filter(|candidate| candidate.id == logical)
        .ok_or_else(|| {
            malformed(format!(
                "closure environment layout#{} does not exist",
                logical.0
            ))
        })?;
    let mut live_bit = 0_u32;
    let live_bits = definition
        .fields
        .iter()
        .filter(|field| {
            field.storage == mir::ClosureEnvironmentStorage::Owned && needs_drop(field.ty)
        })
        .count() as u32;
    let live_state_bytes = live_bits.div_ceil(8);
    let mut offset = live_state_bytes;
    let mut align = 1_u32;
    let mut fields = Vec::with_capacity(definition.fields.len());
    let mut physical = definition.fields.iter().collect::<Vec<_>>();
    physical.sort_by_key(|field| field.physical_index);
    for field in physical {
        let field_layout = match field.storage {
            mir::ClosureEnvironmentStorage::ReadonlyBorrow
            | mir::ClosureEnvironmentStorage::WritableBorrow => NativeLayout {
                size: pointer_size,
                align: pointer_size,
            },
            mir::ClosureEnvironmentStorage::Owned => type_layout(field.ty, pointer_size),
        };
        offset = align_up(offset, field_layout.align)?;
        let field_live_bit = (field.storage == mir::ClosureEnvironmentStorage::Owned
            && needs_drop(field.ty))
        .then(|| {
            let current = live_bit;
            live_bit += 1;
            current
        });
        fields.push(NativeClosureEnvironmentFieldLayout {
            field: field.id,
            offset,
            layout: field_layout,
            live_bit: field_live_bit,
        });
        offset = offset
            .checked_add(field_layout.size)
            .ok_or_else(|| malformed("closure environment size overflow"))?;
        align = align.max(field_layout.align);
    }
    let size = align_up(offset, align)?;
    Ok(NativeClosureEnvironmentLayout {
        logical,
        fields,
        live_state_bytes,
        layout: NativeLayout { size, align },
    })
}

pub const fn type_layout(ty: mir::Type, pointer_size: u32) -> NativeLayout {
    match ty {
        mir::Type::Scalar(mir::ScalarType::Integer(ty)) => NativeLayout {
            size: ty.storage_bytes(),
            align: ty.storage_bytes(),
        },
        mir::Type::Scalar(mir::ScalarType::Float(ty)) => NativeLayout {
            size: ty.storage_bytes(),
            align: ty.storage_bytes(),
        },
        mir::Type::Scalar(mir::ScalarType::Bool) => NativeLayout { size: 1, align: 1 },
        mir::Type::Scalar(mir::ScalarType::Enum(_)) => NativeLayout { size: 4, align: 4 },
        mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::Error
        | mir::Type::NullableError
        | mir::Type::Function(_)
        | mir::Type::NullableFunction(_) => NativeLayout {
            size: pointer_size * 2,
            align: pointer_size,
        },
        mir::Type::PayloadEnum(ty) => NativeLayout {
            size: ty.size,
            align: ty.align,
        },
        mir::Type::NullablePayloadEnum(ty) => NativeLayout {
            size: ty.nullable_size,
            align: ty.align,
        },
        mir::Type::String
        | mir::Type::Mixed
        | mir::Type::NullableMixed
        | mir::Type::Class(_)
        | mir::Type::NullableClass(_)
        | mir::Type::SharedReference(_)
        | mir::Type::WeakReference(_)
        | mir::Type::NullableSharedReference(_)
        | mir::Type::NullableWeakReference(_)
        | mir::Type::WritableSharedReference(_)
        | mir::Type::WritableWeakReference(_)
        | mir::Type::NullableWritableSharedReference(_)
        | mir::Type::NullableWritableWeakReference(_)
        | mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::WritableSharedReferenceAccess(_)
        | mir::Type::NullableReadonlySharedReferenceAccess(_)
        | mir::Type::NullableWritableSharedReferenceAccess(_)
        | mir::Type::Collection(_)
        | mir::Type::NullableCollection(_)
        | mir::Type::ClosureEnvironment(_) => NativeLayout {
            size: pointer_size,
            align: pointer_size,
        },
    }
}

pub const fn needs_drop(ty: mir::Type) -> bool {
    matches!(
        ty,
        mir::Type::String
            | mir::Type::NullableString
            | mir::Type::Mixed
            | mir::Type::NullableMixed
            | mir::Type::Error
            | mir::Type::NullableError
            | mir::Type::Class(_)
            | mir::Type::NullableClass(_)
            | mir::Type::SharedReference(_)
            | mir::Type::WeakReference(_)
            | mir::Type::NullableSharedReference(_)
            | mir::Type::NullableWeakReference(_)
            | mir::Type::WritableSharedReference(_)
            | mir::Type::WritableWeakReference(_)
            | mir::Type::NullableWritableSharedReference(_)
            | mir::Type::NullableWritableWeakReference(_)
            | mir::Type::ReadonlySharedReferenceAccess(_)
            | mir::Type::WritableSharedReferenceAccess(_)
            | mir::Type::NullableReadonlySharedReferenceAccess(_)
            | mir::Type::NullableWritableSharedReferenceAccess(_)
            | mir::Type::Collection(_)
            | mir::Type::NullableCollection(_)
            | mir::Type::PayloadEnum(mir::PayloadEnumType {
                capabilities: EnumCapabilities {
                    needs_drop: true,
                    ..
                },
                ..
            })
            | mir::Type::NullablePayloadEnum(mir::PayloadEnumType {
                capabilities: EnumCapabilities {
                    needs_drop: true,
                    ..
                },
                ..
            })
            | mir::Type::Function(_)
            | mir::Type::NullableFunction(_)
    )
}

fn align_up(value: u32, align: u32) -> Result<u32, BackendError> {
    debug_assert!(align.is_power_of_two());
    value
        .checked_add(align - 1)
        .map(|value| value & !(align - 1))
        .ok_or_else(|| malformed("closure environment alignment overflow"))
}

fn malformed(message: impl Into<String>) -> BackendError {
    BackendError::new(format!(
        "backend emission failure: malformed MIR: {}",
        message.into()
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        carrier_layout, descriptor_layout, environment_layout, NativeCallableHiddenInput,
        NativeCallableSignaturePlan,
    };
    use crate::mir;

    #[test]
    fn carrier_and_descriptor_are_two_aligned_words() {
        for pointer_size in [4, 8] {
            let carrier = carrier_layout(pointer_size);
            assert_eq!(carrier.descriptor_offset, 0);
            assert_eq!(carrier.environment_offset, pointer_size);
            assert_eq!(carrier.layout.size, pointer_size * 2);
            assert_eq!(carrier.layout.align, pointer_size);

            let descriptor = descriptor_layout(pointer_size);
            assert_eq!(descriptor.entry_offset, 0);
            assert_eq!(descriptor.drop_environment_offset, pointer_size);
            assert_eq!(descriptor.layout, carrier.layout);
        }
    }

    #[test]
    fn callable_signature_plans_keep_hidden_inputs_in_one_order() {
        let function = mir::FunctionType {
            id: mir::FunctionTypeId(0),
            invocation_mode: mir::FunctionInvocationMode::Readonly,
            parameters: vec![],
            return_type: mir::ReturnType::Value(mir::Type::Function(mir::FunctionTypeId(1))),
            checked_effects: vec![mir::CheckedEffect::Any],
            ambient_checked_effects: vec![],
            test_assertion_checked_effects: vec![],
            return_borrow: Some(mir::ReturnBorrow {
                source: mir::BorrowSource::Parameter(0),
                writable: false,
            }),
        };
        let plan = NativeCallableSignaturePlan::indirect(&function);
        assert_eq!(
            plan.hidden_inputs,
            vec![
                NativeCallableHiddenInput::CurrentFrame,
                NativeCallableHiddenInput::ResultOut,
                NativeCallableHiddenInput::ErrorOut,
                NativeCallableHiddenInput::BorrowHome,
                NativeCallableHiddenInput::Environment,
            ]
        );
        assert_eq!(plan.source_parameter_offset(), 5);

        let nested = mir::FunctionType {
            id: mir::FunctionTypeId(1),
            invocation_mode: mir::FunctionInvocationMode::Once,
            parameters: vec![mir::FunctionParameter {
                mode: mir::FunctionParameterMode::Take,
                ty: mir::Type::NullableFunction(mir::FunctionTypeId(0)),
            }],
            return_type: mir::ReturnType::Value(mir::Type::Function(mir::FunctionTypeId(0))),
            checked_effects: vec![],
            ambient_checked_effects: vec![],
            test_assertion_checked_effects: vec![],
            return_borrow: None,
        };
        let nested_plan = NativeCallableSignaturePlan::indirect(&nested);
        assert_eq!(
            nested_plan.hidden_inputs,
            vec![
                NativeCallableHiddenInput::CurrentFrame,
                NativeCallableHiddenInput::Environment,
            ]
        );
        assert_eq!(nested_plan.source_parameter_offset(), 2);
    }

    #[test]
    fn escape_analysis_selects_no_stack_or_single_heap_environment_storage() {
        let program = crate::lower_source_to_mir(
            "native-closure-placement.doria",
            r#"
function escaping(string $value): function(): string
{
    return fn() with (take $value) => $value;
}

function main(): void
{
    let $local = 42;
    let $stack = fn() with ($local) => $local;
    let $none = fn() => 1;
    let $heap = escaping("owned");
    echo "{$stack()} {$none()} {$heap()}\n";
}
"#,
        )
        .expect("closure placement source should lower");

        let placements = program
            .closure_descriptors
            .iter()
            .map(|descriptor| descriptor.environment_placement)
            .collect::<Vec<_>>();
        assert!(placements.contains(&mir::ClosureEnvironmentPlacement::None));
        assert!(placements.contains(&mir::ClosureEnvironmentPlacement::Stack));
        assert!(placements.contains(&mir::ClosureEnvironmentPlacement::Heap));

        for descriptor in &program.closure_descriptors {
            let Some(logical) = descriptor.environment_layout else {
                assert_eq!(
                    descriptor.environment_placement,
                    mir::ClosureEnvironmentPlacement::None
                );
                continue;
            };
            let native = environment_layout(&program, logical, 8)
                .expect("validated closure environment should have a native layout");
            assert!(native.layout.align.is_power_of_two());
            assert_eq!(
                native.fields.len(),
                program.closure_environment_layouts[logical.0].fields.len()
            );
            let logical = &program.closure_environment_layouts[logical.0];
            for pair in native.fields.windows(2) {
                assert!(pair[0].offset + pair[0].layout.size <= pair[1].offset);
            }
            assert_eq!(
                native
                    .fields
                    .iter()
                    .filter(|field| field.live_bit.is_some())
                    .count(),
                logical
                    .fields
                    .iter()
                    .filter(|field| {
                        field.storage == mir::ClosureEnvironmentStorage::Owned
                            && super::needs_drop(field.ty)
                    })
                    .count()
            );
            assert_eq!(
                native.live_state_bytes,
                (native
                    .fields
                    .iter()
                    .filter(|field| field.live_bit.is_some())
                    .count() as u32)
                    .div_ceil(8)
            );
        }
    }
}
