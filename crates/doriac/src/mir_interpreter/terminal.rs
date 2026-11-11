//! The scripted terminal (Decision 0140). The interpreter performs every
//! `Doria\Std\Term` operation with the runtime's own operation core, against
//! a terminal that exists only as a script: a size, a color capability and
//! the events it will report. Output is captured as rendered, in order with
//! everything else the program writes to standard output.
use doria_terminal::console::{self, ledger, Host, OutputClass, Refusal, Undo};
use doria_terminal::wire::{self, operation, ColorSupport, Observation};

use super::*;

/// The terminal a program is run against. Without one, standard input and
/// output are redirected streams, as for a program run through a pipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirTerminal {
    pub columns: u32,
    pub rows: u32,
    pub color_support: ColorSupport,
    /// What the event waits observe, in order. A script that has run out is
    /// a terminal that has gone away: the next wait observes `Closed`.
    pub events: Vec<Observation>,
    /// The terminal's answer to a cursor-position query, as a zero-based
    /// column and row. A terminal with no answer does not support the query.
    pub cursor: Option<(u32, u32)>,
}

#[derive(Default)]
pub(super) struct State {
    script: Option<MirTerminal>,
    observed: usize,
    /// The ledger. Output is captured the moment it is rendered, so an
    /// undone entry leaves at once.
    entries: u32,
    screen: u32,
    raw: u32,
}

impl State {
    pub(super) fn new(script: Option<MirTerminal>) -> Self {
        Self {
            script,
            ..Self::default()
        }
    }

    fn class(&self) -> OutputClass {
        self.script
            .as_ref()
            .map_or(OutputClass::NOT_A_TERMINAL, |script| {
                OutputClass::terminal(script.color_support)
            })
    }

    fn raw_mode_is(&self, generation: u32) -> bool {
        self.entries & ledger::RAW_MODE != 0 && self.raw == generation
    }
}

struct Scripted<'a> {
    state: &'a mut State,
    stdout: &'a mut Vec<u8>,
}

impl Host for Scripted<'_> {
    fn append(&mut self, bytes: &[u8]) -> Result<(), Refusal> {
        self.stdout.extend_from_slice(bytes);
        Ok(())
    }

    fn mark(&mut self, entry: u32) {
        self.state.entries |= entry;
    }

    fn unmark(&mut self, entry: u32) {
        self.state.entries &= !entry;
    }

    fn unmark_when_written(&mut self, entry: u32) {
        self.state.entries &= !entry;
    }

    fn active(&self, entry: u32) -> bool {
        self.state.entries & entry != 0
    }

    fn size(&mut self) -> Result<(u32, u32), Refusal> {
        match &self.state.script {
            Some(script) if script.columns != 0 && script.rows != 0 => {
                Ok((script.columns, script.rows))
            }
            Some(_) => Err(Refusal::unsupported()),
            None => Err(Refusal::not_a_terminal()),
        }
    }

    fn enter_screen(&mut self) -> u32 {
        self.state.screen += 1;
        self.state.screen
    }

    fn screen(&self) -> u32 {
        self.state.screen
    }
}

impl Interpreter<'_> {
    fn terminal_integer(&self, local: mir::LocalId) -> Result<i64, InterpreterError> {
        match read_local(&self.current_frame()?.locals, local)? {
            LocalValue::Scalar(mir::ScalarValue::Integer(value)) => Ok(value.bits as i64),
            _ => Err(InterpreterError::new(
                "terminal call operand is not an integer",
            )),
        }
    }

    pub(super) fn terminal_call(
        &mut self,
        call: mir::TerminalCall,
    ) -> Result<IntegerValue, InterpreterError> {
        let result = match call {
            mir::TerminalCall::Operate {
                operation,
                first,
                second,
                text,
            } => {
                let operation = self.terminal_integer(operation)? as u32;
                let first = self.terminal_integer(first)?;
                let second = self.terminal_integer(second)?;
                let LocalValue::String(text) = read_local(&self.current_frame()?.locals, text)?
                else {
                    return Err(InterpreterError::new(
                        "terminal operation text is not a string",
                    ));
                };
                let text = text.clone();
                console::result(self.terminal_operate(operation, first, second, text.as_bytes()))
            }
            mir::TerminalCall::AwaitEvent {
                generation,
                cancellation,
                ..
            } => {
                let generation = self.terminal_integer(generation)? as u32;
                let cancelled = match read_local(&self.current_frame()?.locals, cancellation)? {
                    LocalValue::NullableScalar {
                        value: Some(mir::ScalarValue::Inline(token)),
                        ..
                    } => self.cancellation_state_requested(token.bits)?,
                    LocalValue::NullableScalar { value: None, .. } => false,
                    _ => {
                        return Err(InterpreterError::new(
                            "terminal event wait has no nullable cancellation token",
                        ))
                    }
                };
                wire::success(self.terminal_observe(generation, cancelled).to_payload())
            }
        };
        Ok(IntegerValue::from_bits(IntegerType::Int64, result as u64))
    }

    fn terminal_operate(
        &mut self,
        operation: u32,
        first: i64,
        second: i64,
        text: &[u8],
    ) -> Result<u64, Refusal> {
        let state = &mut self.terminal;
        match operation {
            operation::ENTER_RAW_MODE => {
                if state.script.is_none() {
                    return Err(Refusal::not_a_terminal());
                }
                // Raw mode owns standard input: nothing else reads it, and
                // a transfer already reading it keeps raw mode out.
                if self.standard_claims[0].is_some() {
                    return Err(Refusal::in_use());
                }
                self.standard_claims[0] = Some(StandardClaim::RawMode);
                state.entries |= ledger::RAW_MODE;
                state.raw += 1;
                Ok(u64::from(state.raw))
            }
            operation::LEAVE_RAW_MODE => {
                if state.raw_mode_is(first as u32) {
                    state.entries &= !ledger::RAW_MODE;
                    self.standard_claims[0] = None;
                }
                Ok(0)
            }
            operation::CURSOR_POSITION => {
                if !state.raw_mode_is(first as u32) {
                    return Err(Refusal::in_use());
                }
                state
                    .script
                    .as_ref()
                    .and_then(|script| script.cursor)
                    .map(|(column, row)| wire::pair(column, row))
                    .ok_or_else(Refusal::unsupported)
            }
            // A scripted terminal stands behind all three standard streams.
            operation::STREAM_IS_INTERACTIVE => Ok(u64::from(state.script.is_some())),
            operation::COLOR_SUPPORT => Ok(state.class().color as u64),
            // Captured output has nowhere further to go.
            operation::FLUSH => Ok(0),
            _ => {
                let class = state.class();
                console::operate(
                    &mut Scripted {
                        state,
                        stdout: &mut self.stdout,
                    },
                    class,
                    operation,
                    first,
                    second,
                    text,
                )
            }
        }
    }

    /// A cancelled wait observes nothing and leaves the script where it was.
    fn terminal_observe(&mut self, generation: u32, cancelled: bool) -> Observation {
        let state = &mut self.terminal;
        if !state.raw_mode_is(generation) {
            return Observation::Closed;
        }
        if cancelled {
            return Observation::Cancelled;
        }
        let Some(script) = state.script.as_mut() else {
            return Observation::Closed;
        };
        let Some(observation) = script.events.get(state.observed).copied() else {
            return Observation::Closed;
        };
        state.observed += 1;
        // A resize is news about the window: the size asked for afterwards
        // is the new one.
        if let Observation::Resized { columns, rows } = observation {
            script.columns = columns;
            script.rows = rows;
        }
        observation
    }

    /// What the runtime does when a process ends: the terminal goes back the
    /// way the program found it.
    pub(super) fn terminal_ending(&mut self) {
        let entries = std::mem::take(&mut self.terminal.entries);
        if entries & ledger::RAW_MODE != 0 {
            self.standard_claims[0] = None;
        }
        self.stdout
            .extend_from_slice(Undo::new(entries, false).as_bytes());
    }
}
