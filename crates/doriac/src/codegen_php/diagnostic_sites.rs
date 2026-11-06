//! PHP consumes compiler-measured excerpts, not host Unicode or terminal rules.

use super::*;
use crate::source::{SourceFile, SourceId};
use doria_diagnostic_catalogue::source_excerpt::{self, SourceExcerpt};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Default)]
pub(super) struct Sites {
    ids: HashMap<Span, usize>,
    spans: Vec<Span>,
}

impl Sites {
    pub(super) fn register(&mut self, span: Span) -> usize {
        let span = span.authored();
        if let Some(id) = self.ids.get(&span) {
            return *id;
        }
        let id = self.spans.len() + 1; // Zero means no authored origin.
        self.spans.push(span);
        self.ids.insert(span, id);
        id
    }

    /// Emit site data only; the program prelude owns the runtime declarations.
    pub(super) fn emit_metadata(&self, program: &Program) -> Result<String, BackendError> {
        let standalone = program
            .sources
            .is_empty()
            .then(|| SourceFile::new(&program.source_path, &program.source_text));
        let mut output = format!(
            "$__doria_source_gutter_gap = {};\n$__doria_diagnostic_sites = [\n    0 => [\"start\" => 0, \"end\" => 0, \"line\" => 1, \"gutter\" => 0, \"lines\" => [], \"known\" => false],\n",
            emit_php_string_literal(source_excerpt::GUTTER_GAP),
        );
        for (index, span) in self.spans.iter().enumerate() {
            let source = program
                .source(span.source)
                .or_else(|| standalone.as_ref().filter(|_| span.source == SourceId(0)))
                .ok_or_else(|| {
                    excerpt_error(
                        *span,
                        "diagnostic source is absent from the HIR source inventory",
                    )
                })?;
            if span.start > span.end || span.end > source.text.len() {
                return Err(excerpt_error(
                    *span,
                    "diagnostic span is outside its source",
                ));
            }
            let first_line = source.line_col(span.start).0;
            let final_line = source.line_col(span.end).0;
            let gutter = final_line
                .to_string()
                .len()
                .max(source_excerpt::MINIMUM_GUTTER_WIDTH);
            output.push_str(&format!(
                "    {} => [\"start\" => {}, \"end\" => {}, \"line\" => {}, \"gutter\" => {}, \"known\" => true, \"lines\" => [\n",
                index + 1, php_source_location(*span, span.start),
                php_source_location(*span, span.end), first_line, gutter,
            ));
            for line in first_line..=final_line {
                let raw = source.line_text(line);
                let start = source.line_start(line);
                let end = start + raw.len();
                let marker_start = span.start.max(start).min(end);
                let marker_end = span.end.min(end).max(marker_start);
                let excerpt = SourceExcerpt::new(
                    raw,
                    Some((marker_start - start, marker_end - start)),
                    source_excerpt::DEFAULT_TERMINAL_WIDTH,
                    doria_unicode::grapheme_boundaries,
                    UnicodeWidthStr::width,
                );
                let mut visible = Vec::new();
                excerpt.write(|bytes| visible.extend_from_slice(bytes));
                output.push_str(&format!(
                    "        [{}, hex2bin({}), {}, {}],\n",
                    line,
                    emit_php_string_literal(&hex_bytes(&visible)),
                    excerpt.marker_offset,
                    excerpt.marker_width,
                ));
            }
            output.push_str("    ]],\n");
        }
        output.push_str("];\n$__doria_diagnostic_site_ids = [];\nforeach ($__doria_diagnostic_sites as $__doria_site_id => $__doria_site) {\n    if ($__doria_site[\"known\"]) { $__doria_diagnostic_site_ids[$__doria_site[\"start\"]][$__doria_site[\"end\"]] = $__doria_site_id; }\n}\n");
        Ok(output)
    }
}

/// Only authored entries create logical frames. Their existing body, including
/// all Doria cleanup, stays inside the frame's normal/checked exit boundary.
pub(super) fn emit_frame_body(
    output: &mut String,
    body: &str,
    indent: usize,
    span: Span,
    scopes: &PhpNameScopes,
) {
    if scopes.compiler_owned_callable {
        // Declaration ownership, not a missing source or a helper-like name,
        // identifies transport code. It must not become an authored frame.
        output.push_str(body);
        return;
    }
    writeln(
        output,
        indent + 1,
        &format!(
            "__doria_frame_enter({}, {});",
            scopes.callable_identity(),
            scopes.checked_origin(span),
        ),
    );
    writeln(output, indent + 1, "try {");
    for line in body.lines() {
        output.push_str("    ");
        output.push_str(line);
        output.push('\n');
    }
    writeln(output, indent + 1, "} finally {");
    writeln(output, indent + 2, "__doria_frame_leave();");
    writeln(output, indent + 1, "}");
}

/// Metadata-only scopes preserve original evaluation and storage/ownership
/// plans. Every nested operand restores the enclosing invocation before its
/// actual dispatch, including checked propagation. Panic exits without unwind.
pub(super) fn operation(value: String, span: Span, scopes: &PhpNameScopes) -> String {
    if scopes.compiler_owned_callable {
        // Compiler transport must leave the enclosing authored site intact.
        return value;
    }
    let token = scopes.expression_temp("__doria_operation_", span);
    emit_expression_body(scopes, &format!(
        "${token} = __doria_operation_enter({}); try {{ return {value}; }} finally {{ __doria_operation_leave(${token}); }}",
        scopes.checked_origin(span),
    ), &[])
}

pub(super) fn emit_set_site(
    output: &mut String,
    indent: usize,
    span: Span,
    scopes: &PhpNameScopes,
) {
    if !scopes.compiler_owned_callable {
        writeln(
            output,
            indent,
            &format!("__doria_set_site({});", scopes.checked_origin(span)),
        );
    }
}

pub(super) fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::Block(block) => block.span,
        Stmt::VarDecl(decl) => decl.span,
        Stmt::Assignment(assignment) => assignment.span,
        Stmt::Echo { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::Break { span }
        | Stmt::Continue { span }
        | Stmt::Expr { span, .. } => *span,
        Stmt::If(statement) => statement.span,
        Stmt::While(statement) => statement.span,
        Stmt::DoWhile(statement) => statement.span,
        Stmt::For(statement) => statement.span,
        Stmt::Foreach(statement) => statement.span,
        Stmt::Increment(statement) => statement.span,
        Stmt::Throw(statement) => statement.span,
        Stmt::Try(statement) => statement.span,
    }
}

fn excerpt_error(span: Span, detail: &str) -> BackendError {
    BackendError::from_diagnostics(vec![Diagnostic::new(
        "B0001",
        "PHP diagnostic source data could not be emitted",
        span,
    )
    .with_developer_details(detail)])
}

pub(super) const RUNTIME: &str = r#"
// Flat triples (function, site, enteredFrom) retain capacity at peak depth.
$__doria_frames = [];
$__doria_frame_top = 0;

function __doria_frame_enter(string $function, int $site): void
{
    global $__doria_frames, $__doria_frame_top;
    $base = $__doria_frame_top;
    $__doria_frames[$base] = $function;
    $__doria_frames[$base + 1] = $site;
    $__doria_frames[$base + 2] = $base === 0 ? 0 : $__doria_frames[$base - 2];
    $__doria_frame_top = $base + 3;
}

function __doria_frame_leave(): void
{
    global $__doria_frames, $__doria_frame_top;
    $__doria_frame_top -= 3;
    $__doria_frames[$__doria_frame_top] = null;
    $__doria_frames[$__doria_frame_top + 1] = 0;
    $__doria_frames[$__doria_frame_top + 2] = 0;
}

function __doria_set_site(int $site): void
{
    global $__doria_frames, $__doria_frame_top;
    if ($__doria_frame_top !== 0) { $__doria_frames[$__doria_frame_top - 2] = $site; }
}

function __doria_panic_at_current_site(string $code): void
{
    global $__doria_frames, $__doria_frame_top;
    $site = __doria_error_site($__doria_frame_top === 0 ? 0 : $__doria_frames[$__doria_frame_top - 2]);
    __doria_panic($code, $site["start"], $site["end"]);
}

function __doria_operation_enter(int $site): int
{
    global $__doria_frames, $__doria_frame_top;
    $previous = $__doria_frame_top === 0 ? 0 : $__doria_frames[$__doria_frame_top - 2];
    __doria_set_site($site);
    return $previous;
}

function __doria_operation_leave(int $token): void
{
    __doria_set_site($token);
}

function __doria_panic_frames(array $operation): array
{
    global $__doria_frames, $__doria_frame_top;
    $frames = [];
    for ($base = $__doria_frame_top - 3; $base >= 0; $base -= 3) {
        $site = $frames === [] ? $operation :
            __doria_error_site($__doria_frames[$base + 5]);
        [$path, , $start] = __doria_source_location($site["start"]);
        [, , $end] = __doria_source_location($site["end"]);
        $frames[] = [$__doria_frames[$base], $path, $start, $end, $site["line"]];
    }
    return $frames;
}

function __doria_u64_le(int $value): string
{
    // Arithmetic shifting preserves negative signed facts' two's-complement bits.
    return pack("V2", $value & 0xffffffff, ($value >> 32) & 0xffffffff);
}

function __doria_publish_outcome(string $path, string $record): bool
{
    return @file_put_contents($path, $record, LOCK_EX) === strlen($record);
}

// Fact values are private V2 (kind, value) pairs, not a second public model.
function __doria_panic_facts_match(string $code, array $facts): bool
{
    global $__doria_catalogue;
    $names = $__doria_catalogue[$code][3];
    // Explicit panic's message is carried in the V2 header, as in native output.
    if ($code === "P1000") { $names = array_slice($names, 1); }
    if (array_keys($facts) !== $names) { return false; }
    foreach ($facts as $fact) {
        if (!is_array($fact) || array_keys($fact) !== [0, 1]) { return false; }
        [$kind, $value] = $fact;
        $valid = match ($kind) {
            1 => is_int($value),
            2 => is_int($value) && $value >= 0,
            3 => is_bool($value),
            4 => is_string($value),
            default => false,
        };
        if (!$valid) { return false; }
    }
    return true;
}

function __doria_write_panic_outcome_v2(
    string $path,
    string $code,
    ?string $message,
    array $site,
    string $function,
    array $facts,
    array $frames,
): bool
{
    if (!$site["known"]) { return false; }
    [$sourcePath, $sourceText, $start] = __doria_source_location($site["start"]);
    [, , $end] = __doria_source_location($site["end"]);
    $message ??= "";
    if (strlen($code) > 16 || strlen($message) > 65536 ||
        strlen($sourcePath) > 4096 || strlen($sourceText) > 4194304 ||
        strlen($function) > 1024 || count($frames) > 128 || count($facts) > 32
    ) { return false; }
    foreach ($facts as $name => [$kind, $value]) {
        if (strlen($name) > 1024 || ($kind === 4 && strlen($value) > 65536)) {
            return false;
        }
    }
    foreach ($frames as [$name, $framePath, $frameStart, $frameEnd]) {
        if (strlen($name) > 1024 || strlen($framePath) > 4096) { return false; }
    }
    $record = "DORIAO2\0" . pack("vvVVVvvv", 2, strlen($code), strlen($message),
        strlen($sourcePath), strlen($sourceText), strlen($function), count($frames), count($facts)) .
        __doria_u64_le($start) . __doria_u64_le($end) .
        $code . $message . $sourcePath . $sourceText . $function;
    foreach ($facts as $name => [$kind, $value]) {
        $text = $kind === 4 ? $value : "";
        $scalar = $kind === 4 ? 0 : (int) $value;
        $record .= pack("vC", strlen($name), $kind) . __doria_u64_le($scalar) .
            pack("V", strlen($text)) . $name . $text;
    }
    foreach ($frames as [$name, $framePath, $frameStart, $frameEnd]) {
        $record .= pack("vV", strlen($name), strlen($framePath)) .
            __doria_u64_le($frameStart) . __doria_u64_le($frameEnd) . $name . $framePath;
    }
    return __doria_publish_outcome($path, $record);
}

function __doria_diagnostic_site(int $start, int $end): array
{
    global $__doria_diagnostic_sites, $__doria_diagnostic_site_ids;
    $id = $__doria_diagnostic_site_ids[$start][$end] ?? null;
    if ($id !== null) { return $__doria_diagnostic_sites[$id]; }
    if ($start === 0 && $end === 0) { return $__doria_diagnostic_sites[0]; }
    // Missing emitted metadata is an invariant failure, never a guessed preview.
    __doria_panic("P1001", 0, 0, null, "<runtime>");
}

function __doria_checked_origin(int $start, int $end): int
{
    global $__doria_diagnostic_site_ids;
    if ($start === 0 && $end === 0) { return 0; }
    __doria_diagnostic_site($start, $end);
    return $__doria_diagnostic_site_ids[$start][$end];
}

function __doria_error_site(int $origin): array
{
    global $__doria_diagnostic_sites;
    if (isset($__doria_diagnostic_sites[$origin])) { return $__doria_diagnostic_sites[$origin]; }
    __doria_panic("P1001", 0, 0, null, "<runtime>");
}

function __doria_error_source_location(int $origin): array
{
    $site = __doria_error_site($origin);
    if (!$site["known"]) { return ["", "", 0, 0]; }
    [$path, $text, $start] = __doria_source_location($site["start"]);
    [, , $end] = __doria_source_location($site["end"]);
    return [$path, $text, $start, $end];
}

function __doria_source_excerpt(array $site, string $label): string
{
    global $__doria_source_gutter_gap;
    $output = "";
    foreach ($site["lines"] as $index => [$line, $visible, $offset, $width]) {
        $output .= "\n" . str_pad((string) $line, $site["gutter"], " ", STR_PAD_LEFT) .
            ($visible === "" ? "" : $__doria_source_gutter_gap . $visible);
        $prefix = str_repeat(" ", $site["gutter"]) . $__doria_source_gutter_gap .
            str_repeat(" ", $offset);
        $output .= "\n" . $prefix . str_repeat("^", $width);
        if ($index === 0 && $label !== "") { $output .= "\n" . $prefix . $label; }
    }
    return $output;
}

"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{render_diagnostics, ColorChoice, RenderOptions};
    use std::process::Output;

    fn run_php(php: &str, tail: &str) -> Output {
        let script = format!("{}\n{tail}", php.strip_prefix("<?php").unwrap());
        php_execution::Script::new(&script)
            .command(&["-d", "ffi.enable=0"])
            .env_remove("DORIA_RUNTIME_OUTCOME_V2")
            .output()
            .expect("PHP is required for diagnostic compatibility parity")
    }

    fn main_call() -> String {
        format!("{}();", php_function_name("main"))
    }

    #[test]
    fn scalar_operation_tokens_and_frame_slots_reuse_peak_depth_storage() {
        let php = crate::compile_source_to_php("frame-storage.doria", "function main(): void {}")
            .unwrap();
        assert!(php.contains("function __doria_operation_enter(int $site): int"));
        assert!(php.contains("function __doria_operation_leave(int $token): void"));
        for helper in [
            "__doria_frame_enter",
            "__doria_frame_leave",
            "__doria_operation_enter",
            "__doria_operation_leave",
        ] {
            let start = RUNTIME.find(&format!("function {helper}(")).unwrap();
            let body = RUNTIME[start..].split_once("\n}\n").unwrap().0;
            assert!(
                !body.contains(" = [") && !body.contains("return ["),
                "{body}"
            );
            assert!(
                !body.contains("array_pop(") && !body.contains("new "),
                "{body}"
            );
        }
        let output = run_php(
            &php,
            r#"
$empty = __doria_operation_enter(91);
if ($empty !== 0 || $__doria_frame_top !== 0) { exit(1); }
__doria_operation_leave($empty);
$emptyFrames = [null, 0, 0, null, 0, 0];
for ($iteration = 0; $iteration < 5000; ++$iteration) {
    __doria_frame_enter("outer", 11);
    $outer = __doria_operation_enter(12);
    $nested = __doria_operation_enter(13);
    if ($outer !== 11 || $nested !== 12 || $__doria_frames[1] !== 13) { exit(2); }
    __doria_operation_leave($nested);
    __doria_frame_enter("child", 21);
    $child = __doria_operation_enter(22);
    if ($child !== 21 || $__doria_frames[5] !== 12) { exit(3); }
    __doria_operation_leave($child);
    __doria_frame_leave();
    if ($__doria_frame_top !== 3 || $__doria_frames[1] !== 12) { exit(4); }
    __doria_operation_leave($outer);
    if ($__doria_frames[1] !== 11) { exit(5); }
    __doria_frame_leave();
    if ($__doria_frame_top !== 0 || $__doria_frames !== $emptyFrames) { exit(6); }
}
echo "5000;6";
"#,
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"5000;6");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn operation_sites_restore_through_nested_expression_unwind() {
        let php =
            crate::compile_source_to_php("operation-unwind.doria", "function main(): void {}")
                .unwrap();
        let output = run_php(
            &php,
            r#"
__doria_frame_enter("caller", 11);
$outer = __doria_operation_enter(12);
try {
    $inner = __doria_operation_enter(13);
    try {
        __doria_frame_enter("callee", 21);
        try {
            $callee = __doria_operation_enter(22);
            try { throw new RuntimeException("operation exit"); }
            finally { __doria_operation_leave($callee); }
        } finally { __doria_frame_leave(); }
    } finally { __doria_operation_leave($inner); }
} catch (RuntimeException $error) {
    if ($__doria_frame_top !== 3 || $__doria_frames[1] !== 12) { exit(1); }
} finally { __doria_operation_leave($outer); }
if ($__doria_frames[1] !== 11) { exit(2); }
__doria_frame_leave();
if ($__doria_frame_top !== 0) { exit(3); }
echo "restored";
"#,
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"restored");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn panic_paths_use_active_frames_after_peak_depth_checked_unwind() {
        let text = r#"
class Failure implements Error { string $message = "stop"; }
function descend(int $depth): void throws Failure {
    if ($depth == 0) { throw new Failure(); }
    descend($depth - 1);
}
function main(): void {
    try { descend(6); } catch (Failure) {}
    panic("after unwind");
}
"#;
        let mir = crate::lower_source_to_mir("inactive-frames.doria", text).unwrap();
        let expected = crate::mir_interpreter::interpret(&mir).unwrap();
        let php = crate::compile_source_to_php("inactive-frames.doria", text).unwrap();
        let output = run_php(&php, &main_call());
        assert_eq!(output.status.code(), Some(101), "{output:?}");
        assert_eq!(output.stdout, expected.stdout);
        assert_eq!(output.stderr, expected.stderr);
    }

    #[test]
    fn compiler_owned_callables_preserve_authored_source_inventory_and_frames() {
        let path = "authored-provenance.doria";
        let text = r#"
interface Worker { function value(): int; }
function seed(): int { return 7; }
class Runner implements Worker {
    int $number = seed();
    function value(): int { return $this->number; }
}
class Explicit { function __construct() {} }
function inspect(Worker $worker): int { return $worker->value(); }
function main(): void {
    let $runner = new Runner();
    let $explicit = new Explicit();
    let $callback = function (Worker $worker): int { return inspect($worker); };
    if ($callback($runner) != 7) { panic("initializer changed"); }
    panic("authored stop");
}
"#;
        let program = crate::lower_source(path, text).unwrap();
        assert!(program
            .source(crate::compiler_known_io::SYNTHETIC_SOURCE_ID)
            .is_none());
        let compiler_classes = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(class)
                    if class.package == crate::names::PackageIdentity::CompilerKnown =>
                {
                    Some(class)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(compiler_classes
            .iter()
            .any(|class| class.name == crate::compiler_known_io::IO_ERROR));
        let mir = crate::mir_lowering::lower_program(&program).unwrap();
        crate::mir_validation::validate_program(&mir).unwrap();
        let php = super::super::generate(&program, Some(&mir)).unwrap();
        for class in compiler_classes {
            for function in class.members.iter().flat_map(ClassMember::callables) {
                let identity = format!("{}::{}", class.name, function.name);
                assert!(
                    !php.contains(&format!(
                        "__doria_frame_enter({},",
                        emit_php_string_literal(&identity),
                    )),
                    "compiler-owned transport became an authored frame: {identity}",
                );
            }
        }
        for identity in ["Runner::__construct", "Explicit::__construct"] {
            assert_eq!(
                php.matches(&format!(
                    "__doria_frame_enter({},",
                    emit_php_string_literal(identity),
                ))
                .count(),
                1,
                "real user construction must retain its authored frame: {identity}",
            );
        }
        let output = run_php(
            &php,
            "echo json_encode(array_values($__doria_diagnostic_sites), JSON_THROW_ON_ERROR);",
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let sites: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
        for site in sites.iter().filter(|site| site["known"] == true) {
            for key in ["start", "end"] {
                assert_eq!(site[key].as_u64().unwrap() >> 32, 0, "{site:?}");
            }
        }
        let constructor = text.find("class Runner").unwrap() as u64;
        assert!(sites.iter().any(|site| {
            site["known"] == true
                && site["start"].as_u64() == Some(constructor)
                && site["end"].as_u64() == Some(constructor)
        }));
        let output = run_php(&php, &main_call());
        let expected = crate::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(
            output.status.code(),
            Some(expected.exit_status),
            "{output:?}"
        );
        assert_eq!(output.stdout, expected.stdout);
        assert_eq!(output.stderr, expected.stderr);
    }

    #[test]
    fn missing_registered_sources_remain_invariant_errors() {
        let program =
            crate::lower_source("missing-source.doria", "function main(): void {}").unwrap();
        for source in [SourceId(77), crate::compiler_known_io::SYNTHETIC_SOURCE_ID] {
            let span = Span::in_source(source, 4, 5);
            let mut sites = Sites::default();
            sites.register(span);
            let error = sites.emit_metadata(&program).unwrap_err();
            let diagnostic = &error.diagnostics.as_ref().unwrap()[0];
            assert_eq!(diagnostic.code, "B0001");
            assert_eq!(diagnostic.span, span);
            assert!(diagnostic
                .developer_details
                .as_ref()
                .unwrap()
                .contains("diagnostic source is absent from the HIR source inventory"));
        }
    }

    #[test]
    fn panic_fact_validation_uses_catalogue_names_and_explicit_wire_kinds() {
        let php =
            crate::compile_source_to_php("panic-fact-schema.doria", "function main(): void {}")
                .unwrap();
        let output = run_php(
            &php,
            r#"echo json_encode([
                __doria_panic_facts_match("P1310", ["index" => [1, -1], "length" => [2, 0]]),
                __doria_panic_facts_match("P1310", ["index" => [1, -1], "length" => [2, -1]]),
                __doria_panic_facts_match("P1310", ["index" => [1, -1]]),
                __doria_panic_facts_match("P1310", ["index" => -1, "length" => 0]),
                __doria_panic_facts_match("P1310", ["length" => [2, 0], "index" => [1, -1]]),
                __doria_panic_facts_match("P1310", ["index" => [5, -1], "length" => [2, 0]]),
                __doria_panic_facts_match("P1303", ["length" => [1, -7]]),
                __doria_panic_facts_match("P1501", ["conflictReason" => [4, "Cannot Acquire Writable Access While Readonly Access Is Active"]]),
                __doria_panic_facts_match("P1000", []),
                __doria_panic_facts_match("P1000", ["message" => [4, "stop"]]),
            ], JSON_THROW_ON_ERROR);"#,
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let actual: Vec<bool> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            actual,
            [true, false, false, false, false, false, true, true, true, false]
        );
    }

    #[test]
    fn emitted_site_facts_match_compiler_excerpts_for_late_multiline_and_empty_spans() {
        let text = format!(
            "{}function main(): void {{\n\t/*{}*/ panic(\"stop\");\n\n}}\n",
            "\n".repeat(30),
            "👩‍💻\te\u{301}界".repeat(20),
        );
        let program = crate::lower_source("excerpt.doria", &text).unwrap();
        let source = program.source(SourceId(0)).unwrap();
        let start = text.find("panic").unwrap();
        let blank = start + text[start..].find("\n\n").unwrap() + 1;
        let spans = [
            Span::new(start, blank - 1),
            Span::new(start, text.find('}').unwrap()),
            Span::new(blank, blank),
            Span::new(text.len(), text.len()),
        ];
        let php = crate::compile_source_to_php("excerpt.doria", &text).unwrap();
        for helper in [
            "__doria_frame_enter",
            "__doria_frame_leave",
            "__doria_set_site",
            "__doria_operation_enter",
            "__doria_operation_leave",
            "__doria_panic_frames",
            "__doria_u64_le",
            "__doria_publish_outcome",
            "__doria_panic_facts_match",
            "__doria_write_panic_outcome_v2",
            "__doria_diagnostic_site",
            "__doria_checked_origin",
            "__doria_error_site",
            "__doria_error_source_location",
            "__doria_source_excerpt",
        ] {
            assert_eq!(
                php.matches(&format!("function {helper}(")).count(),
                1,
                "the program prelude must declare {helper} exactly once",
            );
        }
        for span in spans {
            let mut sites = Sites::default();
            let id = sites.register(span);
            assert_eq!(sites.register(span), id);
            assert_eq!(sites.spans.len(), 1);
            let metadata = sites.emit_metadata(&program).unwrap();
            assert!(
                !metadata.contains("function "),
                "site data must not redeclare runtime helpers",
            );
            let output = run_php(&php, &format!(
                "{metadata}\necho 'excerpt.doria · line {}' . \"\\n\" . __doria_source_excerpt(__doria_error_site({id}), 'Selected Here');",
                source.line_col(span.start).0,
            ));
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let diagnostic =
                Diagnostic::new("B0001", "excerpt test", span).with_primary_label("Selected Here");
            let rendered = render_diagnostics(
                source,
                &[diagnostic],
                RenderOptions {
                    color: ColorChoice::Never,
                    context_lines: 0,
                    ..RenderOptions::default()
                },
            );
            let expected = rendered
                .split_once("\n\nWhere\n")
                .unwrap()
                .1
                .split_once("\n\nWhy\n")
                .unwrap()
                .0;
            // Compare the complete source section exactly, without whitespace,
            // Unicode, marker or fixture normalization.
            assert_eq!(output.stdout, expected.as_bytes(), "{span:?}");
        }
    }

    #[test]
    fn panic_frames_use_authored_identities_for_specialized_functions_and_helper_named_methods() {
        let text = r#"
function fail<T>(T $value): void { panic("stop"); }
class Runner {
    function __doria_list_insert_at(): void { fail(1); }
}
function main(): void {
    let $runner = new Runner();
    $runner->__doria_list_insert_at();
}
"#;
        let php = crate::compile_source_to_php("panic-frames.doria", text).unwrap();
        let output = run_php(&php, &main_call());
        assert_eq!(output.status.code(), Some(101), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let mir = crate::lower_source_to_mir("panic-frames.doria", text).unwrap();
        let expected = crate::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(output.stderr, expected.stderr);
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        let path = stderr
            .split_once("\n\nCall Path\n")
            .unwrap()
            .1
            .split_once("\n\nProcess Exited With Status 101\n")
            .unwrap()
            .0;
        assert_eq!(
            path.lines()
                .map(|frame| frame.split_once(" · ").unwrap().0)
                .collect::<Vec<_>>(),
            ["fail", "Runner::__doria_list_insert_at", "main"],
        );
    }

    #[test]
    fn checked_errors_keep_first_throw_full_spans_and_exact_interpreter_output() {
        let long = "e\u{301}👩‍💻界".repeat(30);
        for throw in [
            format!("\tthrow new Failure(\"{long}\");"),
            format!("\tthrow new Failure(\n\t\t\"{long}\"\n\n\t);"),
        ] {
            let text = format!(
                r#"
class Failure implements Error {{ function __construct(string $message) {{}} }}
function first(): void throws Failure {{
{throw}
}}
function relay(): void throws Failure {{
    try {{ first(); }} catch (Failure $error) {{ throw $error; }}
}}
function main(): void throws Failure {{ relay(); }}
"#
            );
            let program = crate::lower_source_to_mir("first-throw.doria", &text).unwrap();
            let expected = crate::mir_interpreter::interpret(&program).unwrap();
            let php = crate::compile_source_to_php("first-throw.doria", &text).unwrap();
            let output = run_php(&php, &main_call());
            assert_eq!(
                output.status.code(),
                Some(expected.exit_status),
                "{output:?}"
            );
            assert_eq!(output.stdout, expected.stdout);
            assert_eq!(output.stderr, expected.stderr);
        }
    }

    #[test]
    fn panic_sites_are_emission_bounded_and_use_shared_preview_facts() {
        let text = format!(
            "function main(): void {{\n\t/*{}*/ panic(\"stop\");\n}}\n",
            "e\u{301}👩‍💻界".repeat(40),
        );
        let php = crate::compile_source_to_php("panic-site.doria", &text).unwrap();
        assert!(!php.contains("$markerOffset = max(0, $sourceOffset - $lineStart)"));
        assert!(!php.contains("preg_match_all('/\\X/u', $prefix"));
        let sites = run_php(
            &php,
            "echo json_encode(array_values($__doria_diagnostic_sites));",
        );
        assert_eq!(sites.status.code(), Some(0), "{sites:?}");
        assert!(sites.stderr.is_empty(), "{sites:?}");
        let entries: Vec<serde_json::Value> = serde_json::from_slice(&sites.stdout).unwrap();
        assert!(entries.len() < 16, "not a table for every source byte");
        let panic_start = text.find("panic").unwrap();
        assert!(entries.iter().any(|site| site["start"] == panic_start));
        let program = crate::lower_source_to_mir("panic-site.doria", &text).unwrap();
        let expected = crate::mir_interpreter::interpret(&program).unwrap();
        let output = run_php(&php, &main_call());
        assert_eq!(output.status.code(), Some(101), "{output:?}");
        assert_eq!(output.stdout, expected.stdout);
        let expected = std::str::from_utf8(&expected.stderr).unwrap();
        let actual = std::str::from_utf8(&output.stderr).unwrap();
        let source_section = |text: &str| {
            text.split_once("\n\nWhere\n")
                .unwrap()
                .1
                .split_once("\n\nWhy\n")
                .unwrap()
                .0
                .to_string()
        };
        assert_eq!(source_section(actual), source_section(expected));
    }
}
