//! Backend-neutral layout authority for the compiler-private native closure ABI.

use crate::backend::BackendError;
use crate::enums::EnumCapabilities;
use crate::mir;

pub const CARRIER_WORDS: u32 = 2;
pub const DESCRIPTOR_WORDS: u32 = 2;

/// A borrow home retains the original storage address, not an intermediate
/// receiver's stack slot. Representation projections also retain the storage's type:
/// an interface vtable is not an open-class descriptor, even though both
/// carriers have two words. Call arguments point to a transient descriptor;
/// borrowed environment fields copy its words rather than retaining that pointer.
pub const BORROW_HOME_WORDS: u32 = 2;

pub fn borrow_home_type_key(ty: mir::Type) -> u64 {
    match ty {
        mir::Type::Class(id) | mir::Type::NullableClass(id) => ((id.0 as u64) << 2) | 1,
        mir::Type::Interface(id) | mir::Type::NullableInterface(id) => ((id.0 as u64) << 2) | 2,
        mir::Type::Collection(id) | mir::Type::NullableCollection(id) => ((id.0 as u64) << 2) | 3,
        // Nonzero multiples of four are disjoint from nominal keys. Unlike
        // pointer-based nullable carriers, a nullable enum has a presence
        // prefix, so its physical nullability must remain part of the key.
        mir::Type::PayloadEnum(ty) => (ty.id.0 as u64 + 1) << 3,
        mir::Type::NullablePayloadEnum(ty) => ((ty.id.0 as u64 + 1) << 3) | 4,
        _ => 0,
    }
}

pub fn local_needs_borrow_home(function: &mir::Function, local: &mir::Local) -> bool {
    !local.owned
        && local.ty.has_move_ownership()
        && !function.params.contains(&local.id)
        && !function.closure.as_ref().is_some_and(|closure| {
            closure
                .capture_locals
                .iter()
                .any(|(_, capture)| *capture == local.id)
        })
}

/// An independently owned temporary has no preexisting source place. Native
/// consumers may give its evaluated carrier stable statement-scoped storage;
/// the existing temporary cleanup remains its only ownership obligation.
/// Never manufacture a replacement home for missing borrowed provenance.
pub fn needs_owned_borrow_home(value: &mir::Rvalue, function: &mir::Function) -> bool {
    value.ty().has_move_ownership()
        && !value.is_null_value()
        && !value.borrows_move_value()
        && !has_addressable_borrow_home(value, function)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowHomeProjection {
    Direct,
    NullableWordPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowHomePlace {
    Local {
        local: mir::LocalId,
        projection: BorrowHomeProjection,
    },
    Property {
        object: mir::LocalId,
        property: crate::class_layout::PropertyId,
    },
}

/// Exact places shared by every native value family. Runtime projections such
/// as mixed unboxing and call results are deliberately not guessed here.
pub fn direct_borrow_home_place(value: &mir::Rvalue) -> Option<BorrowHomePlace> {
    use BorrowHomePlace as P;
    use BorrowHomeProjection as Q;
    let projected_local = match value {
        mir::Rvalue::String(mir::StringExpression::NullableLocalAssumeNonNull(local)) => {
            Some((*local, Q::NullableWordPayload))
        }
        mir::Rvalue::Value(
            mir::ValueExpression::Integer(mir::IntegerExpression::Use {
                operand: mir::Operand::NullablePayload(local),
                ..
            })
            | mir::ValueExpression::Float(mir::FloatExpression::Use {
                operand: mir::Operand::NullablePayload(local),
                ..
            })
            | mir::ValueExpression::Bool(mir::BoolExpression::Use {
                operand: mir::Operand::NullablePayload(local),
            })
            | mir::ValueExpression::Enum(mir::EnumExpression::Use {
                operand: mir::Operand::NullablePayload(local),
                ..
            }),
        ) => Some((*local, Q::NullableWordPayload)),
        mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Use {
            place: mir::PayloadEnumPlace::NullableLocalAssumeNonNull(local),
            ..
        }) => Some((*local, Q::Direct)),
        _ => None,
    };
    if let Some((local, projection)) = projected_local {
        return Some(P::Local { local, projection });
    }
    if let Some(local) = value.direct_place_local() {
        return Some(P::Local {
            local,
            projection: Q::Direct,
        });
    }
    fn shared(value: crate::native_shared::Expression<'_>) -> Option<BorrowHomePlace> {
        use crate::native_shared::Operation as O;
        match value.operation() {
            O::Local { local, .. } => Some(P::Local {
                local,
                projection: Q::Direct,
            }),
            O::Property { object, property } => Some(P::Property { object, property }),
            O::Present(value) => shared(value),
            _ => None,
        }
    }
    if let Some(value) = crate::native_shared::Expression::from_rvalue(value) {
        return shared(value);
    }
    match value {
        mir::Rvalue::Mixed(mir::MixedExpression::Property { object, property })
        | mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Property { object, property })
        | mir::Rvalue::Function(mir::FunctionExpression::Property {
            object, property, ..
        })
        | mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Property {
            object,
            property,
            ..
        })
        | mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Use {
            place: mir::PayloadEnumPlace::Property { object, property },
            ..
        })
        | mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Use {
            place: mir::PayloadEnumPlace::Property { object, property },
            ..
        }) => Some(P::Property {
            object: *object,
            property: *property,
        }),
        mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Mixed(value)) => {
            direct_borrow_home_place(&mir::Rvalue::Mixed(value.clone()))
        }
        mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Present(value)) => {
            direct_borrow_home_place(&mir::Rvalue::Function(value.clone()))
        }
        mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Value(value)) => {
            direct_borrow_home_place(&mir::Rvalue::PayloadEnum(value.clone()))
        }
        mir::Rvalue::Function(mir::FunctionExpression::AssumePresent { value, .. }) => {
            direct_borrow_home_place(&mir::Rvalue::NullableFunction((**value).clone()))
        }
        _ => None,
    }
}

/// A lifetime root on a call result is not an exact place: a method borrowing
/// `$this` may return one of its fields. Only explicit MIR places can forward
/// their home without additional return-place provenance.
pub fn has_addressable_borrow_home(value: &mir::Rvalue, function: &mir::Function) -> bool {
    if direct_borrow_home_place(value).is_some() {
        return true;
    }
    fn nominal_local(function: &mir::Function, local: mir::LocalId) -> bool {
        function.locals.get(local.0).is_some_and(|local| {
            matches!(
                local.ty,
                mir::Type::Class(_)
                    | mir::Type::NullableClass(_)
                    | mir::Type::Interface(_)
                    | mir::Type::NullableInterface(_)
            )
        })
    }
    fn class(value: &mir::ClassExpression, function: &mir::Function) -> bool {
        match value {
            mir::ClassExpression::Local { .. }
            | mir::ClassExpression::NullableLocalAssumeNonNull { .. }
            | mir::ClassExpression::Property { .. }
            | mir::ClassExpression::InterfaceReceiver { .. } => true,
            mir::ClassExpression::InterfacePayload { local, .. } => nominal_local(function, *local),
            _ => false,
        }
    }
    fn nullable_class(value: &mir::NullableClassExpression, function: &mir::Function) -> bool {
        match value {
            mir::NullableClassExpression::Class(value) => class(value, function),
            mir::NullableClassExpression::Local { .. }
            | mir::NullableClassExpression::Property { .. } => true,
            _ => false,
        }
    }
    fn interface(value: &mir::InterfaceValue, function: &mir::Function) -> bool {
        match value {
            mir::InterfaceValue::Local { .. }
            | mir::InterfaceValue::NullableLocalAssumeNonNull { .. }
            | mir::InterfaceValue::Property { .. } => true,
            mir::InterfaceValue::NarrowedLocal { local, .. } => nominal_local(function, *local),
            mir::InterfaceValue::Upcast { source, .. } => interface(&source.value, function),
            mir::InterfaceValue::FromClass { object, .. } => class(object, function),
            mir::InterfaceValue::FromNullableClass { object, .. } => {
                nullable_class(object, function)
            }
            mir::InterfaceValue::FromCollection { value, .. } => {
                has_addressable_borrow_home(value, function)
            }
            _ => false,
        }
    }
    fn nullable_interface(value: &mir::NullableInterfaceValue, function: &mir::Function) -> bool {
        match value {
            mir::NullableInterfaceValue::Local { .. }
            | mir::NullableInterfaceValue::Property { .. } => true,
            mir::NullableInterfaceValue::Present(value) => interface(value, function),
            mir::NullableInterfaceValue::Upcast { source, .. } => {
                nullable_interface(&source.value, function)
            }
            _ => false,
        }
    }
    match value {
        mir::Rvalue::Class(value) => class(value, function),
        mir::Rvalue::NullableClass(value) => nullable_class(value, function),
        mir::Rvalue::Interface(value) => interface(&value.value, function),
        mir::Rvalue::NullableInterface(value) => nullable_interface(&value.value, function),
        mir::Rvalue::Collection(
            mir::CollectionExpression::Local { .. }
            | mir::CollectionExpression::Property { .. }
            | mir::CollectionExpression::InterfaceReceiver { .. },
        )
        | mir::Rvalue::NullableCollection(
            mir::NullableCollectionExpression::Local { .. }
            | mir::NullableCollectionExpression::Property { .. },
        ) => true,
        mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Collection(value)) => {
            has_addressable_borrow_home(&mir::Rvalue::Collection(value.clone()), function)
        }
        _ => false,
    }
}

/// Nominal views and nullable enum views can change representation. Other
/// homes point directly at the checked value, including narrowed Copy words.
pub fn borrow_home_projection_types(program: &mir::Program, target: mir::Type) -> Vec<mir::Type> {
    if let mir::Type::PayloadEnum(ty) | mir::Type::NullablePayloadEnum(ty) = target {
        return vec![
            mir::Type::PayloadEnum(ty),
            mir::Type::NullablePayloadEnum(ty),
        ];
    }
    if borrow_home_type_key(target) == 0 {
        return Vec::new();
    }
    program.classes.iter().map(|class| mir::Type::Class(class.id))
        .chain(program.interface_types.iter().map(|interface| mir::Type::Interface(interface.id)))
        .chain(program.collection_types.iter().map(|collection| mir::Type::Collection(collection.id)))
        .filter(|source| match (*source, target) {
            (mir::Type::Collection(source), mir::Type::Interface(target) | mir::Type::NullableInterface(target)) => program.interface_vtable(mir::ImplementingType::Collection(source), target).is_some(),
            (mir::Type::Interface(source), mir::Type::Collection(target) | mir::Type::NullableCollection(target)) => program.interface_vtable(mir::ImplementingType::Collection(target), source).is_some(),
            (mir::Type::Collection(source), mir::Type::Collection(target) | mir::Type::NullableCollection(target)) => source == target,
            (mir::Type::Collection(_), mir::Type::Class(_) | mir::Type::NullableClass(_))
            | (mir::Type::Class(_), mir::Type::Collection(_) | mir::Type::NullableCollection(_)) => false,
            _ => true,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCallableHiddenInput {
    CurrentFrame,
    ResultOut,
    ErrorOut,
    BorrowHome,
    ResultBorrowHomeOut,
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

    pub fn interface_entry(function: &mir::FunctionType) -> Self {
        Self::new(
            function.return_type,
            function.has_checked_transport(),
            function.return_borrow,
            false,
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
        if return_borrow.is_some()
            && (returns_function_value(return_type)
                || returns_borrowed_value(return_type, return_borrow))
        {
            hidden_inputs.push(NativeCallableHiddenInput::BorrowHome);
        }
        if returns_borrowed_value(return_type, return_borrow) {
            hidden_inputs.push(NativeCallableHiddenInput::ResultBorrowHomeOut);
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

/// A returned Move borrow transports its exact runtime place independently of
/// the lifetime root. Borrowed stored closure carriers use this output as well;
/// an owned closure retaining capture borrows does not.
pub const fn returns_borrowed_value(
    return_type: mir::ReturnType,
    return_borrow: Option<mir::ReturnBorrow>,
) -> bool {
    match return_type {
        mir::ReturnType::Value(ty) => ty.borrows_returned_value(return_borrow),
        mir::ReturnType::Void => false,
    }
}

pub fn return_borrow_source_parameter(
    function: &mir::Function,
) -> Result<Option<mir::LocalId>, BackendError> {
    let Some(return_borrow) = function.return_borrow else {
        return Ok(None);
    };
    if !returns_function_value(function.return_type)
        && !returns_borrowed_value(function.return_type, function.return_borrow)
    {
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
                size: pointer_size * BORROW_HOME_WORDS,
                align: pointer_size,
            },
            mir::ClosureEnvironmentStorage::Owned => value_layout(program, field.ty, pointer_size),
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
    if ty.shared_interface().is_some() {
        return NativeLayout {
            size: pointer_size * 2,
            align: pointer_size,
        };
    }
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
        | mir::Type::Interface(_)
        | mir::Type::NullableInterface(_)
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

pub fn value_layout(program: &mir::Program, ty: mir::Type, pointer_size: u32) -> NativeLayout {
    match ty {
        mir::Type::Class(class) | mir::Type::NullableClass(class)
            if program
                .classes
                .get(class.0)
                .is_some_and(|definition| definition.is_open) =>
        {
            NativeLayout {
                size: pointer_size * 2,
                align: pointer_size,
            }
        }
        _ => type_layout(ty, pointer_size),
    }
}

pub const fn needs_drop(ty: mir::Type) -> bool {
    matches!(
        ty,
        mir::Type::String
            | mir::Type::NullableString
            | mir::Type::Mixed
            | mir::Type::NullableMixed
            | mir::Type::Interface(_)
            | mir::Type::NullableInterface(_)
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
    fn borrow_home_keys_preserve_storage_identity_across_nullable_views() {
        use super::borrow_home_type_key as key;
        let class = crate::class_layout::ClassId(3);
        let interface = mir::InterfaceTypeId(3);
        let collection = mir::CollectionTypeId(3);
        assert_eq!(
            key(mir::Type::Class(class)),
            key(mir::Type::NullableClass(class))
        );
        assert_eq!(
            key(mir::Type::Interface(interface)),
            key(mir::Type::NullableInterface(interface))
        );
        assert_eq!(
            key(mir::Type::Collection(collection)),
            key(mir::Type::NullableCollection(collection))
        );
        let payload_enum = mir::PayloadEnumType {
            id: crate::enums::EnumId(3),
            capabilities: crate::enums::EnumCapabilities {
                copy: false,
                trivial_copy: false,
                needs_drop: true,
                equality: false,
            },
            size: 16,
            align: 8,
            nullable_size: 24,
            nullable_payload_offset: 8,
        };
        let keys = [
            key(mir::Type::Class(class)),
            key(mir::Type::Interface(interface)),
            key(mir::Type::Collection(collection)),
            key(mir::Type::Class(crate::class_layout::ClassId(4))),
            key(mir::Type::PayloadEnum(payload_enum)),
            key(mir::Type::NullablePayloadEnum(payload_enum)),
            key(mir::Type::String),
        ];
        for (index, key) in keys.iter().enumerate() {
            assert!(!keys[index + 1..].contains(key));
        }
    }

    #[test]
    fn every_nonowned_move_family_preserves_explicit_alias_homes() {
        let program =
            crate::lower_source_to_mir("borrow-home-types.doria", "function main(): void {}")
                .expect("empty entry should lower");
        let mut function = program.functions[program.entry.0].clone();
        let payload = mir::SharedPayload::Class(crate::class_layout::ClassId(0));
        let enum_ty = mir::PayloadEnumType {
            id: crate::enums::EnumId(0),
            capabilities: crate::enums::EnumCapabilities {
                copy: false,
                trivial_copy: false,
                needs_drop: true,
                equality: false,
            },
            size: 16,
            align: 8,
            nullable_size: 24,
            nullable_payload_offset: 8,
        };
        let types = [
            mir::Type::Mixed,
            mir::Type::NullableMixed,
            mir::Type::Function(mir::FunctionTypeId(0)),
            mir::Type::NullableFunction(mir::FunctionTypeId(0)),
            mir::Type::SharedReference(payload),
            mir::Type::WeakReference(payload),
            mir::Type::NullableSharedReference(payload),
            mir::Type::NullableWeakReference(payload),
            mir::Type::WritableSharedReference(payload),
            mir::Type::WritableWeakReference(payload),
            mir::Type::NullableWritableSharedReference(payload),
            mir::Type::NullableWritableWeakReference(payload),
            mir::Type::ReadonlySharedReferenceAccess(payload),
            mir::Type::WritableSharedReferenceAccess(payload),
            mir::Type::NullableReadonlySharedReferenceAccess(payload),
            mir::Type::NullableWritableSharedReferenceAccess(payload),
            mir::Type::PayloadEnum(enum_ty),
            mir::Type::NullablePayloadEnum(enum_ty),
        ];
        for ty in types {
            let mut local = mir::Local {
                id: mir::LocalId(0),
                name: "alias".into(),
                ty,
                writable: false,
                owned: false,
                synthetic: false,
            };
            assert!(super::local_needs_borrow_home(&function, &local), "{ty}");
            local.owned = true;
            assert!(!super::local_needs_borrow_home(&function, &local), "{ty}");
        }
        function.params.push(mir::LocalId(0));
        let parameter = mir::Local {
            id: mir::LocalId(0),
            name: "parameter".into(),
            ty: mir::Type::Mixed,
            writable: false,
            owned: false,
            synthetic: false,
        };
        assert!(!super::local_needs_borrow_home(&function, &parameter));

        let narrowed = mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Use {
            ty: enum_ty,
            place: mir::PayloadEnumPlace::NullableLocalAssumeNonNull(mir::LocalId(0)),
            mode: mir::PayloadEnumUseMode::Borrow,
        });
        assert_eq!(
            super::direct_borrow_home_place(&narrowed),
            Some(super::BorrowHomePlace::Local {
                local: mir::LocalId(0),
                projection: super::BorrowHomeProjection::Direct,
            })
        );
        let property = crate::class_layout::PropertyId {
            class: crate::class_layout::ClassId(0),
            index: 0,
        };
        let shared =
            mir::Rvalue::NullableSharedReference(mir::NullableSharedReferenceExpression::Shared(
                mir::SharedReferenceExpression::Property {
                    payload,
                    object: mir::LocalId(0),
                    property,
                },
            ));
        assert_eq!(
            super::direct_borrow_home_place(&shared),
            Some(super::BorrowHomePlace::Property {
                object: mir::LocalId(0),
                property
            })
        );
        let borrowed_call = mir::Rvalue::Mixed(mir::MixedExpression::Call {
            function: mir::FunctionId(0),
            args: vec![],
            return_borrow: Some(mir::ReturnBorrow {
                kind: crate::types::ReturnBorrowKind::Value,
                source: mir::BorrowSource::Parameter(0),
                writable: false,
            }),
        });
        assert!(borrowed_call.borrows_move_value());
        assert!(!super::has_addressable_borrow_home(
            &borrowed_call,
            &function
        ));
        assert!(!super::needs_owned_borrow_home(&borrowed_call, &function));
        let owned_call = mir::Rvalue::Mixed(mir::MixedExpression::Call {
            function: mir::FunctionId(0),
            args: vec![],
            return_borrow: None,
        });
        assert!(super::needs_owned_borrow_home(&owned_call, &function));
        assert!(!super::needs_owned_borrow_home(
            &mir::Rvalue::Mixed(mir::MixedExpression::Null),
            &function
        ));
        assert!(!super::needs_owned_borrow_home(&narrowed, &function));
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
                kind: crate::types::ReturnBorrowKind::Retained,
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

        // Every borrowed Move carrier returns an exact place, including a
        // stored closure. This differs from the owned closure above whose
        // captures retain a source. Keep all call plans aligned.
        for ty in [
            mir::Type::Class(crate::class_layout::ClassId(0)),
            mir::Type::NullableClass(crate::class_layout::ClassId(0)),
            mir::Type::Interface(mir::InterfaceTypeId(0)),
            mir::Type::Mixed,
            mir::Type::Collection(mir::CollectionTypeId(0)),
            mir::Type::Function(mir::FunctionTypeId(1)),
            mir::Type::NullableFunction(mir::FunctionTypeId(1)),
        ] {
            let borrowed = mir::FunctionType {
                return_type: mir::ReturnType::Value(ty),
                return_borrow: Some(mir::ReturnBorrow {
                    kind: crate::types::ReturnBorrowKind::Value,
                    source: mir::BorrowSource::Parameter(0),
                    writable: false,
                }),
                ..function.clone()
            };
            let plan = NativeCallableSignaturePlan::indirect(&borrowed);
            assert_eq!(
                plan.hidden_inputs,
                vec![
                    NativeCallableHiddenInput::CurrentFrame,
                    NativeCallableHiddenInput::ResultOut,
                    NativeCallableHiddenInput::ErrorOut,
                    NativeCallableHiddenInput::BorrowHome,
                    NativeCallableHiddenInput::ResultBorrowHomeOut,
                    NativeCallableHiddenInput::Environment,
                ]
            );
            let entry = NativeCallableSignaturePlan::interface_entry(&borrowed);
            assert_eq!(entry.hidden_inputs, plan.hidden_inputs[..5]);
            let owning = mir::FunctionType {
                return_borrow: None,
                ..borrowed
            };
            assert_eq!(
                NativeCallableSignaturePlan::indirect(&owning)
                    .index_of(NativeCallableHiddenInput::ResultBorrowHomeOut),
                None
            );
        }

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
    fn closure_return_plans_separate_carrier_borrows_from_retained_sources() {
        use crate::types::ReturnBorrowKind;

        for ty in [
            mir::Type::Function(mir::FunctionTypeId(0)),
            mir::Type::NullableFunction(mir::FunctionTypeId(0)),
        ] {
            for checked in [false, true] {
                for kind in [ReturnBorrowKind::Value, ReturnBorrowKind::Retained] {
                    let return_borrow = Some(mir::ReturnBorrow {
                        kind,
                        source: mir::BorrowSource::Parameter(0),
                        writable: false,
                    });
                    let return_type = mir::ReturnType::Value(ty);
                    assert_eq!(
                        super::returns_borrowed_value(return_type, return_borrow),
                        kind == ReturnBorrowKind::Value
                    );
                    for environment in [false, true] {
                        let plan = NativeCallableSignaturePlan::new(
                            return_type,
                            checked,
                            return_borrow,
                            environment,
                        );
                        assert!(plan
                            .index_of(NativeCallableHiddenInput::BorrowHome)
                            .is_some());
                        assert_eq!(
                            plan.index_of(NativeCallableHiddenInput::ResultBorrowHomeOut)
                                .is_some(),
                            kind == ReturnBorrowKind::Value
                        );
                        assert_eq!(
                            plan.index_of(NativeCallableHiddenInput::ResultOut)
                                .is_some(),
                            checked
                        );
                        assert_eq!(
                            plan.index_of(NativeCallableHiddenInput::Environment)
                                .is_some(),
                            environment
                        );
                    }
                }
            }
        }
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
            for (field, native_field) in logical.fields.iter().zip(&native.fields) {
                if field.storage != mir::ClosureEnvironmentStorage::Owned {
                    assert_eq!(native_field.layout.size, 8 * super::BORROW_HOME_WORDS);
                    assert_eq!(native_field.layout.align, 8);
                    assert_eq!(native_field.live_bit, None);
                }
            }
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
