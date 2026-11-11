//! Own constant defaults and executable initialization alongside checked
//! accessors. Calls use the normal argument and full-expression ownership
//! machinery; backing accesses have no call fact.

use super::*;
use crate::ast::PropertyHookKind;

// Constant defaults must not enter the executable expression emitter: that
// path carries diagnostic scopes, checked arithmetic and ownership temporaries.
// One plan owns both admission and emission of PHP's constant-expression subset.
// Everything else remains an executable Doria initializer in the constructor.
pub(super) enum ConstantExpression<'a> {
    Leaf(&'a Expr),
    Array(Vec<(Option<Self>, Self)>),
    Grouped(Box<Self>),
    Not(Box<Self>),
    Binary(BinaryOp, Box<Self>, Box<Self>),
}

impl ConstantExpression<'_> {
    pub(super) fn emit(&self, scopes: &PhpNameScopes) -> String {
        match self {
            Self::Leaf(expr) => match expr {
                Expr::Identifier { name, .. } => php_top_level_constant_name(name),
                Expr::StaticMember {
                    class_name,
                    member,
                    span,
                } => {
                    format!(
                        "{}::{member}",
                        scopes.static_member_class_symbol(class_name, *span)
                    )
                }
                Expr::String { value, .. } => emit_php_string_literal(value),
                Expr::Int { value, span } if is_uint64(scopes.expression_types.get(span)) => {
                    emit_integer_bits(
                        parse_decimal_magnitude(value).expect("checked uint64 literal") as u64
                            as i64,
                    )
                }
                Expr::Int { value, .. } | Expr::Float { value, .. } => value.clone(),
                Expr::Bool { value, .. } => value.to_string(),
                Expr::Null { .. } => "null".to_string(),
                _ => unreachable!("constant property plan admits only constant leaves"),
            },
            Self::Array(elements) => format!(
                "[{}]",
                elements
                    .iter()
                    .map(|(key, value)| match key {
                        Some(key) => format!("{} => {}", key.emit(scopes), value.emit(scopes)),
                        None => value.emit(scopes),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Grouped(value) => format!("({})", value.emit(scopes)),
            Self::Not(value) => format!("!({})", value.emit(scopes)),
            Self::Binary(op, left, right) => format!(
                "(({}) {} ({}))",
                left.emit(scopes),
                if *op == BinaryOp::Xor {
                    "!=="
                } else {
                    emit_binary_op(op)
                },
                right.emit(scopes)
            ),
        }
    }
}

pub(super) fn constant_default<'a>(
    expr: &'a Expr,
    semantic_info: &SemanticInfo,
) -> Option<ConstantExpression<'a>> {
    let storage_type = semantic_info
        .collection_construction_types
        .get(&expr.span())
        .or_else(|| semantic_info.expression_type(expr.span()));
    if is_payload_enum_expression(expr, semantic_info)
        || semantic_info
            .mixed_box_plans
            .get(&expr.span())
            .is_some_and(|plan| plan.requires_boxing())
        || semantic_info
            .interface_conversion_types
            .contains_key(&expr.span())
        || storage_type.is_some_and(core_collection::uses)
        || storage_type.is_some_and(crate::types::resolved_type_is_symbolic)
    {
        return None;
    }
    let constant = match expr {
        Expr::Identifier { name, .. } => Some(ConstKey::TopLevel(name.clone())),
        Expr::StaticMember {
            class_name, member, ..
        } => Some(ConstKey::Class {
            class_name: class_name.clone(),
            name: member.clone(),
        }),
        _ => None,
    };
    if constant
        .as_ref()
        .and_then(|key| semantic_info.const_evaluation.values.get(key))
        .is_some_and(|value| matches!(value.value, ConstValue::PayloadEnum(_)))
    {
        return None;
    }
    match expr {
        Expr::StaticMember {
            class_name, member, ..
        } if semantic_info
            .const_evaluation
            .values
            .contains_key(&ConstKey::Static {
                class_name: class_name.clone(),
                name: member.clone(),
            }) =>
        {
            None
        }
        Expr::Array { elements, .. } => Some(ConstantExpression::Array(
            elements
                .iter()
                .map(|element| {
                    let key = match &element.key {
                        Some(key) => Some(constant_default(key, semantic_info)?),
                        None => None,
                    };
                    Some((key, constant_default(&element.value, semantic_info)?))
                })
                .collect::<Option<Vec<_>>>()?,
        )),
        Expr::Grouped { expr, .. } => Some(ConstantExpression::Grouped(Box::new(
            constant_default(expr, semantic_info)?,
        ))),
        Expr::Unary {
            op: UnaryOp::Not,
            expr,
            ..
        } => Some(ConstantExpression::Not(Box::new(constant_default(
            expr,
            semantic_info,
        )?))),
        Expr::Binary {
            left, op, right, ..
        } if matches!(
            op,
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor | BinaryOp::Coalesce
        ) || (matches!(op, BinaryOp::Equal | BinaryOp::NotEqual)
            && !semantic_info
                .expression_type(left.span())
                .is_some_and(php_builtin_value_equality)
            && !semantic_info
                .expression_type(right.span())
                .is_some_and(php_builtin_value_equality)) =>
        {
            Some(ConstantExpression::Binary(
                op.clone(),
                Box::new(constant_default(left, semantic_info)?),
                Box::new(constant_default(right, semantic_info)?),
            ))
        }
        Expr::Identifier { .. }
        | Expr::String { .. }
        | Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. }
        | Expr::StaticMember { .. } => Some(ConstantExpression::Leaf(expr)),
        _ => None,
    }
}

/// An override contributes a new initialization phase, not another physical
/// field. This identity is checked by semantics and preserved in HIR.
fn inherited_backing_field(
    property: &PropertyDecl,
) -> Option<&crate::property_hooks::PropertyBackingField> {
    property
        .hooks
        .as_ref()
        .and_then(|hooks| hooks.backing_field.as_ref())
        .filter(|field| field.declaration != property.span)
}

pub(super) fn has_runtime_initializer(
    property: &PropertyDecl,
    class: &str,
    semantic_info: &SemanticInfo,
    specialization: &specialization::Plan,
) -> bool {
    !property.is_static
        && property.initializer.as_ref().is_some_and(|value| {
            inherited_backing_field(property).is_some()
                || specialization.has_property_storage(class, &property.name)
                || constant_default(value, semantic_info).is_none()
        })
}

pub(super) fn emit_initializer(
    property: &PropertyDecl,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let initializer = property
        .initializer
        .as_ref()
        .expect("scheduled property initializer");
    // Initializer evaluation and its failure/replacement cleanup belong to
    // this authored operation, including compiler-generated constructors.
    diagnostic_sites::emit_set_site(output, indent, initializer.span(), scopes);
    let mut initializer_scopes = scopes.specialization.property_scope(scopes, &property.name);
    if value_result::storage::initialize_property(property, output, indent, &mut initializer_scopes)
    {
        return;
    }
    let value = emit_owned_expr(initializer, &initializer_scopes);
    if let Some(field) = inherited_backing_field(property) {
        let replacement = scopes.fresh_temp("__doria_property_replacement");
        let previous = scopes.fresh_temp("__doria_property_previous");
        // Evaluate completely before touching the parent's initialized value.
        // If acquisition throws, the completed parent phase still owns it.
        writeln(output, indent, &format!("${replacement} = {value};"));
        writeln(
            output,
            indent,
            &format!("${previous} = $this->{} ?? null;", field.property_name),
        );
        writeln(
            output,
            indent,
            &format!("$this->{} = ${replacement};", field.property_name),
        );
        writeln(output, indent, &format!("unset(${replacement});"));
        writeln(output, indent, &format!("__doria_drop_value(${previous});"));
    } else {
        writeln(
            output,
            indent,
            &format!("$this->{} = {value};", property.name),
        );
    }
}

pub(super) fn receiver(object: &Expr, null_safe: bool, scopes: &PhpNameScopes) -> String {
    receiver_value(
        object,
        emit_member_receiver(object, scopes),
        null_safe,
        scopes,
    )
}

pub(super) fn receiver_value(
    object: &Expr,
    emitted: String,
    null_safe: bool,
    scopes: &PhpNameScopes,
) -> String {
    if scopes
        .expression_types
        .get(&object.span())
        .and_then(shared::kind)
        .is_some()
    {
        format!("{emitted}{}payload()", if null_safe { "?->" } else { "->" })
    } else {
        emitted
    }
}

pub(super) fn getter(
    object: &Expr,
    null_safe: bool,
    span: Span,
    scopes: &PhpNameScopes,
) -> Option<String> {
    let accessor =
        scopes
            .specialization
            .accessor(span, PropertyHookKind::Get, &scopes.substitutions)?;
    let plan = accessor.callable;
    let symbol = scopes.transport_symbol(span, accessor.symbol);
    if plan.caller_receiver_home() {
        return Some(emit_method_with_receiver_home(
            object,
            symbol,
            String::new,
            span,
            null_safe,
            scopes,
        ));
    }
    Some(format!(
        "{}{}{symbol}()",
        receiver(object, null_safe, scopes),
        if null_safe { "?->" } else { "->" }
    ))
}

pub(super) fn emit_assignment(
    assignment: &Assignment,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    let target = expression_region::ungrouped(&assignment.target);
    if assignment.op != AssignOp::Assign
        || !matches!(target, Expr::PropertyAccess { .. })
        || scopes
            .specialization
            .accessor(target.span(), PropertyHookKind::Set, &scopes.substitutions)
            .is_none()
    {
        return false;
    }
    // A setter prepares its invocation without evaluating the target's getter.
    expression_region::region(target, output, indent, scopes, |output, indent, scopes| {
        let call = value_argument::prepare_setter(assignment, output, indent, scopes)
            .expect("checked setter invocation")
            .render(scopes);
        let value = expression_region::guarded(target, call, true, scopes);
        expression_region::consume(output, indent, scopes);
        writeln(output, indent, &format!("{value};"));
    });
    true
}

pub(super) fn assignment(assignment: &Assignment, scopes: &PhpNameScopes) -> Option<String> {
    let mut target = &assignment.target;
    while let Expr::Grouped { expr, .. } = target {
        target = expr;
    }
    let Expr::PropertyAccess { object, span, .. } = target else {
        return None;
    };
    let accessor =
        scopes
            .specialization
            .accessor(*span, PropertyHookKind::Set, &scopes.substitutions)?;
    let setter = accessor.symbol;
    let plan = accessor.callable;
    let mut nested = scopes.clone();
    let temporaries = Rc::new(RefCell::new(Vec::new()));
    nested.expression_temporaries = Some(Rc::clone(&temporaries));
    let getter = (assignment.op != AssignOp::Assign).then(|| {
        scopes
            .specialization
            .accessor(*span, PropertyHookKind::Get, &scopes.substitutions)
            .expect("checked property update has a getter")
    });
    let receiver_home = plan.caller_receiver_home()
        || getter.is_some_and(|accessor| accessor.callable.caller_receiver_home());
    let home_name = scopes.expression_temp("__doria_hook_home_", *span);
    let receiver_name = scopes.expression_temp("__doria_hook_receiver_", *span);
    let mut body = if receiver_home {
        let home = emit_receiver_borrow_home(object, false, &nested);
        format!("${home_name} = {home}; ${receiver_name} = ${home_name}->value; ")
    } else {
        format!("${receiver_name} = {}; ", receiver(object, false, &nested))
    };
    let argument = if assignment.op == AssignOp::Assign {
        emit_call_argument(&assignment.value, plan.parameters.first(), &nested)
    } else {
        let accessor = getter.expect("compound assignment has a getter");
        let getter = accessor.symbol;
        let getter_plan = accessor.callable;
        let arguments = if getter_plan.caller_receiver_home() {
            format!("${home_name}")
        } else {
            String::new()
        };
        let value_name = scopes.expression_temp("__doria_hook_value_", *span);
        body.push_str(&format!(
            "${value_name} = ${receiver_name}->{getter}({arguments}); "
        ));
        let rhs = emit_expr(&assignment.value, &nested);
        if let Some(integer) = integers::ty(nested.expression_types.get(&assignment.value.span()))
            .or_else(|| integers::ty(nested.expression_types.get(&target.span())))
        {
            integers::operation_at(
                integers::assignment_op(&assignment.op).expect("validated integer update"),
                &format!("${value_name}"),
                &rhs,
                integer,
                assignment.span,
                Some(assignment.value.span()),
                &nested,
            )
        } else {
            let float = numeric_place::float_type(assignment.value.span(), &nested)
                .or_else(|| numeric_place::float_type(target.span(), &nested))
                .expect("PHP capability validation admits float updates of a known width");
            numeric_place::float_operation(
                numeric_place::float_assignment_op(&assignment.op)
                    .expect("PHP compound assignment was validated"),
                float,
                &format!("${value_name}"),
                &rhs,
            )
        }
    };
    let argument = if plan.caller_receiver_home() {
        format!("${home_name}, {argument}")
    } else {
        argument
    };
    body.push_str(&format!("${receiver_name}->{setter}({argument});"));
    let temporaries = temporaries.borrow();
    Some(diagnostic_sites::operation(
        emit_expression_body(scopes, &body, &temporaries),
        *span,
        scopes,
    ))
}
