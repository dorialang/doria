//! `Doria\Std\Term` intrinsic bodies for PHP output (Decision 0140).
//!
//! PHP output reaches the terminal through the same native runtime as every
//! other target, so one renderer, one event decoder and one restore ledger
//! serve the program whichever way it was built.
use super::*;
use crate::compiler_known_term::TermFunction;

/// A program that names `Doria\Std\Term` carries the module's declarations.
pub(super) fn used(program: &Program) -> bool {
    program.items.iter().any(|item| {
        matches!(item, Item::Class(class)
            if class.span.source == crate::compiler_known_term::SOURCE_ID)
    })
}

pub(super) fn emit_body(
    intrinsic: TermFunction,
    function: &FunctionDecl,
    parameters: &[Option<value_result::Place>],
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    match intrinsic {
        // The one-word values are the same inline carriers as `Duration`.
        TermFunction::FromWord(_) => writeln(output, indent, "return new self($bits);"),
        TermFunction::Word(_) => writeln(output, indent, "return $this->__doriaInlineBits;"),
        TermFunction::Operate => emit_operate(output, indent),
        TermFunction::AwaitEvent => {
            let wait = value_result::intrinsic_parameter(
                function,
                1,
                parameters[1].as_ref(),
                output,
                indent,
                scopes,
            );
            let tag = wait.tag();
            let duration = wait.scalar_payload("For", 0, &scopes.specialization);
            let deadline = wait.scalar_payload("Until", 0, &scopes.specialization);
            // The runtime's wait kinds: 0 immediate, 1 indefinite, 2 a
            // duration, 3 a deadline. `Wait` declares its cases in the order
            // Immediate, For, Until, Indefinite.
            for line in [
                format!("$instant = {tag} === 1 ? {duration} : ({tag} === 2 ? {deadline} : null);"),
                format!("$kind = match ({tag}) {{ 0 => 0, 1 => 2, 2 => 3, 3 => 1, default => __DoriaNativeIo::fail('protocol') }};"),
                "return __doria_terminal_runtime()->dr_v1_terminal_await_event($generation, $kind,".into(),
                "    $instant === null ? 0 : $instant->__doriaLowWord(),".into(),
                "    $instant === null ? 0 : $instant->__doriaHighWord(),".into(),
                "    $cancellation?->__doriaCancellationState());".into(),
            ] {
                writeln(output, indent, &line);
            }
        }
    }
}

/// The packed terminal operation, shared with the standard views' own
/// questions about their device.
pub(super) fn emit_operate(output: &mut String, indent: usize) {
    for line in [
        "return __doria_terminal_runtime()->dr_v1_host_terminal_operate(",
        "    $operation, $first, $second, $text, strlen($text));",
    ] {
        writeln(output, indent, line);
    }
}

/// What every ending of a PHP program does about the terminal. The panic and
/// unhandled-error writers call it before they write, so the report lands on
/// the screen the user returns to, and PHP's own shutdown calls it for every
/// other exit. It does nothing until the program has used the terminal.
pub(super) const RUNTIME: &str = r#"
$__doria_terminal_runtime = null;

function __doria_terminal_runtime(): FFI
{
    global $__doria_terminal_runtime;
    if ($__doria_terminal_runtime === null) {
        $__doria_terminal_runtime = __DoriaNativeIo::get();
        register_shutdown_function('__doria_terminal_ending');
    }
    return $__doria_terminal_runtime;
}

function __doria_terminal_ending(): void
{
    global $__doria_terminal_runtime;
    if ($__doria_terminal_runtime !== null) {
        $__doria_terminal_runtime->dr_v1_terminal_process_ending();
    }
}
"#;
