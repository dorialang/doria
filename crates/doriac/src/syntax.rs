//! Syntax facts at a cursor, for editors.
//!
//! [`completion_context`] answers two questions about a position in Doria
//! source: what kind of name may be written there, and which keywords may be
//! written there. Both answers come from the compiler's own lexer and parser,
//! so an editor never keeps a second grammar or guesses from nearby tokens.
//!
//! - **Where the cursor is.** The lexer reports comments, strings and numbers.
//!   The partial word before the cursor is set aside, and the real parser
//!   parses the text up to the start of that word. Parser entry points
//!   (a statement, a class member, a type, an expression operand, a declared
//!   name, a qualified name, a member after `->` or `::`) record the first
//!   construct that reaches the end of the text, which is the outermost one.
//!   Once the parser has reported an error at the end of the text, its
//!   recovery records nothing.
//! - **Which keywords.** For each keyword in the lexer's keyword table, the
//!   parser parses the same text with that keyword at the cursor, followed by
//!   a closing parenthesis. A keyword is offered only when the parser takes it
//!   there: some parse path, speculative ones included, consumes the keyword
//!   with no error since reaching it, and that path's next error, if any, is
//!   not on the keyword. (The parenthesis gives a path that takes the keyword
//!   a later token to report; at the end of the text the parser reports what
//!   it expected at the token before.) A keyword taken only before a
//!   particular token, such as `self` before `::`, is tried again with each
//!   token the parser tested right after it. At a name position the parser rejects
//!   keywords it does not take as names and accepts those it does, such as
//!   `use` as a function name or `writable` after `let`.
//! - **What is not covered.** Keywords that the parser accepts and the checker
//!   rejects are offered: `break` and `continue` outside a loop (E0422) and a
//!   constructor parameter role on an ordinary parameter (E0737). The context
//!   reports `constructor_roles_allowed` for the latter.
//!
//! ```
//! use doriac::syntax::{completion_context, CompletionContext};
//!
//! let source = "function main(): void\n{\n    let $done = true x";
//! let at = completion_context("main.doria", source, source.len());
//! assert_eq!(at.context, CompletionContext::Expression { after_operand: true });
//! assert!(at.keywords.contains(&"xor"));
//! assert_eq!(&source[at.replace.clone()], "x");
//! ```

use std::ops::Range;

use crate::lexer::{Lexer, TokenKind, KEYWORDS};
use crate::source::SourceFile;

/// The kind of word that may be written at a cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionContext {
    /// Nothing may be completed: inside a comment, a string or a number, or
    /// where the parser accepts neither a name nor a keyword.
    None,
    /// The start of a top-level declaration, an import or an include.
    TopLevel,
    /// The start of a member in a class, interface, trait or enum body.
    MemberDeclaration,
    /// The start of a statement in a block.
    Statement,
    /// Inside an expression. `after_operand` is true after a complete operand,
    /// where only an operator or the end of the expression may follow; it is
    /// false where an operand begins.
    Expression { after_operand: bool },
    /// A type. `new_expression` is true after `new`; the two other flags say
    /// whether parameter modifiers (`writable`, `take`, `borrow`, `readonly`)
    /// and constructor roles (`internal`, `override`, `parameter`) may come
    /// first, as at the start of a parameter.
    Type {
        new_expression: bool,
        parameter_modifiers_allowed: bool,
        constructor_roles_allowed: bool,
    },
    /// A name being declared: after `class`, `interface`, `enum`, `trait`,
    /// `function`, a constant's type, a parameter's or local's type, or `as`
    /// in an alias. Any new name may be written.
    DeclarationName,
    /// A segment of a qualified name: after `namespace` or `use`, or after a
    /// `\` in any name. `qualifier` is the path already written, ending in `\`
    /// when it is not empty.
    NamespacePath { qualifier: String },
    /// A property or method name after `->` or `?->`.
    Member,
    /// A static member name after `::`. `qualifier` is the class, `self` or
    /// `parent` before it.
    Static { qualifier: String },
    /// A variable being referenced: a word starting with `$` where an
    /// expression may begin.
    Variable,
    /// No name may be written here, but keywords may, such as `extends` and
    /// `implements` after a class name, or `throws` after a return type.
    Continuation,
}

/// What may be written at a cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorCompletion {
    pub context: CompletionContext,
    /// The keywords the parser accepts at the cursor, in keyword-table order.
    pub keywords: Vec<&'static str>,
    /// The partial word before the cursor, which a completion replaces. Empty
    /// at the cursor when no word has been started.
    pub replace: Range<usize>,
}

/// The syntax context and accepted keywords at byte `offset` in `source`.
/// `path` names the source in nothing the caller sees; it keeps diagnostics
/// from the parses well-formed. An offset past the end, or inside a UTF-8
/// character, is moved back to the nearest character boundary.
pub fn completion_context(path: &str, source: &str, offset: usize) -> CursorCompletion {
    let mut offset = offset.min(source.len());
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let prefix = &source[..offset];
    let word_start = prefix
        .char_indices()
        .rev()
        .take_while(|(_, character)| character.is_ascii_alphanumeric() || *character == '_')
        .last()
        .map_or(offset, |(index, _)| index);
    let variable = prefix[..word_start].ends_with('$');
    let name_start = if variable { word_start - 1 } else { word_start };
    let replace = name_start..offset;
    let nothing = || CursorCompletion {
        context: CompletionContext::None,
        keywords: Vec::new(),
        replace: replace.clone(),
    };

    // A word that starts with a digit is a number.
    if prefix[word_start..]
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        return nothing();
    }
    let before = SourceFile::new(path, &source[..name_start]);
    let (tokens, comments) = Lexer::new(&before).lex_with_comments();
    // A string or comment that is still open at the cursor makes the lexer
    // fail or ends at the cursor.
    let Ok(tokens) = tokens else {
        return nothing();
    };
    // A comment that ends at the cursor in the prefix is still open there,
    // unless it is a block comment that its `*/` closed.
    if comments.iter().any(|comment| {
        let text = &source[comment.clone()];
        comment.end == name_start
            && !(text.len() >= 4 && text.starts_with("/*") && text.ends_with("*/"))
    }) {
        return nothing();
    }
    let parsed = crate::parser::Parser::new(tokens).completion_context();
    let context = match parsed {
        Some(CompletionContext::Statement | CompletionContext::Expression { .. }) if variable => {
            CompletionContext::Variable
        }
        Some(CompletionContext::DeclarationName) if variable => CompletionContext::DeclarationName,
        _ if variable => return nothing(),
        Some(context) => context,
        None => CompletionContext::Continuation,
    };
    let keywords = if takes_keywords(&context) {
        accepted_keywords(path, &source[..name_start])
    } else {
        Vec::new()
    };
    let context = match context {
        CompletionContext::Continuation if keywords.is_empty() => CompletionContext::None,
        context => context,
    };
    CursorCompletion {
        context,
        keywords,
        replace,
    }
}

/// No keyword can be written inside a comment, a string or a number, or after
/// the `$` of a variable.
const fn takes_keywords(context: &CompletionContext) -> bool {
    !matches!(
        context,
        CompletionContext::None | CompletionContext::Variable
    )
}

/// The keywords the parser accepts at the end of `prefix`: for each keyword,
/// the parser parses `prefix`, the keyword and a closing parenthesis. The
/// keyword is accepted when some parse path consumes it with no error since
/// reaching it, and that path's next error, if any, is not on the keyword.
/// When none does, the trial is repeated with each token the parser tested
/// right after the keyword placed before the parenthesis.
fn accepted_keywords(path: &str, prefix: &str) -> Vec<&'static str> {
    let separator = if prefix
        .chars()
        .next_back()
        .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        " "
    } else {
        ""
    };
    let start = prefix.len() + separator.len();
    let trial = |keyword: &str, follower: &str| -> Option<crate::parser::TokenTrialOutcome> {
        let source = SourceFile::new(path, format!("{prefix}{separator}{keyword} {follower}"));
        let tokens = Lexer::new(&source).lex().ok()?;
        // The keyword must reach the parser as that keyword.
        let index = tokens.iter().position(|token| {
            token.span.start == start
                && !matches!(token.kind, TokenKind::Identifier(_) | TokenKind::Eof)
        })?;
        Some(crate::parser::Parser::new(tokens).token_trial_at(index))
    };
    KEYWORDS
        .iter()
        .copied()
        .filter(|keyword| {
            // A closing parenthesis gives a path that takes the keyword a
            // later token to report.
            let Some(outcome) = trial(keyword, ")") else {
                return false;
            };
            if outcome.accepted {
                return true;
            }
            // Some keywords are taken only before a particular token, such as
            // `self` before `::`. Try each token the parser asked for next.
            outcome
                .followers
                .iter()
                .filter_map(crate::parser::token_text)
                .filter(|follower| follower != ")")
                .any(|follower| {
                    trial(keyword, &format!("{follower} )")).is_some_and(|outcome| outcome.accepted)
                })
        })
        .collect()
}
