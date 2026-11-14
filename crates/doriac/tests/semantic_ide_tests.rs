use doriac::ast::{ClassMember, Expr, Item, Stmt};
use doriac::semantics::{analyze_program_for_ide, CallableTarget};

#[test]
fn member_surfaces_preserve_retained_loans_for_copy_regions() {
    use doriac::symbols::{BorrowSource, ReturnBorrow};
    use doriac::types::{ClassType, IntegerType, ResolvedType, RetainedLifetime, ReturnBorrowKind};

    let source = r#"
use Doria\Std\Io\ByteRegion;
class Storage {
    Bytes $bytes = Bytes::allocate(1);
    function region(): ByteRegion { return $this->bytes->getRegion(0, 1); }
    function optional(): ?ByteRegion { return $this->bytes->getRegion(0, 1); }
    ByteRegion $view { get => $this->bytes->getRegion(0, 1); }
    ?ByteRegion $optionalView { get => $this->bytes->getRegion(0, 1); }
}
class Snapshot<T> {
    function __construct(take T $stored) {}
    function value(): T { return $this->stored; }
    T $item { borrowed get => $this->stored; }
}
function inspect(Snapshot<int> $snapshot): void {}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("member-loans.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let storage = analysis
        .info
        .class_member_surface(&ClassType::new("Storage", Vec::new()))
        .unwrap();
    let expected = Some(ReturnBorrow {
        source: BorrowSource::Receiver,
        writable: false,
        kind: ReturnBorrowKind::Retained,
        lifetime: RetainedLifetime::LastUse,
    });
    for name in ["region", "optional", "view", "optionalView"] {
        let member = storage
            .members
            .iter()
            .find(|member| member.name == name)
            .unwrap();
        let borrow = member.hooks.as_ref().map_or(member.return_borrow, |hooks| {
            hooks.getter.as_ref().unwrap().return_borrow
        });
        assert_eq!(borrow, expected, "{member:?}");
    }
    let snapshot = analysis
        .info
        .class_member_surfaces
        .iter()
        .find(|surface| {
            surface.receiver.name == "Snapshot"
                && surface.receiver.arguments == [ResolvedType::Integer(IntegerType::Int64)]
        })
        .unwrap();
    for name in ["value", "item"] {
        let member = snapshot
            .members
            .iter()
            .find(|member| member.name == name)
            .unwrap();
        assert!(member.return_borrow.is_none(), "{member:?}");
        if let Some(hooks) = &member.hooks {
            assert!(
                hooks.getter.as_ref().unwrap().return_borrow.is_none(),
                "{member:?}"
            );
        }
    }
}

#[test]
fn transfer_group_member_surface_keeps_nullable_completed_region_loan() {
    use doriac::compiler_known_stream::{READ_TRANSFER_KEY, TRANSFER_GROUP};
    use doriac::symbols::{BorrowSource, ReturnBorrow};
    use doriac::types::{ClassType, RetainedLifetime, ReturnBorrowKind};

    for (prefix, group, key) in [
        (String::new(), TRANSFER_GROUP, READ_TRANSFER_KEY),
        (
            format!("use {TRANSFER_GROUP} as Group; use {READ_TRANSFER_KEY} as Key;"),
            "Group",
            "Key",
        ),
    ] {
        let source = format!("{prefix} function inspect({group} $group, {key} $key): void {{}}");
        let (_, analysis) =
            doriac::analyze_source_for_ide("completed-region-loan.doria", &source).unwrap();
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let member = analysis
            .info
            .class_member_surface(&ClassType::new(TRANSFER_GROUP, Vec::new()))
            .unwrap()
            .members
            .iter()
            .find(|member| member.name == "getReadRegion")
            .unwrap();
        assert_eq!(
            member.return_borrow,
            Some(ReturnBorrow {
                source: BorrowSource::Receiver,
                writable: false,
                kind: ReturnBorrowKind::Retained,
                lifetime: RetainedLifetime::LastUse,
            }),
            "{member:?}"
        );
    }
}

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
            assert!(!requirement.receiver_mode.is_writable());
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
#[test]
fn inherited_generic_member_surfaces_bind_owner_and_method_types_separately() {
    use doriac::types::{ClassType, ResolvedType};
    let source = r#"
open class Parent<T> {
    function first<U>(T $owner, U $value): T { return $owner; }
    function second<U>(T $owner, U $value): U { return $value; }
}
open class Middle<U> extends Parent<List<U>> {}
class Child<V> extends Middle<V> {}
function main(): void { let $value = new Child<string>(); }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("member-identities.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let surface = analysis
        .info
        .class_member_surface(&ClassType::new("Child", vec![ResolvedType::String]))
        .unwrap();
    let owner = ResolvedType::List(Box::new(ResolvedType::String));
    let method = ResolvedType::TypeParameter("U".to_string());
    for (name, result) in [("first", owner.clone()), ("second", method.clone())] {
        let member = surface
            .members
            .iter()
            .find(|member| member.name == name)
            .unwrap();
        let signature = member.signature.as_ref().unwrap();
        assert_eq!(signature.parameters[0].r#type, owner);
        assert_eq!(signature.parameters[1].r#type, method);
        assert_eq!(signature.return_type, result);
        assert_eq!(member.generic_parameters[0].name, "U");
    }
}

#[test]
fn conditional_member_availability_is_scoped_to_each_receiver_occurrence() {
    use doriac::semantics::composition::MemberAvailability;
    let declarations = r#"
interface Ready {}
class Yes implements Ready {}
class No {}
trait Reading<T> {
    function read(): int where (T implements Ready) { return 1; }
    int $state where (T implements Ready) { get => 1; }
}
open class Adapter<T> {
    uses Reading<T> { Reading<T>::read as inspect; }
    function probe(): int { return 0; }
}
class Child<T> extends Adapter<T> {}
"#;
    let callers = r#"
function proved<T implements Ready>(Child<T> $proved): int { return $proved->probe(); }
function unproved<T>(Child<T> $unproved): int { return $unproved->probe(); }
function yes(Child<Yes> $yes): int { return $yes->probe(); }
function no(Child<No> $no): int { return $no->probe(); }
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        let (_, analysis) =
            doriac::analyze_source_for_ide("scoped-members.doria", &source).unwrap();
        assert!(
            analysis.diagnostics.is_empty(),
            "{source}: {:?}",
            analysis.diagnostics
        );
        for (name, expected) in [
            ("proved", MemberAvailability::Proved),
            ("unproved", MemberAvailability::Unproved),
            ("yes", MemberAvailability::Proved),
            ("no", MemberAvailability::Unproved),
        ] {
            let offset = source.find(&format!("${name}->")).unwrap();
            let (span, facts) = analysis
                .info
                .class_member_availability
                .iter()
                .find(|(span, _)| span.start == offset && span.end == offset + name.len() + 1)
                .unwrap();
            for member in ["read", "inspect", "state"] {
                assert_eq!(
                    analysis
                        .info
                        .class_member_availability_at(&facts.receiver, member, *span),
                    Some(expected),
                    "{name}::{member}"
                );
            }
            assert_eq!(
                analysis
                    .info
                    .class_member_availability_at(&facts.receiver, "probe", *span),
                Some(MemberAvailability::Proved)
            );
            // Declaration inventories remain stable even when a caller cannot select a member.
            assert!(analysis
                .info
                .class_member_surface(&facts.receiver)
                .unwrap()
                .members
                .iter()
                .any(|member| member.name == "inspect"));
        }
    }
}

#[test]
fn static_member_availability_uses_the_callers_guard_without_leaking_it() {
    use doriac::semantics::composition::MemberAvailability;
    let source = r#"
interface Ready {}
class Adapter<T> {
    static function guarded(): int where (T implements Ready) { return 1; }
    static int $state where (T implements Ready) { get => 1; }
    static function probe(): int { return 0; }
    function proved(): int where (T implements Ready) { return self::probe(); }
    function unproved(): int { return self::probe(); }
}
"#;
    let (_, analysis) =
        doriac::analyze_source_for_ide("static-scoped-members.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    for (offset, expected) in source
        .match_indices("self::probe")
        .map(|(offset, _)| offset)
        .zip([MemberAvailability::Proved, MemberAvailability::Unproved])
    {
        let (span, facts) = analysis
            .info
            .class_member_availability
            .iter()
            .find(|(span, _)| span.start == offset && span.end == offset + 4)
            .unwrap();
        for name in ["guarded", "state"] {
            assert_eq!(
                analysis
                    .info
                    .class_member_availability_at(&facts.receiver, name, *span),
                Some(expected)
            );
        }
        assert_eq!(
            analysis.info.static_receiver_types.get(span),
            Some(&facts.receiver),
            "static receiver context must survive without executable admission"
        );
    }
}

#[test]
fn member_lookup_keeps_unselected_method_arguments_distinct_from_caller_names() {
    use doriac::semantics::composition::MemberAvailability;
    let source = r#"
interface Ready {}
class No {}
class Adapter<T> {
    function generic<U>(): int where (U implements Ready) { return 1; }
    function bounded<U implements Ready>(): int where (U implements Ready) { return 1; }
    function both<U>(): int where (T implements Ready, U implements Ready) { return 1; }
    function probe(): int { return 0; }
}
function inspect<U implements Ready>(Adapter<No> $value): int { return $value->probe(); }
"#;
    let (_, analysis) =
        doriac::analyze_source_for_ide("method-argument-members.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let offset = source.find("$value->").unwrap();
    let (span, facts) = analysis
        .info
        .class_member_availability
        .iter()
        .find(|(span, _)| span.start == offset && span.end == offset + 6)
        .unwrap();
    for (name, expected) in [
        ("generic", MemberAvailability::RequiresTypeArguments),
        ("bounded", MemberAvailability::Proved),
        ("both", MemberAvailability::Unproved),
    ] {
        assert_eq!(
            analysis
                .info
                .class_member_availability_at(&facts.receiver, name, *span),
            Some(expected)
        );
    }
}

#[test]
fn callable_signatures_carry_authored_defaults_and_parameter_declarations() {
    use doriac::ast::MemberAccess;
    use doriac::semantics::ConstructorParameterSemanticRole;

    let source = r#"open class Root
{
    open function value(int $input = 1): int { return $input; }
}

class Leaf extends Root
{
    override function value(int $input): int { return $input + 1; }
}

class Window
{
    function __construct(
        string $title,
        internal writable int $x = 0,
        int $width = 120,
        string $label = "Doria"
            . " TUI"
    ) {}
}
"#;
    let (program, analysis) =
        doriac::analyze_source_for_ide("defaults.doria", source).expect("source should analyze");
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );

    let constructor = method_declaration_span(&program, "Window", "__construct");
    let parameters = &analysis.info.callable_signatures[&constructor].parameters;
    let defaults = parameters
        .iter()
        .map(|parameter| parameter.default.as_deref())
        .collect::<Vec<_>>();
    // Authored text, with a line break and its indentation read as one space.
    assert_eq!(
        defaults,
        [None, Some("0"), Some("120"), Some("\"Doria\" . \" TUI\"")]
    );

    // The declaration span is the key of the compiler's constructor role.
    let x = parameters[1].declaration.expect("authored parameter");
    assert_eq!(&source[x.start..x.end], "internal writable int $x = 0");
    assert_eq!(
        analysis.info.constructor_parameters[&x].role,
        ConstructorParameterSemanticRole::Promoted {
            access: MemberAccess::Internal
        }
    );

    // An override inherits its virtual root's default, text included.
    let leaf = method_declaration_span(&program, "Leaf", "value");
    let inherited = &analysis.info.callable_signatures[&leaf].parameters[0];
    assert!(inherited.has_default);
    assert_eq!(inherited.default.as_deref(), Some("1"));
}
