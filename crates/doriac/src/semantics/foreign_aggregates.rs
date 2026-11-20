//! Foreign structs and unions (Decision 0148 section 6, with the amendment
//! of 2026-11-16 for unions and fixed-length array fields).
//!
//! A foreign aggregate is a nominal inline Copy value with the target C
//! layout. It has fields only: no methods, hooks, constructors or
//! conformances. A struct is built with every field that has no default; a
//! union with no member (every byte zero) or exactly one named member.

use std::collections::{HashMap, HashSet};

use super::foreign_checks::{FOREIGN_TYPE_PLACEMENT_CODE, NOT_FFI_SAFE_CODE};
use super::*;
use crate::ast::{ForeignAggregateDecl, ForeignAggregateKind};
use crate::foreign::{ForeignAggregateType, ForeignType};

pub(super) const FIELD_CODE: &str = "E0798";
pub(super) const UNION_CONSTRUCTION_CODE: &str = "E0799";

#[derive(Debug, Clone)]
pub(super) struct ForeignField {
    pub(super) name: String,
    pub(super) ty: TypeId,
    pub(super) has_default: bool,
    /// The declared default's constant value.
    pub(super) default: Option<crate::const_eval::ConstValue>,
    pub(super) span: Span,
}

#[derive(Debug, Clone)]
pub(super) struct ForeignAggregateInfo {
    pub(super) kind: ForeignAggregateKind,
    pub(super) declaration: Span,
    /// `None` for an incomplete type declared without a body.
    pub(super) fields: Option<Vec<ForeignField>>,
}

impl ForeignAggregateInfo {
    fn field(&self, name: &str) -> Option<&ForeignField> {
        self.fields
            .as_ref()?
            .iter()
            .find(|field| field.name == name)
    }
}

impl<'program> Checker<'program> {
    /// Registers every foreign aggregate's name, then resolves its fields, so
    /// fields and signatures may name aggregates in any order.
    pub(super) fn collect_foreign_aggregates(&mut self) {
        let program = self.program;
        let declarations = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::ForeignAggregate(declaration) => Some(declaration),
                _ => None,
            })
            .collect::<Vec<_>>();
        for declaration in &declarations {
            self.foreign.aggregates.insert(
                declaration.name.clone(),
                ForeignAggregateInfo {
                    kind: declaration.kind,
                    declaration: declaration.span,
                    fields: declaration.fields.as_ref().map(|_| Vec::new()),
                },
            );
        }
        for declaration in declarations {
            self.check_foreign_abi(&declaration.abi.value, declaration.abi.span);
            let fields = self.resolve_foreign_fields(declaration);
            if let Some(info) = self.foreign.aggregates.get_mut(&declaration.name) {
                info.fields = fields;
            }
        }
    }

    fn resolve_foreign_fields(
        &mut self,
        declaration: &ForeignAggregateDecl,
    ) -> Option<Vec<ForeignField>> {
        let authored = declaration.fields.as_ref()?;
        let mut seen = HashMap::<&str, Span>::new();
        let mut fields = Vec::new();
        if authored.is_empty() {
            self.diagnostics.push(
                Diagnostic::new(
                    FIELD_CODE,
                    format!(
                        "`{}` declares no fields; a C {} has at least one",
                        declaration.name,
                        declaration.kind.keyword()
                    ),
                    declaration.name_span,
                )
                .with_title("Foreign Aggregate Has No Fields")
                .with_help(format!(
                    "declare its fields, or write `extern \"C\" {} {};` for a type C code only points to",
                    declaration.kind.keyword(),
                    declaration.name
                )),
            );
        }
        for field in authored {
            if let Some(previous) = seen.insert(&field.name, field.name_span) {
                self.diagnostics.push(
                    Diagnostic::new(
                        FIELD_CODE,
                        format!(
                            "`{}` declares the field `${}` twice",
                            declaration.name, field.name
                        ),
                        field.name_span,
                    )
                    .with_title("Foreign Field Is Declared Twice")
                    .with_related(previous, "the field is first declared here"),
                );
            }
            let ty = self.resolve_type_ref_in_position(
                &field.ty,
                field.type_span,
                TypePosition::ForeignField,
                None,
            );
            self.require_foreign_field_type(ty, field.type_span);
            let mut default_value = None;
            if let Some(default) = &field.default {
                if declaration.kind == ForeignAggregateKind::Union {
                    self.diagnostics.push(
                        Diagnostic::new(
                            FIELD_CODE,
                            "a union's members overlap, so a member has no default; `new` with no arguments makes every byte zero",
                            default.span(),
                        )
                        .with_title("Union Member Cannot Have A Default"),
                    );
                } else {
                    default_value =
                        self.check_foreign_field_default(declaration, field, ty, default);
                }
            }
            fields.push(ForeignField {
                name: field.name.clone(),
                ty,
                has_default: field.default.is_some()
                    || matches!(
                        self.types.kind(ty),
                        TypeKind::Foreign(ForeignType::FixedArray { .. })
                    ),
                default: default_value,
                span: field.span,
            });
        }
        Some(fields)
    }

    fn require_foreign_field_type(&mut self, ty: TypeId, span: Span) {
        if let TypeKind::Foreign(ForeignType::FixedArray { element, .. }) = self.types.kind(ty) {
            let element = *element;
            if matches!(
                self.types.kind(element),
                TypeKind::Foreign(ForeignType::FixedArray { .. })
            ) {
                self.diagnostics.push(
                    Diagnostic::new(
                        FOREIGN_TYPE_PLACEMENT_CODE,
                        "a fixed-length array field has one dimension",
                        span,
                    )
                    .with_title("Foreign Type Is Not Valid Here")
                    .with_help("declare the field with the product of the dimensions as its count"),
                );
                return;
            }
            self.require_ffi_safe(element, span, "a fixed-length array element");
            return;
        }
        if let TypeKind::Foreign(ForeignType::Aggregate(aggregate)) = self.types.kind(ty) {
            if self
                .foreign
                .aggregates
                .get(&aggregate.name)
                .is_some_and(|info| info.fields.is_none())
            {
                self.report_incomplete_by_value(&aggregate.name.clone(), span);
                return;
            }
        }
        self.require_ffi_safe(ty, span, "a foreign field");
    }

    fn check_foreign_field_default(
        &mut self,
        declaration: &ForeignAggregateDecl,
        field: &crate::ast::ForeignFieldDecl,
        ty: TypeId,
        default: &Expr,
    ) -> Option<crate::const_eval::ConstValue> {
        let Some(value) = crate::const_eval::evaluate_parameter_default(
            &self.const_evaluation,
            default,
            &field.ty,
            None,
        ) else {
            self.diagnostics.push(
                Diagnostic::new(
                    FIELD_CODE,
                    format!(
                        "the default of `{}::${}` must be a constant expression",
                        declaration
                            .name
                            .rsplit('\\')
                            .next()
                            .unwrap_or(&declaration.name),
                        field.name
                    ),
                    default.span(),
                )
                .with_title("Foreign Field Default Is Not Constant"),
            );
            return None;
        };
        let scopes = ScopeStack::new();
        self.record_expected_expression_type(default, ty);
        if !self.is_expr_assignable(ty, default, &scopes, None) {
            let actual = self.infer_expr_type(default, &scopes, None);
            self.diagnostics.push(
                Diagnostic::new(
                    "E0408",
                    format!(
                        "the default of `${}` expects `{}`, got `{}`",
                        field.name,
                        self.types.display(ty),
                        self.types.display(actual)
                    ),
                    default.span(),
                )
                .with_title("Foreign Field Default Type Mismatch"),
            );
            return None;
        }
        Some(value)
    }

    /// The type of a foreign aggregate name, or `None` for another name.
    pub(super) fn resolve_foreign_aggregate_type(
        &mut self,
        ty: &TypeRef,
        span: Span,
        position: TypePosition,
    ) -> Option<TypeId> {
        let info = self.foreign.aggregates.get(&ty.name)?.clone();
        if !self.expect_type_arg_count(ty, 0, span) {
            return Some(self.types.unknown());
        }
        if info.fields.is_none() && position != TypePosition::Pointee {
            self.report_incomplete_by_value(&ty.name, span);
            return Some(self.types.unknown());
        }
        let base = self.types.intern(TypeKind::Foreign(ForeignType::Aggregate(
            ForeignAggregateType {
                name: ty.name.clone(),
                kind: info.kind,
            },
        )));
        Some(if ty.nullable {
            self.types.intern(TypeKind::Nullable(base))
        } else {
            base
        })
    }

    fn report_incomplete_by_value(&mut self, name: &str, span: Span) {
        let short = name.rsplit('\\').next().unwrap_or(name);
        self.diagnostics.push(
            Diagnostic::new(
                FOREIGN_TYPE_PLACEMENT_CODE,
                format!("`{short}` is declared without fields, so it has no size and no value; it is used only through a pointer"),
                span,
            )
            .with_title("Foreign Type Is Not Valid Here")
            .with_help(format!("write `Ptr<{short}>` or `MutPtr<{short}>`")),
        );
    }

    pub(super) fn aggregate_of(&self, ty: TypeId) -> Option<(String, ForeignAggregateInfo)> {
        let ty = match self.types.kind(ty) {
            TypeKind::Nullable(inner) => *inner,
            _ => ty,
        };
        let TypeKind::Foreign(ForeignType::Aggregate(aggregate)) = self.types.kind(ty) else {
            return None;
        };
        let info = self.foreign.aggregates.get(&aggregate.name)?.clone();
        Some((aggregate.name.clone(), info))
    }

    /// `new Name(...)` of a foreign aggregate. Returns false for any other
    /// type, which ordinary construction checks.
    pub(super) fn check_foreign_aggregate_construction(
        &mut self,
        class_type: &TypeRef,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let Some(info) = self.foreign.aggregates.get(&class_type.name).cloned() else {
            return false;
        };
        let short = class_type
            .name
            .rsplit('\\')
            .next()
            .unwrap_or(&class_type.name)
            .to_string();
        let Some(fields) = info.fields.clone() else {
            for argument in args {
                self.check_expr(&argument.value, scopes, method_context);
            }
            self.report_incomplete_by_value(&class_type.name, span);
            return true;
        };
        if info.kind == ForeignAggregateKind::Union
            && (args.len() > 1 || args.iter().any(|argument| argument.name.is_none()))
        {
            for argument in args {
                self.check_expr(&argument.value, scopes, method_context);
            }
            self.diagnostics.push(
                Diagnostic::new(
                    UNION_CONSTRUCTION_CODE,
                    format!(
                        "a `{short}` is built with no arguments, every byte zero, or with exactly one named member"
                    ),
                    span,
                )
                .with_title("Union Construction Names One Member")
                .with_help(format!(
                    "write `new {short}()` or `new {short}(member: $value)`, then write any other member after construction"
                )),
            );
            return true;
        }
        let params = fields
            .iter()
            .map(|field| ParamInfo {
                name: field.name.clone(),
                ty: field.ty,
                take: false,
                writable: false,
                has_default: info.kind == ForeignAggregateKind::Union || field.has_default,
                default: None,
                declaration: Some(field.span),
            })
            .collect::<Vec<_>>();
        // A fixed-length array field is supplied from a readonly region of
        // its element count (section 6, amended 2026-11-16); a length known
        // here that differs is E0803, any other mismatch panics (P1704).
        let mut params = params;
        for (param, field) in params.iter_mut().zip(fields.iter()) {
            if let TypeKind::Foreign(ForeignType::FixedArray { element, length }) =
                self.types.kind(field.ty).clone()
            {
                param.ty = self.types.intern(TypeKind::ArrayRegion(
                    crate::types::ByteRegionKind::Readonly,
                    element,
                ));
                let supplied = args
                    .iter()
                    .enumerate()
                    .find(|(position, argument)| match &argument.name {
                        Some(name) => name.text == field.name,
                        None => fields
                            .get(*position)
                            .is_some_and(|by_position| by_position.name == field.name),
                    });
                if let Some((_, argument)) = supplied {
                    if !self.check_fixed_array_supply(
                        &field.name,
                        length,
                        &argument.value,
                        scopes,
                        method_context,
                    ) {
                        return true;
                    }
                }
            }
        }
        // Each argument is an expression in its own right, as for every
        // call: its own subexpressions, such as a nested construction, are
        // checked before the argument is bound to its field.
        for argument in args {
            self.check_expr(&argument.value, scopes, method_context);
        }
        self.check_call_arguments(
            &format!("`new {short}`"),
            &params,
            args,
            span,
            scopes,
            method_context,
        );
        true
    }

    /// A region supplied for the fixed-length array field `$name` of
    /// `length` elements, at construction or by assignment (section 6,
    /// amended 2026-11-16 and 2026-11-17): a length checking can see that
    /// differs is E0803, any other mismatch panics (P1704). Returns false
    /// when it reported.
    fn check_fixed_array_supply(
        &mut self,
        name: &str,
        length: u64,
        value: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let Some(known) = self.known_region_length(value, scopes, method_context) else {
            return true;
        };
        if known == length {
            return true;
        }
        self.diagnostics.push(
            Diagnostic::new(
                "E0803",
                format!("the field `${name}` holds {length} elements, and this region has {known}"),
                value.span(),
            )
            .with_title("Array Region Length Does Not Match The Field")
            .with_help(format!(
                "supply a region of {length} elements, or copy fewer elements into the field's region with `copyFrom`"
            )),
        );
        false
    }

    /// `$value->field = $region` for a fixed-length array field: the
    /// region's length against the field's count, as at construction.
    /// Returns false when it reported.
    pub(super) fn check_fixed_array_field_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let Expr::PropertyAccess {
            object, property, ..
        } = target
        else {
            return true;
        };
        let object_ty = self.infer_expr_type(object, scopes, method_context);
        let Some((_, info)) = self.aggregate_of(object_ty) else {
            return true;
        };
        let Some(TypeKind::Foreign(ForeignType::FixedArray { length, .. })) = info
            .field(property)
            .map(|field| self.types.kind(field.ty).clone())
        else {
            return true;
        };
        self.check_fixed_array_supply(property, length, value, scopes, method_context)
    }

    /// The element count of a region expression when checking can see it:
    /// a `getRegion` or `getWritableRegion` with a constant length, or a
    /// fixed-length array field.
    fn known_region_length(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<u64> {
        match expr {
            Expr::Grouped { expr, .. } | Expr::Unsafe { expr, .. } => {
                self.known_region_length(expr, scopes, method_context)
            }
            Expr::MethodCall { method, args, .. }
                if matches!(method.as_str(), "getRegion" | "getWritableRegion") =>
            {
                let length = args
                    .iter()
                    .find(|argument| {
                        argument
                            .name
                            .as_ref()
                            .is_some_and(|name| name.text == "length")
                    })
                    .or_else(|| args.get(1).filter(|argument| argument.name.is_none()))?;
                match &length.value {
                    Expr::Int { value, .. } => value.replace('_', "").parse().ok(),
                    _ => None,
                }
            }
            Expr::PropertyAccess {
                object, property, ..
            } => {
                let object_ty = self.infer_expr_type(object, scopes, method_context);
                let (_, info) = self.aggregate_of(object_ty)?;
                match self.types.kind(info.field(property)?.ty) {
                    TypeKind::Foreign(ForeignType::FixedArray { length, .. }) => Some(*length),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The type of a field read, or `None` when `object_ty` is not a
    /// foreign aggregate.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn check_foreign_field_read(
        &mut self,
        object: &Expr,
        object_ty: TypeId,
        property: &str,
        member_span: Span,
        span: Span,
        null_safe: bool,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        let (name, info) = self.aggregate_of(object_ty)?;
        if info.field(property).is_none() {
            self.report_unknown_foreign_field(&name, &info, property, member_span);
            return Some(self.types.unknown());
        }
        if info.kind == ForeignAggregateKind::Union {
            self.require_unsafe(span, &format!("reading the union member `${property}`"));
        }
        self.foreign_field_type(
            object,
            object_ty,
            property,
            null_safe,
            scopes,
            method_context,
        )
    }

    /// The field type of a write target, or `None` when `object_ty` is not a
    /// foreign aggregate. Writing a union member is safe. Whether the place
    /// is writable is the ownership checker's rule, as for every place.
    ///
    /// A whole fixed-length array field `T[N]` is assigned from a readonly
    /// region of exactly `N` elements, copied as at construction (section
    /// 6, amended 2026-11-17).
    pub(super) fn check_foreign_field_write(
        &mut self,
        object_ty: TypeId,
        property: &str,
        member_span: Span,
    ) -> Option<Option<TypeId>> {
        let (name, info) = self.aggregate_of(object_ty)?;
        let Some(field) = info.field(property).cloned() else {
            self.report_unknown_foreign_field(&name, &info, property, member_span);
            return Some(None);
        };
        if let TypeKind::Foreign(ForeignType::FixedArray { element, .. }) =
            self.types.kind(field.ty).clone()
        {
            return Some(Some(self.types.intern(TypeKind::ArrayRegion(
                crate::types::ByteRegionKind::Readonly,
                element,
            ))));
        }
        Some(Some(field.ty))
    }

    fn report_unknown_foreign_field(
        &mut self,
        name: &str,
        info: &ForeignAggregateInfo,
        property: &str,
        span: Span,
    ) {
        let short = name.rsplit('\\').next().unwrap_or(name);
        let mut diagnostic = Diagnostic::new(
            FIELD_CODE,
            format!("`{short}` has no field `${property}`"),
            span,
        )
        .with_title("Foreign Aggregate Has No Such Field")
        .with_related(info.declaration, "the type is declared here");
        if info.fields.is_none() {
            diagnostic = diagnostic.with_help(format!(
                "`{short}` is declared without fields; C code reads it through a pointer"
            ));
        }
        self.diagnostics.push(diagnostic);
    }

    /// `Name::SIZE` and `Name::ALIGNMENT` of a foreign aggregate.
    pub(super) fn foreign_aggregate_constant(
        &mut self,
        qualifier: &StaticQualifier,
        member: &str,
        span: Span,
    ) -> Option<TypeId> {
        let StaticQualifier::Class(name) = qualifier else {
            return None;
        };
        let info = self.foreign.aggregates.get(name)?.clone();
        let short = name.rsplit('\\').next().unwrap_or(name);
        if !matches!(member, "SIZE" | "ALIGNMENT") {
            self.diagnostics.push(
                Diagnostic::new(
                    FIELD_CODE,
                    format!("`{short}` has the constants `SIZE` and `ALIGNMENT` and no `{member}`"),
                    span,
                )
                .with_title("Foreign Aggregate Has No Such Constant"),
            );
            return Some(self.types.unknown());
        }
        if info.fields.is_none() {
            self.report_incomplete_by_value(name, span);
            return Some(self.types.unknown());
        }
        Some(self.types.intern(TypeKind::Integer(IntegerType::Int64)))
    }

    /// The type a field read has, without reporting anything; `None` when
    /// `object_ty` is not a foreign aggregate.
    ///
    /// A fixed-length array field `T[N] $name` reads as a region of its `N`
    /// elements that borrows the containing value (section 6): a
    /// `WritableArrayRegion<T>` from a writable place, an `ArrayRegion<T>`
    /// otherwise.
    pub(super) fn foreign_field_type(
        &mut self,
        object: &Expr,
        object_ty: TypeId,
        property: &str,
        null_safe: bool,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        let (_, info) = self.aggregate_of(object_ty)?;
        let Some(field) = info.field(property) else {
            return Some(self.types.unknown());
        };
        let mut ty = field.ty;
        if let TypeKind::Foreign(ForeignType::FixedArray { element, .. }) =
            self.types.kind(ty).clone()
        {
            let access = if self.object_path_access(object, scopes, method_context)
                == ObjectPathAccess::Writable
            {
                crate::types::ByteRegionKind::Writable
            } else {
                crate::types::ByteRegionKind::Readonly
            };
            ty = self.types.intern(TypeKind::ArrayRegion(access, element));
        }
        Some(
            if null_safe && matches!(self.types.kind(object_ty), TypeKind::Nullable(_)) {
                self.types.intern(TypeKind::Nullable(ty))
            } else {
                ty
            },
        )
    }

    /// The type of `Name::SIZE` or `Name::ALIGNMENT`, without reporting.
    pub(super) fn foreign_aggregate_constant_type(
        &mut self,
        qualifier: &StaticQualifier,
        member: &str,
    ) -> Option<TypeId> {
        let StaticQualifier::Class(name) = qualifier else {
            return None;
        };
        let info = self.foreign.aggregates.get(name)?;
        Some(
            if matches!(member, "SIZE" | "ALIGNMENT") && info.fields.is_some() {
                self.types.intern(TypeKind::Integer(IntegerType::Int64))
            } else {
                self.types.unknown()
            },
        )
    }

    /// Foreign aggregates take no methods; every other name is reported by
    /// ordinary member checking.
    pub(super) fn reject_foreign_aggregate_method(
        &mut self,
        object_ty: TypeId,
        method: &str,
        span: Span,
    ) -> bool {
        let Some((name, info)) = self.aggregate_of(object_ty) else {
            return false;
        };
        let short = name.rsplit('\\').next().unwrap_or(&name);
        self.diagnostics.push(
            Diagnostic::new(
                FIELD_CODE,
                format!(
                    "`{short}` is a foreign {} and has no methods, so it has no `{method}()`",
                    info.kind.keyword()
                ),
                span,
            )
            .with_title("Foreign Aggregate Has No Methods")
            .with_related(info.declaration, "the type is declared here"),
        );
        true
    }

    /// Fields that are themselves foreign aggregates may not contain the
    /// aggregate by value, directly or through other fields.
    pub(super) fn check_foreign_aggregate_cycles(&mut self) {
        let names = self.foreign.aggregates.keys().cloned().collect::<Vec<_>>();
        for name in names {
            let mut visiting = HashSet::new();
            if self.aggregate_contains(&name, &name, &mut visiting) {
                let info = self.foreign.aggregates[&name].clone();
                self.diagnostics.push(
                    Diagnostic::new(
                        NOT_FFI_SAFE_CODE,
                        format!(
                            "`{}` contains itself by value, so it has no finite size",
                            name.rsplit('\\').next().unwrap_or(&name)
                        ),
                        info.declaration,
                    )
                    .with_title("Type Cannot Cross The C Boundary")
                    .with_help("hold the inner value through a pointer"),
                );
            }
        }
    }

    fn aggregate_contains(
        &self,
        target: &str,
        current: &str,
        visiting: &mut HashSet<String>,
    ) -> bool {
        if !visiting.insert(current.to_string()) {
            return false;
        }
        let Some(fields) = self
            .foreign
            .aggregates
            .get(current)
            .and_then(|info| info.fields.as_ref())
        else {
            return false;
        };
        fields.iter().any(|field| {
            let mut ty = field.ty;
            if let TypeKind::Foreign(ForeignType::FixedArray { element, .. }) = self.types.kind(ty)
            {
                ty = *element;
            }
            match self.types.kind(ty) {
                TypeKind::Foreign(ForeignType::Aggregate(aggregate)) => {
                    aggregate.name == target
                        || self.aggregate_contains(target, &aggregate.name, visiting)
                }
                _ => false,
            }
        })
    }
}

impl Checker<'_> {
    /// Records `Name::SIZE` and `Name::ALIGNMENT` of every foreign aggregate
    /// with a layout as class constants with the native target's values, so
    /// every consumer of constant values reads them as it reads any other
    /// class constant.
    pub(super) fn record_foreign_layout_constants(&mut self) {
        let mut constants = Vec::new();
        for (name, facts) in self.foreign_aggregate_facts() {
            let declaration = self.foreign.aggregates[&name].declaration;
            for member in ["SIZE", "ALIGNMENT"] {
                let Some(value) = facts
                    .layout_constant(member)
                    .and_then(|value| i64::try_from(value).ok())
                else {
                    continue;
                };
                constants.push((
                    crate::const_eval::ConstKey::Class {
                        class_name: name.clone(),
                        name: member.to_string(),
                    },
                    crate::const_eval::EvaluatedDecl {
                        value: crate::const_eval::ConstValue::Integer(IntegerValue::from_bits(
                            IntegerType::Int64,
                            value as u64,
                        )),
                        ty: crate::const_eval::ConstType::Integer(IntegerType::Int64),
                        access: crate::ast::MemberAccess::External,
                        writable: false,
                        span: declaration,
                    },
                ));
            }
        }
        self.const_evaluation.values.extend(constants);
    }

    /// The facts MIR lowering builds foreign aggregates from, by name.
    pub(super) fn foreign_aggregate_facts(
        &mut self,
    ) -> HashMap<String, crate::foreign::ForeignAggregateFacts> {
        let aggregates = self.foreign.aggregates.clone();
        aggregates
            .iter()
            .map(|(name, info)| {
                let fields = info
                    .fields
                    .iter()
                    .flatten()
                    .map(|field| crate::foreign::ForeignFieldFacts {
                        name: field.name.clone(),
                        ty: self.types.resolved(field.ty),
                        default: field.default.clone(),
                    })
                    .collect();
                let ty = self.types.intern(TypeKind::Foreign(ForeignType::Aggregate(
                    ForeignAggregateType {
                        name: name.clone(),
                        kind: info.kind,
                    },
                )));
                let layout = match foreign_c_type(
                    &self.types,
                    &aggregates,
                    ty,
                    crate::c_layout::CTarget::host(),
                ) {
                    Some(crate::c_layout::CType::Record(record)) => Some(record),
                    _ => None,
                };
                (
                    name.clone(),
                    crate::foreign::ForeignAggregateFacts {
                        kind: info.kind,
                        fields,
                        layout,
                    },
                )
            })
            .collect()
    }
}

/// The C layout of `ty` on `target` (Decision 0148 sections 5 and 6), or
/// `None` when it has none: a type that cannot cross the C boundary,
/// `Opaque`, a foreign struct declared without fields, or an aggregate that
/// contains itself (reported by `check_foreign_aggregate_cycles`).
pub(crate) fn foreign_c_type(
    types: &TypeRegistry,
    aggregates: &HashMap<String, ForeignAggregateInfo>,
    ty: TypeId,
    target: crate::c_layout::CTarget,
) -> Option<crate::c_layout::CType> {
    foreign_c_type_within(types, aggregates, ty, target, &mut Vec::new())
}

fn foreign_c_type_within(
    types: &TypeRegistry,
    aggregates: &HashMap<String, ForeignAggregateInfo>,
    ty: TypeId,
    target: crate::c_layout::CTarget,
    enclosing: &mut Vec<String>,
) -> Option<crate::c_layout::CType> {
    use crate::c_layout::{CRecord, CScalar, CType};
    let scalar = |scalar| Some(CType::Scalar(scalar));
    match types.kind(ty) {
        TypeKind::Integer(integer) => scalar(CScalar::Integer {
            bytes: integer.storage_bytes() as u8,
            signed: integer.is_signed(),
        }),
        TypeKind::Float(float) => scalar(CScalar::Float {
            bytes: float.storage_bytes() as u8,
        }),
        TypeKind::Bool => scalar(CScalar::Bool),
        TypeKind::Foreign(ForeignType::Pointer { .. } | ForeignType::Function(_)) => {
            scalar(CScalar::Pointer)
        }
        TypeKind::Nullable(inner)
            if matches!(
                types.kind(*inner),
                TypeKind::Foreign(ForeignType::Pointer { .. } | ForeignType::Function(_))
            ) =>
        {
            scalar(CScalar::Pointer)
        }
        TypeKind::Foreign(ForeignType::Boundary(boundary)) => scalar(target.boundary(*boundary)),
        TypeKind::Foreign(ForeignType::FixedArray { element, length }) => Some(CType::Array {
            element: Box::new(foreign_c_type_within(
                types, aggregates, *element, target, enclosing,
            )?),
            length: *length,
        }),
        TypeKind::Foreign(ForeignType::Aggregate(aggregate)) => {
            if enclosing.contains(&aggregate.name) {
                return None;
            }
            let info = aggregates.get(&aggregate.name)?;
            let declared = info.fields.as_ref()?;
            enclosing.push(aggregate.name.clone());
            let fields = declared
                .iter()
                .map(|field| foreign_c_type_within(types, aggregates, field.ty, target, enclosing))
                .collect::<Option<Vec<_>>>();
            enclosing.pop();
            Some(CType::Record(std::sync::Arc::new(CRecord::new(
                info.kind, fields?,
            )?)))
        }
        _ => None,
    }
}
