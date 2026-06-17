# 0006 Console and terminal applications

Status: Accepted

## Decision

`echo` remains supported for simple stdout output. `Console` is the first-class terminal abstraction for CLI and TUI work.

Console should support ANSI-capable terminals, bridge Windows console behavior where practical, and eventually support blocking and non-blocking input.

## Notes

Doria should support terminal games, full-screen TUI apps, exclusive terminal sessions, and portable terminal-game distribution as native executables where practical.

Atatusoft `termutil` is useful inspiration, but Doria should not copy PHP implementation details blindly.
