use doriac::ast::{ClassMember, FunctionBody, Item, TraitAdaptationKind};
use doriac::source::Span;

fn text(source: &str, span: Span) -> &str {
    &source[span.start..span.end]
}

#[test]
fn requirements_preserve_every_signature_boundary_without_executable_bodies() {
    let source = r#"
internal interface Repository<T implements Entity> extends App\Readable<T>, Writable<T> {
    writable function save(take T $entity, writable int $version): void throws StorageError;
    function convert<U implements Displayable>(U $value): string;
}
"#;
    let program = doriac::parse_source("grammar.doria", source).expect("interface grammar");
    let Item::Interface(interface) = &program.items[0] else {
        panic!("interface");
    };
    assert_eq!(text(source, interface.access_span.unwrap()), "internal");
    assert_eq!(text(source, interface.syntax.keyword_span), "interface");
    assert_eq!(text(source, interface.name_span), "Repository");
    let parameters = interface.syntax.type_parameters.as_ref().unwrap();
    assert_eq!(text(source, parameters.open_span), "<");
    assert_eq!(text(source, parameters.close_span), ">");
    assert_eq!(
        text(source, interface.syntax.inheritance_keyword_span.unwrap()),
        "extends"
    );
    assert_eq!(
        interface
            .syntax
            .inheritance_type_spans
            .iter()
            .map(|span| text(source, *span))
            .collect::<Vec<_>>(),
        ["App\\Readable<T>", "Writable<T>"]
    );
    assert_eq!(
        text(source, interface.syntax.inheritance_comma_spans[0]),
        ","
    );
    assert_eq!(text(source, interface.syntax.open_brace_span), "{");
    assert_eq!(text(source, interface.syntax.close_brace_span), "}");
    let method = &interface.requirements[0];
    assert_eq!(text(source, method.writable_span.unwrap()), "writable");
    assert_eq!(text(source, method.syntax.keyword_span), "function");
    assert_eq!(text(source, method.syntax.parameters.open_span), "(");
    assert_eq!(text(source, method.syntax.parameters.comma_spans[0]), ",");
    assert_eq!(text(source, method.syntax.parameters.close_span), ")");
    assert_eq!(text(source, method.syntax.return_colon_span.unwrap()), ":");
    assert_eq!(
        text(source, method.syntax.return_type_span.unwrap()),
        "void"
    );
    assert_eq!(method.params[0].name, "entity");
    assert!(method.params[0].take);
    assert!(method.params[1].writable);
    let FunctionBody::Requirement { semicolon_span } = method.body else {
        panic!("requirement must not have a body");
    };
    assert_eq!(text(source, semicolon_span), ";");
    assert_eq!(
        interface.requirements[1].type_params[0].constraints[0].name,
        "Displayable"
    );
}

#[test]
fn trait_composition_preserves_lexical_positions_origins_and_adaptations() {
    let source = r#"
trait Format<T> { function format(T $value): string; }
class Message implements App\Renderable<int>, Serializable {
    int $first = 1;
    uses Format<int>, Text {
        Format<int>::format insteadof Text;
        Text::format as formatText;
        Text::debug as internal;
        Text::trace as internal traceText;
    }
    int $last = 2;
}
"#;
    let program = doriac::parse_source("grammar.doria", source).unwrap();
    let Item::Trait(declaration) = &program.items[0] else {
        panic!("trait");
    };
    assert_eq!(declaration.type_params[0].name, "T");
    let Item::Class(class) = &program.items[1] else {
        panic!("class");
    };
    assert_eq!(class.implements[0].to_string(), "App\\Renderable<int>");
    assert!(matches!(class.members[0], ClassMember::Property(_)));
    let ClassMember::Uses(composition) = &class.members[1] else {
        panic!("ordered uses entry");
    };
    assert!(matches!(class.members[2], ClassMember::Property(_)));
    assert_eq!(text(source, composition.keyword_span), "uses");
    assert_eq!(text(source, composition.comma_spans[0]), ",");
    assert_eq!(composition.adaptations.len(), 4);
    for adaptation in &composition.adaptations {
        assert_eq!(text(source, adaptation.separator_span), "::");
        assert_eq!(text(source, adaptation.semicolon_span), ";");
    }
    assert!(matches!(
        composition.adaptations[0].kind,
        TraitAdaptationKind::InsteadOf { .. }
    ));
    assert!(matches!(
        composition.adaptations[2].kind,
        TraitAdaptationKind::Alias {
            internal_span: Some(_),
            alias: None,
            ..
        }
    ));
    assert!(matches!(
        composition.adaptations[3].kind,
        TraitAdaptationKind::Alias {
            internal_span: Some(_),
            alias: Some(_),
            ..
        }
    ));
}

#[test]
fn malformed_contracts_recover_without_cascading_into_later_members() {
    for broken in [
        "interface Bad<T,> {}",
        "interface Bad extends {}",
        "interface Bad { function missing(: int; }",
        "interface Bad { function missing(): int;",
        "trait Bad { uses First, ; function okay(): void {} }",
        "trait Bad { uses First { First::method as; } function okay(): void {} }",
        "trait Bad { uses First { First::method as private; } }",
        "trait Bad { #[Tag] uses First; }",
        "trait Bad { uses First { #[Tag] First::method as alias; } }",
        "trait Bad<#[Tag] T> {}",
        "trait Bad { open int $value; }",
    ] {
        let source = format!("{broken}\nclass Later {{ function valid(): void {{}} }}");
        let diagnostics = doriac::parse_source("recovery.doria", &source).expect_err(broken);
        assert!(diagnostics.len() <= 3, "{broken}: {diagnostics:?}");
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.span.start <= broken.len() + 1),
            "{broken}: {diagnostics:?}"
        );
    }
}

#[test]
fn class_use_has_a_local_fix_and_bodyless_class_methods_remain_invalid() {
    let source = "trait Formatting {} class Value { use Formatting; }";
    let diagnostics = doriac::parse_source("fix.doria", source).unwrap_err();
    assert_eq!(diagnostics.len(), 1);
    let edit = &diagnostics[0].fixes[0].edits[0];
    assert_eq!(text(source, edit.span), "use");
    assert_eq!(edit.replacement, "uses");
    for source in [
        "function missing(): void;",
        "class Value { function missing(): void; }",
    ] {
        assert!(doriac::parse_source("body.doria", source).is_err());
    }
}
