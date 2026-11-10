//! `Doria\Std\Term` intrinsic bodies for PHP output (Decision 0140).
use super::*;
use crate::compiler_known_term::TermFunction;

pub(super) fn emit_body(intrinsic: TermFunction, output: &mut String, indent: usize) {
    match intrinsic {
        // The one-word values are the same inline carriers as `Duration`.
        TermFunction::FromWord(_) => writeln(output, indent, "return new self($bits);"),
        TermFunction::Word(_) => writeln(output, indent, "return $this->__doriaInlineBits;"),
        // STAGE 46 OPEN WORK: these two go through the hosted runtime
        // (`dr_v1_host_terminal_operate` and the event wait) and are not
        // written yet. The stage is not deliverable while this arm exists.
        TermFunction::Operate | TermFunction::AwaitEvent => writeln(
            output,
            indent,
            "throw new \\LogicException('Doria\\\\Std\\\\Term is not available in PHP output yet');",
        ),
    }
}
