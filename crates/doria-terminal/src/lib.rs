#![cfg_attr(not(test), no_std)]

//! The terminal layer's pure half (Decision 0140), shared by the runtime and
//! the compiler's debug interpreter: the integer transport between the
//! authored `Doria\Std\Term` module and the layer, the one renderer, the one
//! key decoder, and what each `Console` operation means. Nothing here touches
//! a device, allocates or keeps state, so a real terminal and a scripted one
//! behave the same by construction.

pub mod console;
pub mod decode;
pub mod render;
pub mod wire;
