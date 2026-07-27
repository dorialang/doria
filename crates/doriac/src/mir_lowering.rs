use std::collections::{HashMap, HashSet};

use crate::class_layout::{compute_class_layout, ClassId, FieldType, PropertyId};
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::format_string::{self, FormatConversion, FormatPiece};
use crate::numeric::{parse_decimal_magnitude, FloatType, FloatValue, IntegerType, IntegerValue};
use crate::semantics::{CallableTarget, GenericArgument, GenericSpecialization, SemanticInfo};
use crate::source::Span;
use crate::types::{resolved_type_complexity, ClassType, ResolvedType};
use crate::{hir, mir};

type ClassIds = HashMap<ClassType<ResolvedType>, ClassId>;

#[derive(Clone, Default)]
struct CollectionRegistry {
    ids: HashMap<(mir::CollectionKind, Option<mir::Type>, mir::Type), mir::CollectionTypeId>,
    types: Vec<mir::CollectionType>,
}

impl CollectionRegistry {
    fn intern(
        &mut self,
        kind: mir::CollectionKind,
        key: Option<mir::Type>,
        value: mir::Type,
    ) -> mir::CollectionTypeId {
        let signature = (kind, key, value);
        if let Some(id) = self.ids.get(&signature) {
            return *id;
        }
        let id = mir::CollectionTypeId(self.types.len());
        self.types.push(mir::CollectionType {
            id,
            kind,
            key,
            value,
        });
        self.ids.insert(signature, id);
        id
    }
}

/// Intrinsics, built-ins, and collection methods bind positionally only —
/// semantic analysis rejects named arguments for them (decision 0098 makes
/// parameter names public API for user callables, not for language intrinsics).
/// Lowering for those paths therefore reads argument expressions directly.
fn argument_values(args: &[hir::Argument]) -> Vec<&hir::Expr> {
    args.iter().map(|argument| &argument.value).collect()
}

#[derive(Clone)]
struct FunctionSignature {
    id: mir::FunctionId,
    return_type: mir::ReturnType,
    return_borrow: Option<mir::ReturnBorrow>,
    parameter_types: Vec<mir::Type>,
    /// Declared parameter names, in declaration order. Named-argument binding
    /// (decision 0098) resolves `name: value` against these.
    parameter_names: Vec<String>,
    parameter_defaults: Vec<Option<crate::const_eval::ConstValue>>,
    parameter_transfers: Vec<bool>,
    parameter_owns: Vec<bool>,
    method_class: Option<ClassId>,
    receiver_mode: Option<mir::ReceiverMode>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CallableInstance {
    declaration: usize,
    arguments: Vec<GenericArgument>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FunctionInstanceKey {
    name: String,
    arguments: Vec<GenericArgument>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MethodInstanceKey {
    class: ClassId,
    name: String,
    arguments: Vec<GenericArgument>,
}

#[derive(Clone)]
struct PropertyInitializer {
    expression: hir::Expr,
    type_substitutions: HashMap<String, ResolvedType>,
}

#[derive(Clone, Copy)]
struct CallableDecl<'a> {
    function: &'a hir::FunctionDecl,
    class: Option<ClassId>,
    receiver: Option<ClassId>,
    class_type_params: &'a [hir::TypeParamDecl],
    class_arguments: &'a [ResolvedType],
}

impl CallableDecl<'_> {
    fn is_top_level(self) -> bool {
        self.class.is_none()
    }
}

fn specialize_callable_instance(
    span: &(usize, usize),
    specialization: &GenericSpecialization,
    substitutions: &HashMap<String, ResolvedType>,
    functions: &HashMap<String, usize>,
    methods: &HashMap<(ClassId, String), usize>,
    class_ids: &ClassIds,
    semantic_info: &SemanticInfo,
) -> DiagnosticResult<CallableInstance> {
    let Some(target) = semantic_info.call_targets.get(span) else {
        return Err(vec![Diagnostic::new(
            "I2401",
            "checked generic call has no callable target",
            Span::new(span.0, span.1),
        )]);
    };
    let declaration = match target {
        CallableTarget::Function { name } => functions.get(name).copied(),
        CallableTarget::Method {
            class_type,
            method_name,
        } => {
            let specialized =
                substitute_resolved_type(&ResolvedType::Class(class_type.clone()), substitutions);
            let ResolvedType::Class(class_type) = specialized else {
                unreachable!("class target substitution must remain a class");
            };
            class_ids
                .get(&class_type)
                .and_then(|class| methods.get(&(*class, method_name.clone())))
                .copied()
        }
    }
    .ok_or_else(|| {
        vec![Diagnostic::new(
            "I2401",
            "checked generic call has no callable declaration",
            Span::new(span.0, span.1),
        )]
    })?;
    let arguments = specialization
        .arguments
        .iter()
        .map(|argument| substitute_generic_argument(argument, substitutions))
        .collect::<Vec<_>>();
    if arguments.iter().any(generic_argument_is_symbolic) {
        return Err(vec![Diagnostic::new(
            "I2401",
            "generic specialization retained an unresolved type parameter",
            Span::new(span.0, span.1),
        )]);
    }
    Ok(CallableInstance {
        declaration,
        arguments,
    })
}

fn collect_callable_instances(
    program: &hir::Program,
    declarations: &[CallableDecl<'_>],
    class_ids: &ClassIds,
    semantic_info: &SemanticInfo,
) -> DiagnosticResult<Vec<CallableInstance>> {
    let functions = declarations
        .iter()
        .enumerate()
        .filter(|(_, declaration)| declaration.is_top_level())
        .map(|(index, declaration)| (declaration.function.name.clone(), index))
        .collect::<HashMap<_, _>>();
    let methods = declarations
        .iter()
        .enumerate()
        .filter_map(|(index, declaration)| {
            declaration
                .class
                .map(|class| ((class, declaration.function.name.clone()), index))
        })
        .collect::<HashMap<_, _>>();

    let mut instances = Vec::new();
    let mut parents = Vec::new();
    let mut ids = HashMap::new();
    for (declaration, callable) in declarations.iter().enumerate() {
        if callable.function.type_params.is_empty() {
            let instance = CallableInstance {
                declaration,
                arguments: Vec::new(),
            };
            ids.insert(instance.clone(), instances.len());
            instances.push(instance);
            parents.push(None);
        }
    }

    let mut calls = semantic_info
        .generic_call_specializations
        .iter()
        .collect::<Vec<_>>();
    calls.sort_by_key(|(span, _)| **span);

    for item in &program.items {
        let hir::Item::Class(class) = item else {
            continue;
        };
        for class_info in semantic_info
            .classes
            .iter()
            .filter(|info| info.declaration_name == class.name)
        {
            let substitutions = class
                .type_params
                .iter()
                .zip(&class_info.arguments)
                .map(|(parameter, argument)| (parameter.name.clone(), argument.clone()))
                .collect::<HashMap<_, _>>();
            for member in &class.members {
                let hir::ClassMember::Property(hir::PropertyDecl {
                    is_static: false,
                    initializer: Some(initializer),
                    ..
                }) = member
                else {
                    continue;
                };
                for (span, specialization) in &calls {
                    if span.0 < initializer.span().start || span.1 > initializer.span().end {
                        continue;
                    }
                    let target = specialize_callable_instance(
                        span,
                        specialization,
                        &substitutions,
                        &functions,
                        &methods,
                        class_ids,
                        semantic_info,
                    )?;
                    if !ids.contains_key(&target) {
                        ids.insert(target.clone(), instances.len());
                        instances.push(target);
                        parents.push(None);
                    }
                }
            }
        }
    }

    let mut cursor = 0;
    while cursor < instances.len() {
        let instance_index = cursor;
        let instance = instances[cursor].clone();
        cursor += 1;
        let callable = declarations[instance.declaration];
        let substitutions = type_substitutions(callable, &instance.arguments)?;
        for (span, specialization) in &calls {
            let in_function =
                span.0 >= callable.function.span.start && span.1 <= callable.function.span.end;
            if !in_function {
                continue;
            }
            let target = specialize_callable_instance(
                span,
                specialization,
                &substitutions,
                &functions,
                &methods,
                class_ids,
                semantic_info,
            )?;
            if !ids.contains_key(&target) {
                if specialization_expands_recursively(&instances, &parents, instance_index, &target)
                {
                    let name = &declarations[target.declaration].function.name;
                    return Err(vec![Diagnostic::new(
                        "E0539",
                        format!(
                            "generic specialization of `{name}` recursively expands its type arguments and has no finite monomorphization"
                        ),
                        Span::new(span.0, span.1),
                    )
                    .with_help(
                        "keep recursive generic calls at the same concrete type, or move the type-changing step outside the recursion",
                    )]);
                }
                ids.insert(target.clone(), instances.len());
                instances.push(target);
                parents.push(Some(instance_index));
            }
        }
    }

    Ok(instances)
}

fn specialization_expands_recursively(
    instances: &[CallableInstance],
    parents: &[Option<usize>],
    current: usize,
    target: &CallableInstance,
) -> bool {
    // One type-changing recursive step can still converge (for example, T -> int).
    // Two consecutive increases for the same declaration establish an expanding
    // specialization chain while keeping bounded type changes valid.
    let mut matching_ancestors = Vec::new();
    let mut cursor = Some(current);
    while let Some(index) = cursor {
        if instances[index].declaration == target.declaration {
            matching_ancestors.push(&instances[index]);
            if matching_ancestors.len() == 2 {
                break;
            }
        }
        cursor = parents[index];
    }
    let [nearest, previous] = matching_ancestors.as_slice() else {
        return false;
    };
    specialization_complexity(&target.arguments) > specialization_complexity(&nearest.arguments)
        && specialization_complexity(&nearest.arguments)
            > specialization_complexity(&previous.arguments)
}

fn specialization_complexity(arguments: &[GenericArgument]) -> usize {
    arguments
        .iter()
        .map(|argument| {
            let GenericArgument::Type(ty) = argument;
            resolved_type_complexity(ty)
        })
        .sum()
}

fn type_substitutions(
    callable: CallableDecl<'_>,
    arguments: &[GenericArgument],
) -> DiagnosticResult<HashMap<String, crate::types::ResolvedType>> {
    let function = callable.function;
    if function.type_params.len() != arguments.len() {
        return Err(vec![Diagnostic::new(
            "I2401",
            format!(
                "generic function `{}` expected {} specialization arguments but received {}",
                function.name,
                function.type_params.len(),
                arguments.len()
            ),
            function.span,
        )]);
    }
    let mut substitutions = callable
        .class_type_params
        .iter()
        .zip(callable.class_arguments)
        .map(|(parameter, argument)| (parameter.name.clone(), argument.clone()))
        .collect::<HashMap<_, _>>();
    substitutions.extend(function.type_params.iter().zip(arguments).map(
        |(parameter, argument)| {
            let GenericArgument::Type(ty) = argument;
            (parameter.name.clone(), ty.clone())
        },
    ));
    Ok(substitutions)
}

fn substitute_generic_argument(
    argument: &GenericArgument,
    substitutions: &HashMap<String, crate::types::ResolvedType>,
) -> GenericArgument {
    match argument {
        GenericArgument::Type(ty) => {
            GenericArgument::Type(substitute_resolved_type(ty, substitutions))
        }
    }
}

fn generic_argument_is_symbolic(argument: &GenericArgument) -> bool {
    let GenericArgument::Type(ty) = argument;
    resolved_type_is_symbolic(ty)
}

fn resolved_type_is_symbolic(ty: &crate::types::ResolvedType) -> bool {
    use crate::types::ResolvedType;
    match ty {
        ResolvedType::TypeParameter(_) => true,
        ResolvedType::Nullable(inner)
        | ResolvedType::TypedArray(inner)
        | ResolvedType::List(inner)
        | ResolvedType::Set(inner) => resolved_type_is_symbolic(inner),
        ResolvedType::Dictionary(key, value) => {
            resolved_type_is_symbolic(key) || resolved_type_is_symbolic(value)
        }
        _ => false,
    }
}

fn substitute_resolved_type(
    ty: &crate::types::ResolvedType,
    substitutions: &HashMap<String, crate::types::ResolvedType>,
) -> crate::types::ResolvedType {
    use crate::types::ResolvedType;
    match ty {
        ResolvedType::TypeParameter(name) => substitutions
            .get(name)
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        ResolvedType::Nullable(inner) => {
            ResolvedType::Nullable(Box::new(substitute_resolved_type(inner, substitutions)))
        }
        ResolvedType::TypedArray(inner) => {
            ResolvedType::TypedArray(Box::new(substitute_resolved_type(inner, substitutions)))
        }
        ResolvedType::List(inner) => {
            ResolvedType::List(Box::new(substitute_resolved_type(inner, substitutions)))
        }
        ResolvedType::Dictionary(key, value) => ResolvedType::Dictionary(
            Box::new(substitute_resolved_type(key, substitutions)),
            Box::new(substitute_resolved_type(value, substitutions)),
        ),
        ResolvedType::Set(inner) => {
            ResolvedType::Set(Box::new(substitute_resolved_type(inner, substitutions)))
        }
        ResolvedType::Class(class) => ResolvedType::Class(ClassType::new(
            class.name.clone(),
            class
                .arguments
                .iter()
                .map(|argument| substitute_resolved_type(argument, substitutions))
                .collect(),
        )),
        _ => ty.clone(),
    }
}

pub fn lower_program(program: &hir::Program) -> DiagnosticResult<mir::Program> {
    let class_ids = program
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
    let mut collection_registry = CollectionRegistry::default();
    let mut static_ids = HashMap::new();
    let mut statics = Vec::new();
    for class_info in &program.semantic_info.classes {
        let class = program
            .items
            .iter()
            .find_map(|item| match item {
                hir::Item::Class(class) if class.name == class_info.declaration_name => Some(class),
                _ => None,
            })
            .expect("specialized class has a declaration");
        let class_id = class_info.id;
        let substitutions = class
            .type_params
            .iter()
            .zip(&class_info.arguments)
            .map(|(parameter, argument)| (parameter.name.clone(), argument.clone()))
            .collect::<HashMap<_, _>>();
        for property in class.members.iter().filter_map(|member| match member {
            hir::ClassMember::Property(property) if property.is_static => Some(property),
            _ => None,
        }) {
            let id = mir::StaticId(statics.len());
            let ty = mir_type_ref_with_substitutions(
                &property.ty,
                &class_ids,
                &mut collection_registry,
                &substitutions,
            )
            .ok_or_else(|| {
                vec![unsupported_native_type(
                    &property.ty,
                    property.span,
                    format!(
                        "static property `{}::{}` has a type not supported by native compilation",
                        class.name, property.name
                    ),
                )]
            })?;
            let key = crate::const_eval::ConstKey::Static {
                class_name: class.name.clone(),
                name: property.name.clone(),
            };
            let evaluated = program
                .semantic_info
                .const_evaluation
                .values
                .get(&key)
                .ok_or_else(|| {
                    vec![unsupported(
                        property.span,
                        format!(
                            "static property `{}::{}` has no evaluated initializer",
                            class.name, property.name
                        ),
                    )]
                })?;
            let initializer = lower_static_value(&evaluated.value, ty, property.span)?;
            static_ids.insert((class_id, property.name.clone()), (id, ty));
            statics.push(mir::StaticProperty {
                id,
                class: class_id,
                name: property.name.clone(),
                ty,
                writable: property.writable,
                initializer,
            });
        }
    }
    let property_initializers = program
        .semantic_info
        .classes
        .iter()
        .flat_map(|class_info| {
            let class = program
                .items
                .iter()
                .find_map(|item| match item {
                    hir::Item::Class(class) if class.name == class_info.declaration_name => {
                        Some(class)
                    }
                    _ => None,
                })
                .expect("specialized class has a declaration");
            let substitutions = class
                .type_params
                .iter()
                .zip(&class_info.arguments)
                .map(|(parameter, argument)| (parameter.name.clone(), argument.clone()))
                .collect::<HashMap<_, _>>();
            class.members.iter().filter_map(move |member| match member {
                hir::ClassMember::Property(property) if !property.is_static => {
                    property.initializer.clone().map(|value| {
                        let property_id = class_info
                            .properties
                            .iter()
                            .find(|info| info.name == property.name)
                            .expect("checked property has a stable identity")
                            .id;
                        (
                            property_id,
                            PropertyInitializer {
                                expression: value,
                                type_substitutions: substitutions.clone(),
                            },
                        )
                    })
                }
                hir::ClassMember::Property(_)
                | hir::ClassMember::Method(_)
                | hir::ClassMember::Constant(_) => None,
            })
        })
        .collect::<HashMap<_, _>>();
    let mut constructor_body_initializers = HashSet::new();
    for class_info in &program.semantic_info.classes {
        let class = program
            .items
            .iter()
            .find_map(|item| match item {
                hir::Item::Class(class) if class.name == class_info.declaration_name => Some(class),
                _ => None,
            })
            .expect("specialized class has a declaration");
        if !class.members.iter().any(|member| {
            matches!(member, hir::ClassMember::Method(method) if method.name == "__construct")
        }) {
            continue;
        }

        for property in &class_info.properties {
            if !property.promoted && !property_initializers.contains_key(&property.id) {
                constructor_body_initializers.insert(property.id);
            }
        }
    }
    let mut declarations = Vec::new();

    for item in &program.items {
        match item {
            hir::Item::Function(function) => declarations.push(CallableDecl {
                function,
                class: None,
                receiver: None,
                class_type_params: &[],
                class_arguments: &[],
            }),
            hir::Item::Class(class_decl) => {
                for class_info in program
                    .semantic_info
                    .classes
                    .iter()
                    .filter(|info| info.declaration_name == class_decl.name)
                {
                    for member in &class_decl.members {
                        if let hir::ClassMember::Method(method) = member {
                            declarations.push(CallableDecl {
                                function: method,
                                class: Some(class_info.id),
                                receiver: (!method.is_static).then_some(class_info.id),
                                class_type_params: &class_decl.type_params,
                                class_arguments: &class_info.arguments,
                            });
                        }
                    }
                }
            }
            hir::Item::Statement(statement) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "top-level executable statements are not supported by native compilation",
                )]);
            }
            hir::Item::Constant(_) => {}
        }
    }

    let main_indices = declarations
        .iter()
        .enumerate()
        .filter_map(|(index, declaration)| {
            (declaration.is_top_level() && declaration.function.name == "main").then_some(index)
        })
        .collect::<Vec<_>>();
    if main_indices.len() != 1 {
        let span = main_indices
            .get(1)
            .map_or_else(Span::default, |index| declarations[*index].function.span);
        return Err(vec![unsupported(
            span,
            "native programs require exactly one top-level `main` function",
        )]);
    }

    let instances =
        collect_callable_instances(program, &declarations, &class_ids, &program.semantic_info)?;
    let mut signatures = HashMap::new();
    let mut method_signatures = HashMap::new();
    let mut callable_signatures = Vec::new();
    let mut instance_substitutions = Vec::new();
    for (index, instance) in instances.iter().enumerate() {
        let declaration = declarations[instance.declaration];
        let function = declaration.function;
        let substitutions = type_substitutions(declaration, &instance.arguments)?;
        intern_block_collection_types(
            &function.body,
            &class_ids,
            &mut collection_registry,
            &substitutions,
        );
        for (span, ty) in &program.semantic_info.expression_types {
            if span.0 >= function.span.start && span.1 <= function.span.end {
                let specialized = substitute_resolved_type(ty, &substitutions);
                let _ = intern_resolved_collection_types(
                    &specialized,
                    &class_ids,
                    &mut collection_registry,
                );
            }
        }
        let mut signature = collect_function_signature(
            function,
            mir::FunctionId(index),
            &class_ids,
            &program.semantic_info,
            &mut collection_registry,
            &substitutions,
            SignatureOptions {
                lifecycle: matches!(function.name.as_str(), "__construct" | "__destruct"),
                is_entry: declaration.is_top_level() && function.name == "main",
            },
        )?;
        signature.method_class = declaration.class;
        signature.receiver_mode = declaration.receiver.map(|_| {
            if function.writable_this {
                mir::ReceiverMode::Writable
            } else {
                mir::ReceiverMode::Readonly
            }
        });
        if declaration.is_top_level() {
            signatures.insert(
                FunctionInstanceKey {
                    name: function.name.clone(),
                    arguments: instance.arguments.clone(),
                },
                signature.clone(),
            );
        } else if let Some(class) = declaration.class {
            method_signatures.insert(
                MethodInstanceKey {
                    class,
                    name: function.name.clone(),
                    arguments: instance.arguments.clone(),
                },
                signature.clone(),
            );
        }
        callable_signatures.push(signature);
        instance_substitutions.push(substitutions);
    }

    for ty in program.semantic_info.expression_types.values() {
        let _ = intern_resolved_collection_types(ty, &class_ids, &mut collection_registry);
    }

    let entry = signatures
        .get(&FunctionInstanceKey {
            name: "main".to_string(),
            arguments: Vec::new(),
        })
        .expect("exactly one collected main signature")
        .id;
    let functions = instances
        .iter()
        .zip(callable_signatures)
        .zip(instance_substitutions)
        .map(|((instance, signature), substitutions)| {
            let declaration = declarations[instance.declaration];
            let inputs = FunctionLoweringInputs {
                signatures: &signatures,
                method_signatures: &method_signatures,
                semantic_info: &program.semantic_info,
                property_initializers: &property_initializers,
                constructor_body_initializers: &constructor_body_initializers,
                static_ids: &static_ids,
                collection_registry: &collection_registry,
                type_substitutions: &substitutions,
            };
            lower_function(
                declaration.function,
                signature,
                inputs,
                declaration.class,
                declaration.receiver,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let classes = program
        .semantic_info
        .classes
        .iter()
        .map(|class| {
            let properties = class
                .properties
                .iter()
                .map(|property| {
                    Ok(mir::Property {
                        id: property.id,
                        name: property.name.clone(),
                        ty: intern_resolved_collection_types(
                            &property.ty,
                            &class_ids,
                            &mut collection_registry,
                        )
                        .ok_or_else(|| {
                            vec![unsupported(
                                Span::default(),
                                format!(
                                    "property `${}` has a type that is not supported by native class compilation",
                                    property.name
                                ),
                            )]
                        })?,
                        writable: property.writable,
                        promoted: property.promoted,
                    })
                })
                .collect::<DiagnosticResult<Vec<_>>>()?;
            let layout = compute_class_layout(
                class.id,
                properties.iter().map(|property| {
                    (
                        property.id,
                        field_type(property.ty).expect("checked native property type"),
                    )
                }),
                std::mem::size_of::<usize>() as u32,
            );
            let lifecycle = |name: &str| {
                instances.iter().enumerate().find_map(|(index, instance)| {
                    let declaration = declarations[instance.declaration];
                    (instance.arguments.is_empty()
                        && declaration.receiver == Some(class.id)
                        && declaration.function.name == name)
                        .then_some(mir::FunctionId(index))
                })
            };
            Ok(mir::Class {
                id: class.id,
                name: class.name.clone(),
                properties,
                layout,
                constructor: lifecycle("__construct"),
                destructor: lifecycle("__destruct"),
            })
        })
        .collect::<DiagnosticResult<Vec<_>>>()?;

    Ok(mir::Program {
        classes,
        collection_types: collection_registry.types,
        statics,
        functions,
        entry,
    })
}

fn intern_resolved_collection_types(
    ty: &crate::types::ResolvedType,
    class_ids: &ClassIds,
    collections: &mut CollectionRegistry,
) -> Option<mir::Type> {
    use crate::types::ResolvedType;
    let ty = match ty {
        ResolvedType::Integer(ty) => mir::Type::Scalar(mir::ScalarType::Integer(*ty)),
        ResolvedType::Float(ty) => mir::Type::Scalar(mir::ScalarType::Float(*ty)),
        ResolvedType::Bool => mir::Type::Scalar(mir::ScalarType::Bool),
        ResolvedType::String => mir::Type::String,
        ResolvedType::Bytes => mir::Type::Collection(intern_bytes_type(collections)),
        ResolvedType::Mixed => mir::Type::Mixed,
        ResolvedType::Class(class) => mir::Type::Class(*class_ids.get(class)?),
        ResolvedType::TypedArray(value) => {
            let value = intern_resolved_collection_types(value, class_ids, collections)?;
            mir::Type::Collection(collections.intern(mir::CollectionKind::TypedArray, None, value))
        }
        ResolvedType::List(value) => {
            let value = intern_resolved_collection_types(value, class_ids, collections)?;
            mir::Type::Collection(collections.intern(mir::CollectionKind::List, None, value))
        }
        ResolvedType::Dictionary(key, value) => {
            let key = intern_resolved_collection_types(key, class_ids, collections)?;
            let value = intern_resolved_collection_types(value, class_ids, collections)?;
            mir::Type::Collection(collections.intern(
                mir::CollectionKind::Dictionary,
                Some(key),
                value,
            ))
        }
        ResolvedType::Set(value) => {
            let value = intern_resolved_collection_types(value, class_ids, collections)?;
            mir::Type::Collection(collections.intern(mir::CollectionKind::Set, None, value))
        }
        ResolvedType::Nullable(inner) => {
            match intern_resolved_collection_types(inner, class_ids, collections)? {
                mir::Type::Scalar(ty) => mir::Type::NullableScalar(ty),
                mir::Type::String => mir::Type::NullableString,
                mir::Type::Mixed => mir::Type::NullableMixed,
                mir::Type::Class(class) => mir::Type::NullableClass(class),
                mir::Type::Collection(_)
                | mir::Type::NullableScalar(_)
                | mir::Type::NullableString
                | mir::Type::NullableMixed
                | mir::Type::NullableClass(_) => return None,
            }
        }
        ResolvedType::TypeParameter(_)
        | ResolvedType::Void
        | ResolvedType::Null
        | ResolvedType::Unsupported => return None,
    };
    Some(ty)
}

fn intern_block_collection_types(
    block: &hir::Block,
    class_ids: &ClassIds,
    collections: &mut CollectionRegistry,
    substitutions: &HashMap<String, crate::types::ResolvedType>,
) {
    for statement in &block.statements {
        match statement {
            hir::Stmt::VarDecl(declaration) => {
                if let Some(ty) = &declaration.ty {
                    let _ =
                        mir_type_ref_with_substitutions(ty, class_ids, collections, substitutions);
                }
            }
            hir::Stmt::If(if_statement) => {
                intern_if_collection_types(if_statement, class_ids, collections, substitutions);
            }
            hir::Stmt::While(while_statement) => {
                intern_block_collection_types(
                    &while_statement.body,
                    class_ids,
                    collections,
                    substitutions,
                );
            }
            hir::Stmt::For(for_statement) => {
                if let Some(hir::ForInitializer::VarDecl(declaration)) = &for_statement.initializer
                {
                    if let Some(ty) = &declaration.ty {
                        let _ = mir_type_ref_with_substitutions(
                            ty,
                            class_ids,
                            collections,
                            substitutions,
                        );
                    }
                }
                intern_block_collection_types(
                    &for_statement.body,
                    class_ids,
                    collections,
                    substitutions,
                );
            }
            hir::Stmt::Foreach(foreach) => {
                if let Some(binding) = &foreach.key {
                    if let Some(ty) = &binding.ty {
                        let _ = mir_type_ref_with_substitutions(
                            ty,
                            class_ids,
                            collections,
                            substitutions,
                        );
                    }
                }
                if let Some(ty) = &foreach.value.ty {
                    let _ =
                        mir_type_ref_with_substitutions(ty, class_ids, collections, substitutions);
                }
                intern_block_collection_types(&foreach.body, class_ids, collections, substitutions);
            }
            hir::Stmt::Assignment(_)
            | hir::Stmt::Echo { .. }
            | hir::Stmt::Return { .. }
            | hir::Stmt::Break { .. }
            | hir::Stmt::Continue { .. }
            | hir::Stmt::Increment(_)
            | hir::Stmt::Expr { .. } => {}
        }
    }
}

fn intern_if_collection_types(
    statement: &hir::IfStmt,
    class_ids: &ClassIds,
    collections: &mut CollectionRegistry,
    substitutions: &HashMap<String, crate::types::ResolvedType>,
) {
    intern_block_collection_types(&statement.then_block, class_ids, collections, substitutions);
    if let Some(branch) = &statement.else_branch {
        match branch {
            hir::ElseBranch::If(statement) => {
                intern_if_collection_types(statement, class_ids, collections, substitutions);
            }
            hir::ElseBranch::Block(block) => {
                intern_block_collection_types(block, class_ids, collections, substitutions);
            }
        }
    }
}

fn lower_static_value(
    value: &crate::const_eval::ConstValue,
    ty: mir::Type,
    span: Span,
) -> DiagnosticResult<mir::StaticValue> {
    match (value, ty) {
        (
            crate::const_eval::ConstValue::Integer(value),
            mir::Type::Scalar(mir::ScalarType::Integer(expected))
            | mir::Type::NullableScalar(mir::ScalarType::Integer(expected)),
        ) if value.ty == expected => {
            Ok(mir::StaticValue::Scalar(mir::ScalarValue::Integer(*value)))
        }
        (
            crate::const_eval::ConstValue::Float(value),
            mir::Type::Scalar(mir::ScalarType::Float(expected))
            | mir::Type::NullableScalar(mir::ScalarType::Float(expected)),
        ) if value.ty == expected => Ok(mir::StaticValue::Scalar(mir::ScalarValue::Float(*value))),
        (
            crate::const_eval::ConstValue::Bool(value),
            mir::Type::Scalar(mir::ScalarType::Bool)
            | mir::Type::NullableScalar(mir::ScalarType::Bool),
        ) => Ok(mir::StaticValue::Scalar(mir::ScalarValue::Bool(*value))),
        (
            crate::const_eval::ConstValue::String(value),
            mir::Type::String | mir::Type::NullableString,
        ) => Ok(mir::StaticValue::String(value.clone())),
        (
            crate::const_eval::ConstValue::Null,
            mir::Type::NullableScalar(_) | mir::Type::NullableString | mir::Type::NullableClass(_),
        ) => Ok(mir::StaticValue::Null),
        _ => Err(vec![unsupported(
            span,
            "evaluated static initializer does not match its native type",
        )]),
    }
}

fn collect_function_signature(
    function: &hir::FunctionDecl,
    id: mir::FunctionId,
    class_ids: &ClassIds,
    semantic_info: &SemanticInfo,
    collection_registry: &mut CollectionRegistry,
    substitutions: &HashMap<String, crate::types::ResolvedType>,
    options: SignatureOptions,
) -> DiagnosticResult<FunctionSignature> {
    let return_type = match function.return_type.as_ref() {
        Some(ty) if scalar_type_ref(ty).is_some() => mir::ReturnType::Value(mir::Type::Scalar(
            scalar_type_ref(ty).expect("checked scalar type"),
        )),
        Some(ty) if is_plain_type(ty, "string") => mir::ReturnType::Value(mir::Type::String),
        Some(ty) if is_nullable_string_type(ty) => {
            mir::ReturnType::Value(mir::Type::NullableString)
        }
        Some(ty) if is_plain_type(ty, "void") => mir::ReturnType::Void,
        Some(ty)
            if mir_type_ref_with_substitutions(
                ty,
                class_ids,
                collection_registry,
                substitutions,
            )
            .is_some() =>
        {
            mir::ReturnType::Value(
                mir_type_ref_with_substitutions(ty, class_ids, collection_registry, substitutions)
                    .expect("checked native return"),
            )
        }
        Some(ty) => {
            return Err(vec![unsupported_native_type(
                ty,
                function.span,
                format!(
                    "function `{}` has return type `{ty}`, which is not supported by native compilation",
                    function.name
                ),
            )]);
        }
        None if options.lifecycle => mir::ReturnType::Void,
        None => {
            return Err(vec![unsupported(
                function.span,
                format!(
                    "function `{}` requires an explicit return type for native compilation",
                    function.name
                ),
            )]);
        }
    };

    // Decision 0099: the entry function takes either no parameters or exactly
    // one `List<string>` that the entry glue owns and lends to `main`. Semantic
    // checking rejects every other shape, so this only has to confirm the form
    // it is about to lower.
    if options.is_entry && !function.params.is_empty() {
        let entry_arguments_are_supported = function.params.len() == 1
            && mir_type_ref_with_substitutions(
                &function.params[0].ty,
                class_ids,
                collection_registry,
                substitutions,
            )
            .is_some_and(|ty| match ty {
                mir::Type::Collection(collection) => {
                    let definition = &collection_registry.types[collection.0];
                    definition.kind == mir::CollectionKind::List
                        && definition.value == mir::Type::String
                }
                _ => false,
            });
        if !entry_arguments_are_supported {
            return Err(vec![unsupported(
                function.params[0].span,
                "the native entry function `main` accepts only a `List<string>` argument list",
            )]);
        }
    }

    if options.is_entry
        && !matches!(
            return_type,
            mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                IntegerType::Int64,
            ))) | mir::ReturnType::Void
        )
    {
        return Err(vec![unsupported(
            function.span,
            "the native entry function `main` must return `int`, `int64`, or `void`",
        )]);
    }

    let mut parameter_types = Vec::with_capacity(function.params.len());
    let mut parameter_defaults = Vec::with_capacity(function.params.len());
    let mut parameter_transfers = Vec::with_capacity(function.params.len());
    let mut parameter_owns = Vec::with_capacity(function.params.len());
    for (parameter_index, param) in function.params.iter().enumerate() {
        let parameter_type = if let Some(ty) = mir_type_ref_with_substitutions(
            &param.ty,
            class_ids,
            collection_registry,
            substitutions,
        ) {
            ty
        } else {
            return Err(vec![unsupported_native_type(
                &param.ty,
                param.span,
                format!(
                    "function `{}` has parameter type `{}`, which is not supported by native compilation",
                    function.name, param.ty
                ),
            )]);
        };
        let transfers = matches!(
            parameter_type,
            mir::Type::Class(_)
                | mir::Type::NullableClass(_)
                | mir::Type::Collection(_)
                | mir::Type::Mixed
                | mir::Type::NullableMixed
        ) && param.take;
        let owns = transfers && param.promoted_access.is_none();
        let default = if param.default.is_some() {
            Some(
                semantic_info
                    .parameter_defaults
                    .get(&crate::const_eval::ParameterDefaultKey {
                        function_start: function.span.start,
                        parameter_index,
                    })
                    .cloned()
                    .ok_or_else(|| {
                        vec![Diagnostic::new(
                            "I2001",
                            format!(
                                "checked default for parameter `${}` of `{}` is missing",
                                param.name, function.name
                            ),
                            param.span,
                        )]
                    })?,
            )
        } else {
            None
        };
        parameter_types.push(parameter_type);
        parameter_defaults.push(default);
        parameter_transfers.push(transfers);
        parameter_owns.push(owns);
    }

    Ok(FunctionSignature {
        id,
        return_type,
        return_borrow: semantic_info
            .return_borrows
            .get(&function.span.start)
            .copied()
            .map(mir_return_borrow),
        parameter_types,
        parameter_names: function
            .params
            .iter()
            .map(|param| param.name.clone())
            .collect(),
        parameter_defaults,
        parameter_transfers,
        parameter_owns,
        method_class: None,
        receiver_mode: None,
    })
}

#[derive(Clone, Copy)]
struct SignatureOptions {
    lifecycle: bool,
    is_entry: bool,
}

fn mir_return_borrow(borrow: crate::symbols::ReturnBorrow) -> mir::ReturnBorrow {
    mir::ReturnBorrow {
        source: match borrow.source {
            crate::symbols::BorrowSource::Receiver => mir::BorrowSource::Receiver,
            crate::symbols::BorrowSource::Parameter(index) => mir::BorrowSource::Parameter(index),
        },
        writable: borrow.writable,
    }
}

fn mir_type_ref_with_substitutions(
    ty: &crate::types::TypeRef,
    class_ids: &ClassIds,
    collection_registry: &mut CollectionRegistry,
    substitutions: &HashMap<String, crate::types::ResolvedType>,
) -> Option<mir::Type> {
    let resolved = resolved_type_ref_with_substitutions(ty, substitutions)?;
    intern_resolved_collection_types(&resolved, class_ids, collection_registry)
}

fn resolved_type_ref_with_substitutions(
    ty: &crate::types::TypeRef,
    substitutions: &HashMap<String, ResolvedType>,
) -> Option<ResolvedType> {
    let mut plain = ty.clone();
    plain.nullable = false;
    let base = if plain.arguments.is_empty() {
        if let Some(substitution) = substitutions.get(&plain.name) {
            substitution.clone()
        } else if let Some(integer) = IntegerType::from_source_name(&plain.name) {
            ResolvedType::Integer(integer)
        } else if let Some(float) = FloatType::from_source_name(&plain.name) {
            ResolvedType::Float(float)
        } else {
            match plain.name.as_str() {
                "void" => ResolvedType::Void,
                "string" => ResolvedType::String,
                "Bytes" => ResolvedType::Bytes,
                "bool" => ResolvedType::Bool,
                "mixed" => ResolvedType::Mixed,
                _ => ResolvedType::Class(ClassType::new(plain.name, Vec::new())),
            }
        }
    } else {
        let arguments = plain
            .type_arguments()
            .map(|argument| resolved_type_ref_with_substitutions(argument, substitutions))
            .collect::<Option<Vec<_>>>()?;
        match plain.name.as_str() {
            "[]" if arguments.len() == 1 => {
                ResolvedType::TypedArray(Box::new(arguments[0].clone()))
            }
            "List" if arguments.len() == 1 => ResolvedType::List(Box::new(arguments[0].clone())),
            "Dictionary" if arguments.len() == 2 => ResolvedType::Dictionary(
                Box::new(arguments[0].clone()),
                Box::new(arguments[1].clone()),
            ),
            "Set" if arguments.len() == 1 => ResolvedType::Set(Box::new(arguments[0].clone())),
            _ => ResolvedType::Class(ClassType::new(plain.name, arguments)),
        }
    };
    if ty.nullable {
        Some(ResolvedType::Nullable(Box::new(base)))
    } else {
        Some(base)
    }
}

fn intern_bytes_type(collections: &mut CollectionRegistry) -> mir::CollectionTypeId {
    let byte = mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8));
    collections.intern(mir::CollectionKind::TypedArray, None, byte);
    collections.intern(mir::CollectionKind::Bytes, None, byte)
}

fn field_type(ty: mir::Type) -> Option<FieldType> {
    match ty {
        mir::Type::Scalar(mir::ScalarType::Integer(ty)) => Some(FieldType::Integer(ty)),
        mir::Type::Scalar(mir::ScalarType::Float(ty)) => Some(FieldType::Float(ty)),
        mir::Type::Scalar(mir::ScalarType::Bool) => Some(FieldType::Bool),
        mir::Type::String => Some(FieldType::String),
        mir::Type::Mixed => Some(FieldType::Mixed),
        mir::Type::NullableScalar(mir::ScalarType::Integer(ty)) => {
            Some(FieldType::NullableInteger(ty))
        }
        mir::Type::NullableScalar(mir::ScalarType::Float(ty)) => Some(FieldType::NullableFloat(ty)),
        mir::Type::NullableScalar(mir::ScalarType::Bool) => Some(FieldType::NullableBool),
        mir::Type::NullableString => Some(FieldType::NullableString),
        mir::Type::NullableMixed => Some(FieldType::NullableMixed),
        mir::Type::Class(class) => Some(FieldType::Class(class)),
        mir::Type::NullableClass(class) => Some(FieldType::NullableClass(class)),
        mir::Type::Collection(_) => Some(FieldType::Collection),
    }
}

fn integer_type_ref(ty: &crate::types::TypeRef) -> Option<IntegerType> {
    (!ty.nullable).then_some(()).and_then(|()| {
        ty.arguments
            .is_empty()
            .then(|| IntegerType::from_source_name(&ty.name))
            .flatten()
    })
}

fn float_type_ref(ty: &crate::types::TypeRef) -> Option<FloatType> {
    (!ty.nullable).then_some(()).and_then(|()| {
        ty.arguments
            .is_empty()
            .then(|| FloatType::from_source_name(&ty.name))
            .flatten()
    })
}

fn scalar_type_ref(ty: &crate::types::TypeRef) -> Option<mir::ScalarType> {
    integer_type_ref(ty)
        .map(mir::ScalarType::Integer)
        .or_else(|| float_type_ref(ty).map(mir::ScalarType::Float))
        .or_else(|| is_plain_type(ty, "bool").then_some(mir::ScalarType::Bool))
}

fn is_plain_type(ty: &crate::types::TypeRef, name: &str) -> bool {
    !ty.nullable && ty.name == name && ty.arguments.is_empty()
}

fn is_nullable_string_type(ty: &crate::types::TypeRef) -> bool {
    ty.nullable && ty.name == "string" && ty.arguments.is_empty()
}

struct FunctionLoweringInputs<'a> {
    signatures: &'a HashMap<FunctionInstanceKey, FunctionSignature>,
    method_signatures: &'a HashMap<MethodInstanceKey, FunctionSignature>,
    semantic_info: &'a SemanticInfo,
    property_initializers: &'a HashMap<crate::class_layout::PropertyId, PropertyInitializer>,
    constructor_body_initializers: &'a HashSet<crate::class_layout::PropertyId>,
    static_ids: &'a HashMap<(ClassId, String), (mir::StaticId, mir::Type)>,
    collection_registry: &'a CollectionRegistry,
    type_substitutions: &'a HashMap<String, crate::types::ResolvedType>,
}

fn lower_function(
    function: &hir::FunctionDecl,
    signature: FunctionSignature,
    inputs: FunctionLoweringInputs<'_>,
    class: Option<ClassId>,
    receiver: Option<ClassId>,
) -> DiagnosticResult<mir::Function> {
    let mut context = LoweringContext::new(&inputs);
    context.current_class = class;
    context.return_borrow = signature.return_borrow;
    let mut params = Vec::new();
    if let Some(class) = receiver {
        let writable = matches!(signature.receiver_mode, Some(mir::ReceiverMode::Writable));
        params.push(context.declare_user_local_owned(
            "this",
            writable,
            mir::Type::Class(class),
            false,
        ));
    }
    params.extend(
        function
            .params
            .iter()
            .zip(signature.parameter_types.iter().copied())
            .zip(signature.parameter_owns.iter().copied())
            .map(|((param, ty), owned)| {
                context.declare_user_local_owned(&param.name, param.writable, ty, owned)
            })
            .collect::<Vec<_>>(),
    );

    lower_function_body(
        &function.body,
        &function.name,
        signature.return_type,
        &mut context,
    )?;
    let (locals, blocks) = context.finish();

    Ok(mir::Function {
        id: signature.id,
        name: inputs_method_name(function, class, inputs.semantic_info),
        method: class.map(|class| mir::MethodIdentity {
            class,
            name: function.name.clone(),
        }),
        receiver_mode: receiver.map(|_| {
            if function.writable_this {
                mir::ReceiverMode::Writable
            } else {
                mir::ReceiverMode::Readonly
            }
        }),
        params,
        return_type: signature.return_type,
        locals,
        blocks,
        entry_block: mir::BlockId(0),
    })
}

fn inputs_method_name(
    function: &hir::FunctionDecl,
    class: Option<ClassId>,
    semantic_info: &crate::semantics::SemanticInfo,
) -> String {
    class.map_or_else(
        || function.name.clone(),
        |class| {
            let class_name = semantic_info
                .classes
                .iter()
                .find(|info| info.id == class)
                .map(|info| info.name.as_str())
                .expect("checked method class has semantic metadata");
            format!("{class_name}::{}", function.name)
        },
    )
}

fn lower_function_body(
    body: &hir::Block,
    function_name: &str,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    lower_statement_sequence(&body.statements, return_type, context)?;

    if context.current_block.is_some() {
        if return_type == mir::ReturnType::Void {
            context.cleanup_scopes_from(0);
            context.terminate_current(mir::Terminator::ReturnVoid);
        } else {
            return Err(vec![Diagnostic::new(
                "I1101",
                format!(
                    "internal compiler consistency error: checked int function `{function_name}` reaches MIR fallthrough"
                ),
                body.span,
            )]);
        }
    }

    Ok(())
}

fn lower_statement_sequence(
    statements: &[hir::Stmt],
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    for statement in statements {
        if context.current_block.is_none() {
            break;
        }

        match statement {
            hir::Stmt::Echo { expr, .. } => {
                let echo = lower_echo(expr, context)?;
                context.push_statement(echo);
            }
            hir::Stmt::Return { expr, span } => {
                let terminator = lower_return(expr.as_ref(), *span, return_type, context)?;
                context.terminate_current(terminator);
            }
            hir::Stmt::VarDecl(decl) => lower_var_decl(decl, context)?,
            hir::Stmt::Assignment(assignment) => lower_assignment(assignment, context)?,
            hir::Stmt::Increment(increment) => lower_increment(increment, context)?,
            hir::Stmt::If(if_stmt) => lower_if_statement(if_stmt, return_type, context)?,
            hir::Stmt::While(while_stmt) => {
                lower_while_statement(while_stmt, return_type, context)?;
            }
            hir::Stmt::For(for_stmt) => {
                lower_for_statement(for_stmt, return_type, context)?;
            }
            hir::Stmt::Foreach(foreach) => {
                lower_foreach_statement(foreach, return_type, context)?;
            }
            hir::Stmt::Break { span } => lower_loop_control(*span, LoopControl::Break, context)?,
            hir::Stmt::Continue { span } => {
                lower_loop_control(*span, LoopControl::Continue, context)?;
            }
            hir::Stmt::Expr { expr, span } => {
                if let hir::Expr::FunctionCall {
                    name,
                    args,
                    span: call_span,
                } = expr
                {
                    if lower_byte_file_write_statement(name, args, *call_span, context)? {
                        continue;
                    }
                }
                materialize_nested_collection_places(expr, false, context)?;
                if let hir::Expr::MethodCall {
                    object,
                    method,
                    args,
                    span: call_span,
                    null_safe,
                } = expr
                {
                    if !*null_safe
                        && lower_collection_method_statement(object, method, args, context)?
                    {
                        continue;
                    }
                    let statement = if *null_safe {
                        let (object, signature, args) =
                            lower_null_safe_method_call(object, method, args, *call_span, context)?;
                        discarded_null_safe_call_statement(object, signature, args, *span)?
                    } else {
                        let (signature, args) =
                            lower_instance_method_call(object, method, args, *call_span, context)?;
                        discarded_call_statement("method", signature, args, *span)?
                    };
                    context.push_statement(statement);
                    continue;
                }
                if let hir::Expr::StaticCall {
                    class_name,
                    method,
                    args,
                    span: call_span,
                } = expr
                {
                    let (signature, args) =
                        lower_static_method_call(class_name, method, args, *call_span, context)?;
                    let statement =
                        discarded_call_statement("static method", signature, args, *span)?;
                    context.push_statement(statement);
                    continue;
                }
                if let hir::Expr::FunctionCall {
                    name,
                    args,
                    span: call_span,
                } = expr
                {
                    if name == "panic" {
                        let message = lower_panic_message(args, *call_span, context)?;
                        context.terminate_current(mir::Terminator::Panic(message));
                    } else if name == "printf" {
                        let format = lower_format_expression(args, *call_span, context)?;
                        context.push_statement(mir::Statement::Printf(format));
                    } else if name == "write_file" {
                        let [path, contents] = argument_values(args)[..] else {
                            return Err(vec![unsupported(
                                *call_span,
                                "write_file expects 2 arguments",
                            )]);
                        };
                        let path = lower_string_expression(path, context)?;
                        let contents = lower_string_expression(contents, context)?;
                        context.push_statement(mir::Statement::WriteFile { path, contents });
                    } else if name == "append_file" {
                        let [path, contents] = argument_values(args)[..] else {
                            return Err(vec![unsupported(
                                *call_span,
                                "append_file expects 2 arguments",
                            )]);
                        };
                        let path = lower_string_expression(path, context)?;
                        let contents = lower_string_expression(contents, context)?;
                        context.push_statement(mir::Statement::AppendFile { path, contents });
                    } else if matches!(name.as_str(), "write_stdout_bytes" | "write_stderr_bytes") {
                        let [contents] = argument_values(args)[..] else {
                            return Err(vec![unsupported(
                                *call_span,
                                format!("{name} expects 1 argument"),
                            )]);
                        };
                        let contents = lower_bytes_local(contents, context)?.0;
                        context.push_statement(mir::Statement::WriteStreamBytes {
                            contents,
                            stderr: name == "write_stderr_bytes",
                        });
                    } else if name == "write_stderr" {
                        let [value] = argument_values(args)[..] else {
                            return Err(vec![unsupported(
                                *call_span,
                                "write_stderr expects 1 argument",
                            )]);
                        };
                        let value = lower_string_expression(value, context)?;
                        context.push_statement(mir::Statement::WriteStderr(value));
                    } else {
                        let call = lower_statement_call(name, args, *call_span, context)?;
                        context.push_statement(call);
                    }
                } else {
                    return Err(vec![unsupported(
                        *span,
                        "only calls to void free functions can be used as expression statements in native compilation",
                    )]);
                }
            }
        }
    }

    Ok(())
}

fn lower_if_statement(
    if_stmt: &hir::IfStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let condition_block = context.current_block();
    let fallthrough_blocks = lower_if_tree(if_stmt, condition_block, return_type, context)?;

    if fallthrough_blocks.is_empty() {
        context.current_block = None;
        return Ok(());
    }

    let continuation = context.create_block();
    for block in fallthrough_blocks {
        context.terminate_block(block, mir::Terminator::Jump(continuation));
    }
    context.current_block = context.is_reachable(continuation).then_some(continuation);
    Ok(())
}

fn lower_if_tree(
    if_stmt: &hir::IfStmt,
    condition_block: mir::BlockId,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<Vec<mir::BlockId>> {
    context.current_block = Some(condition_block);
    let then_block = context.create_block();
    let else_block = context.create_block();
    lower_condition_to_blocks(&if_stmt.condition, then_block, else_block, context)?;

    let mut fallthrough_blocks =
        lower_scoped_block(&if_stmt.then_block, then_block, return_type, context)?;

    match &if_stmt.else_branch {
        None => fallthrough_blocks.push(else_block),
        Some(hir::ElseBranch::Block(block)) => {
            fallthrough_blocks.extend(lower_scoped_block(block, else_block, return_type, context)?);
        }
        Some(hir::ElseBranch::If(nested)) => {
            fallthrough_blocks.extend(lower_if_tree(nested, else_block, return_type, context)?);
        }
    }

    Ok(fallthrough_blocks)
}

fn lower_while_statement(
    while_stmt: &hir::WhileStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let header_block = context.create_block();
    let body_block = context.create_block();
    let exit_block = context.create_block();

    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = Some(header_block);
    lower_condition_to_blocks(&while_stmt.condition, body_block, exit_block, context)?;

    context.push_loop_targets(LoopTargets {
        continue_block: header_block,
        break_block: exit_block,
        cleanup_depth: context.local_scopes.len(),
    });
    let body_result = lower_scoped_block(&while_stmt.body, body_block, return_type, context);
    context.pop_loop_targets();
    let fallthrough_blocks = body_result?;

    for block in fallthrough_blocks {
        context.terminate_block(block, mir::Terminator::Jump(header_block));
    }
    context.current_block = context.is_reachable(exit_block).then_some(exit_block);
    Ok(())
}

fn lower_for_statement(
    for_stmt: &hir::ForStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    context.push_scope();
    let result = lower_for_statement_in_scope(for_stmt, return_type, context);
    context.pop_scope();
    result
}

fn lower_for_statement_in_scope(
    for_stmt: &hir::ForStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    if let Some(initializer) = &for_stmt.initializer {
        match initializer {
            hir::ForInitializer::VarDecl(decl) => lower_var_decl(decl, context)?,
            hir::ForInitializer::Assignment(assignment) => {
                lower_assignment(assignment, context)?;
            }
        }
    }

    let header_block = context.create_block();
    let body_block = context.create_block();
    let increment_block = context.create_block();
    let exit_block = context.create_block();

    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = Some(header_block);
    if let Some(condition) = &for_stmt.condition {
        lower_condition_to_blocks(condition, body_block, exit_block, context)?;
    } else {
        context.terminate_condition(
            mir::BoolExpression::Use {
                operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
            },
            body_block,
            exit_block,
        );
    }

    context.push_loop_targets(LoopTargets {
        continue_block: increment_block,
        break_block: exit_block,
        cleanup_depth: context.local_scopes.len(),
    });
    let body_result = lower_scoped_block(&for_stmt.body, body_block, return_type, context);
    context.pop_loop_targets();
    let fallthrough_blocks = body_result?;

    for block in fallthrough_blocks {
        context.terminate_block(block, mir::Terminator::Jump(increment_block));
    }

    context.current_block = Some(increment_block);
    if let Some(increment) = &for_stmt.increment {
        match increment {
            hir::ForIncrement::Increment(increment) => lower_increment(increment, context)?,
            hir::ForIncrement::Assignment(assignment) => {
                lower_assignment(assignment, context)?;
            }
        }
    }
    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = context.is_reachable(exit_block).then_some(exit_block);
    Ok(())
}

fn lower_foreach_statement(
    foreach: &hir::ForeachStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    if let Some((start, end, inclusive)) = grouped_range_parts(&foreach.iterable) {
        if foreach.key.is_some() {
            return Err(vec![unsupported(
                foreach.span,
                "integer range `foreach` does not support key bindings in native compilation",
            )]);
        }
        if let Some(ty) = &foreach.value.ty {
            if integer_type_ref(ty).is_none() {
                return Err(vec![unsupported(
                    foreach.span,
                    format!("integer range foreach bindings require an integer type; got `{ty}`"),
                )]);
            }
        }
        context.push_scope();
        let result =
            lower_range_foreach_in_scope(foreach, start, end, inclusive, return_type, context);
        context.pop_scope();
        return result;
    }

    let (collection_expr, projection) = dictionary_foreach_projection(&foreach.iterable).map_or(
        (&foreach.iterable, CollectionForeachProjection::Main),
        |value| value,
    );
    context.push_scope();
    materialize_nested_collection_places(collection_expr, false, context)?;
    let (collection, collection_type) = match lower_collection_local(collection_expr, context) {
        Ok(place) => place,
        Err(_) => {
            let mir::Type::Collection(collection_type) =
                context.expression_type(collection_expr)?
            else {
                return Err(vec![unsupported(
                    collection_expr.span(),
                    "foreach iterable is not a collection",
                )]);
            };
            let borrowed = collection_place_is_borrowed(collection_expr);
            let value =
                lower_collection_expression(collection_expr, collection_type, !borrowed, context)?;
            let local = if borrowed {
                context.declare_borrowed_temp(mir::Type::Collection(collection_type), false)
            } else {
                context.declare_owned_temp(mir::Type::Collection(collection_type))
            };
            context.push_statement(mir::Statement::AssignLocal {
                target: local,
                value: mir::Rvalue::Collection(value),
            });
            (local, collection_type)
        }
    };
    let result = lower_collection_foreach_in_scope(
        foreach,
        collection,
        collection_type,
        projection,
        return_type,
        context,
    );
    context.pop_scope();
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CollectionForeachProjection {
    Main,
    Keys,
    Values,
}

fn dictionary_foreach_projection(
    expr: &hir::Expr,
) -> Option<(&hir::Expr, CollectionForeachProjection)> {
    match expr {
        hir::Expr::Grouped { expr, .. } => dictionary_foreach_projection(expr),
        hir::Expr::PropertyAccess {
            object,
            property,
            null_safe: false,
            ..
        } => match property.as_str() {
            "keys" => Some((object, CollectionForeachProjection::Keys)),
            "values" => Some((object, CollectionForeachProjection::Values)),
            _ => None,
        },
        _ => None,
    }
}

fn lower_collection_foreach_in_scope(
    foreach: &hir::ForeachStmt,
    collection: mir::LocalId,
    collection_type: mir::CollectionTypeId,
    projection: CollectionForeachProjection,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let definition = context.collection_type(collection_type).clone();
    if projection != CollectionForeachProjection::Main
        && definition.kind != mir::CollectionKind::Dictionary
    {
        return Err(vec![unsupported(
            foreach.span,
            "dictionary projections require a Dictionary",
        )]);
    }
    if projection != CollectionForeachProjection::Main && foreach.key.is_some() {
        return Err(vec![unsupported(
            foreach.span,
            "dictionary projections do not support foreach key bindings",
        )]);
    }
    if projection != CollectionForeachProjection::Main && foreach.value.writable {
        return Err(vec![unsupported(
            foreach.span,
            "dictionary projections are readonly",
        )]);
    }
    let index_type = IntegerType::Int64;
    let index_local = context.declare_temp(true, index_type);
    context.push_statement(mir::Statement::AssignLocal {
        target: index_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(
            mir::IntegerExpression::constant(
                IntegerValue::from_i128(index_type, 0).expect("zero is a valid int"),
            ),
        )),
    });

    let key_local = match (&foreach.key, definition.key, projection) {
        (Some(binding), Some(key_type), CollectionForeachProjection::Main) => {
            Some(context.declare_user_local_owned(&binding.name, false, key_type, false))
        }
        (Some(_), None, CollectionForeachProjection::Main) => {
            return Err(vec![unsupported(
                foreach.span,
                "foreach key bindings require a Dictionary",
            )])
        }
        (None, _, _) => None,
        (Some(_), _, _) => unreachable!("projection key bindings were rejected above"),
    };
    let binding_type = match projection {
        CollectionForeachProjection::Keys => definition
            .key
            .expect("a Dictionary collection type must have a key type"),
        CollectionForeachProjection::Main | CollectionForeachProjection::Values => definition.value,
    };
    let value_local = context.declare_user_local_owned(
        &foreach.value.name,
        foreach.value.writable,
        binding_type,
        false,
    );

    let header_block = context.create_block();
    let body_block = context.create_block();
    let update_block = context.create_block();
    let exit_block = context.create_block();
    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = Some(header_block);
    context.terminate_current(mir::Terminator::Branch {
        condition: mir::BoolExpression::Compare {
            op: mir::CompareOp::Less,
            left: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                index_local,
                index_type,
            ))),
            right: Box::new(mir::ValueExpression::Integer(mir::IntegerExpression::Use {
                ty: index_type,
                operand: mir::Operand::CollectionLength(collection),
            })),
        },
        then_block: body_block,
        else_block: exit_block,
    });

    let offset = collection_offset_rvalue(index_local);
    let key = definition
        .key
        .map(|key_type| collection_key_at_rvalue(collection, offset.clone(), key_type))
        .transpose()?;
    context.current_block = Some(body_block);
    if let (Some(target), Some(key)) = (key_local, key.clone()) {
        context.push_statement(mir::Statement::AssignLocal { target, value: key });
    }
    let binding_value = match projection {
        CollectionForeachProjection::Keys => key
            .clone()
            .expect("a Dictionary collection type must produce a key"),
        CollectionForeachProjection::Main | CollectionForeachProjection::Values => {
            collection_value_rvalue(
                collection,
                key.clone().unwrap_or_else(|| offset.clone()),
                definition.value,
            )?
        }
    };
    context.push_statement(mir::Statement::AssignLocal {
        target: value_local,
        value: binding_value,
    });

    context.push_loop_targets(LoopTargets {
        continue_block: update_block,
        break_block: exit_block,
        cleanup_depth: context.local_scopes.len(),
    });
    context.push_scope();
    let body_result = lower_statement_sequence(&foreach.body.statements, return_type, context);
    let body_fallthrough = context.current_block;
    context.pop_scope();
    context.pop_loop_targets();
    body_result?;
    if let Some(block) = body_fallthrough {
        context.terminate_block(block, mir::Terminator::Jump(update_block));
    }

    context.current_block = Some(update_block);
    if foreach.value.writable && projection == CollectionForeachProjection::Main {
        match definition.value {
            mir::Type::Scalar(_) | mir::Type::String => {
                context.push_statement(mir::Statement::AssignCollectionIndex {
                    collection,
                    index: key.unwrap_or_else(|| offset.clone()),
                    value: foreach_local_rvalue(value_local, definition.value)?,
                });
            }
            mir::Type::Class(_) | mir::Type::Mixed | mir::Type::Collection(_) => {}
            mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::NullableMixed
            | mir::Type::NullableClass(_) => {
                return Err(vec![unsupported(
                    foreach.span,
                    "nullable collection elements are deferred beyond Stage 23 Slice 1",
                )])
            }
        }
    }
    context.push_statement(mir::Statement::AssignLocal {
        target: index_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(
            mir::IntegerExpression::Binary {
                ty: index_type,
                op: mir::IntegerBinaryOp::Add,
                left: Box::new(local_integer_expression(index_local, index_type)),
                right: Box::new(mir::IntegerExpression::constant(IntegerValue::one(
                    index_type,
                ))),
            },
        )),
    });
    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = context.is_reachable(exit_block).then_some(exit_block);
    Ok(())
}

fn collection_offset_rvalue(local: mir::LocalId) -> mir::Rvalue {
    mir::Rvalue::Value(mir::ValueExpression::Integer(local_integer_expression(
        local,
        IntegerType::Int64,
    )))
}

fn collection_key_at_rvalue(
    collection: mir::LocalId,
    offset: mir::Rvalue,
    ty: mir::Type,
) -> DiagnosticResult<mir::Rvalue> {
    match ty {
        mir::Type::Scalar(scalar) => Ok(mir::Rvalue::Value(value_expression_from_operand(
            scalar,
            mir::Operand::CollectionKeyAt {
                collection,
                offset: Box::new(offset),
            },
        ))),
        mir::Type::String => Ok(mir::Rvalue::String(
            mir::StringExpression::CollectionKeyAt {
                collection,
                offset: Box::new(offset),
            },
        )),
        _ => Err(vec![unsupported(
            Span::new(0, 0),
            "Stage 23 Slice 1 dictionary keys must be scalar or string values",
        )]),
    }
}

fn collection_value_rvalue(
    collection: mir::LocalId,
    index: mir::Rvalue,
    ty: mir::Type,
) -> DiagnosticResult<mir::Rvalue> {
    match ty {
        mir::Type::Scalar(scalar) => Ok(mir::Rvalue::Value(value_expression_from_operand(
            scalar,
            mir::Operand::CollectionIndex {
                collection,
                index: Box::new(index),
                remove: false,
            },
        ))),
        mir::Type::String => Ok(mir::Rvalue::String(
            mir::StringExpression::CollectionIndex {
                collection,
                index: Box::new(index),
                remove: false,
            },
        )),
        mir::Type::Class(class) => Ok(mir::Rvalue::Class(mir::ClassExpression::CollectionIndex {
            class,
            collection,
            index: Box::new(index),
            transfer: false,
        })),
        mir::Type::Mixed => Ok(mir::Rvalue::Mixed(mir::MixedExpression::CollectionIndex {
            collection,
            index: Box::new(index),
            transfer: false,
            remove: false,
        })),
        mir::Type::Collection(nested) => {
            Ok(mir::Rvalue::Collection(mir::CollectionExpression::Index {
                collection: nested,
                source: collection,
                index: Box::new(index),
                transfer: false,
            }))
        }
        mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::NullableMixed
        | mir::Type::NullableClass(_) => Err(vec![unsupported(
            Span::new(0, 0),
            "nullable collection elements are deferred beyond Stage 23 Slice 3",
        )]),
    }
}

fn foreach_local_rvalue(local: mir::LocalId, ty: mir::Type) -> DiagnosticResult<mir::Rvalue> {
    match ty {
        mir::Type::Scalar(scalar) => Ok(mir::Rvalue::Value(value_expression_from_operand(
            scalar,
            mir::Operand::Local(local),
        ))),
        mir::Type::String => Ok(mir::Rvalue::String(mir::StringExpression::Local(local))),
        mir::Type::Mixed => Ok(mir::Rvalue::Mixed(mir::MixedExpression::Local {
            local,
            transfer: false,
        })),
        _ => Err(vec![unsupported(
            Span::new(0, 0),
            "this foreach binding cannot be written back in Stage 23 Slice 3",
        )]),
    }
}

fn lower_range_foreach_in_scope(
    foreach: &hir::ForeachStmt,
    start: &hir::Expr,
    end: &hir::Expr,
    inclusive: bool,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let integer_type = context.integer_type(start)?;
    let end_type = context.integer_type(end)?;
    if end_type != integer_type {
        return Err(vec![Diagnostic::new(
            "I1301",
            "internal compiler consistency error: checked range endpoints have different integer types",
            foreach.span,
        )]);
    }

    let start_value = lower_integer_expression(start, context)?;
    ensure_expression_type(&start_value, integer_type, start.span())?;
    let current_local = context.declare_temp(true, integer_type);
    context.push_statement(mir::Statement::AssignLocal {
        target: current_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(start_value)),
    });

    let end_value = lower_integer_expression(end, context)?;
    ensure_expression_type(&end_value, integer_type, end.span())?;
    let end_local = context.declare_temp(false, integer_type);
    context.push_statement(mir::Statement::AssignLocal {
        target: end_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(end_value)),
    });

    let header_block = context.create_block();
    let body_block = context.create_block();
    let update_block = context.create_block();
    let increment_block = inclusive.then(|| context.create_block());
    let exit_block = context.create_block();

    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = Some(header_block);
    context.terminate_current(mir::Terminator::Branch {
        condition: mir::BoolExpression::Compare {
            op: if inclusive {
                mir::CompareOp::LessEqual
            } else {
                mir::CompareOp::Less
            },
            left: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                current_local,
                integer_type,
            ))),
            right: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                end_local,
                integer_type,
            ))),
        },
        then_block: body_block,
        else_block: exit_block,
    });

    let binding_local = context.declare_user_local(
        &foreach.value.name,
        false,
        mir::Type::Scalar(mir::ScalarType::Integer(integer_type)),
    );
    context.push_loop_targets(LoopTargets {
        continue_block: update_block,
        break_block: exit_block,
        cleanup_depth: context.local_scopes.len(),
    });
    context.push_scope();
    context.current_block = Some(body_block);
    context.push_statement(mir::Statement::AssignLocal {
        target: binding_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(local_integer_expression(
            current_local,
            integer_type,
        ))),
    });
    let body_result = lower_statement_sequence(&foreach.body.statements, return_type, context);
    let body_fallthrough = context.current_block;
    context.pop_scope();
    context.pop_loop_targets();
    body_result?;

    if let Some(block) = body_fallthrough {
        context.terminate_block(block, mir::Terminator::Jump(update_block));
    }

    context.current_block = Some(update_block);
    if let Some(increment_block) = increment_block {
        context.terminate_current(mir::Terminator::Branch {
            condition: mir::BoolExpression::Compare {
                op: mir::CompareOp::Equal,
                left: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                    current_local,
                    integer_type,
                ))),
                right: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                    end_local,
                    integer_type,
                ))),
            },
            then_block: exit_block,
            else_block: increment_block,
        });
        context.current_block = Some(increment_block);
    }
    context.push_statement(mir::Statement::AssignLocal {
        target: current_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(
            mir::IntegerExpression::Binary {
                ty: integer_type,
                op: mir::IntegerBinaryOp::Add,
                left: Box::new(local_integer_expression(current_local, integer_type)),
                right: Box::new(mir::IntegerExpression::constant(IntegerValue::one(
                    integer_type,
                ))),
            },
        )),
    });
    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = context.is_reachable(exit_block).then_some(exit_block);
    Ok(())
}

fn grouped_range_parts(expr: &hir::Expr) -> Option<(&hir::Expr, &hir::Expr, bool)> {
    match expr {
        hir::Expr::Grouped { expr, .. } => grouped_range_parts(expr),
        hir::Expr::Range {
            start,
            end,
            inclusive,
            ..
        } => Some((start, end, *inclusive)),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum LoopControl {
    Break,
    Continue,
}

fn lower_loop_control(
    span: Span,
    control: LoopControl,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let targets = context.current_loop_targets().ok_or_else(|| {
        let keyword = match control {
            LoopControl::Break => "break",
            LoopControl::Continue => "continue",
        };
        vec![unsupported(
            span,
            format!("`{keyword}` requires an enclosing loop"),
        )]
    })?;
    let target = match control {
        LoopControl::Break => targets.break_block,
        LoopControl::Continue => targets.continue_block,
    };
    context.cleanup_scopes_from(targets.cleanup_depth);
    context.terminate_current(mir::Terminator::Jump(target));
    Ok(())
}

fn lower_scoped_block(
    block: &hir::Block,
    entry_block: mir::BlockId,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<Vec<mir::BlockId>> {
    context.push_scope();
    context.current_block = Some(entry_block);
    let result = lower_statement_sequence(&block.statements, return_type, context);
    let current_block = context.current_block;
    context.pop_scope();
    result?;
    Ok(current_block.into_iter().collect())
}

struct BlockBuilder {
    id: mir::BlockId,
    statements: Vec<mir::Statement>,
    terminator: Option<mir::Terminator>,
}

#[derive(Clone, Copy)]
struct LoopTargets {
    continue_block: mir::BlockId,
    break_block: mir::BlockId,
    cleanup_depth: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CoalesceSelection {
    Left,
    Right,
    Dynamic,
}

struct LoweringContext<'semantic> {
    signatures: HashMap<FunctionInstanceKey, FunctionSignature>,
    method_signatures: HashMap<MethodInstanceKey, FunctionSignature>,
    semantic_info: &'semantic SemanticInfo,
    property_initializers: HashMap<crate::class_layout::PropertyId, PropertyInitializer>,
    constructor_body_initializers: HashSet<crate::class_layout::PropertyId>,
    static_ids: HashMap<(ClassId, String), (mir::StaticId, mir::Type)>,
    collection_registry: CollectionRegistry,
    type_substitutions: HashMap<String, crate::types::ResolvedType>,
    current_class: Option<ClassId>,
    locals: Vec<mir::Local>,
    local_scopes: Vec<HashMap<String, mir::LocalId>>,
    materialized_collection_places: HashMap<(usize, usize), mir::LocalId>,
    scope_owned_locals: Vec<Vec<DropObligation>>,
    temp_counter: usize,
    blocks: Vec<BlockBuilder>,
    reachable_blocks: Vec<bool>,
    current_block: Option<mir::BlockId>,
    loop_targets: Vec<LoopTargets>,
    return_borrow: Option<mir::ReturnBorrow>,
}

#[derive(Clone, Copy)]
enum DropObligation {
    Class(mir::LocalId, ClassId),
    Mixed(mir::LocalId),
    Collection(mir::LocalId, mir::CollectionTypeId),
}

impl<'semantic> LoweringContext<'semantic> {
    fn new(inputs: &FunctionLoweringInputs<'semantic>) -> Self {
        Self {
            signatures: inputs.signatures.clone(),
            method_signatures: inputs.method_signatures.clone(),
            semantic_info: inputs.semantic_info,
            property_initializers: inputs.property_initializers.clone(),
            constructor_body_initializers: inputs.constructor_body_initializers.clone(),
            static_ids: inputs.static_ids.clone(),
            collection_registry: inputs.collection_registry.clone(),
            type_substitutions: inputs.type_substitutions.clone(),
            current_class: None,
            locals: Vec::new(),
            local_scopes: vec![HashMap::new()],
            materialized_collection_places: HashMap::new(),
            scope_owned_locals: vec![Vec::new()],
            temp_counter: 0,
            blocks: vec![BlockBuilder {
                id: mir::BlockId(0),
                statements: Vec::new(),
                terminator: None,
            }],
            reachable_blocks: vec![true],
            current_block: Some(mir::BlockId(0)),
            loop_targets: Vec::new(),
            return_borrow: None,
        }
    }

    fn finish(self) -> (Vec<mir::Local>, Vec<mir::BasicBlock>) {
        let blocks = self
            .blocks
            .into_iter()
            .map(|block| mir::BasicBlock {
                id: block.id,
                statements: block.statements,
                terminator: block.terminator.unwrap_or(mir::Terminator::Unreachable),
            })
            .collect();
        (self.locals, blocks)
    }

    fn create_block(&mut self) -> mir::BlockId {
        let id = mir::BlockId(self.blocks.len());
        self.blocks.push(BlockBuilder {
            id,
            statements: Vec::new(),
            terminator: None,
        });
        self.reachable_blocks.push(false);
        id
    }

    fn current_block(&self) -> mir::BlockId {
        self.current_block
            .expect("MIR lowering requires a current block")
    }

    fn push_statement(&mut self, statement: mir::Statement) {
        let block = self.current_block();
        self.blocks[block.0].statements.push(statement);
    }

    fn terminate_current(&mut self, terminator: mir::Terminator) {
        let block = self.current_block();
        self.terminate_block(block, terminator);
        self.current_block = None;
    }

    fn terminate_block(&mut self, block: mir::BlockId, terminator: mir::Terminator) {
        if self.is_reachable(block) {
            for target in terminator_targets(&terminator) {
                self.reachable_blocks[target.0] = true;
            }
        }
        let slot = &mut self.blocks[block.0].terminator;
        assert!(slot.is_none(), "MIR block terminated more than once");
        *slot = Some(terminator);
    }

    fn terminate_condition(
        &mut self,
        condition: mir::BoolExpression,
        then_block: mir::BlockId,
        else_block: mir::BlockId,
    ) {
        match condition {
            mir::BoolExpression::Use {
                operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
            } => {
                self.terminate_current(mir::Terminator::Jump(then_block));
            }
            mir::BoolExpression::Use {
                operand: mir::Operand::Scalar(mir::ScalarValue::Bool(false)),
            } => {
                self.terminate_current(mir::Terminator::Jump(else_block));
            }
            condition => self.terminate_current(mir::Terminator::Branch {
                condition,
                then_block,
                else_block,
            }),
        }
    }

    fn is_reachable(&self, block: mir::BlockId) -> bool {
        self.reachable_blocks.get(block.0).copied().unwrap_or(false)
    }

    fn push_scope(&mut self) {
        self.local_scopes.push(HashMap::new());
        self.scope_owned_locals.push(Vec::new());
    }

    fn pop_scope(&mut self) {
        assert!(
            self.local_scopes.len() > 1,
            "MIR lowering cannot pop the root local scope"
        );
        if self.current_block.is_some() {
            self.cleanup_scopes_from(self.local_scopes.len() - 1);
        }
        self.local_scopes.pop();
        self.scope_owned_locals.pop();
    }

    fn cleanup_scopes_from(&mut self, depth: usize) {
        let cleanup = self.scope_owned_locals[depth..]
            .iter()
            .rev()
            .flat_map(|scope| scope.iter().rev().copied())
            .collect::<Vec<_>>();
        for obligation in cleanup {
            self.push_statement(match obligation {
                DropObligation::Class(local, class) => mir::Statement::DropClass { local, class },
                DropObligation::Mixed(local) => mir::Statement::DropMixed { local },
                DropObligation::Collection(local, collection) => {
                    mir::Statement::DropCollection { local, collection }
                }
            });
        }
    }

    fn has_cleanup_obligations(&self) -> bool {
        self.scope_owned_locals
            .iter()
            .any(|scope| !scope.is_empty())
    }

    fn push_loop_targets(&mut self, targets: LoopTargets) {
        self.loop_targets.push(targets);
    }

    fn pop_loop_targets(&mut self) {
        self.loop_targets
            .pop()
            .expect("MIR lowering cannot pop an empty loop-target stack");
    }

    fn current_loop_targets(&self) -> Option<LoopTargets> {
        self.loop_targets.last().copied()
    }

    fn declare_user_local(&mut self, name: &str, writable: bool, ty: mir::Type) -> mir::LocalId {
        let owned = matches!(
            ty,
            mir::Type::Class(_)
                | mir::Type::NullableClass(_)
                | mir::Type::Mixed
                | mir::Type::NullableMixed
                | mir::Type::Collection(_)
        );
        self.declare_user_local_owned(name, writable, ty, owned)
    }

    fn declare_user_local_owned(
        &mut self,
        name: &str,
        writable: bool,
        ty: mir::Type,
        owned: bool,
    ) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        self.locals.push(mir::Local {
            id,
            name: name.to_string(),
            ty,
            writable,
            owned,
            synthetic: false,
        });
        self.local_scopes
            .last_mut()
            .expect("MIR lowering must have a local scope")
            .insert(name.to_string(), id);
        if owned {
            let obligation = match ty {
                mir::Type::Class(class) | mir::Type::NullableClass(class) => {
                    DropObligation::Class(id, class)
                }
                mir::Type::Mixed | mir::Type::NullableMixed => DropObligation::Mixed(id),
                mir::Type::Collection(collection) => DropObligation::Collection(id, collection),
                _ => unreachable!("only move locals may own native drop obligations"),
            };
            self.scope_owned_locals
                .last_mut()
                .expect("MIR lowering must have an ownership scope")
                .push(obligation);
        }
        id
    }

    fn declare_temp(&mut self, writable: bool, ty: IntegerType) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        let name = format!("_tmp{}", self.temp_counter);
        self.temp_counter += 1;
        self.locals.push(mir::Local {
            id,
            name,
            ty: mir::Type::Scalar(mir::ScalarType::Integer(ty)),
            writable,
            owned: false,
            synthetic: true,
        });
        id
    }

    fn declare_return_temp(&mut self, ty: mir::Type, owned: bool) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        let name = format!("_return{}", self.temp_counter);
        self.temp_counter += 1;
        self.locals.push(mir::Local {
            id,
            name,
            ty,
            writable: false,
            owned,
            synthetic: true,
        });
        id
    }

    fn declare_borrowed_temp(&mut self, ty: mir::Type, writable: bool) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        let name = format!("_borrow{}", self.temp_counter);
        self.temp_counter += 1;
        self.locals.push(mir::Local {
            id,
            name,
            ty,
            writable,
            owned: false,
            synthetic: true,
        });
        id
    }

    fn declare_string_temp(&mut self) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        let name = format!("_string{}", self.temp_counter);
        self.temp_counter += 1;
        self.locals.push(mir::Local {
            id,
            name,
            ty: mir::Type::String,
            writable: false,
            owned: false,
            synthetic: true,
        });
        id
    }

    fn declare_owned_temp(&mut self, ty: mir::Type) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        let name = format!("_owned{}", self.temp_counter);
        self.temp_counter += 1;
        self.locals.push(mir::Local {
            id,
            name,
            ty,
            writable: false,
            owned: true,
            synthetic: true,
        });
        let obligation = match ty {
            mir::Type::Class(class) | mir::Type::NullableClass(class) => {
                DropObligation::Class(id, class)
            }
            mir::Type::Mixed | mir::Type::NullableMixed => DropObligation::Mixed(id),
            mir::Type::Collection(collection) => DropObligation::Collection(id, collection),
            _ => unreachable!("only move locals may own native drop obligations"),
        };
        self.scope_owned_locals
            .last_mut()
            .expect("MIR lowering must have an ownership scope")
            .push(obligation);
        id
    }

    fn lookup_local(&self, name: &str, span: Span) -> DiagnosticResult<mir::LocalId> {
        self.local_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .ok_or_else(|| {
                vec![unsupported(
                    span,
                    format!("local `${name}` is not available in this native expression"),
                )]
            })
    }

    fn local_type(&self, id: mir::LocalId) -> mir::Type {
        self.locals
            .get(id.0)
            .filter(|local| local.id == id)
            .expect("lowered MIR local must have a matching slot")
            .ty
    }

    fn local_owns(&self, id: mir::LocalId) -> bool {
        self.locals
            .get(id.0)
            .filter(|local| local.id == id)
            .expect("lowered MIR local must have a matching slot")
            .owned
    }

    fn class_id_for_name(&self, name: &str) -> Option<ClassId> {
        self.semantic_info
            .classes
            .iter()
            .find(|class| {
                class.name == name || (class.declaration_name == name && class.arguments.is_empty())
            })
            .map(|class| class.id)
    }

    fn class_id_for_static_access(&self, name: &str) -> Option<ClassId> {
        self.class_id_for_name(name).or_else(|| {
            let class = self.current_class?;
            self.class_info(class)
                .is_some_and(|info| info.declaration_name == name)
                .then_some(class)
        })
    }

    fn class_id_for_type(&self, class_type: &ClassType<ResolvedType>) -> Option<ClassId> {
        self.semantic_info
            .classes
            .iter()
            .find(|class| {
                class.declaration_name == class_type.name && class.arguments == class_type.arguments
            })
            .map(|class| class.id)
    }

    fn call_target_class_id(&self, span: Span) -> Option<ClassId> {
        let CallableTarget::Method { class_type, .. } = self
            .semantic_info
            .call_targets
            .get(&(span.start, span.end))?
        else {
            return None;
        };
        let specialized = substitute_resolved_type(
            &ResolvedType::Class(class_type.clone()),
            &self.type_substitutions,
        );
        let ResolvedType::Class(class_type) = specialized else {
            return None;
        };
        self.class_id_for_type(&class_type)
    }

    fn class_info(&self, id: ClassId) -> Option<&crate::semantics::ClassSemanticInfo> {
        self.semantic_info
            .classes
            .iter()
            .find(|class| class.id == id)
    }

    fn collection_type(&self, id: mir::CollectionTypeId) -> &mir::CollectionType {
        self.collection_registry
            .types
            .get(id.0)
            .filter(|collection| collection.id == id)
            .expect("lowered collection type must have a matching registry slot")
    }

    fn property_info(
        &self,
        class: ClassId,
        name: &str,
    ) -> Option<&crate::semantics::PropertySemanticInfo> {
        self.class_info(class)?
            .properties
            .iter()
            .find(|property| property.name == name)
    }

    fn lookup_lifecycle(&self, class: ClassId, name: &str) -> Option<FunctionSignature> {
        self.method_signatures
            .get(&MethodInstanceKey {
                class,
                name: name.to_string(),
                arguments: Vec::new(),
            })
            .cloned()
    }

    fn lookup_method(
        &self,
        class: ClassId,
        name: &str,
        span: Span,
    ) -> DiagnosticResult<FunctionSignature> {
        let arguments = self.specialization_arguments(span);
        self.method_signatures
            .get(&MethodInstanceKey {
                class,
                name: name.to_string(),
                arguments,
            })
            .cloned()
            .ok_or_else(|| {
                vec![unsupported(
                    span,
                    format!("call references unknown method `class#{}::{name}`", class.0),
                )]
            })
    }

    fn constant_decl(&self, expr: &hir::Expr) -> Option<&crate::const_eval::EvaluatedDecl> {
        let key = match expr {
            hir::Expr::Identifier { name, .. } => {
                crate::const_eval::ConstKey::TopLevel(name.clone())
            }
            hir::Expr::StaticMember {
                class_name, member, ..
            } => crate::const_eval::ConstKey::Class {
                class_name: class_name.clone(),
                name: member.clone(),
            },
            hir::Expr::Grouped { expr, .. } => return self.constant_decl(expr),
            _ => return None,
        };
        self.semantic_info.const_evaluation.values.get(&key)
    }

    fn constant_value(&self, expr: &hir::Expr) -> Option<&crate::const_eval::ConstValue> {
        self.constant_decl(expr).map(|decl| &decl.value)
    }

    fn static_property(
        &self,
        class_name: &str,
        member: &str,
        span: Span,
    ) -> DiagnosticResult<(mir::StaticId, mir::Type)> {
        let class = self
            .class_id_for_static_access(class_name)
            .ok_or_else(|| vec![unsupported(span, format!("unknown class `{class_name}`"))])?;
        let (id, ty) = self
            .static_ids
            .get(&(class, member.to_string()))
            .copied()
            .ok_or_else(|| {
                vec![unsupported(
                    span,
                    format!("unknown static property `{class_name}::{member}`"),
                )]
            })?;
        Ok((id, ty))
    }

    fn native_type_ref(&self, ty: &crate::types::TypeRef) -> Option<mir::Type> {
        let resolved = resolved_type_ref_with_substitutions(ty, &self.type_substitutions)?;
        self.mir_resolved_type(&resolved)
    }

    fn lookup_function(&self, name: &str, span: Span) -> DiagnosticResult<FunctionSignature> {
        let key = FunctionInstanceKey {
            name: name.to_string(),
            arguments: self.specialization_arguments(span),
        };
        self.signatures.get(&key).cloned().ok_or_else(|| {
            vec![unsupported(
                span,
                format!("call references unknown top-level function `{name}`"),
            )]
        })
    }

    fn specialization_arguments(&self, span: Span) -> Vec<GenericArgument> {
        self.semantic_info
            .generic_call_specializations
            .get(&(span.start, span.end))
            .map(|specialization| {
                specialization
                    .arguments
                    .iter()
                    .map(|argument| substitute_generic_argument(argument, &self.type_substitutions))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn integer_type(&self, expr: &hir::Expr) -> DiagnosticResult<IntegerType> {
        self.semantic_info
            .integer_type(expr.span())
            .or_else(|| match self.expression_type(expr).ok() {
                Some(mir::Type::Scalar(mir::ScalarType::Integer(ty)))
                | Some(mir::Type::NullableScalar(mir::ScalarType::Integer(ty))) => Some(ty),
                _ => None,
            })
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "I1301",
                    "internal compiler consistency error: checked integer expression has no canonical integer type",
                    expr.span(),
                )]
            })
    }

    fn float_type(&self, expr: &hir::Expr) -> DiagnosticResult<FloatType> {
        self.semantic_info
            .float_type(expr.span())
            .or_else(|| match self.expression_type(expr).ok() {
                Some(mir::Type::Scalar(mir::ScalarType::Float(ty)))
                | Some(mir::Type::NullableScalar(mir::ScalarType::Float(ty))) => Some(ty),
                _ => None,
            })
            .ok_or_else(|| {
            vec![Diagnostic::new(
                "I1401",
                "internal compiler consistency error: checked float expression has no canonical float type",
                expr.span(),
            )]
            })
    }

    fn expression_type(&self, expr: &hir::Expr) -> DiagnosticResult<mir::Type> {
        let resolved = self
            .semantic_info
            .expression_type(expr.span())
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "I2201",
                    "checked expression is missing its resolved semantic type",
                    expr.span(),
                )]
            })?;
        let resolved = substitute_resolved_type(resolved, &self.type_substitutions);
        self.mir_resolved_type(&resolved).ok_or_else(|| {
            vec![unsupported(
                expr.span(),
                format!("resolved type `{resolved:?}` has no native representation"),
            )]
        })
    }

    fn expression_is_null(&self, expr: &hir::Expr) -> bool {
        matches!(
            self.semantic_info.expression_type(expr.span()),
            Some(crate::types::ResolvedType::Null)
        )
    }

    fn flow_fact(&self, expr: &hir::Expr) -> Option<&crate::narrowing::Fact> {
        let expr = match expr {
            hir::Expr::Grouped { expr, .. } => return self.flow_fact(expr),
            expr => expr,
        };
        self.semantic_info
            .flow_facts
            .get(&(expr.span().start, expr.span().end))
    }

    fn exact_mixed_local(&self, expr: &hir::Expr) -> Option<(mir::LocalId, mir::Type)> {
        let hir::Expr::Variable { name, span } = unparenthesized_place(expr) else {
            return None;
        };
        let local = self.lookup_local(name, *span).ok()?;
        if !matches!(
            self.local_type(local),
            mir::Type::Mixed | mir::Type::NullableMixed
        ) {
            return None;
        }
        let crate::narrowing::Fact::Exact(type_ref) = self.flow_fact(expr)? else {
            return None;
        };
        self.native_type_ref(type_ref).map(|ty| (local, ty))
    }

    fn coalesce_selection(&self, left: &hir::Expr) -> CoalesceSelection {
        match self.flow_fact(left) {
            Some(crate::narrowing::Fact::Null) => CoalesceSelection::Right,
            Some(crate::narrowing::Fact::NonNull | crate::narrowing::Fact::Exact(_)) => {
                CoalesceSelection::Left
            }
            None if self.expression_is_null(left) => CoalesceSelection::Right,
            None => CoalesceSelection::Dynamic,
        }
    }

    fn mir_resolved_type(&self, ty: &crate::types::ResolvedType) -> Option<mir::Type> {
        use crate::types::ResolvedType;
        match ty {
            ResolvedType::Integer(ty) => Some(mir::Type::Scalar(mir::ScalarType::Integer(*ty))),
            ResolvedType::Float(ty) => Some(mir::Type::Scalar(mir::ScalarType::Float(*ty))),
            ResolvedType::Bool => Some(mir::Type::Scalar(mir::ScalarType::Bool)),
            ResolvedType::String => Some(mir::Type::String),
            ResolvedType::Mixed => Some(mir::Type::Mixed),
            ResolvedType::TypeParameter(name) => self
                .type_substitutions
                .get(name)
                .and_then(|resolved| self.mir_resolved_type(resolved)),
            ResolvedType::Bytes => self
                .collection_registry
                .ids
                .get(&(
                    mir::CollectionKind::Bytes,
                    None,
                    mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8)),
                ))
                .copied()
                .map(mir::Type::Collection),
            ResolvedType::Class(class) => self.class_id_for_type(class).map(mir::Type::Class),
            ResolvedType::TypedArray(value) => self
                .mir_resolved_type(value)
                .and_then(|value| {
                    self.collection_registry.ids.get(&(
                        mir::CollectionKind::TypedArray,
                        None,
                        value,
                    ))
                })
                .copied()
                .map(mir::Type::Collection),
            ResolvedType::List(value) => self
                .mir_resolved_type(value)
                .and_then(|value| {
                    self.collection_registry
                        .ids
                        .get(&(mir::CollectionKind::List, None, value))
                })
                .copied()
                .map(mir::Type::Collection),
            ResolvedType::Dictionary(key, value) => {
                let key = self.mir_resolved_type(key)?;
                let value = self.mir_resolved_type(value)?;
                self.collection_registry
                    .ids
                    .get(&(mir::CollectionKind::Dictionary, Some(key), value))
                    .copied()
                    .map(mir::Type::Collection)
            }
            ResolvedType::Set(value) => self
                .mir_resolved_type(value)
                .and_then(|value| {
                    self.collection_registry
                        .ids
                        .get(&(mir::CollectionKind::Set, None, value))
                })
                .copied()
                .map(mir::Type::Collection),
            ResolvedType::Nullable(inner) => match self.mir_resolved_type(inner)? {
                mir::Type::Scalar(ty) => Some(mir::Type::NullableScalar(ty)),
                mir::Type::String => Some(mir::Type::NullableString),
                mir::Type::Mixed => Some(mir::Type::NullableMixed),
                mir::Type::Class(class) => Some(mir::Type::NullableClass(class)),
                _ => None,
            },
            ResolvedType::Void | ResolvedType::Null | ResolvedType::Unsupported => None,
        }
    }

    fn local_scalar_type(&self, id: mir::LocalId) -> DiagnosticResult<mir::ScalarType> {
        match self.local_type(id) {
            mir::Type::Scalar(ty) => Ok(ty),
            mir::Type::String
            | mir::Type::Mixed
            | mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::NullableMixed
            | mir::Type::Class(_)
            | mir::Type::NullableClass(_)
            | mir::Type::Collection(_) => Err(vec![Diagnostic::new(
                "I1401",
                format!(
                    "internal compiler consistency error: string local local{} used as a scalar",
                    id.0
                ),
                Span::default(),
            )]),
        }
    }
}

fn terminator_targets(terminator: &mir::Terminator) -> Vec<mir::BlockId> {
    match terminator {
        mir::Terminator::Jump(target) => vec![*target],
        mir::Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        mir::Terminator::Return(_)
        | mir::Terminator::ReturnVoid
        | mir::Terminator::Panic(_)
        | mir::Terminator::Unreachable => Vec::new(),
    }
}

fn lower_var_decl(decl: &hir::VarDecl, context: &mut LoweringContext) -> DiagnosticResult<()> {
    materialize_nested_collection_places(&decl.initializer, false, context)?;
    let ty = match &decl.ty {
        Some(ty) if context.native_type_ref(ty).is_some() => {
            context.native_type_ref(ty).expect("guarded native type")
        }
        Some(ty) => {
            return Err(vec![unsupported_native_type(
                ty,
                decl.span,
                format!("local type `{ty}` is not supported by native compilation"),
            )]);
        }
        None => context.expression_type(&decl.initializer)?,
    };

    if ty == mir::Type::String {
        return lower_string_var_decl(decl, context);
    }
    if ty == mir::Type::NullableString {
        let value = lower_nullable_string_expression(&decl.initializer, context)?;
        let local = context.declare_user_local(&decl.name, decl.writable, ty);
        context.push_statement(mir::Statement::AssignLocal {
            target: local,
            value: mir::Rvalue::NullableString(value),
        });
        return Ok(());
    }
    if let mir::Type::NullableScalar(scalar) = ty {
        let value = lower_nullable_scalar_expression(&decl.initializer, scalar, context)?;
        let local = context.declare_user_local(&decl.name, decl.writable, ty);
        context.push_statement(mir::Statement::AssignLocal {
            target: local,
            value: mir::Rvalue::NullableScalar(value),
        });
        return Ok(());
    }
    if let mir::Type::NullableClass(class) = ty {
        let value = lower_nullable_class_expression(&decl.initializer, class, true, context)?;
        let owned = !matches!(
            value,
            mir::NullableClassExpression::DictionaryGet {
                access: mir::NullableCollectionAccess::Get
                    | mir::NullableCollectionAccess::First
                    | mir::NullableCollectionAccess::Last,
                ..
            }
        );
        let local = context.declare_user_local_owned(&decl.name, decl.writable, ty, owned);
        context.push_statement(mir::Statement::AssignLocal {
            target: local,
            value: mir::Rvalue::NullableClass(value),
        });
        return Ok(());
    }
    if ty == mir::Type::Mixed {
        let value = lower_mixed_expression(&decl.initializer, true, context)?;
        let owned = value.ownership() != mir::MixedOwnership::None;
        let local = context.declare_user_local_owned(&decl.name, decl.writable, ty, owned);
        context.push_statement(mir::Statement::AssignLocal {
            target: local,
            value: mir::Rvalue::Mixed(value),
        });
        return Ok(());
    }
    if ty == mir::Type::NullableMixed {
        let value = lower_nullable_mixed_expression(&decl.initializer, true, context)?;
        let local = context.declare_user_local_owned(&decl.name, decl.writable, ty, true);
        context.push_statement(mir::Statement::AssignLocal {
            target: local,
            value: mir::Rvalue::NullableMixed(value),
        });
        return Ok(());
    }
    if let mir::Type::Class(class) = ty {
        let value = lower_class_expression(&decl.initializer, class, true, context)?;
        let local = context.declare_user_local(&decl.name, decl.writable, ty);
        context.push_statement(mir::Statement::AssignLocal {
            target: local,
            value: mir::Rvalue::Class(value),
        });
        return Ok(());
    }
    if let mir::Type::Collection(collection) = ty {
        let value = lower_collection_expression(&decl.initializer, collection, true, context)?;
        let local = context.declare_user_local(&decl.name, decl.writable, ty);
        context.push_statement(mir::Statement::AssignLocal {
            target: local,
            value: mir::Rvalue::Collection(value),
        });
        return Ok(());
    }

    let mir::Type::Scalar(scalar_type) = ty else {
        unreachable!("string locals return through lower_string_var_decl")
    };
    let value = lower_value_expression(&decl.initializer, context)?;
    ensure_value_type(&value, scalar_type, decl.initializer.span())?;
    let local =
        context.declare_user_local(&decl.name, decl.writable, mir::Type::Scalar(scalar_type));
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value: mir::Rvalue::Value(value),
    });
    Ok(())
}

fn inferred_class_type(expr: &hir::Expr, context: &mut LoweringContext) -> Option<ClassId> {
    match context.expression_type(expr).ok()? {
        mir::Type::Class(class) | mir::Type::NullableClass(class) => Some(class),
        _ => None,
    }
}

fn is_nullable_string_expression(expr: &hir::Expr, context: &mut LoweringContext) -> bool {
    context
        .expression_type(expr)
        .is_ok_and(|ty| ty == mir::Type::NullableString)
}

fn is_string_local_initializer(expr: &hir::Expr, context: &mut LoweringContext) -> bool {
    match expr {
        hir::Expr::String { .. } | hir::Expr::InterpolatedString { .. } => true,
        hir::Expr::Grouped { expr, .. } => is_string_local_initializer(expr, context),
        _ if matches!(
            context.constant_value(expr),
            Some(crate::const_eval::ConstValue::String(_))
        ) =>
        {
            true
        }
        hir::Expr::Binary {
            op: hir::BinaryOp::Concat,
            ..
        } => true,
        hir::Expr::Variable { name, span } => context
            .lookup_local(name, *span)
            .is_ok_and(|local| context.local_type(local) == mir::Type::String),
        hir::Expr::PropertyAccess { .. } => {
            lower_property_place(expr, context).is_ok_and(|(_, _, ty)| ty == mir::Type::String)
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => context
            .static_property(class_name, member, *span)
            .is_ok_and(|(_, ty)| ty == mir::Type::String),
        hir::Expr::FunctionCall { name, .. }
            if matches!(name.as_str(), "sprintf" | "read_file") =>
        {
            true
        }
        hir::Expr::FunctionCall { name, span, .. } => {
            context.lookup_function(name, *span).is_ok_and(|signature| {
                signature.return_type == mir::ReturnType::Value(mir::Type::String)
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            span,
            ..
        } => inferred_class_type(object, context).is_some_and(|class| {
            context
                .lookup_method(class, method, *span)
                .is_ok_and(|signature| {
                    signature.return_type == mir::ReturnType::Value(mir::Type::String)
                })
        }),
        hir::Expr::StaticCall {
            class_name,
            method,
            span,
            ..
        } => context.class_id_for_name(class_name).is_some_and(|class| {
            context
                .lookup_method(class, method, *span)
                .is_ok_and(|signature| {
                    signature.return_type == mir::ReturnType::Value(mir::Type::String)
                })
        }),
        _ => false,
    }
}

fn lower_string_var_decl(
    decl: &hir::VarDecl,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let value = lower_string_expression(&decl.initializer, context)?;
    let local = context.declare_user_local(&decl.name, decl.writable, mir::Type::String);
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value: mir::Rvalue::String(value),
    });
    Ok(())
}

#[derive(Clone)]
enum ScalarPlace {
    Local(mir::LocalId),
    NullableLocal(mir::LocalId),
    Property {
        object: mir::LocalId,
        property: crate::class_layout::PropertyId,
    },
    Static(mir::StaticId),
    CollectionIndex {
        collection: mir::LocalId,
        index: mir::Rvalue,
    },
}

impl ScalarPlace {
    fn operand(&self) -> mir::Operand {
        match self {
            Self::Local(local) => mir::Operand::Local(*local),
            Self::NullableLocal(local) => mir::Operand::NullablePayload(*local),
            Self::Property { object, property } => mir::Operand::Property {
                object: *object,
                property: *property,
            },
            Self::Static(id) => mir::Operand::Static(*id),
            Self::CollectionIndex { collection, index } => mir::Operand::CollectionIndex {
                collection: *collection,
                index: Box::new(index.clone()),
                remove: false,
            },
        }
    }

    fn assignment(self, value: mir::ValueExpression) -> mir::Statement {
        match self {
            Self::Local(target) => mir::Statement::AssignLocal {
                target,
                value: mir::Rvalue::Value(value),
            },
            Self::NullableLocal(target) => mir::Statement::AssignLocal {
                target,
                value: mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Value(value)),
            },
            Self::Property { object, property } => mir::Statement::AssignProperty {
                object,
                property,
                value: mir::Rvalue::Value(value),
            },
            Self::Static(target) => mir::Statement::AssignStatic {
                target,
                value: mir::Rvalue::Value(value),
            },
            Self::CollectionIndex { collection, index } => mir::Statement::AssignCollectionIndex {
                collection,
                index,
                value: mir::Rvalue::Value(value),
            },
        }
    }
}

fn unparenthesized_place(expr: &hir::Expr) -> &hir::Expr {
    match expr {
        hir::Expr::Grouped { expr, .. } => unparenthesized_place(expr),
        _ => expr,
    }
}

fn lower_scalar_place(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<(ScalarPlace, mir::ScalarType)> {
    match unparenthesized_place(expr) {
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::Scalar(ty) => Ok((ScalarPlace::Local(local), ty)),
                mir::Type::NullableScalar(ty) => Ok((ScalarPlace::NullableLocal(local), ty)),
                _ => Err(vec![unsupported(
                    *span,
                    "local is not a scalar mutation place",
                )]),
            }
        }
        hir::Expr::PropertyAccess { .. } => {
            let (object, property, ty) = lower_property_place(expr, context)?;
            let mir::Type::Scalar(scalar) = ty else {
                return Err(vec![unsupported(
                    expr.span(),
                    "class property is not a scalar mutation place",
                )]);
            };
            Ok((ScalarPlace::Property { object, property }, scalar))
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, ty) = context.static_property(class_name, member, *span)?;
            let mir::Type::Scalar(scalar) = ty else {
                return Err(vec![unsupported(
                    *span,
                    "static property is not a scalar mutation place",
                )]);
            };
            Ok((ScalarPlace::Static(id), scalar))
        }
        hir::Expr::Index {
            collection,
            index,
            span,
        } => {
            let (collection, collection_type) = lower_collection_local(collection, context)?;
            let info = context.collection_type(collection_type).clone();
            let mir::Type::Scalar(scalar) = info.value else {
                return Err(vec![unsupported(
                    *span,
                    "indexed collection element is not a scalar mutation place",
                )]);
            };
            let index_type = info
                .key
                .unwrap_or(mir::Type::Scalar(mir::ScalarType::Integer(
                    IntegerType::Int64,
                )));
            Ok((
                ScalarPlace::CollectionIndex {
                    collection,
                    index: lower_rvalue_as_expected(index, index_type, context)?,
                },
                scalar,
            ))
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "this scalar mutation place is not supported by native compilation",
        )]),
    }
}

fn lower_assignment(
    assignment: &hir::Assignment,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    materialize_nested_collection_places(&assignment.target, true, context)?;
    materialize_nested_collection_places(&assignment.value, false, context)?;
    if assignment.op != hir::AssignOp::Assign {
        let (place, scalar_type) = lower_scalar_place(&assignment.target, context)?;
        let value = lower_compound_value(
            place.operand(),
            scalar_type,
            &assignment.op,
            &assignment.value,
            context,
        )?;
        context.push_statement(place.assignment(value));
        return Ok(());
    }

    let target = unparenthesized_place(&assignment.target);
    if let hir::Expr::StaticMember {
        class_name,
        member,
        span,
    } = target
    {
        let (target, ty) = context.static_property(class_name, member, *span)?;
        let value = lower_rvalue_as_expected(&assignment.value, ty, context)?;
        context.push_statement(mir::Statement::AssignStatic { target, value });
        return Ok(());
    }
    if matches!(target, hir::Expr::PropertyAccess { .. }) {
        if let Ok((object, property, property_type)) = lower_property_place(target, context) {
            let value = lower_rvalue_as_expected(&assignment.value, property_type, context)?;
            context.push_statement(mir::Statement::AssignProperty {
                object,
                property,
                value,
            });
            return Ok(());
        }
    }
    if let hir::Expr::Index {
        collection,
        index,
        span,
    } = target
    {
        let (collection, collection_type) = lower_collection_local(collection, context)?;
        let info = context.collection_type(collection_type).clone();
        let index_type = info
            .key
            .unwrap_or(mir::Type::Scalar(mir::ScalarType::Integer(
                IntegerType::Int64,
            )));
        let index = lower_rvalue_as_expected(index, index_type, context)?;
        let value = lower_rvalue_as_expected(&assignment.value, info.value, context)?;
        context.push_statement(mir::Statement::AssignCollectionIndex {
            collection,
            index,
            value,
        });
        let _ = span;
        return Ok(());
    }
    let target = lower_assignment_target(target, context)?;
    if context.local_type(target) == mir::Type::String {
        let value = mir::Rvalue::String(lower_string_expression(&assignment.value, context)?);
        context.push_statement(mir::Statement::AssignLocal { target, value });
        return Ok(());
    }
    if context.local_type(target) == mir::Type::NullableString {
        let value = mir::Rvalue::NullableString(lower_nullable_string_expression(
            &assignment.value,
            context,
        )?);
        context.push_statement(mir::Statement::AssignLocal { target, value });
        return Ok(());
    }
    if let mir::Type::NullableScalar(scalar) = context.local_type(target) {
        let value = mir::Rvalue::NullableScalar(lower_nullable_scalar_expression(
            &assignment.value,
            scalar,
            context,
        )?);
        context.push_statement(mir::Statement::AssignLocal { target, value });
        return Ok(());
    }
    if let mir::Type::NullableClass(class) = context.local_type(target) {
        if !context.local_owns(target) {
            return Err(vec![unsupported(
                assignment.span,
                "this compiler version cannot replace a borrowed nullable class value",
            )]);
        }
        let value = mir::Rvalue::NullableClass(lower_nullable_class_expression(
            &assignment.value,
            class,
            true,
            context,
        )?);
        context.push_statement(mir::Statement::AssignLocal { target, value });
        return Ok(());
    }
    if context.local_type(target) == mir::Type::Mixed {
        if !context.local_owns(target) {
            return Err(vec![unsupported(
                assignment.span,
                "this compiler version cannot replace a borrowed mixed value",
            )]);
        }
        let value = mir::Rvalue::Mixed(lower_mixed_expression(&assignment.value, true, context)?);
        context.push_statement(mir::Statement::AssignLocal { target, value });
        return Ok(());
    }
    if context.local_type(target) == mir::Type::NullableMixed {
        if !context.local_owns(target) {
            return Err(vec![unsupported(
                assignment.span,
                "this compiler version cannot replace a borrowed nullable mixed value",
            )]);
        }
        let value = mir::Rvalue::NullableMixed(lower_nullable_mixed_expression(
            &assignment.value,
            true,
            context,
        )?);
        context.push_statement(mir::Statement::AssignLocal { target, value });
        return Ok(());
    }
    if let mir::Type::Class(class) = context.local_type(target) {
        if !context.local_owns(target) {
            return Err(vec![
                Diagnostic::new(
                    "E0505",
                    "this compiler version cannot replace the class value held through a borrowed parameter",
                    assignment.span,
                )
                .with_help("mutate the object's writable properties, or use a `take` parameter when the callee should own a replacement"),
            ]);
        }
        let value = mir::Rvalue::Class(lower_class_expression(
            &assignment.value,
            class,
            true,
            context,
        )?);
        context.push_statement(mir::Statement::AssignLocal { target, value });
        return Ok(());
    }
    if let mir::Type::Collection(collection) = context.local_type(target) {
        if !context.local_owns(target) {
            return Err(vec![unsupported(
                assignment.span,
                "this compiler version cannot replace a borrowed collection value",
            )]);
        }
        let value = mir::Rvalue::Collection(lower_collection_expression(
            &assignment.value,
            collection,
            true,
            context,
        )?);
        context.push_statement(mir::Statement::AssignLocal { target, value });
        return Ok(());
    }

    let scalar_type = context.local_scalar_type(target)?;
    let value = lower_value_expression(&assignment.value, context)?;
    ensure_value_type(&value, scalar_type, assignment.value.span())?;
    context.push_statement(mir::Statement::AssignLocal {
        target,
        value: mir::Rvalue::Value(value),
    });
    Ok(())
}

fn lower_increment(
    increment: &hir::IncrementStmt,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    materialize_nested_collection_places(&increment.target, true, context)?;
    let (place, scalar_type) = lower_scalar_place(&increment.target, context)?;
    let value = lower_increment_value(place.operand(), scalar_type, &increment.op, increment.span)?;
    context.push_statement(place.assignment(value));
    Ok(())
}

fn lower_increment_value(
    target: mir::Operand,
    scalar_type: mir::ScalarType,
    op: &hir::IncrementOp,
    span: Span,
) -> DiagnosticResult<mir::ValueExpression> {
    match scalar_type {
        mir::ScalarType::Integer(integer_type) => {
            let op = match op {
                hir::IncrementOp::Increment => mir::IntegerBinaryOp::Add,
                hir::IncrementOp::Decrement => mir::IntegerBinaryOp::Subtract,
            };
            Ok(mir::ValueExpression::Integer(
                mir::IntegerExpression::Binary {
                    ty: integer_type,
                    op,
                    left: Box::new(mir::IntegerExpression::use_operand(integer_type, target)),
                    right: Box::new(mir::IntegerExpression::constant(IntegerValue::one(
                        integer_type,
                    ))),
                },
            ))
        }
        mir::ScalarType::Float(float_type) => {
            let op = match op {
                hir::IncrementOp::Increment => mir::FloatBinaryOp::Add,
                hir::IncrementOp::Decrement => mir::FloatBinaryOp::Subtract,
            };
            let one = match float_type {
                FloatType::Float32 => FloatValue::from_f32(1.0),
                FloatType::Float64 => FloatValue::from_f64(1.0),
            };
            Ok(mir::ValueExpression::Float(mir::FloatExpression::Binary {
                ty: float_type,
                op,
                left: Box::new(mir::FloatExpression::Use {
                    ty: float_type,
                    operand: target,
                }),
                right: Box::new(mir::FloatExpression::constant(one)),
            }))
        }
        mir::ScalarType::Bool => Err(vec![unsupported(span, "bool increment is invalid")]),
    }
}

fn lower_assignment_target(
    target: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::LocalId> {
    match unparenthesized_place(target) {
        hir::Expr::Variable { name, span } => context.lookup_local(name, *span),
        _ => Err(vec![unsupported(
            target.span(),
            "this assignment target is not supported by native compilation",
        )]),
    }
}

fn lower_echo(expr: &hir::Expr, context: &mut LoweringContext) -> DiagnosticResult<mir::Statement> {
    materialize_nested_collection_places(expr, false, context)?;
    match expr {
        hir::Expr::String { value, .. } => Ok(mir::Statement::EchoStringLiteral(value.clone())),
        _ => lower_display_string_expression(expr, context).map(mir::Statement::EchoString),
    }
}

fn lower_panic_message(
    args: &[hir::Argument],
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::StringExpression> {
    let [message] = args else {
        return Err(vec![unsupported(
            span,
            format!("panic expects exactly 1 argument, got {}", args.len()),
        )]);
    };
    lower_string_expression(&message.value, context)
}

fn lower_string_expression(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::StringExpression> {
    if let Some((collection, index, value_type)) = lower_list_remove_at(expr, context)? {
        if value_type != mir::Type::String {
            return Err(vec![unsupported(
                expr.span(),
                "List::removeAt result has another value type",
            )]);
        }
        return Ok(mir::StringExpression::CollectionIndex {
            collection,
            index: Box::new(index),
            remove: true,
        });
    }
    if let Some(crate::const_eval::ConstValue::String(value)) = context.constant_value(expr) {
        return Ok(mir::StringExpression::Literal(value.clone()));
    }
    match expr {
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_string_expression(left, context),
            CoalesceSelection::Right => lower_string_expression(right, context),
            CoalesceSelection::Dynamic => Ok(mir::StringExpression::Coalesce {
                left: Box::new(lower_nullable_string_expression(left, context)?),
                right: Box::new(lower_string_expression(right, context)?),
            }),
        },
        hir::Expr::String { value, .. } => Ok(mir::StringExpression::Literal(value.clone())),
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            if context.local_type(local) == mir::Type::String {
                Ok(mir::StringExpression::Local(local))
            } else if matches!(
                context.local_type(local),
                mir::Type::Mixed | mir::Type::NullableMixed
            ) && context
                .exact_mixed_local(expr)
                .is_some_and(|(_, narrowed)| narrowed == mir::Type::String)
            {
                Ok(mir::StringExpression::MixedPayload(local))
            } else if context.local_type(local) == mir::Type::NullableString {
                Ok(mir::StringExpression::NullableLocalAssumeNonNull(local))
            } else {
                Err(vec![unsupported(
                    *span,
                    "this local cannot be used as a string expression",
                )])
            }
        }
        hir::Expr::Grouped { expr, .. } => lower_string_expression(expr, context),
        hir::Expr::PropertyAccess { .. } => {
            let (object, property) = lower_property_operand(expr, mir::Type::String, context)?;
            Ok(mir::StringExpression::Property { object, property })
        }
        hir::Expr::Index {
            collection, index, ..
        } => {
            let (collection, index) =
                lower_collection_index_operand(collection, index, mir::Type::String, context)?;
            Ok(mir::StringExpression::CollectionIndex {
                collection,
                index: Box::new(index),
                remove: false,
            })
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, ty) = context.static_property(class_name, member, *span)?;
            if ty != mir::Type::String {
                return Err(vec![unsupported(*span, "static property is not string")]);
            }
            Ok(mir::StringExpression::Static(id))
        }
        hir::Expr::Binary {
            op: hir::BinaryOp::Concat,
            ..
        } => {
            let mut parts = Vec::new();
            append_string_concat_parts(expr, context, &mut parts)?;
            Ok(mir::StringExpression::Concat(parts))
        }
        hir::Expr::InterpolatedString { parts, .. } => {
            let mut lowered = Vec::new();
            for part in parts {
                match part {
                    hir::InterpolatedStringPart::Text { value: text, .. } => {
                        lowered.push(mir::StringExpression::Literal(text.clone()));
                    }
                    hir::InterpolatedStringPart::Expr(expr) => {
                        lowered.push(lower_display_string_expression(expr, context)?);
                    }
                }
            }
            Ok(mir::StringExpression::Concat(lowered))
        }
        hir::Expr::FunctionCall { name, args, span } => {
            if name == "read_file" {
                let [path] = argument_values(args)[..] else {
                    return Err(vec![unsupported(*span, "read_file expects 1 argument")]);
                };
                return Ok(mir::StringExpression::ReadFile(Box::new(
                    lower_string_expression(path, context)?,
                )));
            }
            if name == "sprintf" {
                return Ok(mir::StringExpression::Format(Box::new(
                    lower_format_expression(args, *span, context)?,
                )));
            }
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::String) {
                return Err(vec![unsupported(
                    *span,
                    format!("function `{name}` does not return string"),
                )]);
            }
            Ok(mir::StringExpression::Call {
                function: signature.id,
                args: lower_call_args(name, args, signature, *span, context)?,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe,
        } => {
            if method == "toString"
                && args.is_empty()
                && !null_safe
                && context
                    .semantic_info
                    .constrained_display_calls
                    .contains(&(span.start, span.end))
            {
                return lower_display_string_expression(object, context);
            }
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::String) {
                return Err(vec![unsupported(*span, "method does not return string")]);
            }
            Ok(mir::StringExpression::Call {
                function: signature.id,
                args,
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::String) {
                return Err(vec![unsupported(
                    *span,
                    "static method does not return string",
                )]);
            }
            Ok(mir::StringExpression::Call {
                function: signature.id,
                args,
            })
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "this expression cannot be written by `echo` in native compilation",
        )]),
    }
}

fn lower_nullable_string_expression(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::NullableStringExpression> {
    if let Some((collection, key, value_type, access)) =
        lower_collection_nullable_property(expr, context)?
    {
        if value_type != mir::Type::String {
            return Err(vec![unsupported(
                expr.span(),
                "collection property has another value type",
            )]);
        }
        return Ok(mir::NullableStringExpression::DictionaryGet {
            collection,
            key: Box::new(key),
            access,
        });
    }
    if let Some(value) = context.constant_value(expr) {
        return match value {
            crate::const_eval::ConstValue::String(value) => {
                Ok(mir::NullableStringExpression::String(
                    mir::StringExpression::Literal(value.clone()),
                ))
            }
            crate::const_eval::ConstValue::Null => Ok(mir::NullableStringExpression::Null),
            _ => Err(vec![unsupported(
                expr.span(),
                "constant is not a nullable string",
            )]),
        };
    }
    match expr {
        hir::Expr::Null { .. } => Ok(mir::NullableStringExpression::Null),
        hir::Expr::Grouped { expr, .. } => lower_nullable_string_expression(expr, context),
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_nullable_string_expression(left, context),
            CoalesceSelection::Right => lower_nullable_string_expression(right, context),
            CoalesceSelection::Dynamic => Ok(mir::NullableStringExpression::Coalesce {
                left: Box::new(lower_nullable_string_expression(left, context)?),
                right: Box::new(lower_nullable_string_expression(right, context)?),
            }),
        },
        hir::Expr::PropertyAccess {
            object,
            property,
            null_safe: true,
            span,
        } => {
            let (object, property, ty) =
                lower_null_safe_property(object, property, *span, context)?;
            if !matches!(ty, mir::Type::String | mir::Type::NullableString) {
                return Err(vec![unsupported(
                    *span,
                    "null-safe property does not produce ?string",
                )]);
            }
            Ok(mir::NullableStringExpression::NullSafeProperty {
                object: Box::new(object),
                property,
            })
        }
        hir::Expr::PropertyAccess { .. } => {
            let (object, property, ty) = lower_property_place(expr, context)?;
            match ty {
                mir::Type::NullableString => {
                    Ok(mir::NullableStringExpression::Property { object, property })
                }
                mir::Type::String => Ok(mir::NullableStringExpression::String(
                    mir::StringExpression::Property { object, property },
                )),
                _ => Err(vec![unsupported(
                    expr.span(),
                    "property does not produce string or ?string",
                )]),
            }
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, ty) = context.static_property(class_name, member, *span)?;
            match ty {
                mir::Type::NullableString => Ok(mir::NullableStringExpression::Static(id)),
                mir::Type::String => Ok(mir::NullableStringExpression::String(
                    mir::StringExpression::Static(id),
                )),
                _ => Err(vec![unsupported(
                    *span,
                    "static property does not produce string or ?string",
                )]),
            }
        }
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::NullableString => match context.flow_fact(expr) {
                    Some(crate::narrowing::Fact::Null) => Ok(mir::NullableStringExpression::Null),
                    Some(crate::narrowing::Fact::NonNull | crate::narrowing::Fact::Exact(_)) => {
                        Ok(mir::NullableStringExpression::String(
                            mir::StringExpression::NullableLocalAssumeNonNull(local),
                        ))
                    }
                    None => Ok(mir::NullableStringExpression::Local(local)),
                },
                mir::Type::String => Ok(mir::NullableStringExpression::String(
                    mir::StringExpression::Local(local),
                )),
                _ => Err(vec![unsupported(
                    *span,
                    "expected nullable string expression",
                )]),
            }
        }
        hir::Expr::FunctionCall { name, args, span } if name == "read_line" => {
            if !args.is_empty() {
                return Err(vec![unsupported(*span, "read_line expects no arguments")]);
            }
            Ok(mir::NullableStringExpression::ReadLine)
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableString) => {
                    Ok(mir::NullableStringExpression::Call {
                        function: signature.id,
                        args: lower_call_args(name, args, signature, *span, context)?,
                    })
                }
                mir::ReturnType::Value(mir::Type::String) => Ok(
                    mir::NullableStringExpression::String(lower_string_expression(expr, context)?),
                ),
                _ => Err(vec![unsupported(
                    *span,
                    format!("function `{name}` does not return string or ?string"),
                )]),
            }
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: true,
        } => {
            let (object, signature, args) =
                lower_null_safe_method_call(object, method, args, *span, context)?;
            if !matches!(
                signature.return_type,
                mir::ReturnType::Value(mir::Type::String | mir::Type::NullableString)
            ) {
                return Err(vec![unsupported(
                    *span,
                    "null-safe method does not produce ?string",
                )]);
            }
            Ok(mir::NullableStringExpression::NullSafeCall {
                object: Box::new(object),
                function: signature.id,
                args,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
        } => {
            if let Some((collection, key, value_type, access)) =
                lower_dictionary_get(object, method, args, context)?
            {
                if value_type != mir::Type::String {
                    return Err(vec![unsupported(
                        *span,
                        "Dictionary::get has another value type",
                    )]);
                }
                return Ok(mir::NullableStringExpression::DictionaryGet {
                    collection,
                    key: Box::new(key),
                    access,
                });
            }
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableString) => {
                    Ok(mir::NullableStringExpression::Call {
                        function: signature.id,
                        args,
                    })
                }
                mir::ReturnType::Value(mir::Type::String) => Ok(
                    mir::NullableStringExpression::String(mir::StringExpression::Call {
                        function: signature.id,
                        args,
                    }),
                ),
                _ => Err(vec![unsupported(
                    *span,
                    "method does not return string or ?string",
                )]),
            }
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableString) => {
                    Ok(mir::NullableStringExpression::Call {
                        function: signature.id,
                        args,
                    })
                }
                mir::ReturnType::Value(mir::Type::String) => Ok(
                    mir::NullableStringExpression::String(mir::StringExpression::Call {
                        function: signature.id,
                        args,
                    }),
                ),
                _ => Err(vec![unsupported(
                    *span,
                    "static method does not return string or ?string",
                )]),
            }
        }
        _ if is_string_local_initializer(expr, context) => Ok(
            mir::NullableStringExpression::String(lower_string_expression(expr, context)?),
        ),
        _ => Err(vec![unsupported(
            expr.span(),
            "expected nullable string expression",
        )]),
    }
}

fn lower_nullable_scalar_expression(
    expr: &hir::Expr,
    expected: mir::ScalarType,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::NullableScalarExpression> {
    if let Some((collection, key, value_type, access)) =
        lower_collection_nullable_property(expr, context)?
    {
        if value_type != mir::Type::Scalar(expected) {
            return Err(vec![unsupported(
                expr.span(),
                "collection property has another scalar value type",
            )]);
        }
        return Ok(mir::NullableScalarExpression::DictionaryGet {
            ty: expected,
            collection,
            key: Box::new(key),
            access,
        });
    }
    if let hir::Expr::StaticCall {
        class_name,
        method,
        args,
        span,
    } = unparenthesized_place(expr)
    {
        if method == "parse" {
            let is_int = IntegerType::from_companion_name(class_name) == Some(IntegerType::Int64)
                && expected == mir::ScalarType::Integer(IntegerType::Int64);
            let is_float = matches!(class_name.as_str(), "Float" | "Float64")
                && expected == mir::ScalarType::Float(FloatType::Float64);
            if is_int || is_float {
                let [argument] = argument_values(args)[..] else {
                    return Err(vec![unsupported(
                        *span,
                        "parse expects exactly one string argument",
                    )]);
                };
                return Ok(mir::NullableScalarExpression::Parse {
                    ty: expected,
                    value: Box::new(lower_string_expression(argument, context)?),
                });
            }
        }
    }
    match expr {
        hir::Expr::Null { .. } => Ok(mir::NullableScalarExpression::Null(expected)),
        hir::Expr::Grouped { expr, .. } => {
            lower_nullable_scalar_expression(expr, expected, context)
        }
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_nullable_scalar_expression(left, expected, context),
            CoalesceSelection::Right => lower_nullable_scalar_expression(right, expected, context),
            CoalesceSelection::Dynamic => Ok(mir::NullableScalarExpression::Coalesce {
                ty: expected,
                left: Box::new(lower_nullable_scalar_expression(left, expected, context)?),
                right: Box::new(lower_nullable_scalar_expression(right, expected, context)?),
            }),
        },
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::NullableScalar(ty) if ty == expected => match context.flow_fact(expr) {
                    Some(crate::narrowing::Fact::Null) => {
                        Ok(mir::NullableScalarExpression::Null(ty))
                    }
                    Some(crate::narrowing::Fact::NonNull | crate::narrowing::Fact::Exact(_)) => {
                        Ok(mir::NullableScalarExpression::Value(
                            value_expression_from_operand(ty, mir::Operand::NullablePayload(local)),
                        ))
                    }
                    None => Ok(mir::NullableScalarExpression::Local { ty, local }),
                },
                mir::Type::Scalar(ty) if ty == expected => Ok(
                    mir::NullableScalarExpression::Value(lower_value_expression(expr, context)?),
                ),
                _ => Err(vec![unsupported(
                    *span,
                    "expected nullable scalar expression",
                )]),
            }
        }
        hir::Expr::PropertyAccess {
            object,
            property,
            null_safe: true,
            span,
        } => {
            let (object, property, ty) =
                lower_null_safe_property(object, property, *span, context)?;
            if !matches!(
                ty,
                mir::Type::Scalar(actual) | mir::Type::NullableScalar(actual)
                    if actual == expected
            ) {
                return Err(vec![unsupported(
                    *span,
                    "null-safe property has another scalar type",
                )]);
            }
            Ok(mir::NullableScalarExpression::NullSafeProperty {
                ty: expected,
                object: Box::new(object),
                property,
            })
        }
        hir::Expr::PropertyAccess { .. } => {
            let (object, property, ty) = lower_property_place(expr, context)?;
            match ty {
                mir::Type::NullableScalar(actual) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Property {
                        ty: expected,
                        object,
                        property,
                    })
                }
                mir::Type::Scalar(actual) if actual == expected => Ok(
                    mir::NullableScalarExpression::Value(value_expression_from_operand(
                        expected,
                        mir::Operand::Property { object, property },
                    )),
                ),
                _ => Err(vec![unsupported(
                    expr.span(),
                    "property has another scalar type",
                )]),
            }
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, ty) = context.static_property(class_name, member, *span)?;
            match ty {
                mir::Type::NullableScalar(actual) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Static { ty: expected, id })
                }
                mir::Type::Scalar(actual) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Value(
                        value_expression_from_operand(expected, mir::Operand::Static(id)),
                    ))
                }
                _ => Err(vec![unsupported(
                    *span,
                    "static property has another scalar type",
                )]),
            }
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableScalar(actual)) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Call {
                        ty: expected,
                        function: signature.id,
                        args: lower_call_args(name, args, signature, *span, context)?,
                    })
                }
                mir::ReturnType::Value(mir::Type::Scalar(actual)) if actual == expected => {
                    let value = lower_value_expression(expr, context)?;
                    ensure_value_type(&value, expected, *span)?;
                    Ok(mir::NullableScalarExpression::Value(value))
                }
                _ => Err(vec![unsupported(
                    *span,
                    "function has another scalar return type",
                )]),
            }
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: true,
        } => {
            let (object, signature, args) =
                lower_null_safe_method_call(object, method, args, *span, context)?;
            if !matches!(
                signature.return_type,
                mir::ReturnType::Value(
                    mir::Type::Scalar(actual) | mir::Type::NullableScalar(actual)
                ) if actual == expected
            ) {
                return Err(vec![unsupported(
                    *span,
                    "null-safe method has another scalar return type",
                )]);
            }
            Ok(mir::NullableScalarExpression::NullSafeCall {
                ty: expected,
                object: Box::new(object),
                function: signature.id,
                args,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
        } => {
            if let Some((collection, key, value_type, access)) =
                lower_dictionary_get(object, method, args, context)?
            {
                if value_type != mir::Type::Scalar(expected) {
                    return Err(vec![unsupported(
                        *span,
                        "Dictionary::get has another scalar value type",
                    )]);
                }
                return Ok(mir::NullableScalarExpression::DictionaryGet {
                    ty: expected,
                    collection,
                    key: Box::new(key),
                    access,
                });
            }
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableScalar(actual)) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Call {
                        ty: expected,
                        function: signature.id,
                        args,
                    })
                }
                mir::ReturnType::Value(mir::Type::Scalar(actual)) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Value(call_value_expression(
                        expected,
                        signature.id,
                        args,
                    )))
                }
                _ => Err(vec![unsupported(
                    *span,
                    "method has another scalar return type",
                )]),
            }
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableScalar(actual)) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Call {
                        ty: expected,
                        function: signature.id,
                        args,
                    })
                }
                mir::ReturnType::Value(mir::Type::Scalar(actual)) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Value(call_value_expression(
                        expected,
                        signature.id,
                        args,
                    )))
                }
                _ => Err(vec![unsupported(
                    *span,
                    "static method has another scalar return type",
                )]),
            }
        }
        _ => {
            let value = lower_value_expression(expr, context)?;
            ensure_value_type(&value, expected, expr.span())?;
            Ok(mir::NullableScalarExpression::Value(value))
        }
    }
}

fn lower_nullable_class_expression(
    expr: &hir::Expr,
    expected: ClassId,
    transfer: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::NullableClassExpression> {
    if let Some((collection, key, value_type, access)) =
        lower_collection_nullable_property(expr, context)?
    {
        if value_type != mir::Type::Class(expected) {
            return Err(vec![unsupported(
                expr.span(),
                "collection property has another class value type",
            )]);
        }
        return Ok(mir::NullableClassExpression::DictionaryGet {
            class: expected,
            collection,
            key: Box::new(key),
            access,
        });
    }
    match expr {
        hir::Expr::Null { .. } => Ok(mir::NullableClassExpression::Null(expected)),
        hir::Expr::Grouped { expr, .. } => {
            lower_nullable_class_expression(expr, expected, transfer, context)
        }
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => {
                lower_nullable_class_expression(left, expected, transfer, context)
            }
            CoalesceSelection::Right => {
                lower_nullable_class_expression(right, expected, transfer, context)
            }
            CoalesceSelection::Dynamic => Ok(mir::NullableClassExpression::Coalesce {
                class: expected,
                left: Box::new(lower_nullable_class_expression(
                    left, expected, transfer, context,
                )?),
                right: Box::new(lower_nullable_class_expression(
                    right, expected, transfer, context,
                )?),
                transfer,
            }),
        },
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::NullableClass(class) if class == expected => {
                    if transfer && !context.local_owns(local) {
                        return Err(vec![unsupported(
                            *span,
                            "borrowed nullable class value cannot be given away",
                        )]);
                    }
                    match context.flow_fact(expr) {
                        Some(crate::narrowing::Fact::Null) => {
                            Ok(mir::NullableClassExpression::Null(class))
                        }
                        Some(
                            crate::narrowing::Fact::NonNull | crate::narrowing::Fact::Exact(_),
                        ) => Ok(mir::NullableClassExpression::Class(lower_class_expression(
                            expr, expected, transfer, context,
                        )?)),
                        None => Ok(mir::NullableClassExpression::Local {
                            class,
                            local,
                            transfer,
                        }),
                    }
                }
                mir::Type::Class(class) if class == expected => {
                    Ok(mir::NullableClassExpression::Class(lower_class_expression(
                        expr, expected, transfer, context,
                    )?))
                }
                _ => Err(vec![unsupported(
                    *span,
                    "expected nullable class expression",
                )]),
            }
        }
        hir::Expr::PropertyAccess {
            object,
            property,
            null_safe: true,
            span,
        } => {
            let (object, property, ty) =
                lower_null_safe_property(object, property, *span, context)?;
            if !matches!(
                ty,
                mir::Type::Class(actual) | mir::Type::NullableClass(actual)
                    if actual == expected
            ) {
                return Err(vec![unsupported(
                    *span,
                    "null-safe property has another class type",
                )]);
            }
            Ok(mir::NullableClassExpression::NullSafeProperty {
                class: expected,
                object: Box::new(object),
                property,
            })
        }
        hir::Expr::PropertyAccess { span, .. } => {
            if transfer {
                return Err(vec![unsupported(
                    *span,
                    "moving directly out of an owned nullable class property is not supported",
                )]);
            }
            let (object, property, ty) = lower_property_place(expr, context)?;
            match ty {
                mir::Type::NullableClass(actual) if actual == expected => {
                    Ok(mir::NullableClassExpression::Property {
                        class: expected,
                        object,
                        property,
                    })
                }
                mir::Type::Class(actual) if actual == expected => Ok(
                    mir::NullableClassExpression::Class(mir::ClassExpression::Property {
                        class: expected,
                        object,
                        property,
                    }),
                ),
                _ => Err(vec![unsupported(*span, "property has another class type")]),
            }
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableClass(actual)) if actual == expected => {
                    Ok(mir::NullableClassExpression::Call {
                        class: expected,
                        function: signature.id,
                        return_borrow: signature.return_borrow,
                        args: lower_call_args_with_ownership(
                            name, args, signature, *span, context,
                        )?,
                    })
                }
                mir::ReturnType::Value(mir::Type::Class(actual)) if actual == expected => {
                    Ok(mir::NullableClassExpression::Class(lower_class_expression(
                        expr, expected, transfer, context,
                    )?))
                }
                _ => Err(vec![unsupported(
                    *span,
                    "function has another class return type",
                )]),
            }
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: true,
        } => {
            let (object, signature, args) =
                lower_null_safe_method_call(object, method, args, *span, context)?;
            if !matches!(
                signature.return_type,
                mir::ReturnType::Value(
                    mir::Type::Class(actual) | mir::Type::NullableClass(actual)
                ) if actual == expected
            ) {
                return Err(vec![unsupported(
                    *span,
                    "null-safe method has another class return type",
                )]);
            }
            Ok(mir::NullableClassExpression::NullSafeCall {
                class: expected,
                object: Box::new(object),
                function: signature.id,
                args,
                return_borrow: signature.return_borrow,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
        } => {
            if let Some((collection, key, value_type, access)) =
                lower_dictionary_get(object, method, args, context)?
            {
                if value_type != mir::Type::Class(expected) {
                    return Err(vec![unsupported(
                        *span,
                        "Dictionary::get has another class value type",
                    )]);
                }
                return Ok(mir::NullableClassExpression::DictionaryGet {
                    class: expected,
                    collection,
                    key: Box::new(key),
                    access,
                });
            }
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableClass(actual)) if actual == expected => {
                    Ok(mir::NullableClassExpression::Call {
                        class: expected,
                        function: signature.id,
                        args,
                        return_borrow: signature.return_borrow,
                    })
                }
                mir::ReturnType::Value(mir::Type::Class(actual)) if actual == expected => Ok(
                    mir::NullableClassExpression::Class(mir::ClassExpression::Call {
                        class: expected,
                        function: signature.id,
                        args,
                        return_borrow: signature.return_borrow,
                    }),
                ),
                _ => Err(vec![unsupported(
                    *span,
                    "method has another class return type",
                )]),
            }
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableClass(actual)) if actual == expected => {
                    Ok(mir::NullableClassExpression::Call {
                        class: expected,
                        function: signature.id,
                        args,
                        return_borrow: signature.return_borrow,
                    })
                }
                mir::ReturnType::Value(mir::Type::Class(actual)) if actual == expected => Ok(
                    mir::NullableClassExpression::Class(mir::ClassExpression::Call {
                        class: expected,
                        function: signature.id,
                        args,
                        return_borrow: signature.return_borrow,
                    }),
                ),
                _ => Err(vec![unsupported(
                    *span,
                    "static method has another class return type",
                )]),
            }
        }
        _ => Ok(mir::NullableClassExpression::Class(lower_class_expression(
            expr, expected, transfer, context,
        )?)),
    }
}

fn lower_null_safe_property(
    object: &hir::Expr,
    property: &str,
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<(mir::NullableClassExpression, PropertyId, mir::Type)> {
    let mir::Type::NullableClass(class) = context.expression_type(object)? else {
        return Err(vec![unsupported(
            object.span(),
            "null-safe receiver is not a nullable class",
        )]);
    };
    let property_info = context.property_info(class, property).ok_or_else(|| {
        vec![unsupported(
            span,
            format!("class#{} has no property `${property}`", class.0),
        )]
    })?;
    let ty = context
        .mir_resolved_type(&property_info.ty)
        .ok_or_else(|| {
            vec![unsupported(
                span,
                format!("property `${property}` is not native-lowerable"),
            )]
        })?;
    let property_id = property_info.id;
    Ok((
        lower_nullable_class_expression(object, class, false, context)?,
        property_id,
        ty,
    ))
}

fn lower_null_safe_method_call(
    object: &hir::Expr,
    method: &str,
    args: &[hir::Argument],
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<(
    mir::NullableClassExpression,
    FunctionSignature,
    Vec<mir::Rvalue>,
)> {
    let mir::Type::NullableClass(class) = context.expression_type(object)? else {
        return Err(vec![unsupported(
            object.span(),
            "null-safe receiver is not a nullable class",
        )]);
    };
    let signature = context.lookup_method(class, method, span)?;
    if signature.receiver_mode.is_none() {
        return Err(vec![unsupported(
            span,
            "null-safe call requires an instance method",
        )]);
    }
    let args = lower_call_args_with_ownership(method, args, signature.clone(), span, context)?;
    Ok((
        lower_nullable_class_expression(object, class, false, context)?,
        signature,
        args,
    ))
}

fn lower_format_expression(
    args: &[hir::Argument],
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::FormatExpression> {
    let Some(hir::Expr::String {
        value,
        span: format_span,
    }) = args.first().map(|argument| &argument.value)
    else {
        return Err(vec![unsupported(
            span,
            "format must be a direct string literal",
        )]);
    };
    let pieces = format_string::parse(value, *format_span).map_err(|error| vec![error])?;
    let specs = pieces.iter().filter_map(|piece| match piece {
        FormatPiece::Argument { spec, .. } => Some(*spec),
        FormatPiece::Literal(_) => None,
    });
    let arguments = args[1..]
        .iter()
        .map(|argument| &argument.value)
        .zip(specs)
        .map(|(argument, spec)| {
            if spec.conversion == FormatConversion::Display {
                let lowered = lower_display_string_expression(argument, context)?;
                if inferred_class_type(argument, context).is_some() {
                    Ok(mir::FormatArgument::ClassDisplay(lowered))
                } else {
                    Ok(mir::FormatArgument::String(lowered))
                }
            } else if is_string_local_initializer(argument, context) {
                lower_string_expression(argument, context).map(mir::FormatArgument::String)
            } else {
                lower_value_expression(argument, context).map(mir::FormatArgument::Value)
            }
        })
        .collect::<DiagnosticResult<Vec<_>>>()?;
    Ok(mir::FormatExpression { pieces, arguments })
}

fn lower_display_string_expression(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::StringExpression> {
    let ty = context.expression_type(expr)?;
    if let mir::Type::Class(class) = ty {
        let class_info = context.class_info(class).ok_or_else(|| {
            vec![unsupported(
                expr.span(),
                format!("unknown native class#{}", class.0),
            )]
        })?;
        if !class_info.implements_displayable {
            return Err(vec![unsupported(
                expr.span(),
                format!(
                    "class `{}` does not implement `Displayable`",
                    class_info.name
                ),
            )]);
        }
        let (signature, args) =
            lower_instance_method_call(expr, "toString", &[], expr.span(), context)?;
        if signature.return_type != mir::ReturnType::Value(mir::Type::String) {
            return Err(vec![unsupported(
                expr.span(),
                "`Displayable::toString` does not return string",
            )]);
        }
        return Ok(mir::StringExpression::Call {
            function: signature.id,
            args,
        });
    }
    match ty {
        mir::Type::String => lower_string_expression(expr, context),
        mir::Type::Scalar(_) => {
            lower_value_expression(expr, context).map(mir::StringExpression::Display)
        }
        mir::Type::Mixed => Err(vec![unsupported(
            expr.span(),
            "mixed values must be narrowed before display",
        )]),
        mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::NullableMixed
        | mir::Type::NullableClass(_) => Err(vec![unsupported(
            expr.span(),
            "nullable values must be narrowed or defaulted before display",
        )]),
        mir::Type::Class(_) => unreachable!("class display handled above"),
        mir::Type::Collection(_) => Err(vec![unsupported(
            expr.span(),
            "collection values do not have an implicit display representation",
        )]),
    }
}

fn append_string_concat_parts(
    expr: &hir::Expr,
    context: &mut LoweringContext,
    parts: &mut Vec<mir::StringExpression>,
) -> DiagnosticResult<()> {
    match expr {
        hir::Expr::Grouped { expr, .. } => append_string_concat_parts(expr, context, parts),
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Concat,
            right,
            ..
        } => {
            append_string_concat_parts(left, context, parts)?;
            append_string_concat_parts(right, context, parts)
        }
        hir::Expr::String { value, .. } => {
            parts.push(mir::StringExpression::Literal(value.clone()));
            Ok(())
        }
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            if context.local_type(local) == mir::Type::String {
                parts.push(mir::StringExpression::Local(local));
            } else if context.local_type(local) == mir::Type::NullableString {
                parts.push(mir::StringExpression::NullableLocalAssumeNonNull(local));
            } else {
                parts.push(lower_display_string_expression(expr, context)?);
            }
            Ok(())
        }
        _ => {
            parts.push(lower_display_string_expression(expr, context)?);
            Ok(())
        }
    }
}

fn lower_byte_file_write_statement(
    name: &str,
    args: &[hir::Argument],
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<bool> {
    if !matches!(name, "write_file_bytes" | "append_file_bytes") {
        return Ok(false);
    }
    let [path, contents] = argument_values(args)[..] else {
        return Err(vec![unsupported(
            span,
            format!("{name} expects 2 arguments"),
        )]);
    };

    materialize_nested_collection_places(path, false, context)?;
    let path = lower_string_expression(path, context)?;
    let path_local = context.declare_string_temp();
    context.push_statement(mir::Statement::AssignLocal {
        target: path_local,
        value: mir::Rvalue::String(path),
    });

    materialize_nested_collection_places(contents, false, context)?;
    materialize_collection_place(contents, false, context)?;
    let contents = lower_bytes_local(contents, context)?.0;
    context.push_statement(mir::Statement::WriteFileBytes {
        path: mir::StringExpression::Local(path_local),
        contents,
        append: name == "append_file_bytes",
    });
    context.push_statement(mir::Statement::DropString { local: path_local });
    Ok(true)
}

fn lower_statement_call(
    name: &str,
    args: &[hir::Argument],
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::Statement> {
    let signature = context.lookup_function(name, span)?;
    let args = lower_call_args(name, args, signature.clone(), span, context)?;
    discarded_call_statement("function", signature, args, span)
}

fn discarded_call_statement(
    kind: &str,
    signature: FunctionSignature,
    args: Vec<mir::Rvalue>,
    span: Span,
) -> DiagnosticResult<mir::Statement> {
    let statement = match signature.return_type {
        mir::ReturnType::Void => mir::Statement::CallVoid {
            function: signature.id,
            args,
        },
        mir::ReturnType::Value(
            mir::Type::Class(_)
            | mir::Type::NullableClass(_)
            | mir::Type::Collection(_)
            | mir::Type::Mixed
            | mir::Type::NullableMixed,
        ) if signature.return_borrow.is_some() => mir::Statement::CallBorrowed {
            function: signature.id,
            args,
        },
        mir::ReturnType::Value(_) => {
            return Err(vec![unsupported(
                span,
                format!("non-void {kind} call cannot be used as a statement"),
            )]);
        }
    };
    Ok(statement)
}

fn discarded_null_safe_call_statement(
    object: mir::NullableClassExpression,
    signature: FunctionSignature,
    args: Vec<mir::Rvalue>,
    span: Span,
) -> DiagnosticResult<mir::Statement> {
    let supported = matches!(signature.return_type, mir::ReturnType::Void)
        || matches!(
            signature.return_type,
            mir::ReturnType::Value(
                mir::Type::Class(_)
                    | mir::Type::NullableClass(_)
                    | mir::Type::Collection(_)
                    | mir::Type::Mixed
                    | mir::Type::NullableMixed
            )
        ) && signature.return_borrow.is_some();
    if !supported {
        return Err(vec![unsupported(
            span,
            "non-void method call cannot be used as a statement",
        )]);
    }
    Ok(mir::Statement::CallNullSafe {
        object,
        function: signature.id,
        args,
    })
}

fn lower_integer_call(
    name: &str,
    args: &[hir::Argument],
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<(mir::FunctionId, IntegerType, Vec<mir::Rvalue>)> {
    let signature = context.lookup_function(name, span)?;
    let mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(return_type))) =
        signature.return_type
    else {
        return Err(vec![unsupported(
            span,
            format!("void function `{name}` cannot be used as an integer expression"),
        )]);
    };

    let function = signature.id;
    let args = lower_call_args(name, args, signature, span, context)?;
    Ok((function, return_type, args))
}

fn lower_call_args(
    name: &str,
    args: &[hir::Argument],
    signature: FunctionSignature,
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<Vec<mir::Rvalue>> {
    lower_call_args_with_ownership(name, args, signature, span, context)
}

fn lower_call_args_with_ownership(
    name: &str,
    args: &[hir::Argument],
    signature: FunctionSignature,
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<Vec<mir::Rvalue>> {
    let required = signature
        .parameter_defaults
        .iter()
        .filter(|default| default.is_none())
        .count();
    let total = signature.parameter_types.len();
    if args.len() < required || args.len() > total {
        return Err(vec![unsupported(
            span,
            format!(
                "function `{name}` expects {required}..={total} positional argument(s), got {}",
                args.len()
            ),
        )]);
    }

    // Bind arguments to parameters by name (decision 0098). Positional-only
    // calls bind identically to before: argument i binds parameter i.
    let param_names: Vec<&str> = signature
        .parameter_names
        .iter()
        .map(String::as_str)
        .collect();
    let param_has_default: Vec<bool> = signature
        .parameter_defaults
        .iter()
        .map(Option::is_some)
        .collect();
    let arg_names: Vec<Option<&str>> = args
        .iter()
        .map(|arg| arg.name.as_ref().map(|name| name.text.as_str()))
        .collect();
    let bound = crate::arg_binding::bind_arguments(&param_names, &param_has_default, &arg_names);

    // A call needs no reordering when each argument binds the parameter at its
    // own source position. Then source order *is* parameter order, and the
    // arguments lower straight into the call vector as before — no temporaries.
    let in_order = bound
        .arg_to_param
        .iter()
        .enumerate()
        .all(|(arg_index, param)| *param == Some(arg_index));

    let mut lowered_args: Vec<Option<mir::Rvalue>> = vec![None; total];
    for (arg_index, arg) in args.iter().enumerate() {
        let Some(param_index) = bound.arg_to_param[arg_index] else {
            return Err(vec![Diagnostic::new(
                "I1302",
                format!(
                    "internal compiler consistency error: argument {} of `{name}` was not bound to a parameter after semantic checking",
                    arg_index + 1
                ),
                arg.span,
            )]);
        };
        let expected = signature.parameter_types[param_index];
        let transfers = signature.parameter_transfers[param_index];
        let value = &arg.value;
        let lowered = match expected {
            mir::Type::Class(class) => {
                mir::Rvalue::Class(lower_class_expression(value, class, transfers, context)?)
            }
            mir::Type::NullableClass(class) => mir::Rvalue::NullableClass(
                lower_nullable_class_expression(value, class, transfers, context)?,
            ),
            mir::Type::Collection(collection) => mir::Rvalue::Collection(
                lower_collection_expression(value, collection, transfers, context)?,
            ),
            mir::Type::Mixed => {
                mir::Rvalue::Mixed(lower_mixed_expression(value, transfers, context)?)
            }
            mir::Type::NullableMixed => mir::Rvalue::NullableMixed(
                lower_nullable_mixed_expression(value, transfers, context)?,
            ),
            _ => lower_rvalue_as_expected(value, expected, context)?,
        };
        if lowered.ty() != expected {
            return Err(vec![Diagnostic::new(
                "I1301",
                format!(
                    "internal compiler consistency error: argument to `{name}` has MIR type `{}`, expected `{expected}`",
                    lowered.ty()
                ),
                value.span(),
            )]);
        }

        // When the call reorders, every observable expression and every owned
        // temporary is evaluated in source order into a local here. Ownership is
        // checked from the lowered MIR rather than an expression-shape list:
        // constructing even a syntactically-pure collection affects destruction
        // order. The call vector then reads those locals in parameter order.
        let owns_temporary = lowered.owned_temporary_class().is_some()
            || lowered.owned_temporary_collection().is_some()
            || lowered.mixed_ownership().has_shell();
        lowered_args[param_index] = Some(
            if in_order || (!argument_evaluation_is_observable(value) && !owns_temporary) {
                lowered
            } else {
                hoist_argument_temporary(lowered, expected, context)
            },
        );
    }

    splice_omitted_parameter_defaults(name, &bound, &signature, span, &mut lowered_args)?;

    Ok(lowered_args
        .into_iter()
        .map(|arg| arg.expect("every parameter is supplied or defaulted after binding"))
        .collect())
}

/// Whether an argument expression's evaluation is observable — that is, whether
/// moving it relative to its neighbours could change program behavior. Only
/// expressions that can call user code (or an effectful intrinsic) qualify;
/// reads of locals, properties, and literals are pure, and a read that conflicts
/// with a sibling argument's write is already rejected by the one-writer rule.
fn argument_evaluation_is_observable(expr: &hir::Expr) -> bool {
    match expr {
        hir::Expr::FunctionCall { .. }
        | hir::Expr::MethodCall { .. }
        | hir::Expr::StaticCall { .. }
        | hir::Expr::New { .. } => true,
        hir::Expr::Grouped { expr, .. }
        | hir::Expr::Unary { expr, .. }
        | hir::Expr::IsType { expr, .. } => argument_evaluation_is_observable(expr),
        hir::Expr::Binary { left, right, .. } => {
            argument_evaluation_is_observable(left) || argument_evaluation_is_observable(right)
        }
        hir::Expr::Range { start, end, .. } => {
            argument_evaluation_is_observable(start) || argument_evaluation_is_observable(end)
        }
        hir::Expr::Index {
            collection, index, ..
        } => {
            argument_evaluation_is_observable(collection)
                || argument_evaluation_is_observable(index)
        }
        hir::Expr::PropertyAccess { object, .. } => argument_evaluation_is_observable(object),
        hir::Expr::InterpolatedString { parts, .. } => parts.iter().any(|part| match part {
            hir::InterpolatedStringPart::Expr(expr) => argument_evaluation_is_observable(expr),
            hir::InterpolatedStringPart::Text { .. } => false,
        }),
        hir::Expr::Array { elements, .. } => elements.iter().any(|element| {
            element
                .key
                .as_ref()
                .is_some_and(argument_evaluation_is_observable)
                || argument_evaluation_is_observable(&element.value)
        }),
        // Allocation and the runtime-negative-count check can panic even when
        // both operands are otherwise pure.
        hir::Expr::ArrayRepeat { .. } => true,
        _ => false,
    }
}

/// Evaluate one already-lowered argument into a fresh temporary local, and
/// return an rvalue that reads it back. This is what preserves source-order
/// evaluation when named binding reorders a call: the `AssignLocal` statements
/// are emitted in source order, and the call reads the temporaries in parameter
/// order.
fn hoist_argument_temporary(
    value: mir::Rvalue,
    ty: mir::Type,
    context: &mut LoweringContext,
) -> mir::Rvalue {
    let borrowed_class_value = value.borrows_class_value();
    let local = match ty {
        mir::Type::Scalar(_)
        | mir::Type::NullableScalar(_)
        | mir::Type::String
        | mir::Type::NullableString => context.declare_borrowed_temp(ty, false),
        mir::Type::Class(_) | mir::Type::NullableClass(_) if borrowed_class_value => {
            context.declare_borrowed_temp(ty, false)
        }
        mir::Type::Mixed
        | mir::Type::NullableMixed
        | mir::Type::Class(_)
        | mir::Type::NullableClass(_)
        | mir::Type::Collection(_) => context.declare_owned_temp(ty),
    };
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value,
    });
    read_local_as_rvalue(local, ty, !borrowed_class_value)
}

/// Read a temporary local back as an rvalue of its own type.
fn read_local_as_rvalue(local: mir::LocalId, ty: mir::Type, transfer_owned: bool) -> mir::Rvalue {
    let transfer = transfer_owned
        && matches!(
            ty,
            mir::Type::Mixed
                | mir::Type::NullableMixed
                | mir::Type::Class(_)
                | mir::Type::NullableClass(_)
                | mir::Type::Collection(_)
        );
    local_rvalue(local, ty, transfer)
}

/// Fill every parameter the call did not supply with its const-folded default
/// (decision 0086). Positional calls can only omit a trailing run, but a named
/// call may skip a defaulted parameter in the *middle* — the case 0086 could not
/// express — so the gaps are filled by parameter index rather than by appending.
fn splice_omitted_parameter_defaults(
    name: &str,
    bound: &crate::arg_binding::BoundArguments,
    signature: &FunctionSignature,
    span: Span,
    args: &mut [Option<mir::Rvalue>],
) -> DiagnosticResult<()> {
    for index in 0..signature.parameter_types.len() {
        if bound.param_to_arg[index].is_some() {
            continue;
        }
        let value = signature.parameter_defaults[index]
            .as_ref()
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "I2002",
                    format!(
                        "required parameter {} of `{name}` was omitted after semantic checking",
                        index + 1
                    ),
                    span,
                )]
            })?;
        args[index] = Some(lower_const_parameter_default(
            value,
            signature.parameter_types[index],
            span,
        )?);
    }
    Ok(())
}

fn lower_const_parameter_default(
    value: &crate::const_eval::ConstValue,
    expected: mir::Type,
    span: Span,
) -> DiagnosticResult<mir::Rvalue> {
    if let (crate::const_eval::ConstValue::String(value), mir::Type::String) = (value, expected) {
        return Ok(mir::Rvalue::String(mir::StringExpression::Literal(
            value.clone(),
        )));
    }

    let value = match (value, expected) {
        (
            crate::const_eval::ConstValue::Integer(value),
            mir::Type::Scalar(mir::ScalarType::Integer(integer)),
        ) if value.ty == integer => {
            mir::ValueExpression::Integer(mir::IntegerExpression::constant(*value))
        }
        (
            crate::const_eval::ConstValue::Float(value),
            mir::Type::Scalar(mir::ScalarType::Float(float)),
        ) if value.ty == float => {
            mir::ValueExpression::Float(mir::FloatExpression::constant(*value))
        }
        (crate::const_eval::ConstValue::Bool(value), mir::Type::Scalar(mir::ScalarType::Bool)) => {
            mir::ValueExpression::Bool(mir::BoolExpression::Use {
                operand: mir::Operand::Scalar(mir::ScalarValue::Bool(*value)),
            })
        }
        _ => {
            return Err(vec![Diagnostic::new(
                "I2003",
                "checked parameter default does not match its MIR parameter type",
                span,
            )]);
        }
    };
    Ok(mir::Rvalue::Value(value))
}

fn lower_instance_method_call(
    object: &hir::Expr,
    method: &str,
    args: &[hir::Argument],
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<(FunctionSignature, Vec<mir::Rvalue>)> {
    let class = inferred_class_type(object, context).ok_or_else(|| {
        vec![unsupported(
            object.span(),
            "method receiver does not have a concrete native class type",
        )]
    })?;
    let signature = context.lookup_method(class, method, span)?;
    if signature.receiver_mode.is_none() {
        return Err(vec![unsupported(
            span,
            format!(
                "static method `class#{}::{method}` has no receiver",
                class.0
            ),
        )]);
    }
    let mut lowered =
        lower_call_args_with_ownership(method, args, signature.clone(), span, context)?;
    lowered.insert(
        0,
        mir::Rvalue::Class(lower_class_expression(object, class, false, context)?),
    );
    Ok((signature, lowered))
}

fn lower_static_method_call(
    class_name: &str,
    method: &str,
    args: &[hir::Argument],
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<(FunctionSignature, Vec<mir::Rvalue>)> {
    let class = context
        .call_target_class_id(span)
        .or_else(|| context.class_id_for_name(class_name))
        .ok_or_else(|| vec![unsupported(span, format!("unknown class `{class_name}`"))])?;
    let signature = context.lookup_method(class, method, span)?;
    if signature.receiver_mode.is_some() {
        return Err(vec![unsupported(
            span,
            format!("instance method `{class_name}::{method}` requires a receiver"),
        )]);
    }
    let lowered = lower_call_args_with_ownership(method, args, signature.clone(), span, context)?;
    Ok((signature, lowered))
}

fn lower_return(
    expr: Option<&hir::Expr>,
    span: Span,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::Terminator> {
    if let Some(expr) = expr {
        materialize_nested_collection_places(expr, false, context)?;
    }
    match (return_type, expr) {
        (mir::ReturnType::Void, None) => {
            context.cleanup_scopes_from(0);
            Ok(mir::Terminator::ReturnVoid)
        }
        (mir::ReturnType::Value(expected), Some(expr)) => {
            let borrowed_move = matches!(
                expected,
                mir::Type::Class(_)
                    | mir::Type::NullableClass(_)
                    | mir::Type::Collection(_)
                    | mir::Type::Mixed
                    | mir::Type::NullableMixed
            ) && context.return_borrow.is_some();
            let value = match expected {
                mir::Type::Class(class) => {
                    lower_class_expression(expr, class, !borrowed_move, context)
                        .map(mir::Rvalue::Class)?
                }
                mir::Type::NullableClass(class) => {
                    lower_nullable_class_expression(expr, class, !borrowed_move, context)
                        .map(mir::Rvalue::NullableClass)?
                }
                _ if borrowed_move => lower_rvalue_as_borrowed(expr, expected, context)?,
                _ => lower_rvalue_as_expected(expr, expected, context)?,
            };
            if value.ty() != expected {
                return Err(vec![Diagnostic::new(
                    "I1301",
                    format!(
                        "internal compiler consistency error: return expression has MIR type `{}`, expected `{expected}`",
                        value.ty()
                    ),
                    expr.span(),
                )]);
            }
            if context.has_cleanup_obligations() {
                let result_owns = match expected {
                    mir::Type::Class(_)
                    | mir::Type::NullableClass(_)
                    | mir::Type::Collection(_)
                    | mir::Type::Mixed
                    | mir::Type::NullableMixed => !borrowed_move,
                    _ => false,
                };
                let result = context.declare_return_temp(expected, result_owns);
                context.push_statement(mir::Statement::AssignLocal {
                    target: result,
                    value,
                });
                context.cleanup_scopes_from(0);
                Ok(mir::Terminator::Return(local_rvalue(
                    result,
                    expected,
                    !borrowed_move,
                )))
            } else {
                Ok(mir::Terminator::Return(value))
            }
        }
        (mir::ReturnType::Value(_), None) => Err(vec![unsupported(
            span,
            "a value-returning function cannot use a bare `return`",
        )]),
        (mir::ReturnType::Void, Some(expr)) => Err(vec![unsupported(
            expr.span(),
            "a `void` function cannot return a value",
        )]),
    }
}

fn local_rvalue(local: mir::LocalId, ty: mir::Type, transfer: bool) -> mir::Rvalue {
    match ty {
        mir::Type::Scalar(mir::ScalarType::Integer(ty)) => mir::Rvalue::Value(
            mir::ValueExpression::Integer(local_integer_expression(local, ty)),
        ),
        mir::Type::Scalar(mir::ScalarType::Float(ty)) => mir::Rvalue::Value(
            mir::ValueExpression::Float(local_float_expression(local, ty)),
        ),
        mir::Type::Scalar(mir::ScalarType::Bool) => {
            mir::Rvalue::Value(mir::ValueExpression::Bool(mir::BoolExpression::Use {
                operand: mir::Operand::Local(local),
            }))
        }
        mir::Type::String => mir::Rvalue::String(mir::StringExpression::Local(local)),
        mir::Type::NullableScalar(ty) => {
            mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Local { ty, local })
        }
        mir::Type::NullableString => {
            mir::Rvalue::NullableString(mir::NullableStringExpression::Local(local))
        }
        mir::Type::Mixed => mir::Rvalue::Mixed(mir::MixedExpression::Local { local, transfer }),
        mir::Type::NullableMixed => {
            mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Local { local, transfer })
        }
        mir::Type::Class(class) => mir::Rvalue::Class(mir::ClassExpression::Local {
            class,
            local,
            transfer,
        }),
        mir::Type::NullableClass(class) => {
            mir::Rvalue::NullableClass(mir::NullableClassExpression::Local {
                class,
                local,
                transfer,
            })
        }
        mir::Type::Collection(collection) => {
            mir::Rvalue::Collection(mir::CollectionExpression::Local {
                collection,
                local,
                transfer,
            })
        }
    }
}

fn lower_condition_to_blocks(
    expr: &hir::Expr,
    then_block: mir::BlockId,
    else_block: mir::BlockId,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    match unparenthesized_place(expr) {
        hir::Expr::Unary {
            op: hir::UnaryOp::Not,
            expr,
            ..
        } => lower_condition_to_blocks(expr, else_block, then_block, context),
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::And,
            right,
            ..
        } => {
            let right_block = context.create_block();
            lower_condition_to_blocks(left, right_block, else_block, context)?;
            context.current_block = Some(right_block);
            lower_condition_to_blocks(right, then_block, else_block, context)
        }
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Or,
            right,
            ..
        } => {
            let right_block = context.create_block();
            lower_condition_to_blocks(left, then_block, right_block, context)?;
            context.current_block = Some(right_block);
            lower_condition_to_blocks(right, then_block, else_block, context)
        }
        _ => {
            materialize_nested_collection_places(expr, false, context)?;
            let condition = lower_condition(expr, context)?;
            context.terminate_condition(condition, then_block, else_block);
            Ok(())
        }
    }
}

fn lower_condition(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::BoolExpression> {
    if let Some(crate::const_eval::ConstValue::Bool(value)) = context.constant_value(expr) {
        return Ok(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(*value)),
        });
    }
    match expr {
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_condition(left, context),
            CoalesceSelection::Right => lower_condition(right, context),
            CoalesceSelection::Dynamic => Ok(mir::BoolExpression::Coalesce {
                left: Box::new(lower_nullable_scalar_expression(
                    left,
                    mir::ScalarType::Bool,
                    context,
                )?),
                right: Box::new(lower_condition(right, context)?),
            }),
        },
        hir::Expr::IsType { expr, span, .. } => lower_is_condition(expr, *span, context),
        hir::Expr::Bool { value, .. } => Ok(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(*value)),
        }),
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::Scalar(mir::ScalarType::Bool) => Ok(mir::BoolExpression::Use {
                    operand: mir::Operand::Local(local),
                }),
                mir::Type::Mixed | mir::Type::NullableMixed
                    if context
                        .exact_mixed_local(expr)
                        .is_some_and(|(_, narrowed)| {
                            narrowed == mir::Type::Scalar(mir::ScalarType::Bool)
                        }) =>
                {
                    Ok(mir::BoolExpression::Use {
                        operand: mir::Operand::MixedPayload {
                            mixed: local,
                            tag: mir::MixedTag::Bool,
                        },
                    })
                }
                mir::Type::NullableScalar(mir::ScalarType::Bool) => Ok(mir::BoolExpression::Use {
                    operand: mir::Operand::NullablePayload(local),
                }),
                _ => Err(vec![unsupported(
                    *span,
                    "only bool locals may be used as conditions",
                )]),
            }
        }
        hir::Expr::Grouped { expr, .. } => lower_condition(expr, context),
        hir::Expr::PropertyAccess {
            object, property, ..
        } => {
            if property == "isEmpty" {
                if let Ok((collection, _)) = lower_collection_local(object, context) {
                    return Ok(mir::BoolExpression::CollectionIsEmpty { collection });
                }
            }
            let (object, property) =
                lower_property_operand(expr, mir::Type::Scalar(mir::ScalarType::Bool), context)?;
            Ok(mir::BoolExpression::Use {
                operand: mir::Operand::Property { object, property },
            })
        }
        hir::Expr::Index {
            collection, index, ..
        } => {
            let (collection, index) = lower_collection_index_operand(
                collection,
                index,
                mir::Type::Scalar(mir::ScalarType::Bool),
                context,
            )?;
            Ok(mir::BoolExpression::Use {
                operand: mir::Operand::CollectionIndex {
                    collection,
                    index: Box::new(index),
                    remove: false,
                },
            })
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, ty) = context.static_property(class_name, member, *span)?;
            if ty != mir::Type::Scalar(mir::ScalarType::Bool) {
                return Err(vec![unsupported(*span, "static property is not bool")]);
            }
            Ok(mir::BoolExpression::Use {
                operand: mir::Operand::Static(id),
            })
        }
        hir::Expr::Unary {
            op: hir::UnaryOp::Not,
            expr,
            ..
        } => Ok(mir::BoolExpression::Not(Box::new(lower_condition(
            expr, context,
        )?))),
        hir::Expr::Binary {
            left, op, right, ..
        } => match op {
            hir::BinaryOp::Equal
            | hir::BinaryOp::NotEqual
            | hir::BinaryOp::Less
            | hir::BinaryOp::LessEqual
            | hir::BinaryOp::Greater
            | hir::BinaryOp::GreaterEqual => {
                if matches!(unparenthesized_place(left), hir::Expr::Null { .. })
                    || matches!(unparenthesized_place(right), hir::Expr::Null { .. })
                {
                    lower_null_comparison(left, op, right, context)
                } else if lower_bytes_local(left, context).is_ok()
                    && lower_bytes_local(right, context).is_ok()
                    && matches!(op, hir::BinaryOp::Equal | hir::BinaryOp::NotEqual)
                {
                    let equal = mir::BoolExpression::CollectionEqual {
                        left: lower_bytes_local(left, context)?.0,
                        right: lower_bytes_local(right, context)?.0,
                    };
                    Ok(if *op == hir::BinaryOp::NotEqual {
                        mir::BoolExpression::Not(Box::new(equal))
                    } else {
                        equal
                    })
                } else if is_nullable_string_expression(left, context)
                    || is_nullable_string_expression(right, context)
                {
                    Ok(mir::BoolExpression::NullableStringCompare {
                        op: lower_compare_op(op),
                        left: Box::new(lower_nullable_string_expression(left, context)?),
                        right: Box::new(lower_nullable_string_expression(right, context)?),
                    })
                } else if is_string_local_initializer(left, context)
                    || is_string_local_initializer(right, context)
                {
                    Ok(mir::BoolExpression::StringCompare {
                        op: lower_compare_op(op),
                        left: Box::new(lower_string_expression(left, context)?),
                        right: Box::new(lower_string_expression(right, context)?),
                    })
                } else {
                    Ok(mir::BoolExpression::Compare {
                        op: lower_compare_op(op),
                        left: Box::new(lower_value_expression(left, context)?),
                        right: Box::new(lower_value_expression(right, context)?),
                    })
                }
            }
            hir::BinaryOp::And | hir::BinaryOp::Or | hir::BinaryOp::Xor => {
                Ok(mir::BoolExpression::Binary {
                    op: lower_condition_binary_op(op),
                    left: Box::new(lower_condition(left, context)?),
                    right: Box::new(lower_condition(right, context)?),
                })
            }
            _ => Err(vec![unsupported(
                expr.span(),
                "conditions require boolean values, scalar comparisons, or boolean operators",
            )]),
        },
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Bool))
            {
                return Err(vec![unsupported(
                    *span,
                    format!("function `{name}` does not return bool"),
                )]);
            }
            Ok(mir::BoolExpression::Call {
                function: signature.id,
                args: lower_call_args(name, args, signature, *span, context)?,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            ..
        } => {
            if let Some((collection, index, value_type)) = lower_list_remove_at(expr, context)? {
                if value_type != mir::Type::Scalar(mir::ScalarType::Bool) {
                    return Err(vec![unsupported(
                        *span,
                        "List::removeAt result is not bool",
                    )]);
                }
                return Ok(mir::BoolExpression::Use {
                    operand: mir::Operand::CollectionIndex {
                        collection,
                        index: Box::new(index),
                        remove: true,
                    },
                });
            }
            if let Ok((collection, collection_type)) = lower_collection_local(object, context) {
                let info = context.collection_type(collection_type).clone();
                let op = match (info.kind, method.as_str()) {
                    (mir::CollectionKind::List, "contains")
                    | (mir::CollectionKind::Dictionary, "has")
                    | (mir::CollectionKind::Set, "contains") => {
                        Some(mir::CollectionMembershipOp::Contains)
                    }
                    (mir::CollectionKind::Set, "add") => Some(mir::CollectionMembershipOp::Add),
                    (mir::CollectionKind::Set, "remove") => {
                        Some(mir::CollectionMembershipOp::Remove)
                    }
                    _ => None,
                };
                if let Some(op) = op {
                    let [value] = argument_values(args)[..] else {
                        return Err(vec![unsupported(
                            *span,
                            format!("collection `{method}` expects 1 argument"),
                        )]);
                    };
                    let value_type = info.key.unwrap_or(info.value);
                    let value = if op == mir::CollectionMembershipOp::Add {
                        lower_rvalue_as_expected(value, value_type, context)?
                    } else {
                        lower_rvalue_as_borrowed(value, value_type, context)?
                    };
                    return Ok(mir::BoolExpression::CollectionHas {
                        collection,
                        value: Box::new(value),
                        op,
                    });
                }
            }
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Bool))
            {
                return Err(vec![unsupported(*span, "method does not return bool")]);
            }
            Ok(mir::BoolExpression::Call {
                function: signature.id,
                args,
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Bool))
            {
                return Err(vec![unsupported(
                    *span,
                    "static method does not return bool",
                )]);
            }
            Ok(mir::BoolExpression::Call {
                function: signature.id,
                args,
            })
        }
        hir::Expr::Int { .. } => Err(vec![unsupported(
            expr.span(),
            "integer truthiness is not supported; conditions require a `bool` value",
        )]),
        _ => Err(vec![unsupported(
            expr.span(),
            "this expression cannot be used as a condition in native compilation",
        )]),
    }
}

fn lower_null_comparison(
    left: &hir::Expr,
    op: &hir::BinaryOp,
    right: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::BoolExpression> {
    let value = if matches!(unparenthesized_place(left), hir::Expr::Null { .. }) {
        right
    } else {
        left
    };
    let value_type = if let hir::Expr::Variable { name, span } = unparenthesized_place(value) {
        if let Ok(local) = context.lookup_local(name, *span) {
            context.local_type(local)
        } else {
            context.expression_type(value)?
        }
    } else {
        context.expression_type(value)?
    };
    let present = match value_type {
        mir::Type::NullableScalar(ty) => mir::BoolExpression::NullableScalarIsPresent(Box::new(
            lower_nullable_scalar_presence_subject(value, ty, context)?,
        )),
        mir::Type::NullableString => {
            return Ok(mir::BoolExpression::NullableStringCompare {
                op: lower_compare_op(op),
                left: Box::new(lower_nullable_string_presence_subject(value, context)?),
                right: Box::new(mir::NullableStringExpression::Null),
            });
        }
        mir::Type::NullableClass(class) => mir::BoolExpression::NullableClassIsPresent(Box::new(
            lower_nullable_class_presence_subject(value, class, context)?,
        )),
        mir::Type::NullableMixed => {
            let present = mir::BoolExpression::NullableMixedIsPresent(Box::new(
                lower_nullable_mixed_presence_subject(value, context)?,
            ));
            return Ok(if matches!(op, hir::BinaryOp::Equal) {
                mir::BoolExpression::Not(Box::new(present))
            } else {
                present
            });
        }
        _ => {
            return Err(vec![unsupported(
                value.span(),
                "null comparison requires a nullable value",
            )]);
        }
    };
    Ok(if matches!(op, hir::BinaryOp::Equal) {
        mir::BoolExpression::Not(Box::new(present))
    } else {
        present
    })
}

fn lower_is_condition(
    expr: &hir::Expr,
    type_test_span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::BoolExpression> {
    let tested_type = context
        .semantic_info
        .type_test_type(type_test_span)
        .and_then(|resolved| context.mir_resolved_type(resolved));
    let Some(tested_type) = tested_type else {
        return Err(vec![unsupported(
            expr.span(),
            "type test does not name a native concrete type",
        )]);
    };
    if context.expression_is_null(expr) {
        return Ok(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(false)),
        });
    }
    let value_type = context.expression_type(expr)?;
    let result = match value_type {
        mir::Type::NullableScalar(ty) if tested_type == mir::Type::Scalar(ty) => {
            mir::BoolExpression::NullableScalarIsPresent(Box::new(
                lower_nullable_scalar_presence_subject(expr, ty, context)?,
            ))
        }
        mir::Type::NullableString if tested_type == mir::Type::String => {
            mir::BoolExpression::Not(Box::new(mir::BoolExpression::NullableStringCompare {
                op: mir::CompareOp::Equal,
                left: Box::new(lower_nullable_string_presence_subject(expr, context)?),
                right: Box::new(mir::NullableStringExpression::Null),
            }))
        }
        mir::Type::NullableClass(class) if tested_type == mir::Type::Class(class) => {
            mir::BoolExpression::NullableClassIsPresent(Box::new(
                lower_nullable_class_presence_subject(expr, class, context)?,
            ))
        }
        mir::Type::Mixed => mir::BoolExpression::MixedIs {
            mixed: Box::new(lower_mixed_expression(expr, false, context)?),
            tag: mixed_tag_for_type(tested_type, type_test_span)?,
        },
        mir::Type::NullableMixed => mir::BoolExpression::Binary {
            op: mir::BoolBinaryOp::And,
            left: Box::new(mir::BoolExpression::NullableMixedIsPresent(Box::new(
                lower_nullable_mixed_presence_subject(expr, context)?,
            ))),
            right: Box::new(mir::BoolExpression::MixedIs {
                mixed: Box::new(mir::MixedExpression::Local {
                    local: lower_nullable_mixed_local(expr, context)?,
                    transfer: false,
                }),
                tag: mixed_tag_for_type(tested_type, type_test_span)?,
            }),
        },
        mir::Type::Scalar(_) | mir::Type::String | mir::Type::Class(_) => {
            let evaluated = lower_concrete_is_presence(expr, value_type, context)?;
            if value_type == tested_type {
                evaluated
            } else {
                mir::BoolExpression::Not(Box::new(evaluated))
            }
        }
        mir::Type::NullableScalar(ty) => {
            evaluate_then_false(mir::BoolExpression::NullableScalarIsPresent(Box::new(
                lower_nullable_scalar_presence_subject(expr, ty, context)?,
            )))
        }
        mir::Type::NullableString => evaluate_then_false(mir::BoolExpression::Not(Box::new(
            mir::BoolExpression::NullableStringCompare {
                op: mir::CompareOp::Equal,
                left: Box::new(lower_nullable_string_presence_subject(expr, context)?),
                right: Box::new(mir::NullableStringExpression::Null),
            },
        ))),
        mir::Type::NullableClass(class) => {
            evaluate_then_false(mir::BoolExpression::NullableClassIsPresent(Box::new(
                lower_nullable_class_presence_subject(expr, class, context)?,
            )))
        }
        mir::Type::Collection(_) => {
            let value = mir::BoolExpression::Use {
                operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
            };
            if value_type == tested_type {
                value
            } else {
                mir::BoolExpression::Not(Box::new(value))
            }
        }
    };
    Ok(result)
}

fn lower_nullable_mixed_local(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::LocalId> {
    if let hir::Expr::Variable { name, span } = unparenthesized_place(expr) {
        let local = context.lookup_local(name, *span)?;
        if context.local_type(local) == mir::Type::NullableMixed {
            return Ok(local);
        }
    }
    Err(vec![unsupported(
        expr.span(),
        "this nullable mixed expression cannot be used as a native presence subject",
    )])
}

fn lower_nullable_mixed_presence_subject(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::NullableMixedExpression> {
    let local = lower_nullable_mixed_local(expr, context)?;
    Ok(mir::NullableMixedExpression::Local {
        local,
        transfer: false,
    })
}

fn lower_nullable_scalar_presence_subject(
    expr: &hir::Expr,
    expected: mir::ScalarType,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::NullableScalarExpression> {
    if let hir::Expr::Variable { name, span } = unparenthesized_place(expr) {
        let local = context.lookup_local(name, *span)?;
        if context.local_type(local) == mir::Type::NullableScalar(expected) {
            return Ok(mir::NullableScalarExpression::Local {
                ty: expected,
                local,
            });
        }
    }
    lower_nullable_scalar_expression(expr, expected, context)
}

fn lower_nullable_string_presence_subject(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::NullableStringExpression> {
    if let hir::Expr::Variable { name, span } = unparenthesized_place(expr) {
        let local = context.lookup_local(name, *span)?;
        if context.local_type(local) == mir::Type::NullableString {
            return Ok(mir::NullableStringExpression::Local(local));
        }
    }
    lower_nullable_string_expression(expr, context)
}

fn lower_nullable_class_presence_subject(
    expr: &hir::Expr,
    expected: ClassId,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::NullableClassExpression> {
    if let hir::Expr::Variable { name, span } = unparenthesized_place(expr) {
        let local = context.lookup_local(name, *span)?;
        if context.local_type(local) == mir::Type::NullableClass(expected) {
            return Ok(mir::NullableClassExpression::Local {
                class: expected,
                local,
                transfer: false,
            });
        }
    }
    lower_nullable_class_expression(expr, expected, false, context)
}

fn evaluate_then_false(condition: mir::BoolExpression) -> mir::BoolExpression {
    mir::BoolExpression::Binary {
        op: mir::BoolBinaryOp::And,
        left: Box::new(condition),
        right: Box::new(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(false)),
        }),
    }
}

fn lower_concrete_is_presence(
    expr: &hir::Expr,
    value_type: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::BoolExpression> {
    match value_type {
        mir::Type::Scalar(ty) => Ok(mir::BoolExpression::NullableScalarIsPresent(Box::new(
            mir::NullableScalarExpression::Value({
                let value = lower_value_expression(expr, context)?;
                ensure_value_type(&value, ty, expr.span())?;
                value
            }),
        ))),
        mir::Type::String => Ok(mir::BoolExpression::Not(Box::new(
            mir::BoolExpression::NullableStringCompare {
                op: mir::CompareOp::Equal,
                left: Box::new(mir::NullableStringExpression::String(
                    lower_string_expression(expr, context)?,
                )),
                right: Box::new(mir::NullableStringExpression::Null),
            },
        ))),
        mir::Type::Class(class) => Ok(mir::BoolExpression::NullableClassIsPresent(Box::new(
            mir::NullableClassExpression::Class(lower_class_expression(
                expr, class, false, context,
            )?),
        ))),
        mir::Type::Mixed => Ok(mir::BoolExpression::NullableMixedIsPresent(Box::new(
            mir::NullableMixedExpression::Mixed(lower_mixed_expression(expr, false, context)?),
        ))),
        mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::NullableMixed
        | mir::Type::NullableClass(_) => unreachable!("concrete `is` value type"),
        mir::Type::Collection(_) => Ok(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
        }),
    }
}

fn lower_compare_op(op: &hir::BinaryOp) -> mir::CompareOp {
    match op {
        hir::BinaryOp::Equal => mir::CompareOp::Equal,
        hir::BinaryOp::NotEqual => mir::CompareOp::NotEqual,
        hir::BinaryOp::Less => mir::CompareOp::Less,
        hir::BinaryOp::LessEqual => mir::CompareOp::LessEqual,
        hir::BinaryOp::Greater => mir::CompareOp::Greater,
        hir::BinaryOp::GreaterEqual => mir::CompareOp::GreaterEqual,
        _ => unreachable!("only comparison operators are lowered as MIR comparisons"),
    }
}

fn lower_condition_binary_op(op: &hir::BinaryOp) -> mir::BoolBinaryOp {
    match op {
        hir::BinaryOp::And => mir::BoolBinaryOp::And,
        hir::BinaryOp::Or => mir::BoolBinaryOp::Or,
        hir::BinaryOp::Xor => mir::BoolBinaryOp::Xor,
        _ => unreachable!("only boolean operators are lowered as MIR condition operators"),
    }
}

fn lower_value_expression(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::ValueExpression> {
    if let hir::Expr::FunctionCall { name, span, .. } = expr {
        if context.lookup_function(name, *span)?.return_type == mir::ReturnType::Void {
            return Err(vec![unsupported(
                *span,
                format!("void function `{name}` cannot be used as a scalar expression"),
            )]);
        }
    }
    if context.semantic_info.integer_type(expr.span()).is_some() {
        return lower_integer_expression(expr, context).map(mir::ValueExpression::Integer);
    }
    if context.semantic_info.float_type(expr.span()).is_some() {
        return lower_float_expression(expr, context).map(mir::ValueExpression::Float);
    }
    match context.expression_type(expr)? {
        mir::Type::Scalar(mir::ScalarType::Integer(_))
        | mir::Type::NullableScalar(mir::ScalarType::Integer(_)) => {
            lower_integer_expression(expr, context).map(mir::ValueExpression::Integer)
        }
        mir::Type::Scalar(mir::ScalarType::Float(_))
        | mir::Type::NullableScalar(mir::ScalarType::Float(_)) => {
            lower_float_expression(expr, context).map(mir::ValueExpression::Float)
        }
        mir::Type::Scalar(mir::ScalarType::Bool)
        | mir::Type::NullableScalar(mir::ScalarType::Bool) => {
            lower_condition(expr, context).map(mir::ValueExpression::Bool)
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "this expression is not a scalar value",
        )]),
    }
}

fn call_value_expression(
    ty: mir::ScalarType,
    function: mir::FunctionId,
    args: Vec<mir::Rvalue>,
) -> mir::ValueExpression {
    match ty {
        mir::ScalarType::Integer(ty) => {
            mir::ValueExpression::Integer(mir::IntegerExpression::Call { ty, function, args })
        }
        mir::ScalarType::Float(ty) => {
            mir::ValueExpression::Float(mir::FloatExpression::Call { ty, function, args })
        }
        mir::ScalarType::Bool => {
            mir::ValueExpression::Bool(mir::BoolExpression::Call { function, args })
        }
    }
}

fn value_expression_from_operand(
    ty: mir::ScalarType,
    operand: mir::Operand,
) -> mir::ValueExpression {
    match ty {
        mir::ScalarType::Integer(ty) => {
            mir::ValueExpression::Integer(mir::IntegerExpression::Use { ty, operand })
        }
        mir::ScalarType::Float(ty) => {
            mir::ValueExpression::Float(mir::FloatExpression::Use { ty, operand })
        }
        mir::ScalarType::Bool => mir::ValueExpression::Bool(mir::BoolExpression::Use { operand }),
    }
}

fn lower_rvalue_as_expected(
    expr: &hir::Expr,
    expected: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::Rvalue> {
    if let Some((collection, index, value_type)) = lower_list_remove_at(expr, context)? {
        if value_type != expected {
            return Err(vec![unsupported(
                expr.span(),
                "List::removeAt result has another type",
            )]);
        }
        return collection_remove_at_rvalue(collection, index, expected);
    }
    match expected {
        mir::Type::String => lower_string_expression(expr, context).map(mir::Rvalue::String),
        mir::Type::NullableScalar(ty) => {
            lower_nullable_scalar_expression(expr, ty, context).map(mir::Rvalue::NullableScalar)
        }
        mir::Type::NullableString => {
            lower_nullable_string_expression(expr, context).map(mir::Rvalue::NullableString)
        }
        mir::Type::Mixed => lower_mixed_expression(expr, true, context).map(mir::Rvalue::Mixed),
        mir::Type::NullableMixed => {
            lower_nullable_mixed_expression(expr, true, context).map(mir::Rvalue::NullableMixed)
        }
        mir::Type::Scalar(_) => lower_value_expression(expr, context).map(mir::Rvalue::Value),
        mir::Type::Class(class) => {
            lower_class_expression(expr, class, true, context).map(mir::Rvalue::Class)
        }
        mir::Type::NullableClass(class) => {
            lower_nullable_class_expression(expr, class, true, context)
                .map(mir::Rvalue::NullableClass)
        }
        mir::Type::Collection(collection) => {
            lower_collection_expression(expr, collection, true, context)
                .map(mir::Rvalue::Collection)
        }
    }
}

fn lower_rvalue_as_borrowed(
    expr: &hir::Expr,
    expected: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::Rvalue> {
    match expected {
        mir::Type::Class(class) => {
            lower_class_expression(expr, class, false, context).map(mir::Rvalue::Class)
        }
        mir::Type::NullableClass(class) => {
            lower_nullable_class_expression(expr, class, false, context)
                .map(mir::Rvalue::NullableClass)
        }
        mir::Type::Collection(collection) => {
            lower_collection_expression(expr, collection, false, context)
                .map(mir::Rvalue::Collection)
        }
        mir::Type::Mixed => lower_mixed_expression(expr, false, context).map(mir::Rvalue::Mixed),
        mir::Type::NullableMixed => {
            lower_nullable_mixed_expression(expr, false, context).map(mir::Rvalue::NullableMixed)
        }
        _ => lower_rvalue_as_expected(expr, expected, context),
    }
}

fn lower_mixed_expression(
    expr: &hir::Expr,
    transfer: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::MixedExpression> {
    // `List<mixed>::removeAt(i)` is a collection removal, not a class method call, so it
    // must be intercepted before the method-call arm below (which requires a concrete
    // class receiver). It removes the element and hands back the owned box, mirroring the
    // interception in `lower_rvalue_as_expected`.
    if let Some((collection, index, value_type)) = lower_list_remove_at(expr, context)? {
        if value_type != mir::Type::Mixed {
            return Err(vec![unsupported(
                expr.span(),
                "List::removeAt result has another type",
            )]);
        }
        return Ok(mir::MixedExpression::CollectionIndex {
            collection,
            index: Box::new(index),
            transfer: true,
            remove: true,
        });
    }
    if let hir::Expr::Variable { name, span } = unparenthesized_place(expr) {
        let local = context.lookup_local(name, *span)?;
        if context.local_type(local) == mir::Type::Mixed {
            return Ok(mir::MixedExpression::Local { local, transfer });
        }
        if context.local_type(local) == mir::Type::NullableMixed
            && !transfer
            && context.expression_type(expr)? == mir::Type::Mixed
        {
            return Ok(mir::MixedExpression::Local {
                local,
                transfer: false,
            });
        }
    }
    if matches!(
        unparenthesized_place(expr),
        hir::Expr::PropertyAccess { .. }
    ) {
        let (object, property, ty) = lower_property_place(expr, context)?;
        if ty == mir::Type::Mixed {
            return Ok(mir::MixedExpression::Property { object, property });
        }
    }
    if let hir::Expr::Index {
        collection, index, ..
    } = unparenthesized_place(expr)
    {
        let (collection, index) =
            lower_collection_index_operand(collection, index, mir::Type::Mixed, context)?;
        return Ok(mir::MixedExpression::CollectionIndex {
            collection,
            index: Box::new(index),
            transfer,
            remove: false,
        });
    }
    if let hir::Expr::FunctionCall { name, args, span } = expr {
        let signature = context.lookup_function(name, *span)?;
        if signature.return_type == mir::ReturnType::Value(mir::Type::Mixed) {
            return Ok(mir::MixedExpression::Call {
                function: signature.id,
                return_borrow: signature.return_borrow,
                args: lower_call_args(name, args, signature, *span, context)?,
            });
        }
    }
    if let hir::Expr::MethodCall {
        object,
        method,
        args,
        span,
        ..
    } = expr
    {
        let (signature, args) = lower_instance_method_call(object, method, args, *span, context)?;
        if signature.return_type == mir::ReturnType::Value(mir::Type::Mixed) {
            return Ok(mir::MixedExpression::Call {
                function: signature.id,
                args,
                return_borrow: signature.return_borrow,
            });
        }
    }
    if let hir::Expr::StaticCall {
        class_name,
        method,
        args,
        span,
    } = expr
    {
        let (signature, args) = lower_static_method_call(class_name, method, args, *span, context)?;
        if signature.return_type == mir::ReturnType::Value(mir::Type::Mixed) {
            return Ok(mir::MixedExpression::Call {
                function: signature.id,
                args,
                return_borrow: signature.return_borrow,
            });
        }
    }

    match context.expression_type(expr)? {
        mir::Type::Scalar(_) => Ok(mir::MixedExpression::BoxValue(lower_value_expression(
            expr, context,
        )?)),
        mir::Type::String => {
            let value = lower_string_expression(expr, context)?;
            let payload_owned = transfer || !value.is_borrowed_place();
            Ok(mir::MixedExpression::BoxString {
                value,
                payload_owned,
            })
        }
        mir::Type::Class(class) => {
            let value = lower_class_expression(expr, class, transfer, context)?;
            let payload_owned = transfer || value.owned_temporary_class().is_some();
            Ok(mir::MixedExpression::BoxClass {
                value,
                payload_owned,
            })
        }
        mir::Type::Mixed => Err(vec![unsupported(
            expr.span(),
            "mixed expression could not be lowered as a mixed value",
        )]),
        mir::Type::Collection(_) => Err(vec![Diagnostic::unsupported_stage(
            "M1101",
            "boxing collections, typed arrays, or Bytes into `mixed` lands after Stage 23 Slice 3",
            expr.span(),
        )]),
        mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::NullableClass(_)
        | mir::Type::NullableMixed => Err(vec![Diagnostic::unsupported_stage(
            "M1101",
            "boxing nullable values into `mixed` lands after Stage 23 Slice 3",
            expr.span(),
        )]),
    }
}

fn mixed_tag_for_type(ty: mir::Type, span: Span) -> DiagnosticResult<mir::MixedTag> {
    match ty {
        mir::Type::Scalar(mir::ScalarType::Bool) => Ok(mir::MixedTag::Bool),
        mir::Type::Scalar(mir::ScalarType::Integer(ty)) => Ok(mir::MixedTag::Integer(ty)),
        mir::Type::Scalar(mir::ScalarType::Float(ty)) => Ok(mir::MixedTag::Float(ty)),
        mir::Type::String => Ok(mir::MixedTag::String),
        mir::Type::Class(class) => Ok(mir::MixedTag::Class(class)),
        mir::Type::Mixed
        | mir::Type::NullableMixed
        | mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::NullableClass(_)
        | mir::Type::Collection(_) => Err(vec![Diagnostic::unsupported_stage(
            "M1101",
            "only exact bool, integer, float, string, and concrete-class `is` tests unbox `mixed` in Stage 23 Slice 3",
            span,
        )]),
    }
}

fn lower_nullable_mixed_expression(
    expr: &hir::Expr,
    transfer: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::NullableMixedExpression> {
    if context.expression_is_null(expr) {
        return Ok(mir::NullableMixedExpression::Null);
    }
    match expr {
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_nullable_mixed_expression(left, transfer, context),
            CoalesceSelection::Right => lower_nullable_mixed_expression(right, transfer, context),
            CoalesceSelection::Dynamic => Ok(mir::NullableMixedExpression::Coalesce {
                left: Box::new(lower_nullable_mixed_expression(left, transfer, context)?),
                right: Box::new(lower_nullable_mixed_expression(right, transfer, context)?),
                transfer,
            }),
        },
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::NullableMixed => {
                    Ok(mir::NullableMixedExpression::Local { local, transfer })
                }
                mir::Type::Mixed => Ok(mir::NullableMixedExpression::Mixed(
                    mir::MixedExpression::Local { local, transfer },
                )),
                _ => Ok(mir::NullableMixedExpression::Mixed(lower_mixed_expression(
                    expr, transfer, context,
                )?)),
            }
        }
        hir::Expr::PropertyAccess { .. } => {
            let (object, property, ty) = lower_property_place(expr, context)?;
            match ty {
                mir::Type::NullableMixed => {
                    Ok(mir::NullableMixedExpression::Property { object, property })
                }
                mir::Type::Mixed => Ok(mir::NullableMixedExpression::Mixed(
                    mir::MixedExpression::Property { object, property },
                )),
                _ => Ok(mir::NullableMixedExpression::Mixed(lower_mixed_expression(
                    expr, transfer, context,
                )?)),
            }
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableMixed) => {
                    Ok(mir::NullableMixedExpression::Call {
                        function: signature.id,
                        return_borrow: signature.return_borrow,
                        args: lower_call_args(name, args, signature, *span, context)?,
                    })
                }
                mir::ReturnType::Value(mir::Type::Mixed) => Ok(
                    mir::NullableMixedExpression::Mixed(mir::MixedExpression::Call {
                        function: signature.id,
                        return_borrow: signature.return_borrow,
                        args: lower_call_args(name, args, signature, *span, context)?,
                    }),
                ),
                _ => Ok(mir::NullableMixedExpression::Mixed(lower_mixed_expression(
                    expr, transfer, context,
                )?)),
            }
        }
        _ => Ok(mir::NullableMixedExpression::Mixed(lower_mixed_expression(
            expr, transfer, context,
        )?)),
    }
}

fn lower_collection_expression(
    expr: &hir::Expr,
    expected: mir::CollectionTypeId,
    transfer: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::CollectionExpression> {
    if let Some((collection, index, value_type)) = lower_list_remove_at(expr, context)? {
        if value_type != mir::Type::Collection(expected) {
            return Err(vec![unsupported(
                expr.span(),
                "List::removeAt result has another collection type",
            )]);
        }
        return Ok(mir::CollectionExpression::Index {
            collection: expected,
            source: collection,
            index: Box::new(index),
            transfer: true,
        });
    }
    match expr {
        hir::Expr::Grouped { expr, .. } => {
            lower_collection_expression(expr, expected, transfer, context)
        }
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            if context.local_type(local) != mir::Type::Collection(expected) {
                return Err(vec![unsupported(
                    *span,
                    format!("collection `${name}` does not have the expected collection type"),
                )]);
            }
            if transfer && !context.local_owns(local) {
                return Err(vec![unsupported(
                    *span,
                    format!("borrowed collection `${name}` cannot be given away"),
                )]);
            }
            Ok(mir::CollectionExpression::Local {
                collection: expected,
                local,
                transfer,
            })
        }
        hir::Expr::PropertyAccess { span, .. } => {
            if transfer {
                return Err(vec![unsupported(
                    *span,
                    "collection properties are borrowed and cannot be given away directly",
                )]);
            }
            let (object, property, property_type) = lower_property_place(expr, context)?;
            if property_type != mir::Type::Collection(expected) {
                return Err(vec![unsupported(
                    *span,
                    "collection property does not have the expected collection type",
                )]);
            }
            Ok(mir::CollectionExpression::Property {
                collection: expected,
                object,
                property,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
        } if method == "toArray" && args.is_empty() => {
            let (source, _) = lower_bytes_local(object, context)?;
            Ok(mir::CollectionExpression::FromBytes {
                collection: expected,
                source,
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if class_name == "Bytes" && method == "fromArray" => {
            let [source] = argument_values(args)[..] else {
                return Err(vec![unsupported(
                    *span,
                    "Bytes::fromArray expects 1 argument",
                )]);
            };
            let (source, _) = lower_collection_local(source, context)?;
            Ok(mir::CollectionExpression::BytesFromArray {
                collection: expected,
                source,
            })
        }
        hir::Expr::FunctionCall { name, args, span }
            if name == "read_file_bytes" && args.len() == 1 =>
        {
            Ok(mir::CollectionExpression::ReadFileBytes {
                collection: expected,
                path: Box::new(lower_string_expression(&args[0].value, context)?),
            })
        }
        hir::Expr::FunctionCall { name, args, .. }
            if name == "read_stdin_bytes" && args.is_empty() =>
        {
            Ok(mir::CollectionExpression::ReadStdinBytes {
                collection: expected,
            })
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::Collection(expected)) {
                return Err(vec![unsupported(
                    *span,
                    format!("function `{name}` does not return the expected collection type"),
                )]);
            }
            Ok(mir::CollectionExpression::Call {
                collection: expected,
                function: signature.id,
                return_borrow: signature.return_borrow,
                args: lower_call_args_with_ownership(name, args, signature, *span, context)?,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
        } if !matches!(method.as_str(), "union" | "intersect" | "difference") => {
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::Collection(expected)) {
                return Err(vec![unsupported(
                    *span,
                    "method does not return the expected collection type",
                )]);
            }
            Ok(mir::CollectionExpression::Call {
                collection: expected,
                function: signature.id,
                args,
                return_borrow: signature.return_borrow,
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if !(class_name == "Set" && method == "from") => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::Collection(expected)) {
                return Err(vec![unsupported(
                    *span,
                    "static method does not return the expected collection type",
                )]);
            }
            Ok(mir::CollectionExpression::Call {
                collection: expected,
                function: signature.id,
                args,
                return_borrow: signature.return_borrow,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
        } if matches!(method.as_str(), "union" | "intersect" | "difference") => {
            let (left, left_type) = lower_collection_local(object, context)?;
            if left_type != expected
                || context.collection_type(expected).kind != mir::CollectionKind::Set
            {
                return Err(vec![unsupported(
                    *span,
                    "set algebra requires matching Set<T> operands",
                )]);
            }
            let [right] = argument_values(args)[..] else {
                return Err(vec![unsupported(
                    *span,
                    "set algebra expects one Set<T> argument",
                )]);
            };
            let (right, right_type) = lower_collection_local(right, context)?;
            if right_type != expected {
                return Err(vec![unsupported(
                    *span,
                    "set algebra requires matching Set<T> operands",
                )]);
            }
            let op = match method.as_str() {
                "union" => mir::SetAlgebraOp::Union,
                "intersect" => mir::SetAlgebraOp::Intersect,
                "difference" => mir::SetAlgebraOp::Difference,
                _ => unreachable!(),
            };
            Ok(mir::CollectionExpression::SetFrom {
                collection: expected,
                source: left,
                transfer: false,
                algebra: Some((op, right)),
            })
        }
        hir::Expr::Array { elements, .. } => {
            let collection = context.collection_type(expected).clone();
            let entries = elements
                .iter()
                .map(|element| {
                    let key = match (&collection.key, &element.key) {
                        (Some(key_type), Some(key)) => {
                            Some(lower_rvalue_as_expected(key, *key_type, context)?)
                        }
                        (None, None) => None,
                        (Some(_), None) => {
                            return Err(vec![unsupported(
                                element.value.span(),
                                "dictionary literals require a key for every entry",
                            )])
                        }
                        (None, Some(key)) => {
                            return Err(vec![unsupported(
                                key.span(),
                                "sequence collection literals cannot contain keyed entries",
                            )])
                        }
                    };
                    Ok(mir::CollectionEntry {
                        key,
                        value: lower_rvalue_as_expected(&element.value, collection.value, context)?,
                    })
                })
                .collect::<DiagnosticResult<Vec<_>>>()?;
            Ok(mir::CollectionExpression::Literal {
                collection: expected,
                entries,
            })
        }
        hir::Expr::ArrayRepeat { value, count, .. } => {
            let collection = context.collection_type(expected).clone();
            Ok(mir::CollectionExpression::Fill {
                collection: expected,
                value: Box::new(lower_rvalue_as_expected(value, collection.value, context)?),
                count: Box::new(lower_integer_expression(count, context)?),
            })
        }
        hir::Expr::Index {
            collection,
            index,
            span,
        } => {
            if transfer {
                return Err(vec![unsupported(
                    *span,
                    "indexed collection values are borrowed and cannot be moved out",
                )]);
            }
            let (source, source_type) = lower_collection_local(collection, context)?;
            let source_info = context.collection_type(source_type);
            if source_info.value != mir::Type::Collection(expected) {
                return Err(vec![unsupported(
                    *span,
                    "indexed collection element has another collection type",
                )]);
            }
            let index_type =
                source_info
                    .key
                    .unwrap_or(mir::Type::Scalar(mir::ScalarType::Integer(
                        IntegerType::Int64,
                    )));
            Ok(mir::CollectionExpression::Index {
                collection: expected,
                source,
                index: Box::new(lower_rvalue_as_expected(index, index_type, context)?),
                transfer: false,
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if class_name == "Set" && method == "from" => {
            let [source] = argument_values(args)[..] else {
                return Err(vec![unsupported(*span, "Set::from expects one argument")]);
            };
            if let hir::Expr::Array { elements, .. } = source {
                let collection = context.collection_type(expected).clone();
                let entries = elements
                    .iter()
                    .map(|element| {
                        if element.key.is_some() {
                            return Err(vec![unsupported(
                                element.value.span(),
                                "Set::from accepts a sequence collection",
                            )]);
                        }
                        Ok(mir::CollectionEntry {
                            key: None,
                            value: lower_rvalue_as_expected(
                                &element.value,
                                collection.value,
                                context,
                            )?,
                        })
                    })
                    .collect::<DiagnosticResult<Vec<_>>>()?;
                return Ok(mir::CollectionExpression::Literal {
                    collection: expected,
                    entries,
                });
            }
            let (source, _) = lower_collection_local(source, context)?;
            Ok(mir::CollectionExpression::SetFrom {
                collection: expected,
                source,
                transfer: false,
                algebra: None,
            })
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "this collection expression is not supported by Stage 23 Slice 1",
        )]),
    }
}

fn lower_list_remove_at(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<(mir::LocalId, mir::Rvalue, mir::Type)>> {
    let hir::Expr::MethodCall {
        object,
        method,
        args,
        null_safe: false,
        ..
    } = unparenthesized_place(expr)
    else {
        return Ok(None);
    };
    if method != "removeAt" {
        return Ok(None);
    }
    let (collection, collection_type) = lower_collection_local(object, context)?;
    let definition = context.collection_type(collection_type).clone();
    if definition.kind != mir::CollectionKind::List {
        return Ok(None);
    }
    let [index] = argument_values(args)[..] else {
        return Err(vec![unsupported(
            expr.span(),
            "List::removeAt expects one index",
        )]);
    };
    let index = lower_rvalue_as_expected(
        index,
        mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
        context,
    )?;
    Ok(Some((collection, index, definition.value)))
}

fn collection_remove_at_rvalue(
    collection: mir::LocalId,
    index: mir::Rvalue,
    ty: mir::Type,
) -> DiagnosticResult<mir::Rvalue> {
    match ty {
        mir::Type::Scalar(scalar) => Ok(mir::Rvalue::Value(value_expression_from_operand(
            scalar,
            mir::Operand::CollectionIndex {
                collection,
                index: Box::new(index),
                remove: true,
            },
        ))),
        mir::Type::String => Ok(mir::Rvalue::String(
            mir::StringExpression::CollectionIndex {
                collection,
                index: Box::new(index),
                remove: true,
            },
        )),
        mir::Type::Class(class) => Ok(mir::Rvalue::Class(mir::ClassExpression::CollectionIndex {
            class,
            collection,
            index: Box::new(index),
            transfer: true,
        })),
        mir::Type::Mixed => Ok(mir::Rvalue::Mixed(mir::MixedExpression::CollectionIndex {
            collection,
            index: Box::new(index),
            transfer: true,
            remove: true,
        })),
        mir::Type::Collection(nested) => {
            Ok(mir::Rvalue::Collection(mir::CollectionExpression::Index {
                collection: nested,
                source: collection,
                index: Box::new(index),
                transfer: true,
            }))
        }
        mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::NullableMixed
        | mir::Type::NullableClass(_) => Err(vec![unsupported(
            Span::new(0, 0),
            "removing nullable collection elements is deferred beyond Stage 23 Slice 3",
        )]),
    }
}

fn materialize_nested_collection_places(
    expr: &hir::Expr,
    writable: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    match expr {
        hir::Expr::Array { elements, .. } => {
            for element in elements {
                if let Some(key) = &element.key {
                    materialize_nested_collection_places(key, false, context)?;
                }
                materialize_nested_collection_places(&element.value, false, context)?;
            }
        }
        hir::Expr::ArrayRepeat { value, count, .. } => {
            materialize_nested_collection_places(value, false, context)?;
            materialize_nested_collection_places(count, false, context)?;
        }
        hir::Expr::Index {
            collection, index, ..
        } => {
            materialize_nested_collection_places(collection, writable, context)?;
            materialize_nested_collection_places(index, false, context)?;

            let place = unparenthesized_place(collection);
            if collection_place_is_borrowed(place) {
                let key = (place.span().start, place.span().end);
                if !context.materialized_collection_places.contains_key(&key) {
                    let mir::Type::Collection(collection_type) = context.expression_type(place)?
                    else {
                        return Err(vec![unsupported(
                            place.span(),
                            "nested indexed place does not contain a collection",
                        )]);
                    };
                    let value =
                        lower_collection_expression(place, collection_type, false, context)?;
                    let local = context
                        .declare_borrowed_temp(mir::Type::Collection(collection_type), writable);
                    context.push_statement(mir::Statement::AssignLocal {
                        target: local,
                        value: mir::Rvalue::Collection(value),
                    });
                    context.materialized_collection_places.insert(key, local);
                }
            }
        }
        hir::Expr::InterpolatedString { parts, .. } => {
            for part in parts {
                if let hir::InterpolatedStringPart::Expr(expr) = part {
                    materialize_nested_collection_places(expr, false, context)?;
                }
            }
        }
        hir::Expr::PropertyAccess { object, .. } => {
            materialize_nested_collection_places(object, false, context)?;
            if matches!(
                context.expression_type(object),
                Ok(mir::Type::Collection(_))
            ) {
                materialize_collection_place(object, false, context)?;
            }
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            ..
        } => {
            let receiver_writable = method_receiver_is_writable(object, method, context);
            materialize_nested_collection_places(object, receiver_writable, context)?;
            for arg in args {
                materialize_nested_collection_places(&arg.value, false, context)?;
            }
            if matches!(
                context.expression_type(object),
                Ok(mir::Type::Collection(_))
            ) {
                materialize_collection_place(object, receiver_writable, context)?;
            }
        }
        hir::Expr::IsType { expr, .. }
        | hir::Expr::Grouped { expr, .. }
        | hir::Expr::Unary { expr, .. } => {
            materialize_nested_collection_places(expr, writable, context)?;
        }
        hir::Expr::FunctionCall { name, args, .. } => {
            for arg in args {
                materialize_nested_collection_places(&arg.value, false, context)?;
            }
            let byte_argument = match name.as_str() {
                "write_file_bytes" | "append_file_bytes" => args.get(1),
                "write_stdout_bytes" | "write_stderr_bytes" => args.first(),
                _ => None,
            };
            if let Some(bytes) = byte_argument {
                materialize_collection_place(&bytes.value, false, context)?;
            }
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            ..
        } => {
            for arg in args {
                materialize_nested_collection_places(&arg.value, false, context)?;
            }
            if class_name == "Set" && method == "from" {
                if let Some(source) = args.first() {
                    if !matches!(
                        unparenthesized_place(&source.value),
                        hir::Expr::Array { .. }
                    ) {
                        materialize_collection_place(&source.value, false, context)?;
                    }
                }
            }
            if class_name == "Bytes" && method == "fromArray" {
                if let Some(source) = args.first() {
                    let byte_array = context
                        .collection_registry
                        .ids
                        .get(&(
                            mir::CollectionKind::TypedArray,
                            None,
                            mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8)),
                        ))
                        .copied()
                        .expect("Bytes requires the canonical uint8[] MIR type");
                    materialize_collection_place_as(&source.value, byte_array, false, context)?;
                }
            }
        }
        hir::Expr::New { args, .. } => {
            for arg in args {
                materialize_nested_collection_places(&arg.value, false, context)?;
            }
        }
        hir::Expr::Binary {
            left, op, right, ..
        } => {
            materialize_nested_collection_places(left, false, context)?;
            materialize_nested_collection_places(right, false, context)?;
            if matches!(op, hir::BinaryOp::Equal | hir::BinaryOp::NotEqual)
                && expression_is_bytes(left, context)
                && expression_is_bytes(right, context)
            {
                materialize_collection_place(left, false, context)?;
                materialize_collection_place(right, false, context)?;
            }
        }
        hir::Expr::Range { start, end, .. } => {
            materialize_nested_collection_places(start, false, context)?;
            materialize_nested_collection_places(end, false, context)?;
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
    Ok(())
}

fn lower_collection_local(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<(mir::LocalId, mir::CollectionTypeId)> {
    let expr = unparenthesized_place(expr);
    if let Some(local) = context
        .materialized_collection_places
        .get(&(expr.span().start, expr.span().end))
        .copied()
    {
        let mir::Type::Collection(collection) = context.local_type(local) else {
            unreachable!("materialized collection place must have collection type");
        };
        return Ok((local, collection));
    }
    match expr {
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            let mir::Type::Collection(collection) = context.local_type(local) else {
                return Err(vec![unsupported(
                    *span,
                    "indexed value is not a collection",
                )]);
            };
            Ok((local, collection))
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "collection access requires a materialized collection place",
        )]),
    }
}

fn lower_bytes_local(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<(mir::LocalId, mir::CollectionTypeId)> {
    let (local, collection) = lower_collection_local(expr, context)?;
    if context.collection_type(collection).kind != mir::CollectionKind::Bytes {
        return Err(vec![unsupported(expr.span(), "value is not `Bytes`")]);
    }
    Ok((local, collection))
}

fn materialize_collection_place(
    expr: &hir::Expr,
    writable: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    if lower_collection_local(expr, context).is_ok() {
        return Ok(());
    }
    let mir::Type::Collection(collection) = context.expression_type(expr)? else {
        return Ok(());
    };
    materialize_collection_place_as(expr, collection, writable, context)
}

fn materialize_collection_place_as(
    expr: &hir::Expr,
    collection: mir::CollectionTypeId,
    writable: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    if lower_collection_local(expr, context).is_ok() {
        return Ok(());
    }
    let borrowed = collection_place_is_borrowed(expr);
    let value = lower_collection_expression(expr, collection, !borrowed, context)?;
    let local = if borrowed {
        context.declare_borrowed_temp(mir::Type::Collection(collection), writable)
    } else {
        context.declare_owned_temp(mir::Type::Collection(collection))
    };
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value: mir::Rvalue::Collection(value),
    });
    context
        .materialized_collection_places
        .insert((expr.span().start, expr.span().end), local);
    Ok(())
}

fn collection_method_mutates(method: &str) -> bool {
    matches!(
        method,
        "add" | "insertAt" | "removeAt" | "pop" | "set" | "remove"
    )
}

fn method_receiver_is_writable(
    object: &hir::Expr,
    method: &str,
    context: &mut LoweringContext,
) -> bool {
    match context.expression_type(object) {
        Ok(mir::Type::Collection(_)) => collection_method_mutates(method),
        Ok(mir::Type::Class(class) | mir::Type::NullableClass(class)) => context
            .lookup_method(class, method, object.span())
            .is_ok_and(|signature| signature.receiver_mode == Some(mir::ReceiverMode::Writable)),
        _ => false,
    }
}

fn collection_place_is_borrowed(expr: &hir::Expr) -> bool {
    matches!(
        unparenthesized_place(expr),
        hir::Expr::Index { .. } | hir::Expr::PropertyAccess { .. }
    )
}

fn expression_is_bytes(expr: &hir::Expr, context: &mut LoweringContext) -> bool {
    matches!(
        context.expression_type(expr),
        Ok(mir::Type::Collection(collection))
            if context.collection_type(collection).kind == mir::CollectionKind::Bytes
    )
}

fn lower_collection_index_operand(
    collection_expr: &hir::Expr,
    index: &hir::Expr,
    expected: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<(mir::LocalId, mir::Rvalue)> {
    let (collection, collection_type) = lower_collection_local(collection_expr, context)?;
    let info = context.collection_type(collection_type).clone();
    if info.value != expected {
        return Err(vec![unsupported(
            collection_expr.span(),
            format!(
                "collection element has MIR type `{}`, expected `{expected}`",
                info.value
            ),
        )]);
    }
    let index_type = info
        .key
        .unwrap_or(mir::Type::Scalar(mir::ScalarType::Integer(
            IntegerType::Int64,
        )));
    Ok((
        collection,
        lower_rvalue_as_expected(index, index_type, context)?,
    ))
}

fn lower_collection_method_statement(
    object: &hir::Expr,
    method: &str,
    args: &[hir::Argument],
    context: &mut LoweringContext,
) -> DiagnosticResult<bool> {
    let Ok((collection, collection_type)) = lower_collection_local(object, context) else {
        return Ok(false);
    };
    let info = context.collection_type(collection_type).clone();
    match (info.kind, method, args) {
        (mir::CollectionKind::List | mir::CollectionKind::Set, "add", [value]) => {
            let statement = mir::Statement::CollectionAdd {
                collection,
                value: lower_rvalue_as_expected(&value.value, info.value, context)?,
                index: None,
                op: mir::CollectionMutationOp::Add,
            };
            context.push_statement(statement);
        }
        (mir::CollectionKind::List, "insertAt", [index, value]) => {
            let statement = mir::Statement::CollectionAdd {
                collection,
                value: lower_rvalue_as_expected(&value.value, info.value, context)?,
                index: Some(lower_rvalue_as_expected(
                    &index.value,
                    mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
                    context,
                )?),
                op: mir::CollectionMutationOp::InsertAt,
            };
            context.push_statement(statement);
        }
        (mir::CollectionKind::Set, "remove", [value]) => {
            let statement = mir::Statement::CollectionAdd {
                collection,
                value: lower_rvalue_as_borrowed(&value.value, info.value, context)?,
                index: None,
                op: mir::CollectionMutationOp::Remove,
            };
            context.push_statement(statement);
        }
        (mir::CollectionKind::Dictionary, "set", [key, value]) => {
            let key_type = info.key.expect("dictionary collection has a key type");
            let statement = mir::Statement::CollectionSet {
                collection,
                key: lower_rvalue_as_expected(&key.value, key_type, context)?,
                value: lower_rvalue_as_expected(&value.value, info.value, context)?,
            };
            context.push_statement(statement);
        }
        (mir::CollectionKind::List, "removeAt", [index]) => {
            let index = lower_rvalue_as_expected(
                &index.value,
                mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
                context,
            )?;
            let value = collection_remove_at_rvalue(collection, index, info.value)?;
            lower_discarded_rvalue(value, context);
        }
        (mir::CollectionKind::List, "pop", [])
        | (mir::CollectionKind::Dictionary, "remove", [_]) => {
            let Some((collection, key, value_type, access)) =
                lower_dictionary_get(object, method, args, context)?
            else {
                return Ok(false);
            };
            let value =
                nullable_collection_access_rvalue(collection, key, value_type, access, object)?;
            lower_discarded_rvalue(value, context);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn nullable_collection_access_rvalue(
    collection: mir::LocalId,
    key: mir::Rvalue,
    value_type: mir::Type,
    access: mir::NullableCollectionAccess,
    object: &hir::Expr,
) -> DiagnosticResult<mir::Rvalue> {
    let key = Box::new(key);
    match value_type {
        mir::Type::Scalar(ty) => Ok(mir::Rvalue::NullableScalar(
            mir::NullableScalarExpression::DictionaryGet {
                ty,
                collection,
                key,
                access,
            },
        )),
        mir::Type::String => Ok(mir::Rvalue::NullableString(
            mir::NullableStringExpression::DictionaryGet {
                collection,
                key,
                access,
            },
        )),
        mir::Type::Class(class) => Ok(mir::Rvalue::NullableClass(
            mir::NullableClassExpression::DictionaryGet {
                class,
                collection,
                key,
                access,
            },
        )),
        mir::Type::Mixed => Err(vec![Diagnostic::unsupported_stage(
            "M1101",
            "nullable accessors returning `?mixed` from collections land with the next mixed collection slice",
            object.span(),
        )]),
        mir::Type::Collection(_)
        | mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::NullableMixed
        | mir::Type::NullableClass(_) => Err(vec![unsupported(
            object.span(),
            "discarding this nullable collection element type is not yet supported",
        )]),
    }
}

fn lower_discarded_rvalue(value: mir::Rvalue, context: &mut LoweringContext) {
    let ty = value.ty();
    let owned = matches!(
        ty,
        mir::Type::Class(_)
            | mir::Type::NullableClass(_)
            | mir::Type::Mixed
            | mir::Type::NullableMixed
            | mir::Type::Collection(_)
    );
    let local = context.declare_return_temp(ty, owned);
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value,
    });
    match ty {
        mir::Type::Class(class) | mir::Type::NullableClass(class) => {
            context.push_statement(mir::Statement::DropClass { local, class });
        }
        mir::Type::Collection(collection) => {
            context.push_statement(mir::Statement::DropCollection { local, collection });
        }
        mir::Type::Mixed | mir::Type::NullableMixed => {
            context.push_statement(mir::Statement::DropMixed { local });
        }
        mir::Type::Scalar(_)
        | mir::Type::String
        | mir::Type::NullableScalar(_)
        | mir::Type::NullableString => {}
    }
}

fn lower_dictionary_get(
    object: &hir::Expr,
    method: &str,
    args: &[hir::Argument],
    context: &mut LoweringContext,
) -> DiagnosticResult<
    Option<(
        mir::LocalId,
        mir::Rvalue,
        mir::Type,
        mir::NullableCollectionAccess,
    )>,
> {
    let Ok((collection, collection_type)) = lower_collection_local(object, context) else {
        return Ok(None);
    };
    let definition = context.collection_type(collection_type).clone();
    let (key, access) = match (definition.kind, method, args) {
        (mir::CollectionKind::Dictionary, "get", [key]) => (
            lower_rvalue_as_borrowed(
                &key.value,
                definition
                    .key
                    .expect("dictionary collection has a key type"),
                context,
            )?,
            mir::NullableCollectionAccess::Get,
        ),
        (mir::CollectionKind::Dictionary, "remove", [key]) => (
            lower_rvalue_as_borrowed(
                &key.value,
                definition
                    .key
                    .expect("dictionary collection has a key type"),
                context,
            )?,
            mir::NullableCollectionAccess::Remove,
        ),
        (mir::CollectionKind::List, "pop", []) => (
            mir::Rvalue::Value(mir::ValueExpression::Integer(
                mir::IntegerExpression::constant(
                    IntegerValue::from_i128(IntegerType::Int64, 0).expect("zero is a valid int"),
                ),
            )),
            mir::NullableCollectionAccess::Pop,
        ),
        _ => return Ok(None),
    };
    Ok(Some((collection, key, definition.value, access)))
}

fn lower_collection_nullable_property(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<
    Option<(
        mir::LocalId,
        mir::Rvalue,
        mir::Type,
        mir::NullableCollectionAccess,
    )>,
> {
    let hir::Expr::PropertyAccess {
        object,
        property,
        null_safe: false,
        ..
    } = unparenthesized_place(expr)
    else {
        return Ok(None);
    };
    let access = match property.as_str() {
        "first" => mir::NullableCollectionAccess::First,
        "last" => mir::NullableCollectionAccess::Last,
        _ => return Ok(None),
    };
    let (collection, collection_type) = lower_collection_local(object, context)?;
    let definition = context.collection_type(collection_type).clone();
    if definition.kind != mir::CollectionKind::List {
        return Ok(None);
    }
    let key = mir::Rvalue::Value(mir::ValueExpression::Integer(
        mir::IntegerExpression::constant(
            IntegerValue::from_i128(IntegerType::Int64, 0).expect("zero is a valid int"),
        ),
    ));
    Ok(Some((collection, key, definition.value, access)))
}

fn lower_class_expression(
    expr: &hir::Expr,
    expected: ClassId,
    transfer: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::ClassExpression> {
    if let Some((collection, index, value_type)) = lower_list_remove_at(expr, context)? {
        if value_type != mir::Type::Class(expected) {
            return Err(vec![unsupported(
                expr.span(),
                "List::removeAt result has another class type",
            )]);
        }
        return Ok(mir::ClassExpression::CollectionIndex {
            class: expected,
            collection,
            index: Box::new(index),
            transfer: true,
        });
    }
    match expr {
        hir::Expr::Grouped { expr, .. } => {
            lower_class_expression(expr, expected, transfer, context)
        }
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::Class(class) if class == expected => {
                    if transfer && !context.local_owns(local) {
                        return Err(vec![unsupported(
                            *span,
                            format!("borrowed class local `${name}` cannot be given away"),
                        )]);
                    }
                    Ok(mir::ClassExpression::Local {
                        class: expected,
                        local,
                        transfer,
                    })
                }
                mir::Type::NullableClass(class) if class == expected => {
                    if transfer && !context.local_owns(local) {
                        return Err(vec![unsupported(
                            *span,
                            format!("borrowed nullable class local `${name}` cannot be given away"),
                        )]);
                    }
                    Ok(mir::ClassExpression::NullableLocalAssumeNonNull {
                        class: expected,
                        local,
                        transfer,
                    })
                }
                mir::Type::Mixed | mir::Type::NullableMixed
                    if context
                        .exact_mixed_local(expr)
                        .is_some_and(|(_, narrowed)| narrowed == mir::Type::Class(expected)) =>
                {
                    Ok(mir::ClassExpression::MixedPayload {
                        class: expected,
                        mixed: local,
                        transfer,
                    })
                }
                _ => Err(vec![unsupported(
                    *span,
                    format!("local `${name}` does not have the expected class type"),
                )]),
            }
        }
        hir::Expr::This { span } => {
            if transfer {
                return Err(vec![unsupported(*span, "`$this` cannot be given away")]);
            }
            let local = context.lookup_local("this", *span)?;
            if context.local_type(local) != mir::Type::Class(expected) {
                return Err(vec![unsupported(
                    *span,
                    "`$this` does not have the expected class type",
                )]);
            }
            Ok(mir::ClassExpression::Local {
                class: expected,
                local,
                transfer: false,
            })
        }
        hir::Expr::PropertyAccess { span, .. } => {
            if transfer {
                return Err(vec![unsupported(
                    *span,
                    "moving directly out of an owned class property is not supported",
                )]);
            }
            let (object, property, property_type) = lower_property_place(expr, context)?;
            if property_type != mir::Type::Class(expected) {
                return Err(vec![unsupported(
                    *span,
                    "class property does not have the expected class type",
                )]);
            }
            Ok(mir::ClassExpression::Property {
                class: expected,
                object,
                property,
            })
        }
        hir::Expr::Index {
            collection,
            index,
            span,
        } => {
            if transfer {
                return Err(vec![unsupported(
                    *span,
                    "indexed class values are borrowed and cannot be moved out",
                )]);
            }
            let (collection, index) = lower_collection_index_operand(
                collection,
                index,
                mir::Type::Class(expected),
                context,
            )?;
            Ok(mir::ClassExpression::CollectionIndex {
                class: expected,
                collection,
                index: Box::new(index),
                transfer: false,
            })
        }
        hir::Expr::New {
            class_type,
            args,
            span,
        } => {
            let class_name = &class_type.name;
            let mir::Type::Class(class) = context.native_type_ref(class_type).ok_or_else(|| {
                vec![unsupported(
                    *span,
                    format!("unknown class instantiation `{class_type}`"),
                )]
            })?
            else {
                return Err(vec![unsupported(
                    *span,
                    format!("`{class_type}` is not a native class type"),
                )]);
            };
            if class != expected {
                return Err(vec![unsupported(
                    *span,
                    format!("constructor for `{class_name}` does not produce expected class"),
                )]);
            }
            let constructor = context.lookup_lifecycle(class, "__construct");
            let constructor_args = if let Some(signature) = constructor.as_ref() {
                lower_call_args_with_ownership(class_name, args, signature.clone(), *span, context)?
            } else {
                if !args.is_empty() {
                    return Err(vec![unsupported(
                        *span,
                        format!("class `{class_name}` does not declare a constructor"),
                    )]);
                }
                Vec::new()
            };
            let properties = lower_new_property_values(class, context)?;
            Ok(mir::ClassExpression::New {
                class,
                properties,
                constructor: constructor.map(|signature| signature.id),
                args: constructor_args,
            })
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::Class(expected)) {
                return Err(vec![unsupported(
                    *span,
                    format!("function `{name}` does not return the expected class"),
                )]);
            }
            Ok(mir::ClassExpression::Call {
                class: expected,
                function: signature.id,
                return_borrow: signature.return_borrow,
                args: lower_call_args_with_ownership(name, args, signature, *span, context)?,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            ..
        } => {
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::Class(expected)) {
                return Err(vec![unsupported(
                    *span,
                    "method does not return expected class",
                )]);
            }
            Ok(mir::ClassExpression::Call {
                class: expected,
                function: signature.id,
                return_borrow: signature.return_borrow,
                args,
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::Class(expected)) {
                return Err(vec![unsupported(
                    *span,
                    "static method does not return expected class",
                )]);
            }
            Ok(mir::ClassExpression::Call {
                class: expected,
                function: signature.id,
                return_borrow: signature.return_borrow,
                args,
            })
        }
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_class_expression(left, expected, transfer, context),
            CoalesceSelection::Right => lower_class_expression(right, expected, transfer, context),
            CoalesceSelection::Dynamic => Ok(mir::ClassExpression::Coalesce {
                class: expected,
                left: Box::new(lower_nullable_class_expression(
                    left, expected, transfer, context,
                )?),
                right: Box::new(lower_class_expression(right, expected, transfer, context)?),
                transfer,
            }),
        },
        _ => Err(vec![unsupported(
            expr.span(),
            "this class expression is not supported by native compilation",
        )]),
    }
}

fn lower_property_operand(
    expr: &hir::Expr,
    expected: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<(mir::LocalId, crate::class_layout::PropertyId)> {
    let (object, property, property_type) = lower_property_place(expr, context)?;
    if property_type != expected {
        return Err(vec![unsupported(
            expr.span(),
            format!("property has MIR type `{property_type}`, expected `{expected}`"),
        )]);
    }
    Ok((object, property))
}

fn lower_property_place(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<(mir::LocalId, crate::class_layout::PropertyId, mir::Type)> {
    let expr = unparenthesized_place(expr);
    let hir::Expr::PropertyAccess {
        object,
        property,
        span,
        ..
    } = expr
    else {
        return Err(vec![unsupported(
            expr.span(),
            "expected class property access",
        )]);
    };
    let object_local = match unparenthesized_place(object) {
        hir::Expr::Variable { name, span } => context.lookup_local(name, *span)?,
        hir::Expr::This { span } => context.lookup_local("this", *span)?,
        _ => {
            return Err(vec![unsupported(
                object.span(),
                "native class property access requires a local object path",
            )])
        }
    };
    let class = match context.local_type(object_local) {
        mir::Type::Class(class) | mir::Type::NullableClass(class) => class,
        _ => {
            return Err(vec![unsupported(
                object.span(),
                "property access object is not a native class value",
            )])
        }
    };
    let property_info = context.property_info(class, property).ok_or_else(|| {
        vec![unsupported(
            *span,
            format!("class#{} has no property `${property}`", class.0),
        )]
    })?;
    let property_type = context
        .mir_resolved_type(&property_info.ty)
        .ok_or_else(|| {
            vec![unsupported(
                *span,
                format!("property `${property}` is not native-lowerable"),
            )]
        })?;
    Ok((object_local, property_info.id, property_type))
}

fn lower_new_property_values(
    class: ClassId,
    context: &mut LoweringContext,
) -> DiagnosticResult<Vec<mir::PropertyValue>> {
    let properties = context
        .class_info(class)
        .ok_or_else(|| {
            vec![unsupported(
                Span::default(),
                format!("unknown class#{}", class.0),
            )]
        })?
        .properties
        .clone();
    properties
        .iter()
        .map(|property| {
            if property.promoted {
                let index = promoted_constructor_argument_index(class, &property.name, context)
                    .ok_or_else(|| {
                        vec![unsupported(
                            Span::default(),
                            format!(
                                "promoted property `${}` has no constructor argument",
                                property.name
                            ),
                        )]
                    })?;
                return Ok(mir::PropertyValue {
                    property: property.id,
                    source: mir::PropertyValueSource::ConstructorArgument(index),
                });
            }
            if let Some(initializer) = context.property_initializers.get(&property.id).cloned() {
                let property_type = context.mir_resolved_type(&property.ty).ok_or_else(|| {
                    vec![unsupported(
                        initializer.expression.span(),
                        format!("property `${}` is not native-lowerable", property.name),
                    )]
                })?;
                let caller_substitutions = std::mem::replace(
                    &mut context.type_substitutions,
                    initializer.type_substitutions,
                );
                let source =
                    lower_rvalue_as_expected(&initializer.expression, property_type, context);
                context.type_substitutions = caller_substitutions;
                return Ok(mir::PropertyValue {
                    property: property.id,
                    source: mir::PropertyValueSource::Expression(source?),
                });
            }
            if context
                .constructor_body_initializers
                .contains(&property.id)
            {
                return Ok(mir::PropertyValue {
                    property: property.id,
                    source: mir::PropertyValueSource::ConstructorBody,
                });
            }
            Err(vec![unsupported(
                Span::default(),
                format!(
                    "class property `${}` is not definitely initialized before construction completes",
                    property.name
                ),
            )])
        })
        .collect()
}

fn promoted_constructor_argument_index(
    class: ClassId,
    property_name: &str,
    context: &mut LoweringContext,
) -> Option<usize> {
    let constructor = context.lookup_lifecycle(class, "__construct")?;
    let class_info = context.class_info(class)?;
    class_info
        .properties
        .iter()
        .filter(|property| property.promoted)
        .position(|property| property.name == property_name)
        .filter(|index| *index < constructor.parameter_types.len())
}

fn lower_float_expression(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::FloatExpression> {
    if let Some(crate::const_eval::ConstValue::Float(value)) = context.constant_value(expr) {
        return Ok(mir::FloatExpression::constant(*value));
    }
    let ty = context.float_type(expr)?;
    if let Some((collection, index, value_type)) = lower_list_remove_at(expr, context)? {
        if value_type != mir::Type::Scalar(mir::ScalarType::Float(ty)) {
            return Err(vec![unsupported(
                expr.span(),
                "List::removeAt result has another float type",
            )]);
        }
        return Ok(mir::FloatExpression::Use {
            ty,
            operand: mir::Operand::CollectionIndex {
                collection,
                index: Box::new(index),
                remove: true,
            },
        });
    }
    match expr {
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_float_expression(left, context),
            CoalesceSelection::Right => lower_float_expression(right, context),
            CoalesceSelection::Dynamic => Ok(mir::FloatExpression::Coalesce {
                ty,
                left: Box::new(lower_nullable_scalar_expression(
                    left,
                    mir::ScalarType::Float(ty),
                    context,
                )?),
                right: Box::new(lower_float_expression(right, context)?),
            }),
        },
        hir::Expr::Float { value, .. } => FloatValue::parse_decimal(ty, value)
            .map(mir::FloatExpression::constant)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "I1401",
                    format!("checked floating literal does not fit `{ty}`"),
                    expr.span(),
                )]
            }),
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::Scalar(mir::ScalarType::Float(local_ty)) if local_ty == ty => {
                    Ok(local_float_expression(local, ty))
                }
                mir::Type::Mixed | mir::Type::NullableMixed
                    if context
                        .exact_mixed_local(expr)
                        .is_some_and(|(_, narrowed)| {
                            narrowed == mir::Type::Scalar(mir::ScalarType::Float(ty))
                        }) =>
                {
                    Ok(mir::FloatExpression::Use {
                        ty,
                        operand: mir::Operand::MixedPayload {
                            mixed: local,
                            tag: mir::MixedTag::Float(ty),
                        },
                    })
                }
                mir::Type::NullableScalar(mir::ScalarType::Float(local_ty)) if local_ty == ty => {
                    Ok(mir::FloatExpression::Use {
                        ty,
                        operand: mir::Operand::NullablePayload(local),
                    })
                }
                _ => Err(vec![Diagnostic::new(
                    "I1401",
                    format!("float local `${name}` does not have expected MIR type `{ty}`"),
                    *span,
                )]),
            }
        }
        hir::Expr::PropertyAccess { .. } => {
            let (object, property) = lower_property_operand(
                expr,
                mir::Type::Scalar(mir::ScalarType::Float(ty)),
                context,
            )?;
            Ok(mir::FloatExpression::Use {
                ty,
                operand: mir::Operand::Property { object, property },
            })
        }
        hir::Expr::Index {
            collection, index, ..
        } => {
            let (collection, index) = lower_collection_index_operand(
                collection,
                index,
                mir::Type::Scalar(mir::ScalarType::Float(ty)),
                context,
            )?;
            Ok(mir::FloatExpression::Use {
                ty,
                operand: mir::Operand::CollectionIndex {
                    collection,
                    index: Box::new(index),
                    remove: false,
                },
            })
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, static_ty) = context.static_property(class_name, member, *span)?;
            if static_ty != mir::Type::Scalar(mir::ScalarType::Float(ty)) {
                return Err(vec![unsupported(
                    *span,
                    "static property has another float type",
                )]);
            }
            Ok(mir::FloatExpression::Use {
                ty,
                operand: mir::Operand::Static(id),
            })
        }
        hir::Expr::Grouped { expr, .. } => lower_float_expression(expr, context),
        hir::Expr::Unary {
            op: hir::UnaryOp::Negate,
            expr,
            ..
        } => Ok(mir::FloatExpression::Negate {
            ty,
            operand: Box::new(lower_float_expression(expr, context)?),
        }),
        hir::Expr::Binary {
            left, op, right, ..
        } => Ok(mir::FloatExpression::Binary {
            ty,
            op: match op {
                hir::BinaryOp::Add => mir::FloatBinaryOp::Add,
                hir::BinaryOp::Sub => mir::FloatBinaryOp::Subtract,
                hir::BinaryOp::Mul => mir::FloatBinaryOp::Multiply,
                hir::BinaryOp::Div => mir::FloatBinaryOp::Divide,
                _ => return Err(vec![unsupported(expr.span(), "invalid float operator")]),
            },
            left: Box::new(lower_float_expression(left, context)?),
            right: Box::new(lower_float_expression(right, context)?),
        }),
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Float(ty)))
            {
                return Err(vec![Diagnostic::new(
                    "I1401",
                    format!("function `{name}` does not return `{ty}`"),
                    *span,
                )]);
            }
            Ok(mir::FloatExpression::Call {
                ty,
                function: signature.id,
                args: lower_call_args(name, args, signature, *span, context)?,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            ..
        } => {
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Float(ty)))
            {
                return Err(vec![unsupported(
                    *span,
                    "method does not return expected float",
                )]);
            }
            Ok(mir::FloatExpression::Call {
                ty,
                function: signature.id,
                args,
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if class_name == "Int" && method == "toFloat" => {
            let [value] = argument_values(args)[..] else {
                return Err(vec![Diagnostic::new(
                    "I1401",
                    "checked Int::toFloat call does not have one argument",
                    *span,
                )]);
            };
            Ok(mir::FloatExpression::IntToFloat {
                value: Box::new(lower_integer_expression(value, context)?),
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Float(ty)))
            {
                return Err(vec![unsupported(
                    *span,
                    "static method does not return expected float",
                )]);
            }
            Ok(mir::FloatExpression::Call {
                ty,
                function: signature.id,
                args,
            })
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "this float expression is not supported by native compilation",
        )]),
    }
}

fn lower_integer_expression(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::IntegerExpression> {
    if let Some(crate::const_eval::ConstValue::Integer(value)) = context.constant_value(expr) {
        return Ok(mir::IntegerExpression::constant(*value));
    }
    if let hir::Expr::FunctionCall { name, span, .. } = expr {
        if context.lookup_function(name, *span)?.return_type == mir::ReturnType::Void {
            return Err(vec![unsupported(
                *span,
                format!("void function `{name}` cannot be used as an integer expression"),
            )]);
        }
    }

    if let Some((magnitude, negative)) = integer_literal_parts(expr) {
        let ty = context.integer_type(expr)?;
        let value = IntegerValue::from_literal(ty, magnitude, negative).ok_or_else(|| {
            vec![Diagnostic::new(
                "I1301",
                format!("internal compiler consistency error: checked literal does not fit `{ty}`"),
                expr.span(),
            )]
        })?;
        return Ok(mir::IntegerExpression::constant(value));
    }

    if let Some((collection, index, value_type)) = lower_list_remove_at(expr, context)? {
        let ty = context.integer_type(expr)?;
        if value_type != mir::Type::Scalar(mir::ScalarType::Integer(ty)) {
            return Err(vec![unsupported(
                expr.span(),
                "List::removeAt result has another integer type",
            )]);
        }
        return Ok(mir::IntegerExpression::Use {
            ty,
            operand: mir::Operand::CollectionIndex {
                collection,
                index: Box::new(index),
                remove: true,
            },
        });
    }

    if let hir::Expr::FunctionCall { name, args, span } = expr {
        let (function, return_type, args) = lower_integer_call(name, args, *span, context)?;
        let ty = context.integer_type(expr)?;
        if return_type != ty {
            return Err(vec![Diagnostic::new(
                "I1301",
                format!(
                    "internal compiler consistency error: function `{name}` returns `{return_type}`, expression metadata says `{ty}`"
                ),
                *span,
            )]);
        }
        return Ok(mir::IntegerExpression::Call { ty, function, args });
    }

    if let hir::Expr::MethodCall {
        object,
        method,
        args,
        span,
        ..
    } = expr
    {
        let (signature, args) = lower_instance_method_call(object, method, args, *span, context)?;
        let ty = context.integer_type(expr)?;
        if signature.return_type
            != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(ty)))
        {
            return Err(vec![unsupported(
                *span,
                "method has a different return type",
            )]);
        }
        return Ok(mir::IntegerExpression::Call {
            ty,
            function: signature.id,
            args,
        });
    }

    let ty = context.integer_type(expr)?;
    match expr {
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_integer_expression(left, context),
            CoalesceSelection::Right => lower_integer_expression(right, context),
            CoalesceSelection::Dynamic => Ok(mir::IntegerExpression::Coalesce {
                ty,
                left: Box::new(lower_nullable_scalar_expression(
                    left,
                    mir::ScalarType::Integer(ty),
                    context,
                )?),
                right: Box::new(lower_integer_expression(right, context)?),
            }),
        },
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::Scalar(mir::ScalarType::Integer(local_ty)) if local_ty == ty => {
                    Ok(local_integer_expression(local, ty))
                }
                mir::Type::Mixed | mir::Type::NullableMixed
                    if context
                        .exact_mixed_local(expr)
                        .is_some_and(|(_, narrowed)| {
                            narrowed == mir::Type::Scalar(mir::ScalarType::Integer(ty))
                        }) =>
                {
                    Ok(mir::IntegerExpression::Use {
                        ty,
                        operand: mir::Operand::MixedPayload {
                            mixed: local,
                            tag: mir::MixedTag::Integer(ty),
                        },
                    })
                }
                mir::Type::NullableScalar(mir::ScalarType::Integer(local_ty))
                    if local_ty == ty =>
                {
                    Ok(mir::IntegerExpression::Use {
                        ty,
                        operand: mir::Operand::NullablePayload(local),
                    })
                }
                _ => Err(vec![Diagnostic::new(
                    "I1301",
                    format!(
                        "internal compiler consistency error: `${name}` does not have MIR type `{ty}`"
                    ),
                    *span,
                )]),
            }
        }
        hir::Expr::PropertyAccess {
            object, property, ..
        } => {
            if matches!(property.as_str(), "length" | "count") {
                if let Ok((collection, _)) = lower_collection_local(object, context) {
                    return Ok(mir::IntegerExpression::Use {
                        ty,
                        operand: mir::Operand::CollectionLength(collection),
                    });
                }
            }
            let (object, property) = lower_property_operand(
                expr,
                mir::Type::Scalar(mir::ScalarType::Integer(ty)),
                context,
            )?;
            Ok(mir::IntegerExpression::Use {
                ty,
                operand: mir::Operand::Property { object, property },
            })
        }
        hir::Expr::Index {
            collection, index, ..
        } => {
            let (collection, index) = lower_collection_index_operand(
                collection,
                index,
                mir::Type::Scalar(mir::ScalarType::Integer(ty)),
                context,
            )?;
            Ok(mir::IntegerExpression::Use {
                ty,
                operand: mir::Operand::CollectionIndex {
                    collection,
                    index: Box::new(index),
                    remove: false,
                },
            })
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, static_ty) = context.static_property(class_name, member, *span)?;
            if static_ty != mir::Type::Scalar(mir::ScalarType::Integer(ty)) {
                return Err(vec![unsupported(
                    *span,
                    "static property has another integer type",
                )]);
            }
            Ok(mir::IntegerExpression::Use {
                ty,
                operand: mir::Operand::Static(id),
            })
        }
        hir::Expr::Grouped { expr, .. } => {
            let lowered = lower_integer_expression(expr, context)?;
            ensure_expression_type(&lowered, ty, expr.span())?;
            Ok(lowered)
        }
        hir::Expr::Unary { op, expr, .. } => {
            let operand = lower_integer_expression(expr, context)?;
            ensure_expression_type(&operand, ty, expr.span())?;
            let op = match op {
                hir::UnaryOp::Negate => mir::IntegerUnaryOp::Negate,
                hir::UnaryOp::BitwiseNot => mir::IntegerUnaryOp::BitwiseNot,
                hir::UnaryOp::Not => return Err(vec![unsupported_int_expr(expr)]),
            };
            Ok(mir::IntegerExpression::Unary {
                ty,
                op,
                operand: Box::new(operand),
            })
        }
        hir::Expr::Binary {
            left, op, right, ..
        } => {
            let op = lower_integer_binary_op(op, expr.span())?;
            let left = lower_integer_expression(left, context)?;
            let right = lower_integer_expression(right, context)?;
            ensure_expression_type(&left, ty, expr.span())?;
            ensure_expression_type(&right, ty, expr.span())?;
            Ok(mir::IntegerExpression::Binary {
                ty,
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        }
        hir::Expr::FunctionCall { .. } => unreachable!("function calls return before type lookup"),
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if class_name == "Float" && method == "toInt" => {
            let [value] = argument_values(args)[..] else {
                return Err(vec![Diagnostic::new(
                    "I1401",
                    "checked Float::toInt call does not have one argument",
                    *span,
                )]);
            };
            Ok(mir::IntegerExpression::FloatToInt {
                value: Box::new(lower_float_expression(value, context)?),
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if method == "from" && IntegerType::from_companion_name(class_name).is_some() => {
            let [value] = argument_values(args)[..] else {
                return Err(vec![Diagnostic::new(
                    "I1301",
                    "internal compiler consistency error: checked integer conversion does not have exactly one argument",
                    *span,
                )]);
            };
            let target = IntegerType::from_companion_name(class_name)
                .expect("guarded integer companion name");
            if target != ty {
                return Err(vec![Diagnostic::new(
                    "I1301",
                    format!(
                        "internal compiler consistency error: `{class_name}::from` targets `{target}`, expression metadata says `{ty}`"
                    ),
                    *span,
                )]);
            }
            Ok(mir::IntegerExpression::Convert {
                ty,
                value: Box::new(lower_integer_expression(value, context)?),
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(ty)))
            {
                return Err(vec![unsupported(
                    *span,
                    "static method has a different return type",
                )]);
            }
            Ok(mir::IntegerExpression::Call {
                ty,
                function: signature.id,
                args,
            })
        }
        hir::Expr::Int { .. } => unreachable!("integer literal handled before expression match"),
        _ => Err(vec![unsupported_int_expr(expr)]),
    }
}

fn lower_integer_binary_op(
    op: &hir::BinaryOp,
    span: Span,
) -> DiagnosticResult<mir::IntegerBinaryOp> {
    match op {
        hir::BinaryOp::Add => Ok(mir::IntegerBinaryOp::Add),
        hir::BinaryOp::Sub => Ok(mir::IntegerBinaryOp::Subtract),
        hir::BinaryOp::Mul => Ok(mir::IntegerBinaryOp::Multiply),
        hir::BinaryOp::Div => Ok(mir::IntegerBinaryOp::Divide),
        hir::BinaryOp::Mod => Ok(mir::IntegerBinaryOp::Remainder),
        hir::BinaryOp::ShiftLeft => Ok(mir::IntegerBinaryOp::ShiftLeft),
        hir::BinaryOp::ShiftRight => Ok(mir::IntegerBinaryOp::ShiftRight),
        hir::BinaryOp::BitwiseAnd => Ok(mir::IntegerBinaryOp::BitwiseAnd),
        hir::BinaryOp::BitwiseXor => Ok(mir::IntegerBinaryOp::BitwiseXor),
        hir::BinaryOp::BitwiseOr => Ok(mir::IntegerBinaryOp::BitwiseOr),
        hir::BinaryOp::Less
        | hir::BinaryOp::LessEqual
        | hir::BinaryOp::Greater
        | hir::BinaryOp::GreaterEqual
        | hir::BinaryOp::Equal
        | hir::BinaryOp::NotEqual => Err(vec![unsupported(
            span,
            "comparison results cannot be used as integer runtime values",
        )]),
        hir::BinaryOp::Concat => Err(vec![unsupported(
            span,
            "string concatenation cannot be used as an integer expression",
        )]),
        hir::BinaryOp::And | hir::BinaryOp::Or | hir::BinaryOp::Xor => Err(vec![unsupported(
            span,
            "boolean operator reached integer-only MIR lowering",
        )]),
        hir::BinaryOp::Coalesce => Err(vec![unsupported(
            span,
            "null coalescing cannot be used as an integer expression",
        )]),
    }
}

fn lower_compound_assignment_op(op: &hir::AssignOp) -> mir::IntegerBinaryOp {
    match op {
        hir::AssignOp::AddAssign => mir::IntegerBinaryOp::Add,
        hir::AssignOp::SubAssign => mir::IntegerBinaryOp::Subtract,
        hir::AssignOp::MulAssign => mir::IntegerBinaryOp::Multiply,
        hir::AssignOp::DivAssign => mir::IntegerBinaryOp::Divide,
        hir::AssignOp::ModAssign => mir::IntegerBinaryOp::Remainder,
        hir::AssignOp::ShiftLeftAssign => mir::IntegerBinaryOp::ShiftLeft,
        hir::AssignOp::ShiftRightAssign => mir::IntegerBinaryOp::ShiftRight,
        hir::AssignOp::BitwiseAndAssign => mir::IntegerBinaryOp::BitwiseAnd,
        hir::AssignOp::BitwiseXorAssign => mir::IntegerBinaryOp::BitwiseXor,
        hir::AssignOp::BitwiseOrAssign => mir::IntegerBinaryOp::BitwiseOr,
        hir::AssignOp::Assign => unreachable!("plain assignment does not have a binary operator"),
    }
}

fn lower_compound_value(
    target: mir::Operand,
    ty: mir::ScalarType,
    op: &hir::AssignOp,
    right: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::ValueExpression> {
    match ty {
        mir::ScalarType::Integer(integer) => {
            let right_span = right.span();
            let right = lower_integer_expression(right, context)?;
            ensure_expression_type(&right, integer, right_span)?;
            Ok(mir::ValueExpression::Integer(
                mir::IntegerExpression::Binary {
                    ty: integer,
                    op: lower_compound_assignment_op(op),
                    left: Box::new(mir::IntegerExpression::use_operand(integer, target)),
                    right: Box::new(right),
                },
            ))
        }
        mir::ScalarType::Float(float) => {
            let right = lower_float_expression(right, context)?;
            let op = match op {
                hir::AssignOp::AddAssign => mir::FloatBinaryOp::Add,
                hir::AssignOp::SubAssign => mir::FloatBinaryOp::Subtract,
                hir::AssignOp::MulAssign => mir::FloatBinaryOp::Multiply,
                hir::AssignOp::DivAssign => mir::FloatBinaryOp::Divide,
                _ => {
                    return Err(vec![unsupported(
                        Span::default(),
                        "invalid float compound assignment",
                    )])
                }
            };
            Ok(mir::ValueExpression::Float(mir::FloatExpression::Binary {
                ty: float,
                op,
                left: Box::new(mir::FloatExpression::Use {
                    ty: float,
                    operand: target,
                }),
                right: Box::new(right),
            }))
        }
        mir::ScalarType::Bool => Err(vec![unsupported(
            Span::default(),
            "bool compound assignment is invalid",
        )]),
    }
}

fn local_integer_expression(local: mir::LocalId, ty: IntegerType) -> mir::IntegerExpression {
    mir::IntegerExpression::use_operand(ty, mir::Operand::Local(local))
}

fn local_float_expression(local: mir::LocalId, ty: FloatType) -> mir::FloatExpression {
    mir::FloatExpression::Use {
        ty,
        operand: mir::Operand::Local(local),
    }
}

fn ensure_value_type(
    expression: &mir::ValueExpression,
    expected: mir::ScalarType,
    span: Span,
) -> DiagnosticResult<()> {
    if expression.ty() == expected {
        Ok(())
    } else {
        Err(vec![Diagnostic::new(
            "I1401",
            format!(
                "internal compiler consistency error: scalar expression has MIR type `{}`, expected `{expected}`",
                expression.ty()
            ),
            span,
        )])
    }
}

fn ensure_expression_type(
    expression: &mir::IntegerExpression,
    expected: IntegerType,
    span: Span,
) -> DiagnosticResult<()> {
    if expression.ty() == expected {
        Ok(())
    } else {
        Err(vec![Diagnostic::new(
            "I1301",
            format!(
                "internal compiler consistency error: integer expression has MIR type `{}`, expected `{expected}`",
                expression.ty()
            ),
            span,
        )])
    }
}

fn integer_literal_parts(expr: &hir::Expr) -> Option<(u128, bool)> {
    match expr {
        hir::Expr::Int { value, .. } => parse_decimal_magnitude(value).map(|value| (value, false)),
        hir::Expr::Grouped { expr, .. } => integer_literal_parts(expr),
        hir::Expr::Unary {
            op: hir::UnaryOp::Negate,
            expr,
            ..
        } => unsigned_integer_literal_magnitude(expr).map(|magnitude| (magnitude, true)),
        _ => None,
    }
}

fn unsigned_integer_literal_magnitude(expr: &hir::Expr) -> Option<u128> {
    match expr {
        hir::Expr::Int { value, .. } => parse_decimal_magnitude(value),
        hir::Expr::Grouped { expr, .. } => unsigned_integer_literal_magnitude(expr),
        _ => None,
    }
}

fn unsupported_int_expr(expr: &hir::Expr) -> Diagnostic {
    let detail = match expr {
        hir::Expr::String { .. } | hir::Expr::InterpolatedString { .. } => {
            "a string expression cannot be used as an integer expression"
        }
        hir::Expr::Float { .. } => "a float expression cannot be used as an integer expression",
        hir::Expr::Bool { .. } => "bool value reached integer-only MIR lowering",
        hir::Expr::IsType { .. } => "a type-test result cannot be used as an integer expression",
        hir::Expr::Null { .. } => "`null` cannot be used as an integer expression",
        hir::Expr::Array { .. } | hir::Expr::ArrayRepeat { .. } => {
            "a collection cannot be used as an integer expression"
        }
        hir::Expr::Index { .. } => {
            "this indexed value cannot be used as an integer expression in this lowering path"
        }
        hir::Expr::FunctionCall { .. } => {
            "this function call cannot be used as an integer expression"
        }
        hir::Expr::MethodCall { .. } | hir::Expr::StaticCall { .. } => {
            "a method call cannot be used as an integer expression"
        }
        hir::Expr::StaticMember { .. } => "a static member cannot be used as an integer expression",
        hir::Expr::PropertyAccess { .. } => {
            "class property access cannot be used as an integer expression"
        }
        hir::Expr::New { .. } => "object construction cannot be used as an integer expression",
        hir::Expr::This { .. } => "`$this` cannot be used as an integer expression",
        hir::Expr::Identifier { .. } => "this identifier cannot be used as an integer expression",
        hir::Expr::Unary { .. } => "this unary expression cannot be used as an integer expression",
        hir::Expr::Range { .. } => "a range cannot be used as an integer expression",
        hir::Expr::Binary {
            op:
                hir::BinaryOp::Equal
                | hir::BinaryOp::NotEqual
                | hir::BinaryOp::Less
                | hir::BinaryOp::LessEqual
                | hir::BinaryOp::Greater
                | hir::BinaryOp::GreaterEqual,
            ..
        } => "comparison results cannot be used as integer runtime values",
        hir::Expr::Binary { .. } => {
            "this binary expression cannot be used as an integer expression"
        }
        hir::Expr::Int { .. } | hir::Expr::Variable { .. } | hir::Expr::Grouped { .. } => {
            "this integer expression is not supported by native compilation"
        }
    };
    unsupported(expr.span(), detail)
}

fn stmt_span(statement: &hir::Stmt) -> Span {
    match statement {
        hir::Stmt::VarDecl(decl) => decl.span,
        hir::Stmt::Assignment(assignment) => assignment.span,
        hir::Stmt::Echo { span, .. } | hir::Stmt::Return { span, .. } => *span,
        hir::Stmt::If(if_stmt) => if_stmt.span,
        hir::Stmt::While(while_stmt) => while_stmt.span,
        hir::Stmt::For(for_stmt) => for_stmt.span,
        hir::Stmt::Break { span } | hir::Stmt::Continue { span } => *span,
        hir::Stmt::Foreach(foreach) => foreach.span,
        hir::Stmt::Increment(increment) => increment.span,
        hir::Stmt::Expr { span, .. } => *span,
    }
}

fn unsupported(span: Span, detail: impl Into<String>) -> Diagnostic {
    Diagnostic::new("M1101", detail, span)
}

fn unsupported_native_type(
    ty: &crate::types::TypeRef,
    span: Span,
    detail: impl Into<String>,
) -> Diagnostic {
    if type_ref_contains_mixed(ty) {
        Diagnostic::unsupported_stage(
            "M1101",
            "the boxed `dr_mixed` runtime representation lands in Stage 23 Slice 3",
            span,
        )
    } else {
        unsupported(span, detail)
    }
}

fn type_ref_contains_mixed(ty: &crate::types::TypeRef) -> bool {
    ty.name == "mixed" || ty.type_arguments().any(type_ref_contains_mixed)
}
