use doriac::semantics::composition_rename::{
    CompositionRenameFacts, MemberRenameRefusal, MemberRenameTarget,
};

fn facts(source: &str) -> CompositionRenameFacts {
    let (_, analysis) = doriac::analyze_source_for_ide("rename.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    analysis.info.composition_rename
}

fn target<'a>(
    facts: &'a CompositionRenameFacts,
    source: &str,
    declaration: &str,
    name: &str,
) -> &'a MemberRenameTarget {
    let offset = source.find(declaration).unwrap() + declaration.find(name).unwrap();
    facts
        .targets
        .iter()
        .find(|target| target.name_span.start == offset)
        .unwrap_or_else(|| panic!("missing {declaration}: {facts:#?}"))
}

fn renamed(source: &str, target: &MemberRenameTarget, name: &str) -> String {
    assert_eq!(target.refusal, None, "{target:#?}");
    assert!(!target
        .forbidden_names
        .iter()
        .any(|forbidden| forbidden == name));
    let mut spans = target.references.clone();
    spans.push(target.name_span);
    spans.sort();
    spans.dedup();
    let mut result = source.to_owned();
    for span in spans.into_iter().rev() {
        assert_eq!(span, span.authored());
        let token = &source[span.start..span.end];
        assert_eq!(token.trim_start_matches('$'), target.name);
        result.replace_range(
            span.start..span.end,
            &format!("{}{name}", if token.starts_with('$') { "$" } else { "" }),
        );
    }
    result
}

const GENERIC_ALIASES: &str = r#"
trait Formatting<T> {
    function render(T $value): T { return $value; }
    function apply(T $value): T { return $this->render($value); }
}
class Numbers { uses Formatting<int> { Formatting<int>::render as renderAgain; } }
class Words { uses Formatting<string>; }
function numbers(Numbers $box): int { return $box->renderAgain(1) + $box->render(2); }
function words(Words $box): string { return $box->render("text"); }
"#;

#[test]
fn aliases_and_authored_methods_have_distinct_complete_occurrence_sets() {
    let analysis = facts(GENERIC_ALIASES);
    let original = target(&analysis, GENERIC_ALIASES, "function render", "render");
    let alias = target(&analysis, GENERIC_ALIASES, "as renderAgain", "renderAgain");
    assert_eq!(original.declarations.len(), 3);
    assert_eq!(original.references.len(), 4); // shared body, adaptation, two calls
    assert_eq!(alias.declarations.len(), 1);
    assert_eq!(alias.references.len(), 1);
    assert!(original
        .references
        .iter()
        .all(|span| !alias.references.contains(span)));
    let original_edit = renamed(GENERIC_ALIASES, original, "formatValue");
    assert!(original_edit.contains("formatValue as renderAgain"));
    assert!(original_edit.contains("$box->renderAgain(1)"));
    facts(&original_edit);
    let alias_edit = renamed(GENERIC_ALIASES, alias, "again");
    assert!(alias_edit.contains("::render as again"));
    assert!(alias_edit.contains("function render("));
    facts(&alias_edit);
}

#[test]
fn nested_adaptations_rename_the_selected_alias_not_its_implementation() {
    let source = r#"
trait Original { function render(): int { return 1; } }
trait Wrapper { uses Original { Original::render as firstAlias; } }
class C { uses Wrapper { Wrapper::firstAlias as secondAlias; } }
function read(C $value): int { return $value->render() + $value->firstAlias() + $value->secondAlias(); }
"#;
    let analysis = facts(source);
    let first = target(&analysis, source, "as firstAlias", "firstAlias");
    assert_eq!(first.references.len(), 2);
    let edited = renamed(source, first, "firstRenamed");
    assert!(edited.contains("Wrapper::firstRenamed as secondAlias"));
    assert!(edited.contains("Original::render as firstRenamed"));
    facts(&edited);
    let second = target(&analysis, source, "as secondAlias", "secondAlias");
    assert_eq!(second.references.len(), 1);
    facts(&renamed(source, second, "secondRenamed"));
}

#[test]
fn properties_constants_and_static_cells_include_all_composer_contexts() {
    let source = r#"
trait State {
    int $value = 3;
    static writable int $count = 1;
    const int STEP = 2;
    function read(): int { return $this->value + self::count + self::STEP; }
}
class Left { uses State; }
class Right { uses State; }
function read(Left $left, Right $right): int {
    return $left->value + $right->value + Left::count + Right::count + Left::STEP + Right::STEP;
}
"#;
    let analysis = facts(source);
    for (declaration, token, name) in [
        ("int $value", "$value", "payload"),
        ("int $count", "$count", "total"),
        ("int STEP", "STEP", "DELTA"),
    ] {
        let member = target(&analysis, source, declaration, token);
        assert_eq!(member.references.len(), 3, "{member:#?}");
        facts(&renamed(source, member, name));
    }
}

#[test]
fn shared_body_target_disagreement_refuses_both_authored_methods() {
    let source = r#"
trait Calls { function invoke(): int { return $this->value(); } }
class Left { uses Calls; function value(): int { return 1; } }
class Right { uses Calls; function value(): int { return 2; } }
"#;
    let analysis = facts(source);
    let members = analysis
        .targets
        .iter()
        .filter(|target| target.name == "value")
        .collect::<Vec<_>>();
    assert_eq!(members.len(), 2);
    for member in members {
        assert_eq!(
            member.refusal,
            Some(MemberRenameRefusal::AmbiguousSourceToken)
        );
    }
    let invoke = target(&analysis, source, "function invoke", "invoke");
    facts(&renamed(source, invoke, "callValue"));
}

#[test]
fn contract_and_override_families_are_not_partial_rename_edits() {
    for source in [
        "interface I { function value(): int; } trait T { function value(): int { return 1; } } class C implements I { uses T; }",
        "trait Required { function value(): int; } trait Value { function value(): int { return 1; } } class C { uses Required, Value; }",
        "open class Base { open function value(): int { return 1; } } trait T { function value(): int { return 2; } } class C extends Base { uses T { T::value as internal fromTrait; } override function value(): int { return $this->fromTrait(); } }",
    ] {
        let analysis = facts(source);
        for member in analysis.targets.iter().filter(|target| target.name == "value") {
            assert!(member.refusal.is_some(), "{member:#?}");
        }
    }
}

#[test]
fn exclusion_members_refuse_partial_rename_but_the_alias_is_independent() {
    let source = r#"
trait First { function value(): int { return 1; } }
trait Second { function value(): int { return 2; } }
class C { uses First, Second { First::value insteadof Second; Second::value as alternate; } }
function read(C $value): int { return $value->value() + $value->alternate(); }
"#;
    let analysis = facts(source);
    for member in analysis
        .targets
        .iter()
        .filter(|target| target.name == "value")
    {
        assert!(member.refusal.is_some(), "{member:#?}");
    }
    facts(&renamed(
        source,
        target(&analysis, source, "as alternate", "alternate"),
        "secondValue",
    ));
}

#[test]
fn collision_constraints_cover_descendants_and_other_trait_members() {
    let source = r#"
trait T { function render(): int { return 1; } int $payload = 2; }
open class Base { uses T { T::render as again; } }
class Child extends Base { function childOnly(): int { return 3; } }
"#;
    let analysis = facts(source);
    for name in ["render", "again"] {
        let member = analysis
            .targets
            .iter()
            .find(|target| target.name == name)
            .unwrap();
        assert!(
            member
                .forbidden_names
                .iter()
                .any(|name| name == "childOnly"),
            "{member:#?}"
        );
        assert!(
            member.forbidden_names.iter().any(|name| name == "payload"),
            "{member:#?}"
        );
    }
}

#[test]
fn an_unchecked_source_body_cannot_claim_complete_rename_coverage() {
    let source = "trait T { function value(): int { return $this->value(); } }";
    let analysis = facts(source);
    let member = target(&analysis, source, "function value", "value");
    assert_eq!(
        member.refusal,
        Some(MemberRenameRefusal::IncompleteOccurrences)
    );
}

#[test]
fn invalid_composition_retains_explicit_refusal() {
    let source =
        "trait T { function value(): int { return 1; } } class C { uses T; int $value = 0; }";
    let (_, analysis) = doriac::analyze_source_for_ide("invalid.doria", source).unwrap();
    assert!(!analysis.diagnostics.is_empty());
    for target in analysis.info.composition_rename.targets {
        assert_eq!(
            target.refusal,
            Some(MemberRenameRefusal::InvalidComposition)
        );
    }
}
