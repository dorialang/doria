//! Compiler-owned declarations; no runtime interface representation is introduced here.

use crate::ast::{InterfaceDecl, Item, Program};
use crate::source::{SourceFile, SourceId};

pub const SOURCE_ID: SourceId = SourceId(0x7fff_fffd);
pub const SOURCE_NAME: &str = "<compiler-known:contracts>";
pub const SOURCE_TEXT: &str = r#"
interface Displayable { function toString(): string; }
interface Error {}
interface Comparable<T> { function compare(T $other): Ordering; }
interface Equatable<T> { function equals(T $other): bool; }
interface Hashable { function hash(): uint64; }
interface Cloneable { function clone(): self; }
interface Iterable<T> { function iterator(): Iterator<T>; }
interface Iterator<T> {
    function hasCurrent(): bool;
    function current(): T;
    writable function advance(): void;
}
enum Ordering { case Less; case Equal; case Greater; }
"#;

pub fn declarations() -> &'static Program {
    static DECLARATIONS: std::sync::LazyLock<Program> = std::sync::LazyLock::new(|| {
        crate::parse_source_file(&SourceFile::with_id(SOURCE_ID, SOURCE_NAME, SOURCE_TEXT))
            .expect("compiler-known contract declarations parse")
    });
    &DECLARATIONS
}

pub fn interfaces() -> impl Iterator<Item = &'static InterfaceDecl> {
    declarations().items.iter().filter_map(|item| match item {
        Item::Interface(declaration) => Some(declaration),
        _ => None,
    })
}

pub fn requires_core_execution(name: &str) -> bool {
    matches!(
        name,
        "Comparable" | "Equatable" | "Hashable" | "Cloneable" | "Iterable" | "Iterator"
    )
}
