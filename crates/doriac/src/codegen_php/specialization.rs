//! Emission identities for the shared, checked monomorphization worklist.

use super::*;
use crate::class_layout::ClassId;
use crate::monomorphization::{self, substitute_generic_argument, type_substitutions};
use crate::php_closure::{requires_owned_cell, PhpCallableParameter, PhpCallablePlan};
use crate::semantics::{CallableTarget, GenericArgument};
use crate::types::InterfaceType;
use crate::types::{substitute_resolved_type, ClassType};

type RequirementKey = (InterfaceType<ResolvedType>, Span, Vec<GenericArgument>);
type ArgumentBindings = Vec<(Option<String>, Option<BindingId>)>;

#[derive(Debug, Clone)]
pub(super) struct Callable {
    pub id: mir::FunctionId,
    pub declaration: Span,
    pub class: Option<ClassId>,
    pub arguments: Vec<GenericArgument>,
    pub substitutions: HashMap<String, ResolvedType>,
    pub symbol: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Plan {
    pub callables: Vec<Callable>,
    pub class_symbols: HashMap<ClassType<ResolvedType>, String>,
    classes: HashMap<ClassType<ResolvedType>, ClassId>,
    functions: HashMap<String, Span>,
    methods: HashMap<(ClassId, String), Span>,
    generic_methods: HashMap<(String, Vec<GenericArgument>), String>,
    call_targets: HashMap<Span, CallableTarget>,
    call_arguments: HashMap<Span, Vec<GenericArgument>>,
    semantic: Rc<SemanticInfo>,
    callable_plans: HashMap<mir::FunctionId, PhpCallablePlan>,
    requirements: HashMap<RequirementKey, PhpCallablePlan>,
    argument_bindings: HashMap<Span, ArgumentBindings>,
    enums: HashMap<String, crate::enums::EnumType>,
    interfaces: HashMap<InterfaceType<ResolvedType>, ()>,
    property_write_types: HashMap<Span, ResolvedType>,
}

impl Plan {
    pub(super) fn core_capability(
        &self,
        receiver: &ResolvedType,
        operation: crate::compiler_known_contracts::CoreValueOperation,
    ) -> bool {
        let receiver = match receiver {
            ResolvedType::Nullable(inner) => inner.as_ref(),
            _ => receiver,
        };
        let canonical = |origins: &[crate::semantics::contracts::RequirementOrigin]| {
            origins.iter().any(|origin| {
                crate::compiler_known_contracts::CoreValueOperation::from_requirement(
                    origin.declaration,
                ) == Some(operation)
            })
        };
        match receiver {
            ResolvedType::Interface(interface) => self.semantic.contracts.interface_specializations.iter().any(|facts| {
                facts.valid && &facts.specialization == interface && facts.requirements.iter().any(|requirement| canonical(&requirement.origins)
                    && (!matches!(operation, crate::compiler_known_contracts::CoreValueOperation::Equal | crate::compiler_known_contracts::CoreValueOperation::Compare)
                        || requirement.signature.parameters.first().is_some_and(|parameter| &parameter.r#type == receiver)))
            }),
            ResolvedType::Class(_) => self.semantic.contracts.conformances.iter().any(|conformance| {
                conformance.status == crate::semantics::contracts::ConformanceStatus::Checked
                    && &conformance.implementing_type == receiver
                    && (!matches!(operation, crate::compiler_known_contracts::CoreValueOperation::Equal | crate::compiler_known_contracts::CoreValueOperation::Compare)
                        || conformance.interface.arguments.as_slice() == [receiver.clone()])
                    && conformance.implementations.iter().any(|implementation| canonical(&implementation.requirement_origins))
            }),
            _ => false,
        }
    }

    pub(super) fn core_operation(
        &self,
        span: Span,
        operation: crate::compiler_known_contracts::CoreValueOperation,
        receiver: &ResolvedType,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> bool {
        let receiver = substitute_resolved_type(receiver, substitutions);
        let receiver = match &receiver {
            ResolvedType::Nullable(inner) => inner.as_ref(),
            _ => &receiver,
        };
        self.semantic
            .core_operation_calls
            .get(&span)
            .is_some_and(|calls| {
                calls.iter().any(|call| {
                    call.operation == operation
                        && &substitute_resolved_type(&call.receiver_type, substitutions) == receiver
                })
            })
    }

    pub fn build(program: &Program, closures: &PhpClosurePlan) -> Result<Self, BackendError> {
        let synthetic = monomorphization::synthetic_constructors(program);
        let declarations = monomorphization::callable_declarations(program, &synthetic);
        let classes = program
            .semantic_info
            .classes
            .iter()
            .map(|class| {
                (
                    ClassType::new(class.declaration_name.clone(), class.arguments.clone()),
                    class.id,
                )
            })
            .collect::<HashMap<_, _>>();
        let (instances, interface_calls) = monomorphization::collect_callable_instances(
            program,
            &declarations,
            &classes,
            &program.semantic_info,
        )
        .map_err(BackendError::from_diagnostics)?;
        let mut plan = Self {
            classes,
            call_targets: program.semantic_info.call_targets.clone(),
            call_arguments: program
                .semantic_info
                .generic_call_specializations
                .iter()
                .map(|(span, specialization)| (*span, specialization.arguments.clone()))
                .collect(),
            semantic: Rc::new(program.semantic_info.clone()),
            enums: program
                .semantic_info
                .enums
                .iter()
                .map(|definition| {
                    (
                        definition.name.clone(),
                        crate::enums::EnumType::new(definition.id, definition.name.clone()),
                    )
                })
                .collect(),
            interfaces: program
                .semantic_info
                .contracts
                .interface_specializations
                .iter()
                .filter(|interface| interface.valid)
                .map(|interface| (interface.specialization.clone(), ()))
                .collect(),
            property_write_types: closures.property_write_types.clone(),
            ..Self::default()
        };
        crate::hir::visit::expressions(program, &mut |expr| {
            let (span, args) = match expr {
                Expr::FunctionCall { span, args, .. }
                | Expr::MethodCall { span, args, .. }
                | Expr::StaticCall { span, args, .. }
                | Expr::New { span, args, .. } => (*span, args),
                Expr::CallableCall(call) => (call.span, &call.args),
                _ => return,
            };
            plan.argument_bindings.insert(
                span,
                args.iter()
                    .map(|argument| {
                        let binding = program
                            .semantic_info
                            .binding_resolution
                            .uses_by_span
                            .get(&argument.value.span())
                            .copied();
                        (
                            argument.name.as_ref().map(|name| name.text.clone()),
                            binding,
                        )
                    })
                    .collect(),
            );
        });
        let mut names = declarations
            .iter()
            .map(|declaration| declaration.function.name.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        for (index, instance) in instances.iter().enumerate() {
            let declaration = declarations[instance.declaration];
            let function = declaration.function;
            if let Some(class) = declaration.class {
                plan.methods
                    .insert((class, function.name.clone()), function.span);
            } else {
                plan.functions.insert(function.name.clone(), function.span);
            }
            let symbol = if declaration.class.is_some() {
                if instance.arguments.is_empty() {
                    function.name.clone()
                } else {
                    plan.generic_methods
                        .entry((function.name.clone(), instance.arguments.clone()))
                        .or_insert_with(|| allocate_symbol("__doriaSpecializedMethod", &mut names))
                        .clone()
                }
            } else if instance.arguments.is_empty() {
                php_function_name(&function.name)
            } else {
                format!("{}_instance{index}", php_function_name(&function.name))
            };
            plan.callables.push(Callable {
                id: mir::FunctionId(index),
                declaration: function.span,
                class: declaration.class,
                arguments: instance.arguments.clone(),
                substitutions: type_substitutions(declaration, &instance.arguments)
                    .map_err(BackendError::from_diagnostics)?,
                symbol,
            });
        }
        let mut names = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(class) => Some(php_symbol_name(&class.name).to_ascii_lowercase()),
                Item::Enum(enumeration) => {
                    Some(php_symbol_name(&enumeration.name).to_ascii_lowercase())
                }
                _ => None,
            })
            .collect();
        for class in &program.semantic_info.classes {
            let symbol = if class.arguments.is_empty() {
                php_symbol_name(&class.declaration_name)
            } else {
                allocate_symbol("__DoriaClassSpecialization", &mut names)
            };
            plan.class_symbols.insert(
                ClassType::new(class.declaration_name.clone(), class.arguments.clone()),
                symbol,
            );
        }
        plan.callable_plans = plan
            .callables
            .iter()
            .map(|callable| (callable.id, plan.callable_plan(callable, closures)))
            .collect();
        for interface in &program.semantic_info.contracts.interface_specializations {
            if !interface.valid {
                continue;
            }
            for requirement in &interface.requirements {
                let tuples = if requirement.generic_parameters.is_empty() {
                    vec![Vec::new()]
                } else {
                    interface_calls
                        .iter()
                        .filter(|call| {
                            call.interface == interface.specialization
                                && requirement
                                    .origins
                                    .iter()
                                    .any(|origin| origin.declaration == call.requirement)
                        })
                        .map(|call| call.arguments.clone())
                        .collect()
                };
                for arguments in tuples {
                    let substitutions = requirement
                        .generic_parameters
                        .iter()
                        .zip(&arguments)
                        .map(|(parameter, GenericArgument::Type(ty))| {
                            (parameter.name.clone(), ty.clone())
                        })
                        .collect();
                    let callable = PhpCallablePlan {
                        returns_borrow: requirement.return_borrow.is_some(),
                        parameters: requirement
                            .signature
                            .parameters
                            .iter()
                            .enumerate()
                            .map(|(index, parameter)| {
                                let ty =
                                    substitute_resolved_type(&parameter.r#type, &substitutions);
                                PhpCallableParameter {
                                    name: parameter.name.clone(),
                                    take: parameter.take,
                                    cell: requires_owned_cell(&ty)
                                        && (parameter.take || parameter.writable)
                                        || requirement.return_borrow.is_some_and(|borrow| {
                                            borrow.source
                                                == crate::symbols::BorrowSource::Parameter(index)
                                        }),
                                }
                            })
                            .collect(),
                    };
                    for origin in &requirement.origins {
                        plan.requirements.insert(
                            (
                                interface.specialization.clone(),
                                origin.declaration,
                                arguments.clone(),
                            ),
                            callable.clone(),
                        );
                    }
                }
            }
        }
        // Propagate parameter homes only across the same checked specialization.
        // Source spans alone conflate e.g. Read<int> and Read<OwnedObject>.
        loop {
            let mut changed = false;
            for conformance in &program.semantic_info.contracts.conformances {
                if conformance.status != crate::semantics::contracts::ConformanceStatus::Checked {
                    continue;
                }
                let ResolvedType::Class(class) = &conformance.implementing_type else {
                    continue;
                };
                let Some(class_id) = plan.classes.get(class) else {
                    continue;
                };
                let owner_classes = std::iter::once(*class_id)
                    .chain(
                        program.semantic_info.classes[class_id.0]
                            .ancestors
                            .iter()
                            .filter_map(|class| plan.classes.get(class).copied()),
                    )
                    .collect::<HashSet<_>>();
                for implementation in &conformance.implementations {
                    let Some(body) = implementation.implementation else {
                        continue;
                    };
                    for callable in plan.callables.iter().filter(|callable| {
                        callable
                            .class
                            .is_some_and(|class| owner_classes.contains(&class))
                            && callable.declaration == body
                    }) {
                        for origin in &implementation.requirement_origins {
                            let key = (
                                conformance.interface.clone(),
                                origin.declaration,
                                callable.arguments.clone(),
                            );
                            let Some(requirement) = plan.requirements.get_mut(&key) else {
                                continue;
                            };
                            let body = plan.callable_plans.get_mut(&callable.id).unwrap();
                            for (body, requirement) in
                                body.parameters.iter_mut().zip(&mut requirement.parameters)
                            {
                                let cell = body.cell || requirement.cell;
                                changed |= body.cell != cell || requirement.cell != cell;
                                body.cell = cell;
                                requirement.cell = cell;
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        Ok(plan)
    }

    pub fn callable(&self, id: mir::FunctionId) -> &Callable {
        &self.callables[id.0]
    }

    pub fn class_symbol(&self, id: ClassId) -> &str {
        let class = self
            .semantic
            .classes
            .iter()
            .find(|class| class.id == id)
            .expect("checked class identity");
        &self.class_symbols
            [&ClassType::new(class.declaration_name.clone(), class.arguments.clone())]
    }

    pub fn property_scope(&self, scopes: &PhpNameScopes, name: &str) -> PhpNameScopes {
        let class = self
            .semantic
            .classes
            .iter()
            .find(|class| Some(class.id) == scopes.current_class_id)
            .expect("property initializer has a concrete declaring class");
        let property = class
            .properties
            .iter()
            .find(|property| property.name == name)
            .expect("checked property identity");
        let mut scopes = scopes.clone();
        scopes.closure_owner = Some(mir::ClosureOwner::PropertyInitializer(property.id));
        scopes
    }

    pub fn class_symbol_for_call(
        &self,
        span: Span,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<&str> {
        let CallableTarget::Method { class_type, .. } = self.target(span, substitutions)? else {
            return None;
        };
        self.class_symbols.get(&class_type).map(String::as_str)
    }

    pub fn class_symbol_for_type(
        &self,
        ty: &TypeRef,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<&str> {
        let mut ty = ty.clone();
        ty.nullable = false;
        let ResolvedType::Class(class) = self.resolve_type(&ty, substitutions)? else {
            return None;
        };
        self.class_symbols.get(&class).map(String::as_str)
    }

    pub fn resolve_type(
        &self,
        ty: &TypeRef,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<ResolvedType> {
        let resolved = crate::types::resolved_type_ref_with_substitutions(ty, substitutions)?;
        Some(crate::types::resolve_nominal_kinds(
            resolved,
            &self.enums,
            &self.interfaces,
        ))
    }

    pub fn scope(&self, scopes: &PhpNameScopes, callable: &Callable) -> PhpNameScopes {
        let mut scopes = scopes.expression_scope();
        scopes.closure_owner = Some(mir::ClosureOwner::Callable(callable.id));
        scopes.substitutions = callable.substitutions.clone();
        scopes.expression_types = php_expression_types(&self.semantic);
        scopes.type_test_types = self.semantic.type_test_types.clone();
        scopes.throw_error_types = self.semantic.throw_error_types.clone();
        scopes.catch_error_types = self.semantic.catch_error_types.clone();
        scopes.mixed_box_plans = self.semantic.mixed_box_plans.clone();
        scopes.matches = self.semantic.matches.clone();
        scopes.whens = self.semantic.whens.clone();
        let substitute = |ty: &ResolvedType| substitute_resolved_type(ty, &callable.substitutions);
        for types in [
            &mut scopes.expression_types,
            &mut scopes.type_test_types,
            &mut scopes.throw_error_types,
            &mut scopes.catch_error_types,
        ] {
            for ty in types.values_mut() {
                *ty = substitute(ty);
            }
        }
        for plan in scopes.mixed_box_plans.values_mut() {
            plan.source_type = substitute(&plan.source_type);
        }
        for plan in scopes.whens.values_mut() {
            plan.result_type = substitute(&plan.result_type);
        }
        for plan in scopes.matches.values_mut() {
            plan.scrutinee_type = substitute(&plan.scrutinee_type);
            plan.result_type = substitute(&plan.result_type);
            for arm in &mut plan.arms {
                if let ResolvedMatchPattern::ExactType(ty) = &mut arm.pattern {
                    *ty = substitute(ty);
                }
                for binding in &mut arm.bindings {
                    binding.ty = substitute(&binding.ty);
                }
            }
        }
        let mut closures = (*scopes.closure_plan).clone();
        closures.cell_bindings = closures.binding_homes.clone();
        closures.binding_resolution = self.semantic.binding_resolution.clone();
        closures.semantic_closures = self.semantic.closures.clone();
        closures.callable_value_calls = self.semantic.callable_value_calls.clone();
        closures.property_write_types = self.property_write_types.clone();
        closures.call_site_plans.clear();
        for declaration in closures.binding_resolution.declarations_by_id.values_mut() {
            if let Some(ty) = &mut declaration.source_type {
                *ty = substitute(ty);
                if crate::php_closure::requires_owned_cell(ty)
                    && (declaration.ownership == crate::symbols::BindingOwnership::Owned
                        || declaration.writable)
                {
                    closures.cell_bindings.insert(declaration.id);
                }
            }
        }
        for closure in closures.semantic_closures.values_mut() {
            closure.function_type = substitute(&closure.function_type);
            closure.execution_function_type = substitute(&closure.execution_function_type);
            closure.inferred_return_type = substitute(&closure.inferred_return_type);
            for capture in &mut closure.captures {
                capture.source_type = substitute(&capture.source_type);
            }
            for effects in [
                &mut closure.inferred_checked_effects,
                &mut closure.required_checked_effects,
                &mut closure.ambient_checked_effects,
                &mut closure.test_assertion_checked_effects,
            ] {
                for effect in effects {
                    *effect = substitute(effect);
                }
            }
        }
        for call in closures.callable_value_calls.values_mut() {
            call.function_type = substitute(&call.function_type);
            call.return_type = substitute(&call.return_type);
            for effects in [
                &mut call.checked_effects,
                &mut call.required_checked_effects,
                &mut call.ambient_checked_effects,
                &mut call.test_assertion_checked_effects,
            ] {
                for effect in effects {
                    *effect = substitute(effect);
                }
            }
        }
        for ty in closures.property_write_types.values_mut() {
            *ty = substitute(ty);
        }
        closures.callables.insert(
            callable.declaration,
            self.callable_plans[&callable.id].clone(),
        );
        for span in self.call_targets.keys() {
            if let Some(target) = self.direct_call(*span, &callable.substitutions) {
                closures
                    .call_site_plans
                    .insert(*span, self.callable_plans[&target.id].clone());
            } else if let Some(CallableTarget::InterfaceMethod {
                interface,
                requirement,
                ..
            }) = self.target(*span, &callable.substitutions)
            {
                let key = (
                    interface,
                    requirement,
                    self.arguments(*span, &callable.substitutions),
                );
                if let Some(plan) = self.requirements.get(&key) {
                    closures.call_site_plans.insert(*span, plan.clone());
                }
            }
        }
        for (span, arguments) in &self.argument_bindings {
            let mut positional = 0;
            for (name, binding) in arguments {
                let index = name
                    .as_ref()
                    .and_then(|name| {
                        closures.callable_at(*span).and_then(|callable| {
                            callable
                                .parameters
                                .iter()
                                .position(|parameter| &parameter.name == name)
                        })
                    })
                    .unwrap_or_else(|| {
                        let index = positional;
                        positional += 1;
                        index
                    });
                let cell = closures.callable_at(*span).is_some_and(|callable| {
                    callable.parameters.get(index).is_some_and(|parameter| parameter.cell)
                }) || closures.callable_value_calls.get(span).is_some_and(|call| {
                    matches!(&call.function_type, ResolvedType::Function(function) if function.parameters.get(index)
                        .is_some_and(|parameter| parameter.ownership_mode == crate::types::FunctionTypeParameterMode::Writable))
                });
                if cell {
                    if let Some(binding) = binding {
                        closures.cell_bindings.insert(*binding);
                    }
                }
            }
        }
        scopes.closure_plan = Rc::new(closures);
        scopes
    }

    fn callable_plan(
        &self,
        callable: &Callable,
        closures: &PhpClosurePlan,
    ) -> crate::php_closure::PhpCallablePlan {
        let mut plan = closures
            .source_callables
            .get(&callable.declaration)
            .cloned()
            .unwrap_or(crate::php_closure::PhpCallablePlan {
                parameters: Vec::new(),
                returns_borrow: false,
            });
        if let Some(signature) = self.semantic.callable_signatures.get(&callable.declaration) {
            for (parameter, checked) in plan.parameters.iter_mut().zip(&signature.parameters) {
                let ty = substitute_resolved_type(&checked.r#type, &callable.substitutions);
                parameter.cell |= crate::php_closure::requires_owned_cell(&ty)
                    && (checked.take || checked.writable);
            }
        }
        plan
    }

    pub fn arguments(
        &self,
        span: Span,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Vec<GenericArgument> {
        self.call_arguments
            .get(&span)
            .into_iter()
            .flatten()
            .map(|argument| substitute_generic_argument(argument, substitutions))
            .collect()
    }

    pub fn target(
        &self,
        span: Span,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<CallableTarget> {
        self.call_targets
            .get(&span)?
            .specialize(|ty| substitute_resolved_type(ty, substitutions))
    }

    pub fn direct_call(
        &self,
        span: Span,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<&Callable> {
        let (declaration, class) = match self.target(span, substitutions)? {
            CallableTarget::Function { name } => (*self.functions.get(&name)?, None),
            CallableTarget::Method {
                class_type,
                method_name,
                ..
            } => {
                let class = *self.classes.get(&class_type)?;
                (*self.methods.get(&(class, method_name))?, Some(class))
            }
            CallableTarget::InterfaceMethod { .. } => return None,
            CallableTarget::ConstrainedMethod { .. } => return None,
        };
        let arguments = self.arguments(span, substitutions);
        self.callables.iter().find(|callable| {
            callable.declaration == declaration
                && callable.class == class
                && callable.arguments == arguments
        })
    }

    pub fn call_symbol(
        &self,
        span: Span,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<&str> {
        if let Some(callable) = self.direct_call(span, substitutions) {
            return Some(&callable.symbol);
        }
        let CallableTarget::InterfaceMethod { method_name, .. } =
            self.target(span, substitutions)?
        else {
            return None;
        };
        let arguments = self.arguments(span, substitutions);
        if arguments.is_empty() {
            let CallableTarget::InterfaceMethod { method_name, .. } =
                self.call_targets.get(&span)?
            else {
                return None;
            };
            Some(method_name)
        } else {
            self.generic_methods
                .get(&(method_name, arguments))
                .map(String::as_str)
        }
    }
}

fn allocate_symbol(prefix: &str, names: &mut HashSet<String>) -> String {
    for index in 0.. {
        let symbol = format!("{prefix}{index}");
        if names.insert(symbol.to_ascii_lowercase()) {
            return symbol;
        }
    }
    unreachable!("a generated symbol is available")
}
