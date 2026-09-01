use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::{ast, hir};

#[derive(Clone, Copy)]
struct ClassContext<'a> {
    name: &'a str,
    type_params: &'a [ast::TypeParamDecl],
    parent: Option<&'a crate::types::TypeRef>,
}

impl ClassContext<'_> {
    fn self_type(self) -> crate::types::TypeRef {
        crate::types::TypeRef::generic(
            self.name,
            self.type_params
                .iter()
                .map(|param| crate::types::TypeRef::named(&param.name))
                .collect(),
        )
    }
}

pub fn lower_program(program: &ast::Program) -> DiagnosticResult<hir::Program> {
    lower_program_with_semantics(program, crate::semantics::SemanticInfo::default())
}

pub fn lower_program_with_semantics(
    program: &ast::Program,
    semantic_info: crate::semantics::SemanticInfo,
) -> DiagnosticResult<hir::Program> {
    let mut items = Vec::with_capacity(program.items.len());
    let mut diagnostics = Vec::new();
    for item in &program.items {
        match lower_item(item) {
            Ok(item) => items.push(item),
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    apply_checked_error_semantics(&mut items, &semantic_info);
    apply_global_identities(&mut items, &semantic_info);

    let test_suites = semantic_info.test_semantics.suites.clone();
    let tests = semantic_info.test_semantics.tests.clone();
    Ok(hir::Program {
        sources: Vec::new(),
        packages: Vec::new(),
        selected_target: hir::SelectedTarget {
            package: semantic_info.compilation_context.package.clone(),
            kind: crate::build_plan::TargetKind::Binary,
            entry_source: Some(semantic_info.compilation_context.source.clone()),
        },
        source_path: String::new(),
        source_text: String::new(),
        namespace: program
            .namespace
            .as_ref()
            .map(|namespace| hir::NamespaceDecl {
                name: namespace.name.canonical(),
                span: namespace.span,
            }),
        items,
        attribute_metadata: semantic_info.attributes.clone(),
        semantic_info,
        test_suites,
        tests,
    })
}

fn apply_global_identities(
    items: &mut [hir::Item],
    semantic_info: &crate::semantics::SemanticInfo,
) {
    for item in items {
        let (span, global_id, source_identity, package) = match item {
            hir::Item::Class(value) => (
                value.span,
                &mut value.global_id,
                &mut value.source_identity,
                &mut value.package,
            ),
            hir::Item::Enum(value) => (
                value.span,
                &mut value.global_id,
                &mut value.source_identity,
                &mut value.package,
            ),
            hir::Item::Function(value) => (
                value.span,
                &mut value.global_id,
                &mut value.source_identity,
                &mut value.package,
            ),
            hir::Item::Constant(value) => (
                value.span,
                &mut value.global_id,
                &mut value.source_identity,
                &mut value.package,
            ),
            hir::Item::Statement(_) => continue,
        };
        if let Some(context) = semantic_info.compilation_contexts.get(&span.source) {
            *source_identity = context.source.clone();
            *package = context.package.clone();
        }
        *global_id = semantic_info
            .global_symbols
            .declarations
            .iter()
            .find(|declaration| declaration.declaration_span == span)
            .map(|declaration| declaration.id.clone());
    }
}

fn apply_checked_error_semantics(
    items: &mut [hir::Item],
    semantic_info: &crate::semantics::SemanticInfo,
) {
    for item in items {
        match item {
            hir::Item::Function(function) => {
                apply_function_checked_error_semantics(function, semantic_info)
            }
            hir::Item::Class(class) => {
                for member in &mut class.members {
                    if let hir::ClassMember::Method(method) = member {
                        apply_function_checked_error_semantics(method, semantic_info);
                    }
                }
            }
            hir::Item::Statement(statement) => {
                apply_statement_checked_error_semantics(statement, semantic_info)
            }
            hir::Item::Enum(_) | hir::Item::Constant(_) => {}
        }
    }
}

fn apply_function_checked_error_semantics(
    function: &mut hir::FunctionDecl,
    semantic_info: &crate::semantics::SemanticInfo,
) {
    function.checked_effects = semantic_info
        .callable_effective_checked_effects
        .get(&function.span)
        .cloned()
        .unwrap_or_default();
    let profile =
        crate::checked_effects::CheckedEffectProfile::classify(function.checked_effects.clone());
    function.required_checked_effects = profile.required;
    function.ambient_checked_effects = profile.ambient;
    function.test_assertion_checked_effects = profile.test_assertion;
    if let Some(throws) = &mut function.throws {
        if let Some(effects) = semantic_info
            .callable_effective_checked_effects
            .get(&function.span)
        {
            for (entry, effect) in throws.entries.iter_mut().zip(effects) {
                entry.resolved = effect.clone();
            }
        }
    }
    apply_block_checked_error_semantics(&mut function.body, semantic_info);
}

fn apply_block_checked_error_semantics(
    block: &mut hir::Block,
    semantic_info: &crate::semantics::SemanticInfo,
) {
    for statement in &mut block.statements {
        apply_statement_checked_error_semantics(statement, semantic_info);
    }
}

fn apply_statement_checked_error_semantics(
    statement: &mut hir::Stmt,
    semantic_info: &crate::semantics::SemanticInfo,
) {
    match statement {
        hir::Stmt::Block(block) => apply_block_checked_error_semantics(block, semantic_info),
        hir::Stmt::Throw(statement) => {
            if let Some(error_type) = semantic_info.throw_error_types.get(&statement.span) {
                statement.error_type = error_type.clone();
            }
            apply_expr_checked_error_semantics(&mut statement.expr, semantic_info);
        }
        hir::Stmt::Try(statement) => {
            apply_block_checked_error_semantics(&mut statement.body, semantic_info);
            for catch in &mut statement.catches {
                if let Some(error_type) = semantic_info.catch_error_types.get(&catch.span) {
                    catch.error_type = error_type.clone();
                }
                apply_block_checked_error_semantics(&mut catch.body, semantic_info);
            }
            if let Some(finally) = &mut statement.finally {
                apply_block_checked_error_semantics(&mut finally.body, semantic_info);
            }
            statement.uncovered_effects = semantic_info
                .try_uncovered_effects
                .get(&statement.span)
                .cloned()
                .unwrap_or_default();
        }
        hir::Stmt::If(statement) => apply_if_checked_error_semantics(statement, semantic_info),
        hir::Stmt::While(statement) => {
            if let Some(given) = &mut statement.given {
                apply_block_checked_error_semantics(&mut given.block, semantic_info);
            }
            apply_expr_checked_error_semantics(&mut statement.condition, semantic_info);
            apply_block_checked_error_semantics(&mut statement.body, semantic_info);
            if let Some(finally) = &mut statement.finally {
                apply_block_checked_error_semantics(&mut finally.block, semantic_info);
            }
        }
        hir::Stmt::DoWhile(statement) => {
            apply_block_checked_error_semantics(&mut statement.body, semantic_info);
            apply_expr_checked_error_semantics(&mut statement.condition, semantic_info);
            if let Some(finally) = &mut statement.finally {
                apply_block_checked_error_semantics(&mut finally.block, semantic_info);
            }
        }
        hir::Stmt::For(statement) => {
            if let Some(initializer) = &mut statement.initializer {
                match initializer {
                    hir::ForInitializer::VarDecl(declaration) => {
                        apply_expr_checked_error_semantics(
                            &mut declaration.initializer,
                            semantic_info,
                        );
                    }
                    hir::ForInitializer::Assignment(assignment) => {
                        apply_expr_checked_error_semantics(&mut assignment.target, semantic_info);
                        apply_expr_checked_error_semantics(&mut assignment.value, semantic_info);
                    }
                }
            }
            if let Some(condition) = &mut statement.condition {
                apply_expr_checked_error_semantics(condition, semantic_info);
            }
            if let Some(increment) = &mut statement.increment {
                match increment {
                    hir::ForIncrement::Increment(increment) => {
                        apply_expr_checked_error_semantics(&mut increment.target, semantic_info);
                    }
                    hir::ForIncrement::Assignment(assignment) => {
                        apply_expr_checked_error_semantics(&mut assignment.target, semantic_info);
                        apply_expr_checked_error_semantics(&mut assignment.value, semantic_info);
                    }
                }
            }
            apply_block_checked_error_semantics(&mut statement.body, semantic_info)
        }
        hir::Stmt::Foreach(statement) => {
            apply_expr_checked_error_semantics(&mut statement.iterable, semantic_info);
            apply_block_checked_error_semantics(&mut statement.body, semantic_info)
        }
        hir::Stmt::VarDecl(declaration) => {
            apply_expr_checked_error_semantics(&mut declaration.initializer, semantic_info)
        }
        hir::Stmt::Assignment(assignment) => {
            apply_expr_checked_error_semantics(&mut assignment.target, semantic_info);
            apply_expr_checked_error_semantics(&mut assignment.value, semantic_info);
        }
        hir::Stmt::Echo { expr, .. } | hir::Stmt::Expr { expr, .. } => {
            apply_expr_checked_error_semantics(expr, semantic_info)
        }
        hir::Stmt::Return { expr, .. } => {
            if let Some(expr) = expr {
                apply_expr_checked_error_semantics(expr, semantic_info);
            }
        }
        hir::Stmt::Increment(increment) => {
            apply_expr_checked_error_semantics(&mut increment.target, semantic_info)
        }
        hir::Stmt::Break { .. } | hir::Stmt::Continue { .. } => {}
    }
}

fn apply_if_checked_error_semantics(
    statement: &mut hir::IfStmt,
    semantic_info: &crate::semantics::SemanticInfo,
) {
    if let Some(given) = &mut statement.given {
        apply_block_checked_error_semantics(&mut given.block, semantic_info);
    }
    apply_expr_checked_error_semantics(&mut statement.condition, semantic_info);
    apply_block_checked_error_semantics(&mut statement.then_block, semantic_info);
    if let Some(branch) = &mut statement.else_branch {
        match branch {
            hir::ElseBranch::If(nested) => apply_if_checked_error_semantics(nested, semantic_info),
            hir::ElseBranch::Block(block) => {
                apply_block_checked_error_semantics(block, semantic_info)
            }
        }
    }
    if let Some(finally) = &mut statement.finally {
        apply_block_checked_error_semantics(&mut finally.block, semantic_info);
    }
}

fn apply_expr_checked_error_semantics(
    expression: &mut hir::Expr,
    semantic_info: &crate::semantics::SemanticInfo,
) {
    if let Some(plan) = semantic_info.assertions.get(&expression.span()) {
        let (actual, expected, user_message) = match expression.clone() {
            hir::Expr::FunctionCall { mut args, .. }
                if plan.matcher == crate::assertions::AssertionMatcher::Fail =>
            {
                (
                    None,
                    None,
                    args.pop().map(|argument| Box::new(argument.value)),
                )
            }
            hir::Expr::MethodCall {
                object, mut args, ..
            } => {
                let base = match object.as_ref() {
                    hir::Expr::PropertyAccess { object, .. } if plan.negated => object.as_ref(),
                    expression => expression,
                };
                let actual = match base {
                    hir::Expr::FunctionCall { args, .. } => args
                        .first()
                        .map(|argument| Box::new(argument.value.clone())),
                    _ => None,
                };
                (
                    actual,
                    args.pop().map(|argument| Box::new(argument.value)),
                    None,
                )
            }
            _ => return,
        };
        *expression = hir::Expr::Assertion(Box::new(hir::Assertion {
            matcher: plan.matcher,
            negated: plan.negated,
            actual,
            expected,
            user_message,
            actual_type: plan.actual_type.clone(),
            expected_type: plan.expected_type.clone(),
            member_span: plan.member_span,
            span: plan.terminal_span,
            checked_effect: plan.checked_effect.clone(),
        }));
    }

    if let hir::Expr::MethodCall {
        object,
        method,
        args,
        null_safe,
        span,
    } = expression.clone()
    {
        if let Some(plan) = semantic_info.list_algorithm_calls.get(&span) {
            let kind = match plan.kind {
                crate::semantics::ListAlgorithmKind::Map => hir::ListAlgorithmKind::Map,
                crate::semantics::ListAlgorithmKind::Filter => hir::ListAlgorithmKind::Filter,
                crate::semantics::ListAlgorithmKind::Reduce => hir::ListAlgorithmKind::Reduce,
            };
            let callback_access = match plan.callback_access {
                crate::semantics::ListCallbackAccess::Readonly => hir::ListCallbackAccess::Readonly,
                crate::semantics::ListCallbackAccess::Writable => hir::ListCallbackAccess::Writable,
            };
            *expression = hir::Expr::ListAlgorithmCall(Box::new(hir::ListAlgorithmCall {
                kind,
                receiver: object,
                arguments: args,
                receiver_type: plan.receiver_type.clone(),
                element_type: plan.element_type.clone(),
                result_type: plan.result_type.clone(),
                accumulator_type: plan.accumulator_type.clone(),
                callback_type: plan.callback_type.clone(),
                callback_access,
                checked_effects: plan.checked_effects.clone(),
                required_checked_effects: plan.required_checked_effects.clone(),
                ambient_checked_effects: plan.ambient_checked_effects.clone(),
                test_assertion_checked_effects: plan.test_assertion_checked_effects.clone(),
                receiver_span: plan.receiver_span,
                callback_span: plan.callback_span,
                span,
            }));
        } else if semantic_info
            .callable_value_calls
            .get(&span)
            .is_some_and(|call| {
                call.target_kind == crate::semantics::CallableValueTargetKind::Property
            })
        {
            *expression = hir::Expr::CallableCall(Box::new(hir::CallableCall {
                callee: Box::new(hir::Expr::PropertyAccess {
                    object,
                    property: method,
                    null_safe,
                    span,
                }),
                args,
                span,
            }));
        }
    }

    match expression {
        hir::Expr::Assertion(assertion) => {
            if let Some(actual) = &mut assertion.actual {
                apply_expr_checked_error_semantics(actual, semantic_info);
            }
            if let Some(expected) = &mut assertion.expected {
                apply_expr_checked_error_semantics(expected, semantic_info);
            }
            if let Some(message) = &mut assertion.user_message {
                apply_expr_checked_error_semantics(message, semantic_info);
            }
        }
        hir::Expr::Closure(closure) => match &mut closure.body {
            hir::ClosureBody::Expression(expression) => {
                apply_expr_checked_error_semantics(expression, semantic_info)
            }
            hir::ClosureBody::Block(block) => {
                apply_block_checked_error_semantics(block, semantic_info)
            }
        },
        hir::Expr::CallableCall(call) => {
            apply_expr_checked_error_semantics(&mut call.callee, semantic_info);
            for argument in &mut call.args {
                apply_expr_checked_error_semantics(&mut argument.value, semantic_info);
            }
        }
        hir::Expr::ListAlgorithmCall(call) => {
            apply_expr_checked_error_semantics(&mut call.receiver, semantic_info);
            for argument in &mut call.arguments {
                apply_expr_checked_error_semantics(&mut argument.value, semantic_info);
            }
        }
        hir::Expr::InterpolatedString { parts, .. } => {
            for part in parts {
                if let hir::InterpolatedStringPart::Expr(expression) = part {
                    apply_expr_checked_error_semantics(expression, semantic_info);
                }
            }
        }
        hir::Expr::Array { elements, .. } => {
            for element in elements {
                if let Some(key) = &mut element.key {
                    apply_expr_checked_error_semantics(key, semantic_info);
                }
                apply_expr_checked_error_semantics(&mut element.value, semantic_info);
            }
        }
        hir::Expr::ArrayRepeat { value, count, .. }
        | hir::Expr::Index {
            collection: value,
            index: count,
            ..
        }
        | hir::Expr::Binary {
            left: value,
            right: count,
            ..
        }
        | hir::Expr::Range {
            start: value,
            end: count,
            ..
        } => {
            apply_expr_checked_error_semantics(value, semantic_info);
            apply_expr_checked_error_semantics(count, semantic_info);
        }
        hir::Expr::PropertyAccess { object, .. }
        | hir::Expr::IsType { expr: object, .. }
        | hir::Expr::Grouped { expr: object, .. }
        | hir::Expr::Unary { expr: object, .. } => {
            apply_expr_checked_error_semantics(object, semantic_info)
        }
        hir::Expr::MethodCall { object, args, .. } => {
            apply_expr_checked_error_semantics(object, semantic_info);
            for argument in args {
                apply_expr_checked_error_semantics(&mut argument.value, semantic_info);
            }
        }
        hir::Expr::FunctionCall { args, .. }
        | hir::Expr::StaticCall { args, .. }
        | hir::Expr::New { args, .. } => {
            for argument in args {
                apply_expr_checked_error_semantics(&mut argument.value, semantic_info);
            }
        }
        hir::Expr::Match {
            scrutinee, arms, ..
        } => {
            apply_expr_checked_error_semantics(scrutinee, semantic_info);
            for arm in arms {
                if let hir::MatchPattern::Expression(expression) = &mut arm.pattern {
                    apply_expr_checked_error_semantics(expression, semantic_info);
                }
                if let Some(guard) = &mut arm.guard {
                    apply_expr_checked_error_semantics(&mut guard.condition, semantic_info);
                }
                apply_expr_checked_error_semantics(&mut arm.value, semantic_info);
            }
        }
        hir::Expr::When(when) => {
            if let Some(given) = &mut when.given {
                apply_block_checked_error_semantics(&mut given.block, semantic_info);
            }
            for branch in &mut when.branches {
                if let Some(condition) = &mut branch.condition {
                    apply_expr_checked_error_semantics(condition, semantic_info);
                }
                apply_block_checked_error_semantics(&mut branch.block, semantic_info);
            }
            if let Some(finally) = &mut when.finally {
                apply_block_checked_error_semantics(&mut finally.block, semantic_info);
            }
        }
        hir::Expr::Variable { .. }
        | hir::Expr::This { .. }
        | hir::Expr::Identifier { .. }
        | hir::Expr::String { .. }
        | hir::Expr::Int { .. }
        | hir::Expr::Float { .. }
        | hir::Expr::Bool { .. }
        | hir::Expr::Null { .. }
        | hir::Expr::StaticMember { .. } => {}
    }
}

fn lower_item(item: &ast::Item) -> Result<hir::Item, Diagnostic> {
    match item {
        ast::Item::Class(class_decl) => Ok(hir::Item::Class(lower_class(class_decl))),
        ast::Item::Enum(enum_decl) => Ok(hir::Item::Enum(lower_enum(enum_decl))),
        ast::Item::Interface(interface_decl) => Err(
            crate::semantics::interface_declaration_diagnostic(interface_decl),
        ),
        ast::Item::Trait(trait_decl) => {
            Err(crate::semantics::trait_declaration_diagnostic(trait_decl))
        }
        ast::Item::Function(function) => Ok(hir::Item::Function(lower_function(function, None))),
        ast::Item::Constant(constant) => Ok(hir::Item::Constant(lower_constant(constant, None))),
        ast::Item::Statement(statement) => Ok(hir::Item::Statement(lower_stmt(statement, None))),
    }
}

fn lower_enum(enum_decl: &ast::EnumDecl) -> hir::EnumDecl {
    hir::EnumDecl {
        global_id: None,
        source_identity: crate::names::SourceIdentity("<unknown>".to_string()),
        package: crate::names::PackageIdentity::Standalone,
        access: enum_decl.access,
        access_span: enum_decl.access_span,
        name: enum_decl.name.clone(),
        type_params: enum_decl
            .type_params
            .iter()
            .map(|param| lower_type_param(param, None))
            .collect(),
        backing_type: enum_decl
            .backing_type
            .as_ref()
            .map(|ty| lower_type_ref(ty, None)),
        cases: enum_decl
            .cases
            .iter()
            .map(|case| hir::EnumCaseDecl {
                name: case.name.clone(),
                payload: case
                    .payload
                    .iter()
                    .map(|field| hir::EnumPayloadField {
                        ty: lower_type_ref(&field.ty, None),
                        name: field.name.clone(),
                        span: field.span,
                    })
                    .collect(),
                backing_value: case
                    .backing_value
                    .as_ref()
                    .map(|value| lower_expr(value, None)),
                span: case.span,
            })
            .collect(),
        span: enum_decl.span,
    }
}

fn lower_class(class_decl: &ast::ClassDecl) -> hir::ClassDecl {
    let class_context = ClassContext {
        name: &class_decl.name,
        type_params: &class_decl.type_params,
        parent: class_decl.parent.as_ref(),
    };
    hir::ClassDecl {
        global_id: None,
        source_identity: crate::names::SourceIdentity("<unknown>".to_string()),
        package: crate::names::PackageIdentity::Standalone,
        access: class_decl.access,
        access_span: class_decl.access_span,
        is_open: class_decl.is_open,
        open_span: class_decl.open_span,
        name: class_decl.name.clone(),
        type_params: class_decl
            .type_params
            .iter()
            .map(|param| lower_type_param(param, Some(class_context)))
            .collect(),
        parent: class_decl
            .parent
            .as_ref()
            .map(|parent| lower_type_ref(parent, Some(class_context))),
        extends_span: class_decl.extends_span,
        parent_span: class_decl.parent_span,
        modifier_prefix_span: class_decl.modifier_prefix_span,
        implements: class_decl.implements.clone(),
        members: class_decl
            .members
            .iter()
            .map(|member| lower_class_member(member, class_context))
            .collect(),
        span: class_decl.span,
    }
}

fn lower_class_member(member: &ast::ClassMember, class_name: ClassContext<'_>) -> hir::ClassMember {
    match member {
        ast::ClassMember::Property(property) => {
            hir::ClassMember::Property(lower_property(property, Some(class_name)))
        }
        ast::ClassMember::Method(method) => {
            hir::ClassMember::Method(lower_function(method, Some(class_name)))
        }
        ast::ClassMember::Constant(constant) => {
            hir::ClassMember::Constant(lower_constant(constant, Some(class_name)))
        }
    }
}

fn lower_property(
    property: &ast::PropertyDecl,
    class_name: Option<ClassContext<'_>>,
) -> hir::PropertyDecl {
    hir::PropertyDecl {
        access: property.access,
        is_static: property.is_static,
        writable: property.writable,
        ty: lower_type_ref(&property.ty, class_name),
        name: property.name.clone(),
        initializer: property
            .initializer
            .as_ref()
            .map(|expr| lower_expr(expr, class_name)),
        span: property.span,
    }
}

fn lower_constant(
    constant: &ast::ConstDecl,
    class_name: Option<ClassContext<'_>>,
) -> hir::ConstDecl {
    hir::ConstDecl {
        global_id: None,
        source_identity: crate::names::SourceIdentity("<unknown>".to_string()),
        package: crate::names::PackageIdentity::Standalone,
        access: constant.access,
        access_span: constant.access_span,
        ty: constant
            .ty
            .as_ref()
            .map(|ty| lower_type_ref(ty, class_name)),
        name: constant.name.clone(),
        initializer: lower_expr(&constant.initializer, class_name),
        span: constant.span,
    }
}

fn lower_function(
    function: &ast::FunctionDecl,
    class_name: Option<ClassContext<'_>>,
) -> hir::FunctionDecl {
    hir::FunctionDecl {
        global_id: None,
        source_identity: crate::names::SourceIdentity("<unknown>".to_string()),
        package: crate::names::PackageIdentity::Standalone,
        access: function.access,
        access_span: function.access_span,
        is_open: function.is_open,
        open_span: function.open_span,
        is_override: function.is_override,
        override_span: function.override_span,
        writable_this: function.writable_this,
        is_static: function.is_static,
        name: function.name.clone(),
        type_params: function
            .type_params
            .iter()
            .map(|param| lower_type_param(param, class_name))
            .collect(),
        params: function
            .params
            .iter()
            .map(|param| lower_param(param, class_name))
            .collect(),
        return_type: function
            .return_type
            .as_ref()
            .map(|ty| lower_type_ref(ty, class_name)),
        throws: function.throws.as_ref().map(|throws| hir::ThrowsClause {
            keyword_span: throws.keyword_span,
            entries: throws
                .entries
                .iter()
                .map(|entry| hir::ThrowsEntry {
                    source: lower_type_ref(&entry.ty, class_name),
                    resolved: crate::types::ResolvedType::Unsupported,
                    span: entry.span,
                })
                .collect(),
            span: throws.span,
        }),
        checked_effects: Vec::new(),
        required_checked_effects: Vec::new(),
        ambient_checked_effects: Vec::new(),
        test_assertion_checked_effects: Vec::new(),
        body: lower_block(&function.body, class_name),
        modifier_prefix_span: function.modifier_prefix_span,
        span: function.span,
    }
}

fn lower_type_param(
    param: &ast::TypeParamDecl,
    class_name: Option<ClassContext<'_>>,
) -> hir::TypeParamDecl {
    hir::TypeParamDecl {
        name: param.name.clone(),
        constraints: param
            .constraints
            .iter()
            .map(|constraint| lower_type_ref(constraint, class_name))
            .collect(),
        default_type: param
            .default_type
            .as_ref()
            .map(|default| lower_type_ref(default, class_name)),
        span: param.span,
    }
}

fn lower_param(param: &ast::Param, class_name: Option<ClassContext<'_>>) -> hir::Param {
    hir::Param {
        promoted_access: param.promoted_access,
        take: param.take,
        writable: param.writable,
        ty: lower_type_ref(&param.ty, class_name),
        name: param.name.clone(),
        default: param
            .default
            .as_ref()
            .map(|expr| lower_expr(expr, class_name)),
        span: param.span,
    }
}

fn lower_block(block: &ast::Block, class_name: Option<ClassContext<'_>>) -> hir::Block {
    hir::Block {
        statements: block
            .statements
            .iter()
            .map(|statement| lower_stmt(statement, class_name))
            .collect(),
        span: block.span,
    }
}

fn lower_stmt(statement: &ast::Stmt, class_name: Option<ClassContext<'_>>) -> hir::Stmt {
    match statement {
        ast::Stmt::Block(block) => hir::Stmt::Block(lower_block(block, class_name)),
        ast::Stmt::VarDecl(decl) => hir::Stmt::VarDecl(hir::VarDecl {
            writable: decl.writable,
            ty: decl.ty.as_ref().map(|ty| lower_type_ref(ty, class_name)),
            bindings: decl
                .bindings
                .iter()
                .map(|binding| hir::VarBinding {
                    name: binding.name.clone(),
                    span: binding.span,
                })
                .collect(),
            initializer: lower_expr(&decl.initializer, class_name),
            span: decl.span,
        }),
        ast::Stmt::Assignment(assignment) => hir::Stmt::Assignment(hir::Assignment {
            target: lower_expr(&assignment.target, class_name),
            op: assignment.op.clone(),
            value: lower_expr(&assignment.value, class_name),
            span: assignment.span,
        }),
        ast::Stmt::Echo { expr, span } => hir::Stmt::Echo {
            expr: lower_expr(expr, class_name),
            span: *span,
        },
        ast::Stmt::Return { expr, span } => hir::Stmt::Return {
            expr: expr.as_ref().map(|expr| lower_expr(expr, class_name)),
            span: *span,
        },
        ast::Stmt::Throw(throw) => hir::Stmt::Throw(hir::ThrowStmt {
            keyword_span: throw.keyword_span,
            expr: lower_expr(&throw.expr, class_name),
            error_type: crate::types::ResolvedType::Unsupported,
            transfers_ownership: true,
            semicolon_span: throw.semicolon_span,
            span: throw.span,
        }),
        ast::Stmt::Try(try_stmt) => hir::Stmt::Try(hir::TryStmt {
            keyword_span: try_stmt.keyword_span,
            body: lower_block(&try_stmt.body, class_name),
            catches: try_stmt
                .catches
                .iter()
                .map(|catch| hir::CatchClause {
                    keyword_span: catch.keyword_span,
                    source_type: lower_type_ref(&catch.ty, class_name),
                    error_type: crate::types::ResolvedType::Unsupported,
                    binding: catch.binding.as_ref().map(|binding| hir::CatchBinding {
                        name: binding.name.clone(),
                        span: binding.span,
                    }),
                    body: lower_block(&catch.body, class_name),
                    span: catch.span,
                })
                .collect(),
            finally: try_stmt.finally.as_ref().map(|finally| hir::TryFinally {
                keyword_span: finally.keyword_span,
                body: lower_block(&finally.body, class_name),
                span: finally.span,
            }),
            uncovered_effects: Vec::new(),
            span: try_stmt.span,
        }),
        ast::Stmt::If(if_stmt) => hir::Stmt::If(lower_if_stmt(if_stmt, class_name)),
        ast::Stmt::While(while_stmt) => hir::Stmt::While(hir::WhileStmt {
            given: while_stmt
                .given
                .as_ref()
                .map(|given| lower_given_prelude(given, class_name)),
            condition: lower_expr(&while_stmt.condition, class_name),
            body: lower_block(&while_stmt.body, class_name),
            finally: while_stmt
                .finally
                .as_ref()
                .map(|finally| lower_finally(finally, class_name)),
            span: while_stmt.span,
        }),
        ast::Stmt::DoWhile(do_while) => hir::Stmt::DoWhile(hir::DoWhileStmt {
            body: lower_block(&do_while.body, class_name),
            condition: lower_expr(&do_while.condition, class_name),
            semicolon_span: do_while.semicolon_span,
            finally: do_while
                .finally
                .as_ref()
                .map(|finally| lower_finally(finally, class_name)),
            span: do_while.span,
        }),
        ast::Stmt::For(for_stmt) => hir::Stmt::For(Box::new(hir::ForStmt {
            initializer: for_stmt
                .initializer
                .as_ref()
                .map(|initializer| lower_for_initializer(initializer, class_name)),
            condition: for_stmt
                .condition
                .as_ref()
                .map(|expr| lower_expr(expr, class_name)),
            increment: for_stmt
                .increment
                .as_ref()
                .map(|increment| lower_for_increment(increment, class_name)),
            body: lower_block(&for_stmt.body, class_name),
            span: for_stmt.span,
        })),
        ast::Stmt::Break { span } => hir::Stmt::Break { span: *span },
        ast::Stmt::Continue { span } => hir::Stmt::Continue { span: *span },
        ast::Stmt::Foreach(foreach) => hir::Stmt::Foreach(hir::ForeachStmt {
            iterable: lower_expr(&foreach.iterable, class_name),
            key: foreach
                .key
                .as_ref()
                .map(|binding| lower_foreach_binding(binding, class_name)),
            value: lower_foreach_binding(&foreach.value, class_name),
            body: lower_block(&foreach.body, class_name),
            span: foreach.span,
        }),
        ast::Stmt::Increment(increment) => hir::Stmt::Increment(hir::IncrementStmt {
            target: lower_expr(&increment.target, class_name),
            op: increment.op.clone(),
            position: increment.position.clone(),
            span: increment.span,
        }),
        ast::Stmt::Expr { expr, span } => hir::Stmt::Expr {
            expr: lower_expr(expr, class_name),
            span: *span,
        },
    }
}

fn lower_for_initializer(
    initializer: &ast::ForInitializer,
    class_name: Option<ClassContext<'_>>,
) -> hir::ForInitializer {
    match initializer {
        ast::ForInitializer::VarDecl(decl) => hir::ForInitializer::VarDecl(hir::VarDecl {
            writable: decl.writable,
            ty: decl.ty.as_ref().map(|ty| lower_type_ref(ty, class_name)),
            bindings: decl
                .bindings
                .iter()
                .map(|binding| hir::VarBinding {
                    name: binding.name.clone(),
                    span: binding.span,
                })
                .collect(),
            initializer: lower_expr(&decl.initializer, class_name),
            span: decl.span,
        }),
        ast::ForInitializer::Assignment(assignment) => {
            hir::ForInitializer::Assignment(hir::Assignment {
                target: lower_expr(&assignment.target, class_name),
                op: assignment.op.clone(),
                value: lower_expr(&assignment.value, class_name),
                span: assignment.span,
            })
        }
    }
}

fn lower_for_increment(
    increment: &ast::ForIncrement,
    class_name: Option<ClassContext<'_>>,
) -> hir::ForIncrement {
    match increment {
        ast::ForIncrement::Increment(increment) => {
            hir::ForIncrement::Increment(hir::IncrementStmt {
                target: lower_expr(&increment.target, class_name),
                op: increment.op.clone(),
                position: increment.position.clone(),
                span: increment.span,
            })
        }
        ast::ForIncrement::Assignment(assignment) => {
            hir::ForIncrement::Assignment(hir::Assignment {
                target: lower_expr(&assignment.target, class_name),
                op: assignment.op.clone(),
                value: lower_expr(&assignment.value, class_name),
                span: assignment.span,
            })
        }
    }
}

fn lower_if_stmt(if_stmt: &ast::IfStmt, class_name: Option<ClassContext<'_>>) -> hir::IfStmt {
    hir::IfStmt {
        given: if_stmt
            .given
            .as_ref()
            .map(|given| lower_given_prelude(given, class_name)),
        condition: lower_expr(&if_stmt.condition, class_name),
        then_block: lower_block(&if_stmt.then_block, class_name),
        else_branch: if_stmt
            .else_branch
            .as_ref()
            .map(|branch| lower_else_branch(branch, class_name)),
        finally: if_stmt
            .finally
            .as_ref()
            .map(|finally| lower_finally(finally, class_name)),
        span: if_stmt.span,
    }
}

fn lower_given_prelude(
    given: &ast::GivenPrelude,
    class_name: Option<ClassContext<'_>>,
) -> hir::GivenPrelude {
    hir::GivenPrelude {
        block: lower_block(&given.block, class_name),
        span: given.span,
    }
}

fn lower_finally(
    finally: &ast::ControlFlowFinally,
    class_name: Option<ClassContext<'_>>,
) -> hir::ControlFlowFinally {
    hir::ControlFlowFinally {
        keyword_span: finally.keyword_span,
        block: lower_block(&finally.block, class_name),
        span: finally.span,
    }
}

fn lower_else_branch(
    branch: &ast::ElseBranch,
    class_name: Option<ClassContext<'_>>,
) -> hir::ElseBranch {
    match branch {
        ast::ElseBranch::If(if_stmt) => {
            hir::ElseBranch::If(Box::new(lower_if_stmt(if_stmt, class_name)))
        }
        ast::ElseBranch::Block(block) => hir::ElseBranch::Block(lower_block(block, class_name)),
    }
}

fn lower_foreach_binding(
    binding: &ast::ForeachBinding,
    class_name: Option<ClassContext<'_>>,
) -> hir::ForeachBinding {
    hir::ForeachBinding {
        writable: binding.writable,
        ty: binding.ty.as_ref().map(|ty| lower_type_ref(ty, class_name)),
        name: binding.name.clone(),
    }
}

fn lower_argument(argument: &ast::Argument, class_name: Option<ClassContext<'_>>) -> hir::Argument {
    hir::Argument {
        name: argument.name.clone(),
        value: lower_expr(&argument.value, class_name),
        span: argument.span,
    }
}

fn lower_expr(expr: &ast::Expr, class_name: Option<ClassContext<'_>>) -> hir::Expr {
    match expr {
        ast::Expr::Closure(closure) => hir::Expr::Closure(Box::new(hir::ClosureExpression {
            closure_id: crate::symbols::ClosureId::from_span(closure.span),
            form: closure.form,
            parameters: closure
                .parameters
                .iter()
                .map(|parameter| hir::ClosureParameter {
                    take: parameter.take,
                    writable: parameter.writable,
                    ty: lower_type_ref(&parameter.ty, class_name),
                    name: parameter.name.clone(),
                    name_span: parameter.name_span,
                    span: parameter.span,
                })
                .collect(),
            return_type: closure
                .return_type
                .as_ref()
                .map(|return_type| lower_type_ref(&return_type.ty, class_name)),
            captures: closure
                .captures
                .as_ref()
                .map(|captures| {
                    captures
                        .captures
                        .iter()
                        .map(|capture| hir::ClosureCapture {
                            mode: capture.mode,
                            name: capture.name.clone(),
                            name_span: capture.name_span,
                            span: capture.span,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            body: match &closure.body {
                ast::ClosureBody::Expression { expression, .. } => {
                    hir::ClosureBody::Expression(Box::new(lower_expr(expression, class_name)))
                }
                ast::ClosureBody::Block(block) => {
                    hir::ClosureBody::Block(lower_block(block, class_name))
                }
            },
            span: closure.span,
        })),
        ast::Expr::CallableCall {
            callee, args, span, ..
        } => hir::Expr::CallableCall(Box::new(hir::CallableCall {
            callee: Box::new(lower_expr(callee, class_name)),
            args: args
                .iter()
                .map(|argument| lower_argument(argument, class_name))
                .collect(),
            span: *span,
        })),
        ast::Expr::Variable { name, span } => hir::Expr::Variable {
            name: name.clone(),
            span: *span,
        },
        ast::Expr::This { span } => hir::Expr::This { span: *span },
        ast::Expr::Identifier { name, span } => hir::Expr::Identifier {
            name: name.clone(),
            span: *span,
        },
        ast::Expr::String { value, span } => hir::Expr::String {
            value: value.clone(),
            span: *span,
        },
        ast::Expr::InterpolatedString { parts, span } => hir::Expr::InterpolatedString {
            parts: parts
                .iter()
                .map(|part| lower_interpolated_string_part(part, class_name))
                .collect(),
            span: *span,
        },
        ast::Expr::Int { value, span } => hir::Expr::Int {
            value: value.clone(),
            span: *span,
        },
        ast::Expr::Float { value, span } => hir::Expr::Float {
            value: value.clone(),
            span: *span,
        },
        ast::Expr::Bool { value, span } => hir::Expr::Bool {
            value: *value,
            span: *span,
        },
        ast::Expr::Null { span } => hir::Expr::Null { span: *span },
        ast::Expr::Array { elements, span } => hir::Expr::Array {
            elements: elements
                .iter()
                .map(|element| lower_array_element(element, class_name))
                .collect(),
            span: *span,
        },
        ast::Expr::ArrayRepeat { value, count, span } => hir::Expr::ArrayRepeat {
            value: Box::new(lower_expr(value, class_name)),
            count: Box::new(lower_expr(count, class_name)),
            span: *span,
        },
        ast::Expr::Index {
            collection,
            index,
            span,
        } => hir::Expr::Index {
            collection: Box::new(lower_expr(collection, class_name)),
            index: Box::new(lower_expr(index, class_name)),
            span: *span,
        },
        ast::Expr::PropertyAccess {
            object,
            property,
            null_safe,
            span,
            ..
        } => hir::Expr::PropertyAccess {
            object: Box::new(lower_expr(object, class_name)),
            property: property.clone(),
            null_safe: *null_safe,
            span: *span,
        },
        ast::Expr::MethodCall {
            object,
            method,
            args,
            null_safe,
            span,
            ..
        } => hir::Expr::MethodCall {
            object: Box::new(lower_expr(object, class_name)),
            method: method.clone(),
            args: args
                .iter()
                .map(|arg| lower_argument(arg, class_name))
                .collect(),
            null_safe: *null_safe,
            span: *span,
        },
        ast::Expr::IsType { expr, ty, span } => hir::Expr::IsType {
            expr: Box::new(lower_expr(expr, class_name)),
            ty: lower_type_ref(ty, class_name),
            span: *span,
        },
        ast::Expr::FunctionCall { name, args, span } => hir::Expr::FunctionCall {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| lower_argument(arg, class_name))
                .collect(),
            span: *span,
        },
        ast::Expr::StaticCall {
            qualifier,
            method,
            args,
            span,
            ..
        } => hir::Expr::StaticCall {
            class_name: resolved_qualifier_name(qualifier, class_name),
            method: method.clone(),
            args: args
                .iter()
                .map(|arg| lower_argument(arg, class_name))
                .collect(),
            span: *span,
        },
        ast::Expr::StaticMember {
            qualifier,
            member,
            span,
            ..
        } => hir::Expr::StaticMember {
            class_name: resolved_qualifier_name(qualifier, class_name),
            member: member.clone(),
            span: *span,
        },
        ast::Expr::New {
            class_type: constructed_class,
            args,
            shared,
            span,
        } => hir::Expr::New {
            class_type: lower_type_ref(constructed_class, class_name),
            args: args
                .iter()
                .map(|arg| lower_argument(arg, class_name))
                .collect(),
            shared: *shared,
            span: *span,
        },
        ast::Expr::Grouped { expr, span } => hir::Expr::Grouped {
            expr: Box::new(lower_expr(expr, class_name)),
            span: *span,
        },
        ast::Expr::Unary { op, expr, span } => hir::Expr::Unary {
            op: op.clone(),
            expr: Box::new(lower_expr(expr, class_name)),
            span: *span,
        },
        ast::Expr::Binary {
            left,
            op,
            right,
            span,
        } => hir::Expr::Binary {
            left: Box::new(lower_expr(left, class_name)),
            op: op.clone(),
            right: Box::new(lower_expr(right, class_name)),
            span: *span,
        },
        ast::Expr::Range {
            start,
            end,
            inclusive,
            span,
        } => hir::Expr::Range {
            start: Box::new(lower_expr(start, class_name)),
            end: Box::new(lower_expr(end, class_name)),
            inclusive: *inclusive,
            span: *span,
        },
        ast::Expr::Match {
            scrutinee,
            mode,
            arms,
            origin,
            span,
        } => hir::Expr::Match {
            scrutinee: Box::new(lower_expr(scrutinee, class_name)),
            mode: *mode,
            arms: arms
                .iter()
                .map(|arm| hir::MatchArm {
                    pattern: lower_match_pattern(&arm.pattern, class_name),
                    guard: arm.guard.as_ref().map(|guard| hir::MatchGuard {
                        condition: lower_expr(&guard.condition, class_name),
                        keyword_span: guard.keyword_span,
                        span: guard.span,
                    }),
                    value: lower_expr(&arm.value, class_name),
                    span: arm.span,
                })
                .collect(),
            origin: *origin,
            span: *span,
        },
        ast::Expr::When(when) => hir::Expr::When(Box::new(hir::WhenExpression {
            given: when
                .given
                .as_ref()
                .map(|given| lower_given_prelude(given, class_name)),
            result_type: when
                .result_type
                .as_ref()
                .map(|ty| lower_type_ref(ty, class_name)),
            branches: when
                .branches
                .iter()
                .map(|branch| hir::WhenBranch {
                    condition: branch
                        .condition
                        .as_ref()
                        .map(|condition| lower_expr(condition, class_name)),
                    block: lower_block(&branch.block, class_name),
                    span: branch.span,
                })
                .collect(),
            finally: when
                .finally
                .as_ref()
                .map(|finally| lower_finally(finally, class_name)),
            span: when.span,
        })),
    }
}

fn lower_match_pattern(
    pattern: &ast::MatchPattern,
    class_name: Option<ClassContext<'_>>,
) -> hir::MatchPattern {
    match pattern {
        ast::MatchPattern::Default { span } => hir::MatchPattern::Default { span: *span },
        ast::MatchPattern::EnumCase {
            qualifier,
            qualifier_span,
            case,
            case_span,
            bindings,
            span,
        } => hir::MatchPattern::EnumCase {
            qualifier: qualifier.clone(),
            qualifier_span: *qualifier_span,
            case: case.clone(),
            case_span: *case_span,
            bindings: bindings.as_ref().map(|bindings| {
                bindings
                    .iter()
                    .map(|binding| hir::MatchBinding {
                        name: binding.name.clone(),
                        span: binding.span,
                    })
                    .collect()
            }),
            span: *span,
        },
        ast::MatchPattern::TypeBinding { ty, binding, span } => hir::MatchPattern::TypeBinding {
            ty: ty.clone(),
            binding: hir::MatchBinding {
                name: binding.name.clone(),
                span: binding.span,
            },
            span: *span,
        },
        ast::MatchPattern::Expression(expr) => {
            hir::MatchPattern::Expression(lower_expr(expr, class_name))
        }
    }
}

fn lower_interpolated_string_part(
    part: &ast::InterpolatedStringPart,
    class_name: Option<ClassContext<'_>>,
) -> hir::InterpolatedStringPart {
    match part {
        ast::InterpolatedStringPart::Text { value, span } => hir::InterpolatedStringPart::Text {
            value: value.clone(),
            span: *span,
        },
        ast::InterpolatedStringPart::Expr(expr) => {
            hir::InterpolatedStringPart::Expr(lower_expr(expr, class_name))
        }
    }
}

fn lower_array_element(
    element: &ast::ArrayElement,
    class_name: Option<ClassContext<'_>>,
) -> hir::ArrayElement {
    hir::ArrayElement {
        key: element.key.as_ref().map(|key| lower_expr(key, class_name)),
        value: lower_expr(&element.value, class_name),
    }
}

fn resolved_qualifier_name(
    qualifier: &ast::StaticQualifier,
    class_name: Option<ClassContext<'_>>,
) -> String {
    match qualifier {
        ast::StaticQualifier::Class(name) => name.clone(),
        ast::StaticQualifier::SelfType => class_name
            .expect("checked `self::` access has a declaring class")
            .name
            .to_string(),
        ast::StaticQualifier::Parent => class_name
            .and_then(|context| context.parent)
            .expect("checked `parent::` access has a parent class")
            .name
            .clone(),
        ast::StaticQualifier::InvalidStatic => {
            unreachable!("rejected qualifier must not reach Doria IR lowering")
        }
    }
}

fn lower_type_ref(
    ty: &crate::types::TypeRef,
    class_name: Option<ClassContext<'_>>,
) -> crate::types::TypeRef {
    class_name.map_or_else(
        || ty.clone(),
        |class_context| ty.resolve_self_in(&class_context.self_type()),
    )
}
