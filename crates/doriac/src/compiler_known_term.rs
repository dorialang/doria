//! Compiler-owned `Doria\Std\Term` declarations (Decision 0140).
//!
//! The module is written in Doria over two runtime primitives: one terminal
//! operation and one event wait. Both return a packed integer whose layout
//! is `doria_terminal::wire`; the Doria below unpacks it and raises
//! `TerminalError`. `Size`, `Position`, `KeyModifiers` and `Style` are
//! one-word inline values built and read through a word intrinsic each.

use crate::ast::{ClassMember, CompilerIntrinsic, FunctionBody, Item, Program};
use crate::inline_value::TerminalValue;
use crate::names::{CompilerSymbolIdentity, GlobalSymbolFacts, GlobalSymbolOwner};
use crate::source::{SourceFile, SourceId};

pub const CONSOLE: &str = "Doria\\Std\\Term\\Console";
pub const RAW_MODE: &str = "Doria\\Std\\Term\\RawMode";
pub const ALTERNATE_SCREEN: &str = "Doria\\Std\\Term\\AlternateScreen";
pub const TERMINAL_ERROR: &str = "Doria\\Std\\Term\\TerminalError";
pub const TERMINAL_ERROR_REASON: &str = "Doria\\Std\\Term\\TerminalErrorReason";
pub const TERMINAL_WAIT_RESULT: &str = "Doria\\Std\\Term\\TerminalWaitResult";
pub const TERMINAL_EVENT: &str = "Doria\\Std\\Term\\TerminalEvent";
pub const KEY: &str = "Doria\\Std\\Term\\Key";
pub const KEY_MODIFIERS: &str = "Doria\\Std\\Term\\KeyModifiers";
pub const SIZE: &str = "Doria\\Std\\Term\\Size";
pub const POSITION: &str = "Doria\\Std\\Term\\Position";
pub const COLOR: &str = "Doria\\Std\\Term\\Color";
pub const COLOR_SUPPORT: &str = "Doria\\Std\\Term\\ColorSupport";
pub const LINE_WRAP: &str = "Doria\\Std\\Term\\LineWrap";
pub const STYLE: &str = "Doria\\Std\\Term\\Style";
pub const CANONICAL_TYPES: [&str; 15] = [
    CONSOLE,
    RAW_MODE,
    ALTERNATE_SCREEN,
    TERMINAL_ERROR,
    TERMINAL_ERROR_REASON,
    TERMINAL_WAIT_RESULT,
    TERMINAL_EVENT,
    KEY,
    KEY_MODIFIERS,
    SIZE,
    POSITION,
    COLOR,
    COLOR_SUPPORT,
    LINE_WRAP,
    STYLE,
];
pub const SOURCE_ID: SourceId = SourceId(0x7fff_fff9);
pub const SOURCE_NAME: &str = "<compiler-known:term>";

/// The members bound to the runtime instead of compiled from their Doria
/// body. Everything else in [`SOURCE_TEXT`] is ordinary Doria.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermFunction {
    /// `Console::operate`: one `dr_v1_terminal_operate` call.
    Operate,
    /// `Console::awaitEvent`: one `dr_v1_terminal_await_event` call.
    AwaitEvent,
    /// `fromBits` of a one-word value.
    FromWord(TerminalValue),
    /// The `bits` getter of a one-word value.
    Word(TerminalValue),
}

// The numbers in this text are `doria_terminal::wire`: operation selectors,
// result and observation layouts, and the bit fields of the four one-word
// values. `term_contract_tests.rs` runs this text against the Rust operation
// core, so a number that disagrees with that module fails there. A raw mode's
// watch identity is the runtime's `WatchSource` layout (Decision 0140).
pub const SOURCE_TEXT: &str = r#"
enum TerminalErrorReason {
    case NotATerminal;
    case InUse;
    case Unsupported;
    case DeviceFailure;
}
class TerminalError implements Error {
    function __construct(string $message, Doria\Std\Term\TerminalErrorReason $reason, ?int $systemCode = null) {}
}
enum ColorSupport {
    case None;
    case Basic;
    case Indexed;
    case TrueColor;
}
enum LineWrap {
    case Enabled;
    case Disabled;
}
enum Color {
    case Default;
    case Black;
    case Red;
    case Green;
    case Yellow;
    case Blue;
    case Magenta;
    case Cyan;
    case White;
    case BrightBlack;
    case BrightRed;
    case BrightGreen;
    case BrightYellow;
    case BrightBlue;
    case BrightMagenta;
    case BrightCyan;
    case BrightWhite;
    case Indexed(int $index);
    case Rgb(int $red, int $green, int $blue);
}
enum Key {
    case Character(string $text);
    case Enter;
    case Escape;
    case Backspace;
    case Tab;
    case Up;
    case Down;
    case Left;
    case Right;
    case Home;
    case End;
    case PageUp;
    case PageDown;
    case Insert;
    case Delete;
    case Function(int $number);
}
enum TerminalEvent {
    case Key(Doria\Std\Term\Key $key, Doria\Std\Term\KeyModifiers $modifiers);
    case Resized(Doria\Std\Term\Size $size);
}
enum TerminalWaitResult {
    case Event(Doria\Std\Term\TerminalEvent $event);
    case TimedOut;
    case Cancelled;
    case Closed;
}
class Size {
    internal function __construct() {}
    internal static function fromBits(int $bits): self {}
    internal int $bits { get {} }
    int $columns { get => $this->bits & 16777215; }
    int $rows { get => ($this->bits >> 24) & 16777215; }
}
class Position {
    internal function __construct() {}
    internal static function fromBits(int $bits): self {}
    internal int $bits { get {} }
    int $column { get => $this->bits & 16777215; }
    int $row { get => ($this->bits >> 24) & 16777215; }
}
class KeyModifiers {
    internal function __construct() {}
    internal static function fromBits(int $bits): self {}
    internal int $bits { get {} }
    bool $control { get => ($this->bits & 1) != 0; }
    bool $alt { get => ($this->bits & 2) != 0; }
    bool $shift { get => ($this->bits & 4) != 0; }
}
class Style {
    internal function __construct() {}
    internal static function fromBits(int $bits): self {}
    internal int $bits { get {} }

    static Doria\Std\Term\Style $plain { get => Doria\Std\Term\Style::fromBits(0); }

    function withForeground(Doria\Std\Term\Color $color): Doria\Std\Term\Style {
        return Doria\Std\Term\Style::fromBits(
            ($this->bits & ~67108863) | Doria\Std\Term\Console::colorBits($color));
    }
    function withBackground(Doria\Std\Term\Color $color): Doria\Std\Term\Style {
        return Doria\Std\Term\Style::fromBits(
            ($this->bits & ~(67108863 << 26)) | (Doria\Std\Term\Console::colorBits($color) << 26));
    }
    function withBold(): Doria\Std\Term\Style {
        return Doria\Std\Term\Style::fromBits($this->bits | (1 << 52));
    }
    function withDim(): Doria\Std\Term\Style {
        return Doria\Std\Term\Style::fromBits($this->bits | (2 << 52));
    }
    function withItalic(): Doria\Std\Term\Style {
        return Doria\Std\Term\Style::fromBits($this->bits | (4 << 52));
    }
    function withUnderline(): Doria\Std\Term\Style {
        return Doria\Std\Term\Style::fromBits($this->bits | (8 << 52));
    }
    function withReverse(): Doria\Std\Term\Style {
        return Doria\Std\Term\Style::fromBits($this->bits | (16 << 52));
    }
    function withStrikethrough(): Doria\Std\Term\Style {
        return Doria\Std\Term\Style::fromBits($this->bits | (32 << 52));
    }

    Doria\Std\Term\Color $foreground {
        get => Doria\Std\Term\Console::colorFromBits($this->bits & 67108863);
    }
    Doria\Std\Term\Color $background {
        get => Doria\Std\Term\Console::colorFromBits(($this->bits >> 26) & 67108863);
    }
    bool $isBold { get => ($this->bits & (1 << 52)) != 0; }
    bool $isDim { get => ($this->bits & (2 << 52)) != 0; }
    bool $isItalic { get => ($this->bits & (4 << 52)) != 0; }
    bool $isUnderlined { get => ($this->bits & (8 << 52)) != 0; }
    bool $isReversed { get => ($this->bits & (16 << 52)) != 0; }
    bool $isStruckThrough { get => ($this->bits & (32 << 52)) != 0; }
}
class AlternateScreen {
    internal writable int $generation;

    internal function __construct(parameter int $generation) {
        $this->generation = $generation;
    }

    consuming function leave(): void throws Doria\Std\Term\TerminalError {
        let writable $screen = $this;
        let $generation = $screen->generation;
        $screen->generation = 0;
        Doria\Std\Term\Console::run(21, $generation, 0, "");
    }

    // Leaving twice does nothing: the runtime ignores a generation that has
    // already left, and zero is never a generation.
    function __destruct(): void {
        let $ignored = Doria\Std\Term\Console::operate(21, $this->generation, 0, "");
    }
}
class RawMode implements Doria\Std\Io\Watchable {
    internal writable int $generation;

    internal function __construct(parameter int $generation) {
        $this->generation = $generation;
    }

    function getNextEvent(Doria\Std\Io\Wait $wait,
        ?Doria\Std\Concurrency\CancellationToken $cancellation = null): Doria\Std\Term\TerminalWaitResult throws Doria\Std\Term\TerminalError {
        let $observed = Doria\Std\Term\Console::payload(
            Doria\Std\Term\Console::awaitEvent($this->generation, $wait, $cancellation));
        let $kind = $observed & 15;
        if ($kind == 0) {
            return Doria\Std\Term\TerminalWaitResult::TimedOut;
        }
        if ($kind == 1) {
            return Doria\Std\Term\TerminalWaitResult::Closed;
        }
        if ($kind == 2) {
            return Doria\Std\Term\TerminalWaitResult::Cancelled;
        }
        if ($kind == 4) {
            return Doria\Std\Term\TerminalWaitResult::Event(Doria\Std\Term\TerminalEvent::Resized(
                Doria\Std\Term\Size::fromBits($observed >> 4)));
        }
        let $key = Doria\Std\Term\Console::key(($observed >> 4) & 255, $observed >> 16);
        let $modifiers = Doria\Std\Term\KeyModifiers::fromBits(($observed >> 12) & 7);
        return Doria\Std\Term\TerminalWaitResult::Event(
            Doria\Std\Term\TerminalEvent::Key($key, $modifiers));
    }

    function getCursorPosition(): Doria\Std\Term\Position throws Doria\Std\Term\TerminalError {
        return Doria\Std\Term\Position::fromBits(
            Doria\Std\Term\Console::run(23, $this->generation, 0, ""));
    }

    // What a transfer group waits on for this raw mode: watch kind 1 above
    // bit 32 and this raw mode's generation below.
    Doria\Std\Io\WatchSource $watchSource {
        get => Doria\Std\Io\WatchSource::fromBits(4294967296 | $this->generation);
    }

    consuming function leave(): void throws Doria\Std\Term\TerminalError {
        let writable $mode = $this;
        let $generation = $mode->generation;
        $mode->generation = 0;
        Doria\Std\Term\Console::run(19, $generation, 0, "");
    }

    function __destruct(): void {
        let $ignored = Doria\Std\Term\Console::operate(19, $this->generation, 0, "");
    }
}
class Console {
    internal function __construct() {}

    internal static function operate(int $operation, int $first, int $second, string $text): int {}
    internal static function awaitEvent(int $generation, Doria\Std\Io\Wait $wait,
        ?Doria\Std\Concurrency\CancellationToken $cancellation): int {}

    static function getSize(): Doria\Std\Term\Size throws Doria\Std\Term\TerminalError {
        return Doria\Std\Term\Size::fromBits(Doria\Std\Term\Console::run(0, 0, 0, ""));
    }

    // A standard output held by a pending transfer cannot be asked; it is
    // reported as having no color until it can.
    static Doria\Std\Term\ColorSupport $colorSupport {
        get {
            let $result = Doria\Std\Term\Console::operate(1, 0, 0, "");
            if (($result & 255) != 0) {
                return Doria\Std\Term\ColorSupport::None;
            }
            let $support = $result >> 8;
            if ($support == 1) {
                return Doria\Std\Term\ColorSupport::Basic;
            }
            if ($support == 2) {
                return Doria\Std\Term\ColorSupport::Indexed;
            }
            if ($support == 3) {
                return Doria\Std\Term\ColorSupport::TrueColor;
            }
            return Doria\Std\Term\ColorSupport::None;
        }
    }

    static function write(string $text, ?Doria\Std\Term\Style $style = null): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(2, Doria\Std\Term\Console::styleBits($style), 0, $text);
    }

    static function writeAt(int $column, int $row, string $text,
        ?Doria\Std\Term\Style $style = null): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(3, Doria\Std\Term\Console::cell($column, $row),
            Doria\Std\Term\Console::styleBits($style), $text);
    }

    static function flush(): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(4, 0, 0, "");
    }

    static function clear(): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(5, 0, 0, "");
    }

    static function clearLine(): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(6, 0, 0, "");
    }

    static function clearLineFromCursor(): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(7, 0, 0, "");
    }

    static function eraseAt(int $column, int $row): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(8, Doria\Std\Term\Console::cell($column, $row), 0, "");
    }

    static function setTitle(string $title): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(9, 0, 0, $title);
    }

    static function setLineWrap(Doria\Std\Term\LineWrap $wrap): void throws Doria\Std\Term\TerminalError {
        let $disabled = match ($wrap) {
            Doria\Std\Term\LineWrap::Enabled => 0,
            Doria\Std\Term\LineWrap::Disabled => 1,
        };
        Doria\Std\Term\Console::run(10, $disabled, 0, "");
    }

    static function moveTo(int $column, int $row): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(11, Doria\Std\Term\Console::cell($column, $row), 0, "");
    }

    static function moveUp(int $count = 1): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(12, Doria\Std\Term\Console::distance($count), 0, "");
    }

    static function moveDown(int $count = 1): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(13, Doria\Std\Term\Console::distance($count), 0, "");
    }

    static function moveLeft(int $count = 1): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(14, Doria\Std\Term\Console::distance($count), 0, "");
    }

    static function moveRight(int $count = 1): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(15, Doria\Std\Term\Console::distance($count), 0, "");
    }

    static function showCursor(): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(16, 0, 0, "");
    }

    static function hideCursor(): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(17, 0, 0, "");
    }

    static function enterRawMode(): Doria\Std\Term\RawMode throws Doria\Std\Term\TerminalError {
        return new Doria\Std\Term\RawMode(Doria\Std\Term\Console::run(18, 0, 0, ""));
    }

    static function enterAlternateScreen(): Doria\Std\Term\AlternateScreen throws Doria\Std\Term\TerminalError {
        return new Doria\Std\Term\AlternateScreen(Doria\Std\Term\Console::run(20, 0, 0, ""));
    }

    static function reset(): void throws Doria\Std\Term\TerminalError {
        Doria\Std\Term\Console::run(22, 0, 0, "");
    }

    internal static function run(int $operation, int $first, int $second, string $text): int throws Doria\Std\Term\TerminalError {
        return Doria\Std\Term\Console::payload(
            Doria\Std\Term\Console::operate($operation, $first, $second, $text));
    }

    // A result carries its status in the low byte. A failed one carries the
    // native error code above it when the system gave one.
    internal static function payload(int $result): int throws Doria\Std\Term\TerminalError {
        let $status = $result & 255;
        if ($status == 0) {
            return $result >> 8;
        }
        writable ?int $systemCode = null;
        if (($result & (1 << 40)) != 0) {
            $systemCode = ($result >> 8) & 4294967295;
        }
        if ($status == 1) {
            throw new Doria\Std\Term\TerminalError(
                "this operation needs a terminal, but the standard stream it uses is redirected; run the program in a terminal",
                Doria\Std\Term\TerminalErrorReason::NotATerminal, $systemCode);
        }
        if ($status == 2) {
            throw new Doria\Std\Term\TerminalError(
                "the terminal is held by another owner; leave the raw mode or alternate screen already entered, or finish the pending transfer on the standard stream",
                Doria\Std\Term\TerminalErrorReason::InUse, $systemCode);
        }
        if ($status == 3) {
            throw new Doria\Std\Term\TerminalError(
                "this terminal does not support the operation; use a terminal that accepts ANSI escape sequences (on Windows, Windows 10 or later)",
                Doria\Std\Term\TerminalErrorReason::Unsupported, $systemCode);
        }
        throw new Doria\Std\Term\TerminalError(
            "the terminal device failed; check that the terminal is still open",
            Doria\Std\Term\TerminalErrorReason::DeviceFailure, $systemCode);
    }

    internal static function styleBits(?Doria\Std\Term\Style $style): int {
        return ($style ?? Doria\Std\Term\Style::plain)->bits;
    }

    internal static function cell(int $column, int $row): int {
        if ($column < 0 || $row < 0) {
            panic("a terminal position cannot be negative; columns and rows start at 0");
        }
        return (Doria\Std\Term\Console::limited($column) << 32) | Doria\Std\Term\Console::limited($row);
    }

    internal static function distance(int $count): int {
        if ($count < 0) {
            panic("a cursor movement cannot be negative; use the opposite direction");
        }
        return Doria\Std\Term\Console::limited($count);
    }

    internal static function limited(int $value): int {
        if ($value > 16777215) {
            return 16777215;
        }
        return $value;
    }

    internal static function colorBits(Doria\Std\Term\Color $color): int {
        return match ($color) {
            Doria\Std\Term\Color::Default => 0,
            Doria\Std\Term\Color::Black => 1,
            Doria\Std\Term\Color::Red => 5,
            Doria\Std\Term\Color::Green => 9,
            Doria\Std\Term\Color::Yellow => 13,
            Doria\Std\Term\Color::Blue => 17,
            Doria\Std\Term\Color::Magenta => 21,
            Doria\Std\Term\Color::Cyan => 25,
            Doria\Std\Term\Color::White => 29,
            Doria\Std\Term\Color::BrightBlack => 33,
            Doria\Std\Term\Color::BrightRed => 37,
            Doria\Std\Term\Color::BrightGreen => 41,
            Doria\Std\Term\Color::BrightYellow => 45,
            Doria\Std\Term\Color::BrightBlue => 49,
            Doria\Std\Term\Color::BrightMagenta => 53,
            Doria\Std\Term\Color::BrightCyan => 57,
            Doria\Std\Term\Color::BrightWhite => 61,
            Doria\Std\Term\Color::Indexed($index) => 2 | (Doria\Std\Term\Console::component($index) << 2),
            Doria\Std\Term\Color::Rgb($red, $green, $blue) => 3
                | (Doria\Std\Term\Console::component($red) << 2)
                | (Doria\Std\Term\Console::component($green) << 10)
                | (Doria\Std\Term\Console::component($blue) << 18),
        };
    }

    internal static function component(int $value): int {
        if ($value < 0 || $value > 255) {
            panic("a color index or component must be between 0 and 255");
        }
        return $value;
    }

    internal static function colorFromBits(int $bits): Doria\Std\Term\Color {
        let $form = $bits & 3;
        let $value = $bits >> 2;
        if ($form == 0) {
            return Doria\Std\Term\Color::Default;
        }
        if ($form == 2) {
            return Doria\Std\Term\Color::Indexed($value & 255);
        }
        if ($form == 3) {
            return Doria\Std\Term\Color::Rgb($value & 255, ($value >> 8) & 255, ($value >> 16) & 255);
        }
        let $named = $value & 15;
        if ($named == 0) { return Doria\Std\Term\Color::Black; }
        if ($named == 1) { return Doria\Std\Term\Color::Red; }
        if ($named == 2) { return Doria\Std\Term\Color::Green; }
        if ($named == 3) { return Doria\Std\Term\Color::Yellow; }
        if ($named == 4) { return Doria\Std\Term\Color::Blue; }
        if ($named == 5) { return Doria\Std\Term\Color::Magenta; }
        if ($named == 6) { return Doria\Std\Term\Color::Cyan; }
        if ($named == 7) { return Doria\Std\Term\Color::White; }
        if ($named == 8) { return Doria\Std\Term\Color::BrightBlack; }
        if ($named == 9) { return Doria\Std\Term\Color::BrightRed; }
        if ($named == 10) { return Doria\Std\Term\Color::BrightGreen; }
        if ($named == 11) { return Doria\Std\Term\Color::BrightYellow; }
        if ($named == 12) { return Doria\Std\Term\Color::BrightBlue; }
        if ($named == 13) { return Doria\Std\Term\Color::BrightMagenta; }
        if ($named == 14) { return Doria\Std\Term\Color::BrightCyan; }
        return Doria\Std\Term\Color::BrightWhite;
    }

    internal static function key(int $code, int $value): Doria\Std\Term\Key {
        if ($code == 0) {
            return Doria\Std\Term\Key::Character(Doria\Std\Term\Console::text($value));
        }
        if ($code == 1) { return Doria\Std\Term\Key::Enter; }
        if ($code == 2) { return Doria\Std\Term\Key::Escape; }
        if ($code == 3) { return Doria\Std\Term\Key::Backspace; }
        if ($code == 4) { return Doria\Std\Term\Key::Tab; }
        if ($code == 5) { return Doria\Std\Term\Key::Up; }
        if ($code == 6) { return Doria\Std\Term\Key::Down; }
        if ($code == 7) { return Doria\Std\Term\Key::Left; }
        if ($code == 8) { return Doria\Std\Term\Key::Right; }
        if ($code == 9) { return Doria\Std\Term\Key::Home; }
        if ($code == 10) { return Doria\Std\Term\Key::End; }
        if ($code == 11) { return Doria\Std\Term\Key::PageUp; }
        if ($code == 12) { return Doria\Std\Term\Key::PageDown; }
        if ($code == 13) { return Doria\Std\Term\Key::Insert; }
        if ($code == 14) { return Doria\Std\Term\Key::Delete; }
        return Doria\Std\Term\Key::Function($value);
    }

    // One Unicode scalar value as UTF-8 text. The runtime only reports valid
    // scalars, so the replacement character is never the answer in practice.
    internal static function text(int $scalar): string {
        let writable $length = 4;
        if ($scalar < 128) {
            $length = 1;
        } else if ($scalar < 2048) {
            $length = 2;
        } else if ($scalar < 65536) {
            $length = 3;
        }
        writable Bytes $bytes = Bytes::allocate($length);
        if ($length == 1) {
            $bytes[0] = UInt8::from($scalar);
        } else if ($length == 2) {
            $bytes[0] = UInt8::from(192 | ($scalar >> 6));
            $bytes[1] = UInt8::from(128 | ($scalar & 63));
        } else if ($length == 3) {
            $bytes[0] = UInt8::from(224 | ($scalar >> 12));
            $bytes[1] = UInt8::from(128 | (($scalar >> 6) & 63));
            $bytes[2] = UInt8::from(128 | ($scalar & 63));
        } else {
            $bytes[0] = UInt8::from(240 | ($scalar >> 18));
            $bytes[1] = UInt8::from(128 | (($scalar >> 12) & 63));
            $bytes[2] = UInt8::from(128 | (($scalar >> 6) & 63));
            $bytes[3] = UInt8::from(128 | ($scalar & 63));
        }
        return String::fromBytes($bytes) ?? "�";
    }
}
"#;

fn value_class(name: &str) -> Option<TerminalValue> {
    TerminalValue::ALL
        .into_iter()
        .find(|value| value.name().rsplit('\\').next() == Some(name))
}

fn intrinsic(function: TermFunction, body: &FunctionBody) -> FunctionBody {
    FunctionBody::Intrinsic {
        implementation: CompilerIntrinsic::Term(function),
        span: body.span(),
    }
}

pub fn resolved_facts_use_term(facts: &GlobalSymbolFacts) -> bool {
    facts.references.iter().any(|reference| {
        matches!(
            reference.symbol_id.owner,
            GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardTerm(_))
        )
    })
}

fn declarations() -> &'static Program {
    static DECLARATIONS: std::sync::LazyLock<Program> = std::sync::LazyLock::new(|| {
        let mut program =
            crate::parse_source_file(&SourceFile::with_id(SOURCE_ID, SOURCE_NAME, SOURCE_TEXT))
                .expect("compiler-known Term declarations parse");
        for item in &mut program.items {
            match item {
                Item::Class(class) => {
                    let value = value_class(&class.name);
                    let console = class.name == "Console";
                    for member in &mut class.members {
                        match member {
                            ClassMember::Method(method) => {
                                let function = match (method.name.as_str(), value, console) {
                                    ("fromBits", Some(value), _) => TermFunction::FromWord(value),
                                    ("operate", _, true) => TermFunction::Operate,
                                    ("awaitEvent", _, true) => TermFunction::AwaitEvent,
                                    _ => continue,
                                };
                                method.body = intrinsic(function, &method.body);
                            }
                            ClassMember::Property(property) if property.name == "bits" => {
                                let Some(value) = value else { continue };
                                for hook in &mut property.hooks {
                                    hook.body = intrinsic(TermFunction::Word(value), &hook.body);
                                }
                            }
                            _ => {}
                        }
                    }
                    class.name = format!("Doria\\Std\\Term\\{}", class.name);
                }
                Item::Enum(declaration) => {
                    declaration.name = format!("Doria\\Std\\Term\\{}", declaration.name);
                }
                _ => unreachable!("compiler-known Term declarations are classes and enums"),
            }
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
    if !program.items.iter().any(|item| match item {
        crate::hir::Item::Class(class) => class.span.source == SOURCE_ID,
        crate::hir::Item::Enum(declaration) => declaration.span.source == SOURCE_ID,
        _ => false,
    }) {
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
