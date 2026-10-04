use doriac::ast::{ClassMember, Expr, Item, Stmt};
use doriac::semantics::{analyze_program_for_ide, CallableTarget};

#[test]
fn constrained_member_surfaces_keep_lexical_bounds_and_substituted_contracts() {
    use doriac::ast::PropertyHookKind::Get;
    use doriac::source::Span;
    use doriac::types::ResolvedType;

    let source = r#"
interface Reader<T> {
    function read(): T;
    function(): T $callback { get; }
}
interface Fault extends Error { function code(): int; }
function number<T implements Reader<int>>(T $number): void { $number->read(); }
function text<T implements Reader<string>>(T $text): void { $text->read(); }
function failure<T implements Fault>(T $failure): void { $failure->code(); }
class Container<T implements Reader<bool>> {
    function outer(T $outer): void { $outer->read(); }
    function inner<U implements Reader<float>>(U $inner): void { $inner->read(); }
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("members.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    for (name, expected) in [
        (
            "number",
            ResolvedType::Integer(doriac::types::IntegerType::Int64),
        ),
        ("text", ResolvedType::String),
        ("outer", ResolvedType::Bool),
        (
            "inner",
            ResolvedType::Float(doriac::types::FloatType::Float64),
        ),
    ] {
        let start = source.find(&format!("${name}->")).unwrap();
        let surface = &analysis.info.contracts.constrained_member_surfaces
            [&Span::new(start, start + name.len() + 1)];
        assert!(!surface.has_error_message);
        assert_eq!(surface.requirements.len(), 2);
        let read = surface
            .requirements
            .iter()
            .find(|member| member.name == "read")
            .unwrap();
        assert_eq!(read.signature.return_type, expected);
        let callback = surface
            .requirements
            .iter()
            .find(|member| member.name == "callback")
            .unwrap();
        assert_eq!(callback.accessor, Some(Get));
        let ResolvedType::Function(function) = &callback.signature.return_type else {
            panic!("{:?}", callback.signature.return_type);
        };
        assert_eq!(function.return_type, expected);
    }
    let start = source.find("$failure->").unwrap();
    assert!(
        analysis.info.contracts.constrained_member_surfaces
            [&Span::new(start, start + "$failure".len())]
            .has_error_message
    );
}

#[test]
fn constrained_member_surfaces_share_intersection_selection_with_calls() {
    use doriac::source::Span;
    for constraints in ["Read, Cached, Conflict", "Conflict, Cached, Read"] {
        let source = format!(
            r#"
interface Read {{
    int $value {{ get; }}
    function read(): int;
    function clash(): int;
}}
interface Cached {{
    int $value {{ writable get; }}
    writable function read(): int;
}}
interface Conflict {{ function clash(): string; }}
function inspect<T implements {constraints}>(T $receiver): int {{ return $receiver->value; }}
"#
        );
        let (_, analysis) = doriac::analyze_source_for_ide("intersection.doria", &source).unwrap();
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let start = source.find("$receiver->").unwrap();
        let surface = &analysis.info.contracts.constrained_member_surfaces
            [&Span::new(start, start + "$receiver".len())];
        assert_eq!(surface.requirements.len(), 2, "{surface:?}");
        for requirement in &surface.requirements {
            assert!(matches!(requirement.name.as_str(), "read" | "value"));
            assert!(!requirement.writable_receiver);
            assert_eq!(requirement.origins.len(), 2);
        }
    }
}

#[test]
fn member_receivers_expose_checked_access_without_inventing_writable_capability() {
    use doriac::semantics::ObjectPathAccess::{ConstructionRoot, Readonly, Writable};
    let source = r#"
interface Reading { function read(): int; }
class Leaf implements Reading {
    writable int $value = 0;
    function read(): int { return $this->value; }
}
class Holder {
    writable Leaf $child = new Leaf();
    Leaf $fixed = new Leaf();
    function __construct() { $this->child->read(); }
    function inspect(): int { return $this->fixed->read(); }
    writable function change(): void { $this->child->value = 1; }
}
function borrowLeaf(writable Leaf $source): Leaf { return $source; }
function inspect(Leaf $readonly, writable Leaf $writable,
    Reading $interface, writable Reading $writableInterface,
    SharedReference<Leaf> $shared,
    ReadonlySharedReferenceAccess<Leaf> $readAccess,
    WritableSharedReferenceAccess<Leaf> $writeAccess): void {
    $readonly->read();
    $writable->read();
    $interface->read();
    $writableInterface->read();
    $shared->read();
    $readAccess->read();
    $writeAccess->read();
    borrowLeaf($writable)->read();
    new Leaf()->read();
    let $local = new Leaf();
    let writable $writableLocal = new Leaf();
    $local->read();
    $writableLocal->read();
    $writableLocal->value = 2;
}
function constrained<T implements Reading>(T $generic, writable T $writableGeneric): void {
    $generic->read();
    $writableGeneric->read();
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("access.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    for (operation, expected) in [
        ("$this->value", Readonly),
        ("$this->child->read()", Writable),
        ("$this->child", ConstructionRoot),
        ("$this->fixed->read()", Readonly),
        ("$this->fixed", Readonly),
        ("$this->child->value = 1", Writable),
        ("$readonly->read()", Readonly),
        ("$writable->read()", Writable),
        ("$interface->read()", Readonly),
        ("$writableInterface->read()", Writable),
        ("$shared->read()", Readonly),
        ("$readAccess->read()", Readonly),
        ("$writeAccess->read()", Writable),
        ("borrowLeaf($writable)->read()", Writable),
        ("new Leaf()->read()", Writable),
        ("$local->read()", Readonly),
        ("$writableLocal->read()", Writable),
        ("$writableLocal->value = 2", Writable),
        ("$generic->read()", Readonly),
        ("$writableGeneric->read()", Writable),
    ] {
        let start = source.find(operation).unwrap();
        let end = start + operation.rfind("->").unwrap();
        assert_eq!(
            analysis
                .info
                .member_receiver_access(doriac::source::Span::new(start, end)),
            Some(expected),
            "{operation}"
        );
    }
}

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
                method.body.statements().iter().find_map(|statement| {
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

open class Generic<T> {}
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
    let generic = analysis
        .info
        .class_hierarchy
        .values()
        .find(|class| class.name == "Generic")
        .expect("uninstantiated generic class hierarchy fact");
    assert!(generic.is_open);
    assert_eq!(generic.generic_parameter_count, 1);
    assert!(generic.parent.is_none());
    let leaf_class = analysis
        .info
        .class_hierarchy
        .values()
        .find(|class| class.name == "Leaf")
        .expect("derived class hierarchy fact");
    assert_eq!(leaf_class.ancestors, ["Middle", "Root"]);

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
