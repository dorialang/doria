//! Checking pointer members and C boundary conversions (Decision 0148
//! sections 4, 5 and 7).
//!
//! The members and their contracts are the table in
//! [`crate::pointer_intrinsics`]; this module types them. A conversion whose
//! target comes from the expected type (`Pointer::cast`, `Pointer::fromAddress`,
//! `Pointer::toFunction`) reads the type recorded for the call's position.

use super::*;
use crate::byte_intrinsics::RegionCompanion;
use crate::foreign::{ForeignType, PointerAccess};
use crate::pointer_intrinsics::{
    self, BoundaryConversion, ForeignCompanion, PointerMember, PointerMemberKind, PointerOperation,
};

pub(super) const UNSIZED_POINTEE_CODE: &str = "E0800";
pub(super) const POINTER_PLACE_CODE: &str = "E0801";
pub(super) const POINTER_TARGET_CODE: &str = "E0802";
pub(super) const REGION_OWNER_CODE: &str = "E0806";

#[derive(Debug, Clone, Copy)]
struct PointerReceiver {
    /// The pointer type without its nullability.
    pointer: TypeId,
    access: PointerAccess,
    pointee: TypeId,
}

impl<'program> Checker<'program> {
    fn pointer_receiver(&self, ty: TypeId) -> Option<PointerReceiver> {
        let pointer = match self.types.kind(ty) {
            TypeKind::Nullable(inner) => *inner,
            _ => ty,
        };
        match self.types.kind(pointer) {
            TypeKind::Foreign(ForeignType::Pointer { access, pointee }) => Some(PointerReceiver {
                pointer,
                access: *access,
                pointee: *pointee,
            }),
            _ => None,
        }
    }

    fn pointer_parameter(&mut self, name: &str, ty: TypeId) -> ParamInfo {
        ParamInfo {
            name: name.to_string(),
            ty,
            take: false,
            writable: false,
            has_default: false,
            default: None,
            declaration: None,
        }
    }

    /// The parameters of a pointer method, or `None` when `object_ty` is not
    /// a pointer or has no such method.
    fn pointer_method_parameters(
        &mut self,
        receiver: PointerReceiver,
        method: &str,
    ) -> Option<Vec<ParamInfo>> {
        let member = pointer_intrinsics::instance_member(receiver.access, method)
            .filter(|member| member.kind == PointerMemberKind::Method)?;
        let int = self.types.intern(TypeKind::Integer(IntegerType::Int64));
        Some(match member.operation {
            PointerOperation::Write { .. } => {
                vec![self.pointer_parameter("value", receiver.pointee)]
            }
            PointerOperation::Offset => vec![self.pointer_parameter("count", int)],
            PointerOperation::ByteOffset => vec![self.pointer_parameter("bytes", int)],
            _ => Vec::new(),
        })
    }

    /// Records the expected type of each argument of a pointer method.
    /// Returns false when `object_ty` is not a pointer.
    pub(super) fn record_pointer_method_argument_types(
        &mut self,
        object_ty: TypeId,
        method: &str,
        args: &[Argument],
    ) -> bool {
        let Some(receiver) = self.pointer_receiver(object_ty) else {
            return false;
        };
        if let Some(parameters) = self.pointer_method_parameters(receiver, method) {
            self.record_expected_argument_types(&parameters, args);
        }
        true
    }

    /// The pointee's size is unknown: `Opaque`, or a foreign struct declared
    /// without fields. Returns the pointee's name.
    fn unsized_pointee(&self, pointee: TypeId) -> Option<(String, bool)> {
        match self.types.kind(pointee) {
            TypeKind::Foreign(ForeignType::Opaque) => Some(("Opaque".to_string(), true)),
            TypeKind::Foreign(ForeignType::Aggregate(aggregate))
                if self
                    .foreign
                    .aggregates
                    .get(&aggregate.name)
                    .is_some_and(|info| info.fields.is_none()) =>
            {
                let short = aggregate
                    .name
                    .rsplit('\\')
                    .next()
                    .unwrap_or(&aggregate.name);
                Some((short.to_string(), false))
            }
            _ => None,
        }
    }

    fn report_unsized_pointee(
        &mut self,
        receiver: PointerReceiver,
        member: &PointerMember,
        span: Span,
    ) -> bool {
        let Some((pointee, opaque)) = self.unsized_pointee(receiver.pointee) else {
            return false;
        };
        let pointer = self.types.display(receiver.pointer);
        let name = member.name;
        let mut diagnostic = if opaque {
            Diagnostic::new(
                UNSIZED_POINTEE_CODE,
                format!("`{pointer}` has no pointee type, so it has no `{name}()`"),
                span,
            )
            .with_help(format!(
                "convert it to a pointer to the C type it addresses first, as in `{}<uint8> $bytes = {}::cast($pointer);`",
                receiver.access.short_name(),
                receiver.access.short_name()
            ))
        } else {
            Diagnostic::new(
                UNSIZED_POINTEE_CODE,
                format!(
                    "`{pointee}` is declared without fields and has no size, so `{pointer}` has no `{name}()`"
                ),
                span,
            )
            .with_help(
                "pass the pointer to the C functions that use it, or declare the struct's fields",
            )
        };
        diagnostic = diagnostic
            .with_title("Pointer Operation Needs A Pointee With A Size")
            .with_explanation(
                "Reading, writing and offsetting by elements use the size of the pointee. `Opaque` and a foreign struct declared without fields have none.",
            );
        if member.operation == PointerOperation::Offset {
            diagnostic =
                diagnostic.with_note("`byteOffset` moves any pointer by a number of bytes");
        }
        self.diagnostics.push(diagnostic);
        true
    }

    /// `$pointer->method(...)`. Returns false when the receiver is not a
    /// pointer.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn check_pointer_method_call(
        &mut self,
        object: &Expr,
        method: &str,
        member_span: Span,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let object_ty = self.infer_expr_type(object, scopes, method_context);
        let Some(receiver) = self.pointer_receiver(object_ty) else {
            return false;
        };
        let member = pointer_intrinsics::instance_member(receiver.access, method)
            .filter(|member| member.kind == PointerMemberKind::Method);
        let Some(member) = member else {
            self.report_unknown_pointer_member(receiver, method, member_span, true);
            return true;
        };
        if let Some(operation) = member.operation.unsafe_description() {
            self.require_unsafe(span, operation);
        }
        if member.operation.needs_sized_pointee()
            && self.report_unsized_pointee(receiver, &member, member_span)
        {
            return true;
        }
        let callee = format!("pointer method `{method}`");
        if self.reject_named_arguments(&callee, args) {
            return true;
        }
        let parameters = self
            .pointer_method_parameters(receiver, method)
            .expect("a pointer method has parameters");
        self.check_call_arguments(&callee, &parameters, args, span, scopes, method_context);
        true
    }

    fn report_unknown_pointer_member(
        &mut self,
        receiver: PointerReceiver,
        name: &str,
        span: Span,
        called: bool,
    ) {
        let pointer = self.types.display(receiver.pointer);
        let mut diagnostic = if called {
            Diagnostic::new(
                "E0304",
                format!("type `{pointer}` has no instance method `{name}`"),
                span,
            )
            .with_title("Type Has No Instance Method")
        } else {
            Diagnostic::new(
                "E0303",
                format!("type `{pointer}` has no property `{name}`"),
                span,
            )
            .with_title("Unknown Property")
        };
        let other_access = match receiver.access {
            PointerAccess::Read => PointerAccess::Write,
            PointerAccess::Write => PointerAccess::Read,
        };
        if let Some(member) = pointer_intrinsics::instance_member(receiver.access, name) {
            diagnostic = diagnostic.with_help(match member.kind {
                PointerMemberKind::Property => {
                    format!("`{name}` is a property: write `$pointer->{name}`")
                }
                _ => format!("`{name}` is a method: write `$pointer->{name}(...)`"),
            });
        } else if pointer_intrinsics::instance_member(other_access, name).is_some() {
            diagnostic = diagnostic.with_help(format!(
                "`Pointer<T>` gives read access only; `{name}` needs a `WritablePointer<T>`"
            ));
        } else {
            let members = pointer_intrinsics::instance_members(receiver.access)
                .iter()
                .map(|member| format!("`{}`", member.name))
                .collect::<Vec<_>>()
                .join(", ");
            diagnostic = diagnostic.with_help(format!(
                "a `{}` has {members}",
                receiver.access.short_name()
            ));
        }
        self.diagnostics.push(diagnostic);
    }

    /// The result of `$pointer->method(...)`, without reporting; `None` when
    /// the receiver is not a pointer.
    pub(super) fn pointer_method_type(
        &mut self,
        object_ty: TypeId,
        method: &str,
        null_safe: bool,
    ) -> Option<TypeId> {
        let receiver = self.pointer_receiver(object_ty)?;
        let member = pointer_intrinsics::instance_member(receiver.access, method)
            .filter(|member| member.kind == PointerMemberKind::Method);
        if member.is_some_and(|member| member.operation.needs_sized_pointee())
            && self.unsized_pointee(receiver.pointee).is_some()
        {
            return Some(self.types.unknown());
        }
        let result = match member.map(|member| member.operation) {
            Some(PointerOperation::Read { .. }) => receiver.pointee,
            Some(PointerOperation::Write { .. }) => self.types.intern(TypeKind::Void),
            Some(PointerOperation::Offset | PointerOperation::ByteOffset) => receiver.pointer,
            _ => return Some(self.types.unknown()),
        };
        Some(self.null_safe_result_type(result, null_safe))
    }

    /// `$pointer->property`. Returns `None` when the receiver is not a
    /// pointer.
    pub(super) fn check_pointer_property(
        &mut self,
        object_ty: TypeId,
        property: &str,
        member_span: Span,
        null_safe: bool,
    ) -> Option<TypeId> {
        let receiver = self.pointer_receiver(object_ty)?;
        let known = pointer_intrinsics::instance_member(receiver.access, property)
            .is_some_and(|member| member.kind == PointerMemberKind::Property);
        if !known {
            self.report_unknown_pointer_member(receiver, property, member_span, false);
        }
        self.pointer_property_type(object_ty, property, null_safe)
    }

    pub(super) fn pointer_property_type(
        &mut self,
        object_ty: TypeId,
        property: &str,
        null_safe: bool,
    ) -> Option<TypeId> {
        let receiver = self.pointer_receiver(object_ty)?;
        let result = match pointer_intrinsics::instance_member(receiver.access, property)
            .filter(|member| member.kind == PointerMemberKind::Property)
            .map(|member| member.operation)
        {
            Some(PointerOperation::Address) => {
                self.types.intern(TypeKind::Integer(IntegerType::UInt64))
            }
            _ => return Some(self.types.unknown()),
        };
        Some(self.null_safe_result_type(result, null_safe))
    }

    /// A static call on `Pointer`, `WritablePointer`, `Opaque` or a C boundary type.
    /// Returns false for every other class.
    pub(super) fn check_foreign_companion_call(
        &mut self,
        class_name: &str,
        access: StaticAccess<'_>,
        args: &[Argument],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        if class_name == crate::foreign::OPAQUE {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0304",
                    format!("`Opaque` has no static method `{}`", access.member),
                    access.member_span,
                )
                .with_title("Unknown Method")
                .with_help("`Opaque` names the untyped pointee of `Pointer<Opaque>` and `WritablePointer<Opaque>`; it has no members"),
            );
            return true;
        }
        if self.check_handle_static_call(class_name, access, args, scopes, method_context) {
            return true;
        }
        if let Some(region) = RegionCompanion::from_class_name(class_name) {
            self.check_region_from_pointer(region, access, args, scopes, method_context);
            return true;
        }
        let Some(companion) = ForeignCompanion::from_class_name(class_name) else {
            return false;
        };
        match companion {
            ForeignCompanion::Pointer(pointer) => {
                self.check_pointer_static_call(pointer, access, args, scopes, method_context)
            }
            ForeignCompanion::Boundary(boundary) => {
                self.check_boundary_conversion(boundary, access, args, scopes, method_context)
            }
            ForeignCompanion::OwnedString(encoding) => {
                self.check_owned_string_construction(encoding, access, args, scopes, method_context)
            }
        }
        true
    }

    /// `ByteRegion::fromPointer($owner, $pointer, $length)` and its three
    /// siblings (section 10): an unsafe constructor whose region borrows
    /// `$owner`, a place, as `getRegion` borrows its receiver.
    fn check_region_from_pointer(
        &mut self,
        region: RegionCompanion,
        access: StaticAccess<'_>,
        args: &[Argument],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let companion = region.short_name();
        if access.member != crate::byte_intrinsics::FROM_POINTER {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0304",
                    format!("unknown method `{companion}::{}`", access.member),
                    access.member_span,
                )
                .with_title("Unknown Method")
                .with_help(format!(
                    "`{companion}::fromPointer($owner, $pointer, $length)` views memory a foreign library owns; a region over Doria storage comes from `getRegion` or `getWritableRegion`"
                )),
            );
            return;
        }
        let callee = format!("`{companion}::fromPointer`");
        self.require_unsafe(access.span, "forming a region over foreign memory");
        if args.len() != crate::byte_intrinsics::FROM_POINTER_PARAMETERS.len() {
            self.report_argument_count_mismatch(&callee, 3, 3, args.len(), access.span);
            return;
        }
        let owner = crate::byte_intrinsics::from_pointer_argument(args, 0);
        let owner_ty = match owner {
            Some((_, owner)) => self.infer_expr_type(&owner.value, scopes, method_context),
            None => self.types.unknown(),
        };
        let pointer_ty = self.region_pointer_parameter(region, args, scopes, method_context);
        let int = self.types.intern(TypeKind::Integer(IntegerType::Int64));
        let [owner_name, pointer_name, length_name] =
            crate::byte_intrinsics::FROM_POINTER_PARAMETERS;
        let mut owner_parameter = self.pointer_parameter(owner_name, owner_ty);
        owner_parameter.writable = region.kind.is_writable();
        let parameters = [
            owner_parameter,
            self.pointer_parameter(pointer_name, pointer_ty),
            self.pointer_parameter(length_name, int),
        ];
        self.check_call_arguments(
            &callee,
            &parameters,
            args,
            access.span,
            scopes,
            method_context,
        );
        if let Some((_, owner)) = owner {
            if !matches!(ungroup_place(&owner.value), Expr::This { .. })
                && !self.is_pointer_place(&owner.value, method_context)
            {
                self.diagnostics.push(
                    Diagnostic::new(
                        REGION_OWNER_CODE,
                        format!("the region from {callee} borrows its owner, and this is a value with no place"),
                        owner.value.span(),
                    )
                    .with_title("Region Owner Is Not A Place")
                    .with_explanation(
                        "A region over foreign memory borrows its owner, the Doria value whose destructor releases the memory, so the region cannot outlive it. The owner is a local, `$this` or a property; a computed value would be destroyed while the region still views its memory.",
                    )
                    .with_help("store the owner in a local first, then pass the local"),
                );
            }
        }
    }

    /// The pointer `fromPointer` on `region` takes: `Pointer<uint8>` or
    /// `WritablePointer<uint8>` for a byte region, and for a typed region
    /// the pointer the call passes, whose pointee is the element type and
    /// must be FFI-safe.
    fn region_pointer_parameter(
        &mut self,
        region: RegionCompanion,
        args: &[Argument],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        let access = region.pointer_access();
        if !region.typed {
            let byte = self.types.intern(TypeKind::Integer(IntegerType::UInt8));
            return self.types.intern(TypeKind::Foreign(ForeignType::Pointer {
                access,
                pointee: byte,
            }));
        }
        let Some((_, argument)) = crate::byte_intrinsics::from_pointer_argument(args, 1) else {
            return self.types.unknown();
        };
        let argument_ty = self.infer_expr_type(&argument.value, scopes, method_context);
        let Some(receiver) = self.pointer_receiver(argument_ty) else {
            if !matches!(self.types.kind(argument_ty), TypeKind::Unknown) {
                let got = self.types.display(argument_ty);
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0408",
                        format!(
                            "argument `pointer` of `{}::fromPointer` expects `{}<T>`, got `{got}`",
                            region.short_name(),
                            access.short_name()
                        ),
                        argument.value.span(),
                    )
                    .with_title("Argument Type Mismatch"),
                );
            }
            return argument_ty;
        };
        if !self.defer_generic_pointee(receiver.pointee) {
            self.require_ffi_safe(
                receiver.pointee,
                argument.value.span(),
                "an array region element",
            );
        }
        self.types.intern(TypeKind::Foreign(ForeignType::Pointer {
            access,
            pointee: receiver.pointee,
        }))
    }

    /// `CString::from($text)`, `CString::tryFrom($text)` and the
    /// `Utf16String` pair (section 9).
    fn check_owned_string_construction(
        &mut self,
        encoding: crate::foreign::ForeignStringEncoding,
        access: StaticAccess<'_>,
        args: &[Argument],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let companion = encoding.short_name();
        let Some(member) = crate::foreign_strings::member(encoding, access.member)
            .filter(|member| member.kind == crate::byte_intrinsics::ByteMemberKind::StaticMethod)
        else {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0304",
                    format!("unknown method `{companion}::{}`", access.member),
                    access.member_span,
                )
                .with_title("Unknown Method")
                .with_help(format!(
                    "`{companion}::from(...)` and `{companion}::tryFrom(...)` are available; `pointer` gives C the address of the buffer"
                )),
            );
            return;
        };
        let parameters = member.parameter_info(&mut self.types);
        self.check_call_arguments(
            &format!("{companion}::{}", member.name),
            &parameters,
            args,
            access.span,
            scopes,
            method_context,
        );
    }

    fn check_pointer_static_call(
        &mut self,
        pointer: PointerAccess,
        access: StaticAccess<'_>,
        args: &[Argument],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let companion = pointer.short_name();
        let member_name = access.member;
        let Some(member) = pointer_intrinsics::static_member(pointer, member_name) else {
            let mut diagnostic = Diagnostic::new(
                "E0304",
                format!("unknown method `{companion}::{member_name}`"),
                access.member_span,
            )
            .with_title("Unknown Method");
            diagnostic = diagnostic.with_help(match (pointer, member_name) {
                (PointerAccess::Write, "fromAddress") => {
                    "convert the address with `Pointer::fromAddress`, then give it write access with `WritablePointer::cast`".to_string()
                }
                (PointerAccess::Write, "toFunction") => {
                    "convert the address with `Pointer::toFunction`".to_string()
                }
                (PointerAccess::Write, "from") => {
                    "form a `WritablePointer` with `WritablePointer::to` or `WritablePointer::cast`; `Pointer::from` drops write access".to_string()
                }
                _ => format!(
                    "`{companion}` has {}",
                    pointer_intrinsics::static_members(pointer)
                        .iter()
                        .map(|member| format!("`{companion}::{}`", member.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
            self.diagnostics.push(diagnostic);
            return;
        };
        let callee = format!("`{companion}::{member_name}`");
        if self.reject_named_arguments(&callee, args) {
            return;
        }
        if let Some(operation) = member.operation.unsafe_description() {
            self.require_unsafe(access.span, operation);
        }
        if args.len() != 1 {
            self.report_argument_count_mismatch(&callee, 1, 1, args.len(), access.span);
            return;
        }
        let argument = &args[0].value;
        match member.operation {
            PointerOperation::Cast => {
                let source = self.infer_expr_type(argument, scopes, method_context);
                if self.pointer_receiver(source).is_none()
                    && !matches!(self.types.kind(source), TypeKind::Unknown)
                {
                    let got = self.types.display(source);
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0408",
                            format!("argument 1 of {callee} expects a pointer, got `{got}`"),
                            argument.span(),
                        )
                        .with_title("Argument Type Mismatch"),
                    );
                }
                self.expected_pointer_target(member.operation, pointer, &callee, access.span);
            }
            PointerOperation::FromAddress => {
                let address = self.types.intern(TypeKind::Integer(IntegerType::UInt64));
                let parameters = [self.pointer_parameter("address", address)];
                self.check_call_arguments(
                    &callee,
                    &parameters,
                    args,
                    access.span,
                    scopes,
                    method_context,
                );
                self.expected_pointer_target(member.operation, pointer, &callee, access.span);
            }
            PointerOperation::ToFunction => {
                let opaque = self.types.intern(TypeKind::Foreign(ForeignType::Opaque));
                let address = self.types.intern(TypeKind::Foreign(ForeignType::Pointer {
                    access: PointerAccess::Read,
                    pointee: opaque,
                }));
                let parameters = [self.pointer_parameter("address", address)];
                self.check_call_arguments(
                    &callee,
                    &parameters,
                    args,
                    access.span,
                    scopes,
                    method_context,
                );
                self.expected_pointer_target(member.operation, pointer, &callee, access.span);
            }
            PointerOperation::DropWriteAccess => {
                let source = self.infer_expr_type(argument, scopes, method_context);
                let mutable = matches!(
                    self.types.kind(source),
                    TypeKind::Foreign(ForeignType::Pointer {
                        access: PointerAccess::Write,
                        ..
                    }) | TypeKind::Unknown
                );
                if !mutable {
                    let got = self.types.display(source);
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0408",
                            format!(
                                "argument 1 of {callee} expects `WritablePointer<T>`, got `{got}`"
                            ),
                            argument.span(),
                        )
                        .with_title("Argument Type Mismatch")
                        .with_help("narrow a nullable pointer with `is` or `??` first"),
                    );
                }
            }
            PointerOperation::AddressOf => {
                self.check_pointer_place(pointer, argument, scopes, method_context);
            }
            PointerOperation::Read { .. }
            | PointerOperation::Write { .. }
            | PointerOperation::Offset
            | PointerOperation::ByteOffset
            | PointerOperation::Address => {
                unreachable!("instance members are not static members")
            }
        }
    }

    /// The target a call states for a conversion, which declares one type
    /// parameter (Decision 0105, Amendment: Type Arguments At The Call): the
    /// pointee for `cast` and `fromAddress`, as in `Pointer::cast<int32>`,
    /// and the C function pointer type itself for `toFunction`.
    fn stated_conversion_target(
        &mut self,
        operation: PointerOperation,
        pointer: PointerAccess,
        span: Span,
    ) -> Option<TypeId> {
        let member = match operation {
            PointerOperation::Cast => "cast",
            PointerOperation::FromAddress => "fromAddress",
            PointerOperation::ToFunction => "toFunction",
            _ => return None,
        };
        let callee = format!("`{}::{member}`", pointer.short_name());
        if operation == PointerOperation::ToFunction {
            return Some(self.stated_type_argument_types(&callee, 1, span)?[0]);
        }
        let pointee = *self
            .stated_type_argument_types_in_position(&callee, 1, span, TypePosition::Pointee)?
            .first()?;
        Some(self.pointer_type(pointer, pointee, span))
    }

    /// The target of a conversion, from a type argument stated at the call
    /// or else the expected type, or E0802 when neither is of the right
    /// kind.
    fn expected_pointer_target(
        &mut self,
        operation: PointerOperation,
        pointer: PointerAccess,
        callee: &str,
        span: Span,
    ) -> Option<TypeId> {
        if let Some(target) = self.conversion_target(operation, pointer, span) {
            return Some(target);
        }
        let wanted = match operation {
            PointerOperation::ToFunction => "a C function pointer type".to_string(),
            _ => format!("a `{}<T>` type", pointer.short_name()),
        };
        let found = self
            .stated_conversion_target(operation, pointer, span)
            .or_else(|| self.contextual_expression_types.get(&span).copied());
        let mut diagnostic = Diagnostic::new(
            POINTER_TARGET_CODE,
            match found {
                Some(found) => format!(
                    "{callee} takes its target from its type argument or the expected type, and `{}` is not {wanted}",
                    self.types.display(found)
                ),
                None => format!(
                    "{callee} takes its target from its type argument or the expected type, and nothing here states one"
                ),
            },
            span,
        )
        .with_title("Pointer Conversion Needs A Target Type");
        diagnostic = diagnostic.with_help(match operation {
            PointerOperation::ToFunction => {
                "state the target at the call or declare the result with its type, as in `extern \"C\" function(int32): int32 $callback = Pointer::toFunction($address);`".to_string()
            }
            PointerOperation::FromAddress => {
                "state the pointee at the call, as in `Pointer::fromAddress<uint8>($address)`, or declare the result with its type".to_string()
            }
            _ => format!(
                "state the pointee at the call, as in `{0}::cast<uint8>($pointer)`, or declare the result with its type",
                pointer.short_name()
            ),
        });
        self.diagnostics.push(diagnostic);
        None
    }

    /// The non-null target type the expected type at `span` names for
    /// `operation`, without reporting.
    fn conversion_target(
        &mut self,
        operation: PointerOperation,
        pointer: PointerAccess,
        span: Span,
    ) -> Option<TypeId> {
        let stated = self.stated_conversion_target(operation, pointer, span);
        let expected = stated.or_else(|| self.contextual_expression_types.get(&span).copied())?;
        let target = match self.types.kind(expected) {
            TypeKind::Nullable(inner) => *inner,
            _ => expected,
        };
        let fits = match (operation, self.types.kind(target)) {
            (PointerOperation::ToFunction, TypeKind::Foreign(ForeignType::Function(_))) => true,
            (
                PointerOperation::Cast | PointerOperation::FromAddress,
                TypeKind::Foreign(ForeignType::Pointer { access, .. }),
            ) => *access == pointer,
            _ => false,
        };
        fits.then_some(target)
    }

    /// `Pointer::to($place)` and `WritablePointer::to($place)`: a local, a foreign struct
    /// field or a property, of an FFI-safe type. Whether a `WritablePointer::to`
    /// place is writable is the ownership checker's rule, as for every write.
    fn check_pointer_place(
        &mut self,
        pointer: PointerAccess,
        place: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let companion = pointer.short_name();
        if !self.is_pointer_place(place, method_context) {
            self.diagnostics.push(
                Diagnostic::new(
                    POINTER_PLACE_CODE,
                    format!("`{companion}::to` needs a place to address, and this is a value"),
                    place.span(),
                )
                .with_title("Pointer Source Is Not A Place")
                .with_explanation(
                    "`Pointer::to` and `WritablePointer::to` address a local, a foreign struct field or a property, and keep it in place until the unsafe context ends. A computed value has no lasting storage to address.",
                )
                .with_help("store the value in a local first, then form the pointer to the local"),
            );
            return;
        }
        let ty = self.infer_expr_type(place, scopes, method_context);
        if matches!(self.types.kind(ty), TypeKind::Unknown) {
            return;
        }
        let addressable = matches!(
            self.types.kind(ty),
            TypeKind::Foreign(ForeignType::Aggregate(_))
        );
        if !addressable && !self.defer_generic_pointee(ty) {
            self.require_ffi_safe(ty, place.span(), "a pointee");
        }
    }

    fn is_pointer_place(&self, expr: &Expr, method_context: Option<&MethodContext>) -> bool {
        match expr {
            Expr::Grouped { expr, .. } | Expr::Unsafe { expr, .. } => {
                self.is_pointer_place(expr, method_context)
            }
            Expr::Variable { .. } => true,
            Expr::PropertyAccess {
                object,
                null_safe: false,
                take_span: None,
                span,
                ..
            } => {
                !self.property_accessor_calls.contains_key(span)
                    && (matches!(ungroup_place(object), Expr::This { .. })
                        || self.is_pointer_place(object, method_context))
            }
            Expr::StaticMember {
                qualifier,
                member,
                span,
                ..
            } => {
                !self.property_accessor_calls.contains_key(span)
                    && self
                        .static_qualifier_class_name(qualifier, method_context)
                        .and_then(|class| self.classes.get(&class))
                        .is_some_and(|class| class.static_properties.contains_key(member))
            }
            _ => false,
        }
    }

    /// The result of a static call on a foreign companion, without
    /// reporting; `None` for every other class.
    pub(super) fn foreign_companion_call_type(
        &mut self,
        class_name: &str,
        method: &str,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        if class_name == crate::foreign::OPAQUE {
            return Some(self.types.unknown());
        }
        if let Some(result) = self.handle_static_call_type(class_name, method, span) {
            return Some(result);
        }
        if let Some(region) = RegionCompanion::from_class_name(class_name) {
            if method != crate::byte_intrinsics::FROM_POINTER {
                return Some(self.types.unknown());
            }
            let element = match crate::byte_intrinsics::from_pointer_argument(args, 1) {
                Some((_, argument)) if region.typed => {
                    let pointer = self.infer_expr_type(&argument.value, scopes, method_context);
                    match self.pointer_receiver(pointer) {
                        Some(receiver) => self.types.resolved(receiver.pointee),
                        None => return Some(self.types.unknown()),
                    }
                }
                _ => ResolvedType::Integer(IntegerType::UInt8),
            };
            return Some(self.types.intern_resolved(&region.region_type(element)));
        }
        let companion = ForeignCompanion::from_class_name(class_name)?;
        let argument = args.first().map(|argument| &argument.value);
        let result = match companion {
            ForeignCompanion::OwnedString(encoding) => {
                match crate::foreign_strings::member(encoding, method).filter(|member| {
                    member.kind == crate::byte_intrinsics::ByteMemberKind::StaticMethod
                }) {
                    Some(member) => self.types.intern_resolved(&member.return_type),
                    None => self.types.unknown(),
                }
            }
            ForeignCompanion::Boundary(boundary) => {
                let value = self
                    .types
                    .intern(TypeKind::Foreign(ForeignType::Boundary(boundary)));
                match BoundaryConversion::from_name(method) {
                    Some(BoundaryConversion::From) => value,
                    Some(BoundaryConversion::TryFrom) => {
                        self.types.intern(TypeKind::Nullable(value))
                    }
                    None => self.types.unknown(),
                }
            }
            ForeignCompanion::Pointer(pointer) => {
                let Some(member) = pointer_intrinsics::static_member(pointer, method) else {
                    return Some(self.types.unknown());
                };
                let source =
                    argument.map(|argument| self.infer_expr_type(argument, scopes, method_context));
                match member.operation {
                    PointerOperation::Cast => {
                        match self.conversion_target(member.operation, pointer, span) {
                            Some(target)
                                if source.is_some_and(|source| {
                                    matches!(self.types.kind(source), TypeKind::Nullable(_))
                                }) =>
                            {
                                self.types.intern(TypeKind::Nullable(target))
                            }
                            Some(target) => target,
                            None => self.types.unknown(),
                        }
                    }
                    PointerOperation::FromAddress => {
                        match self.conversion_target(member.operation, pointer, span) {
                            Some(target) => self.types.intern(TypeKind::Nullable(target)),
                            None => self.types.unknown(),
                        }
                    }
                    PointerOperation::ToFunction => self
                        .conversion_target(member.operation, pointer, span)
                        .unwrap_or_else(|| self.types.unknown()),
                    PointerOperation::DropWriteAccess => {
                        match source.map(|source| self.types.kind(source).clone()) {
                            Some(TypeKind::Foreign(ForeignType::Pointer {
                                access: PointerAccess::Write,
                                pointee,
                            })) => self.types.intern(TypeKind::Foreign(ForeignType::Pointer {
                                access: PointerAccess::Read,
                                pointee,
                            })),
                            _ => self.types.unknown(),
                        }
                    }
                    PointerOperation::AddressOf => match source {
                        Some(pointee) if !matches!(self.types.kind(pointee), TypeKind::Unknown) => {
                            self.types.intern(TypeKind::Foreign(ForeignType::Pointer {
                                access: pointer,
                                pointee,
                            }))
                        }
                        _ => self.types.unknown(),
                    },
                    PointerOperation::Read { .. }
                    | PointerOperation::Write { .. }
                    | PointerOperation::Offset
                    | PointerOperation::ByteOffset
                    | PointerOperation::Address => self.types.unknown(),
                }
            }
        };
        Some(result)
    }

    /// `CLong::from($value)` and `CLong::tryFrom($value)`: any integer.
    fn check_boundary_conversion(
        &mut self,
        boundary: crate::foreign::CBoundaryType,
        access: StaticAccess<'_>,
        args: &[Argument],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let companion = boundary.short_name();
        let Some(conversion) = BoundaryConversion::from_name(access.member) else {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0304",
                    format!("unknown method `{companion}::{}`", access.member),
                    access.member_span,
                )
                .with_title("Unknown Method")
                .with_help(format!(
                    "`{companion}::from(...)` and `{companion}::tryFrom(...)` are available; a boundary value has no arithmetic, ordering or display, so convert it with a fixed-width companion such as `Int64::from(...)`"
                )),
            );
            return;
        };
        let callee = format!("`{companion}::{}`", conversion.name());
        if self.reject_named_arguments(&callee, args) {
            return;
        }
        if args.len() != 1 {
            self.diagnostics.push(Diagnostic::new(
                "E0443",
                format!("{callee} expects exactly 1 argument, got {}", args.len()),
                access.span,
            ));
            return;
        }
        let argument = &args[0].value;
        self.contextualize_integer_literals(argument, IntegerType::Int64);
        let argument_ty = self.infer_expr_type(argument, scopes, method_context);
        match self.types.kind(argument_ty) {
            TypeKind::Integer(_) | TypeKind::Unknown => {}
            TypeKind::Foreign(ForeignType::Boundary(source)) => {
                let source = source.short_name();
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0443",
                        format!("{callee} requires an integer argument, got `{source}`"),
                        argument.span(),
                    )
                    .with_help(format!(
                        "convert it to a fixed-width integer first, as in `{companion}::{}(Int64::from($value))`",
                        conversion.name()
                    )),
                );
            }
            _ => {
                let got = self.types.display(argument_ty);
                self.diagnostics.push(Diagnostic::new(
                    "E0443",
                    format!("{callee} requires an integer argument, got `{got}`"),
                    argument.span(),
                ));
            }
        }
    }
}

impl<'program> Checker<'program> {
    /// Arithmetic or ordering on a pointer or C boundary value, which have
    /// `==` and `!=` only (sections 4 and 5).
    pub(super) fn report_foreign_operand(&mut self, operand: TypeId, span: Span) -> bool {
        let base = match self.types.kind(operand) {
            TypeKind::Nullable(inner) => *inner,
            _ => operand,
        };
        let help = match self.types.kind(base) {
            TypeKind::Foreign(ForeignType::Boundary(boundary)) => format!(
                "convert it with a fixed-width companion first, as in `Int64::from($value)`; `{}` values compare only with `==` and `!=`",
                boundary.short_name()
            ),
            TypeKind::Foreign(ForeignType::Pointer { .. }) => {
                "move a pointer with `offset` or `byteOffset`, or compare the `address` values".to_string()
            }
            _ => return false,
        };
        let display = self.types.display(operand);
        self.diagnostics.push(
            Diagnostic::new(
                "E0441",
                format!("`{display}` has no arithmetic or ordering; it has `==` and `!=` only"),
                span,
            )
            .with_title("Foreign Value Has No Arithmetic Or Ordering")
            .with_help(help),
        );
        true
    }
}

fn ungroup_place(expr: &Expr) -> &Expr {
    match expr {
        Expr::Grouped { expr, .. } | Expr::Unsafe { expr, .. } => ungroup_place(expr),
        _ => expr,
    }
}
