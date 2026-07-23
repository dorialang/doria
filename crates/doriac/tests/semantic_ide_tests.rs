use doriac::ast::{ClassMember, Expr, Item, Stmt};
use doriac::semantics::{analyze_program_for_ide, CallableTarget};

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
            class_name: "Greeter".to_string(),
            method_name: "greet".to_string(),
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
            class_name: "Greeter".to_string(),
            method_name: "greet".to_string(),
        })
    );
}
