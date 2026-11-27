//! Compiler-owned text views over `string` (Decision 0147).
//!
//! `$text->graphemes` is written in Doria below over two compiler-private
//! primitives: the end of the grapheme cluster at a byte offset, and the text
//! between two cluster boundaries. No canonical type list names these classes,
//! so no authored source can refer to them; the compiler reaches them only
//! when it lowers `$text->graphemes`, and a program sees the result as
//! `Iterable<string>`.
//!
//! The view and the cursor hold the string itself. A string is Copy and
//! immutable, so holding it shares its buffer, copies no text, and needs no
//! borrow of the receiver.

use crate::ast::{ClassMember, FunctionBody, Item, Program};
use crate::lexer::{Lexer, TokenKind};
use crate::source::{SourceFile, SourceId};

pub const SOURCE_ID: SourceId = SourceId(0x7fff_fff8);
pub const SOURCE_NAME: &str = "<compiler-known:text>";
pub const GRAPHEME_VIEW: &str = "Doria\\Std\\Text\\GraphemeView";
/// The static factory that `$text->graphemes` lowers to.
pub const GRAPHEME_VIEW_FACTORY: &str = "of";

/// The members bound to MIR String intrinsics instead of compiled from their
/// Doria body. Everything else in [`SOURCE_TEXT`] is ordinary Doria.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFunction {
    /// `GraphemeCursor::boundaryAfter(string $text, int $offset): int`.
    GraphemeBoundaryAfter,
    /// `GraphemeCursor::between(string $text, int $start, int $end): string`.
    TextBetween,
}

impl TextFunction {
    pub const fn string_intrinsic(self) -> crate::mir::StringIntrinsicKind {
        match self {
            Self::GraphemeBoundaryAfter => crate::mir::StringIntrinsicKind::GraphemeBoundaryAfter,
            Self::TextBetween => crate::mir::StringIntrinsicKind::TextBetween,
        }
    }
}

pub const SOURCE_TEXT: &str = r#"
class GraphemeCursor implements Iterator<string> {
    internal writable int $start = 0;
    internal writable int $end = 0;
    function __construct(internal string $text) {
        $this->end = self::boundaryAfter($this->text, 0);
    }
    internal static function boundaryAfter(string $text, int $offset): int {}
    internal static function between(string $text, int $start, int $end): string {}
    function hasCurrent(): bool {
        return $this->start < $this->end;
    }
    function getCurrent(): string {
        return self::between($this->text, $this->start, $this->end);
    }
    writable function advance(): void {
        $this->start = $this->end;
        $this->end = self::boundaryAfter($this->text, $this->end);
    }
}
class GraphemeView implements Iterable<string> {
    internal function __construct(internal string $text) {}
    static function of(string $text): Iterable<string> {
        return new Doria\Std\Text\GraphemeView($text);
    }
    function iterator(): Iterator<string> {
        return new Doria\Std\Text\GraphemeCursor($this->text);
    }
}
"#;

/// Whether `source` may read `$text->graphemes`: some `->graphemes` or
/// `?->graphemes` appears in it. The checker decides whether the receiver is a
/// string; declarations that end up unused are never lowered.
pub fn source_uses_graphemes(source: &SourceFile) -> bool {
    let Ok(tokens) = Lexer::new(source).lex() else {
        return false;
    };
    tokens.windows(2).any(|pair| {
        matches!(pair[0].kind, TokenKind::Arrow | TokenKind::QuestionArrow)
            && matches!(&pair[1].kind, TokenKind::Identifier(name) if name == "graphemes")
    })
}

fn declarations() -> &'static Program {
    static DECLARATIONS: std::sync::LazyLock<Program> = std::sync::LazyLock::new(|| {
        let mut program =
            crate::parse_source_file(&SourceFile::with_id(SOURCE_ID, SOURCE_NAME, SOURCE_TEXT))
                .expect("compiler-known Text declarations parse");
        for item in &mut program.items {
            let Item::Class(class) = item else {
                unreachable!("compiler-known Text declarations are classes")
            };
            for member in &mut class.members {
                let ClassMember::Method(method) = member else {
                    continue;
                };
                let function = match method.name.as_str() {
                    "boundaryAfter" => TextFunction::GraphemeBoundaryAfter,
                    "between" => TextFunction::TextBetween,
                    _ => continue,
                };
                method.body = FunctionBody::Intrinsic {
                    implementation: crate::ast::CompilerIntrinsic::Text(function),
                    span: method.body.span(),
                };
            }
            class.name = format!("Doria\\Std\\Text\\{}", class.name);
        }
        program
    });
    &DECLARATIONS
}

pub fn augment_program(program: &Program) -> Program {
    let mut augmented = program.clone();
    augmented.items.extend(declarations().items.iter().cloned());
    augmented
}

pub(crate) fn append_hir_source(program: &mut crate::hir::Program) {
    if !program.items.iter().any(
        |item| matches!(item, crate::hir::Item::Class(class) if class.span.source == SOURCE_ID),
    ) {
        return;
    }
    program.sources.push(crate::hir::SourceUnit {
        id: SOURCE_ID,
        identity: crate::names::SourceIdentity(SOURCE_NAME.to_string()),
        package: crate::names::PackageIdentity::CompilerKnown,
        display_path: SOURCE_NAME.to_string(),
        scope: crate::build_plan::SourceScope::Main,
        origin: crate::build_plan::SourceOrigin::Explicit,
        generated_for: None,
        active: true,
        source: SourceFile::with_id(SOURCE_ID, SOURCE_NAME, SOURCE_TEXT),
    });
    if !program
        .packages
        .iter()
        .any(|package| package.identity == crate::names::PackageIdentity::CompilerKnown)
    {
        program.packages.push(crate::hir::PackageUnit {
            identity: crate::names::PackageIdentity::CompilerKnown,
            normal_dependencies: Vec::new(),
            development_dependencies: Vec::new(),
        });
    }
}
