//! Runtime data symbols in Windows objects (Decision 0148 section 23): an
//! executable links the runtime archive and names the symbol itself, while
//! the companion DLL reaches the runtime DLL's data through its import
//! slot, which is all an import library defines for data.

use super::lower_validated_mir_to_object_for_isa;
use crate::coff_symbols::undefined_external_symbols;
use crate::native_abi::FOREIGN_CALL_FRAME;

/// A region whose foreign call stores the Call Path record. LLVM knows no
/// meaning for `getpid`, so the call and its record survive optimization.
pub(crate) const FOREIGN_CALL_IN_A_REGION: &str = r#"
extern "C"
{
    /** @safety Always safe. */
    function getpid(): int32;
}

function main(): void
{
    writable int32 $value = 0;
    unsafe {
        $value = getpid();
    }
    echo "{$value}\n";
}
"#;

#[test]
fn windows_objects_reach_the_call_path_record_as_they_link_the_runtime() {
    let program = crate::lower_source_to_mir("record.doria", FOREIGN_CALL_IN_A_REGION)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let library = crate::mir::companion::library(&program);
    let isa = || cranelift_codegen::isa::lookup_by_name("x86_64-pc-windows-msvc").unwrap();
    let import_slot = format!("__imp_{FOREIGN_CALL_FRAME}");

    let executable = lower_validated_mir_to_object_for_isa(&program, isa(), None, None).unwrap();
    let imports = undefined_external_symbols(&executable);
    assert!(
        imports.iter().any(|name| name == FOREIGN_CALL_FRAME),
        "{imports:?}"
    );
    assert!(!imports.contains(&import_slot), "{imports:?}");

    let companion =
        lower_validated_mir_to_object_for_isa(&library.program, isa(), None, Some(&library))
            .unwrap();
    let imports = undefined_external_symbols(&companion);
    assert!(imports.contains(&import_slot), "{imports:?}");
    assert!(
        !imports.iter().any(|name| name == FOREIGN_CALL_FRAME),
        "{imports:?}"
    );
}
