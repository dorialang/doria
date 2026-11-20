//! Checking handles (Decision 0148 section 11).
//!
//! The members and their contracts are the table in
//! [`crate::foreign_handles`]; this module types them. `reclaim` and
//! `borrow` take their value type from the expected type, as
//! `Pointer::cast` does.

use super::*;
use crate::foreign::{ForeignType, PointerAccess};
use crate::foreign_handles::{HandleFamily, HandleOperation};

pub(super) const HANDLE_VALUE_CODE: &str = "E0807";
pub(super) const HANDLE_TARGET_CODE: &str = "E0808";
pub(super) const HANDLE_BORROW_ESCAPE_CODE: &str = "E0809";

impl<'program> Checker<'program> {
    /// `WritablePointer<Opaque>`, the type of every handle.
    fn handle_type(&mut self) -> TypeId {
        let opaque = self.types.intern(TypeKind::Foreign(ForeignType::Opaque));
        self.types.intern(TypeKind::Foreign(ForeignType::Pointer {
            access: PointerAccess::Write,
            pointee: opaque,
        }))
    }

    /// Whether `ty` is a class value a handle holds: an instance of a class
    /// stored on the heap, not an inline Copy value.
    fn is_handle_class(&self, ty: TypeId) -> bool {
        matches!(
            self.types.resolved(ty),
            ResolvedType::Class(class) if class.inline_copy_type().is_none()
        )
    }

    /// A static call on `Handle`, `SharedReference` or
    /// `WritableSharedReference` that names a handle operation, or any
    /// static call on `Handle`. Returns false otherwise, so the shared
    /// references' other static members check as before.
    pub(super) fn check_handle_static_call(
        &mut self,
        class_name: &str,
        access: StaticAccess<'_>,
        args: &[Argument],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let Some((family, operation)) =
            crate::foreign_handles::static_operation(class_name, access.member)
        else {
            if class_name != crate::foreign_handles::HANDLE {
                return false;
            }
            let members = crate::foreign_handles::static_members(HandleFamily::Owned)
                .iter()
                .map(|member| format!("`Handle::{member}`"))
                .collect::<Vec<_>>()
                .join(", ");
            self.diagnostics.push(
                Diagnostic::new(
                    "E0304",
                    format!("unknown method `Handle::{}`", access.member),
                    access.member_span,
                )
                .with_title("Unknown Method")
                .with_help(format!("`Handle` has {members}")),
            );
            return true;
        };
        let callee = format!("`{}::{}`", family.companion(), operation.name(family));
        self.require_unsafe(access.span, operation.unsafe_description());
        if args.len() != 1 {
            self.report_argument_count_mismatch(&callee, 1, 1, args.len(), access.span);
            return true;
        }
        let parameter = match operation {
            HandleOperation::Create => {
                let value = self.infer_expr_type(&args[0].value, scopes, method_context);
                if !self.is_handle_class(value)
                    && !matches!(self.types.kind(value), TypeKind::Unknown)
                {
                    let got = self.types.display(value);
                    self.diagnostics.push(
                        Diagnostic::new(
                            HANDLE_VALUE_CODE,
                            format!("{callee} hands an owned class value to C, and `{got}` is not a class"),
                            args[0].value.span(),
                        )
                        .with_title("Handle Holds A Class Value")
                        .with_explanation(
                            "A handle holds an instance of a class for C to keep and pass back. Values of other types reach C as FFI-safe values or through a pointer.",
                        )
                        .with_help("pass a class instance, or wrap the value in a class"),
                    );
                }
                ParamInfo {
                    name: "value".to_string(),
                    ty: value,
                    take: true,
                    writable: false,
                    has_default: false,
                    default: None,
                    declaration: None,
                }
            }
            HandleOperation::Reclaim | HandleOperation::Borrow => {
                self.handle_target(family, operation, &callee, access.span);
                let handle = self.handle_type();
                ParamInfo {
                    name: "handle".to_string(),
                    ty: handle,
                    take: false,
                    writable: false,
                    has_default: false,
                    default: None,
                    declaration: None,
                }
            }
        };
        self.check_call_arguments(
            &callee,
            &[parameter],
            args,
            access.span,
            scopes,
            method_context,
        );
        true
    }

    /// The value type `reclaim` or `borrow` on `family` gives at `span`, from
    /// the expected type, without reporting: a class for `Handle`, and a
    /// reference of the family's kind for the shared references.
    fn handle_target_type(&self, family: HandleFamily, span: Span) -> Option<TypeId> {
        let expected = self.contextual_expression_types.get(&span).copied()?;
        let fits = match (family, self.types.kind(expected)) {
            (HandleFamily::Owned, _) => self.is_handle_class(expected),
            (HandleFamily::Shared(kind), TypeKind::SharedHandle(target, _)) => *target == kind,
            _ => false,
        };
        fits.then_some(expected)
    }

    /// The target of `reclaim` or `borrow`, or E0808 when the position
    /// states none of the family's kind.
    fn handle_target(
        &mut self,
        family: HandleFamily,
        operation: HandleOperation,
        callee: &str,
        span: Span,
    ) -> Option<TypeId> {
        if let Some(target) = self.handle_target_type(family, span) {
            return Some(target);
        }
        let wanted = match family {
            HandleFamily::Owned => "a class type".to_string(),
            HandleFamily::Shared(kind) => format!("a `{}<T>` type", kind.source_name()),
        };
        let message = match self.contextual_expression_types.get(&span).copied() {
            Some(expected) => format!(
                "{callee} takes its value type from the expected type, and `{}` is not {wanted}",
                self.types.display(expected)
            ),
            None => format!(
                "{callee} takes its value type from the expected type, and nothing here states one"
            ),
        };
        let example = match family {
            HandleFamily::Owned => format!(
                "Counter $counter = Handle::{}($handle);",
                operation.name(family)
            ),
            HandleFamily::Shared(kind) => format!(
                "{0}<Counter> $counter = {0}::{1}($handle);",
                kind.source_name(),
                operation.name(family)
            ),
        };
        self.diagnostics.push(
            Diagnostic::new(HANDLE_TARGET_CODE, message, span)
                .with_title("Handle Needs A Value Type")
                .with_explanation(
                    "A handle is an untyped `WritablePointer<Opaque>`, so the type of the value it holds comes from where the result goes. Reclaiming or borrowing it as another type is undefined behavior.",
                )
                .with_help(format!("declare the result with its type, as in `{example}`")),
        );
        None
    }

    /// The result of a handle static call, without reporting; `None` when
    /// the call names no handle operation.
    pub(super) fn handle_static_call_type(
        &mut self,
        class_name: &str,
        method: &str,
        span: Span,
    ) -> Option<TypeId> {
        let Some((family, operation)) =
            crate::foreign_handles::static_operation(class_name, method)
        else {
            return (class_name == crate::foreign_handles::HANDLE).then(|| self.types.unknown());
        };
        Some(match operation {
            HandleOperation::Create => self.handle_type(),
            HandleOperation::Reclaim | HandleOperation::Borrow => self
                .handle_target_type(family, span)
                .unwrap_or_else(|| self.types.unknown()),
        })
    }

    /// `$reference->toHandle()` on a `SharedReference<T>` or a
    /// `WritableSharedReference<T>`. Returns false for every other call.
    pub(super) fn check_handle_method_call(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Argument],
        null_safe: bool,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        if null_safe || method != crate::foreign_handles::TO_HANDLE {
            return false;
        }
        let object_ty = self.infer_expr_type(object, scopes, method_context);
        let Some((kind, _)) = self.shared_handle_type(object_ty, false) else {
            return false;
        };
        let Some(family) = crate::foreign_handles::instance_operation(kind, method) else {
            return false;
        };
        self.check_expr(object, scopes, method_context);
        self.require_unsafe(span, HandleOperation::Create.unsafe_description());
        if !args.is_empty() {
            for argument in args {
                self.check_expr(&argument.value, scopes, method_context);
            }
            self.report_argument_count_mismatch(
                &format!("`{}::toHandle`", family.companion()),
                0,
                0,
                args.len(),
                span,
            );
        }
        true
    }

    /// The result of `$reference->toHandle()`, without reporting.
    pub(super) fn handle_method_type(
        &mut self,
        object_ty: TypeId,
        method: &str,
        null_safe: bool,
    ) -> Option<TypeId> {
        if null_safe {
            return None;
        }
        let (kind, _) = self.shared_handle_type(object_ty, false)?;
        crate::foreign_handles::instance_operation(kind, method)?;
        Some(self.handle_type())
    }

    /// A value `borrow` lends lives for its enclosing unsafe context, so it
    /// cannot be the whole value of an `unsafe(<expression>)`, which ends
    /// before the value is used (section 11).
    pub(super) fn reject_escaping_handle_borrow(&mut self, inner: &Expr) {
        let mut expression = inner;
        while let Expr::Grouped { expr, .. } = expression {
            expression = expr;
        }
        let Expr::StaticCall {
            qualifier,
            method,
            span,
            ..
        } = expression
        else {
            return;
        };
        let Some((family, HandleOperation::Borrow)) =
            crate::foreign_handles::static_call(qualifier, method)
        else {
            return;
        };
        let companion = family.companion();
        self.diagnostics.push(
            Diagnostic::new(
                HANDLE_BORROW_ESCAPE_CODE,
                format!("`{companion}::borrow` lends its value for its unsafe context, and this `unsafe(...)` ends before the value is used"),
                *span,
            )
            .with_title("Borrowed Handle Value Leaves Its Unsafe Context")
            .with_explanation(
                "A handle's value belongs to C while it holds the handle, so a borrow of it lasts only for the unsafe context it is formed in.",
            )
            .with_help(format!(
                "use the value inside the context, as in `unsafe({companion}::borrow($handle)->count)`, or bind it inside an `unsafe {{ ... }}` block"
            )),
        );
    }
}

impl<'program> Checker<'program> {
    /// Records the expected type of each argument of a compiler-known
    /// static call whose parameters have fixed types: the handle of
    /// `reclaim` and `borrow`, and the pointer of a byte region's and the
    /// length of every region's `fromPointer` (section 10), so a conversion
    /// such as `WritablePointer::cast` takes its target from the parameter.
    /// Returns false for every other call.
    pub(super) fn record_foreign_companion_argument_types(
        &mut self,
        class_name: &str,
        method: &str,
        args: &[Argument],
    ) -> bool {
        if let Some((_, HandleOperation::Reclaim | HandleOperation::Borrow)) =
            crate::foreign_handles::static_operation(class_name, method)
        {
            if let Some(argument) = args.first() {
                let handle = self.handle_type();
                self.record_expected_expression_type(&argument.value, handle);
            }
            return true;
        }
        let Some(region) = crate::byte_intrinsics::RegionCompanion::from_class_name(class_name)
        else {
            return false;
        };
        if method != crate::byte_intrinsics::FROM_POINTER {
            return true;
        }
        if !region.typed {
            if let Some((_, pointer)) = crate::byte_intrinsics::from_pointer_argument(args, 1) {
                let byte = self.types.intern(TypeKind::Integer(IntegerType::UInt8));
                let ty = self.types.intern(TypeKind::Foreign(ForeignType::Pointer {
                    access: region.pointer_access(),
                    pointee: byte,
                }));
                self.record_expected_expression_type(&pointer.value, ty);
            }
        }
        if let Some((_, length)) = crate::byte_intrinsics::from_pointer_argument(args, 2) {
            let int = self.types.intern(TypeKind::Integer(IntegerType::Int64));
            self.record_expected_expression_type(&length.value, int);
        }
        true
    }
}
