//! Stage 40 grammar (Decision 0148): every unsafe and foreign form parses with
//! no parser error, keeps its source spans, and reaches checking.

use doriac::ast::{ExternBlock, ForeignAggregateKind, FunctionBody, FunctionLinkage, Item, Stmt};
use doriac::diagnostics::Diagnostic;
use doriac::source::Span;

fn text(source: &str, span: Span) -> &str {
    &source[span.start..span.end]
}

/// One program with every Stage 40 form.
const EVERY_FORM: &str = r#"declare(unsafe: "allow");

namespace App\Native;

use Doria\Std\Ffi\{CChar, CSize, CUnsignedLong, Opaque, Pointer, WritablePointer};

/**
 * Bindings to zlib.
 *
 * @safety Every pointer argument addresses memory that stays valid for the call.
 */
#[Library("zlib")]
internal extern "C"
{
    /** @safety `$sourceLength` is any value. */
    function compressBound(CUnsignedLong $sourceLength): CUnsignedLong;

    #[LinkName("printf")]
    #[Variadic(fixedParameters: 1)]
    function printInteger(Pointer<CChar> $format, int32 $value = 0): int32;

    function malloc(CSize $size): ?WritablePointer<Opaque>;
}

extern "C" struct ZStream;

extern "C" struct Color
{
    uint8 $red;
    uint8 $green;
    uint8 $blue;
    uint8 $alpha = 255;
}

extern "C" union EventStorage
{
    Color $color;
    uint16[128] $tip;
}

extern "C" function windowProcedure(Pointer<Opaque> $window, uint32 $message): int64
{
    return 0;
}

class Buffer
{
    ?extern "C" function(Pointer<Opaque>, uint32): int64 $procedure = null;

    /** @safety The buffer outlives every pointer it hands out. */
    unsafe function address(): uint64
    {
        return 0;
    }

    static unsafe function fromAddress(uint64 $address): Buffer
    {
        return new Buffer();
    }
}

interface Device
{
    unsafe function reset(): void;
}

/** @safety The caller owns `$pointer`. */
unsafe function release(WritablePointer<Opaque> $pointer): void
{
    unsafe {
        free($pointer);
    }
}

function main(): void
{
    let $bound = unsafe(compressBound(CUnsignedLong::from(10)));
    unsafe {
        echo "inside";
    }
}
"#;

#[test]
fn every_stage40_form_parses_without_a_parser_error() {
    let program = doriac::parse_source("ffi.doria", EVERY_FORM).unwrap_or_else(|diagnostics| {
        panic!("Stage 40 forms must parse: {diagnostics:#?}");
    });
    let source = EVERY_FORM;

    assert_eq!(program.directives.len(), 1);
    let directive = &program.directives[0];
    assert_eq!(directive.key.text, "unsafe");
    assert_eq!(directive.value, "allow");
    assert_eq!(text(source, directive.value_span), "\"allow\"");
    assert_eq!(text(source, directive.span), "declare(unsafe: \"allow\");");

    let block = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::ExternBlock(block) => Some(block),
            _ => None,
        })
        .expect("extern block");
    let ExternBlock {
        access_span,
        abi,
        documentation,
        declarations,
        span,
        ..
    } = block;
    assert_eq!(text(source, access_span.unwrap()), "internal");
    assert_eq!(abi.value, "C");
    assert_eq!(text(source, abi.span), "\"C\"");
    assert_eq!(declarations.len(), 3);
    assert_eq!(
        documentation
            .as_ref()
            .and_then(|comment| comment.safety.as_ref())
            .map(|section| section.text.as_str()),
        Some("Every pointer argument addresses memory that stays valid for the call.")
    );

    let foreign = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) if function.linkage.is_foreign() => Some(function),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        foreign
            .iter()
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>(),
        ["compressBound", "printInteger", "malloc"]
    );
    for function in &foreign {
        assert_eq!(function.linkage, FunctionLinkage::Foreign { block: *span });
        assert!(matches!(function.body, FunctionBody::Foreign { .. }));
    }
    assert_eq!(
        foreign[0]
            .documentation
            .as_ref()
            .and_then(|comment| comment.safety.as_ref())
            .map(|section| section.text.as_str()),
        Some("`$sourceLength` is any value.")
    );
    assert!(foreign[1].documentation.is_none());
    assert!(foreign[1].params[1].default.is_some());

    let attribute_targets = program
        .attributes
        .iter()
        .map(|attachment| {
            (
                attachment.target.kind.protocol_name(),
                attachment
                    .groups
                    .iter()
                    .flat_map(|group| &group.attributes)
                    .map(|attribute| attribute.name.canonical())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    assert!(attribute_targets.contains(&("externBlock", vec!["Library".to_string()])));
    assert!(attribute_targets.contains(&(
        "foreignFunction",
        vec!["LinkName".to_string(), "Variadic".to_string()]
    )));

    let aggregates = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::ForeignAggregate(aggregate) => Some(aggregate),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(aggregates.len(), 3);
    assert_eq!(aggregates[0].name, "ZStream");
    assert!(aggregates[0].fields.is_none());
    assert_eq!(aggregates[1].kind, ForeignAggregateKind::Struct);
    let color = aggregates[1].fields.as_ref().unwrap();
    assert_eq!(color.len(), 4);
    assert!(color[3].default.is_some());
    assert_eq!(aggregates[2].kind, ForeignAggregateKind::Union);
    let union = aggregates[2].fields.as_ref().unwrap();
    assert_eq!(union[1].ty.to_string(), "uint16[128]");
    assert_eq!(text(source, union[1].type_span), "uint16[128]");

    let callback = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(function) if function.linkage.is_c_abi() => Some(function),
            _ => None,
        })
        .expect("C-ABI function");
    assert_eq!(callback.name, "windowProcedure");
    assert!(matches!(callback.body, FunctionBody::Block(_)));

    let class = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Class(class) => Some(class),
            _ => None,
        })
        .expect("class");
    let doriac::ast::ClassMember::Property(procedure) = &class.members[0] else {
        panic!("property");
    };
    assert_eq!(
        procedure.ty.to_string(),
        "?extern \"C\" function(Pointer<Opaque>, uint32): int64"
    );
    let doriac::ast::ClassMember::Method(address) = &class.members[1] else {
        panic!("method");
    };
    assert_eq!(text(source, address.unsafe_span.unwrap()), "unsafe");
    assert_eq!(
        address
            .documentation
            .as_ref()
            .and_then(|comment| comment.safety.as_ref())
            .map(|section| section.text.as_str()),
        Some("The buffer outlives every pointer it hands out.")
    );
    let doriac::ast::ClassMember::Method(from_address) = &class.members[2] else {
        panic!("static method");
    };
    assert!(from_address.is_static && from_address.unsafe_span.is_some());

    let interface = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Interface(interface) => Some(interface),
            _ => None,
        })
        .expect("interface");
    assert!(interface.requirements[0].unsafe_span.is_some());

    let release = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(function) if function.name == "release" => Some(function),
            _ => None,
        })
        .expect("unsafe free function");
    assert!(release.unsafe_span.is_some());
    let body = release.body.as_block().unwrap();
    let Stmt::Unsafe(unsafe_block) = &body.statements[0] else {
        panic!("unsafe block");
    };
    assert_eq!(text(source, unsafe_block.keyword_span), "unsafe");

    let main = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(function) if function.name == "main" => Some(function),
            _ => None,
        })
        .expect("main");
    let Stmt::VarDecl(bound) = &main.body.as_block().unwrap().statements[0] else {
        panic!("declaration");
    };
    let doriac::ast::Expr::Unsafe {
        keyword_span, span, ..
    } = &bound.initializer
    else {
        panic!("unsafe expression");
    };
    assert_eq!(text(source, *keyword_span), "unsafe");
    assert_eq!(
        text(source, *span),
        "unsafe(compressBound(CUnsignedLong::from(10)))"
    );
}

fn check_codes(source: &str) -> Vec<Diagnostic> {
    doriac::check_source("ffi.doria", source).expect_err("unsafe code is not checked yet")
}

#[test]
fn every_stage40_form_reaches_checking_as_not_available_yet() {
    let diagnostics = check_codes(EVERY_FORM);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "E0784"),
        "only not-available-yet diagnostics: {diagnostics:#?}"
    );
    let titles = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.title.clone())
        .collect::<std::collections::BTreeSet<_>>();
    // The forms checking does not cover yet; the rest are checked in
    // stage40_checking_tests.
    assert!(
        titles.contains("Declare Directives Are Not Available Yet"),
        "{titles:#?}"
    );
}

#[test]
fn a_program_without_unsafe_forms_is_unaffected() {
    doriac::check_source(
        "plain.doria",
        "function main(): void { let $value = (1 + 2); echo $value; }",
    )
    .expect("ordinary program checks");
}

fn parse_error(source: &str) -> Diagnostic {
    let diagnostics = doriac::parse_source("bad.doria", source)
        .expect_err("malformed Stage 40 syntax must be a parser error");
    diagnostics.into_iter().next().unwrap()
}

#[test]
fn malformed_stage40_syntax_reports_its_rule() {
    for (source, title) in [
        (
            "function main(): void { let $x = 1; unsafe($x) = 2; }",
            "Unsafe Expression Is Not A Place",
        ),
        (
            "extern { function f(): void; }",
            "Extern ABI String Is Missing",
        ),
        (
            "extern \"C\" { function f(): void { } }",
            "Extern Declaration Has A Body",
        ),
        (
            "extern \"C\" { unsafe function f(): void; }",
            "Extern Declaration Does Not Take Unsafe",
        ),
        (
            "extern \"C\" { int $x; }",
            "Extern Block Member Is Not A Function",
        ),
        ("extern \"C\" 5;", "Extern Declaration Is Incomplete"),
        (
            "extern \"C\" struct Point { static int $x; }",
            "Foreign Aggregate Member Is Not A Field",
        ),
        (
            "namespace App; declare(unsafe: \"deny\");",
            "Declare Directive Is Not First",
        ),
        (
            "declare(unsafe = \"deny\");",
            "Declare Directive Expects A Colon",
        ),
        (
            "declare(unsafe: deny);",
            "Declare Value Must Be A String Literal",
        ),
        (
            "declare(unsafe: \"deny\", other: \"x\");",
            "Declare Directive Names One Key",
        ),
        (
            "function main(): void { let $x = unsafe 1; }",
            "Unsafe Expression Expects Parentheses",
        ),
        (
            "class Box { unsafe int $value = 0; }",
            "Unsafe Modifier Requires A Function",
        ),
    ] {
        let diagnostic = parse_error(source);
        assert_eq!(diagnostic.title, title, "{source}: {diagnostic:#?}");
    }
}

#[test]
fn a_declare_colon_fix_replaces_the_equals_sign() {
    let diagnostic = parse_error("declare(unsafe = \"deny\");");
    assert_eq!(diagnostic.fixes.len(), 1);
}

#[test]
fn fixed_length_arrays_are_written_only_in_foreign_fields() {
    let diagnostic = parse_error("function main(): void { uint16[4] $values = []; }");
    assert_eq!(diagnostic.code, "P0001");
}
