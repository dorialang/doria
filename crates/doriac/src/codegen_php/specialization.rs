//! Emission identities for the shared, checked monomorphization worklist.

use super::*;
use crate::class_layout::ClassId;
use crate::monomorphization::{self, substitute_generic_argument, type_substitutions};
use crate::php_closure::{
    borrowed_value_uses_cell, parameter_copies_value, requires_owned_cell, requires_parameter_cell,
    PhpCallableParameter, PhpCallablePlan,
};
use crate::semantics::{CallableTarget, GenericArgument};
use crate::types::InterfaceType;
use crate::types::{substitute_resolved_type, ClassType};

mod dispatch;

type RequirementKey = (InterfaceType<ResolvedType>, Span, Vec<GenericArgument>);
type ArgumentBindings = Vec<(Option<String>, Vec<BindingId>)>;

#[derive(Clone, Copy)]
pub(super) struct Accessor<'a> {
    pub symbol: &'a str,
    pub callable: &'a PhpCallablePlan,
    pub field_parameters: &'a [Option<ResolvedType>],
}

#[derive(Debug, Clone)]
pub(super) struct Callable {
    pub id: mir::FunctionId,
    pub declaration: Span,
    pub compiler_owned: bool,
    pub class: Option<ClassId>,
    pub arguments: Vec<GenericArgument>,
    pub substitutions: HashMap<String, ResolvedType>,
    pub symbol: String,
    pub field_result: Option<ResolvedType>,
    pub field_parameters: Vec<Option<ResolvedType>>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Plan {
    pub native_standard_io: bool,
    pub callables: Vec<Callable>,
    pub class_symbols: HashMap<ClassType<ResolvedType>, String>,
    // One PHP class per enum specialization, so runtime type identity survives.
    pub enum_symbols: HashMap<crate::enums::EnumType, String>,
    classes: HashMap<ClassType<ResolvedType>, ClassId>,
    functions: HashMap<String, Span>,
    methods: HashMap<(ClassId, String), Span>,
    exact_symbols: HashMap<mir::FunctionId, String>,
    dispatch_bridges: Vec<dispatch::Bridge>,
    requirement_symbols: HashMap<RequirementKey, String>,
    generic_methods: HashMap<(String, Vec<GenericArgument>), String>,
    accessor_symbols: HashMap<String, String>,
    result_symbols: HashMap<String, String>,
    argument_symbols: HashMap<String, (String, String)>,
    argument_parameter_prefix: String,
    property_storage_symbols: HashMap<(String, String), (String, String)>,
    call_targets: HashMap<Span, CallableTarget>,
    call_arguments: HashMap<Span, Vec<GenericArgument>>,
    semantic: Rc<SemanticInfo>,
    callable_plans: HashMap<mir::FunctionId, PhpCallablePlan>,
    requirements: HashMap<RequirementKey, PhpCallablePlan>,
    interface_edges: Vec<(mir::FunctionId, RequirementKey)>,
    requirement_fields: HashMap<RequirementKey, Vec<Option<ResolvedType>>>,
    argument_bindings: HashMap<Span, ArgumentBindings>,
    enums: HashMap<ClassType<ResolvedType>, crate::enums::EnumType>,
    enum_instances: HashMap<crate::enums::EnumType, crate::enums::EnumId>,
    interfaces: HashMap<InterfaceType<ResolvedType>, ()>,
}

impl Plan {
    pub(super) fn enum_case(&self, span: Span) -> Option<crate::enums::EnumCaseSource> {
        self.semantic
            .enum_case_constructions
            .get(&span)
            .copied()
            .or_else(|| self.semantic.enum_case_values.get(&span).copied())
    }

    pub(super) fn function_argument_fields(
        &self,
        function: &mir::FunctionType,
    ) -> Vec<Option<ResolvedType>> {
        function
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                if crate::php_closure::parameter_uses_cell(function, index) {
                    return None;
                }
                self.field_type(parameter.ty)
            })
            .collect()
    }

    pub(super) fn function_field_result(
        &self,
        function: &mir::FunctionType,
    ) -> Option<ResolvedType> {
        let mir::ReturnType::Value(ty) = function.return_type else {
            return None;
        };
        self.field_type(ty)
    }

    fn field_type(&self, ty: mir::Type) -> Option<ResolvedType> {
        match ty {
            mir::Type::ByteRegion(kind) => return Some(ResolvedType::ByteRegion(kind)),
            mir::Type::NullableByteRegion(kind) => {
                return Some(ResolvedType::Nullable(Box::new(ResolvedType::ByteRegion(
                    kind,
                ))))
            }
            _ => {}
        }
        let (ty, nullable) = match ty {
            mir::Type::PayloadEnum(ty) => (ty, false),
            mir::Type::NullablePayloadEnum(ty) => (ty, true),
            _ => return None,
        };
        let definition = &self.semantic.enums[ty.id.0];
        let ty = ResolvedType::Enum(definition.source_type.clone());
        let ty = if nullable {
            ResolvedType::Nullable(Box::new(ty))
        } else {
            ty
        };
        value_result::uses_fields(&ty, &self.semantic.enums).then_some(ty)
    }

    pub(super) fn enum_definition(
        &self,
        name: &str,
    ) -> Option<&crate::semantics::EnumSemanticInfo> {
        self.semantic
            .enums
            .iter()
            .find(|definition| definition.name == name)
    }

    /// Concrete applications of a generic enum declaration, in a stable order.
    pub(super) fn enum_specializations(&self, name: &str) -> Vec<crate::enums::EnumType> {
        let mut instances = self
            .semantic
            .enums
            .iter()
            .filter(|definition| {
                definition.source_type.name == name
                    && !definition.source_type.arguments.is_empty()
                    && !definition
                        .source_type
                        .arguments
                        .iter()
                        .any(crate::types::resolved_type_is_symbolic)
            })
            .map(|definition| definition.source_type.clone())
            .collect::<Vec<_>>();
        instances.sort_by_key(|instance| self.enum_symbol(instance));
        instances
    }

    /// The PHP class of an applied enum type; concrete enums keep their name.
    pub(super) fn enum_symbol(&self, ty: &crate::enums::EnumType) -> String {
        self.enum_symbols
            .get(ty)
            .cloned()
            .unwrap_or_else(|| php_type_symbol(&ty.name))
    }

    pub(super) fn enum_definition_for_type(
        &self,
        ty: &crate::enums::EnumType,
    ) -> Option<&crate::semantics::EnumSemanticInfo> {
        self.enum_definition_by_id(*self.enum_instances.get(ty)?)
    }

    pub(super) fn enum_definition_by_id(
        &self,
        id: crate::enums::EnumId,
    ) -> Option<&crate::semantics::EnumSemanticInfo> {
        self.semantic.enums.get(id.0)
    }

    /// The PHP class of a MIR enum; MIR enum ids name semantic instances.
    pub(super) fn enum_symbol_by_id(&self, id: crate::enums::EnumId) -> String {
        self.enum_definition_by_id(id).map_or_else(
            || php_type_symbol(&self.semantic.enums[id.0].name),
            |definition| self.enum_symbol(&definition.source_type),
        )
    }

    /// The PHP class of a checked enum case expression; a generic enum's case
    /// names the class of its applied type, not of its declaration.
    pub(super) fn enum_case_symbol(
        &self,
        span: Span,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<String> {
        if !self.semantic.enum_case_constructions.contains_key(&span)
            && !self.semantic.enum_case_values.contains_key(&span)
        {
            return None;
        }
        let ty = substitute_resolved_type(self.semantic.expression_type(span)?, substitutions);
        match ty {
            ResolvedType::Enum(ty) => Some(self.enum_symbol(&ty)),
            ResolvedType::Nullable(inner) => match *inner {
                ResolvedType::Enum(ty) => Some(self.enum_symbol(&ty)),
                _ => None,
            },
            _ => None,
        }
    }

    pub(super) fn has_checked_effects(&self, span: Span) -> bool {
        !crate::checked_effects::effects_at(&self.semantic.checked_effect_sites, span).is_empty()
    }

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
                        ClassType::new(
                            definition.source_type.name.clone(),
                            definition.source_type.arguments.clone(),
                        ),
                        definition.source_type.clone(),
                    )
                })
                .collect(),
            enum_instances: program
                .semantic_info
                .enums
                .iter()
                .map(|definition| (definition.source_type.clone(), definition.id))
                .collect(),
            interfaces: program
                .semantic_info
                .contracts
                .interface_specializations
                .iter()
                .filter(|interface| interface.valid)
                .map(|interface| (interface.specialization.clone(), ()))
                .collect(),
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
                        let binding =
                            crate::php_closure::argument_place_bindings(program, &argument.value);
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
        for requirement in program
            .semantic_info
            .contracts
            .interfaces
            .iter()
            .flat_map(|interface| &interface.requirements)
        {
            if let Some(kind) = requirement.accessor {
                let name = crate::property_hooks::accessor_name(&requirement.name, kind);
                plan.accessor_symbols
                    .entry(name)
                    .or_insert_with(|| allocate_symbol("__doriaAccessor", &mut names));
            }
        }
        for property in program
            .items
            .iter()
            .flat_map(|item| match item {
                Item::Class(class) => class.members.as_slice(),
                _ => &[],
            })
            .filter_map(|member| match member {
                ClassMember::Property(property) => property.hooks.as_ref(),
                _ => None,
            })
        {
            for accessor in &property.accessors {
                plan.accessor_symbols
                    .entry(accessor.function.name.clone())
                    .or_insert_with(|| allocate_symbol("__doriaAccessor", &mut names));
            }
        }
        let class_packages = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(class) => Some((class.name.as_str(), &class.package)),
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        for (index, instance) in instances.iter().enumerate() {
            let declaration = declarations[instance.declaration];
            let function = declaration.function;
            // Member ownership comes from its checked declaring class. HIR
            // member declarations do not independently carry resolved package
            // ownership; synthetic user constructors still belong to users.
            let package = if let Some(id) = declaration.class {
                let class = program
                    .semantic_info
                    .classes
                    .iter()
                    .find(|class| class.id == id)
                    .expect("checked callable has a concrete declaring class");
                class_packages[class.declaration_name.as_str()]
            } else {
                &function.package
            };
            if let Some(class) = declaration.class {
                plan.methods
                    .insert((class, function.name.clone()), function.span);
            } else {
                plan.functions.insert(function.name.clone(), function.span);
            }
            let symbol = if declaration.class.is_some() {
                if let Some(symbol) = plan.accessor_symbols.get(&function.name) {
                    symbol.clone()
                } else if instance.arguments.is_empty() {
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
            let substitutions = type_substitutions(declaration, &instance.arguments)
                .map_err(BackendError::from_diagnostics)?;
            let field_result = function
                .return_type
                .as_ref()
                .and_then(|ty| plan.resolve_type(ty, &substitutions))
                .filter(|ty| value_result::uses_fields(ty, &program.semantic_info.enums));
            plan.callables.push(Callable {
                id: mir::FunctionId(index),
                declaration: function.span,
                compiler_owned: *package == crate::names::PackageIdentity::CompilerKnown,
                class: declaration.class,
                arguments: instance.arguments.clone(),
                substitutions,
                symbol,
                field_result,
                field_parameters: Vec::new(),
            });
            if program
                .semantic_info
                .method_hierarchy
                .get(&function.span)
                .is_some_and(|method| method.virtual_root.is_some())
                && program
                    .semantic_info
                    .exact_method_calls
                    .iter()
                    .any(|(_, declaration)| *declaration == function.span)
            {
                plan.exact_symbols.insert(
                    mir::FunctionId(index),
                    allocate_symbol("__doriaExactMethod", &mut names),
                );
            }
        }
        let mut result_names = names;
        let mut names = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(class) => Some(php_type_symbol(&class.name).to_ascii_lowercase()),
                Item::Enum(enumeration) => {
                    Some(php_type_symbol(&enumeration.name).to_ascii_lowercase())
                }
                _ => None,
            })
            .collect();
        for class in &program.semantic_info.classes {
            let symbol = if class.arguments.is_empty() {
                php_type_symbol(&class.declaration_name)
            } else {
                allocate_symbol("__DoriaClassSpecialization", &mut names)
            };
            plan.class_symbols.insert(
                ClassType::new(class.declaration_name.clone(), class.arguments.clone()),
                symbol,
            );
        }
        for definition in &program.semantic_info.enums {
            let symbol = if definition.source_type.arguments.is_empty() {
                php_type_symbol(&definition.name)
            } else {
                allocate_symbol("__DoriaEnumSpecialization", &mut names)
            };
            plan.enum_symbols
                .insert(definition.source_type.clone(), symbol);
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
                    let returned = substitute_resolved_type(
                        &requirement.signature.return_type,
                        &substitutions,
                    );
                    let returns_borrow = requirement.return_borrow.is_some_and(|borrow| {
                        borrow.kind == crate::types::ReturnBorrowKind::Value
                            && borrowed_value_uses_cell(&returned)
                    });
                    let callable = PhpCallablePlan {
                        consumes_receiver: requirement.receiver_mode.is_consuming(),
                        receiver_home: requirement.return_borrow.is_some_and(|borrow| {
                            returns_borrow
                                && borrow.source == crate::symbols::BorrowSource::Receiver
                        }),
                        lifecycle_receiver: false,
                        returns_borrow,
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
                                    cell: requires_parameter_cell(
                                        &ty,
                                        parameter.take,
                                        parameter.writable,
                                        &program.semantic_info.enums,
                                    ) || requirement.return_borrow.is_some_and(|borrow| {
                                        returns_borrow
                                            && borrow.source
                                                == crate::symbols::BorrowSource::Parameter(index)
                                    }),
                                }
                            })
                            .collect(),
                    };
                    for origin in &requirement.origins {
                        let key = (
                            interface.specialization.clone(),
                            origin.declaration,
                            arguments.clone(),
                        );
                        plan.requirement_fields.insert(
                            key.clone(),
                            requirement
                                .signature
                                .parameters
                                .iter()
                                .map(|parameter| {
                                    let ty =
                                        substitute_resolved_type(&parameter.r#type, &substitutions);
                                    value_result::uses_fields(&ty, &program.semantic_info.enums)
                                        .then_some(ty)
                                })
                                .collect(),
                        );
                        plan.requirements.insert(key, callable.clone());
                    }
                }
            }
        }
        let callable_ids = plan
            .callables
            .iter()
            .map(|callable| {
                (
                    crate::semantics::CallableInstanceKey::new(
                        callable.declaration,
                        &callable.substitutions,
                    ),
                    callable.id,
                )
            })
            .collect::<HashMap<_, _>>();
        for conformance in &program.semantic_info.contracts.conformances {
            if conformance.status != crate::semantics::contracts::ConformanceStatus::Checked {
                continue;
            }
            let ResolvedType::Class(class) = &conformance.implementing_type else {
                continue;
            };
            if !plan.classes.contains_key(class) {
                continue;
            }
            let witnesses = program
                .semantic_info
                .interface_witnesses
                .conformance(conformance)
                .map_err(BackendError::from_diagnostics)?;
            for key in plan
                .requirements
                .keys()
                .filter(|key| key.0 == conformance.interface)
            {
                let witness = witnesses
                    .iter()
                    .find(|witness| {
                        witness
                            .origins
                            .iter()
                            .any(|origin| origin.declaration == key.1)
                    })
                    .ok_or_else(|| {
                        BackendError::from_diagnostics(vec![Diagnostic::new(
                            "I2401",
                            "checked interface requirement has no canonical witness",
                            key.1,
                        )])
                    })?;
                let instance = witness
                    .instance(&key.2, key.1)
                    .map_err(BackendError::from_diagnostics)?;
                let id = callable_ids.get(&instance).copied().ok_or_else(|| {
                    BackendError::from_diagnostics(vec![Diagnostic::new(
                        "I2401",
                        "checked interface witness has no materialized callable",
                        witness.declaration,
                    )])
                })?;
                plan.interface_edges.push((id, key.clone()));
            }
        }
        // Propagate parameter homes only across the same checked specialization.
        // Source spans alone conflate e.g. Read<int> and Read<OwnedObject>.
        loop {
            let mut changed = false;
            for callable in &plan.callables {
                let homes = plan.argument_homes(&callable.substitutions);
                let function = declarations[instances[callable.id.0].declaration].function;
                for (index, parameter) in function.params.iter().enumerate() {
                    let Some(binding) = crate::php_closure::binding_declared_in_span(
                        program,
                        &parameter.name,
                        parameter.span,
                    )
                    .filter(|binding| homes.contains(binding)) else {
                        continue;
                    };
                    let declaration =
                        &program.semantic_info.binding_resolution.declarations_by_id[&binding];
                    let Some(ty) = &declaration.source_type else {
                        continue;
                    };
                    let ty = substitute_resolved_type(ty, &callable.substitutions);
                    // Copy inputs keep their own snapshot; only borrowed Move
                    // inputs may forward their caller's physical storage home.
                    if declaration.ownership == crate::symbols::BindingOwnership::Owned
                        || parameter_copies_value(&ty, &plan.semantic.enums)
                        || !requires_owned_cell(&ty)
                    {
                        continue;
                    }
                    let parameter = &mut plan
                        .callable_plans
                        .get_mut(&callable.id)
                        .unwrap()
                        .parameters[index];
                    changed |= !parameter.cell;
                    parameter.cell = true;
                }
            }
            for callable in &plan.callables {
                let Some(class) = callable.class else {
                    continue;
                };
                let Some(root) = program
                    .semantic_info
                    .method_hierarchy
                    .get(&callable.declaration)
                    .and_then(|method| method.virtual_root)
                else {
                    continue;
                };
                let ancestors = &program.semantic_info.classes[class.0].ancestors;
                for root_callable in plan.callables.iter().filter(|candidate| {
                    candidate.declaration == root
                        && candidate.arguments == callable.arguments
                        && candidate.class.is_some_and(|owner| {
                            owner == class
                                || ancestors
                                    .iter()
                                    .any(|ancestor| plan.classes.get(ancestor) == Some(&owner))
                        })
                }) {
                    let home = plan.callable_plans[&callable.id].receiver_home
                        || plan.callable_plans[&root_callable.id].receiver_home;
                    for id in [callable.id, root_callable.id] {
                        let body = plan.callable_plans.get_mut(&id).unwrap();
                        changed |= body.receiver_home != home;
                        body.receiver_home = home;
                    }
                    let cells = plan.callable_plans[&callable.id]
                        .parameters
                        .iter()
                        .zip(&plan.callable_plans[&root_callable.id].parameters)
                        .map(|(body, root)| body.cell || root.cell)
                        .collect::<Vec<_>>();
                    for id in [callable.id, root_callable.id] {
                        for (parameter, cell) in plan
                            .callable_plans
                            .get_mut(&id)
                            .unwrap()
                            .parameters
                            .iter_mut()
                            .zip(&cells)
                        {
                            changed |= parameter.cell != *cell;
                            parameter.cell = *cell;
                        }
                    }
                }
            }
            for (id, key) in &plan.interface_edges {
                let requirement = plan.requirements.get_mut(key).unwrap();
                let body = plan.callable_plans.get_mut(id).unwrap();
                let home = body.receiver_home || requirement.receiver_home;
                changed |= body.receiver_home != home || requirement.receiver_home != home;
                body.receiver_home = home;
                requirement.receiver_home = home;
                for (body, requirement) in
                    body.parameters.iter_mut().zip(&mut requirement.parameters)
                {
                    let cell = body.cell || requirement.cell;
                    changed |= body.cell != cell || requirement.cell != cell;
                    body.cell = cell;
                    requirement.cell = cell;
                }
            }
            if !changed {
                break;
            }
        }
        plan.native_standard_io =
            super::standard_io::required(program, &plan, &declarations, &instances);
        plan.prepare_dispatch(&mut result_names)?;
        let mut symbols = plan
            .callables
            .iter()
            .map(|callable| callable.symbol.to_ascii_lowercase())
            .chain(
                plan.exact_symbols
                    .values()
                    .map(|name| name.to_ascii_lowercase()),
            )
            .chain(
                plan.accessor_symbols
                    .values()
                    .map(|name| name.to_ascii_lowercase()),
            )
            .chain(
                plan.generic_methods
                    .values()
                    .map(|name| name.to_ascii_lowercase()),
            )
            .chain(
                plan.dispatch_bridges
                    .iter()
                    .map(|bridge| bridge.symbol.to_ascii_lowercase()),
            )
            .collect::<std::collections::BTreeSet<_>>();
        for target in plan.call_targets.values() {
            if let CallableTarget::InterfaceMethod { method_name, .. } = target {
                symbols.insert(method_name.to_ascii_lowercase());
            }
        }
        result_names.extend(symbols.iter().cloned());
        for symbol in symbols {
            plan.result_symbols.insert(
                symbol.clone(),
                allocate_symbol("__doriaFieldResult", &mut result_names),
            );
            plan.argument_symbols.insert(
                symbol,
                (
                    allocate_symbol("__doriaFieldArguments", &mut result_names),
                    allocate_symbol("__doriaFieldArgumentsResult", &mut result_names),
                ),
            );
        }
        for (callable, instance) in plan.callables.iter_mut().zip(&instances) {
            if plan.callable_plans[&callable.id].returns_borrow {
                callable.field_result = None;
            }
            let function = declarations[instance.declaration].function;
            let Some(signature) = program
                .semantic_info
                .callable_signatures
                .get(&function.span)
                .filter(|_| {
                    value_result::supports_intrinsic_arguments(function.intrinsic)
                        && !matches!(function.name.as_str(), "__construct" | "__destruct")
                })
            else {
                callable.field_parameters = vec![None; function.params.len()];
                continue;
            };
            callable.field_parameters = signature
                .parameters
                .iter()
                .zip(&plan.callable_plans[&callable.id].parameters)
                .map(|(parameter, home)| {
                    let ty = substitute_resolved_type(&parameter.r#type, &callable.substitutions);
                    (!home.cell && value_result::uses_fields(&ty, &program.semantic_info.enums))
                        .then_some(ty)
                })
                .collect();
        }
        plan.argument_parameter_prefix = (0..)
            .map(|index| format!("__doriaFieldParameter{index}_"))
            .find(|prefix| {
                !program
                    .semantic_info
                    .binding_resolution
                    .declarations_by_id
                    .values()
                    .any(|binding| binding.name.starts_with(prefix))
                    && !program
                        .semantic_info
                        .enums
                        .iter()
                        .flat_map(|definition| &definition.cases)
                        .flat_map(|case| &case.payload)
                        .any(|field| field.name.starts_with(prefix))
            })
            .expect("an unused field-parameter prefix exists");
        plan.coordinate_argument_fields(program);
        result_names.extend(program.semantic_info.classes.iter().flat_map(|class| {
            class
                .properties
                .iter()
                .map(|property| property.name.to_ascii_lowercase())
        }));
        result_names.extend(
            program
                .items
                .iter()
                .flat_map(|item| match item {
                    Item::Class(class) => class.members.as_slice(),
                    _ => &[],
                })
                .filter_map(|member| match member {
                    ClassMember::Property(property) => Some(property.name.to_ascii_lowercase()),
                    ClassMember::Constant(constant) => Some(constant.name.to_ascii_lowercase()),
                    _ => None,
                }),
        );
        for property in program
            .semantic_info
            .classes
            .iter()
            .flat_map(|class| &class.properties)
        {
            if property.borrowed_source.is_some()
                || !value_result::uses_fields(&property.ty, &program.semantic_info.enums)
            {
                continue;
            }
            plan.property_storage_symbols
                .entry((property.declaring_class.clone(), property.name.clone()))
                .or_insert_with(|| {
                    (
                        allocate_symbol("__doriaPropertyStorage", &mut result_names),
                        allocate_symbol("__doriaPropertyCell", &mut result_names),
                    )
                });
        }
        Ok(plan)
    }

    fn property_info(
        &self,
        class: ClassId,
        name: &str,
    ) -> Option<&crate::semantics::PropertySemanticInfo> {
        self.semantic
            .classes
            .iter()
            .find(|value| value.id == class)?
            .properties
            .iter()
            .rev()
            .find(|property| property.name == name)
    }

    pub(super) fn property_storage_symbols(
        &self,
        class: ClassId,
        name: &str,
    ) -> Option<(&str, &str)> {
        let property = self.property_info(class, name)?;
        if property.borrowed_source.is_some()
            || !value_result::uses_fields(&property.ty, &self.semantic.enums)
        {
            return None;
        }
        self.property_storage_symbols
            .get(&(property.declaring_class.clone(), property.name.clone()))
            .map(|(storage, cell)| (storage.as_str(), cell.as_str()))
    }

    pub(super) fn has_property_storage(&self, class: &str, name: &str) -> bool {
        self.property_storage_symbols
            .contains_key(&(class.to_string(), name.to_string()))
    }

    pub(super) fn property_receiver_class(
        &self,
        receiver: &ResolvedType,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<ClassId> {
        let ty = substitute_resolved_type(receiver, substitutions);
        let mut receiver = &ty;
        while let ResolvedType::Nullable(inner) | ResolvedType::SharedHandle(_, inner) = receiver {
            receiver = inner;
        }
        let ResolvedType::Class(class) = receiver else {
            return None;
        };
        self.classes.get(class).copied()
    }

    fn coordinate_argument_fields(&mut self, program: &Program) {
        for (key, fields) in &mut self.requirement_fields {
            for (field, parameter) in fields.iter_mut().zip(&self.requirements[key].parameters) {
                if parameter.cell {
                    *field = None;
                }
            }
        }
        let mut virtual_edges = Vec::new();
        for callable in &self.callables {
            let Some(class) = callable.class else {
                continue;
            };
            let Some(root) = program
                .semantic_info
                .method_hierarchy
                .get(&callable.declaration)
                .and_then(|method| method.virtual_root)
            else {
                continue;
            };
            let ancestors = &program.semantic_info.classes[class.0].ancestors;
            virtual_edges.extend(
                self.callables
                    .iter()
                    .filter(|candidate| {
                        candidate.declaration == root
                            && candidate.arguments == callable.arguments
                            && candidate.class.is_some_and(|owner| {
                                owner == class
                                    || ancestors
                                        .iter()
                                        .any(|ancestor| self.classes.get(ancestor) == Some(&owner))
                            })
                    })
                    .map(|root| (callable.id.0, root.id.0)),
            );
        }
        for conformance in &program.semantic_info.contracts.conformances {
            if conformance.status != crate::semantics::contracts::ConformanceStatus::Checked {
                continue;
            }
            // A symbolic template conformance has no runtime instance to
            // dispatch to, so it cannot constrain the family's physical shape.
            if matches!(&conformance.implementing_type, ResolvedType::Class(class) if self.classes.contains_key(class))
                || crate::types::resolved_type_is_symbolic(&conformance.implementing_type)
            {
                continue;
            }
            for implementation in &conformance.implementations {
                for origin in &implementation.requirement_origins {
                    for (_, fields) in self.requirement_fields.iter_mut().filter(|(key, _)| {
                        key.0 == conformance.interface && key.1 == origin.declaration
                    }) {
                        fields.fill(None);
                    }
                }
            }
        }
        // Every implementation of one checked dispatch family uses one physical shape.
        loop {
            let mut changed = false;
            for &(left, right) in &virtual_edges {
                if left == right {
                    continue;
                }
                let (first, second) = if left < right {
                    (left, right)
                } else {
                    (right, left)
                };
                let (before, after) = self.callables.split_at_mut(second);
                changed |= intersect_field_parameters(
                    &mut before[first].field_parameters,
                    &mut after[0].field_parameters,
                );
            }
            for (id, key) in &self.interface_edges {
                changed |= intersect_field_parameters(
                    &mut self.callables[id.0].field_parameters,
                    self.requirement_fields.get_mut(key).unwrap(),
                );
            }
            if !changed {
                break;
            }
        }
    }

    pub fn callable(&self, id: mir::FunctionId) -> &Callable {
        &self.callables[id.0]
    }

    pub(super) fn exact_symbol(&self, id: mir::FunctionId) -> Option<&str> {
        self.exact_symbols.get(&id).map(String::as_str)
    }

    fn selected_symbol<'a>(&'a self, span: Span, callable: &'a Callable) -> &'a str {
        if self
            .semantic
            .exact_method_calls
            .contains(&(span, callable.declaration))
        {
            if let Some(symbol) = self.exact_symbol(callable.id) {
                return symbol;
            }
        }
        &callable.symbol
    }

    pub(super) fn result_symbol(&self, symbol: &str) -> &str {
        &self.result_symbols[&symbol.to_ascii_lowercase()]
    }

    pub(super) fn argument_symbol(&self, symbol: &str, result_fields: bool) -> &str {
        let symbols = &self.argument_symbols[&symbol.to_ascii_lowercase()];
        if result_fields {
            &symbols.1
        } else {
            &symbols.0
        }
    }

    pub(super) fn argument_parameter_name(&self, index: usize, component: &str) -> String {
        format!("{}{index}_{component}", self.argument_parameter_prefix)
    }

    pub(super) fn argument_fields(
        &self,
        span: Span,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<&[Option<ResolvedType>]> {
        if let Some(callable) = self.direct_call(span, substitutions) {
            return Some(&callable.field_parameters);
        }
        let CallableTarget::InterfaceMethod {
            interface,
            requirement,
            ..
        } = self.target(span, substitutions)?
        else {
            return None;
        };
        self.requirement_fields
            .get(&(interface, requirement, self.arguments(span, substitutions)))
            .map(Vec::as_slice)
    }

    pub(super) fn argument_defaults(
        &self,
        span: Span,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<Vec<Option<ConstValue>>> {
        let declaration = if let Some(callable) = self.direct_call(span, substitutions) {
            callable.declaration
        } else {
            let CallableTarget::InterfaceMethod { requirement, .. } =
                self.target(span, substitutions)?
            else {
                return None;
            };
            requirement
        };
        let signature = self.semantic.callable_signatures.get(&declaration)?;
        Some(
            signature
                .parameters
                .iter()
                .enumerate()
                .map(|(parameter_index, parameter)| {
                    parameter.has_default.then(|| {
                        self.semantic
                            .parameter_defaults
                            .get(&ParameterDefaultKey {
                                function: declaration,
                                parameter_index,
                            })
                            .expect("checked parameter default is folded")
                            .clone()
                    })
                })
                .collect(),
        )
    }

    pub(super) fn field_call_symbol(
        &self,
        span: Span,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<&str> {
        if let Some(callable) = self.direct_call(span, substitutions) {
            callable.field_result.as_ref()?;
            return Some(self.result_symbol(self.selected_symbol(span, callable)));
        }
        if let Some(accessor) =
            self.accessor(span, crate::ast::PropertyHookKind::Get, substitutions)
        {
            return (!accessor.callable.returns_borrow)
                .then(|| self.result_symbol(accessor.symbol));
        }
        if matches!(
            self.target(span, substitutions),
            Some(CallableTarget::InterfaceMethod { .. })
        ) {
            let symbol = self.call_symbol(span, substitutions)?;
            return Some(self.result_symbol(symbol));
        }
        None
    }

    pub(super) fn accessor_symbol(
        &self,
        property: &str,
        kind: crate::ast::PropertyHookKind,
    ) -> &str {
        &self.accessor_symbols[&crate::property_hooks::accessor_name(property, kind)]
    }

    pub(super) fn stream_accessor(
        &self,
        receiver: &ResolvedType,
        operation: crate::compiler_known_stream::StreamOperation,
    ) -> Option<(&str, &PhpCallablePlan)> {
        let (_, plan, _, symbol) = self.canonical_requirement(receiver, |_, span| {
            crate::compiler_known_stream::StreamOperation::from_requirement(span) == Some(operation)
        })?;
        Some((symbol, plan))
    }

    pub(super) fn stream_capability(
        &self,
        receiver: &ResolvedType,
        operation: crate::compiler_known_stream::StreamOperation,
    ) -> bool {
        self.canonical_requirement(receiver, |_, span| {
            crate::compiler_known_stream::StreamOperation::from_requirement(span) == Some(operation)
        })
        .is_some()
    }

    pub(super) fn stream_callable(
        &self,
        receiver: &ResolvedType,
        operation: crate::compiler_known_stream::StreamOperation,
    ) -> Option<(&str, &PhpCallablePlan, &[Option<ResolvedType>])> {
        let (_, plan, fields, symbol) = self.canonical_requirement(receiver, |_, span| {
            crate::compiler_known_stream::StreamOperation::from_requirement(span) == Some(operation)
        })?;
        Some((symbol, plan, fields))
    }

    pub(super) fn iteration_callable(
        &self,
        receiver: &ResolvedType,
        operation: crate::compiler_known_contracts::IterationOperation,
    ) -> Option<(
        &crate::semantics::contracts::RequirementFacts,
        &PhpCallablePlan,
        &str,
    )> {
        self.canonical_requirement(receiver, |_, span| {
            crate::compiler_known_contracts::IterationOperation::from_requirement(span)
                == Some(operation)
        })
        .map(|(requirement, plan, _, symbol)| (requirement, plan, symbol))
    }

    pub(super) fn core_callable(
        &self,
        receiver: &ResolvedType,
        operation: crate::compiler_known_contracts::CoreValueOperation,
    ) -> Option<(&str, &PhpCallablePlan)> {
        if !self.core_capability(receiver, operation) {
            return None;
        }
        let receiver = match receiver {
            ResolvedType::Nullable(inner) => inner.as_ref(),
            _ => receiver,
        };
        let (_, plan, _, symbol) = self.canonical_requirement(receiver, |requirement, span| {
            crate::compiler_known_contracts::CoreValueOperation::from_requirement(span)
                == Some(operation)
                && (!matches!(
                    operation,
                    crate::compiler_known_contracts::CoreValueOperation::Equal
                        | crate::compiler_known_contracts::CoreValueOperation::Compare
                ) || requirement
                    .signature
                    .parameters
                    .first()
                    .is_some_and(|parameter| &parameter.r#type == receiver))
        })?;
        Some((symbol, plan))
    }

    fn canonical_requirement(
        &self,
        receiver: &ResolvedType,
        matches: impl Fn(&crate::semantics::contracts::RequirementFacts, Span) -> bool,
    ) -> Option<(
        &crate::semantics::contracts::RequirementFacts,
        &PhpCallablePlan,
        &[Option<ResolvedType>],
        &str,
    )> {
        let receiver = match receiver {
            ResolvedType::Nullable(inner) => inner.as_ref(),
            _ => receiver,
        };
        let interfaces = match receiver {
            ResolvedType::Interface(interface) => vec![interface],
            ResolvedType::Class(_) => self
                .semantic
                .contracts
                .conformances
                .iter()
                .filter(|fact| {
                    fact.status == crate::semantics::contracts::ConformanceStatus::Checked
                        && &fact.implementing_type == receiver
                })
                .map(|fact| &fact.interface)
                .collect(),
            _ => return None,
        };
        for interface in interfaces {
            let Some(facts) = self
                .semantic
                .contracts
                .interface_specializations
                .iter()
                .find(|facts| facts.valid && &facts.specialization == interface)
            else {
                continue;
            };
            for requirement in &facts.requirements {
                let Some(origin) = requirement
                    .origins
                    .iter()
                    .find(|origin| matches(requirement, origin.declaration))
                else {
                    continue;
                };
                let key = (interface.clone(), origin.declaration, Vec::new());
                let symbol = self
                    .requirement_symbols
                    .get(&key)
                    .map(String::as_str)
                    .unwrap_or_else(|| {
                        requirement
                            .accessor
                            .map_or(requirement.name.as_str(), |kind| {
                                self.accessor_symbol(&requirement.name, kind)
                            })
                    });
                return Some((
                    requirement,
                    &self.requirements[&key],
                    &self.requirement_fields[&key],
                    symbol,
                ));
            }
        }
        None
    }

    pub(super) fn accessor(
        &self,
        span: Span,
        kind: crate::ast::PropertyHookKind,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<Accessor<'_>> {
        let calls = self.semantic.property_accessor_calls.get(&span)?;
        let call = match kind {
            crate::ast::PropertyHookKind::Get => calls.getter.as_ref(),
            crate::ast::PropertyHookKind::Set => calls.setter.as_ref(),
        }?;
        let target = call
            .target
            .specialize(|ty| substitute_resolved_type(ty, substitutions))
            .expect("checked accessor has a concrete target");
        let (symbol, callable, field_parameters) = match target {
            CallableTarget::Method {
                class_type,
                method_name,
                ..
            } => {
                let class = self.classes[&class_type];
                let name = crate::property_hooks::accessor_name(&method_name, kind);
                let declaration = self.methods[&(class, name)];
                let callable = self
                    .callables
                    .iter()
                    .find(|callable| {
                        callable.declaration == declaration && callable.class == Some(class)
                    })
                    .expect("checked accessor has a callable instance");
                (
                    self.selected_symbol(span, callable),
                    &self.callable_plans[&callable.id],
                    callable.field_parameters.as_slice(),
                )
            }
            CallableTarget::InterfaceMethod {
                interface,
                requirement,
                method_name,
                ..
            } => {
                let key = (interface, requirement, Vec::new());
                (
                    self.requirement_symbols
                        .get(&key)
                        .map(String::as_str)
                        .unwrap_or_else(|| self.accessor_symbol(&method_name, kind)),
                    &self.requirements[&key],
                    self.requirement_fields[&key].as_slice(),
                )
            }
            _ => unreachable!("checked accessor specializes to an instance callable"),
        };
        Some(Accessor {
            symbol,
            callable,
            field_parameters,
        })
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
        let class = scopes
            .current_class_id
            .expect("property initializer has a concrete declaring class");
        let property = self
            .property_info(class, name)
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

    pub fn class_symbol_for_static_member(
        &self,
        span: Span,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<&str> {
        let class = self.semantic.static_member_targets.get(&span)?;
        let ResolvedType::Class(class) =
            substitute_resolved_type(&ResolvedType::Class(class.clone()), substitutions)
        else {
            unreachable!("a checked static owner remains a class");
        };
        self.class_symbols.get(&class).map(String::as_str)
    }

    pub fn resolve_type(
        &self,
        ty: &TypeRef,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<ResolvedType> {
        let resolved =
            crate::types::resolved_type_ref_with_checked_functions(ty, substitutions, &|span| {
                self.semantic
                    .function_types_by_span
                    .get(&span)
                    .map(|function| function.ty.clone())
            })?;
        Some(crate::types::resolve_nominal_kinds(
            resolved,
            &self.enums,
            &self.interfaces,
        ))
    }

    pub fn scope(&self, scopes: &PhpNameScopes, callable: &Callable) -> PhpNameScopes {
        let semantic = self
            .semantic
            .with_callable_ownership(callable.declaration, &callable.substitutions);
        let mut scopes = scopes.expression_scope();
        scopes.closure_owner = Some(mir::ClosureOwner::Callable(callable.id));
        scopes.compiler_owned_callable = callable.compiler_owned;
        scopes.executable_effects = self
            .semantic
            .executable_effects
            .instance(callable.declaration, &callable.substitutions)
            .cloned();
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
        scopes.mixed_box_plans.retain(|_, plan| {
            plan.source_type = substitute(&plan.source_type);
            plan.requires_boxing()
        });
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
        closures.binding_resolution = semantic.binding_resolution.clone();
        closures.ownership = semantic.closure_ownership.clone();
        closures.semantic_closures = self.semantic.closures.clone();
        closures.callable_value_calls = self.semantic.callable_value_calls.clone();
        for declaration in closures.binding_resolution.declarations_by_id.values_mut() {
            if let Some(ty) = &mut declaration.source_type {
                *ty = substitute(ty);
            }
            if crate::php_closure::requires_binding_cell(declaration, &self.semantic.enums) {
                closures.cell_bindings.insert(declaration.id);
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
        closures.callables.insert(
            callable.declaration,
            self.callable_plans[&callable.id].clone(),
        );
        self.prepare_call_sites(&mut closures, &callable.substitutions);
        scopes.closure_plan = Rc::new(closures);
        scopes
    }

    pub(super) fn prepare_call_sites(
        &self,
        closures: &mut PhpClosurePlan,
        substitutions: &HashMap<String, ResolvedType>,
    ) {
        closures.call_site_plans.clear();
        for span in self.call_targets.keys() {
            if let Some(plan) = self.call_site_plan(*span, substitutions) {
                closures.call_site_plans.insert(*span, plan.clone());
            }
        }
        closures
            .cell_bindings
            .extend(self.argument_homes(substitutions));
    }

    fn call_site_plan(
        &self,
        span: Span,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<&PhpCallablePlan> {
        if let Some(target) = self.direct_call(span, substitutions) {
            return self.callable_plans.get(&target.id);
        }
        let CallableTarget::InterfaceMethod {
            interface,
            requirement,
            ..
        } = self.target(span, substitutions)?
        else {
            return None;
        };
        self.requirements
            .get(&(interface, requirement, self.arguments(span, substitutions)))
    }

    fn argument_homes(&self, substitutions: &HashMap<String, ResolvedType>) -> HashSet<BindingId> {
        let mut homes = HashSet::new();
        for (span, arguments) in &self.argument_bindings {
            let callable = self.call_site_plan(*span, substitutions);
            let callback = self
                .semantic
                .callable_value_calls
                .get(span)
                .map(|call| substitute_resolved_type(&call.function_type, substitutions));
            let mut positional = 0;
            for (name, bindings) in arguments {
                let index = name
                    .as_ref()
                    .and_then(|name| {
                        callable.and_then(|callable| {
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
                let cell = callable.is_some_and(|callable| {
                    callable.parameters.get(index).is_some_and(|parameter| parameter.cell)
                }) || callback.as_ref().is_some_and(|ty| {
                    matches!(ty, ResolvedType::Function(function) if function.parameters.get(index)
                        .is_some_and(|parameter| parameter.ownership_mode == crate::types::FunctionTypeParameterMode::Writable)
                        || crate::php_closure::function_parameter_requires_return_home(function, index))
                });
                if cell {
                    homes.extend(bindings.iter().copied());
                }
            }
        }
        homes
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
                consumes_receiver: false,
                returns_borrow: false,
                receiver_home: false,
                lifecycle_receiver: false,
            });
        let returned_source =
            if let Some(return_borrow) = closures.callable_return_borrows.get(&callable.id) {
                plan.returns_borrow =
                    return_borrow.is_some_and(|borrow| borrow.kind == mir::ReturnBorrowKind::Value);
                return_borrow.map(|borrow| match borrow.source {
                    mir::BorrowSource::Receiver => crate::symbols::BorrowSource::Receiver,
                    mir::BorrowSource::Parameter(index) => {
                        crate::symbols::BorrowSource::Parameter(index)
                    }
                })
            } else {
                self.semantic
                    .return_borrows
                    .get(&callable.declaration)
                    .map(|borrow| borrow.source)
            };
        if let Some(signature) = self.semantic.callable_signatures.get(&callable.declaration) {
            let returned =
                substitute_resolved_type(&signature.return_type, &callable.substitutions);
            plan.returns_borrow &= borrowed_value_uses_cell(&returned);
            for (index, (parameter, checked)) in plan
                .parameters
                .iter_mut()
                .zip(&signature.parameters)
                .enumerate()
            {
                let ty = substitute_resolved_type(&checked.r#type, &callable.substitutions);
                if parameter_copies_value(&ty, &self.semantic.enums) || !requires_owned_cell(&ty) {
                    parameter.cell = plan.returns_borrow
                        && returned_source == Some(crate::symbols::BorrowSource::Parameter(index));
                }
                parameter.cell |= requires_parameter_cell(
                    &ty,
                    checked.take,
                    checked.writable,
                    &self.semantic.enums,
                );
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

    /// An explicit `toString` on a Displayable type parameter, which semantic
    /// analysis checks as the receiver's display conversion.
    pub fn constrained_display_call(&self, span: Span) -> bool {
        self.semantic.constrained_display_calls.contains(&span)
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
            return Some(self.selected_symbol(span, callable));
        }
        let CallableTarget::InterfaceMethod {
            interface,
            requirement,
            method_name,
            ..
        } = self.target(span, substitutions)?
        else {
            return None;
        };
        let arguments = self.arguments(span, substitutions);
        if let Some(symbol) =
            self.requirement_symbols
                .get(&(interface, requirement, arguments.clone()))
        {
            return Some(symbol);
        }
        if arguments.is_empty() {
            // Specialization replaces the receiver, never the method name: a
            // call on a type parameter bound to an interface is authored as a
            // constrained call and selected here as an interface call.
            match self.call_targets.get(&span)? {
                CallableTarget::InterfaceMethod { method_name, .. }
                | CallableTarget::ConstrainedMethod { method_name, .. } => Some(method_name),
                _ => None,
            }
        } else {
            self.generic_methods
                .get(&(method_name, arguments))
                .map(String::as_str)
        }
    }
}

fn intersect_field_parameters(
    left: &mut [Option<ResolvedType>],
    right: &mut [Option<ResolvedType>],
) -> bool {
    // These inventories already contain only checked field-backed parameters.
    // Nullable carriers share the payload layout and carry presence separately.
    fn payload(ty: &Option<ResolvedType>) -> Option<&ResolvedType> {
        let mut ty = ty.as_ref()?;
        while let ResolvedType::Nullable(inner) = ty {
            ty = inner;
        }
        Some(ty)
    }
    assert_eq!(left.len(), right.len(), "checked callable family arity");
    let mut changed = false;
    for (left, right) in left.iter_mut().zip(right) {
        if payload(left).is_none() || payload(left) != payload(right) {
            changed |= left.is_some() || right.is_some();
            *left = None;
            *right = None;
        }
    }
    changed
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_constants_emit_checked_application_symbols_and_payloads() {
        use crate::const_eval::{ConstEnumValue, ConstPayloadEnumValue};
        use crate::enums::{EnumCaseSource, EnumId, EnumType};
        let mut program = crate::lower_source(
            "php-enum-constants.doria",
            r#"
enum Status { case Ready; }
enum Packet { case Value(Status $value); }
function main(): void {}
"#,
        )
        .unwrap();
        for definition in &mut program.semantic_info.enums {
            if matches!(definition.name.as_str(), "Status" | "Packet") {
                let declaration = if definition.name == "Status" { 90 } else { 91 };
                definition.source_type = EnumType::with_arguments(
                    EnumId(declaration),
                    definition.name.clone(),
                    vec![ResolvedType::Bool],
                );
                definition.name.push_str("Bool");
            }
        }
        // Applications name their own PHP classes, as `Plan::build` allocates.
        let plan = Plan {
            enum_instances: program
                .semantic_info
                .enums
                .iter()
                .map(|definition| (definition.source_type.clone(), definition.id))
                .collect(),
            enum_symbols: program
                .semantic_info
                .enums
                .iter()
                .filter(|definition| !definition.source_type.arguments.is_empty())
                .map(|definition| {
                    (
                        definition.source_type.clone(),
                        php_type_symbol(&definition.name),
                    )
                })
                .collect(),
            semantic: Rc::new(program.semantic_info),
            ..Plan::default()
        };
        let identity = |name: &str, declaration| ConstEnumValue {
            enum_type: EnumType::with_arguments(
                EnumId(declaration),
                name,
                vec![ResolvedType::Bool],
            ),
            case: EnumCaseSource {
                declaration: EnumId(declaration),
                index: 0,
            },
        };
        let scalar = ConstValue::Enum(identity("Status", 90));
        assert_eq!(
            emit_const_value(&scalar, &plan),
            format!("{}::Ready", php_type_symbol("StatusBool"))
        );
        let payload = ConstValue::PayloadEnum(ConstPayloadEnumValue {
            value: identity("Packet", 91),
            fields: vec![scalar],
            field_types: vec![ResolvedType::TypeParameter("T".into())],
        });
        assert_eq!(
            emit_const_value(&payload, &plan),
            format!(
                "{}::{}({}::Ready)",
                php_type_symbol("PacketBool"),
                php_payload_case_method("Value", true),
                php_type_symbol("StatusBool")
            )
        );
        let mut missing = identity("Packet", 91);
        missing.enum_type.arguments = vec![ResolvedType::String];
        assert!(plan.enum_definition_for_type(&missing.enum_type).is_none());
        let definition = plan
            .enum_definition_for_type(&identity("Packet", 91).enum_type)
            .unwrap();
        assert!(definition
            .resolve_typed_case(&missing.enum_type, missing.case)
            .is_none());
    }

    #[test]
    fn dispatch_families_preserve_matching_region_and_enum_field_shapes() {
        let readonly = ResolvedType::ByteRegion(crate::types::ByteRegionKind::Readonly);
        let writable = ResolvedType::ByteRegion(crate::types::ByteRegionKind::Writable);
        let packet = ResolvedType::Enum(crate::enums::EnumType::new(
            crate::enums::EnumId(0),
            "Packet",
        ));
        let other = ResolvedType::Enum(crate::enums::EnumType::new(
            crate::enums::EnumId(1),
            "Other",
        ));
        let shapes = [readonly, writable, packet, other];
        for (left_index, left) in shapes.iter().enumerate() {
            for (right_index, right) in shapes.iter().enumerate() {
                for nullable in [false, true] {
                    let left = Some(left.clone());
                    let right = Some(if nullable {
                        crate::types::nullable_of(right.clone())
                    } else {
                        right.clone()
                    });
                    let mut actual_left = [left.clone()];
                    let mut actual_right = [right.clone()];
                    let changed = intersect_field_parameters(&mut actual_left, &mut actual_right);
                    let matching = left_index == right_index;
                    assert_eq!(changed, !matching);
                    assert_eq!(actual_left, [if matching { left } else { None }]);
                    assert_eq!(actual_right, [if matching { right } else { None }]);
                    assert!(!intersect_field_parameters(
                        &mut actual_left,
                        &mut actual_right
                    ));
                }
            }
        }
        let mut fields = [Some(shapes[0].clone())];
        let mut boxed = [None];
        assert!(intersect_field_parameters(&mut fields, &mut boxed));
        assert_eq!(fields, [None]);
        assert_eq!(boxed, [None]);
    }

    #[test]
    fn callable_specialization_keeps_checked_retained_borrows_inside_wrappers() {
        let program = crate::lower_source(
            "php-checked-callable.doria",
            r#"
use Doria\Std\Io\ByteRegion;
function apply(function(ByteRegion): ByteRegion $call, ByteRegion $view): ByteRegion {
    return $call($view);
}
function main(): void {}
"#,
        )
        .unwrap();
        let function = program
            .items
            .iter()
            .find_map(|item| match item {
                Item::Function(function) if function.name == "apply" => Some(function),
                _ => None,
            })
            .unwrap();
        let syntax = &function.params[0].ty;
        let span = syntax.function.as_ref().unwrap().span;
        let expected = &program.semantic_info.function_types_by_span[&span].ty;
        let ResolvedType::Function(callable) = expected else {
            panic!("checked annotation must describe a callable");
        };
        assert_eq!(
            callable.return_borrow.unwrap().kind,
            crate::types::ReturnBorrowKind::Retained
        );
        let plan = Plan {
            semantic: Rc::new(program.semantic_info.clone()),
            ..Plan::default()
        };
        let substitutions = HashMap::new();
        assert_eq!(
            plan.resolve_type(syntax, &substitutions).as_ref(),
            Some(expected)
        );
        let wrapped = TypeRef::generic(
            "List",
            vec![TypeRef::grouped(syntax.clone(), span, span).nullable()],
        );
        assert_eq!(
            plan.resolve_type(&wrapped, &substitutions),
            Some(ResolvedType::List(Box::new(crate::types::nullable_of(
                expected.clone()
            ))))
        );
        assert_eq!(Plan::default().resolve_type(&wrapped, &substitutions), None);
    }
}
