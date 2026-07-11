//! Shared helpers for the implementation-private native function ABI.

use crate::mir;

pub fn function_symbol(function: &mir::Function) -> String {
    let sanitized = function
        .name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("__doria_fn_{}_{}", function.id.0, sanitized)
}
