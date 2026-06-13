pub mod ast;
pub mod codegen_php;
pub mod diagnostics;
pub mod lexer;
pub mod parser;
pub mod semantics;
pub mod source;
pub mod symbols;
pub mod types;

use ast::Program;
use diagnostics::{Diagnostic, DiagnosticResult};
use source::SourceFile;

pub fn lex_source(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<Vec<lexer::Token>> {
    let source = SourceFile::new(path, text);
    lexer::Lexer::new(&source).lex()
}

pub fn parse_source(path: impl Into<String>, text: impl Into<String>) -> DiagnosticResult<Program> {
    let source = SourceFile::new(path, text);
    parse_source_file(&source)
}

pub fn check_source(path: impl Into<String>, text: impl Into<String>) -> DiagnosticResult<Program> {
    let source = SourceFile::new(path, text);
    let program = parse_source_file(&source)?;
    semantics::check_program(&program)?;
    Ok(program)
}

pub fn compile_source_to_php(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<String> {
    let program = check_source(path, text)?;
    Ok(codegen_php::generate(&program))
}

pub fn parse_source_file(source: &SourceFile) -> DiagnosticResult<Program> {
    let tokens = lexer::Lexer::new(source).lex()?;
    parser::Parser::new(tokens).parse_program()
}

pub fn render_diagnostics(
    path: impl Into<String>,
    text: impl Into<String>,
    diagnostics: &[Diagnostic],
) -> String {
    let source = SourceFile::new(path, text);
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.render(&source))
        .collect::<Vec<_>>()
        .join("\n")
}
