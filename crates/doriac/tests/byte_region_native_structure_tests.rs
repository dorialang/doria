use std::collections::{BTreeMap, BTreeSet};

use doriac::mir::{ReturnType, Type};
use doriac::native_closure_abi::{NativeCallableHiddenInput, NativeCallableSignaturePlan};
use doriac::types::{ByteRegionKind, ReturnBorrowKind};

#[path = "common/native_structure.rs"]
mod native_structure;
use native_structure::reachable_references;

const SOURCE: &str = r#"
use Doria\Std\Io\{ByteRegion, WritableByteRegion};

function pair(ByteRegion $view): ByteRegion { return $view; }
function triple(?ByteRegion $view): ?ByteRegion { return $view; }

function main(): void {
    writable Bytes $bytes = Bytes::allocate(4);
    {
        let writable $whole = $bytes->getWritableRegion(0, 4);
        {
            let writable $part = $whole->getWritableRegion(1, 2);
            $part[0] = 41;
            uint8 $observed = $part[0];
        }
    }
    {
        let $view = pair($bytes->getRegion(0, 0));
        let $optional = triple($view);
        if ($optional != null) { int $length = $optional->length; }
    }
}
"#;

fn lower() -> doriac::mir::Program {
    doriac::lower_source_to_mir("byte-region-native-structure.doria", SOURCE)
        .unwrap_or_else(|errors| panic!("{SOURCE}\n{errors:#?}"))
}

#[test]
fn reference_scope_excludes_unreachable_constructors_but_follows_helpers() {
    let graph = BTreeMap::from([
        ("authored".to_owned(), BTreeSet::from(["helper".to_owned()])),
        (
            "helper".to_owned(),
            BTreeSet::from(["forbidden-copy".to_owned(), "authored".to_owned()]),
        ),
        (
            "unrelated-constructor".to_owned(),
            BTreeSet::from(["unrelated-retain".to_owned()]),
        ),
    ]);
    let references = reachable_references(&graph, vec!["authored".to_owned()]);
    assert!(references.contains("forbidden-copy"));
    assert!(!references.contains("unrelated-retain"));
}

fn assert_region_runtime_references(references: BTreeSet<String>, context: &str) {
    use doriac::native_abi::*;
    assert!(
        references
            .iter()
            .all(|target| !target.starts_with("unresolved text target")),
        "cannot establish exact region helper reachability: {references:?}\n{context}"
    );
    for required in [
        BYTES_ALLOCATE,
        BYTES_REGION,
        BYTE_REGION_SUBREGION,
        BYTE_REGION_GET,
        BYTE_REGION_SET,
    ] {
        assert!(
            references.contains(required),
            "missing {required}: {references:?}\n{context}"
        );
    }
    // The owner allocation is intentional. None of these operations may be
    // introduced merely to form, move, return, narrow, or drop a region.
    for forbidden in [
        BYTES_COPY,
        BYTES_FROM_COLLECTION,
        BYTES_TO_COLLECTION,
        CLASS_ALLOCATE,
        CLOSURE_ENVIRONMENT_ALLOCATE,
        COLLECTION_NEW,
        COLLECTION_STAGE26_NEW,
        COLLECTION_AGGREGATE_NEW,
        CORE_COLLECTION_NEW,
        COLLECTION_STAGE26_FROM_COPY,
        STRING_RETAIN,
        SHARED_RETAIN,
        WRITABLE_SHARED_RETAIN,
        MIXED_NEW,
        MIXED_NEW_BORROWED,
        MIXED_NEW_AGGREGATE,
        MIXED_NEW_AGGREGATE_BORROWED,
    ] {
        assert!(
            !references.contains(forbidden),
            "region transport references {forbidden}: {references:?}\n{context}"
        );
    }
}

#[test]
fn native_region_layout_and_returns_use_inline_carriers_not_borrow_home_results() {
    for kind in [ByteRegionKind::Readonly, ByteRegionKind::Writable] {
        for word in [4, 8] {
            for (ty, words) in [
                (Type::ByteRegion(kind), 2),
                (Type::NullableByteRegion(kind), 3),
            ] {
                let layout = doriac::native_closure_abi::type_layout(ty, word);
                assert_eq!(layout.size, word * words);
                assert_eq!(layout.align, word);
                assert!(!doriac::native_closure_abi::needs_drop(ty));
            }
        }
    }

    // A retained region returns its carrier, never a borrow-home result. The
    // two-word pair fits the direct return budget and stays in registers. The
    // three-word nullable triple exceeds it and returns through the caller's
    // result slot (Decision log: "Returns wider than two registers take the
    // result slot").
    let mir = lower();
    for (name, expected, hidden_inputs) in [
        (
            "pair",
            Type::ByteRegion(ByteRegionKind::Readonly),
            vec![NativeCallableHiddenInput::CurrentFrame],
        ),
        (
            "triple",
            Type::NullableByteRegion(ByteRegionKind::Readonly),
            vec![
                NativeCallableHiddenInput::CurrentFrame,
                NativeCallableHiddenInput::ResultOut,
            ],
        ),
    ] {
        let function = mir
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        assert_eq!(function.return_type, ReturnType::Value(expected));
        assert_eq!(function.locals[function.params[0].0].ty, expected);
        assert_eq!(
            function.return_borrow.unwrap().kind,
            ReturnBorrowKind::Retained
        );
        let signature = NativeCallableSignaturePlan::direct(function);
        assert_eq!(signature.hidden_inputs, hidden_inputs, "{name}");
        assert!(!signature.hidden_inputs.iter().any(|input| matches!(
            input,
            NativeCallableHiddenInput::BorrowHome | NativeCallableHiddenInput::ResultBorrowHomeOut
        )));
        assert_eq!(
            signature.returns_value_directly(),
            doriac::native_closure_abi::direct_value_slots(expected)
                .fits(doriac::native_closure_abi::DIRECT_RETURN_SLOTS),
            "{name}"
        );
    }
}

#[test]
fn cranelift_regions_reference_only_nonowning_region_runtime_operations() {
    let mir = lower();
    let graph = native_structure::cranelift_references(&mir);
    // Relocation references are not dynamic calls or even call instructions:
    // AArch64 PIC can emit two GOT relocations for one address materialization.
    assert_region_runtime_references(
        reachable_references(
            &graph,
            native_structure::authored_symbols(&mir, &["main", "pair", "triple"]),
        ),
        "Cranelift byte-region references",
    );
}

#[cfg(feature = "llvm-backend")]
#[test]
fn llvm_region_calls_and_signatures_preserve_pair_and_explicit_presence_triple() {
    let mir = lower();
    let (ir, graph) = native_structure::llvm_references(&mir);
    assert_region_runtime_references(
        reachable_references(
            &graph,
            native_structure::authored_symbols(&mir, &["main", "pair", "triple"]),
        ),
        &ir,
    );

    // The pair returns its `{ ptr, word }` carrier directly. The presence
    // triple exceeds the direct return budget: its definition returns `void`
    // and writes the `{ word, { ptr, word } }` carrier through the result slot
    // after the current frame, while its parameter keeps the explicit triple.
    let word = format!("i{}", usize::BITS);
    let pair = format!("{{ ptr, {word} }}");
    let triple = format!("{{ {word}, {pair} }}");
    for (name, carrier, direct) in [("pair", pair, true), ("triple", triple, false)] {
        let function = mir
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        let symbol = doriac::native_abi::function_symbol(function);
        let signature = ir
            .lines()
            .find(|line| line.starts_with("define ") && line.contains(&format!("@{symbol}(")))
            .unwrap_or_else(|| panic!("missing {symbol}: {ir}"));
        let (returned, parameters) = signature
            .split_once(&format!("@{symbol}("))
            .and_then(|(returned, rest)| Some((returned, rest.split_once(')')?.0)))
            .unwrap_or_else(|| panic!("{name} has no parameter list: {signature}"));
        // The hidden inputs (the current frame, then any result slot) lead the
        // parameters as pointers; the region parameter keeps its carrier.
        let hidden = NativeCallableSignaturePlan::direct(function)
            .hidden_inputs
            .len();
        let mut rest = parameters;
        for _ in 0..hidden {
            assert!(
                rest.starts_with("ptr "),
                "{name}'s hidden inputs must be pointers: {signature}"
            );
            rest = rest
                .split_once(", ")
                .map(|(_, rest)| rest)
                .unwrap_or_else(|| panic!("{name} has no source parameter: {signature}"));
        }
        assert!(
            rest.starts_with(&format!("{carrier} ")),
            "{name}'s parameter must keep {carrier}: {signature}"
        );
        if direct {
            assert!(
                returned.contains(&carrier),
                "{name} must return {carrier}: {signature}"
            );
        } else {
            assert!(
                returned.ends_with(" void "),
                "{name} must return through its result slot: {signature}"
            );
        }
    }
}
