//! Test support for the terminal layer (Decision 0140): run a program inside
//! a pseudo-terminal on any tier-1 platform, type at it, and read what it
//! drew.
//!
//! A Unix pty hands over the program's bytes exactly as written. Windows
//! ConPTY is itself a terminal: it parses the program's output and sends a
//! rendering of the resulting screen, which is not the same bytes. What is
//! the same on every platform is the screen, so [`Screen`] is the common
//! ground: feed it either stream and compare what it shows.

mod screen;
mod session;

pub use screen::Screen;
pub use session::{Exit, Session};
