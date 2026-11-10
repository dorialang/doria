# Decision 0140: Terminal Layer, `Console`, And Raw Mode

**Status:** Accepted. Delivery is tracked in the end-to-end plan and
`docs/notes/current-pipeline.md`. The exact `Doria\Std\Io` spelling for a
terminal joining a transfer group is still to be settled (see "Waiting On A
Terminal And Other Sources").

## Approval And Boundary

On 2026-11-09 Andrew asked to "get started with the next stage" after the
Stage 36a delivery. The approved sequence puts Stage 46 next. On 2026-11-10
Andrew selected the following options, each on its own question:

- Event owner: **"The raw-mode guard (Recommended)"**.
- Event call: **"One call taking Wait (Recommended)"**.
- Restore scope: **"Guard, exits and signals (Recommended)"**.
- Shared wait: **"Join the 36a group wait (Recommended)"**.
- Windows: **"One VT renderer everywhere (Recommended)"**.
- v0 additions: **"Alternate screen (Recommended)"**, and no other addition.
- Build budget: **"12 hours, plus wasm32 target (Recommended)"**.

These approve the surface and runtime contract below. They change three
points of the plan's earlier wording: events are read from the raw-mode guard
through one `Wait`-taking call instead of `Console::readKey` and
`Console::pollKey`; restoration on panic and on termination signals is part of
this stage instead of future work; and the alternate screen is in v0.

The build budget is 12 hours of guarded build and test time shared by Stage
46, the wasm32 compiler build and the compiler-pin moves, under the Stage 36a
limits (one cargo job at a time, 3 GiB memory, the 25 GB cache ceiling,
offline and `--locked`), plus one download: the `wasm32-unknown-unknown`
target for the pinned toolchain. It approves no other install, no new
dependency, no pin change beyond the two compiler pins, and no publishing.

Later the same day Andrew settled three more points, each on its own
question:

- Missing crates: **"Yes, fetch them (Recommended)"**. One `cargo fetch
  --locked` for the ten crate versions that dependency updates had added to
  `Cargo.lock` and that this machine had never downloaded. Builds stay
  offline and locked afterwards.
- A cancelled event wait: **"A `Cancelled` case (Recommended)"**.
- Building a `Style`: **"Fluent `with…` methods (Recommended)"**.

This is the Console/terminal decision that plan §9 ("The terminal layer") and
the Stage 46 entry refer to. It settles the `Doria\Std\Term` v0 surface and the
runtime contract behind it.

## Context

Plan §9 and Stage 46 fix these points and this record does not reopen them:

- `Doria\Std\Term` is capability-based. `Console` is a static facade. No public
  `Doria\Std\Term` type carries escape sequences or platform encodings.
- The terminal is stateless: there is no `ScreenBuffer` standard type. A
  back-buffer or diffing renderer is userland.
- Raw mode is entered through an ownership guard whose `__destruct` restores
  the terminal on every structured exit.
- Stage 46 reuses Stage 36a's standard-stream views, readiness, blocking
  substrate, deadlines, cancellation and platform-device layer. It does not
  build a second readiness or polling path.
- Both platform backends land together. Acceptance is one interactive demo
  source (input echo, a moving glyph, resize handling) on Linux, macOS and
  Windows, driven by a pty harness on Unix CI and a ConPTY harness on Windows
  CI, with no escape sequence in the demo source.

Decision 0110 adds two rules: mutating a standard device's mode requires
exclusive control and a restoring guard, and `Doria\Std\Term` owns terminal raw
mode and terminal events while `Doria\Std\Io` owns generic blocking mode.

What exists today (develop `f5adbd2`):

- The runtime can tell whether a standard stream is a terminal
  (`dr_v1_stream_is_interactive`). Nothing in the compiler, the PHP backend or
  the documentation uses it.
- Exclusive use of a standard device is runtime-private: one `DeviceLoan` per
  device, held for one operation or for a transfer group's lifetime.
- There is no terminal-mode code of any kind: no termios, no window-size
  query, no resize signal, no console-mode change, no console input records,
  no pty or ConPTY harness.
- The runtime installs no signal handlers and has no exit hook. Every panic,
  `exit` and broken-pipe exit ends in one function, `abort_process`. A normal
  return from `main` goes back through the entry wrapper.
- The interpreter's I/O is in memory. The PHP backend reaches devices through
  FFI into the shared runtime.

## Ownership

The terminal is the device behind standard input and standard output. The
runtime's standard devices already own those handles, their input buffer, and
their exclusivity. Term adds one runtime module, the terminal state ledger,
and every terminal mutation goes through it.

- The ledger records the value each setting had before the program first
  changed it: the input mode, the output mode, cursor visibility, line wrap,
  the screen in use, and any reporting the program switched on.
- A guard restores its own entry when it ends. Anything still recorded when
  the process ends is restored by the runtime (see "Restoring The Terminal").
- Console never opens a second handle to the terminal. If standard input or
  standard output is redirected, the operations that need it fail with a typed
  error. Opening the controlling terminal behind redirected streams is a
  separate capability and is not part of v0; a second owner of the device
  behind standard input would break the single-owner rule above.
- "Is this stream a terminal" is a fact about a standard device, so it belongs
  to Io: each standard view gains a read-only `bool $isInteractive`. This is
  the `stream_isatty` row in the stream capability inventory. `Console`
  answers questions about the terminal itself (size, color support).

## Public Surface

Names follow the API guidelines: nouns are properties, verbs are methods,
fallible operations declare `throws`, modes are named, and waiting takes
`Doria\Std\Io\Wait`. Columns and rows are zero-based with `(0, 0)` at the top
left; the column comes first.

```doria
namespace Doria\Std\Term;

class Console {
    internal function __construct() {}

    // The terminal
    static function getSize(): Size throws TerminalError {}
    static ColorSupport $colorSupport { get {} }

    // Output
    static function write(string $text, ?Style $style = null): void throws TerminalError {}
    static function writeAt(int $column, int $row, string $text, ?Style $style = null): void throws TerminalError {}
    static function flush(): void throws TerminalError {}

    // Screen
    static function clear(): void throws TerminalError {}
    static function clearLine(): void throws TerminalError {}
    static function clearLineFromCursor(): void throws TerminalError {}
    static function eraseAt(int $column, int $row): void throws TerminalError {}
    static function setTitle(string $title): void throws TerminalError {}
    static function setLineWrap(LineWrap $wrap): void throws TerminalError {}

    // Cursor
    static function moveTo(int $column, int $row): void throws TerminalError {}
    static function moveUp(int $count = 1): void throws TerminalError {}
    static function moveDown(int $count = 1): void throws TerminalError {}
    static function moveLeft(int $count = 1): void throws TerminalError {}
    static function moveRight(int $count = 1): void throws TerminalError {}
    static function showCursor(): void throws TerminalError {}
    static function hideCursor(): void throws TerminalError {}

    // Modes
    static function enterRawMode(): RawMode throws TerminalError {}
    static function enterAlternateScreen(): AlternateScreen throws TerminalError {}
    static function reset(): void throws TerminalError {}
}

class AlternateScreen {
    internal function __construct() {}
    consuming function leave(): void throws TerminalError {}
    function __destruct(): void {}
}

class RawMode {
    internal function __construct() {}
    function getNextEvent(Wait $wait, ?CancellationToken $cancellation = null): TerminalWaitResult throws TerminalError {}
    function getCursorPosition(): Position throws TerminalError {}
    consuming function leave(): void throws TerminalError {}
    function __destruct(): void {}
}

enum TerminalWaitResult { case Event(TerminalEvent $event); case TimedOut; case Cancelled; case Closed; }
enum TerminalEvent { case Key(Key $key, KeyModifiers $modifiers); case Resized(Size $size); }
enum Key {
    case Character(string $text);
    case Enter; case Escape; case Backspace; case Tab;
    case Up; case Down; case Left; case Right;
    case Home; case End; case PageUp; case PageDown; case Insert; case Delete;
    case Function(int $number);
}
enum ColorSupport { case None; case Basic; case Indexed; case TrueColor; }
enum LineWrap { case Enabled; case Disabled; }
```

`Size` (`$columns`, `$rows`), `Position` (`$column`, `$row`), `KeyModifiers`
(`$control`, `$alt`, `$shift`), `Color` and `Style` are values. `Color` is an
ordinary enum with `Default`, the sixteen named colors, `Indexed(int $index)`
and `Rgb(int $red, int $green, int $blue)`. `Style` combines a foreground, a
background and text attributes (bold, dim, italic, underline, reverse,
strikethrough). `TerminalError` implements `Error` and carries a
`TerminalErrorReason`: not a terminal, in use, unsupported, or a device
failure.

A `Style` starts from `Style::plain`, the terminal's default colors with no
attributes, and each `with…` method returns a changed copy:

```doria
let $title = Style::plain
    ->withForeground(Color::Cyan)
    ->withBold();

Console::write("Saved\n", $title);
```

The methods are `withForeground(Color $color)`, `withBackground(Color
$color)`, `withBold()`, `withDim()`, `withItalic()`, `withUnderline()`,
`withReverse()` and `withStrikethrough()`. `$foreground`, `$background`,
`$isBold`, `$isDim`, `$isItalic`, `$isUnderlined`, `$isReversed` and
`$isStruckThrough` read a style back. There is no factory taking a cluster of
boolean flags.

`Console` has no event members. Reading a key requires a `RawMode`, so a
program that reads keys without raw mode does not compile. `Wait::Immediate`
is the poll and `Wait::Indefinite` is the blocking read. A wait whose
cancellation token is cancelled returns `Cancelled`: a typed observation of
the wait, as a timeout is, and it consumes no input.

### Output

- **Styling belongs to a write.** There is no "current style" on `Console`. A
  styled write renders its style, its text, and a return to the default
  attributes. A program cannot leave color behind by forgetting a reset.
- **Capability decides rendering.** A color the terminal cannot show is
  mapped to the nearest one it can. With `ColorSupport::None`, and whenever
  standard output is not a terminal, `write` emits the text alone. Cursor,
  screen and position operations on a non-terminal throw
  `TerminalError`; they have no plain-text meaning.
- **Output is buffered and ordered.** Console output goes through the standard
  output device, in order with `echo` and `StandardOutput` writes. It is
  flushed by `flush()`, before any wait for events, before any other write to
  the device, and at exit. A frame of several thousand `writeAt` calls costs
  one device write, which is what the performance target requires.
- **No allocation per styled write.** `Color`, `Style`, `Size`, `Position` and
  `KeyModifiers` are inline Copy values, like the standard views.
- **Console does not measure text.** Display width is a string question owned
  by the String API. Console does not clip, wrap or pad.

### Raw mode

- **Raw mode changes input only.** Line buffering, echo, signal keys and flow
  control are switched off on the input device. Output newline handling is the
  same in every mode on every platform, so `echo "a\n"` behaves identically
  inside and outside raw mode.
- **Raw mode holds standard input exclusively.** While a `RawMode` is alive,
  `read_line`, the byte intrinsics and `StandardInput` reads fail admission,
  the same way they do while a transfer group holds the device. A second
  `enterRawMode()` throws `TerminalError` (in use).
- **Interrupt keys are key events.** Control+C arrives as
  `Key::Character("c")` with `$control`, on every platform.
- **Operations that read a reply from the terminal live on the guard.**
  `getCursorPosition()` asks the terminal and reads the answer from input, so
  it is only safe for the owner of input.
- **Child processes never inherit a raw terminal.** `Command` gives every
  child three pipes; Stage 46 does not add terminal inheritance.

### Alternate screen

`Console::enterAlternateScreen()` switches to the terminal's second screen
and returns a guard. When the guard ends the first screen and its scrollback
are shown again. It is independent of raw mode: either may be held without the
other, and they may end in either order. A second `enterAlternateScreen()`
while one is alive throws `TerminalError` (in use).

### Events

- Keys and resizes are one ordered stream. A program that redraws on resize
  and moves on a key sees them in the order they happened.
- Resizes are coalesced: several resizes between two reads produce one
  `Resized` with the latest size.
- The portable key set is the same on all three platforms: characters, the
  named keys above, Control with a letter (except the letters a terminal
  cannot tell apart from Tab, Enter, Backspace and Escape), Alt with a key,
  and Shift with a named key where the terminal reports it. The Windows
  backend receives richer records than a Unix terminal sends; it reports the
  portable set so one program behaves the same everywhere. A wider set is a
  later capability.
- `Key::Character` carries one Unicode scalar value as a string. Windows
  surrogate pairs are joined before the event is reported.
- The event enums leave room for mouse, paste and focus cases without
  changing existing ones.

## Restoring The Terminal

- **Structured exit.** The guard's `__destruct` restores its entry. This is
  the plan's contract.
- **Runtime exits.** Before the runtime ends the process (normal return from
  `main`, `exit`, a broken-pipe exit, or a panic) it restores every ledger
  entry. On a panic the restore runs before the panic message is written, so
  the message is readable. This is the runtime putting back device state it
  changed; no Doria destructor runs, so Decision 0081's abort-only rule is
  unchanged.
- **Termination and suspension.** On Unix, the first terminal mutation
  installs handlers for the terminating signals and for stop/continue. A
  terminating signal restores the ledger and then re-delivers itself with the
  previous disposition; a stop restores and a continue re-applies. On
  Windows, the console control handler restores on close, logoff and
  shutdown. Handlers chain to whatever was installed before and are removed
  when the ledger is empty, so a program that never touches the terminal
  keeps today's behavior, and so does a host process that loads a Doria
  library.
- **Not recoverable.** A forced kill (`SIGKILL`, `TerminateProcess`) or power
  loss leaves the terminal as it was. No design can restore there.

The resize signal follows the same rules: its handler is installed when raw
mode is entered, chains to the previous one, and is removed when raw mode
ends. These are the runtime's first signal handlers.

## Backends

- **One renderer, one decoder.** The mapping from operations to terminal
  output and from Unix input bytes to events exists once, in the runtime. The
  native backends, the PHP backend (through FFI), and the interpreter all use
  it.
- **Unix.** termios for the input mode, the window-size query for size, the
  resize signal woken through the Stage 36a wait mechanism, the shared decoder
  for input. A lone Escape is told apart from the start of a sequence by a
  bounded delay.
- **Windows.** Console modes for input and output, `ReadConsoleInputW`
  records for keys and resizes. Output uses the same VT renderer as Unix: the
  runtime switches VT processing on, and a console that refuses it gets
  `TerminalError` (unsupported). There is no renderer on the legacy console
  attribute API.
- **Interpreter.** A scripted terminal joins the in-memory I/O: a size, a
  capability set, and a list of events. Output is captured as rendered. Parity
  fixtures run the same script through the interpreter and through native
  binaries under the pty harness and compare the captured output.
- **PHP.** Any use of `Doria\Std\Term` selects the native standard-I/O
  transport for the whole program, as any standard-view method does today.
- **Hosted terminals.** The scripted terminal is also the shape of a terminal
  supplied by a host, which is how the browser playground will run terminal
  programs. No browser work is part of Stage 46.

## Waiting On A Terminal And Other Sources

A program that shows a child's output while it accepts keys, or later reads a
socket while it accepts keys, has to wait for whichever is ready first. Stage
36a's `ReadinessSet` and `TransferGroup` are the only multi-source wait until
async exists, so terminal events join it in this stage:

- A `RawMode` can be registered with a `ReadinessSet`, and a group wait
  reports that terminal events are ready beside its transfer completions.
- The program then takes the events with `getNextEvent(Wait::Immediate)`.
  `getNextEvent` with any other `Wait` is the single-source form of the same
  runtime wait, not a second mechanism.
- This needs a completion kind that is not a byte transfer, which is an
  amendment to Decision 0110's group contract. The exact `Doria\Std\Io`
  spelling is not settled here. It will be recorded as an amendment to this
  record and to 0110 before the group work starts, and goes back to Andrew if
  it has real alternatives.

## Decisions Made During Implementation

These fill in points the sections above leave open. Each is the simplest
behavior that keeps those sections true, and each is covered by a test.

**Output**

- Waiting console output is also written before any read from standard input
  and before any write to standard error, so a prompt is on the screen before
  the program waits for its answer and the two output streams keep their
  order. Other waits (a sleep, a child process) do not flush; call `flush()`.
- A write that fails while nobody is waiting on it, such as one made on
  behalf of an `echo`, is reported by the next `Console` operation or
  `flush()`. A closed pipe ends the program with status 0, as for every
  ordinary write to standard output.
- `clear()` erases the screen and moves the cursor to the top left; the
  scrollback is left alone. `eraseAt` erases one cell and leaves the cursor
  on it. `setTitle` drops control characters from the title. `write` does not
  filter its text.
- `reset()` shows the cursor, switches line wrap on and returns to the
  default attributes. It does not end raw mode or the alternate screen, which
  their guards own, and it succeeds without effect when standard output is
  not a terminal.
- A movement count of zero does nothing. Counts and coordinates above
  16,777,215 are treated as that value.
- Color support on Unix comes from the environment: a non-empty `NO_COLOR`
  means none, as does an unset or `dumb` `TERM`; `COLORTERM` of `truecolor`
  or `24bit`, or a `TERM` naming `direct`, means true color; a `TERM` naming
  `256color` means indexed; anything else is basic. A Windows console that
  accepts the renderer's sequences has true color unless `NO_COLOR` is set.
- A true color shown on an indexed terminal maps to the 6×6×6 cube or the
  gray ramp, never to the sixteen named colors, whose appearance the user's
  theme decides.

**Input**

- The escape delay is 50 milliseconds. Byte `0x08` is Backspace, a line feed
  is Control+J, and the function keys are F1 to F12. The Meta modifier is not
  reported. Input that is not valid UTF-8 arrives as U+FFFD.
- Input the program had typed ahead before raw mode began is delivered as
  events first. Keys typed in raw mode and never asked for go back to
  standard input when raw mode ends.
- `getSize()` and `Resized` report the terminal behind standard output;
  `Resized` uses the one behind standard input when standard output is
  redirected. A terminal that reports no columns or no rows has no size, and
  the operation throws `TerminalError` (unsupported).
- `getCursorPosition()` waits one second for the terminal's answer and throws
  `TerminalError` (unsupported) without one. Keys typed around the answer stay
  queued in order. On Windows the console is asked directly and nothing is
  read from input.
- `Closed` is reported once the terminal behind standard input has gone
  away, after every key already read.

**Restoring**

- Cursor visibility, line wrap and the screen in use cannot be read back from
  a terminal. The ledger takes their starting values to be the ones a
  terminal gives a new program: visible, wrapping, main screen.
- Undoing a change through `Console` (`showCursor()` after `hideCursor()`,
  leaving the alternate screen) removes its ledger entry when the undoing
  output reaches the terminal, not when it is buffered.
- A signal handler cannot take the output device, so console output still
  waiting when a signal ends the program is not written. The restore itself
  is.
- A program in the background does not own the terminal, and a signal that
  arrives there restores nothing; the signal still does what it would have
  done.
- A stop is honored inside the handler. When the program runs again, because
  it was continued or because the system discarded the stop, it takes the
  terminal again: the input mode if the shell changed it, the ledger's
  sequences if the stop had undone them, and one `Resized` event so the
  program draws again. A continue after a stop that cannot be caught is
  handled the same way.
- A signal that was ignored when the first terminal change was made keeps
  being ignored.
- On Windows the control handler also restores on Control+C and
  Control+Break when the program is not in raw mode, since those end the
  process the same way close does.

## Scope

In v0: everything under "Public Surface" including the alternate screen, the
ledger and full restoration, the group wait, the backends, the interpreter's
scripted terminal, the pty and ConPTY harnesses, the demo, and
`$isInteractive` on the standard views.

Later slices, each with its own acceptance: mouse events, an interactive line
editor (the `readline_*` rows of the stream capability inventory, which stay
assigned to `Doria\Std\Term`), and scroll regions.

Not in v0: setting the terminal size (terminals rarely honor it), a screen
buffer or read-back of screen contents, opening the controlling terminal
behind redirected streams, terminal inheritance for child processes, and
keyboard reporting beyond the portable key set.

## Required Before Delivery

- The plan's Stage 46 integration prerequisite: acceptance tracking in the
  compiler, tooling and website must select stages by explicit completed
  stage and slice identity. Stage 46 is delivered before Stages 37 to 45, so
  "highest stage number" is wrong from this stage on.
- `docs/stdlib-reference.md`, the stream capability inventory, the I/O
  surface audit, `SPEC.md` where it lists standard namespaces, and the
  completeness and docs-authority guards.
- Language server: hover and documentation for the new compiler-known module,
  the new inline value kinds, and editor fixtures.
- Website: the `Doria\Std\Term` API pages, which today describe `readKey`,
  `pollKey` and a `KeyEvent` that includes resizes.
- CI: the pty harness on the Linux and macOS jobs and the ConPTY harness on
  the Windows job.
