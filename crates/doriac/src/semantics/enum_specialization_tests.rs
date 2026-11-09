use super::*;

fn with_checker(inspect: impl FnOnce(&mut Checker<'_>)) {
    let source = "enum Packet { case Value(bool $value); } function main(): void {}";
    with_source_checker(source, |checker| {
        assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
        inspect(checker);
    });
}

#[test]
fn folded_enum_admission_records_the_checked_application() {
    with_source_checker(
        "enum Packet<T> { case Value(T $value); } const Packet<bool> VALUE = Packet::Value(true);",
        |checker| {
            let value = checker.const_evaluation.values
                [&crate::const_eval::ConstKey::TopLevel("VALUE".into())]
                .value
                .clone();
            let crate::const_eval::ConstValue::PayloadEnum(payload) = &value else {
                panic!("expected folded enum");
            };
            let application = payload
                .value
                .enum_type
                .map_arguments(|ty| checker.types.intern_resolved(ty));
            checker.enum_instantiations.remove(&application);
            assert!(checker.admit_constant_enum_applications(&value, Span::default(), None));
            assert!(checker.enum_instantiations.contains(&application));
        },
    );
}

fn with_source_checker(source: &str, inspect: impl FnOnce(&mut Checker<'_>)) {
    let path = "enum-specialization.doria";
    let context = crate::names::CompilationContext::standalone(path);
    let program = crate::parse_source(path, source).unwrap();
    let source_file = crate::source::SourceFile::new(path, source);
    let prepared = crate::prepare_parsed_source(&source_file, context.clone(), program).unwrap();
    let program = prepared.resolved;
    let mut texts = HashMap::from([(crate::source::SourceId(0), source)]);
    let mut contexts = HashMap::new();
    crate::append_compiler_known_semantic_context(&program, &context, &mut texts, &mut contexts);
    let mut checker = Checker::new(
        &program,
        crate::const_eval::evaluate_program_with_diagnostics(&program).0,
        &texts,
        context,
        contexts,
        prepared.global_symbols,
        prepared.source_semantic_contexts,
        prepared.test_semantics,
    );
    checker.predeclare_classes();
    checker.collect_enums();
    assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
    inspect(&mut checker);
}

// Model checker-owned instances before enabling generic source construction.
fn instance(
    template: &EnumDefinition,
    id: usize,
    name: &str,
    argument: TypeId,
    payload: TypeId,
) -> EnumDefinition {
    let mut instance = template.clone();
    instance.id = EnumId(id);
    instance.name = name.to_owned();
    instance.source_type =
        EnumType::with_arguments(template.source_type.id, "Packet", vec![argument]);
    instance.cases[0].id.enum_id = instance.id;
    instance.cases[0].payload[0].ty = payload;
    instance
}

fn instance_named<'a>(checker: &'a Checker<'_>, name: &str) -> &'a EnumDefinition {
    checker
        .enum_instances
        .values()
        .find(|definition| definition.name == name)
        .unwrap()
}

#[test]
fn checker_substitution_preserves_enum_arguments_through_wrappers() {
    with_checker(|checker| {
        let declaration = checker.enums["Packet"].source_type.id;
        let parameter = checker.types.intern(TypeKind::TypeParameter("T".into()));
        let symbolic = checker
            .types
            .intern(TypeKind::Enum(EnumType::with_arguments(
                declaration,
                "Packet",
                vec![parameter],
            )));
        let optional = checker.types.intern(TypeKind::Nullable(symbolic));
        let list = checker.types.intern(TypeKind::List(optional));
        assert!(checker.type_is_symbolic(list));
        let mut applications = Vec::new();
        for argument in [TypeKind::Bool, TypeKind::String] {
            let argument = checker.types.intern(argument);
            let substitutions = HashMap::from([("T".to_owned(), argument)]);
            let concrete = checker.substitute_type_id(list, &substitutions);
            assert!(!checker.type_is_symbolic(concrete));
            assert_eq!(checker.substitute_type(list, &substitutions), concrete);
            assert_eq!(
                checker.types.resolved(concrete),
                ResolvedType::List(Box::new(crate::types::nullable_of(ResolvedType::Enum(
                    EnumType::with_arguments(
                        declaration,
                        "Packet",
                        vec![checker.types.resolved(argument)],
                    ),
                )))),
            );
            applications.push(concrete);
        }
        assert_ne!(applications[0], applications[1]);
        assert!(checker.type_is_symbolic(list));
    });
}

#[test]
fn enum_match_payload_substitution_preserves_symbolic_identity_without_materialization() {
    with_source_checker(
        "enum Packet<T> { case Value(List<T> $value); }",
        |checker| {
            let before = checker
                .enum_instances
                .keys()
                .cloned()
                .collect::<HashSet<_>>();
            let template = checker.enums["Packet"].clone();
            let parameter = checker.types.intern(TypeKind::TypeParameter("U".into()));
            let source =
                EnumType::with_arguments(template.source_type.id, "Packet", vec![parameter]);
            let symbolic = checker.types.intern(TypeKind::Enum(source.clone()));
            assert!(checker.type_is_move_type(symbolic));
            assert!(checker.parameter_can_supply_return_borrow(symbolic, false));
            let payload = checker.enum_case_payload(&source, 0).unwrap();
            assert_eq!(
                checker.types.resolved(payload[0].ty),
                ResolvedType::List(Box::new(ResolvedType::TypeParameter("U".into())))
            );
            assert_eq!(
                checker
                    .types
                    .resolved(checker.enums["Packet"].cases[0].payload[0].ty),
                ResolvedType::List(Box::new(ResolvedType::TypeParameter("T".into())))
            );
            assert!(checker.enum_case_payload(&source, 1).is_none());
            let wrong = EnumType::with_arguments(EnumId(usize::MAX), "Packet", vec![parameter]);
            assert!(checker.enum_case_payload(&wrong, 0).is_none());
            let missing = EnumType::with_arguments(template.source_type.id, "Packet", vec![]);
            assert!(checker.enum_case_payload(&missing, 0).is_none());
            assert_eq!(
                checker
                    .enum_instances
                    .keys()
                    .cloned()
                    .collect::<HashSet<_>>(),
                before
            );
        },
    );
}

#[test]
fn enum_inference_preserves_argument_bindings_and_fills_contextual_gaps() {
    with_source_checker(
        "enum Result<T, U> { case First(T $value); case Second(U $value); }",
        |checker| {
            let template = checker.enums["Result"].source_type.clone();
            let pattern = checker.types.intern(TypeKind::Enum(template.clone()));
            let boolean = checker.types.intern(TypeKind::Bool);
            let string = checker.types.intern(TypeKind::String);
            let integer = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
            let actual = checker
                .types
                .intern(TypeKind::Enum(EnumType::with_arguments(
                    template.id,
                    "Result",
                    vec![boolean, string],
                )));
            let pattern = checker.types.intern(TypeKind::Nullable(pattern));
            let pattern = checker.types.intern(TypeKind::List(pattern));
            let actual = checker.types.intern(TypeKind::Nullable(actual));
            let actual = checker.types.intern(TypeKind::List(actual));
            let mut bindings = HashMap::new();
            checker.infer_type_parameter_bindings(
                "consume",
                pattern,
                actual,
                Span::default(),
                &mut bindings,
                false,
            );
            assert_eq!(
                bindings,
                HashMap::from([("T".to_owned(), boolean), ("U".to_owned(), string)])
            );

            let mut contextual = HashMap::from([("T".to_owned(), integer)]);
            checker.infer_type_parameter_bindings(
                "consume",
                pattern,
                actual,
                Span::default(),
                &mut contextual,
                true,
            );
            assert_eq!(
                contextual,
                HashMap::from([("T".to_owned(), integer), ("U".to_owned(), string)])
            );
            assert!(checker.diagnostics.is_empty());

            checker.infer_type_parameter_bindings(
                "consume",
                pattern,
                actual,
                Span::default(),
                &mut contextual,
                false,
            );
            let conflicts = checker
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "E0532")
                .collect::<Vec<_>>();
            assert_eq!(conflicts.len(), 1);
            assert!(conflicts[0].message.contains("`T`"));
            assert_eq!(contextual["T"], integer);
        },
    );
}

#[test]
fn enum_inference_requires_the_same_nominal_declaration_and_arity() {
    with_source_checker("enum Packet<T> { case Value(T $value); }", |checker| {
        let template = checker.enums["Packet"].source_type.clone();
        let pattern = checker.types.intern(TypeKind::Enum(template.clone()));
        let boolean = checker.types.intern(TypeKind::Bool);
        for (id, name, arguments) in [
            (EnumId(template.id.0 + 1), "Packet", vec![boolean]),
            (template.id, "Different", vec![boolean]),
            (template.id, "Packet", vec![boolean, boolean]),
        ] {
            let actual = checker
                .types
                .intern(TypeKind::Enum(EnumType::with_arguments(
                    id, name, arguments,
                )));
            for from_context in [false, true] {
                let mut bindings = HashMap::new();
                checker.infer_type_parameter_bindings(
                    "consume",
                    pattern,
                    actual,
                    Span::default(),
                    &mut bindings,
                    from_context,
                );
                assert!(bindings.is_empty());
            }
        }
        assert!(checker.diagnostics.is_empty());
    });
}

#[test]
fn concrete_enum_capabilities_and_nested_layouts_follow_full_source_identity() {
    with_checker(|checker| {
        let template = checker.enums["Packet"].clone();
        let boolean = checker.types.intern(TypeKind::Bool);
        let string = checker.types.intern(TypeKind::String);
        let owner = checker
            .types
            .intern(TypeKind::Class(ClassType::new("Owner", Vec::new())));
        let first = instance(&template, 41, "PacketBool", boolean, boolean);
        let second = instance(&template, 57, "PacketString", string, string);
        let third = instance(&template, 73, "PacketOwner", owner, owner);
        let first_ty = checker
            .types
            .intern(TypeKind::Enum(first.source_type.clone()));
        let optional = checker.types.intern(TypeKind::Nullable(first_ty));
        let nested = instance(&template, 89, "PacketNested", optional, optional);
        checker.enum_instances = [first, second, third, nested]
            .into_iter()
            .map(|definition| (definition.source_type.clone(), definition))
            .collect();
        checker.finalize_enum_metadata();
        assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
        let mut types = Vec::new();
        for (name, copy, trivial, drop) in [
            ("PacketBool", true, true, false),
            ("PacketString", true, false, true),
            ("PacketOwner", false, false, true),
            ("PacketNested", true, true, false),
        ] {
            let definition = instance_named(checker, name).clone();
            assert_eq!(
                definition.capabilities,
                EnumCapabilities {
                    copy,
                    trivial_copy: trivial,
                    needs_drop: drop,
                    equality: true
                },
            );
            assert_eq!(definition.layout.as_ref().unwrap().enum_id, definition.id);
            let ty = checker
                .types
                .intern(TypeKind::Enum(definition.source_type.clone()));
            assert_eq!(checker.type_is_move_type(ty), !copy);
            assert_eq!(
                checker.enum_definition(&definition.source_type).unwrap().id,
                definition.id
            );
            types.push(ty);
        }
        assert!(
            instance_named(checker, "PacketBool")
                .layout
                .as_ref()
                .unwrap()
                .size
                < instance_named(checker, "PacketString")
                    .layout
                    .as_ref()
                    .unwrap()
                    .size
        );
        assert!(!checker.is_equality_compatible(types[0], types[1]));
        assert!(!checker.is_equality_compatible(types[0], types[3]));
    });
}

#[test]
fn inline_cycles_are_reported_per_application_without_poisoning_siblings() {
    with_checker(|checker| {
        let template = checker.enums["Packet"].clone();
        let boolean = checker.types.intern(TypeKind::Bool);
        let string = checker.types.intern(TypeKind::String);
        let integer = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
        let mut first = instance(&template, 83, "PacketBool", boolean, boolean);
        let mut second = instance(&template, 91, "PacketString", string, string);
        let first_ty = checker
            .types
            .intern(TypeKind::Enum(first.source_type.clone()));
        let second_ty = checker
            .types
            .intern(TypeKind::Enum(second.source_type.clone()));
        first.cases[0].payload[0].ty = second_ty;
        second.cases[0].payload[0].ty = checker.types.intern(TypeKind::Nullable(first_ty));
        let sibling = instance(&template, 107, "PacketInt", integer, integer);
        checker.enum_instances = [first, second, sibling]
            .into_iter()
            .map(|definition| (definition.source_type.clone(), definition))
            .collect();
        checker.finalize_enum_metadata();
        assert_eq!(checker.diagnostics.len(), 1, "{:?}", checker.diagnostics);
        assert_eq!(checker.diagnostics[0].code, "E0581");
        for name in ["PacketBool", "PacketString"] {
            assert!(instance_named(checker, name).layout.is_none());
            assert!(!instance_named(checker, name).capabilities.copy);
        }
        assert!(instance_named(checker, "PacketInt").layout.is_some());
        assert!(
            instance_named(checker, "PacketInt")
                .capabilities
                .trivial_copy
        );
    });
}

#[test]
fn admitted_enum_dependencies_substitute_payloads_and_reuse_instances() {
    let source = r#"
class Owner {}
class Hidden<T> {}
class Wrapper<T> {}
enum Packet<T> { case Value(T $value); }
enum Unused<U> { case Value(Hidden<int> $value); }
enum Root {
    case Value(Packet<Packet<bool>> $nested, Packet<bool> $repeated,
        Packet<string> $text, Packet<Owner> $owner);
}
"#;
    with_source_checker(source, |checker| {
        checker.collect_classes();
        let integer = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
        let packet = checker
            .types
            .intern(TypeKind::Enum(EnumType::with_arguments(
                checker.enums["Packet"].source_type.id,
                "Packet",
                vec![integer],
            )));
        checker
            .class_instantiations
            .insert(ClassType::new("Wrapper", vec![packet]));
        let mut expanded = HashSet::new();
        instantiation::expand_nominal_instantiations(checker, &mut expanded);
        checker.finalize_enum_metadata();
        assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
        assert!(!checker
            .class_instantiations
            .iter()
            .any(|class| class.name == "Hidden"));
        assert!(!checker
            .enum_instances
            .keys()
            .any(|enumeration| enumeration.name == "Unused"));
        let instances = checker
            .enum_instances
            .values()
            .filter(|definition| definition.source_type.name == "Packet")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(instances.len(), 5);
        let template = &checker.enums["Packet"];
        assert!(template.layout.is_none());
        assert!(checker.type_is_symbolic(template.cases[0].payload[0].ty));
        for definition in &instances {
            assert_eq!(
                definition.cases[0].payload[0].ty,
                definition.source_type.arguments[0]
            );
            assert_eq!(definition.span, template.span);
            assert_eq!(definition.cases[0].span, template.cases[0].span);
            assert_eq!(definition.layout.as_ref().unwrap().enum_id, definition.id);
            assert_eq!(
                definition.capabilities.copy,
                !matches!(
                    checker.types.kind(definition.source_type.arguments[0]),
                    TypeKind::Class(_)
                )
            );
        }
        let before = collect_ordered_enum_semantics(checker);
        instantiation::expand_nominal_instantiations(checker, &mut HashSet::new());
        checker.finalize_enum_metadata();
        assert_eq!(collect_ordered_enum_semantics(checker), before);
        for (index, definition) in before.iter().enumerate() {
            assert_eq!(definition.id, EnumId(index));
        }
    });
}

#[test]
fn authored_case_identity_resolves_per_concrete_instance() {
    with_source_checker(
        "enum Packet<T> { case Empty; case Value(T $value); } enum Root { case Pair(Packet<bool> $first, Packet<string> $second); }",
        |checker| {
            checker.collect_classes();
            instantiation::expand_nominal_instantiations(checker, &mut HashSet::new());
            checker.finalize_enum_metadata();
            let source = crate::enums::EnumCaseSource {
                declaration: checker.enums["Packet"].source_type.id,
                index: 1,
            };
            let instances = collect_ordered_enum_semantics(checker);
            let applications = instances.iter().filter(|instance| instance.source_type.id == source.declaration).collect::<Vec<_>>();
            assert_eq!(applications.len(), 2);
            let cases = applications.iter().map(|instance| {
                let case = instance.resolve_case(source).unwrap();
                assert_eq!(case.enum_id, instance.id);
                assert_eq!(case.index, source.index);
                assert!(instance.resolve_case(crate::enums::EnumCaseSource { index: 2, ..source }).is_none());
                assert!(instance.resolve_case(crate::enums::EnumCaseSource { declaration: EnumId(usize::MAX), ..source }).is_none());
                case
            }).collect::<HashSet<_>>();
            assert_eq!(cases.len(), 2);
            let root = instances.iter().find(|instance| instance.source_type.name == "Root").unwrap();
            assert!(root.resolve_case(source).is_none());
        },
    );
}

#[test]
fn enum_and_class_dependencies_share_the_expansion_guard() {
    for source in [
        "enum Chain<T> { case Next(Chain<List<T>> $next); } enum Root { case Value(Chain<int> $value); }",
        "class Link<T> { ?Chain<List<T>> $next = null; } enum Chain<T> { case Next(Link<T> $next); } enum Root { case Value(Chain<int> $value); }",
    ] {
        with_source_checker(source, |checker| {
            checker.collect_classes();
            instantiation::expand_nominal_instantiations(checker, &mut HashSet::new());
            assert!(checker.diagnostics.iter().any(|diagnostic| diagnostic.code == "E0539"), "{:?}", checker.diagnostics);
            assert!(checker.diagnostics.iter().all(|diagnostic| diagnostic.code == "E0539"), "{:?}", checker.diagnostics);
            let before = (checker.enum_instances.len(), checker.class_instantiations.len(), checker.diagnostics.len());
            instantiation::expand_nominal_instantiations(checker, &mut HashSet::new());
            assert_eq!((checker.enum_instances.len(), checker.class_instantiations.len(), checker.diagnostics.len()), before);
            for root in checker.nominal_instantiation_parents.keys() {
                let mut seen = HashSet::new();
                let mut current = Some(root);
                while let Some(instance) = current {
                    assert!(seen.insert(instance), "cyclic ancestry: {root:?}");
                    current = checker.nominal_instantiation_parents.get(instance);
                }
            }
        });
    }
}

#[test]
fn nominal_proofs_do_not_admit_enum_instances_or_dependencies() {
    with_source_checker("enum Packet<T> { case Value(T $value); }", |checker| {
        let instances = checker.enum_instantiations.clone();
        let templates = checker.nominal_instantiation_templates.clone();
        let callables = checker.callable_nominal_instantiation_templates.clone();
        let reference = TypeRef::generic("Packet", vec![TypeRef::named("int")]);
        checker.current_callable = Some(Span::new(1, 2));
        checker.with_nominal_identity(|checker| {
            let ty = checker.resolve_type_ref(&reference, Span::default());
            checker.substitute_type(ty, &HashMap::new());
        });
        assert_eq!(checker.enum_instantiations, instances);
        assert_eq!(checker.nominal_instantiation_templates, templates);
        assert_eq!(checker.callable_nominal_instantiation_templates, callables);
        checker.resolve_type_ref(&reference, Span::default());
        assert_eq!(checker.enum_instantiations, instances);
        assert!(checker.callable_nominal_instantiation_templates[&Span::new(1, 2)].iter().any(|instance| matches!(instance, instantiation::NominalInstance::Enum(enumeration) if enumeration.name == "Packet")));
    });
}
