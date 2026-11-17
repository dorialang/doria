//! Checking unsafe code and foreign declarations (Decision 0148).
//!
//! An unsafe context is lexical: an `unsafe { }` block or an
//! `unsafe(<expression>)`, never extended into a closure body. Every
//! operation that needs one asks [`Checker::require_unsafe`], which marks the
//! enclosing context used or reports E0785.

use std::collections::{HashMap, HashSet};

use super::*;
use crate::attributes::{AttributeTarget, AttributeValueKind};
use crate::foreign::{
    CBoundaryType, CFunctionType, ForeignBinding, ForeignType, PointerAccess, SystemCodeSource,
};

pub(super) const UNSAFE_OPERATION_CODE: &str = "E0785";
pub(super) const NOT_FFI_SAFE_CODE: &str = "E0786";
pub(super) const FOREIGN_TYPE_PLACEMENT_CODE: &str = "E0787";
pub(super) const UNSUPPORTED_ABI_CODE: &str = "E0788";
pub(super) const FOREIGN_SIGNATURE_CODE: &str = "E0789";
pub(super) const CONFLICTING_SYMBOL_CODE: &str = "E0790";
pub(super) const LINK_NAME_REQUIRED_CODE: &str = "E0791";
pub(super) const RESERVED_RUNTIME_SYMBOL_CODE: &str = "E0792";
pub(super) const VARIADIC_ARGUMENT_CODE: &str = "E0793";
pub(super) const UNSAFE_OVERRIDE_CODE: &str = "E0794";
pub(super) const FOREIGN_ATTRIBUTE_TARGET_CODE: &str = "E0795";
pub(super) const MISSING_SAFETY_CODE: &str = "E0796";
pub(super) const UNUSED_UNSAFE_CODE: &str = "E0797";

#[derive(Debug)]
struct UnsafeContext {
    keyword: Span,
    /// Written inside another unsafe context, so it adds nothing.
    redundant: bool,
    used: bool,
}

/// Facts about foreign declarations and the unsafe contexts being checked.
#[derive(Debug, Default)]
pub(super) struct ForeignState {
    /// Innermost last. `None` is a closure body, which no unsafe context
    /// written around it reaches.
    contexts: Vec<Option<UnsafeContext>>,
    /// Foreign structs and unions by canonical name.
    pub(super) aggregates: HashMap<String, super::foreign_aggregates::ForeignAggregateInfo>,
    /// Declarations every call of which needs an unsafe context: `unsafe
    /// function`s, unsafe interface requirements and extern declarations.
    unsafe_callables: HashSet<Span>,
    /// The binding of each extern declaration, by declaration span.
    pub(super) bindings: HashMap<Span, ForeignBinding>,
}

impl<'program> Checker<'program> {
    pub(super) fn enter_unsafe_context(&mut self, keyword: Span) {
        let redundant = matches!(self.foreign.contexts.last(), Some(Some(_)));
        self.foreign.contexts.push(Some(UnsafeContext {
            keyword,
            redundant,
            used: false,
        }));
    }

    pub(super) fn leave_unsafe_context(&mut self) {
        let Some(Some(context)) = self.foreign.contexts.pop() else {
            unreachable!("unsafe contexts are entered and left in pairs");
        };
        if context.redundant {
            self.diagnostics.push(
                Diagnostic::new(
                    UNUSED_UNSAFE_CODE,
                    "this `unsafe` is inside another unsafe context, which already permits everything in it",
                    context.keyword,
                )
                .with_title("Unused Unsafe Context")
                .with_severity(crate::diagnostics::DiagnosticSeverity::Warning)
                .with_help("remove the inner `unsafe`"),
            );
        } else if !context.used {
            self.diagnostics.push(
                Diagnostic::new(
                    UNUSED_UNSAFE_CODE,
                    "nothing in this unsafe context needs one",
                    context.keyword,
                )
                .with_title("Unused Unsafe Context")
                .with_severity(crate::diagnostics::DiagnosticSeverity::Warning)
                .with_help(
                    "remove `unsafe`; keep unsafe contexts around the operations that need them",
                ),
            );
        }
    }

    pub(super) fn enter_closure_barrier(&mut self) {
        self.foreign.contexts.push(None);
    }

    pub(super) fn leave_closure_barrier(&mut self) {
        let barrier = self.foreign.contexts.pop();
        debug_assert!(matches!(barrier, Some(None)), "closure barriers pair");
    }

    fn in_unsafe_context(&self) -> bool {
        matches!(self.foreign.contexts.last(), Some(Some(_)))
    }

    /// Records an operation that needs an unsafe context, reporting E0785
    /// when none encloses it. `operation` completes "… needs an unsafe
    /// context", as in "calling the extern function `deflate`".
    pub(super) fn require_unsafe(&mut self, span: Span, operation: &str) -> bool {
        if self.in_unsafe_context() {
            for context in self.foreign.contexts.iter_mut().rev() {
                match context {
                    Some(context) => context.used = true,
                    None => break,
                }
            }
            return true;
        }
        let inside_closure = matches!(self.foreign.contexts.last(), Some(None))
            && self
                .foreign
                .contexts
                .iter()
                .any(|context| context.is_some());
        let mut diagnostic = Diagnostic::new(
            UNSAFE_OPERATION_CODE,
            format!("{operation} needs an unsafe context"),
            span,
        )
        .with_title("Unsafe Operation Outside An Unsafe Context")
        .with_help(
            "write the operation inside `unsafe { ... }` or `unsafe(<expression>)`, after checking the obligations its Safety section states",
        );
        if inside_closure {
            diagnostic = diagnostic.with_note(
                "an unsafe context does not extend into a closure body written inside it",
            );
        }
        self.diagnostics.push(diagnostic);
        false
    }

    /// A call of `declaration`: an unsafe callable needs an unsafe context.
    pub(super) fn require_unsafe_callable(&mut self, span: Span, declaration: Span, name: &str) {
        if self.foreign.unsafe_callables.contains(&declaration) {
            self.require_unsafe(span, &format!("calling `{name}`"));
        }
    }

    pub(super) fn is_unsafe_callable(&self, declaration: Span) -> bool {
        self.foreign.unsafe_callables.contains(&declaration)
    }

    /// Every declaration whose calls need an unsafe context.
    pub(super) fn collect_unsafe_callables(&mut self) {
        let program = self.program;
        for item in &program.items {
            match item {
                Item::Function(function) => {
                    if function.unsafe_span.is_some() || function.linkage.is_foreign() {
                        self.foreign.unsafe_callables.insert(function.span);
                    }
                }
                Item::Class(class) => self.collect_unsafe_members(&class.members),
                Item::Trait(declaration) => self.collect_unsafe_members(&declaration.members),
                Item::Interface(declaration) => {
                    for requirement in &declaration.requirements {
                        if requirement.unsafe_span.is_some() {
                            self.foreign.unsafe_callables.insert(requirement.span);
                        }
                    }
                }
                Item::Enum(_)
                | Item::Constant(_)
                | Item::ExternBlock(_)
                | Item::ForeignAggregate(_)
                | Item::Statement(_) => {}
            }
        }
        if let CallableBodyProgram::Expanded(expanded) = self.callable_body_program {
            for item in &expanded.items {
                if let Item::Class(class) = item {
                    self.collect_unsafe_members(&class.members);
                }
            }
        }
    }

    fn collect_unsafe_members(&mut self, members: &[ClassMember]) {
        for member in members {
            if let ClassMember::Method(method) = member {
                if method.unsafe_span.is_some() {
                    self.foreign.unsafe_callables.insert(method.span);
                }
            }
        }
    }

    /// Resolves a `Doria\Std\Ffi` type, a C function pointer type or a
    /// fixed-length array, or returns `None` for every other type.
    pub(super) fn resolve_foreign_type_ref(
        &mut self,
        ty: &TypeRef,
        span: Span,
        position: TypePosition,
        declaring_class: Option<&str>,
    ) -> Option<TypeId> {
        if let Some(function) = ty
            .function
            .as_ref()
            .filter(|function| function.c_abi.is_some())
        {
            let resolved = self.resolve_c_function_type_ref(function, declaring_class);
            return Some(if ty.nullable {
                self.types.intern(TypeKind::Nullable(resolved))
            } else {
                resolved
            });
        }
        if let Some((element, count)) = ty.fixed_array_parts() {
            let element_type = self.resolve_type_ref_in_position(
                element,
                span,
                TypePosition::Value,
                declaring_class,
            );
            if position != TypePosition::ForeignField {
                return Some(self.report_foreign_type_placement(
                    span,
                    format!("`{ty}` is a fixed-length array, which only a foreign struct or union field can have"),
                    "use a typed array `T[]` or an `ArrayRegion<T>` here",
                ));
            }
            let length = count.parse::<u64>().ok().filter(|length| *length > 0);
            let Some(length) = length else {
                return Some(self.report_foreign_type_placement(
                    span,
                    format!("`{ty}` needs an element count of at least one"),
                    "write the element count of the C array, as in `uint16[128]`",
                ));
            };
            return Some(
                self.types
                    .intern(TypeKind::Foreign(ForeignType::FixedArray {
                        element: element_type,
                        length,
                    })),
            );
        }
        if let Some(aggregate) = self.resolve_foreign_aggregate_type(ty, span, position) {
            return Some(aggregate);
        }
        let pointer = PointerAccess::from_source_name(&ty.name);
        let boundary = CBoundaryType::from_source_name(&ty.name);
        let opaque = ty.name == crate::foreign::OPAQUE;
        if pointer.is_none() && boundary.is_none() && !opaque {
            return None;
        }
        let base = if let Some(access) = pointer {
            if !self.expect_type_arg_count(ty, 1, span) {
                return Some(self.types.unknown());
            }
            let pointee = self.resolve_type_ref_in_position(
                ty.type_argument(0).expect("one type argument was checked"),
                span,
                TypePosition::Pointee,
                declaring_class,
            );
            let addressable = matches!(
                self.types.kind(pointee),
                TypeKind::Foreign(ForeignType::Opaque | ForeignType::Aggregate(_))
            );
            if !addressable {
                self.require_ffi_safe(pointee, span, "a pointee");
            }
            self.types
                .intern(TypeKind::Foreign(ForeignType::Pointer { access, pointee }))
        } else if let Some(boundary) = boundary {
            self.resolve_zero_arg_type(ty, span, TypeKind::Foreign(ForeignType::Boundary(boundary)))
        } else {
            if position != TypePosition::Pointee {
                return Some(self.report_foreign_type_placement(
                    span,
                    "`Opaque` has no values; it names the untyped pointee of `Ptr<Opaque>` and `MutPtr<Opaque>`",
                    "write `Ptr<Opaque>` for C's `const void *` or `MutPtr<Opaque>` for `void *`",
                ));
            }
            self.resolve_zero_arg_type(ty, span, TypeKind::Foreign(ForeignType::Opaque))
        };
        Some(if ty.nullable {
            self.types.intern(TypeKind::Nullable(base))
        } else {
            base
        })
    }

    fn report_foreign_type_placement(
        &mut self,
        span: Span,
        message: impl Into<String>,
        help: &str,
    ) -> TypeId {
        self.diagnostics.push(
            Diagnostic::new(FOREIGN_TYPE_PLACEMENT_CODE, message, span)
                .with_title("Foreign Type Is Not Valid Here")
                .with_help(help),
        );
        self.types.unknown()
    }

    fn resolve_c_function_type_ref(
        &mut self,
        function: &FunctionTypeRef,
        declaring_class: Option<&str>,
    ) -> TypeId {
        let abi = function
            .c_abi
            .as_ref()
            .expect("a C function type has an ABI");
        self.check_foreign_abi(&abi.abi.value, abi.abi.span);
        if let Some(span) = function.invocation_modifier_span {
            self.diagnostics.push(
                Diagnostic::new(
                    FOREIGN_SIGNATURE_CODE,
                    "a C function pointer type has no invocation mode",
                    span,
                )
                .with_title("C Function Pointer Type Has No Invocation Mode"),
            );
        }
        if let Some(clause) = &function.throws_clause {
            self.diagnostics.push(
                Diagnostic::new(
                    FOREIGN_SIGNATURE_CODE,
                    "a C function cannot throw a checked error",
                    clause.span,
                )
                .with_title("C Function Cannot Throw")
                .with_help("remove the `throws` clause"),
            );
        }
        let mut parameters = Vec::new();
        for parameter in &function.parameters {
            if let Some(span) = parameter.ownership_modifier_span {
                self.diagnostics.push(
                    Diagnostic::new(
                        FOREIGN_SIGNATURE_CODE,
                        "a C function's parameters take their values by copy and have no `writable` or `take` mode",
                        span,
                    )
                    .with_title("C Function Parameter Has A Mode"),
                );
            }
            let ty = self.resolve_type_ref_in_position(
                &parameter.ty,
                parameter.type_span,
                TypePosition::Value,
                declaring_class,
            );
            self.require_ffi_safe(ty, parameter.type_span, "a C function pointer parameter");
            parameters.push(ty);
        }
        let return_type = self.resolve_type_ref_in_position(
            &function.return_type,
            function.return_type_span,
            TypePosition::Return,
            declaring_class,
        );
        if !matches!(self.types.kind(return_type), TypeKind::Void) {
            self.require_ffi_safe(
                return_type,
                function.return_type_span,
                "a C function pointer return",
            );
        }
        self.types
            .intern(TypeKind::Foreign(ForeignType::Function(CFunctionType {
                parameters,
                return_type,
            })))
    }

    pub(super) fn check_foreign_abi(&mut self, abi: &str, span: Span) {
        if abi != "C" {
            self.diagnostics.push(
                Diagnostic::new(
                    UNSUPPORTED_ABI_CODE,
                    format!("`extern \"{abi}\"` names an ABI Doria does not have; `\"C\"` is the only one"),
                    span,
                )
                .with_title("Unknown Foreign ABI")
                .with_explanation(
                    "`extern \"C\"` means the target's C calling convention on every tier-1 platform.",
                )
                .with_fix(span, "\"C\""),
            );
        }
    }

    /// Whether a value of `ty` crosses the C boundary unchanged (section 6).
    pub(super) fn is_ffi_safe(&self, ty: TypeId) -> bool {
        match self.types.kind(ty) {
            TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Bool => true,
            TypeKind::Foreign(foreign) => match foreign {
                ForeignType::Pointer { .. }
                | ForeignType::Boundary(_)
                | ForeignType::Function(_)
                | ForeignType::Aggregate(_) => true,
                ForeignType::Opaque | ForeignType::FixedArray { .. } => false,
            },
            TypeKind::Nullable(inner) => matches!(
                self.types.kind(*inner),
                TypeKind::Foreign(ForeignType::Pointer { .. } | ForeignType::Function(_))
            ),
            _ => false,
        }
    }

    /// Reports E0786 when `ty` is not FFI-safe, naming the type to use.
    pub(super) fn require_ffi_safe(&mut self, ty: TypeId, span: Span, place: &str) -> bool {
        if self.is_ffi_safe(ty) || matches!(self.types.kind(ty), TypeKind::Unknown) {
            return true;
        }
        let display = self.types.display(ty);
        let help = self.ffi_safe_replacement(ty);
        self.diagnostics.push(
            Diagnostic::new(
                NOT_FFI_SAFE_CODE,
                format!("`{display}` cannot cross the C boundary as {place}"),
                span,
            )
            .with_title("Type Cannot Cross The C Boundary")
            .with_explanation(
                "A foreign signature, foreign aggregate field or C function pointer type uses only fixed-width integers and floats, `bool`, pointers, the `Doria\\Std\\Ffi` boundary types, C function pointer types and foreign structs and unions.",
            )
            .with_help(help),
        );
        false
    }

    fn ffi_safe_replacement(&self, ty: TypeId) -> String {
        match self.types.kind(ty) {
            TypeKind::String => "pass `Ptr<CChar>` from a `CString`, or `$text->byteRegion->pointer` with its length for a pointer-and-length API".to_string(),
            TypeKind::Bytes | TypeKind::ByteRegion(_) => "pass `$region->pointer` (`Ptr<uint8>`) with `$region->length`".to_string(),
            TypeKind::TypedArray(element) => format!(
                "pass a region pointer, `Ptr<{}>`, with its length",
                self.types.display(*element)
            ),
            TypeKind::Class(_) | TypeKind::Interface(_) | TypeKind::SharedHandle(..) => {
                "pass a raw handle, `MutPtr<Opaque>` from `Handle::intoRaw`".to_string()
            }
            TypeKind::Function(_) => {
                "pass a C-ABI function, typed `extern \"C\" function(...)`".to_string()
            }
            TypeKind::Enum(_) => "pass the enum's backing integer".to_string(),
            TypeKind::Nullable(_) => {
                "only pointers and C function pointers are nullable at the C boundary; pass a sentinel value or a pointer".to_string()
            }
            TypeKind::Foreign(ForeignType::Opaque) => {
                "write `Ptr<Opaque>` or `MutPtr<Opaque>`".to_string()
            }
            _ => "use a fixed-width integer or float, `bool`, a pointer, a boundary type or a foreign struct".to_string(),
        }
    }

    /// Extern declarations and C-ABI functions: their signatures, symbols
    /// and Safety sections. Runs after attributes are bound.
    pub(super) fn check_foreign_declarations(&mut self) {
        let program = self.program;
        let mut blocks = HashMap::new();
        for item in &program.items {
            match item {
                Item::ExternBlock(block) => {
                    self.check_foreign_abi(&block.abi.value, block.abi.span);
                    blocks.insert(block.span, block);
                }
                Item::ForeignAggregate(aggregate) => {
                    self.check_foreign_abi(&aggregate.abi.value, aggregate.abi.span);
                }
                _ => {}
            }
        }
        let link_names = self.foreign_link_names();
        let libraries = self.foreign_string_arguments(crate::compiler_known_ffi::LIBRARY);
        let fixed_parameters = self.variadic_fixed_parameters();
        let system_codes = self
            .foreign_attribute_arguments(crate::compiler_known_ffi::CAPTURE_SYSTEM_CODE)
            .into_iter()
            .filter_map(|(span, value)| match value {
                AttributeValueKind::Enum { case } => {
                    SystemCodeSource::from_case_name(&case).map(|source| (span, source))
                }
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        let mut symbols: HashMap<String, Vec<(&FunctionDecl, bool)>> = HashMap::new();
        for item in &program.items {
            let Item::Function(function) = item else {
                continue;
            };
            match &function.linkage {
                FunctionLinkage::Foreign { block: block_span } => {
                    let block = blocks.get(block_span).copied();
                    self.check_foreign_signature(function, "an extern declaration");
                    let link_name = link_names.get(&function.span).cloned();
                    let symbol = link_name.clone().unwrap_or_else(|| {
                        function
                            .name
                            .rsplit('\\')
                            .next()
                            .unwrap_or(&function.name)
                            .to_string()
                    });
                    self.check_foreign_symbol(function, &symbol, link_name.is_some());
                    self.foreign.bindings.insert(
                        function.span,
                        ForeignBinding {
                            symbol: symbol.clone(),
                            library: libraries.get(block_span).cloned(),
                            fixed_parameters: fixed_parameters
                                .get(&function.span)
                                .and_then(|count| usize::try_from(*count).ok()),
                            system_code: system_codes.get(&function.span).copied(),
                        },
                    );
                    symbols
                        .entry(symbol)
                        .or_default()
                        .push((function, link_name.is_some()));
                    let shared = block
                        .and_then(|block| block.documentation.as_ref())
                        .and_then(|comment| comment.safety.as_ref())
                        .is_some_and(|section| !section.text.is_empty());
                    self.check_safety_section(function, shared, "extern declaration");
                }
                FunctionLinkage::CAbi { abi, .. } => {
                    self.check_foreign_abi(&abi.value, abi.span);
                    self.check_foreign_signature(function, "a C-ABI function");
                    if let Some(span) = function.unsafe_span {
                        self.diagnostics.push(
                            Diagnostic::new(
                                FOREIGN_SIGNATURE_CODE,
                                "a C-ABI function is called by C through its address; Doria code does not call it, so it takes no `unsafe` modifier",
                                span,
                            )
                            .with_title("C-ABI Function Cannot Be Unsafe"),
                        );
                    }
                }
                FunctionLinkage::Doria => {
                    if function.unsafe_span.is_some() {
                        self.check_safety_section(function, false, "unsafe function");
                    }
                }
            }
        }
        self.check_foreign_symbol_conflicts(symbols);
        self.check_unsafe_members();
        self.check_foreign_attribute_targets();
    }

    fn check_unsafe_members(&mut self) {
        let program = self.program;
        for item in &program.items {
            let (owner, members): (&str, Vec<&FunctionDecl>) = match item {
                Item::Class(class) => (
                    &class.name,
                    class
                        .members
                        .iter()
                        .filter_map(|member| match member {
                            ClassMember::Method(method) => Some(method),
                            _ => None,
                        })
                        .collect(),
                ),
                Item::Trait(declaration) => (
                    &declaration.name,
                    declaration
                        .members
                        .iter()
                        .filter_map(|member| match member {
                            ClassMember::Method(method) => Some(method),
                            _ => None,
                        })
                        .collect(),
                ),
                Item::Interface(declaration) => {
                    (&declaration.name, declaration.requirements.iter().collect())
                }
                _ => continue,
            };
            for method in members {
                if method.unsafe_span.is_some() {
                    self.check_safety_section(method, false, "unsafe method");
                }
                if let Some(unsafe_span) = method.unsafe_span {
                    if let Some(parent) = self.overridden_safe_method(owner, method) {
                        self.diagnostics.push(
                            Diagnostic::new(
                                UNSAFE_OVERRIDE_CODE,
                                format!(
                                    "`{}` overrides or implements a safe method, so it cannot add `unsafe`",
                                    method.name
                                ),
                                unsafe_span,
                            )
                            .with_title("Override Cannot Add Unsafe")
                            .with_related(parent, "the safe method is declared here")
                            .with_explanation(
                                "Callers of the parent or interface call it without an unsafe context, so every implementation must be safe to call that way.",
                            )
                            .with_help("make the implementation safe and keep its unsafe operations in `unsafe { ... }` blocks inside it"),
                        );
                    }
                }
            }
        }
    }

    /// The declaration of a safe method that `method` of `owner` overrides
    /// or implements, if any.
    fn overridden_safe_method(&self, owner: &str, method: &FunctionDecl) -> Option<Span> {
        let mut candidates = Vec::new();
        if let Some(class) = self.classes.get(owner) {
            let mut parent = class.parent.as_ref().map(|parent| parent.name.clone());
            while let Some(name) = parent {
                let Some(info) = self.classes.get(&name) else {
                    break;
                };
                if let Some(inherited) = info.methods.get(&method.name) {
                    candidates.push(inherited.declaration);
                    break;
                }
                parent = info.parent.as_ref().map(|parent| parent.name.clone());
            }
        }
        for item in &self.program.items {
            let Item::Class(class) = item else {
                continue;
            };
            if class.name != owner {
                continue;
            }
            for conformance in &class.implements {
                let Some(interface) = self.interface_declaration(&conformance.ty.name) else {
                    continue;
                };
                candidates.extend(
                    interface
                        .requirements
                        .iter()
                        .filter(|requirement| requirement.name == method.name)
                        .map(|requirement| requirement.span),
                );
            }
        }
        candidates
            .into_iter()
            .find(|declaration| !self.foreign.unsafe_callables.contains(declaration))
    }

    fn check_safety_section(&mut self, function: &FunctionDecl, shared: bool, what: &str) {
        let written = function
            .documentation
            .as_ref()
            .and_then(|comment| comment.safety.as_ref())
            .is_some_and(|section| !section.text.is_empty());
        if written || shared {
            return;
        }
        if function.span.source.is_compiler_owned() {
            return;
        }
        self.diagnostics.push(
            Diagnostic::new(
                MISSING_SAFETY_CODE,
                format!(
                    "{what} `{}` does not state what its callers must uphold",
                    function.name.rsplit('\\').next().unwrap_or(&function.name)
                ),
                function.name_span,
            )
            .with_title("Missing Safety Section")
            .with_severity(crate::diagnostics::DiagnosticSeverity::Warning)
            .with_help(
                "write `/** @safety ... */` before the declaration with the conditions a caller checks before calling it; an editor shows it at every call",
            ),
        );
    }

    /// Shape rules shared by extern declarations and C-ABI functions.
    fn check_foreign_signature(&mut self, function: &FunctionDecl, what: &str) {
        if let Some(parameters) = &function.syntax.type_parameters {
            self.diagnostics.push(
                Diagnostic::new(
                    FOREIGN_SIGNATURE_CODE,
                    format!("{what} has no type parameters"),
                    parameters.open_span.merge(parameters.close_span),
                )
                .with_title("Foreign Function Cannot Be Generic"),
            );
        }
        if let Some(throws) = &function.throws {
            self.diagnostics.push(
                Diagnostic::new(
                    FOREIGN_SIGNATURE_CODE,
                    format!("{what} cannot throw a checked error"),
                    throws.span,
                )
                .with_title("C Function Cannot Throw")
                .with_help(if function.linkage.is_c_abi() {
                    "handle every checked error inside the body and remove the `throws` clause"
                } else {
                    "remove the `throws` clause; report a foreign failure through the return value"
                }),
            );
        }
        let Some(signature) = self.function_signatures.get(&function.span).cloned() else {
            return;
        };
        for (parameter, info) in function.params.iter().zip(&signature.params) {
            if let Some(span) = parameter.take_span.or(parameter.writable_span) {
                self.diagnostics.push(
                    Diagnostic::new(
                        FOREIGN_SIGNATURE_CODE,
                        format!("a parameter of {what} takes its value by copy and has no `writable` or `take` mode"),
                        span,
                    )
                    .with_title("Foreign Parameter Has A Mode")
                    .with_help("pass a `MutPtr<T>` for a value C writes through")
                    .with_fix(
                        Span::in_source(span.source, span.start, parameter.type_span.start),
                        "",
                    ),
                );
            }
            self.require_ffi_safe(info.ty, parameter.type_span, "a parameter");
        }
        if let (Some(return_type), Some(span)) =
            (&function.return_type, function.syntax.return_type_span)
        {
            if return_type.name != "void" || return_type.function.is_some() {
                self.require_ffi_safe(signature.return_ty, span, "a return value");
            }
        }
    }

    /// The `#[LinkName]` symbol of each foreign declaration that has one.
    fn foreign_link_names(&self) -> HashMap<Span, String> {
        self.foreign_string_arguments(crate::compiler_known_ffi::LINK_NAME)
    }

    /// The string argument of every application of the attribute class
    /// `class`, keyed by the declaration or block span it applies to.
    fn foreign_string_arguments(&self, class: &str) -> HashMap<Span, String> {
        self.foreign_attribute_arguments(class)
            .into_iter()
            .filter_map(|(span, value)| match value {
                AttributeValueKind::String(symbol) => Some((span, symbol)),
                _ => None,
            })
            .collect()
    }

    /// The first argument of every application of the attribute class
    /// `class` to a foreign declaration, keyed by the declaration's span.
    pub(super) fn foreign_attribute_arguments(
        &self,
        class: &str,
    ) -> Vec<(Span, AttributeValueKind)> {
        let mut values = Vec::new();
        for application in &self.attributes.applications {
            if application.canonical_class_name != class {
                continue;
            }
            let Some(declaration) = self.attribute_declaration_span(&application.target) else {
                continue;
            };
            if let Some(argument) = application.bound_arguments.first() {
                values.push((declaration, argument.value.value.clone()));
            }
        }
        values
    }

    fn attribute_declaration_span(&self, target: &AttributeTarget) -> Option<Span> {
        match target {
            AttributeTarget::GlobalDeclaration { declaration, .. } => self
                .global_symbols
                .declarations
                .iter()
                .find(|candidate| &candidate.id == declaration)
                .map(|candidate| candidate.declaration_span),
            AttributeTarget::ExternBlock { span, .. } => Some(*span),
            _ => None,
        }
    }

    fn check_foreign_symbol(&mut self, function: &FunctionDecl, symbol: &str, linked: bool) {
        let span = function.name_span;
        if symbol.starts_with("dr_") {
            self.diagnostics.push(
                Diagnostic::new(
                    RESERVED_RUNTIME_SYMBOL_CODE,
                    format!("`{symbol}` names a symbol of the Doria runtime, which foreign declarations cannot bind"),
                    span,
                )
                .with_title("Runtime Symbols Are Reserved")
                .with_explanation(
                    "Symbols beginning with `dr_` belong to the Doria runtime. Their interface is private and changes between releases; the standard library reaches the runtime through the compiler.",
                )
                .with_help("use the standard library API for the behavior instead"),
            );
            return;
        }
        let source_name = function.name.rsplit('\\').next().unwrap_or(&function.name);
        if !linked && crate::builtins::is_reserved_intrinsic_name(source_name) {
            self.diagnostics.push(
                Diagnostic::new(
                    LINK_NAME_REQUIRED_CODE,
                    format!("`{source_name}` is a compiler intrinsic, so a foreign declaration cannot use it as its Doria name"),
                    span,
                )
                .with_title("Foreign Symbol Needs A Link Name")
                .with_help(format!(
                    "give the declaration its own Doria name and `#[LinkName(\"{source_name}\")]`"
                )),
            );
        }
    }

    fn check_foreign_symbol_conflicts(
        &mut self,
        symbols: HashMap<String, Vec<(&FunctionDecl, bool)>>,
    ) {
        let mut symbols = symbols.into_iter().collect::<Vec<_>>();
        symbols.sort_by_key(|(_, declarations)| declarations[0].0.span.start);
        for (symbol, declarations) in symbols {
            if declarations.len() < 2 || declarations.iter().all(|(_, linked)| *linked) {
                continue;
            }
            let first = declarations[0].0;
            let first_signature = self.foreign_signature_key(first);
            for (declaration, _) in declarations.iter().skip(1) {
                if self.foreign_signature_key(declaration) == first_signature {
                    continue;
                }
                self.diagnostics.push(
                    Diagnostic::new(
                        CONFLICTING_SYMBOL_CODE,
                        format!("`{symbol}` is declared again with a different signature"),
                        declaration.name_span,
                    )
                    .with_title("Conflicting Declarations Of A Foreign Symbol")
                    .with_related(first.name_span, "the symbol is first declared here")
                    .with_help(format!(
                        "give each binding its own Doria name and `#[LinkName(\"{symbol}\")]`"
                    )),
                );
            }
        }
    }

    fn foreign_signature_key(&self, function: &FunctionDecl) -> Option<(Vec<String>, String)> {
        let signature = self.function_signatures.get(&function.span)?;
        Some((
            signature
                .params
                .iter()
                .map(|parameter| self.types.display(parameter.ty))
                .collect(),
            self.types.display(signature.return_ty),
        ))
    }

    /// The `Doria\Std\Ffi` attributes are valid only on the foreign targets
    /// they describe; each foreign declaration attribute applies once.
    fn check_foreign_attribute_targets(&mut self) {
        let applications = self.attributes.applications.clone();
        let mut seen: HashMap<(Span, String), Span> = HashMap::new();
        for application in &applications {
            let class = application.canonical_class_name.as_str();
            let (allowed, expected) = if class == crate::compiler_known_ffi::LIBRARY {
                (
                    matches!(application.target, AttributeTarget::ExternBlock { .. }),
                    "an `extern \"C\"` block",
                )
            } else if crate::compiler_known_ffi::FOREIGN_FUNCTION_ATTRIBUTES.contains(&class) {
                (
                    matches!(
                        application.target,
                        AttributeTarget::GlobalDeclaration {
                            kind: AttributeTargetKind::ForeignFunction,
                            ..
                        }
                    ),
                    "a declaration inside an `extern \"C\"` block",
                )
            } else {
                continue;
            };
            let short = class.rsplit('\\').next().unwrap_or(class);
            if !allowed {
                self.diagnostics.push(
                    Diagnostic::new(
                        FOREIGN_ATTRIBUTE_TARGET_CODE,
                        format!("`#[{short}]` applies only to {expected}"),
                        application.span,
                    )
                    .with_title("Foreign Attribute Is Not Valid Here"),
                );
                continue;
            }
            if let Some(declaration) = self.attribute_declaration_span(&application.target) {
                if let Some(previous) =
                    seen.insert((declaration, class.to_string()), application.span)
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            FOREIGN_ATTRIBUTE_TARGET_CODE,
                            format!("`#[{short}]` is applied twice to one declaration"),
                            application.span,
                        )
                        .with_title("Foreign Attribute Is Repeated")
                        .with_related(previous, "it is first applied here"),
                    );
                }
            }
        }
        self.check_variadic_declarations();
    }

    /// The `fixedParameters` count of every `#[Variadic]` declaration.
    fn variadic_fixed_parameters(&self) -> HashMap<Span, i64> {
        self.foreign_attribute_arguments(crate::compiler_known_ffi::VARIADIC)
            .into_iter()
            .filter_map(|(span, value)| match value {
                AttributeValueKind::Integer { value } => {
                    value.parse::<i64>().ok().map(|count| (span, count))
                }
                _ => None,
            })
            .collect()
    }

    fn check_variadic_declarations(&mut self) {
        let program = self.program;
        let fixed = self.variadic_fixed_parameters();
        for item in &program.items {
            let Item::Function(function) = item else {
                continue;
            };
            let Some(&count) = fixed.get(&function.span) else {
                continue;
            };
            let Some(signature) = self.function_signatures.get(&function.span).cloned() else {
                continue;
            };
            if count < 1 || count as usize > function.params.len() {
                self.diagnostics.push(
                    Diagnostic::new(
                        VARIADIC_ARGUMENT_CODE,
                        format!(
                            "`fixedParameters: {count}` must name between 1 and the declaration's {} parameters",
                            function.params.len()
                        ),
                        function.name_span,
                    )
                    .with_title("Variadic Fixed Parameter Count Is Out Of Range"),
                );
                continue;
            }
            for (parameter, info) in function
                .params
                .iter()
                .zip(&signature.params)
                .skip(count as usize)
            {
                let promoted = match self.types.kind(info.ty) {
                    TypeKind::Float(crate::types::FloatType::Float32) => Some("float64"),
                    TypeKind::Bool => Some("int32"),
                    TypeKind::Integer(integer) if integer.bit_width() < 32 => {
                        Some(if integer.is_signed() {
                            "int32"
                        } else {
                            "uint32"
                        })
                    }
                    TypeKind::Foreign(ForeignType::Boundary(
                        CBoundaryType::Char | CBoundaryType::WideChar,
                    )) => Some("int32"),
                    _ => None,
                };
                if let Some(promoted) = promoted {
                    self.diagnostics.push(
                        Diagnostic::new(
                            VARIADIC_ARGUMENT_CODE,
                            format!(
                                "a variadic argument of type `{}` is promoted by C, so the binding declares the promoted type",
                                self.types.display(info.ty)
                            ),
                            parameter.type_span,
                        )
                        .with_title("Variadic Argument Needs Its Promoted C Type")
                        .with_help(format!("declare the parameter as `{promoted}`"))
                        .with_fix(parameter.type_span, promoted),
                    );
                }
            }
        }
    }
}
