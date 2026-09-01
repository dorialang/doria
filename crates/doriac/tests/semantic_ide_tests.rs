use doriac::ast::{ClassMember, Expr, Item, Stmt};
use doriac::semantics::{analyze_program_for_ide, CallableTarget};

fn method_declaration_span(
    program: &doriac::ast::Program,
    class_name: &str,
    method_name: &str,
) -> doriac::source::Span {
    program
        .items
        .iter()
        .find_map(|item| {
            let Item::Class(class) = item else {
                return None;
            };
            (class.name == class_name).then(|| {
                class.members.iter().find_map(|member| {
                    let ClassMember::Method(method) = member else {
                        return None;
                    };
                    (method.name == method_name).then_some(method.span)
                })
            })?
        })
        .expect("fixture should contain the requested method declaration")
}

fn method_call_span(program: &doriac::ast::Program, method_name: &str) -> doriac::source::Span {
    program
        .items
        .iter()
        .find_map(|item| {
            let Item::Class(class) = item else {
                return None;
            };
            class.members.iter().find_map(|member| {
                let ClassMember::Method(method) = member else {
                    return None;
                };
                method.body.statements.iter().find_map(|statement| {
                    let Stmt::Expr { expr, .. } = statement else {
                        return None;
                    };
                    match expr {
                        Expr::MethodCall { method, span, .. } if method == method_name => {
                            Some(*span)
                        }
                        _ => None,
                    }
                })
            })
        })
        .expect("fixture should contain the requested method call")
}

#[test]
fn exposes_compiler_resolved_method_targets() {
    let source = r#"class Greeter
{
    function greet(): void
    {
    }

    function run(): void
    {
        $this->greet();
    }
}
"#;
    let program = doriac::parse_source("test.doria", source).expect("source should parse");
    let call_span = method_call_span(&program, "greet");
    let analysis = analyze_program_for_ide(&program);

    assert!(analysis.diagnostics.is_empty());
    assert_eq!(
        analysis.info.call_target(call_span),
        Some(&CallableTarget::Method {
            class_type: doriac::types::ClassType::new("Greeter", Vec::new()),
            method_name: "greet".to_string(),
            direct_parent: false,
        })
    );
}

#[test]
fn keeps_resolved_targets_when_other_semantic_diagnostics_exist() {
    let source = r#"class Greeter
{
    function greet(): void
    {
    }

    function run(): void
    {
        $this->greet();
        missing();
    }
}
"#;
    let program = doriac::parse_source("test.doria", source).expect("source should parse");
    let call_span = method_call_span(&program, "greet");
    let analysis = analyze_program_for_ide(&program);

    assert!(analysis
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("unknown function `missing`")));
    assert_eq!(
        analysis.info.call_target(call_span),
        Some(&CallableTarget::Method {
            class_type: doriac::types::ClassType::new("Greeter", Vec::new()),
            method_name: "greet".to_string(),
            direct_parent: false,
        })
    );
}

#[test]
fn exposes_compiler_owned_override_and_call_family_identities() {
    let source = r#"open class Root
{
    open function value(int $input = 1): int { return $input; }
}

open class Middle extends Root
{
    override function value(int $input): int { return parent::value($input); }
}

class Leaf extends Middle
{
    override function value(int $input): int { return $input + 1; }
    function run(): int { return $this->value(); }
}
"#;
    let program = doriac::parse_source("hierarchy.doria", source).expect("source should parse");
    let root = method_declaration_span(&program, "Root", "value");
    let middle = method_declaration_span(&program, "Middle", "value");
    let leaf = method_declaration_span(&program, "Leaf", "value");
    let analysis = analyze_program_for_ide(&program);

    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let root_info = &analysis.info.method_hierarchy[&root];
    assert!(root_info.is_open);
    assert!(!root_info.is_override);
    assert_eq!(root_info.virtual_root, Some(root));
    assert_eq!(root_info.overridden_declaration, None);

    let middle_info = &analysis.info.method_hierarchy[&middle];
    assert!(!middle_info.is_open);
    assert!(middle_info.is_override);
    assert_eq!(middle_info.virtual_root, Some(root));
    assert_eq!(middle_info.overridden_declaration, Some(root));

    let leaf_info = &analysis.info.method_hierarchy[&leaf];
    assert_eq!(leaf_info.virtual_root, Some(root));
    assert_eq!(leaf_info.overridden_declaration, Some(middle));
    assert!(analysis.info.callable_signatures[&middle].parameters[0].has_default);
    assert!(analysis.info.callable_signatures[&leaf].parameters[0].has_default);

    assert!(analysis.info.method_call_targets.values().any(|target| {
        target.declaration == root && target.virtual_root == Some(root) && target.direct_parent
    }));
    assert!(analysis.info.method_call_targets.values().any(|target| {
        target.declaration == leaf && target.virtual_root == Some(root) && !target.direct_parent
    }));
}
