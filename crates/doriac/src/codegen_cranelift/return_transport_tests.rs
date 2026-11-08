//! Doria callable signatures return at most the shared direct budget on every
//! host, and every durable native program lowers for every tier-1 target from
//! any host. AArch64 has eight return registers and accepts a three-slot
//! return that x86-64 rejects, so these checks hold every target's limits on a
//! host that cannot reproduce them by running.

use cranelift_codegen::ir::{types, AbiParam};
use cranelift_module::{default_libcall_names, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use super::{
    append_type_abi_params, function_signature, indirect_function_signature,
    lower_validated_mir_to_object_for_isa, native_flags,
};
use crate::mir;
use crate::native_closure_abi::{
    self, direct_value_slots, DirectValueSlots, NativeCallableHiddenInput,
    NativeCallableSignaturePlan, DIRECT_RETURN_SLOTS,
};

const SOURCE: &str = r#"
use Doria\Std\Io\ByteRegion;
use Doria\Std\Time\{ClockError, Deadline, Duration};
class Packet {
    Bytes $bytes = Bytes::allocate(2);
    function optional(): ?ByteRegion { return $this->bytes->getRegion(0, 1); }
}
function optional(Bytes $source, bool $present): ?ByteRegion {
    if ($present) { return $source->getRegion(0, 1); }
    return null;
}
function later(Deadline $deadline): Deadline { return $deadline; }
function pick(Deadline $deadline, bool $present): ?Deadline {
    if ($present) { return $deadline; }
    return null;
}
function through(function(?ByteRegion): ?ByteRegion $call, ?ByteRegion $value): ?ByteRegion {
    return $call($value);
}
function main(): void throws ClockError {
    let $deadline = Deadline::fromNow(Duration::fromNanoseconds(5));
    if (pick($deadline, false) == null && pick($deadline, true) != null) { echo "picked\n"; }
    if (later($deadline) == $deadline) { echo "later\n"; }
    let $bytes = Bytes::allocate(2);
    let $call = fn(?ByteRegion $value) => $value;
    let $view = through($call, optional($bytes, true));
    if ($view != null) { echo "{$view->length}\n"; }
    let $packet = new Packet();
    let $own = $packet->optional();
    if ($own != null) { echo "{$own->length}\n"; }
}
"#;

fn module() -> ObjectModule {
    let isa = cranelift_native::builder()
        .unwrap()
        .finish(native_flags().unwrap())
        .unwrap();
    ObjectModule::new(ObjectBuilder::new(isa, "return-transport", default_libcall_names()).unwrap())
}

/// The register slots a Cranelift parameter or return list uses. A 128-bit
/// integer takes two integer registers on every supported target.
fn slots(params: &[AbiParam]) -> DirectValueSlots {
    params
        .iter()
        .fold(DirectValueSlots { total: 0, float: 0 }, |slots, param| {
            DirectValueSlots {
                total: slots.total
                    + if param.value_type == types::I128 {
                        2
                    } else {
                        1
                    },
                float: slots.float + u32::from(param.value_type.is_float()),
            }
        })
}

#[test]
fn every_callable_signature_returns_within_the_direct_budget() {
    let program = crate::lower_source_to_mir("return-transport.doria", SOURCE)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let mut module = module();
    for function in &program.functions {
        let signature = function_signature(&mut module, &program, function).unwrap();
        assert!(
            slots(&signature.returns).fits(DIRECT_RETURN_SLOTS),
            "{} returns {:?}",
            function.name,
            signature.returns
        );
    }
    for function_type in &program.function_types {
        for environment in [false, true] {
            let signature =
                indirect_function_signature(&mut module, &program, function_type, environment);
            assert!(
                slots(&signature.returns).fits(DIRECT_RETURN_SLOTS),
                "function type#{} returns {:?}",
                function_type.id.0,
                signature.returns
            );
        }
    }
}

#[test]
fn values_wider_than_the_budget_return_through_the_result_slot() {
    let program = crate::lower_source_to_mir("return-transport.doria", SOURCE)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let mut wide = Vec::new();
    for function in &program.functions {
        let mir::ReturnType::Value(ty) = function.return_type else {
            continue;
        };
        if direct_value_slots(ty).fits(DIRECT_RETURN_SLOTS) {
            continue;
        }
        let plan = NativeCallableSignaturePlan::direct(function);
        assert!(
            plan.index_of(NativeCallableHiddenInput::ResultOut)
                .is_some(),
            "{} returns {ty:?} without a result slot",
            function.name
        );
        assert!(!plan.returns_value_directly());
        // A checked callee (the closure carries ambient I/O) always used the
        // slot; an unchecked one uses it because of this rule.
        if !plan.checked {
            wide.push(ty);
        }
    }
    // Unchecked ?ByteRegion from a free function and a method, and ?Deadline.
    assert!(wide
        .iter()
        .any(|ty| matches!(ty, mir::Type::NullableByteRegion(_))));
    assert!(wide
        .iter()
        .any(|ty| matches!(ty, mir::Type::NullableScalar(mir::ScalarType::Inline(_)))));
}

#[test]
fn the_shared_shape_bounds_every_cranelift_carrier() {
    // `direct_value_slots` decides the result slot for both backends, so it must
    // never undercount the registers Cranelift's carrier uses.
    let program = crate::lower_source_to_mir("return-transport.doria", SOURCE)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let pointer = module().target_config().pointer_type();
    for function in &program.functions {
        for local in &function.locals {
            let mut params = Vec::new();
            append_type_abi_params(&mut params, &program, local.ty, pointer);
            let carrier = slots(&params);
            let shape = native_closure_abi::direct_value_slots(local.ty);
            assert!(
                carrier.total <= shape.total && carrier.float <= shape.float,
                "{:?}: carrier {carrier:?} exceeds shape {shape:?}",
                local.ty
            );
        }
    }
}

/// The Cranelift targets of the tier-1 platforms: System V and ELF on Linux
/// (x86-64 and AArch64), fastcall and COFF on Windows x64, AAPCS64 and Mach-O
/// on macOS. Windows on ARM64 is absent because cranelift-object 0.134 has no
/// AArch64 COFF symbol relocations (`Aarch64AdrGotPage21`,
/// `Aarch64AdrPrelPgHi21`, `Aarch64AddAbsLo12Nc` are ELF and Mach-O only), so
/// the fast profile cannot emit that target.
const TIER_ONE_TARGETS: [&str; 4] = [
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-unknown-linux-gnu",
    "aarch64-apple-darwin",
];

fn lower_for_every_tier_one_target(name: &str, program: &mir::Program) {
    for target in TIER_ONE_TARGETS {
        let isa = cranelift_codegen::isa::lookup_by_name(target)
            .unwrap_or_else(|error| panic!("{target} is not compiled in: {error:?}"));
        lower_validated_mir_to_object_for_isa(program, isa, None)
            .unwrap_or_else(|error| panic!("{name} does not lower for {target}: {error:?}"));
    }
}

#[test]
fn wide_returns_lower_for_every_tier_one_target() {
    let program = crate::lower_source_to_mir("return-transport.doria", SOURCE)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    lower_for_every_tier_one_target("return-transport.doria", &program);
}

#[test]
fn the_durable_native_manifest_lowers_for_every_tier_one_target() {
    // The executable parity matrix runs each program on the host only. Lowering
    // it for every target keeps a target-specific ABI limit from reaching CI
    // first. Programs are independent, so they lower on every available core.
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = crate_root.join("../..");
    let manifest =
        std::fs::read_to_string(crate_root.join("tests/fixtures/native_parity_examples.txt"))
            .expect("the durable native manifest is readable");
    let entries = manifest
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    assert!(
        !entries.is_empty(),
        "the durable native manifest names no programs"
    );
    // Four workers keep the test inside the vet's memory limit beside the rest
    // of the library suite.
    let workers = std::thread::available_parallelism().map_or(1, |cores| usize::from(cores).min(4));
    std::thread::scope(|scope| {
        for chunk in entries.chunks(entries.len().div_ceil(workers)) {
            let repository = &repository;
            scope.spawn(move || {
                for entry in chunk {
                    let path = repository.join(entry);
                    let source = std::fs::read_to_string(&path)
                        .unwrap_or_else(|error| panic!("{entry} is unreadable: {error}"));
                    let program = crate::lower_source_to_mir(path.to_string_lossy(), source)
                        .unwrap_or_else(|errors| {
                            panic!("{entry} does not lower to MIR: {errors:#?}")
                        });
                    lower_for_every_tier_one_target(entry, &program);
                }
            });
        }
    });
}
