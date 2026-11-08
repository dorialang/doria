//! Backend-neutral layout authority for the compiler-private native closure ABI.

use crate::backend::BackendError;
use crate::mir;

pub const CARRIER_WORDS: u32 = 2;
pub const DESCRIPTOR_WORDS: u32 = 2;

/// A borrow home retains the original storage address, not an intermediate
/// receiver's stack slot. Representation projections also retain the storage's type:
/// an interface vtable is not an open-class descriptor, even though both
/// carriers have two words. Call arguments point to a transient descriptor;
/// borrowed environment fields copy its words rather than retaining that pointer.
pub const BORROW_HOME_WORDS: u32 = 2;
pub const BORROW_HOME_NOMINAL_ID_SHIFT: u32 = 4;
pub const BORROW_HOME_CLASS_KIND: u64 = 1;
pub const BORROW_HOME_INTERFACE_KIND: u64 = 2;
pub const BORROW_HOME_BOXED_CLASS_KIND: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowHomeStorage {
    Value(mir::Type),
    // Mixed class payloads retain a dynamic descriptor even for closed classes.
    BoxedClass(crate::class_layout::ClassId),
}

impl BorrowHomeStorage {
    pub fn ty(self) -> mir::Type {
        match self {
            Self::Value(ty) => ty,
            Self::BoxedClass(class) => mir::Type::Class(class),
        }
    }

    pub fn key(self) -> u64 {
        match self {
            Self::Value(ty) => borrow_home_type_key(ty),
            Self::BoxedClass(class) => {
                ((class.0 as u64) << BORROW_HOME_NOMINAL_ID_SHIFT) | BORROW_HOME_BOXED_CLASS_KIND
            }
        }
    }

    pub fn is_boxed_class(self) -> bool {
        matches!(self, Self::BoxedClass(_))
    }
}

pub fn borrow_home_type_key(ty: mir::Type) -> u64 {
    match ty {
        mir::Type::Class(id) | mir::Type::NullableClass(id) => {
            ((id.0 as u64) << BORROW_HOME_NOMINAL_ID_SHIFT) | BORROW_HOME_CLASS_KIND
        }
        mir::Type::Interface(id) | mir::Type::NullableInterface(id) => {
            ((id.0 as u64) << BORROW_HOME_NOMINAL_ID_SHIFT) | BORROW_HOME_INTERFACE_KIND
        }
        mir::Type::Collection(id) | mir::Type::NullableCollection(id) => {
            ((id.0 as u64) << BORROW_HOME_NOMINAL_ID_SHIFT) | 3
        }
        // Inline nullable carriers have a presence prefix. Their original
        // layout must stay distinct from the corresponding nonnullable slot.
        mir::Type::PayloadEnum(ty) => ((ty.id.0 as u64) << BORROW_HOME_NOMINAL_ID_SHIFT) | 4,
        mir::Type::NullablePayloadEnum(ty) => {
            ((ty.id.0 as u64) << BORROW_HOME_NOMINAL_ID_SHIFT) | 5
        }
        mir::Type::Scalar(ty) => (scalar_home_identity(ty) << BORROW_HOME_NOMINAL_ID_SHIFT) | 6,
        mir::Type::NullableScalar(ty) => {
            (scalar_home_identity(ty) << BORROW_HOME_NOMINAL_ID_SHIFT) | 7
        }
        mir::Type::String => 8,
        mir::Type::NullableString => 9,
        mir::Type::ByteRegion(kind) => {
            (u64::from(kind.is_writable()) << BORROW_HOME_NOMINAL_ID_SHIFT) | 11
        }
        mir::Type::NullableByteRegion(kind) => {
            (u64::from(kind.is_writable()) << BORROW_HOME_NOMINAL_ID_SHIFT) | 12
        }
        _ => 0,
    }
}

fn scalar_home_identity(ty: mir::ScalarType) -> u64 {
    match ty {
        mir::ScalarType::Integer(ty) => (ty as u64) << 2,
        mir::ScalarType::Float(ty) => ((ty as u64) << 2) | 1,
        mir::ScalarType::Bool => 2,
        mir::ScalarType::Enum(id) => ((id.0 as u64 + 1) << 2) | 2,
        mir::ScalarType::Inline(ty) => (u64::from(ty.type_id()) << 2) | 3,
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

pub fn local_reads_borrow_home(function: &mir::Function, local: &mir::Local) -> bool {
    !local.owned
        && (local.ty.has_move_ownership() || function.is_writable_borrowed_place(local.id))
        && (!function.params.contains(&local.id) || function.is_writable_borrowed_place(local.id))
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

pub fn has_nominal_borrow_home(ty: mir::Type) -> bool {
    ty.interface_implementing_type().is_some()
        || matches!(
            ty,
            mir::Type::Interface(_) | mir::Type::NullableInterface(_)
        )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowHomeProjection {
    Direct,
    FileReadinessSource(crate::class_layout::ClassId),
    StandardReadinessSource(crate::class_layout::ClassId, mir::StandardIoStream),
    MixedPayload(mir::Type),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowHomePlace {
    Static(mir::StaticId),
    Local {
        local: mir::LocalId,
        projection: BorrowHomeProjection,
    },
    Property {
        object: mir::LocalId,
        property: crate::class_layout::PropertyId,
    },
}

/// Exact places shared by every native value family. Mixed projections retain
/// the box's actual payload storage; a call result still needs explicit provenance.
pub fn direct_borrow_home_place(value: &mir::Rvalue) -> Option<BorrowHomePlace> {
    match value {
        mir::Rvalue::Class(mir::ClassExpression::Upcast { value, .. }) => {
            return direct_borrow_home_place(&mir::Rvalue::Class((**value).clone()));
        }
        mir::Rvalue::NullableClass(mir::NullableClassExpression::Upcast { value, .. }) => {
            return direct_borrow_home_place(&mir::Rvalue::NullableClass((**value).clone()));
        }
        _ => {}
    }
    use BorrowHomePlace as P;
    use BorrowHomeProjection as Q;
    let projected_local = match value {
        mir::Rvalue::Function(mir::FunctionExpression::MixedPayload {
            mixed,
            function_type,
            transfer: false,
        }) => Some((*mixed, Q::MixedPayload(mir::Type::Function(*function_type)))),
        mir::Rvalue::Class(mir::ClassExpression::MixedPayload {
            mixed,
            class,
            transfer: false,
        }) => Some((*mixed, Q::MixedPayload(mir::Type::Class(*class)))),
        mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Use {
            ty,
            place: mir::PayloadEnumPlace::MixedPayload { mixed },
            mode: mir::PayloadEnumUseMode::Borrow,
        }) => Some((*mixed, Q::MixedPayload(mir::Type::PayloadEnum(*ty)))),
        mir::Rvalue::Class(mir::ClassExpression::FileReadinessSource { class, file }) => {
            Some((*file, Q::FileReadinessSource(*class)))
        }
        mir::Rvalue::Class(mir::ClassExpression::StandardReadinessSource {
            class,
            stream,
            borrowed_from,
        }) => Some((*borrowed_from, Q::StandardReadinessSource(*class, *stream))),
        mir::Rvalue::ByteRegion(mir::ByteRegionExpression::AssumePresent { value, .. }) => {
            match value.as_ref() {
                mir::NullableByteRegionExpression::Local { local, .. } => Some((*local, Q::Direct)),
                _ => None,
            }
        }
        mir::Rvalue::String(mir::StringExpression::NullableLocalAssumeNonNull(local)) => {
            Some((*local, Q::Direct))
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
            })
            | mir::ValueExpression::Inline(mir::InlineValueExpression::Use {
                operand: mir::Operand::NullablePayload(local),
                ..
            }),
        ) => Some((*local, Q::Direct)),
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
        mir::Rvalue::String(mir::StringExpression::Static(id))
        | mir::Rvalue::NullableString(mir::NullableStringExpression::Static(id))
        | mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Static { id, .. })
        | mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Use {
            place: mir::PayloadEnumPlace::Static(id),
            mode: mir::PayloadEnumUseMode::Borrow,
            ..
        })
        | mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Use {
            place: mir::PayloadEnumPlace::Static(id),
            mode: mir::PayloadEnumUseMode::Borrow,
            ..
        }) => Some(P::Static(*id)),
        mir::Rvalue::Value(value) => {
            let operand = match value {
                mir::ValueExpression::Integer(mir::IntegerExpression::Use { operand, .. })
                | mir::ValueExpression::Float(mir::FloatExpression::Use { operand, .. })
                | mir::ValueExpression::Bool(mir::BoolExpression::Use { operand })
                | mir::ValueExpression::Enum(mir::EnumExpression::Use { operand, .. })
                | mir::ValueExpression::Inline(mir::InlineValueExpression::Use {
                    operand, ..
                }) => operand,
                _ => return None,
            };
            match operand {
                mir::Operand::Static(id) => Some(P::Static(*id)),
                mir::Operand::Property { object, property } => Some(P::Property {
                    object: *object,
                    property: *property,
                }),
                _ => None,
            }
        }
        mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Value(value)) => {
            direct_borrow_home_place(&mir::Rvalue::Value(value.clone()))
        }
        mir::Rvalue::NullableString(mir::NullableStringExpression::String(value)) => {
            direct_borrow_home_place(&mir::Rvalue::String(value.clone()))
        }
        mir::Rvalue::Class(mir::ClassExpression::Property {
            object, property, ..
        })
        | mir::Rvalue::NullableClass(mir::NullableClassExpression::Property {
            object,
            property,
            ..
        })
        | mir::Rvalue::Collection(mir::CollectionExpression::Property {
            object, property, ..
        })
        | mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Property {
            object,
            property,
            ..
        })
        | mir::Rvalue::String(mir::StringExpression::Property { object, property })
        | mir::Rvalue::NullableString(mir::NullableStringExpression::Property {
            object,
            property,
        })
        | mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Property {
            object,
            property,
            ..
        })
        | mir::Rvalue::Mixed(mir::MixedExpression::Property { object, property })
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
        mir::Rvalue::NullableByteRegion(mir::NullableByteRegionExpression::Present(value)) => {
            direct_borrow_home_place(&mir::Rvalue::ByteRegion(value.clone()))
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
        function
            .locals
            .get(local.0)
            .is_some_and(|local| has_nominal_borrow_home(local.ty))
    }
    fn class(value: &mir::ClassExpression, function: &mir::Function) -> bool {
        match value {
            mir::ClassExpression::Local { .. }
            | mir::ClassExpression::NullableLocalAssumeNonNull { .. }
            | mir::ClassExpression::Property { .. }
            | mir::ClassExpression::InterfaceReceiver { .. } => true,
            mir::ClassExpression::InterfacePayload { local, .. } => nominal_local(function, *local),
            mir::ClassExpression::MixedPayload {
                transfer: false, ..
            } => true,
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
            mir::InterfaceValue::NarrowedLocal {
                local, transfer, ..
            } => {
                nominal_local(function, *local)
                    || (!transfer
                        && function.locals.get(local.0).is_some_and(|local| {
                            matches!(local.ty, mir::Type::Mixed | mir::Type::NullableMixed)
                        }))
            }
            mir::InterfaceValue::Upcast { source, .. } => interface(&source.value, function),
            mir::InterfaceValue::FromClass { object, .. } => class(object, function),
            mir::InterfaceValue::FromNullableClass { object, .. } => {
                nullable_class(object, function)
            }
            mir::InterfaceValue::FromValue { value, .. } => {
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
            | mir::CollectionExpression::Index {
                transfer: false, ..
            }
            | mir::CollectionExpression::InterfaceReceiver { .. },
        )
        | mir::Rvalue::NullableCollection(
            mir::NullableCollectionExpression::Local { .. }
            | mir::NullableCollectionExpression::Index {
                transfer: false, ..
            }
            | mir::NullableCollectionExpression::Property { .. },
        ) => true,
        mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Collection(value)) => {
            has_addressable_borrow_home(&mir::Rvalue::Collection(value.clone()), function)
        }
        mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::CollectionGet {
            access,
            ..
        })
        | mir::Rvalue::NullableMixed(mir::NullableMixedExpression::CollectionGet {
            access, ..
        }) => !access.removes_element(),
        _ => false,
    }
}

/// Nominal views and nullable enum views can change representation. Other
/// homes point directly at the checked value, including narrowed Copy words.
pub fn borrow_home_projection_types(program: &mir::Program, target: mir::Type) -> Vec<mir::Type> {
    match target {
        mir::Type::Scalar(ty) | mir::Type::NullableScalar(ty) => {
            return vec![mir::Type::Scalar(ty), mir::Type::NullableScalar(ty)];
        }
        mir::Type::String | mir::Type::NullableString => {
            return vec![mir::Type::String, mir::Type::NullableString];
        }
        mir::Type::ByteRegion(kind) | mir::Type::NullableByteRegion(kind) => {
            return vec![
                mir::Type::ByteRegion(kind),
                mir::Type::NullableByteRegion(kind),
            ];
        }
        _ => {}
    }
    if let mir::Type::PayloadEnum(ty) | mir::Type::NullablePayloadEnum(ty) = target {
        return vec![
            mir::Type::PayloadEnum(ty),
            mir::Type::NullablePayloadEnum(ty),
        ];
    }
    if borrow_home_type_key(target) == 0 {
        return Vec::new();
    }
    program.classes.iter().flat_map(|class| match class.inline_copy_type() {
            Some(ty) => vec![mir::Type::Scalar(mir::ScalarType::Inline(ty)), mir::Type::NullableScalar(mir::ScalarType::Inline(ty))],
            None => vec![mir::Type::Class(class.id)],
        })
        .chain(program.interface_types.iter().map(|interface| mir::Type::Interface(interface.id)))
        .chain(program.collection_types.iter().map(|collection| mir::Type::Collection(collection.id)))
        .filter(|source| match (*source, target) {
            (mir::Type::Scalar(mir::ScalarType::Inline(ty)) | mir::Type::NullableScalar(mir::ScalarType::Inline(ty)), mir::Type::Interface(interface) | mir::Type::NullableInterface(interface)) => ty.nominal_class().is_some_and(|class| program.interface_vtable(mir::ImplementingType::Class(class), interface).is_some()),
            (mir::Type::Scalar(_) | mir::Type::NullableScalar(_), _) => false,
            (mir::Type::Collection(source), mir::Type::Interface(target) | mir::Type::NullableInterface(target)) => program.interface_vtable(mir::ImplementingType::Collection(source), target).is_some(),
            (mir::Type::Interface(source), mir::Type::Collection(target) | mir::Type::NullableCollection(target)) => program.interface_vtable(mir::ImplementingType::Collection(target), source).is_some(),
            (mir::Type::Collection(source), mir::Type::Collection(target) | mir::Type::NullableCollection(target)) => source == target,
            (mir::Type::Collection(_), mir::Type::Class(_) | mir::Type::NullableClass(_))
            | (mir::Type::Class(_), mir::Type::Collection(_) | mir::Type::NullableCollection(_)) => false,
            _ => true,
        })
        .collect()
}

pub fn borrow_home_projection_storage(
    program: &mir::Program,
    target: mir::Type,
) -> Vec<BorrowHomeStorage> {
    borrow_home_projection_types(program, target)
        .into_iter()
        .flat_map(|ty| {
            let boxed = match ty {
                mir::Type::Class(class) => Some(BorrowHomeStorage::BoxedClass(class)),
                _ => None,
            };
            std::iter::once(BorrowHomeStorage::Value(ty)).chain(boxed)
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
    ParameterBorrowHome(usize),
    Environment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCallableSignaturePlan {
    pub hidden_inputs: Vec<NativeCallableHiddenInput>,
    pub checked: bool,
}

pub fn parameter_uses_borrow_home(
    ty: mir::Type,
    mode: mir::FunctionParameterMode,
    writable_copy: bool,
) -> bool {
    mode != mir::FunctionParameterMode::Take
        && (ty.has_move_ownership()
            || writable_copy && mode == mir::FunctionParameterMode::Writable)
}

impl NativeCallableSignaturePlan {
    pub fn direct(function: &mir::Function) -> Self {
        let plan = Self::new(
            function.return_type,
            function.has_checked_transport(),
            function.return_borrow,
            false,
        );
        let parameters = function
            .params
            .iter()
            .enumerate()
            .skip(usize::from(function.closure.is_some()))
            .map(|(index, local)| mir::FunctionParameter {
                ty: function.locals[local.0].ty,
                mode: function.parameter_modes[index],
            });
        plan.with_parameter_homes(
            parameters,
            function.return_borrow.map(|borrow| {
                return_borrow_argument_index(borrow, function.receiver_mode.is_some())
            }),
            function.closure.is_some(),
            |index| {
                let local = function.params[index + usize::from(function.closure.is_some())];
                function.retains_writable_input(local)
            },
        )
    }

    pub fn indirect(function: &mir::FunctionType) -> Self {
        Self::new(
            function.return_type,
            function.has_checked_transport(),
            function.return_borrow,
            true,
        )
        .with_parameter_homes(
            function.parameters.iter().cloned(),
            function
                .return_borrow
                .map(|borrow| return_borrow_argument_index(borrow, false)),
            true,
            |_| false,
        )
    }

    pub fn interface_entry(function: &mir::FunctionType) -> Self {
        Self::new(
            function.return_type,
            function.has_checked_transport(),
            function.return_borrow,
            false,
        )
        .with_parameter_homes(
            function.parameters.iter().cloned(),
            function
                .return_borrow
                .map(|borrow| return_borrow_argument_index(borrow, false)),
            false,
            |_| false,
        )
    }

    fn with_parameter_homes(
        mut self,
        parameters: impl Iterator<Item = mir::FunctionParameter>,
        returned_source: Option<usize>,
        writable_copy: bool,
        retained_place: impl Fn(usize) -> bool,
    ) -> Self {
        // A borrowed input may be retained in caller-owned storage without
        // being the return value. Forward its original place through every
        // call shape rather than exposing a callee's transient parameter slot.
        let homes = parameters
            .enumerate()
            .filter_map(|(index, parameter)| {
                (parameter_uses_borrow_home(parameter.ty, parameter.mode, writable_copy)
                    || retained_place(index))
                .then_some(NativeCallableHiddenInput::ParameterBorrowHome(index))
            })
            .collect::<Vec<_>>();
        if returned_source.is_some_and(|index| {
            homes.contains(&NativeCallableHiddenInput::ParameterBorrowHome(index))
        }) {
            self.hidden_inputs
                .retain(|input| *input != NativeCallableHiddenInput::BorrowHome);
        }
        let environment = self
            .index_of(NativeCallableHiddenInput::Environment)
            .unwrap_or(self.hidden_inputs.len());
        self.hidden_inputs.splice(environment..environment, homes);
        self
    }

    pub fn parameter_homes(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.hidden_inputs
            .iter()
            .enumerate()
            .filter_map(|(abi, input)| match input {
                NativeCallableHiddenInput::ParameterBorrowHome(parameter) => {
                    Some((abi, *parameter))
                }
                _ => None,
            })
    }

    fn new(
        return_type: mir::ReturnType,
        checked: bool,
        return_borrow: Option<mir::ReturnBorrow>,
        environment: bool,
    ) -> Self {
        let mut hidden_inputs = vec![NativeCallableHiddenInput::CurrentFrame];
        if returns_through_result_slot(return_type, checked) {
            hidden_inputs.push(NativeCallableHiddenInput::ResultOut);
        }
        if checked {
            hidden_inputs.push(NativeCallableHiddenInput::ErrorOut);
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

    /// The plan of a direct call that receives `physical_return`, the return
    /// carrier of the entry the call actually reaches. A virtual call reaches
    /// its family's return adapter, whose carrier may differ from the
    /// statically named method's. The result slot follows the carrier the
    /// entry returns; every other hidden input keeps the named method's
    /// contract, exactly as the call's argument lowering does.
    pub fn direct_for_return(function: &mir::Function, physical_return: mir::ReturnType) -> Self {
        let mut plan = Self::direct(function);
        let result = plan.index_of(NativeCallableHiddenInput::ResultOut);
        let after_frame = plan
            .index_of(NativeCallableHiddenInput::CurrentFrame)
            .map_or(0, |index| index + 1);
        match (
            returns_through_result_slot(physical_return, plan.checked),
            result,
        ) {
            (true, None) => plan
                .hidden_inputs
                .insert(after_frame, NativeCallableHiddenInput::ResultOut),
            (false, Some(index)) => {
                plan.hidden_inputs.remove(index);
            }
            _ => {}
        }
        plan
    }

    /// Whether the callee hands its value back in return registers. Otherwise
    /// it writes the value through the `ResultOut` slot, and a checked callee
    /// returns only its status byte.
    pub fn returns_value_directly(&self) -> bool {
        !self.checked
            && self
                .index_of(NativeCallableHiddenInput::ResultOut)
                .is_none()
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

/// Register slots a value occupies when the private native ABI passes or
/// returns it directly. `float` counts the slots a backend may carry in a
/// floating-point register; a 128-bit inline scalar takes two slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectValueSlots {
    pub total: u32,
    pub float: u32,
}

impl DirectValueSlots {
    const fn integer(total: u32) -> Self {
        Self { total, float: 0 }
    }

    const fn plus(self, other: Self) -> Self {
        Self {
            total: self.total + other.total,
            float: self.float + other.float,
        }
    }

    pub const fn fits(self, budget: Self) -> bool {
        self.total <= budget.total && self.float <= budget.float
    }
}

/// The largest value a callable returns in registers: two slots, at most one
/// of them floating-point. This is the smallest return-register budget of the
/// supported native targets. x86-64 System V and Windows x64 return two
/// integer registers (`rax`, `rdx`), and Windows x64 one floating-point
/// register (`xmm0`). It is also C's threshold: System V and AAPCS64 return
/// an aggregate of at most 16 bytes in registers and a larger one through
/// caller memory. One budget on every target keeps the lowering identical
/// across hosts, so a run on any host exercises the transport every other host
/// uses.
pub const DIRECT_RETURN_SLOTS: DirectValueSlots = DirectValueSlots { total: 2, float: 1 };

/// The direct register shape of `ty`, independent of the program. A class
/// reports its widest carrier (an open class's object and descriptor words).
/// Payload enums are inline aggregates and travel by address; their slot
/// count is that address.
pub const fn direct_value_slots(ty: mir::Type) -> DirectValueSlots {
    match ty {
        mir::Type::Scalar(scalar) => direct_scalar_slots(scalar),
        mir::Type::NullableScalar(scalar) => {
            DirectValueSlots::integer(1).plus(direct_scalar_slots(scalar))
        }
        mir::Type::ByteRegion(_)
        | mir::Type::NullableString
        | mir::Type::Interface(_)
        | mir::Type::NullableInterface(_)
        | mir::Type::Function(_)
        | mir::Type::NullableFunction(_)
        | mir::Type::Class(_)
        | mir::Type::NullableClass(_) => DirectValueSlots::integer(2),
        mir::Type::NullableByteRegion(_) => DirectValueSlots::integer(3),
        mir::Type::SharedReference(_)
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
        | mir::Type::NullableWritableSharedReferenceAccess(_) => {
            if ty.shared_interface().is_some() {
                DirectValueSlots::integer(2)
            } else {
                DirectValueSlots::integer(1)
            }
        }
        mir::Type::String
        | mir::Type::Mixed
        | mir::Type::NullableMixed
        | mir::Type::Collection(_)
        | mir::Type::NullableCollection(_)
        | mir::Type::PayloadEnum(_)
        | mir::Type::NullablePayloadEnum(_)
        | mir::Type::ClosureEnvironment(_) => DirectValueSlots::integer(1),
    }
}

const fn direct_scalar_slots(scalar: mir::ScalarType) -> DirectValueSlots {
    match scalar {
        mir::ScalarType::Float(_) => DirectValueSlots { total: 1, float: 1 },
        mir::ScalarType::Inline(inline) => {
            DirectValueSlots::integer(inline.storage_bytes().div_ceil(8))
        }
        mir::ScalarType::Integer(_) | mir::ScalarType::Bool | mir::ScalarType::Enum(_) => {
            DirectValueSlots::integer(1)
        }
    }
}

/// Whether a callable's value travels through the caller's result slot
/// (`NativeCallableHiddenInput::ResultOut`) instead of return registers. A
/// checked callable returns its status in the register and its value through
/// the slot. Otherwise a payload enum, an inline aggregate, always uses the
/// slot, and any other value uses it when it exceeds [`DIRECT_RETURN_SLOTS`].
/// Every definition and call shape in both native backends derives its
/// signature from this one rule.
pub const fn returns_through_result_slot(return_type: mir::ReturnType, checked: bool) -> bool {
    match return_type {
        mir::ReturnType::Void => false,
        mir::ReturnType::Value(_) if checked => true,
        mir::ReturnType::Value(mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)) => {
            true
        }
        mir::ReturnType::Value(ty) => !direct_value_slots(ty).fits(DIRECT_RETURN_SLOTS),
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
        mir::Type::Scalar(mir::ScalarType::Inline(ty)) => NativeLayout {
            size: ty.storage_bytes(),
            align: ty.storage_bytes(),
        },
        mir::Type::NullableScalar(scalar) => {
            let payload = type_layout(mir::Type::Scalar(scalar), pointer_size);
            let align = if payload.align > pointer_size {
                payload.align
            } else {
                pointer_size
            };
            NativeLayout {
                size: align * 2,
                align,
            }
        }
        mir::Type::NullableString
        | mir::Type::Interface(_)
        | mir::Type::NullableInterface(_)
        | mir::Type::Function(_)
        | mir::Type::NullableFunction(_)
        | mir::Type::ByteRegion(_) => NativeLayout {
            size: pointer_size * 2,
            align: pointer_size,
        },
        mir::Type::NullableByteRegion(_) => NativeLayout {
            size: pointer_size * 3,
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
    matches!(ty, mir::Type::String | mir::Type::NullableString)
        || (ty.requires_owned_cleanup()
            && !matches!(
                ty,
                mir::Type::ByteRegion(_) | mir::Type::NullableByteRegion(_)
            ))
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
    #[test]
    fn parameter_homes_distinguish_copy_values_writable_callbacks_and_move_loans() {
        use super::parameter_uses_borrow_home;
        use crate::mir::{FunctionParameterMode as Mode, ScalarType, Type};
        for ty in [
            Type::Scalar(ScalarType::Bool),
            Type::NullableScalar(ScalarType::Bool),
            Type::String,
        ] {
            assert!(!parameter_uses_borrow_home(ty, Mode::Readonly, false));
            assert!(!parameter_uses_borrow_home(ty, Mode::Writable, false));
            assert!(!parameter_uses_borrow_home(ty, Mode::Readonly, true));
            assert!(parameter_uses_borrow_home(ty, Mode::Writable, true));
            assert!(!parameter_uses_borrow_home(ty, Mode::Take, true));
        }
        for ty in [Type::Mixed, Type::NullableMixed] {
            for callback in [false, true] {
                assert!(parameter_uses_borrow_home(ty, Mode::Readonly, callback));
                assert!(parameter_uses_borrow_home(ty, Mode::Writable, callback));
                assert!(!parameter_uses_borrow_home(ty, Mode::Take, callback));
            }
        }
    }

    use super::{
        carrier_layout, descriptor_layout, environment_layout, NativeCallableHiddenInput,
        NativeCallableSignaturePlan,
    };
    use crate::mir;

    #[test]
    fn repeatable_owned_captures_preserve_invocation_access_without_transfer() {
        use mir::{ClosureEnvironmentStorage as Storage, FunctionInvocationMode as Invocation};
        assert_eq!(
            Storage::Owned.invocation_binding(Invocation::Readonly),
            Storage::ReadonlyBorrow
        );
        assert_eq!(
            Storage::Owned.invocation_binding(Invocation::Writable),
            Storage::WritableBorrow
        );
        assert_eq!(
            Storage::Owned.invocation_binding(Invocation::Once),
            Storage::Owned
        );
        for storage in [Storage::ReadonlyBorrow, Storage::WritableBorrow] {
            for invocation in [Invocation::Readonly, Invocation::Writable, Invocation::Once] {
                assert_eq!(storage.invocation_binding(invocation), storage);
            }
        }
    }

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
            super::BorrowHomeStorage::BoxedClass(class).key(),
            super::BorrowHomeStorage::BoxedClass(crate::class_layout::ClassId(4)).key(),
            key(mir::Type::Interface(interface)),
            key(mir::Type::Collection(collection)),
            key(mir::Type::Class(crate::class_layout::ClassId(4))),
            key(mir::Type::PayloadEnum(payload_enum)),
            key(mir::Type::NullablePayloadEnum(payload_enum)),
            key(mir::Type::String),
            key(mir::Type::NullableString),
            key(mir::Type::ByteRegion(
                crate::types::ByteRegionKind::Readonly,
            )),
            key(mir::Type::NullableByteRegion(
                crate::types::ByteRegionKind::Readonly,
            )),
            key(mir::Type::ByteRegion(
                crate::types::ByteRegionKind::Writable,
            )),
            key(mir::Type::NullableByteRegion(
                crate::types::ByteRegionKind::Writable,
            )),
            key(mir::Type::Scalar(mir::ScalarType::Bool)),
            key(mir::Type::NullableScalar(mir::ScalarType::Bool)),
            key(mir::Type::Scalar(mir::ScalarType::Integer(
                crate::numeric::IntegerType::Int64,
            ))),
            key(mir::Type::NullableScalar(mir::ScalarType::Integer(
                crate::numeric::IntegerType::Int64,
            ))),
            key(mir::Type::Scalar(mir::ScalarType::Float(
                crate::numeric::FloatType::Float64,
            ))),
            key(mir::Type::NullableScalar(mir::ScalarType::Float(
                crate::numeric::FloatType::Float64,
            ))),
            key(mir::Type::Scalar(mir::ScalarType::Enum(
                crate::enums::EnumId(0),
            ))),
        ];
        for (index, key) in keys.iter().enumerate() {
            assert_ne!(*key, 0);
            assert!(!keys[index + 1..].contains(key));
        }
    }

    #[test]
    fn boxed_class_homes_preserve_the_static_domain_and_dynamic_carrier() {
        use super::BorrowHomeStorage;
        let program = crate::lower_source_to_mir(
            "boxed-home.doria",
            "open class Base {} class Child extends Base {} function main(): void {}",
        )
        .expect("classes should lower");
        let target = mir::Type::Class(program.classes[0].id);
        let storage = super::borrow_home_projection_storage(&program, target);
        for class in &program.classes {
            let ordinary = BorrowHomeStorage::Value(mir::Type::Class(class.id));
            let boxed = BorrowHomeStorage::BoxedClass(class.id);
            assert_eq!(ordinary.ty(), boxed.ty());
            assert_ne!(ordinary.key(), boxed.key());
            assert!(storage.contains(&ordinary));
            assert!(storage.contains(&boxed));
        }
        assert_eq!(
            storage.len(),
            super::borrow_home_projection_types(&program, target).len() + program.classes.len()
        );
        let keys = storage
            .iter()
            .map(|storage| storage.key())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(keys.len(), storage.len());
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
        let readiness = mir::Rvalue::Class(mir::ClassExpression::FileReadinessSource {
            class: property.class,
            file: mir::LocalId(0),
        });
        assert_eq!(
            super::direct_borrow_home_place(&readiness),
            Some(super::BorrowHomePlace::Local {
                local: mir::LocalId(0),
                projection: super::BorrowHomeProjection::FileReadinessSource(property.class),
            })
        );
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
            span: function.source_span,
            function: mir::FunctionId(0),
            args: vec![],
            return_borrow: Some(mir::ReturnBorrow {
                kind: crate::types::ReturnBorrowKind::Value,
                lifetime: mir::RetainedLifetime::LastUse,
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
            span: function.source_span,
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
        for transfer in [false, true] {
            for value in [
                mir::Rvalue::Function(mir::FunctionExpression::MixedPayload {
                    mixed: mir::LocalId(0),
                    function_type: mir::FunctionTypeId(0),
                    transfer,
                }),
                mir::Rvalue::Class(mir::ClassExpression::MixedPayload {
                    mixed: mir::LocalId(0),
                    class: crate::class_layout::ClassId(0),
                    transfer,
                }),
                mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Use {
                    ty: enum_ty,
                    place: mir::PayloadEnumPlace::MixedPayload {
                        mixed: mir::LocalId(0),
                    },
                    mode: if transfer {
                        mir::PayloadEnumUseMode::Move
                    } else {
                        mir::PayloadEnumUseMode::Borrow
                    },
                }),
            ] {
                assert_eq!(value.borrows_move_value(), !transfer);
                assert_eq!(
                    super::has_addressable_borrow_home(&value, &function),
                    !transfer
                );
                assert_eq!(super::needs_owned_borrow_home(&value, &function), transfer);
            }
        }
        for nullable in [false, true] {
            for transfer in [false, true] {
                let index = Box::new(mir::Rvalue::Value(mir::ValueExpression::Integer(
                    mir::IntegerExpression::constant(
                        crate::numeric::IntegerValue::from_i128(
                            crate::numeric::IntegerType::Int64,
                            0,
                        )
                        .unwrap(),
                    ),
                )));
                let value = if nullable {
                    mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Index {
                        collection: mir::CollectionTypeId(0),
                        source: mir::LocalId(0),
                        index,
                        positional: false,
                        index_span: crate::source::Span::default(),
                        transfer,
                    })
                } else {
                    mir::Rvalue::Collection(mir::CollectionExpression::Index {
                        collection: mir::CollectionTypeId(0),
                        source: mir::LocalId(0),
                        index,
                        positional: false,
                        index_span: crate::source::Span::default(),
                        transfer,
                    })
                };
                assert_eq!(value.borrows_move_value(), !transfer);
                assert_eq!(
                    super::has_addressable_borrow_home(&value, &function),
                    !transfer,
                );
                assert_eq!(super::needs_owned_borrow_home(&value, &function), transfer,);
            }
        }
    }

    #[test]
    fn nominal_interface_projections_preserve_concrete_and_nullable_inline_homes() {
        let program = crate::lower_source_to_mir(
            "nominal-home.doria",
            r#"
use Doria\Std\Io\{StandardStreams, ReadinessStream};
function observe(ReadinessStream $stream): void {}
function main(): void { let $input = StandardStreams::input; observe($input); }
"#,
        )
        .expect("empty entry should lower");
        let mut function = program.functions[program.entry.0].clone();
        let class = crate::class_layout::ClassId(0);
        let inline = crate::inline_value::InlineValueType::StandardOutput(class);
        let projected = mir::Rvalue::Interface(mir::InterfaceExpression {
            interface: mir::InterfaceTypeId::ERROR,
            value: mir::InterfaceValue::NarrowedLocal {
                local: mir::LocalId(0),
                interface: mir::InterfaceTypeId::ERROR,
                transfer: false,
            },
        });
        for ty in [
            mir::Type::Class(class),
            mir::Type::NullableClass(class),
            mir::Type::Interface(mir::InterfaceTypeId::ERROR),
            mir::Type::NullableInterface(mir::InterfaceTypeId::ERROR),
            mir::Type::Collection(mir::CollectionTypeId(0)),
            mir::Type::NullableCollection(mir::CollectionTypeId(0)),
            mir::Type::Scalar(mir::ScalarType::Inline(inline)),
            mir::Type::NullableScalar(mir::ScalarType::Inline(inline)),
        ] {
            function.locals = vec![mir::Local {
                id: mir::LocalId(0),
                name: "source".into(),
                ty,
                writable: false,
                owned: false,
                synthetic: false,
            }];
            assert!(super::has_nominal_borrow_home(ty), "{ty}");
            assert!(
                super::has_addressable_borrow_home(&projected, &function),
                "{ty}"
            );
        }
        function.locals[0].ty = mir::Type::Scalar(mir::ScalarType::Bool);
        assert!(!super::has_addressable_borrow_home(&projected, &function));
        function.locals.clear();
        assert!(!super::has_addressable_borrow_home(&projected, &function));

        let class = program
            .classes
            .iter()
            .find(|class| class.name == crate::compiler_known_stream::STANDARD_INPUT)
            .expect("standard input class");
        let ty = class.inline_copy_type().expect("inline standard input");
        for interface in &program.interface_types {
            let sources =
                super::borrow_home_projection_storage(&program, mir::Type::Interface(interface.id));
            let conforms = program
                .interface_vtable(mir::ImplementingType::Class(class.id), interface.id)
                .is_some();
            for source in [
                mir::Type::Scalar(mir::ScalarType::Inline(ty)),
                mir::Type::NullableScalar(mir::ScalarType::Inline(ty)),
            ] {
                assert_eq!(
                    sources.contains(&super::BorrowHomeStorage::Value(source)),
                    conforms
                );
            }
            assert!(!sources.contains(&super::BorrowHomeStorage::Value(mir::Type::Class(class.id))));
            assert!(!sources.contains(&super::BorrowHomeStorage::BoxedClass(class.id)));
        }
    }

    #[test]
    fn borrowed_parameters_keep_their_homes_without_a_borrowed_return() {
        let function = mir::FunctionType {
            id: mir::FunctionTypeId(0),
            invocation_mode: mir::FunctionInvocationMode::Readonly,
            parameters: vec![
                mir::FunctionParameter {
                    ty: mir::Type::Class(crate::class_layout::ClassId(0)),
                    mode: mir::FunctionParameterMode::Readonly,
                },
                mir::FunctionParameter {
                    ty: mir::Type::Collection(mir::CollectionTypeId(0)),
                    mode: mir::FunctionParameterMode::Writable,
                },
                mir::FunctionParameter {
                    ty: mir::Type::Class(crate::class_layout::ClassId(0)),
                    mode: mir::FunctionParameterMode::Take,
                },
                mir::FunctionParameter {
                    ty: mir::Type::Scalar(mir::ScalarType::Integer(
                        crate::numeric::IntegerType::Int64,
                    )),
                    mode: mir::FunctionParameterMode::Readonly,
                },
            ],
            return_type: mir::ReturnType::Void,
            checked_effects: vec![mir::CheckedEffect::Any],
            ambient_checked_effects: vec![],
            test_assertion_checked_effects: vec![],
            return_borrow: None,
        };
        for plan in [
            NativeCallableSignaturePlan::indirect(&function),
            NativeCallableSignaturePlan::interface_entry(&function),
        ] {
            assert_eq!(
                plan.parameter_homes()
                    .map(|(_, parameter)| parameter)
                    .collect::<Vec<_>>(),
                vec![0, 1]
            );
            assert!(plan
                .index_of(NativeCallableHiddenInput::BorrowHome)
                .is_none());
        }
        let returned = mir::FunctionType {
            return_type: mir::ReturnType::Value(mir::Type::Function(mir::FunctionTypeId(1))),
            return_borrow: Some(mir::ReturnBorrow {
                kind: crate::types::ReturnBorrowKind::Retained,
                lifetime: mir::RetainedLifetime::LastUse,
                source: mir::BorrowSource::Parameter(0),
                writable: false,
            }),
            ..function
        };
        let plan = NativeCallableSignaturePlan::indirect(&returned);
        assert_eq!(
            plan.parameter_homes()
                .map(|(_, parameter)| parameter)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(plan
            .index_of(NativeCallableHiddenInput::BorrowHome)
            .is_none());
    }

    #[test]
    fn writable_copy_constructor_inputs_have_homes_only_when_retained() {
        let program = crate::lower_source_to_mir(
            "constructor-borrow-home.doria",
            "class Holder { function __construct(writable int $value) {} }\n\
             function main(): void { let writable $value = 1; let $holder = new Holder($value); }",
        )
        .unwrap();
        let constructor = program
            .classes
            .iter()
            .find(|class| class.name == "Holder")
            .and_then(|class| class.constructor)
            .unwrap();
        let mut function = program.functions[constructor.0].clone();
        let input = function.params[1];
        let homes = |function: &mir::Function| {
            NativeCallableSignaturePlan::direct(function)
                .parameter_homes()
                .map(|(_, index)| index)
                .collect::<Vec<_>>()
        };
        assert!(!homes(&function).contains(&1));

        // Source admission for retained writable fields is still gated.
        function.blocks[function.entry_block.0].statements.insert(
            0,
            mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(
                mir::RetainedSourcesPlan {
                    constructs: vec![mir::RetainedSource {
                        local: input,
                        inherited: false,
                        writable: true,
                        lifetime: mir::RetainedLifetime::Cleanup,
                        owned_path: vec![],
                    }],
                    ..Default::default()
                },
            )),
        );
        assert!(homes(&function).contains(&1));
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
                lifetime: mir::RetainedLifetime::LastUse,
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
                    lifetime: mir::RetainedLifetime::LastUse,
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
                        lifetime: mir::RetainedLifetime::LastUse,
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

    #[test]
    fn direct_returns_fit_the_smallest_target_return_budget() {
        use super::{direct_value_slots, returns_through_result_slot, DIRECT_RETURN_SLOTS};
        use crate::inline_value::InlineValueType;
        use crate::numeric::{FloatType, IntegerType};
        use crate::types::ByteRegionKind;

        let value = mir::ReturnType::Value;
        let int = mir::ScalarType::Integer(IntegerType::Int64);
        let float = mir::ScalarType::Float(FloatType::Float64);
        let deadline = mir::ScalarType::Inline(InlineValueType::Deadline);
        // Two slots, at most one floating-point: x86-64 System V and Windows
        // x64 return these in rax/rdx and xmm0, AArch64 in x0/x1 and v0.
        for ty in [
            mir::Type::Scalar(int),
            mir::Type::Scalar(float),
            mir::Type::Scalar(deadline),
            mir::Type::NullableScalar(int),
            mir::Type::NullableScalar(float),
            mir::Type::ByteRegion(ByteRegionKind::Readonly),
            mir::Type::NullableString,
            mir::Type::Interface(mir::InterfaceTypeId(0)),
            mir::Type::NullableFunction(mir::FunctionTypeId(0)),
            mir::Type::NullableClass(crate::class_layout::ClassId(0)),
        ] {
            assert!(direct_value_slots(ty).fits(DIRECT_RETURN_SLOTS), "{ty:?}");
            assert!(!returns_through_result_slot(value(ty), false), "{ty:?}");
            assert!(returns_through_result_slot(value(ty), true), "{ty:?}");
        }
        // Three slots exceed x86-64's two integer return registers.
        for ty in [
            mir::Type::NullableByteRegion(ByteRegionKind::Readonly),
            mir::Type::NullableByteRegion(ByteRegionKind::Writable),
            mir::Type::NullableScalar(deadline),
        ] {
            assert_eq!(direct_value_slots(ty).total, 3, "{ty:?}");
            assert!(returns_through_result_slot(value(ty), false), "{ty:?}");
        }
        assert!(!returns_through_result_slot(mir::ReturnType::Void, false));
        assert!(!returns_through_result_slot(mir::ReturnType::Void, true));
    }
}
