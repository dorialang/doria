//! The native companion library's plan (Decision 0148 sections 22 and 23,
//! entry ABI version 1): one C-ABI entry per region identity, shared by
//! every lowering of the region, and the table of host operations its
//! regions call back through, with the C layout of every slot record and
//! the entry function generated from the region's blocks. Lowering builds
//! the plan once every region has its inputs and outputs, and MIR
//! validation derives it again and requires the same result, so the plan
//! has one owner.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Not;

use super::local_access::{statement_touches, terminator_touches, HostReach, LocalAccess};
use super::*;
use crate::ast::ForeignAggregateKind;
use crate::c_layout::Placement;
use crate::inline_value::InlineValueType;

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
/// order and then block and statement order; 0 is reserved for accepting an
/// output. Entries take the function ids after every function of `program`.
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
    let error = |region: &UnsafeRegion, details: String| CompanionPlanError {
        span: region.span,
        details,
    };
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
        let first_operation = companion.host_operations.len();
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
                    let id = companion.host_operations.len() as u32 + 1;
                    let record = |values: Vec<UnsafeRegionValue>, index: bool| {
                        SlotRecord::of(canonical, values, index).ok_or_else(|| {
                            error(
                                region,
                                format!("host operation {id} has too large a record"),
                            )
                        })
                    };
                    companion.host_operations.push(HostOperation {
                        id,
                        region: index,
                        block: block.id,
                        site,
                        arguments: record(arguments, false)?,
                        results: record(results, site == HostOperationSite::Terminator)?,
                        successors,
                    });
                }
            }
        }
        let too_large = || error(region, format!("region {id} has too large a slot record"));
        let inputs =
            SlotRecord::of(canonical, region.inputs.clone(), false).ok_or_else(too_large)?;
        let outputs = SlotRecord::of(
            canonical,
            region
                .outputs
                .iter()
                .copied()
                .filter(|output| Some(output.local) != region.result)
                .chain(
                    region
                        .outputs
                        .iter()
                        .copied()
                        .filter(|output| Some(output.local) == region.result),
                )
                .collect(),
            true,
        )
        .ok_or_else(too_large)?;
        let entry = EntryBuilder::new(
            canonical,
            region,
            &inputs,
            &outputs,
            &program.collection_types,
        )
        .build(
            FunctionId(program.functions.len() + index),
            &companion.host_operations[first_operation..],
        );
        companion.regions.push(CompanionRegion {
            id: id.to_string(),
            lowerings,
            inputs,
            outputs,
            entry,
        });
    }
    Ok(companion)
}

/// A terminator that leaves the function is a region exit the host
/// finishes itself (status 1 or 4), never a host operation: the host runs
/// the terminator in its own frame, so the locals it reads are outputs.
pub(crate) fn leaves_function(terminator: &Terminator) -> bool {
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
/// touch, with whether it writes or moves them, and whether it reaches the
/// host.
#[derive(Default)]
struct Touches {
    locals: Vec<(LocalId, bool)>,
    reaches_host: bool,
}

impl Touches {
    fn local(locals: &mut Vec<(LocalId, bool)>, local: LocalId, access: LocalAccess) {
        let writes = access != LocalAccess::Read;
        match locals.iter_mut().find(|(seen, _)| *seen == local) {
            Some((_, written)) => *written |= writes,
            None => locals.push((local, writes)),
        }
    }

    /// A statement is a host operation when it reaches the host or touches
    /// a value that stays on the host. Its arguments are the other values it
    /// touches, with their current values (the companion starts every local
    /// it owns zeroed, so a value written before it is read crosses defined),
    /// and its results the ones it writes or moves (a moved one crosses
    /// back with no value). Host tokens never leave the host frame, so they
    /// cross in neither direction.
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

impl SlotRecord {
    /// The record of `values`, locals of `function`, in order: host tokens
    /// have no slot, and an `int32` index ends it when `index` is set.
    /// `None` when its size does not fit in 64 bits.
    fn of(function: &Function, values: Vec<UnsafeRegionValue>, index: bool) -> Option<Self> {
        let values = values
            .into_iter()
            .filter(|value| value.kind != UnsafeRegionValueKind::HostToken)
            .collect::<Vec<_>>();
        let members = values.iter().map(|value| {
            crate::native_abi::companion_slot_layout(*value, function.locals[value.local.0].ty)
                .expect("a value other than a host token has a slot")
        });
        let placement = Placement::of(
            ForeignAggregateKind::Struct,
            members.chain(index.then_some(crate::native_abi::COMPANION_INDEX_LAYOUT)),
        )?;
        Some(Self {
            slots: values
                .iter()
                .zip(&placement.offsets)
                .map(|(value, offset)| CompanionSlot {
                    value: *value,
                    offset: *offset,
                })
                .collect(),
            index: index.then(|| placement.offsets[values.len()]),
            size: placement.size,
            align: placement.align,
        })
    }
}

impl UnsafeRegionValueKind {
    /// Content the companion copies into a value it owns: a string, a
    /// buffer or a foreign string. It leaves through operation 0 and is
    /// released at every exit.
    pub const fn is_content(self) -> bool {
        matches!(self, Self::Text | Self::Buffer | Self::ForeignString)
    }
}

const POINTER: Type = Type::Scalar(ScalarType::Inline(InlineValueType::Pointer));
const INDEX: IntegerType = IntegerType::Int32;

/// Generates a region's companion entry from its first lowering: the
/// lowering's locals, every region block at its own id (other blocks
/// become unreachable), a prologue that loads `inputs`, an epilogue per
/// edge out of the region, and a host call at each host-operation site.
struct EntryBuilder<'f> {
    canonical: &'f Function,
    region: &'f UnsafeRegion,
    inputs: &'f SlotRecord,
    outputs: &'f SlotRecord,
    locals: Vec<Local>,
    blocks: Vec<BasicBlock>,
    host: LocalId,
    caller_frame: LocalId,
    input_record: LocalId,
    output_record: LocalId,
    /// The exit or target index an epilogue writes and a host operation at
    /// a terminator returns.
    index: LocalId,
    /// The output slot operation 0 accepts.
    slot: LocalId,
    arguments: LocalId,
    results: LocalId,
}

impl<'f> EntryBuilder<'f> {
    fn new(
        canonical: &'f Function,
        region: &'f UnsafeRegion,
        inputs: &'f SlotRecord,
        outputs: &'f SlotRecord,
        collections: &[CollectionType],
    ) -> Self {
        // The entry's frame owns the content it copies in, and holds no
        // host token: such a local is a placeholder the entry never
        // touches, since every statement that touches one is a host call.
        let mut locals = canonical.locals.clone();
        for slot in &inputs.slots {
            if slot.value.kind.is_content() {
                let local = &mut locals[slot.value.local.0];
                local.owned = true;
                local.synthetic = true;
            }
        }
        for local in &mut locals {
            if UnsafeRegionValueKind::of(local.ty, collections) == UnsafeRegionValueKind::HostToken
            {
                local.owned = false;
                local.synthetic = true;
            }
        }
        let mut local = |name: &str, ty: Type| {
            let id = LocalId(locals.len());
            locals.push(Local {
                id,
                name: name.to_string(),
                ty,
                writable: true,
                owned: false,
                synthetic: true,
            });
            id
        };
        let host = local("$host", POINTER);
        let caller_frame = local("$callerFrame", POINTER);
        let input_record = local("$inputs", POINTER);
        let output_record = local("$outputs", POINTER);
        let index = local("$index", Type::Scalar(ScalarType::Integer(INDEX)));
        let slot = local("$slot", POINTER);
        let arguments = local("$arguments", POINTER);
        let results = local("$results", POINTER);
        Self {
            canonical,
            region,
            inputs,
            outputs,
            locals,
            blocks: Vec::new(),
            host,
            caller_frame,
            input_record,
            output_record,
            index,
            slot,
            arguments,
            results,
        }
    }

    fn build(mut self, id: FunctionId, operations: &[HostOperation]) -> Function {
        let region = self.region;
        self.blocks = self
            .canonical
            .blocks
            .iter()
            .map(|block| {
                if region.contains(block.id) {
                    block.clone()
                } else {
                    BasicBlock {
                        id: block.id,
                        statements: Vec::new(),
                        terminator: Terminator::Unreachable,
                    }
                }
            })
            .collect();
        let exit_blocks = region
            .exits
            .iter()
            .enumerate()
            .map(|(position, exit)| {
                let statements = self.epilogue(Some(position));
                self.push_block(statements, status(exit.kind.status()))
            })
            .collect::<Vec<_>>();
        let statements = self.epilogue(None);
        self.blocks[region.continuation.0] = BasicBlock {
            id: region.continuation,
            statements,
            terminator: status(0),
        };
        let retarget = |source: BlockId, target: BlockId| {
            region
                .exits
                .iter()
                .zip(&exit_blocks)
                .find(|(exit, _)| exit.source == source && exit.target == Some(target))
                .map_or(target, |(_, epilogue)| *epilogue)
        };
        for &block in &region.blocks {
            let original = std::mem::replace(
                &mut self.blocks[block.0],
                BasicBlock {
                    id: block,
                    statements: Vec::new(),
                    terminator: Terminator::Unreachable,
                },
            );
            let site = |site: HostOperationSite| {
                operations
                    .iter()
                    .find(|operation| operation.block == block && operation.site == site)
            };
            let mut statements = Vec::with_capacity(original.statements.len());
            for (position, statement) in original.statements.into_iter().enumerate() {
                match site(HostOperationSite::Statement(position)) {
                    Some(operation) => statements.extend(self.host_call(operation)),
                    None => statements.push(statement),
                }
            }
            let leaves = region
                .exits
                .iter()
                .position(|exit| exit.source == block && exit.target.is_none());
            let terminator = if let Some(operation) = site(HostOperationSite::Terminator) {
                statements.extend(self.host_call(operation));
                let successors = operation
                    .successors
                    .iter()
                    .map(|target| retarget(block, *target))
                    .collect::<Vec<_>>();
                self.branch_on_index(&successors)
            } else if let Some(position) = leaves {
                Terminator::Jump(exit_blocks[position])
            } else {
                let mut terminator = original.terminator;
                for target in terminator.targets_mut() {
                    *target = retarget(block, *target);
                }
                terminator
            };
            self.blocks[block.0] = BasicBlock {
                id: block,
                statements,
                terminator,
            };
        }
        let prologue = self
            .inputs
            .slots
            .iter()
            .map(|slot| Statement::RegionSlot {
                record: self.input_record,
                slot: *slot,
                access: SlotAccess::Load,
            })
            .collect();
        let entry_block = self.push_block(prologue, Terminator::Jump(region.entry));
        let canonical = self.canonical;
        Function {
            id,
            name: canonical.name.clone(),
            source_span: canonical.source_span,
            diagnostic_frame: canonical.diagnostic_frame,
            method: None,
            virtual_slot: None,
            receiver_mode: None,
            closure: None,
            params: vec![
                self.host,
                self.caller_frame,
                self.input_record,
                self.output_record,
            ],
            parameter_modes: vec![FunctionParameterMode::Readonly; 4],
            return_type: ReturnType::Value(Type::Scalar(ScalarType::Integer(INDEX))),
            return_borrow: None,
            required_checked_effects: Vec::new(),
            ambient_checked_effects: Vec::new(),
            test_assertion_checked_effects: Vec::new(),
            checked_effects: Vec::new(),
            locals: self.locals,
            blocks: self.blocks,
            entry_block,
            unsafe_regions: Vec::new(),
        }
    }

    fn push_block(&mut self, statements: Vec<Statement>, terminator: Terminator) -> BlockId {
        let id = BlockId(self.blocks.len());
        self.blocks.push(BasicBlock {
            id,
            statements,
            terminator,
        });
        id
    }

    /// The statements of one edge out of the region, before it returns its
    /// status: every output slot, each content output handed to the host
    /// through operation 0, the exit's index (none for fallthrough), then
    /// the release of every value the companion copied in, in reverse
    /// order.
    fn epilogue(&self, exit: Option<usize>) -> Vec<Statement> {
        let mut statements = Vec::new();
        for slot in &self.outputs.slots {
            statements.push(Statement::RegionSlot {
                record: self.output_record,
                slot: *slot,
                access: SlotAccess::Store,
            });
            if slot.value.kind.is_content() {
                statements.push(Statement::Pointer {
                    target: Some(self.slot),
                    operation: PointerOperation::Offset {
                        pointer: self.output_record,
                        count: IntegerExpression::constant(IntegerValue::from_bits(
                            IntegerType::Int64,
                            slot.offset,
                        )),
                        scale: 1,
                    },
                    span: self.region.span,
                });
                statements.push(Statement::CallHost {
                    host: self.host,
                    operation: ACCEPT_OUTPUT_OPERATION,
                    arguments: Some(self.slot),
                    results: Some(self.slot),
                });
            }
        }
        if let (Some(position), Some(offset)) = (exit, self.outputs.index) {
            statements.push(Statement::AssignLocal {
                target: self.index,
                value: index_value(position),
            });
            statements.push(Statement::RegionSlot {
                record: self.output_record,
                slot: self.index_slot(offset),
                access: SlotAccess::Store,
            });
        }
        for slot in self.inputs.slots.iter().rev() {
            let local = slot.value.local;
            match (slot.value.kind, self.locals[local.0].ty) {
                (UnsafeRegionValueKind::Text, _) => {
                    statements.push(Statement::DropString { local });
                }
                (
                    UnsafeRegionValueKind::Buffer | UnsafeRegionValueKind::ForeignString,
                    Type::Collection(collection),
                ) => statements.push(Statement::DropCollection { local, collection }),
                _ => {}
            }
        }
        statements
    }

    /// One host operation: its argument record written from the current
    /// values, the call, then every result loaded back, and for a
    /// terminator the index of the target the host took.
    fn host_call(&self, operation: &HostOperation) -> Vec<Statement> {
        let mut statements = Vec::new();
        let arguments = (!operation.arguments.slots.is_empty()).then_some(self.arguments);
        let results = (!operation.results.slots.is_empty() || operation.results.index.is_some())
            .then_some(self.results);
        for (local, record) in [
            (arguments, &operation.arguments),
            (results, &operation.results),
        ] {
            if let Some(target) = local {
                statements.push(Statement::HostRecord {
                    target,
                    size: record.size,
                    align: record.align,
                });
            }
        }
        statements.extend(
            operation
                .arguments
                .slots
                .iter()
                .map(|slot| Statement::RegionSlot {
                    record: self.arguments,
                    slot: *slot,
                    access: SlotAccess::Store,
                }),
        );
        statements.push(Statement::CallHost {
            host: self.host,
            operation: operation.id,
            arguments,
            results,
        });
        statements.extend(
            operation
                .results
                .slots
                .iter()
                .map(|slot| Statement::RegionSlot {
                    record: self.results,
                    slot: *slot,
                    access: SlotAccess::Load,
                }),
        );
        if let Some(offset) = operation.results.index {
            statements.push(Statement::RegionSlot {
                record: self.results,
                slot: self.index_slot(offset),
                access: SlotAccess::Load,
            });
        }
        statements
    }

    fn index_slot(&self, offset: u64) -> CompanionSlot {
        CompanionSlot {
            value: UnsafeRegionValue {
                local: self.index,
                kind: UnsafeRegionValueKind::Native,
                nullable: false,
            },
            offset,
        }
    }

    /// Continues at `successors[index]`: one comparison per target but
    /// the last, which takes every other index.
    fn branch_on_index(&mut self, successors: &[BlockId]) -> Terminator {
        let (last, earlier) = successors
            .split_last()
            .expect("a host operation at a terminator has a target");
        let mut otherwise = Terminator::Jump(*last);
        for (position, target) in earlier.iter().enumerate().rev() {
            let terminator = Terminator::Branch {
                condition: BoolExpression::Compare {
                    op: CompareOp::Equal,
                    left: Box::new(ValueExpression::Integer(IntegerExpression::use_operand(
                        INDEX,
                        Operand::Local(self.index),
                    ))),
                    right: Box::new(ValueExpression::Integer(IntegerExpression::constant(
                        IntegerValue::from_bits(INDEX, position as u64),
                    ))),
                },
                then_block: *target,
                else_block: match otherwise {
                    Terminator::Jump(block) => block,
                    otherwise => self.push_block(Vec::new(), otherwise),
                },
            };
            otherwise = terminator;
        }
        otherwise
    }
}

fn index_value(position: usize) -> Rvalue {
    Rvalue::Value(ValueExpression::Integer(IntegerExpression::constant(
        IntegerValue::from_bits(INDEX, position as u64),
    )))
}

fn status(status: i32) -> Terminator {
    Terminator::Return(Rvalue::Value(ValueExpression::Integer(
        IntegerExpression::constant(IntegerValue::from_bits(INDEX, status as u32 as u64)),
    )))
}

/// The program the companion library compiles: `program` with each
/// region's entry after its functions, at the index the entry's id names.
pub fn library(program: &Program) -> Program {
    let mut library = program.clone();
    library.functions.extend(
        program
            .companion
            .regions
            .iter()
            .map(|region| region.entry.clone()),
    );
    library
}
