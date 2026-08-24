use std::collections::{HashMap, HashSet};

use crate::ast::ClosureCaptureMode;
use crate::hir::{self, ClassMember, Expr, Item, Stmt};
use crate::mir;
use crate::ownership::CaptureAcquisitionKind;
use crate::symbols::{BindingId, BorrowSource, ClosureId, LexicalOwner};
use crate::types::{FunctionTypeParameterMode, ResolvedType};

#[derive(Debug, Clone)]
pub(crate) struct PhpClosureDescriptor {
    pub(crate) closure_id: ClosureId,
    pub(crate) descriptor: mir::ClosureDescriptorId,
    pub(crate) function_type: mir::FunctionTypeId,
    pub(crate) environment_layout: Option<mir::ClosureEnvironmentLayoutId>,
    pub(crate) invocation_mode: mir::FunctionInvocationMode,
    pub(crate) helper_name: String,
    pub(crate) carrier_name: String,
    pub(crate) environment_name: Option<String>,
    pub(crate) owner_class: Option<String>,
    pub(crate) debug_identity: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PhpClosurePlan {
    pub(crate) requires_runtime: bool,
    pub(crate) descriptors: HashMap<ClosureId, PhpClosureDescriptor>,
    pub(crate) layouts: HashMap<mir::ClosureEnvironmentLayoutId, mir::ClosureEnvironmentLayout>,
    pub(crate) function_types: HashMap<mir::FunctionTypeId, mir::FunctionType>,
    pub(crate) cell_bindings: HashSet<BindingId>,
    pub(crate) binding_resolution: crate::symbols::BindingResolution,
    pub(crate) closures: HashMap<ClosureId, hir::ClosureExpression>,
    pub(crate) semantic_closures: HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
    pub(crate) ownership: HashMap<ClosureId, crate::ownership::ClosureOwnershipInfo>,
    pub(crate) callable_value_calls:
        HashMap<(usize, usize), crate::semantics::CallableValueCallInfo>,
    pub(crate) property_write_types: HashMap<(usize, usize), ResolvedType>,
    pub(crate) callables: HashMap<usize, PhpCallablePlan>,
    pub(crate) call_targets: HashMap<(usize, usize), usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct PhpCallableParameter {
    pub(crate) name: String,
    pub(crate) cell: bool,
    pub(crate) take: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PhpCallablePlan {
    pub(crate) parameters: Vec<PhpCallableParameter>,
}

impl PhpClosurePlan {
    pub(crate) fn build(program: &hir::Program, mir: Option<&mir::Program>) -> Self {
        let callable_classes = callable_classes(program);
        let owner_classes = closure_owner_classes(program, &callable_classes);
        let used_class_names = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(class) => Some(class.name.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let mut allocated_names = used_class_names;
        let mut used_helpers = callable_names(program);
        let mut descriptors = HashMap::new();

        for descriptor in mir
            .into_iter()
            .flat_map(|program| program.closure_descriptors.iter())
        {
            let suffix = descriptor.id.0;
            let carrier_name =
                allocate_class_name(format!("__DoriaClosureValue{suffix}"), &mut allocated_names);
            let environment_name = descriptor.environment_layout.map(|_| {
                allocate_class_name(
                    format!("__DoriaClosureEnvironment{suffix}"),
                    &mut allocated_names,
                )
            });
            let owner_class = owner_classes
                .get(&descriptor.source_closure)
                .cloned()
                .flatten();
            let helper_name = allocate_helper_name(
                format!("__doriaClosureEntry{suffix}"),
                owner_class.as_deref(),
                &mut used_helpers,
            );
            descriptors.insert(
                descriptor.source_closure,
                PhpClosureDescriptor {
                    closure_id: descriptor.source_closure,
                    descriptor: descriptor.id,
                    function_type: descriptor.function_type,
                    environment_layout: descriptor.environment_layout,
                    invocation_mode: descriptor.invocation_mode,
                    helper_name,
                    carrier_name,
                    environment_name,
                    owner_class,
                    debug_identity: descriptor.debug_identity.clone(),
                },
            );
        }

        let mut cell_bindings = program
            .semantic_info
            .binding_resolution
            .declarations_by_id
            .values()
            .filter(|declaration| {
                declaration
                    .source_type
                    .as_ref()
                    .is_some_and(is_function_type)
                    && (declaration.ownership == crate::symbols::BindingOwnership::Owned
                        || declaration.writable)
            })
            .map(|declaration| declaration.id)
            .collect::<HashSet<_>>();

        for closure in program.semantic_info.closures.values() {
            let ownership = program
                .semantic_info
                .closure_ownership
                .get(&closure.closure_id);
            for capture in &closure.captures {
                let needs_place = ownership
                    .and_then(|ownership| {
                        ownership.acquisitions.iter().find(|acquisition| {
                            acquisition.environment_binding_id == capture.environment_binding_id
                        })
                    })
                    .is_some_and(|acquisition| {
                        matches!(
                            acquisition.kind,
                            CaptureAcquisitionKind::ReadonlyLease
                                | CaptureAcquisitionKind::WritableLease
                                | CaptureAcquisitionKind::MoveIntoEnvironment
                        )
                    })
                    || capture.mode != ClosureCaptureMode::Take;
                if needs_place {
                    cell_bindings.insert(capture.source_binding_id);
                }
            }
        }

        mark_parameter_home_bindings(program, &mut cell_bindings);
        let callables = collect_callable_plans(program, &cell_bindings);
        let call_targets = collect_call_targets(program);
        mark_call_argument_places(program, &callables, &call_targets, &mut cell_bindings);

        let property_write_types = program
            .semantic_info
            .property_writes
            .iter()
            .filter_map(|(span, write)| {
                program
                    .semantic_info
                    .classes
                    .iter()
                    .find(|class| class.name == write.class_name)
                    .and_then(|class| {
                        class
                            .properties
                            .iter()
                            .find(|property| property.name == write.property_name)
                    })
                    .map(|property| (*span, property.ty.clone()))
            })
            .collect();

        Self {
            requires_runtime: !program.semantic_info.function_types_by_span.is_empty()
                || !program.semantic_info.closures.is_empty()
                || !program.semantic_info.callable_value_calls.is_empty(),
            descriptors,
            layouts: mir
                .into_iter()
                .flat_map(|program| program.closure_environment_layouts.iter())
                .cloned()
                .map(|layout| (layout.id, layout))
                .collect(),
            function_types: mir
                .into_iter()
                .flat_map(|program| program.function_types.iter())
                .cloned()
                .map(|function_type| (function_type.id, function_type))
                .collect(),
            cell_bindings,
            binding_resolution: program.semantic_info.binding_resolution.clone(),
            closures: collect_closures(program),
            semantic_closures: program.semantic_info.closures.clone(),
            ownership: program.semantic_info.closure_ownership.clone(),
            callable_value_calls: program.semantic_info.callable_value_calls.clone(),
            property_write_types,
            callables,
            call_targets,
        }
    }

    pub(crate) fn descriptor(&self, closure: ClosureId) -> &PhpClosureDescriptor {
        self.descriptors
            .get(&closure)
            .expect("validated MIR must describe every checked closure")
    }

    pub(crate) fn callable_at(&self, span: crate::source::Span) -> Option<&PhpCallablePlan> {
        self.call_targets
            .get(&(span.start, span.end))
            .and_then(|target| self.callables.get(target))
    }

    pub(crate) fn callable_definition(&self, start: usize) -> Option<&PhpCallablePlan> {
        self.callables.get(&start)
    }

    pub(crate) fn function_type(&self, id: mir::FunctionTypeId) -> &mir::FunctionType {
        self.function_types
            .get(&id)
            .expect("validated MIR must describe every closure function type")
    }

    pub(crate) fn layout(
        &self,
        id: mir::ClosureEnvironmentLayoutId,
    ) -> &mir::ClosureEnvironmentLayout {
        self.layouts
            .get(&id)
            .expect("validated MIR must describe every closure environment")
    }
}

fn collect_closures(program: &hir::Program) -> HashMap<ClosureId, hir::ClosureExpression> {
    let mut closures = HashMap::new();
    for item in &program.items {
        match item {
            Item::Function(function) => collect_block_closures(&function.body, &mut closures),
            Item::Class(class) => {
                for member in &class.members {
                    match member {
                        ClassMember::Property(property) => {
                            if let Some(initializer) = &property.initializer {
                                collect_expr_closures(initializer, &mut closures);
                            }
                        }
                        ClassMember::Method(method) => {
                            collect_block_closures(&method.body, &mut closures)
                        }
                        ClassMember::Constant(_) => {}
                    }
                }
            }
            Item::Statement(statement) => collect_statement_closures(statement, &mut closures),
            Item::Enum(_) | Item::Constant(_) => {}
        }
    }
    closures
}

fn collect_block_closures(
    block: &hir::Block,
    closures: &mut HashMap<ClosureId, hir::ClosureExpression>,
) {
    for statement in &block.statements {
        collect_statement_closures(statement, closures);
    }
}

fn collect_statement_closures(
    statement: &Stmt,
    closures: &mut HashMap<ClosureId, hir::ClosureExpression>,
) {
    match statement {
        Stmt::Block(block) => collect_block_closures(block, closures),
        Stmt::VarDecl(decl) => collect_expr_closures(&decl.initializer, closures),
        Stmt::Assignment(assignment) => {
            collect_expr_closures(&assignment.target, closures);
            collect_expr_closures(&assignment.value, closures);
        }
        Stmt::Echo { expr, .. }
        | Stmt::Return {
            expr: Some(expr), ..
        }
        | Stmt::Expr { expr, .. } => collect_expr_closures(expr, closures),
        Stmt::Return { expr: None, .. } | Stmt::Break { .. } | Stmt::Continue { .. } => {}
        Stmt::Throw(statement) => collect_expr_closures(&statement.expr, closures),
        Stmt::Try(statement) => {
            collect_block_closures(&statement.body, closures);
            for catch in &statement.catches {
                collect_block_closures(&catch.body, closures);
            }
            if let Some(finally) = &statement.finally {
                collect_block_closures(&finally.body, closures);
            }
        }
        Stmt::If(statement) => {
            collect_expr_closures(&statement.condition, closures);
            collect_block_closures(&statement.then_block, closures);
            if let Some(branch) = &statement.else_branch {
                match branch {
                    hir::ElseBranch::If(statement) => {
                        collect_statement_closures(&Stmt::If((**statement).clone()), closures)
                    }
                    hir::ElseBranch::Block(block) => collect_block_closures(block, closures),
                }
            }
        }
        Stmt::While(statement) => {
            collect_expr_closures(&statement.condition, closures);
            collect_block_closures(&statement.body, closures);
        }
        Stmt::DoWhile(statement) => {
            collect_block_closures(&statement.body, closures);
            collect_expr_closures(&statement.condition, closures);
        }
        Stmt::For(statement) => {
            if let Some(condition) = &statement.condition {
                collect_expr_closures(condition, closures);
            }
            collect_block_closures(&statement.body, closures);
        }
        Stmt::Foreach(statement) => {
            collect_expr_closures(&statement.iterable, closures);
            collect_block_closures(&statement.body, closures);
        }
        Stmt::Increment(statement) => collect_expr_closures(&statement.target, closures),
    }
}

fn collect_expr_closures(expr: &Expr, closures: &mut HashMap<ClosureId, hir::ClosureExpression>) {
    match expr {
        Expr::Closure(closure) => {
            closures.insert(closure.closure_id, (**closure).clone());
            match &closure.body {
                hir::ClosureBody::Expression(body) => collect_expr_closures(body, closures),
                hir::ClosureBody::Block(body) => collect_block_closures(body, closures),
            }
        }
        Expr::CallableCall(call) => {
            collect_expr_closures(&call.callee, closures);
            for argument in &call.args {
                collect_expr_closures(&argument.value, closures);
            }
        }
        Expr::FunctionCall { args, .. }
        | Expr::MethodCall { args, .. }
        | Expr::StaticCall { args, .. }
        | Expr::New { args, .. } => {
            for argument in args {
                collect_expr_closures(&argument.value, closures);
            }
        }
        Expr::Grouped { expr, .. } | Expr::Unary { expr, .. } | Expr::IsType { expr, .. } => {
            collect_expr_closures(expr, closures)
        }
        Expr::Binary { left, right, .. } => {
            collect_expr_closures(left, closures);
            collect_expr_closures(right, closures);
        }
        Expr::PropertyAccess { object, .. } => collect_expr_closures(object, closures),
        Expr::Index {
            collection, index, ..
        } => {
            collect_expr_closures(collection, closures);
            collect_expr_closures(index, closures);
        }
        Expr::Array { elements, .. } => {
            for element in elements {
                if let Some(key) = &element.key {
                    collect_expr_closures(key, closures);
                }
                collect_expr_closures(&element.value, closures);
            }
        }
        Expr::InterpolatedString { parts, .. } => {
            for part in parts {
                if let hir::InterpolatedStringPart::Expr(expr) = part {
                    collect_expr_closures(expr, closures);
                }
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            collect_expr_closures(scrutinee, closures);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_expr_closures(&guard.condition, closures);
                }
                collect_expr_closures(&arm.value, closures);
            }
        }
        Expr::When(when) => {
            for branch in &when.branches {
                if let Some(condition) = &branch.condition {
                    collect_expr_closures(condition, closures);
                }
                collect_block_closures(&branch.block, closures);
            }
        }
        Expr::Variable { .. }
        | Expr::This { .. }
        | Expr::Identifier { .. }
        | Expr::String { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. }
        | Expr::Int { .. }
        | Expr::StaticMember { .. }
        | Expr::ArrayRepeat { .. }
        | Expr::Range { .. } => {}
    }
}

fn allocate_class_name(base: String, used: &mut HashSet<String>) -> String {
    if used.insert(base.clone()) {
        return base;
    }
    for suffix in 1.. {
        let candidate = format!("{base}_{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("a collision-free generated PHP class name must exist")
}

fn callable_names(program: &hir::Program) -> HashMap<Option<String>, HashSet<String>> {
    let mut names = HashMap::new();
    for item in &program.items {
        match item {
            Item::Function(function) => {
                names
                    .entry(None)
                    .or_insert_with(HashSet::new)
                    .insert(function.name.clone());
            }
            Item::Class(class) => {
                let members = names
                    .entry(Some(class.name.clone()))
                    .or_insert_with(HashSet::new);
                for member in &class.members {
                    if let ClassMember::Method(method) = member {
                        members.insert(method.name.clone());
                    }
                }
            }
            Item::Enum(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }
    names
}

fn allocate_helper_name(
    base: String,
    owner: Option<&str>,
    used: &mut HashMap<Option<String>, HashSet<String>>,
) -> String {
    let namespace = used.entry(owner.map(str::to_string)).or_default();
    if namespace.insert(base.clone()) {
        return base;
    }
    for suffix in 1.. {
        let candidate = format!("{base}_{suffix}");
        if namespace.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("a collision-free generated PHP helper name must exist")
}

fn callable_classes(program: &hir::Program) -> HashMap<usize, String> {
    program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class) => Some(class),
            _ => None,
        })
        .flat_map(|class| {
            class.members.iter().filter_map(|member| match member {
                ClassMember::Method(method) => Some((method.span.start, class.name.clone())),
                _ => None,
            })
        })
        .collect()
}

fn closure_owner_classes(
    program: &hir::Program,
    callable_classes: &HashMap<usize, String>,
) -> HashMap<ClosureId, Option<String>> {
    program
        .semantic_info
        .binding_resolution
        .closure_owners
        .keys()
        .copied()
        .map(|closure| {
            let mut owner = LexicalOwner::Closure(closure);
            let class = loop {
                owner = match program
                    .semantic_info
                    .binding_resolution
                    .lexical_parents
                    .get(&owner)
                    .copied()
                {
                    Some(parent) => parent,
                    None => break None,
                };
                match owner {
                    LexicalOwner::Callable(start) => {
                        break callable_classes.get(&start).cloned();
                    }
                    LexicalOwner::TopLevel => break None,
                    LexicalOwner::Closure(_) => {}
                }
            };
            (closure, class)
        })
        .collect()
}

fn mark_parameter_home_bindings(program: &hir::Program, cells: &mut HashSet<BindingId>) {
    let resolution = &program.semantic_info.binding_resolution;
    for item in &program.items {
        match item {
            Item::Function(function) => {
                mark_callable_parameter_homes(function, program, cells);
            }
            Item::Class(class) => {
                for member in &class.members {
                    if let ClassMember::Method(method) = member {
                        mark_callable_parameter_homes(method, program, cells);
                    }
                }
            }
            Item::Enum(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }

    let _ = resolution;
}

fn mark_callable_parameter_homes(
    function: &hir::FunctionDecl,
    program: &hir::Program,
    cells: &mut HashSet<BindingId>,
) {
    let Some(borrow) = program
        .semantic_info
        .return_borrows
        .get(&function.span.start)
    else {
        return;
    };
    let BorrowSource::Parameter(index) = borrow.source else {
        return;
    };
    let Some(param) = function.params.get(index) else {
        return;
    };
    if let Some(binding) = binding_declared_in_span(program, &param.name, param.span) {
        cells.insert(binding);
    }
}

fn mark_call_argument_places(
    program: &hir::Program,
    callables: &HashMap<usize, PhpCallablePlan>,
    call_targets: &HashMap<(usize, usize), usize>,
    cells: &mut HashSet<BindingId>,
) {
    for item in &program.items {
        match item {
            Item::Function(function) => {
                visit_block_calls(&function.body, program, callables, call_targets, cells)
            }
            Item::Class(class) => {
                for member in &class.members {
                    match member {
                        ClassMember::Method(method) => {
                            visit_block_calls(&method.body, program, callables, call_targets, cells)
                        }
                        ClassMember::Property(property) => {
                            if let Some(initializer) = &property.initializer {
                                visit_expr_calls(
                                    initializer,
                                    program,
                                    callables,
                                    call_targets,
                                    cells,
                                );
                            }
                        }
                        ClassMember::Constant(_) => {}
                    }
                }
            }
            Item::Statement(statement) => {
                visit_statement_calls(statement, program, callables, call_targets, cells)
            }
            Item::Enum(_) | Item::Constant(_) => {}
        }
    }
}

fn visit_block_calls(
    block: &hir::Block,
    program: &hir::Program,
    callables: &HashMap<usize, PhpCallablePlan>,
    call_targets: &HashMap<(usize, usize), usize>,
    cells: &mut HashSet<BindingId>,
) {
    for statement in &block.statements {
        visit_statement_calls(statement, program, callables, call_targets, cells);
    }
}

fn visit_statement_calls(
    statement: &Stmt,
    program: &hir::Program,
    callables: &HashMap<usize, PhpCallablePlan>,
    call_targets: &HashMap<(usize, usize), usize>,
    cells: &mut HashSet<BindingId>,
) {
    match statement {
        Stmt::Block(block) => visit_block_calls(block, program, callables, call_targets, cells),
        Stmt::VarDecl(decl) => {
            visit_expr_calls(&decl.initializer, program, callables, call_targets, cells)
        }
        Stmt::Assignment(assignment) => {
            visit_expr_calls(&assignment.target, program, callables, call_targets, cells);
            visit_expr_calls(&assignment.value, program, callables, call_targets, cells);
        }
        Stmt::Echo { expr, .. }
        | Stmt::Return {
            expr: Some(expr), ..
        }
        | Stmt::Expr { expr, .. } => {
            visit_expr_calls(expr, program, callables, call_targets, cells)
        }
        Stmt::Return { expr: None, .. } | Stmt::Break { .. } | Stmt::Continue { .. } => {}
        Stmt::Throw(statement) => {
            visit_expr_calls(&statement.expr, program, callables, call_targets, cells)
        }
        Stmt::Try(statement) => {
            visit_block_calls(&statement.body, program, callables, call_targets, cells);
            for catch in &statement.catches {
                visit_block_calls(&catch.body, program, callables, call_targets, cells);
            }
            if let Some(finally) = &statement.finally {
                visit_block_calls(&finally.body, program, callables, call_targets, cells);
            }
        }
        Stmt::If(statement) => {
            visit_expr_calls(
                &statement.condition,
                program,
                callables,
                call_targets,
                cells,
            );
            visit_block_calls(
                &statement.then_block,
                program,
                callables,
                call_targets,
                cells,
            );
            if let Some(branch) = &statement.else_branch {
                match branch {
                    hir::ElseBranch::If(statement) => visit_statement_calls(
                        &Stmt::If((**statement).clone()),
                        program,
                        callables,
                        call_targets,
                        cells,
                    ),
                    hir::ElseBranch::Block(block) => {
                        visit_block_calls(block, program, callables, call_targets, cells)
                    }
                }
            }
        }
        Stmt::While(statement) => {
            visit_expr_calls(
                &statement.condition,
                program,
                callables,
                call_targets,
                cells,
            );
            visit_block_calls(&statement.body, program, callables, call_targets, cells);
        }
        Stmt::DoWhile(statement) => {
            visit_block_calls(&statement.body, program, callables, call_targets, cells);
            visit_expr_calls(
                &statement.condition,
                program,
                callables,
                call_targets,
                cells,
            );
        }
        Stmt::For(statement) => {
            if let Some(condition) = &statement.condition {
                visit_expr_calls(condition, program, callables, call_targets, cells);
            }
            visit_block_calls(&statement.body, program, callables, call_targets, cells);
        }
        Stmt::Foreach(statement) => {
            visit_expr_calls(&statement.iterable, program, callables, call_targets, cells);
            visit_block_calls(&statement.body, program, callables, call_targets, cells);
        }
        Stmt::Increment(statement) => {
            visit_expr_calls(&statement.target, program, callables, call_targets, cells)
        }
    }
}

fn visit_expr_calls(
    expr: &Expr,
    program: &hir::Program,
    callables: &HashMap<usize, PhpCallablePlan>,
    call_targets: &HashMap<(usize, usize), usize>,
    cells: &mut HashSet<BindingId>,
) {
    match expr {
        Expr::CallableCall(call) => {
            if let Some(ResolvedType::Function(function_type)) = program
                .semantic_info
                .callable_value_calls
                .get(&(call.span.start, call.span.end))
                .map(|call| &call.function_type)
            {
                mark_mode_arguments(&call.args, &function_type.parameters, program, cells);
            }
            visit_expr_calls(&call.callee, program, callables, call_targets, cells);
            for argument in &call.args {
                visit_expr_calls(&argument.value, program, callables, call_targets, cells);
            }
        }
        Expr::Closure(closure) => match &closure.body {
            hir::ClosureBody::Expression(body) => {
                visit_expr_calls(body, program, callables, call_targets, cells)
            }
            hir::ClosureBody::Block(body) => {
                visit_block_calls(body, program, callables, call_targets, cells)
            }
        },
        Expr::FunctionCall { args, span, .. }
        | Expr::MethodCall { args, span, .. }
        | Expr::StaticCall { args, span, .. }
        | Expr::New { args, span, .. } => {
            if let Some(callable) = call_targets
                .get(&(span.start, span.end))
                .and_then(|target| callables.get(target))
            {
                mark_callable_arguments(args, callable, program, cells);
            }
            for argument in args {
                visit_expr_calls(&argument.value, program, callables, call_targets, cells);
            }
        }
        Expr::Grouped { expr, .. } | Expr::Unary { expr, .. } | Expr::IsType { expr, .. } => {
            visit_expr_calls(expr, program, callables, call_targets, cells)
        }
        Expr::Binary { left, right, .. } => {
            visit_expr_calls(left, program, callables, call_targets, cells);
            visit_expr_calls(right, program, callables, call_targets, cells);
        }
        Expr::PropertyAccess { object, .. } => {
            visit_expr_calls(object, program, callables, call_targets, cells)
        }
        Expr::Index {
            collection, index, ..
        } => {
            visit_expr_calls(collection, program, callables, call_targets, cells);
            visit_expr_calls(index, program, callables, call_targets, cells);
        }
        Expr::Array { elements, .. } => {
            for element in elements {
                if let Some(key) = &element.key {
                    visit_expr_calls(key, program, callables, call_targets, cells);
                }
                visit_expr_calls(&element.value, program, callables, call_targets, cells);
            }
        }
        Expr::InterpolatedString { parts, .. } => {
            for part in parts {
                if let hir::InterpolatedStringPart::Expr(expr) = part {
                    visit_expr_calls(expr, program, callables, call_targets, cells);
                }
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            visit_expr_calls(scrutinee, program, callables, call_targets, cells);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    visit_expr_calls(&guard.condition, program, callables, call_targets, cells);
                }
                visit_expr_calls(&arm.value, program, callables, call_targets, cells);
            }
        }
        Expr::When(when) => {
            for branch in &when.branches {
                if let Some(condition) = &branch.condition {
                    visit_expr_calls(condition, program, callables, call_targets, cells);
                }
                visit_block_calls(&branch.block, program, callables, call_targets, cells);
            }
        }
        Expr::Variable { .. }
        | Expr::This { .. }
        | Expr::Identifier { .. }
        | Expr::String { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. }
        | Expr::Int { .. }
        | Expr::StaticMember { .. }
        | Expr::ArrayRepeat { .. }
        | Expr::Range { .. } => {}
    }
}

fn collect_callable_plans(
    program: &hir::Program,
    cells: &HashSet<BindingId>,
) -> HashMap<usize, PhpCallablePlan> {
    let mut callables = HashMap::new();
    for item in &program.items {
        match item {
            Item::Function(function) => {
                callables.insert(function.span.start, callable_plan(function, program, cells));
            }
            Item::Class(class) => {
                for member in &class.members {
                    if let ClassMember::Method(method) = member {
                        callables.insert(method.span.start, callable_plan(method, program, cells));
                    }
                }
            }
            Item::Enum(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }
    callables
}

fn callable_plan(
    function: &hir::FunctionDecl,
    program: &hir::Program,
    cells: &HashSet<BindingId>,
) -> PhpCallablePlan {
    PhpCallablePlan {
        parameters: function
            .params
            .iter()
            .map(|parameter| {
                let binding = binding_declared_in_span(program, &parameter.name, parameter.span);
                PhpCallableParameter {
                    name: parameter.name.clone(),
                    cell: binding.is_some_and(|binding| cells.contains(&binding)),
                    take: parameter.take,
                }
            })
            .collect(),
    }
}

fn collect_call_targets(program: &hir::Program) -> HashMap<(usize, usize), usize> {
    let functions = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) => Some((function.name.clone(), function.span.start)),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let methods = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class) => Some(class),
            _ => None,
        })
        .flat_map(|class| {
            class.members.iter().filter_map(|member| match member {
                ClassMember::Method(method) => {
                    Some(((class.name.clone(), method.name.clone()), method.span.start))
                }
                _ => None,
            })
        })
        .collect::<HashMap<_, _>>();

    program
        .semantic_info
        .call_targets
        .iter()
        .filter_map(|(span, target)| {
            let start = match target {
                crate::semantics::CallableTarget::Function { name } => functions.get(name),
                crate::semantics::CallableTarget::Method {
                    class_type,
                    method_name,
                } => methods.get(&(class_type.name.clone(), method_name.clone())),
            }?;
            Some((*span, *start))
        })
        .collect()
}

fn mark_callable_arguments(
    args: &[hir::Argument],
    callable: &PhpCallablePlan,
    program: &hir::Program,
    cells: &mut HashSet<BindingId>,
) {
    let mut next_positional = 0;
    for argument in args {
        let index = argument
            .name
            .as_ref()
            .and_then(|name| {
                callable
                    .parameters
                    .iter()
                    .position(|parameter| parameter.name == name.text)
            })
            .unwrap_or_else(|| {
                let index = next_positional;
                next_positional += 1;
                index
            });
        if callable
            .parameters
            .get(index)
            .is_some_and(|parameter| parameter.cell)
        {
            if let Some(binding) = binding_used_at(program, argument.value.span()) {
                cells.insert(binding);
            }
        }
    }
}

fn mark_mode_arguments(
    args: &[hir::Argument],
    parameters: &[crate::types::SemanticFunctionParameter<ResolvedType>],
    program: &hir::Program,
    cells: &mut HashSet<BindingId>,
) {
    for (argument, parameter) in args.iter().zip(parameters) {
        if parameter.ownership_mode == FunctionTypeParameterMode::Writable {
            if let Some(binding) = binding_used_at(program, argument.value.span()) {
                cells.insert(binding);
            }
        }
    }
}

pub(crate) fn binding_used_at(
    program: &hir::Program,
    span: crate::source::Span,
) -> Option<BindingId> {
    program
        .semantic_info
        .binding_resolution
        .uses_by_span
        .get(&(span.start, span.end))
        .copied()
}

pub(crate) fn binding_declared_in_span(
    program: &hir::Program,
    name: &str,
    span: crate::source::Span,
) -> Option<BindingId> {
    program
        .semantic_info
        .binding_resolution
        .declarations_by_id
        .values()
        .find(|declaration| {
            declaration.name == name
                && declaration.span.is_some_and(|declared| {
                    declared.start >= span.start && declared.end <= span.end
                })
        })
        .map(|declaration| declaration.id)
}

pub(crate) fn is_function_type(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::Function(_) => true,
        ResolvedType::Nullable(inner)
        | ResolvedType::TypedArray(inner)
        | ResolvedType::List(inner)
        | ResolvedType::Set(inner)
        | ResolvedType::SortedSet(inner)
        | ResolvedType::PriorityQueue(inner)
        | ResolvedType::Deque(inner) => is_function_type(inner),
        ResolvedType::Dictionary(_, value) | ResolvedType::SortedDictionary(_, value) => {
            is_function_type(value)
        }
        _ => false,
    }
}
