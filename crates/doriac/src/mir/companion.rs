//! The native companion library's plan (Decision 0148 sections 22 and 23,
//! entry ABI version 1): one C-ABI entry per region identity, shared by
//! every lowering of the region, and the table of host operations its
//! regions call back through. Lowering builds the plan once every region
//! has its inputs and outputs, and MIR validation derives it again and
//! requires the same result, so the plan has one owner.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Not;

use super::local_access::{statement_touches, terminator_touches, HostReach, LocalAccess};
use super::*;

/// A region whose lowerings disagree, or a lowering that names a region it
/// does not have: the span of the region and what differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionPlanError {
    pub span: Span,
    pub details: String,
}

/// The companion plan of `program`. Regions group by identity in identity
/// order; the first lowering of each, in function order, is the one the
/// companion compiles, and every other lowering must have the same inputs,
/// outputs, result and exits. Host operations are numbered from 1 in region
/// order and then block and statement order; 0 is reserved for accepting a
/// string output.
pub fn plan(program: &Program) -> Result<Companion, CompanionPlanError> {
    let mut groups = BTreeMap::<&str, Vec<RegionLowering>>::new();
    for function in &program.functions {
        for (index, region) in function.unsafe_regions.iter().enumerate() {
            groups
                .entry(region.id.as_str())
                .or_default()
                .push(RegionLowering {
                    function: function.id,
                    region: index,
                });
        }
    }
    let parameter_modes = program
        .functions
        .iter()
        .map(|function| (function.id, function.parameter_modes.as_slice()))
        .collect::<HashMap<_, _>>();
    let modes = |function: FunctionId, position: usize| {
        parameter_modes
            .get(&function)
            .and_then(|modes| modes.get(position))
            .copied()
    };
    let function = |id: FunctionId| {
        program
            .functions
            .iter()
            .find(|function| function.id == id)
            .expect("a grouped lowering names an emitted function")
    };
    let bindings = program
        .functions
        .iter()
        .filter(|function| function.is_foreign_binding())
        .map(|function| function.id)
        .collect::<HashSet<_>>();
    let host = |reach: HostReach| match reach {
        HostReach::Call(Some(function)) => !bindings.contains(&function),
        HostReach::Call(None) | HostReach::Construct | HostReach::Static | HostReach::Io => true,
    };
    let mut companion = Companion::default();
    for (id, lowerings) in groups {
        let canonical = function(lowerings[0].function);
        let region = &canonical.unsafe_regions[lowerings[0].region];
        let shape = RegionShape::of(canonical, region);
        for lowering in &lowerings[1..] {
            let other = function(lowering.function);
            let other_shape = RegionShape::of(other, &other.unsafe_regions[lowering.region]);
            if other_shape != shape {
                return Err(CompanionPlanError {
                    span: region.span,
                    details: format!(
                        "region {id} has {shape:?} in `{}` but {other_shape:?} in `{}`",
                        canonical.name, other.name
                    ),
                });
            }
        }
        let index = companion.regions.len();
        for block in &region.blocks {
            let block = &canonical.blocks[block.0];
            let sites = block
                .statements
                .iter()
                .enumerate()
                .map(|(position, statement)| {
                    let mut touches = Touches::default();
                    statement_touches(
                        statement,
                        &modes,
                        &mut |local, access| Touches::local(&mut touches.locals, local, access),
                        &mut |reach| touches.reaches_host |= host(reach),
                    );
                    (HostOperationSite::Statement(position), touches)
                })
                .chain(leaves_function(&block.terminator).not().then(|| {
                    let mut touches = Touches::default();
                    terminator_touches(
                        &block.terminator,
                        &modes,
                        &mut |local, access| Touches::local(&mut touches.locals, local, access),
                        &mut |reach| touches.reaches_host |= host(reach),
                    );
                    (HostOperationSite::Terminator, touches)
                }));
            for (site, touches) in sites {
                if let Some((arguments, results)) =
                    touches.host_operation(canonical, &program.collection_types)
                {
                    let successors = match site {
                        HostOperationSite::Statement(_) => Vec::new(),
                        HostOperationSite::Terminator => block.terminator.targets(),
                    };
                    companion.host_operations.push(HostOperation {
                        id: companion.host_operations.len() as u32 + 1,
                        region: index,
                        block: block.id,
                        site,
                        arguments,
                        results,
                        successors,
                    });
                }
            }
        }
        companion.regions.push(CompanionRegion {
            id: id.to_string(),
            lowerings,
        });
    }
    Ok(companion)
}

/// A terminator that leaves the function is a region exit the host
/// finishes itself (status 1 or 4), never a host operation.
fn leaves_function(terminator: &Terminator) -> bool {
    matches!(
        terminator,
        Terminator::Return(_) | Terminator::ReturnVoid | Terminator::PropagateError { .. }
    )
}

/// What one lowering of a region exchanges with its host, independently of
/// the lowering's local numbering.
#[derive(Debug, PartialEq, Eq)]
struct RegionShape {
    inputs: Vec<(Type, UnsafeRegionValueKind, bool)>,
    outputs: Vec<(Type, UnsafeRegionValueKind, bool)>,
    result: Option<Type>,
    exits: Vec<i32>,
}

impl RegionShape {
    fn of(function: &Function, region: &UnsafeRegion) -> Self {
        let values = |values: &[UnsafeRegionValue]| {
            values
                .iter()
                .map(|value| {
                    (
                        function.locals[value.local.0].ty,
                        value.kind,
                        value.nullable,
                    )
                })
                .collect()
        };
        Self {
            inputs: values(&region.inputs),
            outputs: values(&region.outputs),
            result: region.result.map(|local| function.locals[local.0].ty),
            exits: region.exits.iter().map(|exit| exit.kind.status()).collect(),
        }
    }
}

/// The locals one statement or terminator touches, in order of first
/// touch, with whether it writes them, and whether it reaches the host.
#[derive(Default)]
struct Touches {
    locals: Vec<(LocalId, bool)>,
    reaches_host: bool,
}

impl Touches {
    fn local(locals: &mut Vec<(LocalId, bool)>, local: LocalId, access: LocalAccess) {
        let writes = access == LocalAccess::Write;
        match locals.iter_mut().find(|(seen, _)| *seen == local) {
            Some((_, written)) => *written |= writes,
            None => locals.push((local, writes)),
        }
    }

    /// A statement is a host operation when it reaches the host or touches
    /// a value that stays on the host. Its arguments are the other values it
    /// touches, with their current values (the companion starts every local
    /// it owns zeroed, so a value written before it is read crosses defined),
    /// and its results the ones it writes. Host tokens never leave the host
    /// frame, so they cross in neither direction.
    fn host_operation(
        self,
        function: &Function,
        collections: &[CollectionType],
    ) -> Option<(Vec<UnsafeRegionValue>, Vec<UnsafeRegionValue>)> {
        let values = self
            .locals
            .iter()
            .map(|(local, writes)| {
                let ty = function.locals[local.0].ty;
                (UnsafeRegionValue::of(*local, ty, collections), *writes)
            })
            .collect::<Vec<_>>();
        let token = values
            .iter()
            .any(|(value, _)| value.kind == UnsafeRegionValueKind::HostToken);
        if !self.reaches_host && !token {
            return None;
        }
        let crossing = values
            .into_iter()
            .filter(|(value, _)| value.kind != UnsafeRegionValueKind::HostToken)
            .collect::<Vec<_>>();
        let results = crossing
            .iter()
            .filter(|(_, writes)| *writes)
            .map(|(value, _)| *value)
            .collect();
        Some((
            crossing.into_iter().map(|(value, _)| value).collect(),
            results,
        ))
    }
}
