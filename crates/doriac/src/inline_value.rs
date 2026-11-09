//! Compiler-known nominal Copy values. Their bits are private ABI, not integers
//! in the language and not heap-class ownership or user-defined Copy classes.

use crate::class_layout::ClassId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InlineValueKind {
    Duration,
    Deadline,
    RegistrationKey,
    ReadTransferKey,
    WriteTransferKey,
    CancellationToken,
    StandardInput,
    StandardOutput,
    StandardError,
}

impl InlineValueKind {
    pub fn for_name(name: &str) -> Option<Self> {
        match name {
            crate::compiler_known_time::DURATION => Some(Self::Duration),
            crate::compiler_known_time::DEADLINE => Some(Self::Deadline),
            crate::compiler_known_stream::REGISTRATION_KEY => Some(Self::RegistrationKey),
            crate::compiler_known_stream::READ_TRANSFER_KEY => Some(Self::ReadTransferKey),
            crate::compiler_known_stream::WRITE_TRANSFER_KEY => Some(Self::WriteTransferKey),
            crate::compiler_known_concurrency::CANCELLATION_TOKEN => Some(Self::CancellationToken),
            crate::compiler_known_stream::STANDARD_INPUT => Some(Self::StandardInput),
            crate::compiler_known_stream::STANDARD_OUTPUT => Some(Self::StandardOutput),
            crate::compiler_known_stream::STANDARD_ERROR => Some(Self::StandardError),
            _ => None,
        }
    }

    pub fn for_nominal(name: &str, argument_count: usize) -> Option<Self> {
        Self::for_name(name).filter(|kind| argument_count == kind.argument_count())
    }

    pub const fn argument_count(self) -> usize {
        match self {
            Self::Duration
            | Self::Deadline
            | Self::ReadTransferKey
            | Self::WriteTransferKey
            | Self::CancellationToken
            | Self::StandardInput
            | Self::StandardOutput
            | Self::StandardError => 0,
            Self::RegistrationKey => 1,
        }
    }

    pub const fn specialize(self, class: ClassId) -> InlineValueType {
        match self {
            Self::Duration => InlineValueType::Duration,
            Self::Deadline => InlineValueType::Deadline,
            Self::RegistrationKey => InlineValueType::RegistrationKey(class),
            Self::ReadTransferKey => InlineValueType::ReadTransferKey(class),
            Self::WriteTransferKey => InlineValueType::WriteTransferKey(class),
            Self::CancellationToken => InlineValueType::CancellationToken(class),
            Self::StandardInput => InlineValueType::StandardInput(class),
            Self::StandardOutput => InlineValueType::StandardOutput(class),
            Self::StandardError => InlineValueType::StandardError(class),
        }
    }

    pub const fn storage_bytes(self) -> u32 {
        match self {
            Self::Duration
            | Self::CancellationToken
            | Self::StandardInput
            | Self::StandardOutput
            | Self::StandardError => 8,
            Self::Deadline
            | Self::RegistrationKey
            | Self::ReadTransferKey
            | Self::WriteTransferKey => 16,
        }
    }

    pub const fn requires_owned_cleanup(self) -> bool {
        self.specialize(ClassId(0)).requires_owned_cleanup()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InlineValueType {
    Duration,
    Deadline,
    RegistrationKey(ClassId),
    ReadTransferKey(ClassId),
    WriteTransferKey(ClassId),
    CancellationToken(ClassId),
    StandardInput(ClassId),
    StandardOutput(ClassId),
    StandardError(ClassId),
}

/// Cleanup belongs to the value representation, independently of Copy capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineValueLifecycle {
    Trivial,
    SharedCancellation,
}

impl InlineValueType {
    pub fn for_name(name: &str) -> Option<Self> {
        match name {
            crate::compiler_known_time::DURATION => Some(Self::Duration),
            crate::compiler_known_time::DEADLINE => Some(Self::Deadline),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Duration => crate::compiler_known_time::DURATION,
            Self::Deadline => crate::compiler_known_time::DEADLINE,
            Self::RegistrationKey(_) => crate::compiler_known_stream::REGISTRATION_KEY,
            Self::ReadTransferKey(_) => crate::compiler_known_stream::READ_TRANSFER_KEY,
            Self::WriteTransferKey(_) => crate::compiler_known_stream::WRITE_TRANSFER_KEY,
            Self::CancellationToken(_) => crate::compiler_known_concurrency::CANCELLATION_TOKEN,
            Self::StandardInput(_) => crate::compiler_known_stream::STANDARD_INPUT,
            Self::StandardOutput(_) => crate::compiler_known_stream::STANDARD_OUTPUT,
            Self::StandardError(_) => crate::compiler_known_stream::STANDARD_ERROR,
        }
    }

    pub const fn storage_bytes(self) -> u32 {
        match self {
            Self::Duration
            | Self::CancellationToken(_)
            | Self::StandardInput(_)
            | Self::StandardOutput(_)
            | Self::StandardError(_) => 8,
            Self::Deadline
            | Self::RegistrationKey(_)
            | Self::ReadTransferKey(_)
            | Self::WriteTransferKey(_) => 16,
        }
    }

    pub fn type_id(self) -> u32 {
        match self {
            Self::Duration => 0,
            Self::Deadline => 1,
            Self::RegistrationKey(class)
            | Self::ReadTransferKey(class)
            | Self::WriteTransferKey(class)
            | Self::CancellationToken(class)
            | Self::StandardInput(class)
            | Self::StandardOutput(class)
            | Self::StandardError(class) => u32::try_from(class.0)
                .ok()
                .and_then(|id| id.checked_add(2))
                .expect("validated inline nominal type identity"),
        }
    }

    pub fn nominal_class(self) -> Option<ClassId> {
        match self {
            Self::RegistrationKey(class)
            | Self::ReadTransferKey(class)
            | Self::WriteTransferKey(class)
            | Self::CancellationToken(class)
            | Self::StandardInput(class)
            | Self::StandardOutput(class)
            | Self::StandardError(class) => Some(class),
            Self::Duration | Self::Deadline => None,
        }
    }

    pub const fn lifecycle(self) -> InlineValueLifecycle {
        match self {
            Self::CancellationToken(_) => InlineValueLifecycle::SharedCancellation,
            Self::Duration
            | Self::Deadline
            | Self::RegistrationKey(_)
            | Self::ReadTransferKey(_)
            | Self::WriteTransferKey(_)
            | Self::StandardInput(_)
            | Self::StandardOutput(_)
            | Self::StandardError(_) => InlineValueLifecycle::Trivial,
        }
    }

    pub const fn requires_owned_cleanup(self) -> bool {
        !matches!(self.lifecycle(), InlineValueLifecycle::Trivial)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InlineValue {
    pub ty: InlineValueType,
    pub bits: u128,
}

impl InlineValue {
    pub const fn fits_storage(self) -> bool {
        self.ty.storage_bytes() == 16 || self.bits <= u64::MAX as u128
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::{ScalarType, Type};

    #[test]
    fn cancellation_token_kind_requires_its_exact_zero_argument_nominal_name() {
        let name = crate::compiler_known_concurrency::CANCELLATION_TOKEN;
        let kind = InlineValueKind::CancellationToken;
        assert_eq!(InlineValueKind::for_name(name), Some(kind));
        assert_eq!(InlineValueKind::for_nominal(name, 0), Some(kind));
        assert_eq!(InlineValueKind::for_nominal(name, 1), None);
        for ordinary in ["CancellationToken", "User\\CancellationToken"] {
            assert_eq!(InlineValueKind::for_name(ordinary), None);
        }
        assert_eq!(kind.argument_count(), 0);
        assert_eq!(kind.storage_bytes(), 8);
        assert!(kind.requires_owned_cleanup());
        assert_eq!(
            kind.specialize(ClassId(7)),
            InlineValueType::CancellationToken(ClassId(7))
        );
    }

    #[test]
    fn managed_inline_copy_keeps_cleanup_independent_of_move_capability() {
        let token = InlineValueType::CancellationToken(ClassId(7));
        assert_eq!(token.lifecycle(), InlineValueLifecycle::SharedCancellation);
        assert!(token.requires_owned_cleanup());
        for ty in [
            Type::Scalar(ScalarType::Inline(token)),
            Type::NullableScalar(ScalarType::Inline(token)),
        ] {
            assert!(!ty.has_move_ownership());
            assert!(ty.requires_owned_cleanup());
            assert!(crate::native_closure_abi::needs_drop(ty));
            assert!(!ty.borrows_returned_value(None));
        }
    }

    #[test]
    fn managed_inline_identity_and_layout_do_not_become_heap_class_storage() {
        let token = InlineValueType::CancellationToken(ClassId(7));
        assert_eq!(
            token.name(),
            crate::compiler_known_concurrency::CANCELLATION_TOKEN
        );
        assert_eq!(token.nominal_class(), Some(ClassId(7)));
        assert_eq!(token.type_id(), 9);
        assert_eq!(token.storage_bytes(), 8);
        assert_ne!(token, InlineValueType::ReadTransferKey(ClassId(7)));
        assert_ne!(token.type_id(), InlineValueType::Duration.type_id());
        assert_ne!(token.type_id(), InlineValueType::Deadline.type_id());
        for pointer_size in [4, 8] {
            let plain = Type::Scalar(ScalarType::Inline(token));
            let nullable = Type::NullableScalar(ScalarType::Inline(token));
            let layout = crate::native_closure_abi::type_layout(plain, pointer_size);
            assert_eq!((layout.size, layout.align), (8, 8));
            let optional = crate::native_closure_abi::type_layout(nullable, pointer_size);
            assert_eq!((optional.size, optional.align), (16, 8));
            assert_eq!(
                crate::native_abi::collection_value_width(plain, pointer_size as u8),
                Some(8)
            );
            assert_eq!(
                crate::native_abi::nullable_payload_type(nullable),
                Some(plain)
            );
        }
    }

    #[test]
    fn existing_inline_copy_types_remain_trivial_and_cleanup_free() {
        for (kind, bytes) in [
            (InlineValueKind::Duration, 8),
            (InlineValueKind::Deadline, 16),
            (InlineValueKind::RegistrationKey, 16),
            (InlineValueKind::ReadTransferKey, 16),
            (InlineValueKind::WriteTransferKey, 16),
        ] {
            let inline = kind.specialize(ClassId(7));
            assert_eq!(inline.storage_bytes(), bytes);
            assert_eq!(kind.storage_bytes(), bytes);
            assert_eq!(inline.lifecycle(), InlineValueLifecycle::Trivial);
            assert!(!kind.requires_owned_cleanup());
            assert!(!inline.requires_owned_cleanup());
            for ty in [
                Type::Scalar(ScalarType::Inline(inline)),
                Type::NullableScalar(ScalarType::Inline(inline)),
            ] {
                assert!(!ty.has_move_ownership());
                assert!(!ty.requires_owned_cleanup());
                assert!(!crate::native_closure_abi::needs_drop(ty));
            }
        }
    }

    #[test]
    fn borrow_cleanup_is_not_a_native_value_destructor() {
        for ty in [
            Type::ByteRegion(crate::mir::ByteRegionKind::Writable),
            Type::NullableByteRegion(crate::mir::ByteRegionKind::Writable),
        ] {
            assert!(ty.requires_owned_cleanup());
            assert!(!crate::native_closure_abi::needs_drop(ty));
        }
        for ty in [Type::String, Type::NullableString] {
            assert!(!ty.has_move_ownership());
            assert!(!ty.requires_owned_cleanup());
            assert!(crate::native_closure_abi::needs_drop(ty));
        }
    }

    #[test]
    fn inline_claim_modes_distinguish_copy_handoff_and_borrow() {
        use crate::mir::{
            InlineValueExpression, InlineValueUseMode, LocalId, Operand, Rvalue, ValueExpression,
        };
        let token = InlineValueType::CancellationToken(ClassId(7));
        for mode in [
            InlineValueUseMode::Borrow,
            InlineValueUseMode::Copy,
            InlineValueUseMode::TransferOwnedCopy,
        ] {
            let value = Rvalue::Value(ValueExpression::Inline(InlineValueExpression::Use {
                ty: token,
                operand: Operand::Local(LocalId(3)),
                mode,
            }));
            assert_eq!(
                value.borrows_move_value(),
                mode == InlineValueUseMode::Borrow
            );
            assert_eq!(
                value.transferred_owned_local(),
                (mode == InlineValueUseMode::TransferOwnedCopy).then_some(LocalId(3))
            );
            assert!(!value.is_null_value());
        }
    }

    #[test]
    fn nullable_inline_claims_preserve_absence_and_own_selected_branches() {
        use crate::mir::{InlineValueUseMode, LocalId, NullableScalarExpression, Rvalue};
        let ty = InlineValueType::CancellationToken(ClassId(7));
        let null = NullableScalarExpression::Null(ScalarType::Inline(ty));
        assert!(Rvalue::NullableScalar(null.clone()).is_null_value());
        for mode in [
            InlineValueUseMode::Borrow,
            InlineValueUseMode::Copy,
            InlineValueUseMode::TransferOwnedCopy,
        ] {
            let claim = NullableScalarExpression::InlineClaim {
                ty,
                value: Box::new(NullableScalarExpression::Local {
                    ty: ScalarType::Inline(ty),
                    local: LocalId(3),
                }),
                mode,
            };
            assert_eq!(
                claim.owned_inline_temporary(),
                mode != InlineValueUseMode::Borrow
            );
            let value = Rvalue::NullableScalar(claim.clone());
            assert_eq!(
                value.transferred_owned_local(),
                (mode == InlineValueUseMode::TransferOwnedCopy).then_some(LocalId(3))
            );
            let coalesce = NullableScalarExpression::Coalesce {
                ty: ScalarType::Inline(ty),
                left: Box::new(null.clone()),
                right: Box::new(claim),
            };
            assert_eq!(
                coalesce.owned_inline_temporary(),
                mode != InlineValueUseMode::Borrow
            );
        }
    }
}
