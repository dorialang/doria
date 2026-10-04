use doriac::ast::{
    AssignOp, BinaryOp, ClassMember, ElseBranch, Expr, ForIncrement, ForInitializer, IncrementOp,
    IncrementPosition, InterpolatedStringPart, Item, MemberAccess, Stmt, UnaryOp,
};
use doriac::types::TypeArgumentRef;

#[test]
fn parses_stage36_property_hook_bodies_modes_effects_and_spans() {
    use doriac::ast::{ConstructorParameterRole, FunctionBody, PropertyHookKind};

    let source = r#"
open class Temperature
{
    internal writable float $celsius = 0.0;
    open writable float $fahrenheit = 32.0 {
        get throws ReadError => $this->celsius * 9.0 / 5.0 + 32.0;
        set (float $value) throws WriteError, OtherError => $this->celsius = ($value - 32.0) * 5.0 / 9.0;
    }
    override float $cached {
        writable get { $this->celsius = 1.0; return $this->celsius; }
    }
    writable Value $owned {
        get;
        set (take Value $value) { $this->owned = $value; }
    }
    writable Value $borrowed {
        set (writable Value $value) => update($value);
    }
}
"#;
    let program = doriac::parse_source("hooks.doria", source).expect("hooks should parse");
    let Item::Class(class) = &program.items[0] else {
        panic!("expected class");
    };
    let property = |index| match &class.members[index] {
        ClassMember::Property(property) => property,
        _ => panic!("expected property"),
    };
    assert!(property(0).hooks.is_empty());
    let fahrenheit = property(1);
    let authored = |span: doriac::source::Span| &source[span.start..span.end];
    assert_eq!(authored(fahrenheit.open_span.unwrap()), "open");
    assert!(fahrenheit.writable);
    assert!(fahrenheit.initializer.is_some());
    assert_eq!(fahrenheit.hooks.len(), 2);
    let get = &fahrenheit.hooks[0];
    assert_eq!(get.kind, PropertyHookKind::Get);
    assert_eq!(authored(get.keyword_span), "get");
    assert_eq!(authored(get.arrow_span.unwrap()), "=>");
    assert!(get.parameter.is_none());
    assert!(get.writable_span.is_none());
    assert!(get.borrowed_span.is_none());
    assert!(matches!(get.body.statements(), [Stmt::Return { .. }]));
    let throws = get.throws.as_ref().unwrap();
    assert_eq!(authored(throws.keyword_span), "throws");
    assert_eq!(authored(throws.entries[0].span), "ReadError");
    assert!(authored(get.body.span()).starts_with("=>"));
    assert!(authored(get.span).ends_with(';'));
    let set = &fahrenheit.hooks[1];
    assert_eq!(set.kind, PropertyHookKind::Set);
    assert!(set.borrowed_span.is_none());
    assert!(matches!(set.body.statements(), [Stmt::Assignment(_)]));
    assert_eq!(set.throws.as_ref().unwrap().entries.len(), 2);
    let parameter = set.parameter.as_ref().unwrap();
    assert_eq!(
        parameter.constructor_role,
        ConstructorParameterRole::Ordinary
    );
    assert_eq!(parameter.ty.name, "float");
    assert_eq!(authored(parameter.name_span), "$value");
    assert_eq!(authored(property(2).override_span.unwrap()), "override");
    let cached = &property(2).hooks[0];
    assert_eq!(authored(cached.writable_span.unwrap()), "writable");
    assert!(cached.arrow_span.is_none());
    assert_eq!(cached.body.statements().len(), 2);
    assert!(matches!(
        property(3).hooks[0].body,
        FunctionBody::Requirement { .. }
    ));
    assert!(property(3).hooks[1].parameter.as_ref().unwrap().take);
    assert!(property(4).hooks[0].parameter.as_ref().unwrap().writable);
    assert!(matches!(
        property(4).hooks[0].body.statements(),
        [Stmt::Expr { .. }]
    ));
}

#[test]
fn parses_stage36_interface_and_trait_hooks_without_reserving_get_or_set() {
    use doriac::ast::FunctionBody;

    let program = doriac::parse_source(
        "hooks.doria",
        r#"
interface ValueSource<T> {
    T $value { get throws ReadError; }
    writable T $editable { writable get; set (take T $value) throws WriteError; }
    function get(): T;
    writable function set(take T $value): void;
}
trait HasValue<T> {
    writable T $value { get; set (take T $value); }
    function get(): T { return $this->value; }
    writable function set(take T $value): void { $this->value = $value; }
}
function main(): void {
    let $value = $dictionary->get("key");
    $dictionary->set("key", $value);
}
"#,
    )
    .expect("hook names should remain contextual");
    let Item::Interface(interface) = &program.items[0] else {
        panic!("expected interface");
    };
    assert_eq!(interface.properties.len(), 2);
    assert_eq!(interface.requirements.len(), 2);
    assert_eq!(interface.requirements[0].name, "get");
    assert_eq!(interface.requirements[1].name, "set");
    for property in &interface.properties {
        for hook in &property.hooks {
            assert!(matches!(hook.body, FunctionBody::Requirement { .. }));
            assert!(hook.arrow_span.is_none());
        }
    }
    let Item::Trait(declaration) = &program.items[1] else {
        panic!("expected trait");
    };
    let ClassMember::Property(property) = &declaration.members[0] else {
        panic!("expected trait property");
    };
    assert_eq!(property.hooks.len(), 2);
}

#[test]
fn parses_stage36_borrowed_getters_in_all_declaration_and_body_forms() {
    use doriac::ast::{FunctionBody, PropertyHookKind};

    for owner in ["class", "trait", "interface"] {
        for head in ["borrowed get", "writable borrowed get"] {
            for effects in ["", " throws ReadError<T>"] {
                for body in [";", " => $this->value;", " { return $this->value; }"] {
                    let accessor = format!("{head}{effects}{body}");
                    let source = format!("{owner} Source<T> {{ T $value {{ {accessor} }} }}");
                    let program = doriac::parse_source("borrowed-hooks.doria", &source)
                        .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
                    let property = match &program.items[0] {
                        Item::Interface(interface) => &interface.properties[0],
                        Item::Class(class) => match &class.members[0] {
                            ClassMember::Property(property) => property,
                            _ => panic!("expected class property"),
                        },
                        Item::Trait(declaration) => match &declaration.members[0] {
                            ClassMember::Property(property) => property,
                            _ => panic!("expected trait property"),
                        },
                        _ => panic!("expected declaration"),
                    };
                    assert_eq!(property.hooks.len(), 1);
                    let hook = &property.hooks[0];
                    let authored = |span: doriac::source::Span| &source[span.start..span.end];
                    assert_eq!(hook.kind, PropertyHookKind::Get);
                    assert_eq!(authored(hook.borrowed_span.unwrap()), "borrowed");
                    assert_eq!(authored(hook.keyword_span), "get");
                    assert_eq!(authored(hook.span), accessor);
                    assert_eq!(hook.writable_span.is_some(), head.starts_with("writable"));
                    if let Some(span) = hook.writable_span {
                        assert_eq!(authored(span), "writable");
                    }
                    assert!(hook.parameter.is_none());
                    assert_eq!(hook.throws.is_some(), !effects.is_empty());
                    if let Some(throws) = &hook.throws {
                        assert_eq!(authored(throws.keyword_span), "throws");
                        assert_eq!(authored(throws.entries[0].span), "ReadError<T>");
                    }
                    assert_eq!(hook.arrow_span.is_some(), body.starts_with(" =>"));
                    if body == ";" {
                        let FunctionBody::Requirement { semicolon_span } = hook.body else {
                            panic!("expected requirement");
                        };
                        assert_eq!(authored(semicolon_span), ";");
                    } else {
                        assert!(matches!(hook.body.statements(), [Stmt::Return { .. }]));
                    }
                }
            }
        }
    }
}

#[test]
fn stage36_borrowed_remains_an_identifier_outside_accessor_heads() {
    let program = doriac::parse_source(
        "borrowed-identifiers.doria",
        r#"
function borrowed(int $value): int { return $value; }
class Example {
    int $borrowed = 1;
    function borrowed(): int { return borrowed($this->borrowed); }
}
function main(): void {
    let $borrowed = new Example();
    echo $borrowed->borrowed();
}
"#,
    )
    .expect("borrowed must remain contextual");
    let Item::Function(function) = &program.items[0] else {
        panic!("expected function");
    };
    assert_eq!(function.name, "borrowed");
    let Item::Class(class) = &program.items[1] else {
        panic!("expected class");
    };
    let ClassMember::Method(method) = &class.members[1] else {
        panic!("expected method");
    };
    assert_eq!(method.name, "borrowed");
}

#[test]
fn rejects_stage36_malformed_hook_signatures() {
    for hooks in [
        "",
        "get();",
        "set;",
        "set ();",
        "set ($value);",
        "set (int $first, int $second);",
        "set (int $value = 1);",
        "get throws => 1;",
        "get => ;",
        "get => 1; get => 2;",
        "set (int $value); set (int $value);",
        "set (writable writable int $value);",
        "borrowed;",
        "borrowed get();",
        "borrowed set (int $value);",
        "writable borrowed set (int $value);",
        "borrowed borrowed get;",
        "borrowed writable get;",
        "get; borrowed get;",
        "borrowed get; writable get;",
        "borrow get;",
        "writable borrow get;",
        "borrowable get;",
        "lendable get;",
    ] {
        let source = format!("class Value {{ writable int $value {{ {hooks} }} }}");
        let diagnostics = doriac::parse_source("hooks.doria", &source)
            .expect_err("malformed hook syntax must be diagnosed");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "P0001"),
            "missing parser diagnostic for {hooks}: {diagnostics:?}"
        );
    }
}

#[test]
fn stage36_hook_recovery_preserves_following_hooks_and_class_members() {
    let source = r#"
class Value {
    writable int $value {
        set ($value) { if (true) { echo "nested"; } }
        get => 1;
        get => 2;
    }
    open int $stored;
}
"#;
    let diagnostics = doriac::parse_source("hooks.doria", source)
        .expect_err("untyped setter, duplicate getter, and open stored field are invalid");
    assert_eq!(diagnostics.len(), 3, "{diagnostics:?}");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.message == "a property cannot repeat the same hook" }));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.title == "Property Cannot Have A Method Modifier" }));
}

#[test]
fn stage36_borrowed_hook_recovery_preserves_modifiers_and_following_members() {
    let source = r#"
class Value {
    writable int $value {
        get() { if (true) { echo "nested"; } }
        borrowed set (int $value) { $this->value = $value; }
        borrowed get => 1;
        writable borrowed get => 2;
    }
    open int $stored;
}
"#;
    let diagnostics = doriac::parse_source("borrowed-recovery.doria", source)
        .expect_err("malformed getter, borrowed setter, duplicate getter and stored modifier");
    assert_eq!(diagnostics.len(), 4, "{diagnostics:?}");
    for message in [
        "a getter has no parameter list",
        "`borrowed` is only allowed on a getter",
        "a property cannot repeat the same hook",
        "`open` and `override` require a method or a hooked property",
    ] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message == message),
            "missing {message}: {diagnostics:?}"
        );
    }
}

#[test]
fn stage36_hook_transform_preserves_all_types_and_source_locations() {
    use doriac::ast::transform::{Transform, Transformable};
    use doriac::source::{SourceId, Span};
    use doriac::types::TypeRef;

    struct Substitute;
    impl Transform for Substitute {
        fn type_ref(&mut self, ty: &mut TypeRef) {
            ty.walk(self);
            if ty.name == "T" {
                ty.name = "Value".into();
            }
        }
        fn span(&mut self, span: &mut Span) {
            span.source = SourceId(7);
            span.start += 100;
            span.end += 100;
        }
    }
    let mut program = doriac::parse_source(
        "hooks.doria",
        "interface Source<T> { writable T $value { writable borrowed get throws Failure<T> => new T(); set (take T $value) throws Failure<T>; } }",
    )
    .expect("source shapes parse before interface body checking");
    let Item::Interface(interface) = &mut program.items[0] else {
        panic!("expected interface");
    };
    let old_property = interface.properties[0].clone();
    interface.transform(&mut Substitute);
    let property = &interface.properties[0];
    assert_eq!(property.ty.name, "Value");
    assert_eq!(property.name_span.source, SourceId(7));
    assert_eq!(property.name_span.start, old_property.name_span.start + 100);
    let get = &property.hooks[0];
    assert_eq!(get.arrow_span.unwrap().source, SourceId(7));
    assert_eq!(get.borrowed_span.unwrap().source, SourceId(7));
    assert_eq!(
        get.borrowed_span.unwrap().start,
        old_property.hooks[0].borrowed_span.unwrap().start + 100
    );
    assert_eq!(
        get.borrowed_span.unwrap().end,
        old_property.hooks[0].borrowed_span.unwrap().end + 100
    );
    assert_eq!(get.writable_span.unwrap().source, SourceId(7));
    assert_eq!(
        get.keyword_span.start,
        old_property.hooks[0].keyword_span.start + 100
    );
    let effect = &get.throws.as_ref().unwrap().entries[0];
    assert_eq!(effect.span.source, SourceId(7));
    let TypeArgumentRef::Type(argument) = &effect.ty.arguments[0] else {
        panic!("expected type argument");
    };
    assert_eq!(argument.name, "Value");
    let Stmt::Return {
        expr: Some(Expr::New {
            class_type, span, ..
        }),
        ..
    } = &get.body.statements()[0]
    else {
        panic!("expected normalized getter return");
    };
    assert_eq!(class_type.name, "Value");
    assert_eq!(span.source, SourceId(7));
    let set = &property.hooks[1];
    assert_eq!(set.parameter.as_ref().unwrap().ty.name, "Value");
    assert_eq!(
        set.parameter.as_ref().unwrap().type_span.source,
        SourceId(7)
    );
    assert_eq!(set.body.span().source, SourceId(7));
}

#[test]
fn stage36_property_expression_visitor_includes_hook_bodies_in_source_order() {
    let program = doriac::parse_source(
        "hooks.doria",
        "class Value { writable int $value = 1 { get => 2; set (int $value) => $this->value = 3; } }",
    )
    .expect("initialized hook property should parse");
    let Item::Class(class) = &program.items[0] else {
        panic!("expected class");
    };
    let ClassMember::Property(property) = &class.members[0] else {
        panic!("expected property");
    };
    let mut values = Vec::new();
    doriac::ast::visit::property(property, &mut |expr| {
        if let Expr::Int { value, .. } = expr {
            values.push(value.clone());
        }
    });
    assert_eq!(values, ["1", "2", "3"]);
}

#[test]
fn parses_stage29_checked_error_declarations_and_control_flow() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
class LoadError implements Error
{
    function __construct(string $message)
        throws LoadError
    {
        throw new LoadError($message);
    }

    static function create(): LoadError throws LoadError
    {
        return new LoadError("create");
    }
}

function load(): string throws LoadError, Error
{
    try {
        throw new LoadError("load");
    } catch (LoadError $error) {
        throw $error;
    } catch (Error) {
        throw new LoadError("other");
    } finally {
        try {
            echo "cleanup";
        } finally {
            echo "done";
        }
    }
}

function main(): void throws Error
{
    load();
}
"#,
    )
    .expect("Stage 29 checked-error syntax should parse");

    let Item::Class(class) = &program.items[0] else {
        panic!("expected class declaration");
    };
    let ClassMember::Method(constructor) = &class.members[0] else {
        panic!("expected constructor");
    };
    assert_eq!(constructor.throws.as_ref().unwrap().entries.len(), 1);
    let ClassMember::Method(static_method) = &class.members[1] else {
        panic!("expected static method");
    };
    assert!(static_method.is_static);
    assert!(static_method.throws.is_some());

    let Item::Function(load) = &program.items[1] else {
        panic!("expected load function");
    };
    assert_eq!(load.throws.as_ref().unwrap().entries.len(), 2);
    let Stmt::Try(try_statement) = &load.body.statements()[0] else {
        panic!("expected try statement");
    };
    assert_eq!(try_statement.catches.len(), 2);
    assert!(try_statement.catches[0].binding.is_some());
    assert!(try_statement.catches[1].binding.is_none());
    assert!(try_statement.finally.is_some());

    let Item::Function(main) = &program.items[2] else {
        panic!("expected main function");
    };
    assert!(main.throws.is_some());
}

#[test]
fn rejects_malformed_stage29_checked_error_syntax() {
    for (source, code) in [
        ("function f(): void { throw; }", "E0624"),
        ("function f(): void { try {} }", "E0625"),
        ("function f(): void { return throw value; }", "E0636"),
        (
            "function f(): void { try {} finally {} catch (Error) {} }",
            "E0637",
        ),
    ] {
        let diagnostics =
            doriac::parse_source("test.doria", source).expect_err("source should be rejected");
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic.code == code),
            "expected {code}, got {diagnostics:?}"
        );
    }

    doriac::parse_source("test.doria", "function f(): void throws { }")
        .expect_err("an empty throws clause should be rejected");
    doriac::parse_source("test.doria", "function f(): void { try {} catch () {} }")
        .expect_err("a catch without a type should be rejected");
}

#[test]
fn parses_standalone_lexical_blocks_as_statements() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
function main(): void
{
    {
        let $value = 1;
        {
            echo "{$value}";
        }
    }
}
"#,
    )
    .expect("standalone lexical blocks should parse as statements");

    let Item::Function(function) = &program.items[0] else {
        panic!("expected function declaration");
    };
    let Stmt::Block(outer) = &function.body.statements()[0] else {
        panic!("expected outer standalone block");
    };
    assert!(matches!(outer.statements[1], Stmt::Block(_)));
}

#[test]
fn parses_class_workflow_and_qualified_type_syntax_before_semantics_land() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
namespace Vendor\App;

class Child extends Vendor\Base implements Vendor\Contracts\Printable
{
    function convert(Vendor\Input $input): Vendor\Output
    {
        return new Vendor\Output();
    }
}
"#,
    )
    .expect("accepted namespace and inheritance syntax should parse");

    assert_eq!(
        program
            .namespace
            .as_ref()
            .map(|namespace| namespace.name.canonical()),
        Some("Vendor\\App".to_string())
    );
    let Item::Class(class) = &program.items[0] else {
        panic!("expected class declaration");
    };
    assert_eq!(
        class.parent.as_ref().map(|parent| parent.name.as_str()),
        Some("Vendor\\Base")
    );
    assert_eq!(
        class
            .implements
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["Vendor\\Contracts\\Printable"]
    );
    let ClassMember::Method(method) = &class.members[0] else {
        panic!("expected method declaration");
    };
    assert_eq!(method.params[0].ty.name, "Vendor\\Input");
    assert_eq!(
        method.return_type.as_ref().map(|ty| ty.name.as_str()),
        Some("Vendor\\Output")
    );
}

#[test]
fn parses_interface_declarations_before_semantics_land() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
interface Printable
{
    function render(): string;
}
"#,
    )
    .expect("accepted interface syntax should parse");

    let Item::Interface(interface) = &program.items[0] else {
        panic!("expected interface declaration");
    };
    assert_eq!(interface.name, "Printable");
}

#[test]
fn parses_variable_declarations() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
let $x = 5;
let writable $name = "Doria";
writable int $score = 1;
null $empty = null;
int[] $numbers = [1, 2, 3];
"#,
    )
    .expect("parse should succeed");

    assert_eq!(program.items.len(), 5);
    assert!(matches!(
        &program.items[0],
        Item::Statement(Stmt::VarDecl(decl))
            if !decl.writable && decl.bindings.len() == 1 && decl.bindings[0].name == "x"
    ));
    assert!(matches!(
        &program.items[1],
        Item::Statement(Stmt::VarDecl(decl))
            if decl.writable && decl.bindings.len() == 1 && decl.bindings[0].name == "name"
    ));
    assert!(matches!(
        &program.items[2],
        Item::Statement(Stmt::VarDecl(decl)) if decl.writable && decl.ty.is_some()
    ));
    assert!(matches!(
        &program.items[3],
        Item::Statement(Stmt::VarDecl(decl))
            if !decl.writable
                && decl.bindings.len() == 1
                && decl.bindings[0].name == "empty"
                && matches!(decl.ty.as_ref(), Some(ty) if ty.name == "null")
    ));
    assert!(matches!(
        &program.items[4],
        Item::Statement(Stmt::VarDecl(decl))
            if !decl.writable
                && decl.bindings.len() == 1
                && decl.bindings[0].name == "numbers"
                && matches!(decl.ty.as_ref(), Some(ty)
                    if ty.name == "[]"
                        && ty.type_argument_count() == 1
                        && ty.type_argument(0).is_some_and(|argument| argument.name == "int"))
    ));
}

#[test]
fn parses_sequence_fill_literal_without_confusing_element_lists() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
bool[] $flags = [true; count()];
List<int> $values = [1, 2];
"#,
    )
    .expect("both bracket literal forms should parse");

    let Item::Statement(Stmt::VarDecl(fill)) = &program.items[0] else {
        panic!("expected fill declaration");
    };
    assert!(matches!(
        &fill.initializer,
        Expr::ArrayRepeat { value, count, .. }
            if matches!(value.as_ref(), Expr::Bool { value: true, .. })
                && matches!(count.as_ref(), Expr::FunctionCall { name, .. } if name == "count")
    ));

    let Item::Statement(Stmt::VarDecl(elements)) = &program.items[1] else {
        panic!("expected element-list declaration");
    };
    assert!(matches!(
        &elements.initializer,
        Expr::Array { elements, .. } if elements.len() == 2
    ));
}

#[test]
fn rejects_php_parameter_references_without_stealing_bitwise_syntax() {
    let diagnostics = doriac::parse_source("test.doria", "function mutate(int &$value): void {}")
        .expect_err("PHP-style parameter references should be rejected contextually");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("does not support PHP-style parameter references")
    }));
}

#[test]
fn parses_stage_13_primitive_type_spellings() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
int8 $int8Value = 0;
int16 $int16Value = 0;
int32 $int32Value = 0;
int64 $int64Value = 0;
uint8 $uint8Value = 0;
uint16 $uint16Value = 0;
uint32 $uint32Value = 0;
uint64 $uint64Value = 0;
float32 $float32Value = 0.0;
float64 $float64Value = 0.0;
"#,
    )
    .expect("parse should succeed");

    let names = program
        .items
        .iter()
        .map(|item| {
            let Item::Statement(Stmt::VarDecl(decl)) = item else {
                panic!("expected variable declaration");
            };
            decl.ty
                .as_ref()
                .expect("expected explicit type")
                .name
                .as_str()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        [
            "int8", "int16", "int32", "int64", "uint8", "uint16", "uint32", "uint64", "float32",
            "float64",
        ]
    );
}

#[test]
fn parses_adjacent_nested_generic_type_closers_after_shift_tokens_are_added() {
    let program = doriac::parse_source(
        "test.doria",
        "Dictionary<string, List<uint64>> $values = [];",
    )
    .expect("nested generic type should parse without whitespace between closing angles");

    let Item::Statement(Stmt::VarDecl(decl)) = &program.items[0] else {
        panic!("expected variable declaration");
    };
    let ty = decl.ty.as_ref().expect("expected explicit type");
    assert_eq!(ty.name, "Dictionary");
    let list = ty.type_argument(1).expect("second type argument");
    assert_eq!(list.name, "List");
    assert_eq!(
        list.type_argument(0).expect("list element type").name,
        "uint64"
    );
}

#[test]
fn preserves_mixed_generic_argument_kinds_in_source_order() {
    let program = doriac::parse_source(
        "test.doria",
        "function consume(Buffer<4096, float32> $buffer): void {}",
    )
    .expect("reserved value arguments should parse");
    let Item::Function(function) = &program.items[0] else {
        panic!("expected function declaration");
    };
    let ty = &function.params[0].ty;
    assert_eq!(ty.to_string(), "Buffer<4096, float32>");
    assert!(matches!(
        &ty.arguments[0],
        TypeArgumentRef::Value(value) if value == "4096"
    ));
    assert!(matches!(
        &ty.arguments[1],
        TypeArgumentRef::Type(argument) if argument.name == "float32"
    ));
}

fn parse_echo_expr(source: &str) -> Expr {
    let program = doriac::parse_source("test.doria", source).expect("parse should succeed");
    let Item::Statement(Stmt::Echo { expr, .. }) = &program.items[0] else {
        panic!("expected echo statement");
    };
    expr.clone()
}

#[test]
fn parses_boolean_word_operators() {
    assert!(matches!(
        parse_echo_expr("echo true and false;"),
        Expr::Binary {
            op: BinaryOp::And,
            ..
        }
    ));
    assert!(matches!(
        parse_echo_expr("echo false or true;"),
        Expr::Binary {
            op: BinaryOp::Or,
            ..
        }
    ));
    assert!(matches!(
        parse_echo_expr("echo true xor false;"),
        Expr::Binary {
            op: BinaryOp::Xor,
            ..
        }
    ));
    assert!(matches!(
        parse_echo_expr("echo not false;"),
        Expr::Unary {
            op: UnaryOp::Not,
            ..
        }
    ));
}

#[test]
fn parses_stage_13_unary_and_binary_operators() {
    for (source, expected) in [
        ("echo -$value;", UnaryOp::Negate),
        ("echo ~$value;", UnaryOp::BitwiseNot),
    ] {
        assert!(matches!(
            parse_echo_expr(source),
            Expr::Unary { op, .. } if op == expected
        ));
    }

    for (source, expected) in [
        ("echo $a / $b;", BinaryOp::Div),
        ("echo $a % $b;", BinaryOp::Mod),
        ("echo $a << $b;", BinaryOp::ShiftLeft),
        ("echo $a >> $b;", BinaryOp::ShiftRight),
        ("echo $a & $b;", BinaryOp::BitwiseAnd),
        ("echo $a ^ $b;", BinaryOp::BitwiseXor),
        ("echo $a | $b;", BinaryOp::BitwiseOr),
    ] {
        assert!(matches!(
            parse_echo_expr(source),
            Expr::Binary { op, .. } if op == expected
        ));
    }
}

#[test]
fn parses_shift_below_additive_precedence() {
    let Expr::Binary {
        left,
        op: BinaryOp::ShiftLeft,
        right,
        ..
    } = parse_echo_expr("echo 1 + 2 << 1;")
    else {
        panic!("expected outer shift-left expression");
    };

    assert!(matches!(
        left.as_ref(),
        Expr::Binary {
            op: BinaryOp::Add,
            ..
        }
    ));
    assert!(matches!(right.as_ref(), Expr::Int { value, .. } if value == "1"));
}

#[test]
fn parses_equality_before_bitwise_and() {
    let Expr::Binary {
        left,
        op: BinaryOp::BitwiseAnd,
        right,
        ..
    } = parse_echo_expr("echo 1 & 2 == 0;")
    else {
        panic!("expected outer bitwise-and expression");
    };

    assert!(matches!(left.as_ref(), Expr::Int { value, .. } if value == "1"));
    assert!(matches!(
        right.as_ref(),
        Expr::Binary {
            op: BinaryOp::Equal,
            ..
        }
    ));
}

#[test]
fn keeps_bitwise_and_boolean_xor_distinct() {
    assert!(matches!(
        parse_echo_expr("echo $a ^ $b;"),
        Expr::Binary {
            op: BinaryOp::BitwiseXor,
            ..
        }
    ));
    assert!(matches!(
        parse_echo_expr("echo $a xor $b;"),
        Expr::Binary {
            op: BinaryOp::Xor,
            ..
        }
    ));
}

#[test]
fn parses_all_stage_13_compound_assignments() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
$value += 1;
$value -= 1;
$value *= 1;
$value /= 1;
$value %= 1;
$value <<= 1;
$value >>= 1;
$value &= 1;
$value |= 1;
$value ^= 1;
"#,
    )
    .expect("compound assignments should parse");

    let expected = [
        AssignOp::AddAssign,
        AssignOp::SubAssign,
        AssignOp::MulAssign,
        AssignOp::DivAssign,
        AssignOp::ModAssign,
        AssignOp::ShiftLeftAssign,
        AssignOp::ShiftRightAssign,
        AssignOp::BitwiseAndAssign,
        AssignOp::BitwiseOrAssign,
        AssignOp::BitwiseXorAssign,
    ];

    for (item, expected) in program.items.iter().zip(expected) {
        assert!(matches!(
            item,
            Item::Statement(Stmt::Assignment(assignment)) if assignment.op == expected
        ));
    }
}

#[test]
fn structural_lowering_preserves_stage_13_operator_variants() {
    let ast = doriac::parse_source(
        "test.doria",
        "let writable $value = 2; let $mask = 1; let $other = 0; $value <<= 1; echo ~-$value | $mask ^ $other & 1;",
    )
    .expect("parse should succeed");
    let hir = doriac::lowering::lower_program_with_semantics(&ast, Default::default())
        .expect("operator AST should lower structurally without semantic facts");

    assert!(matches!(
        &hir.items[3],
        doriac::hir::Item::Statement(doriac::hir::Stmt::Assignment(assignment))
            if assignment.op == AssignOp::ShiftLeftAssign
    ));
    assert!(matches!(
        &hir.items[4],
        doriac::hir::Item::Statement(doriac::hir::Stmt::Echo {
            expr: doriac::hir::Expr::Binary {
                op: BinaryOp::BitwiseOr,
                ..
            },
            ..
        })
    ));
}

#[test]
fn direct_lowering_keeps_interfaces_in_contract_facts_and_ordering_executable() {
    let ast = doriac::parse_source("test.doria", "interface Printable {}")
        .expect("accepted interface declaration should parse");
    let hir =
        doriac::lowering::lower_program(&ast).expect("declarations are compile-time material");
    assert!(matches!(
        hir.items.as_slice(),
        [doriac::hir::Item::Enum(declaration)]
            if declaration.name == "Ordering"
                && declaration.span.source == doriac::compiler_known_contracts::SOURCE_ID
    ));
    assert_eq!(hir.semantic_info.contracts.interfaces[0].name, "Printable");
}

#[test]
fn parses_string_concat_operator() {
    let expr = parse_echo_expr(r#"echo "Hello " . $name . "!";"#);
    let Expr::Binary {
        left,
        op: BinaryOp::Concat,
        right,
        ..
    } = expr
    else {
        panic!("expected outer concat expression");
    };

    assert!(matches!(right.as_ref(), Expr::String { value, .. } if value == "!"));
    let Expr::Binary {
        left: inner_left,
        op: BinaryOp::Concat,
        right: inner_right,
        ..
    } = left.as_ref()
    else {
        panic!("expected left-associative inner concat expression");
    };
    assert!(matches!(inner_left.as_ref(), Expr::String { value, .. } if value == "Hello "));
    assert!(matches!(inner_right.as_ref(), Expr::Variable { name, .. } if name == "name"));
}

#[test]
fn rejects_ambiguous_xor_expressions() {
    for source in [
        "echo true xor false xor true;",
        "echo true and false xor true;",
        "echo true xor false or true;",
    ] {
        let err = doriac::parse_source("test.doria", source)
            .expect_err("ambiguous xor expression should be rejected");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("ambiguous `xor`")),
            "expected ambiguous xor diagnostic, got {err:?}"
        );
    }
}

#[test]
fn accepts_parenthesized_xor_expressions() {
    for source in [
        "echo (true xor false) xor true;",
        "echo (true and false) xor true;",
        "echo true xor (false or true);",
    ] {
        doriac::parse_source("test.doria", source)
            .unwrap_or_else(|err| panic!("parenthesized xor expression should parse: {err:?}"));
    }
}

#[test]
fn parses_plain_and_interpolated_strings() {
    assert!(matches!(
        parse_echo_expr("echo '{$name}';"),
        Expr::String { value, .. } if value == "{$name}"
    ));
    assert!(matches!(
        parse_echo_expr("echo \"Hello\";"),
        Expr::String { value, .. } if value == "Hello"
    ));
    assert!(matches!(
        parse_echo_expr("echo \"\\{}\";"),
        Expr::String { value, .. } if value == "{}"
    ));

    let Expr::InterpolatedString { parts, .. } = parse_echo_expr("echo \"Hello, {$name}\";") else {
        panic!("expected interpolated string");
    };
    assert!(matches!(
        &parts[0],
        InterpolatedStringPart::Text { value, span }
            if value == "Hello, " && *span == doriac::source::Span::new(6, 13)
    ));
    assert!(matches!(
        &parts[1],
        InterpolatedStringPart::Expr(Expr::Variable { name, .. }) if name == "name"
    ));

    let Expr::InterpolatedString { parts, .. } = parse_echo_expr("echo \"Hello, {$this->name}\";")
    else {
        panic!("expected interpolated string");
    };
    assert!(matches!(
        &parts[1],
        InterpolatedStringPart::Expr(Expr::PropertyAccess { object, property, .. })
            if matches!(object.as_ref(), Expr::This { .. }) && property == "name"
    ));

    let Expr::InterpolatedString { parts, .. } = parse_echo_expr("echo \"{$first} {$last}\";")
    else {
        panic!("expected interpolated string");
    };
    assert_eq!(parts.len(), 3);
    assert!(matches!(
        &parts[0],
        InterpolatedStringPart::Expr(Expr::Variable { name, .. }) if name == "first"
    ));
    assert!(matches!(
        &parts[1],
        InterpolatedStringPart::Text { value, .. } if value == " "
    ));
    assert!(matches!(
        &parts[2],
        InterpolatedStringPart::Expr(Expr::Variable { name, .. }) if name == "last"
    ));
}

#[test]
fn parses_full_expressions_inside_interpolated_strings() {
    for (source, expected) in [
        ("echo \"{$a + $b}\";", "binary"),
        ("echo \"{formatValue($value)}\";", "function call"),
        ("echo \"{($a + $b) * 2}\";", "grouped expression"),
        ("echo \"{Counter::next()}\";", "static call"),
        ("echo \"{true}\";", "boolean"),
        ("echo \"{$a < $b}\";", "comparison"),
    ] {
        let Expr::InterpolatedString { parts, .. } = parse_echo_expr(source) else {
            panic!("expected an interpolated string for {expected}");
        };
        assert!(
            matches!(parts.as_slice(), [InterpolatedStringPart::Expr(_)]),
            "expected one {expected} interpolation, got {parts:?}"
        );
    }

    let Expr::InterpolatedString { parts, .. } =
        parse_echo_expr("echo \"{formatValue(\"left\")} {formatValue('right')}\";")
    else {
        panic!("expected interpolated string with nested quoted arguments");
    };
    assert_eq!(parts.len(), 3);

    let Expr::InterpolatedString { parts, .. } =
        parse_echo_expr("echo \"{formatValue(1 /* } */ + 2)}\";")
    else {
        panic!("expected interpolation with an ordinary expression comment");
    };
    assert!(matches!(
        parts.as_slice(),
        [InterpolatedStringPart::Expr(Expr::FunctionCall { .. })]
    ));

    let Expr::InterpolatedString { parts, .. } = parse_echo_expr(r#"echo "{$first}{$second}";"#)
    else {
        panic!("expected adjacent interpolation parts");
    };
    assert_eq!(parts.len(), 2);
}

#[test]
fn applies_the_stage_18_literal_brace_rule() {
    assert!(matches!(
        parse_echo_expr(r#"echo "\{literal}";"#),
        Expr::String { value, .. } if value == "{literal}"
    ));
    assert!(matches!(
        parse_echo_expr(r#"echo "right } and escaped \}";"#),
        Expr::String { value, .. } if value == "right } and escaped }"
    ));

    let Expr::InterpolatedString { parts, .. } = parse_echo_expr(r#"echo "\{{$value}}";"#) else {
        panic!("expected escaped brace adjacent to interpolation");
    };
    assert!(matches!(
        parts.as_slice(),
        [
            InterpolatedStringPart::Text { value: open, .. },
            InterpolatedStringPart::Expr(Expr::Variable { name, .. }),
            InterpolatedStringPart::Text { value: close, .. },
        ] if open == "{" && name == "value" && close == "}"
    ));

    let Expr::InterpolatedString { parts, .. } = parse_echo_expr(r#"echo "{formatValue("\{")}";"#)
    else {
        panic!("expected interpolation containing an escaped brace in an inner string");
    };
    assert!(matches!(
        parts.as_slice(),
        [InterpolatedStringPart::Expr(Expr::FunctionCall { name, .. })]
            if name == "formatValue"
    ));
}

#[test]
fn rejects_malformed_string_interpolation() {
    for (source, message) in [
        (
            "echo \"Hello, {$name\";",
            "unterminated string interpolation",
        ),
        ("echo \"Hello, {}\";", "empty string interpolation"),
        ("echo \"Hello, {$}\";", "expected variable name after `$`"),
        ("echo \"{/* comment */}\";", "expected expression"),
        ("echo \"{// comment\n}\";", "expected expression"),
        ("echo \"A literal {word}\";", "unescaped `{`"),
        ("echo \"{foo-bar}\";", "unescaped `{`"),
        ("echo \"{word . suffix}\";", "unescaped `{`"),
        ("echo \"{{word}}\";", "unescaped `{`"),
    ] {
        let err = doriac::parse_source("test.doria", source)
            .expect_err("parse should reject malformed interpolation");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains(message)),
            "expected diagnostic containing {message}, got {err:?}"
        );
    }
}

#[test]
fn rejects_truncated_collections_without_recursive_eof_parsing() {
    for source in [
        "[",
        "f.unction ma { ec ;void { ec ; [\n",
        "function mainfuncti(",
    ] {
        let diagnostics = doriac::parse_source("fuzz-regression.doria", source)
            .expect_err("truncated input must produce a diagnostic");
        assert!(!diagnostics.is_empty());
    }
}

#[test]
fn identifier_composite_interpolations_receive_the_literal_brace_fix() {
    for source in ["echo \"{foo-bar}\";", "echo \"{word . suffix}\";"] {
        let diagnostics = doriac::parse_source("test.doria", source)
            .expect_err("bare identifier composites must not become interpolation expressions");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "P0002")
            .unwrap_or_else(|| panic!("expected P0002 for {source}, got {diagnostics:?}"));
        let fix = diagnostic
            .fix
            .as_ref()
            .expect("P0002 should carry a machine-applicable fix");
        assert_eq!(fix.replacement, "\\{");
        assert_eq!(fix.span.start, source.find('{').expect("opening brace"));
    }
}

#[test]
fn keeps_interpolation_diagnostics_on_original_source_offsets() {
    let source = "echo \"prefix {1 + } suffix\";";
    let diagnostics = doriac::parse_source("test.doria", source)
        .expect_err("malformed interpolation expression should fail");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("expected expression"))
        .expect("ordinary parser diagnostic should be preserved");
    assert_eq!(diagnostic.span.start, source.find('+').expect("operator"));

    let source = "echo \"literal {word}\";";
    let diagnostics = doriac::parse_source("test.doria", source)
        .expect_err("bare literal opening brace should fail");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "P0002")
        .expect("literal brace diagnostic should be present");
    let fix = diagnostic
        .fix
        .as_ref()
        .expect("diagnostic should carry a fix");
    assert_eq!(fix.span, diagnostic.span);
    assert_eq!(fix.replacement, "\\{");

    let diagnostics = doriac::parse_source("test.doria", "echo \"{1 + }\"; echo \"{2 + }\";")
        .expect_err("each malformed interpolation should be diagnosed");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message == "expected expression")
            .count(),
        2
    );
}
#[test]
fn parses_if_else_and_while_control_flow() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
if (true) {
    echo "yes";
}

if ($age < 13) {
    echo "child";
} else if ($age < 20) {
    echo "teen";
} else {
    echo "adult";
}

while ($count < 10) {
    $count += 1;
}
"#,
    )
    .expect("parse should succeed");

    let Item::Statement(Stmt::If(simple_if)) = &program.items[0] else {
        panic!("expected if statement");
    };
    assert!(matches!(
        simple_if.condition,
        Expr::Bool { value: true, .. }
    ));
    assert_eq!(simple_if.then_block.statements.len(), 1);
    assert!(simple_if.else_branch.is_none());

    let Item::Statement(Stmt::If(if_stmt)) = &program.items[1] else {
        panic!("expected if statement");
    };
    let Some(ElseBranch::If(else_if)) = &if_stmt.else_branch else {
        panic!("expected else-if branch");
    };
    assert!(matches!(else_if.condition, Expr::Binary { .. }));
    assert!(matches!(else_if.else_branch, Some(ElseBranch::Block(_))));

    let Item::Statement(Stmt::While(while_stmt)) = &program.items[2] else {
        panic!("expected while statement");
    };
    assert!(matches!(while_stmt.condition, Expr::Binary { .. }));
    assert_eq!(while_stmt.body.statements.len(), 1);
}

#[test]
fn parses_break_and_continue_statements() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
while (true) {
    break;
}

while (true) {
    continue;
}
"#,
    )
    .expect("parse should succeed");

    let Item::Statement(Stmt::While(break_loop)) = &program.items[0] else {
        panic!("expected break loop");
    };
    assert!(matches!(
        break_loop.body.statements.as_slice(),
        [Stmt::Break { .. }]
    ));

    let Item::Statement(Stmt::While(continue_loop)) = &program.items[1] else {
        panic!("expected continue loop");
    };
    assert!(matches!(
        continue_loop.body.statements.as_slice(),
        [Stmt::Continue { .. }]
    ));
}

#[test]
fn rejects_numeric_or_labeled_loop_control() {
    for (source, message) in [
        (
            "while (true) { break 2; }",
            "`break` does not accept a value or label in this Doria slice",
        ),
        (
            "while (true) { continue 2; }",
            "`continue` does not accept a value or label in this Doria slice",
        ),
        (
            "while (true) { break outer; }",
            "`break` does not accept a value or label in this Doria slice",
        ),
        (
            "while (true) { continue outer; }",
            "`continue` does not accept a value or label in this Doria slice",
        ),
    ] {
        let err = doriac::parse_source("test.doria", source)
            .expect_err("numeric or labeled loop control should be rejected");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains(message)),
            "expected diagnostic containing {message}, got {err:?}"
        );
    }
}

#[test]
fn parses_stage_9_for_loops_and_mutation_statements() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
for (let writable $i = 0; $i < 10; $i++) {
    echo $i;
}

for (let writable $i = 0; $i < 10; ++$i) {
}

for (let writable $i = 10; $i > 0; $i--) {
}

$i++;
++$i;
$i--;
--$i;
"#,
    )
    .expect("parse should succeed");

    let Item::Statement(Stmt::For(first_for)) = &program.items[0] else {
        panic!("expected for loop");
    };
    assert!(matches!(
        &first_for.initializer,
        Some(ForInitializer::VarDecl(decl))
            if decl.writable && decl.bindings.len() == 1 && decl.bindings[0].name == "i"
    ));
    assert!(matches!(first_for.condition, Some(Expr::Binary { .. })));
    assert!(matches!(
        &first_for.increment,
        Some(ForIncrement::Increment(increment))
            if increment.op == IncrementOp::Increment
                && increment.position == IncrementPosition::Post
    ));

    let Item::Statement(Stmt::For(second_for)) = &program.items[1] else {
        panic!("expected for loop");
    };
    assert!(matches!(
        &second_for.increment,
        Some(ForIncrement::Increment(increment))
            if increment.op == IncrementOp::Increment
                && increment.position == IncrementPosition::Pre
    ));

    let Item::Statement(Stmt::For(third_for)) = &program.items[2] else {
        panic!("expected for loop");
    };
    assert!(matches!(
        &third_for.increment,
        Some(ForIncrement::Increment(increment))
            if increment.op == IncrementOp::Decrement
                && increment.position == IncrementPosition::Post
    ));

    assert!(matches!(
        &program.items[3],
        Item::Statement(Stmt::Increment(increment))
            if increment.op == IncrementOp::Increment
                && increment.position == IncrementPosition::Post
    ));
    assert!(matches!(
        &program.items[4],
        Item::Statement(Stmt::Increment(increment))
            if increment.op == IncrementOp::Increment
                && increment.position == IncrementPosition::Pre
    ));
    assert!(matches!(
        &program.items[5],
        Item::Statement(Stmt::Increment(increment))
            if increment.op == IncrementOp::Decrement
                && increment.position == IncrementPosition::Post
    ));
    assert!(matches!(
        &program.items[6],
        Item::Statement(Stmt::Increment(increment))
            if increment.op == IncrementOp::Decrement
                && increment.position == IncrementPosition::Pre
    ));
}

#[test]
fn parses_stage_9_foreach_ranges() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
foreach (0..10 as $i) {
}

foreach (0..<10 as $i) {
}
"#,
    )
    .expect("parse should succeed");

    let Item::Statement(Stmt::Foreach(inclusive)) = &program.items[0] else {
        panic!("expected foreach");
    };
    assert!(matches!(
        inclusive.iterable,
        Expr::Range {
            inclusive: true,
            ..
        }
    ));

    let Item::Statement(Stmt::Foreach(exclusive)) = &program.items[1] else {
        panic!("expected foreach");
    };
    assert!(matches!(
        exclusive.iterable,
        Expr::Range {
            inclusive: false,
            ..
        }
    ));
}

#[test]
fn rejects_value_producing_increment_expressions() {
    for source in ["let $x = $i++;", "let $x = ++$i;"] {
        doriac::parse_source("test.doria", source)
            .expect_err("value-producing increment should not parse in Stage 9");
    }
}

#[test]
fn parses_class_with_writable_method() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
class Person
{
    writable string $name;

    writable function rename(string $name): void
    {
        $this->name = $name;
    }
}
"#,
    )
    .expect("parse should succeed");

    assert!(matches!(&program.items[0], Item::Class(class_decl) if class_decl.name == "Person"));
}

#[test]
fn parses_default_external_and_internal_members() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
class Parser
{
    string $name;
    internal string $slug;
    internal writable int $position = 0;

    function parse(): Ast
    {
        return $this->parseProgram();
    }

    internal function parseProgram(): Ast
    {
        return new Ast();
    }

    internal writable function advance(): void
    {
        $this->position = $this->position + 1;
    }
}
"#,
    )
    .expect("parse should succeed");

    let Item::Class(class_decl) = &program.items[0] else {
        panic!("expected class");
    };

    assert!(matches!(
        &class_decl.members[0],
        ClassMember::Property(property)
            if property.name == "name"
                && property.access == MemberAccess::External
                && !property.writable
    ));
    assert!(matches!(
        &class_decl.members[1],
        ClassMember::Property(property)
            if property.name == "slug"
                && property.access == MemberAccess::Internal
                && !property.writable
    ));
    assert!(matches!(
        &class_decl.members[2],
        ClassMember::Property(property)
            if property.name == "position"
                && property.access == MemberAccess::Internal
                && property.writable
    ));
    assert!(matches!(
        &class_decl.members[3],
        ClassMember::Method(method)
            if method.name == "parse"
                && method.access == MemberAccess::External
                && !method.writable_this
    ));
    assert!(matches!(
        &class_decl.members[4],
        ClassMember::Method(method)
            if method.name == "parseProgram"
                && method.access == MemberAccess::Internal
                && !method.writable_this
    ));
    assert!(matches!(
        &class_decl.members[5],
        ClassMember::Method(method)
            if method.name == "advance"
                && method.access == MemberAccess::Internal
                && method.writable_this
    ));
}

#[test]
fn parses_given_when_and_preserves_its_finalizer() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
let $value = given {
    let $ready = true;
} when ($ready): int {
    return 1;
} else when (false) {
    return 2;
} else {
    return 3;
} finally {
};
"#,
    )
    .expect("Stage 28a Slice 1 control flow must parse without diagnostics");

    let Item::Statement(Stmt::VarDecl(declaration)) = &program.items[0] else {
        panic!("expected a top-level declaration");
    };
    let Expr::When(when) = &declaration.initializer else {
        panic!("expected a when expression");
    };
    assert!(when.given.is_some());
    assert_eq!(
        when.result_type.as_ref().map(|ty| ty.name.as_str()),
        Some("int")
    );
    assert_eq!(when.branches.len(), 3);
    assert!(when.branches[0].condition.is_some());
    assert!(when.branches[1].condition.is_some());
    assert!(when.branches[2].condition.is_none());
    assert!(when.finally.is_some());
}

#[test]
fn parses_do_while_and_requires_its_ordinary_semicolon() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
function main(): void
{
    do {
        echo "once";
    } while (false);
}
"#,
    )
    .expect("ordinary do-while should parse");
    let Item::Function(main) = &program.items[0] else {
        panic!("expected main");
    };
    assert!(matches!(
        &main.body.statements()[0],
        Stmt::DoWhile(statement) if statement.semicolon_span.is_some()
    ));

    let diagnostics = doriac::parse_source(
        "test.doria",
        "function main(): void { do {} while (false) }",
    )
    .expect_err("missing do-while semicolon must fail");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "P0018"));
}

#[test]
fn rejects_finally_on_excluded_control_flow_families() {
    for source in [
        "function main(): void { for (;;) {} finally {} }",
        "function main(): void { foreach (0..1 as int $item) {} finally {} }",
        "function main(): void { let $value = match (true) { true => 1, default => 0 } finally {}; }",
        "function main(): void { {} finally {} }",
    ] {
        let diagnostics = doriac::parse_source("test.doria", source)
            .expect_err("excluded finally attachment must fail in parsing");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "P0021"),
            "expected P0021 for {source}, got {diagnostics:?}"
        );
    }
}

#[test]
fn rejects_when_without_else_and_given_on_do() {
    let missing_else =
        doriac::parse_source("test.doria", "let $value = when (true): int { return 1; };")
            .expect_err("value-returning when requires else");
    assert!(missing_else
        .iter()
        .any(|diagnostic| diagnostic.message.contains("requires an `else` block")));

    let given_do = doriac::parse_source(
        "test.doria",
        "function main(): void { given {} do {} while (false); }",
    )
    .expect_err("given must not attach to do-while");
    assert!(given_do.iter().any(|diagnostic| diagnostic.code == "P0019"));
}

#[test]
fn accepted_named_argument_grammar_is_shared_by_every_call_form() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
function main(): void
{
    save(name: "free");
    $store->save(name: "method");
    Store::save(name: "static");
    new Store(name: "constructor");
}
"#,
    )
    .expect("named arguments parse cleanly for every call form");

    let Item::Function(main) = &program.items[0] else {
        panic!("expected the parsed function");
    };
    let names: Vec<Option<&str>> = main
        .body
        .statements()
        .iter()
        .map(|statement| {
            let Stmt::Expr { expr, .. } = statement else {
                panic!("expected a call statement");
            };
            let args = match expr {
                Expr::FunctionCall { args, .. }
                | Expr::MethodCall { args, .. }
                | Expr::StaticCall { args, .. }
                | Expr::New { args, .. } => args,
                _ => panic!("expected a call expression"),
            };
            assert_eq!(args.len(), 1, "each call carries one argument");
            args[0].name.as_ref().map(|name| name.text.as_str())
        })
        .collect();

    // Free function, instance method, static method, and constructor all carry
    // the captured argument name (decision 0098's four callable forms).
    assert_eq!(
        names,
        vec![Some("name"), Some("name"), Some("name"), Some("name")]
    );
}

#[test]
fn constructor_targets_must_be_non_nullable_class_types() {
    doriac::parse_source(
        "test.doria",
        "function main(): void { new Box<int>(); new Box<?string>(); }",
    )
    .expect("generic class targets and nullable generic arguments should parse");

    for target in ["?Box<int>", "Box<int>[]"] {
        let diagnostics = doriac::parse_source(
            "test.doria",
            format!("function main(): void {{ new {target}(); }}"),
        )
        .expect_err("nullable and array-shaped constructor targets must be rejected");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code.starts_with('P')
                && diagnostic
                    .message
                    .contains("requires a non-nullable class type")
        }));
    }
}

#[test]
fn positional_argument_after_named_argument_is_rejected() {
    let diagnostics = doriac::parse_source(
        "test.doria",
        r#"
function main(): void
{
    save(name: "free", 1);
}
"#,
    )
    .expect_err("a positional argument may not follow a named argument");

    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0515")
            .count(),
        1
    );
}

#[test]
fn positional_arguments_may_precede_named_arguments() {
    doriac::parse_source(
        "test.doria",
        r#"
function main(): void
{
    save(1, name: "free");
}
"#,
    )
    .expect("positional arguments may precede named arguments");
}

#[test]
fn rejects_unsupported_visibility_member_syntax() {
    for source in [
        "class Person { public string $name; }",
        "class Person { public function greet(): void {} }",
        "class Person { private string $name; }",
        "class Person { private function greet(): void {} }",
        "class Person { protected string $name; }",
        "class Person { protected function greet(): void {} }",
    ] {
        let err = doriac::parse_source("test.doria", source)
            .expect_err("unsupported visibility syntax should be rejected");

        assert!(
            err.iter().any(|diagnostic| diagnostic.code == "P0001"),
            "expected parse diagnostic for source `{source}`"
        );
    }
}
