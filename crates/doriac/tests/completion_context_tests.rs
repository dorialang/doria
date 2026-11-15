//! `doriac::syntax::completion_context`: the syntax context and accepted
//! keywords at a cursor. The replay fixtures are the three files of the
//! terminal example whose PhpStorm completion offered keywords the parser
//! rejects (the author line and two sample names are removed).

use doriac::lexer::{keyword_kind, Lexer, TokenKind};
use doriac::source::SourceFile;
use doriac::syntax::{completion_context, CompletionContext, CursorCompletion};

/// The completion at the `|` in `marked`.
fn at(marked: &str) -> (CursorCompletion, String) {
    let offset = marked.find('|').expect("a cursor marker");
    let source = marked.replacen('|', "", 1);
    (completion_context("cursor.doria", &source, offset), source)
}

const BODY: &str = "function f(): void\n{\n    ";

struct Case {
    name: &'static str,
    marked: String,
    context: CompletionContext,
    offered: &'static [&'static str],
    refused: &'static [&'static str],
    replaced: &'static str,
}

fn case(
    name: &'static str,
    marked: impl Into<String>,
    context: CompletionContext,
    offered: &'static [&'static str],
    refused: &'static [&'static str],
) -> Case {
    Case {
        name,
        marked: marked.into(),
        context,
        offered,
        refused,
        replaced: "",
    }
}

const PARAMETER: CompletionContext = CompletionContext::Type {
    new_expression: false,
    parameter_modifiers_allowed: true,
    constructor_roles_allowed: false,
};

#[test]
fn reports_each_context_and_only_keywords_the_parser_accepts() {
    use CompletionContext as C;
    let mut cases = vec![
        case(
            "empty file",
            "|",
            C::TopLevel,
            &[
                "namespace",
                "use",
                "class",
                "function",
                "interface",
                "const",
            ],
            &["case", "extends", "implements", "insteadof", "xor"],
        ),
        case(
            "after an item",
            "class A {}\n|",
            C::TopLevel,
            &["class", "function", "use", "enum"],
            &["namespace", "case", "extends", "xor"],
        ),
        case(
            "class body",
            "class A\n{\n    |",
            C::MemberDeclaration,
            &[
                "function", "const", "static", "internal", "writable", "uses",
            ],
            &[
                "continue",
                "return",
                "echo",
                "interface",
                "if",
                "case",
                "xor",
            ],
        ),
        case(
            "statement",
            format!("{BODY}|"),
            C::Statement,
            &[
                "if", "while", "foreach", "return", "let", "echo", "new", "true",
            ],
            &[
                "case",
                "implements",
                "extends",
                "insteadof",
                "interface",
                "xor",
            ],
        ),
        case(
            "statement after unfinished code",
            format!("{BODY}let $a = 1\n    let $b = 2;\n    |"),
            C::Statement,
            &["if", "return", "let"],
            &["case", "implements", "xor"],
        ),
        case(
            "operand",
            format!("{BODY}let $x = |"),
            C::Expression {
                after_operand: false,
            },
            &["new", "true", "false", "null", "not", "match", "when"],
            &["xor", "and", "or", "return", "class", "implements"],
        ),
        case(
            "after an operand",
            format!("{BODY}let $x = $y |"),
            C::Expression {
                after_operand: true,
            },
            &["xor", "and", "or", "is"],
            &["new", "true", "return", "class", "writable"],
        ),
        case(
            "function parameter",
            "function f(|",
            PARAMETER,
            &["writable", "take", "int", "string", "bool"],
            &["static", "interface", "continue", "class", "return"],
        ),
        case(
            "constructor parameter",
            "class A\n{\n    function __construct(|",
            C::Type {
                new_expression: false,
                parameter_modifiers_allowed: true,
                constructor_roles_allowed: true,
            },
            &["internal", "writable", "int"],
            &["static", "interface", "continue", "return"],
        ),
        case(
            "after new",
            format!("{BODY}let $x = new |"),
            C::Type {
                new_expression: true,
                parameter_modifiers_allowed: false,
                constructor_roles_allowed: false,
            },
            &[],
            &["xor", "return", "class", "writable"],
        ),
        case("class name", "class |", C::DeclarationName, &[], &["class"]),
        case(
            "function name",
            "function |",
            C::DeclarationName,
            &[],
            &["function"],
        ),
        case(
            "constant name",
            "class A\n{\n    const int |",
            C::DeclarationName,
            &[],
            &["int", "xor"],
        ),
        case(
            "parameter name",
            "function f(int |",
            C::DeclarationName,
            &[],
            &["writable"],
        ),
        case(
            "namespace",
            "namespace |",
            C::NamespacePath {
                qualifier: String::new(),
            },
            &[],
            &["class"],
        ),
        case(
            "import path",
            "use Doria\\Std\\|",
            C::NamespacePath {
                qualifier: "Doria\\Std\\".to_string(),
            },
            &[],
            &["class"],
        ),
        case("member", format!("{BODY}$x->|"), C::Member, &[], &["xor"]),
        case(
            "static member",
            format!("{BODY}Foo::|"),
            C::Static {
                qualifier: "Foo".to_string(),
            },
            &[],
            &["class"],
        ),
        case(
            "after a class name",
            "class A |",
            C::Continuation,
            &["extends", "implements"],
            &["class", "function", "xor"],
        ),
        case("line comment", "// a commen|", C::None, &[], &["class"]),
        case(
            "open block comment",
            "/* a commen|",
            C::None,
            &[],
            &["class"],
        ),
        case(
            "string",
            format!("{BODY}let $x = \"wri|"),
            C::None,
            &[],
            &["writable"],
        ),
        case(
            "number",
            format!("{BODY}let $x = 12|"),
            C::None,
            &[],
            &["xor"],
        ),
    ];
    cases.push(Case {
        replaced: "wri",
        ..case(
            "partial member keyword",
            "class A\n{\n    wri|",
            C::MemberDeclaration,
            &["writable"],
            &["return"],
        )
    });
    cases.push(Case {
        replaced: "$va",
        ..case(
            "variable",
            format!("{BODY}$va|"),
            C::Variable,
            &[],
            &["xor"],
        )
    });
    cases.push(case(
        "after a closed block comment",
        "/* done */ |",
        C::TopLevel,
        &["class"],
        &[],
    ));

    let mut failures = Vec::new();
    for case in &cases {
        let (completion, source) = at(&case.marked);
        if completion.context != case.context {
            failures.push(format!(
                "{}: context {:?}, expected {:?}",
                case.name, completion.context, case.context
            ));
        }
        for keyword in case.offered {
            if !completion.keywords.contains(keyword) {
                failures.push(format!("{}: `{keyword}` is not offered", case.name));
            }
        }
        for keyword in case.refused {
            if completion.keywords.contains(keyword) {
                failures.push(format!("{}: `{keyword}` is offered", case.name));
            }
        }
        // Where nothing can be completed, the replaced word does not matter.
        if case.context != CompletionContext::None
            && &source[completion.replace.clone()] != case.replaced
        {
            failures.push(format!(
                "{}: replaces {:?}, expected {:?}",
                case.name,
                &source[completion.replace.clone()],
                case.replaced
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const FIXTURES: [&str; 3] = [
    include_str!("fixtures/completion/window.doria"),
    include_str!("fixtures/completion/application.doria"),
    include_str!("fixtures/completion/main.doria"),
];

/// Keywords that can never begin where these contexts are reported. Each
/// list names the shapes the editor replay found offered there.
fn never_offered(context: &CompletionContext) -> &'static [&'static str] {
    match context {
        CompletionContext::MemberDeclaration => &[
            "continue",
            "break",
            "return",
            "echo",
            "if",
            "while",
            "foreach",
            "interface",
            "case",
            "xor",
            "and",
            "or",
        ],
        CompletionContext::Type {
            parameter_modifiers_allowed: true,
            ..
        } => &[
            "static",
            "interface",
            "continue",
            "class",
            "return",
            "echo",
            "xor",
        ],
        CompletionContext::Expression {
            after_operand: true,
        } => &[
            "new", "true", "false", "null", "return", "writable", "class",
        ],
        CompletionContext::Expression {
            after_operand: false,
        } => &["xor", "and", "or", "implements", "extends", "return"],
        CompletionContext::Statement => &["case", "implements", "extends", "insteadof", "xor"],
        // `break` and `continue` parse at the top level; the checker rejects
        // them there (E0422).
        CompletionContext::TopLevel => &["case", "implements", "extends", "xor"],
        _ => &[],
    }
}

/// Replays each fixture keystroke by keystroke at every word start and every
/// position inside a keyword: a keyword the file writes next is offered, no
/// keyword is offered at a name or in a comment or string, and none of the
/// shapes the editor replay found is offered.
#[test]
fn replaying_the_terminal_example_offers_only_keywords_that_fit() {
    let mut failures = Vec::new();
    for (file, text) in FIXTURES.iter().enumerate() {
        let source = SourceFile::new("fixture.doria", *text);
        let (tokens, comments) = Lexer::new(&source).lex_with_comments();
        let tokens = tokens.expect("the fixture lexes");
        for token in &tokens {
            if matches!(token.kind, TokenKind::Eof) {
                continue;
            }
            let word = &text[token.span.start..token.span.end];
            let keyword = keyword_kind(word).is_some();
            let ends = if keyword {
                token.span.start..token.span.end
            } else {
                token.span.start..token.span.start + 1
            };
            for offset in ends {
                let completion = completion_context("fixture.doria", text, offset);
                let forbidden = never_offered(&completion.context);
                for offered in &completion.keywords {
                    if forbidden.contains(offered) {
                        failures.push(format!(
                            "file {file} offset {offset}: `{offered}` offered in {:?}",
                            completion.context
                        ));
                    }
                }
                if keyword && !completion.keywords.contains(&word) {
                    failures.push(format!(
                        "file {file} offset {offset}: the file's next keyword `{word}` is not offered ({:?}, {:?})",
                        completion.context, completion.keywords
                    ));
                }
            }
            if let TokenKind::StringLiteral { .. } = token.kind {
                let inside = token.span.start + 1..token.span.end - 1;
                for offset in inside.step_by(4) {
                    if !text.is_char_boundary(offset) {
                        continue;
                    }
                    let completion = completion_context("fixture.doria", text, offset);
                    if completion.context != CompletionContext::None {
                        failures.push(format!(
                            "file {file} offset {offset}: inside a string, {:?}",
                            completion.context
                        ));
                    }
                }
            }
        }
        for comment in comments {
            for offset in (comment.start + 2..comment.end).step_by(7) {
                if !text.is_char_boundary(offset) {
                    continue;
                }
                let completion = completion_context("fixture.doria", text, offset);
                if completion.context != CompletionContext::None || !completion.keywords.is_empty()
                {
                    failures.push(format!(
                        "file {file} offset {offset}: inside a comment, {:?} {:?}",
                        completion.context, completion.keywords
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
