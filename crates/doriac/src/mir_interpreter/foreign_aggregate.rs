//! Foreign structs and unions on the host (Decision 0148 section 6). A
//! value is its bytes in the target's C layout ([`c_value`]), so a field
//! access reads or writes the field's bytes at its offset, a union member
//! reads the bytes written last through any member, and the companion
//! library receives the same bytes a native frame holds.

use super::byte_region::{ByteRegionValue, RegionSource};
use super::c_value::{self, ForeignAggregateValue};
use super::*;

/// Storage that holds C-layout bytes in place: the bytes `offset` into the
/// foreign struct or union, or the FFI-safe scalar, a host place holds, or memory a foreign library
/// owns, which the host reads and writes through the shared runtime
/// (Decision 0148 sections 10 and 23).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InlineStorage {
    /// `offset` bytes into the C layout of the value `place` holds; a
    /// nullable place holds a present value.
    Place {
        place: InterpreterPlace,
        offset: u64,
    },
    /// Memory a foreign library owns at `address`: a region over foreign
    /// memory views it as address and length, with no copy.
    Foreign { address: u64 },
}

impl InlineStorage {
    /// The same storage `delta` bytes further on.
    pub fn at(self, delta: u64) -> Self {
        match self {
            Self::Place { place, offset } => Self::Place {
                place,
                offset: offset + delta,
            },
            Self::Foreign { address } => Self::Foreign {
                address: address + delta,
            },
        }
    }
}

/// What a field store or a construction does with one evaluated value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FieldStore {
    offset: u64,
    ty: mir::Type,
    copied_elements: Option<u64>,
}

#[derive(Debug, Clone)]
pub(super) enum Task {
    Value(mir::ForeignAggregateExpression),
    Nullable(mir::NullableForeignAggregateExpression),
    /// Pops one value per field, in order, and pushes the aggregate.
    Build {
        ty: mir::ForeignAggregateType,
        fields: Vec<FieldStore>,
        span: Span,
    },
    /// Pops a field's value and stores it at `storage`.
    Store {
        storage: InlineStorage,
        field: FieldStore,
        span: Span,
    },
    Wrap,
    IsPresent,
    Box,
}

/// The C-layout bytes of the value a place holds: a foreign struct or
/// union, or an FFI-safe scalar, each present when the place is nullable.
fn place_bytes(value: &LocalValue) -> Result<Vec<u8>, InterpreterError> {
    match value {
        LocalValue::ForeignAggregate(value)
        | LocalValue::NullableForeignAggregate {
            value: Some(value), ..
        } => Ok(value.bytes.clone()),
        LocalValue::Scalar(scalar)
        | LocalValue::NullableScalar {
            value: Some(scalar),
            ..
        } => Ok(scalar.data_bytes()),
        _ => Err(InterpreterError::new(
            "a place in C layout holds a present foreign struct, union or scalar",
        )),
    }
}

/// `value` with its present payload replaced by `bytes`.
fn with_place_bytes(value: LocalValue, bytes: Vec<u8>) -> Result<LocalValue, InterpreterError> {
    let scalar =
        |scalar: mir::ScalarValue| match c_value::decode(mir::Type::Scalar(scalar.ty()), &bytes)? {
            LocalValue::Scalar(value) => Ok(value),
            _ => unreachable!("a scalar type decodes as a scalar"),
        };
    Ok(match value {
        LocalValue::ForeignAggregate(value) => {
            LocalValue::ForeignAggregate(ForeignAggregateValue { bytes, ..value })
        }
        LocalValue::NullableForeignAggregate {
            ty,
            value: Some(value),
        } => LocalValue::NullableForeignAggregate {
            ty,
            value: Some(ForeignAggregateValue { bytes, ..value }),
        },
        LocalValue::Scalar(value) => LocalValue::Scalar(scalar(value)?),
        LocalValue::NullableScalar {
            ty,
            value: Some(value),
        } => LocalValue::NullableScalar {
            ty,
            value: Some(scalar(value)?),
        },
        _ => {
            return Err(InterpreterError::new(
                "a place in C layout holds a present foreign struct, union or scalar",
            ))
        }
    })
}

fn payload(value: &LocalValue) -> Result<&ForeignAggregateValue, InterpreterError> {
    match value {
        LocalValue::ForeignAggregate(value)
        | LocalValue::NullableForeignAggregate {
            value: Some(value), ..
        } => Ok(value),
        _ => Err(InterpreterError::new(
            "a foreign field access found no foreign struct or union in place",
        )),
    }
}

impl Interpreter<'_> {
    /// `size` bytes at `storage`.
    pub(super) fn inline_bytes(
        &self,
        storage: InlineStorage,
        size: usize,
    ) -> Result<Vec<u8>, InterpreterError> {
        match storage {
            InlineStorage::Place { place, offset } => {
                let bytes = place_bytes(&self.read_place(place)?)?;
                let start = offset as usize;
                bytes
                    .get(start..start + size)
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| InterpreterError::new("a foreign field lies inside its value"))
            }
            InlineStorage::Foreign { address } => {
                Ok(self.loaded_companion()?.read_foreign(address, size))
            }
        }
    }

    /// Writes `bytes` at `storage`.
    pub(super) fn write_inline_bytes(
        &mut self,
        storage: InlineStorage,
        bytes: &[u8],
    ) -> Result<(), InterpreterError> {
        match storage {
            InlineStorage::Place { place, offset } => {
                let value = self.read_place(place)?;
                let mut stored = place_bytes(&value)?;
                let start = offset as usize;
                stored
                    .get_mut(start..start + bytes.len())
                    .ok_or_else(|| InterpreterError::new("a foreign field lies inside its value"))?
                    .copy_from_slice(bytes);
                self.write_place(place, with_place_bytes(value, stored)?)?;
                Ok(())
            }
            InlineStorage::Foreign { address } => {
                self.loaded_companion()?.write_foreign(address, bytes);
                Ok(())
            }
        }
    }

    /// The companion a program with regions over foreign memory has loaded:
    /// only its regions build them.
    fn loaded_companion(&self) -> Result<&companion_host::LoadedCompanion, InterpreterError> {
        self.companion.as_ref().ok_or_else(|| {
            InterpreterError::new("a region over foreign memory comes from a loaded companion")
        })
    }

    /// The storage an inline place names, or the panic its index raises.
    pub(super) fn inline_storage(
        &mut self,
        place: mir::InlinePlace,
    ) -> Result<Result<InlineStorage, StepOutcome>, InterpreterError> {
        let at_place = |place| InlineStorage::Place { place, offset: 0 };
        Ok(Ok(match place {
            mir::InlinePlace::Local(local)
            | mir::InlinePlace::NullableLocalAssumeNonNull(local) => {
                at_place(self.place_for_local(local)?)
            }
            mir::InlinePlace::Property { object, property }
            | mir::InlinePlace::NullablePropertyAssumeNonNull { object, property } => {
                at_place(self.place_for_property(object, property)?)
            }
            mir::InlinePlace::Static(id) | mir::InlinePlace::NullableStaticAssumeNonNull(id) => {
                let ty = self
                    .program
                    .statics
                    .get(id.0)
                    .ok_or_else(|| InterpreterError::new("a static property exists"))?
                    .ty;
                at_place(InterpreterPlace {
                    storage: InterpreterPlaceStorage::Static(id),
                    ty,
                    payload_projection: None,
                })
            }
            mir::InlinePlace::CollectionElement {
                collection,
                index,
                positional,
                index_span,
            }
            | mir::InlinePlace::NullableCollectionElementAssumeNonNull {
                collection,
                index,
                positional,
                index_span,
            } => {
                let key = self.read_local_value(index)?;
                let position = match self.collection_position(collection, &key, positional) {
                    Ok(position) => position,
                    Err(error) => {
                        return self
                            .collection_access_panic_step_at(error, index_span)
                            .map(Err)
                    }
                };
                at_place(self.place_for_collection_element(collection, position)?)
            }
            mir::InlinePlace::RegionElement {
                region,
                index,
                index_span,
            } => {
                let region = self.region_local(region)?;
                let LocalValue::Scalar(mir::ScalarValue::Integer(index)) =
                    self.read_local_value(index)?
                else {
                    return Err(InterpreterError::new("a region index is an integer"));
                };
                let index = index.signed_value();
                if index < 0 || index as u128 >= region.length as u128 {
                    return self
                        .collection_access_panic_step_at(
                            CollectionAccessError::Bounds {
                                code: "P1310",
                                index: index as i64,
                                length: region.length,
                            },
                            index_span,
                        )
                        .map(Err);
                }
                let (_, stride) = self.region_element(region.kind)?;
                match region
                    .source
                    .expect("present region")
                    .advance(index as usize, stride)
                {
                    RegionSource::Elements(element) => at_place(InterpreterPlace {
                        storage: InterpreterPlaceStorage::CollectionElement(element),
                        ty: element.ty,
                        payload_projection: None,
                    }),
                    RegionSource::Inline(storage) => storage,
                }
            }
        }))
    }

    /// Runs one `Statement::ForeignField`.
    pub(super) fn execute_foreign_field(
        &mut self,
        aggregate: mir::InlinePlace,
        offset: u64,
        access: mir::ForeignFieldAccess,
        span: Span,
    ) -> Result<StepOutcome, InterpreterError> {
        let storage = match self.inline_storage(aggregate)? {
            Ok(storage) => storage.at(offset),
            Err(panic) => return Ok(panic),
        };
        let function = function_in(self.program, self.current_frame()?.function)?;
        match access {
            mir::ForeignFieldAccess::Read { target } => {
                let ty = local_in(function, target)?.ty;
                let bytes = self.inline_bytes(storage, c_value::size_of(ty))?;
                let value = c_value::decode(ty, &bytes)?;
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::Assign(target));
                self.push_local_value(value)?;
            }
            mir::ForeignFieldAccess::Write {
                value,
                copied_elements,
            } => {
                let field = FieldStore {
                    offset: 0,
                    ty: value.ty(),
                    copied_elements,
                };
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::ForeignAggregate(Task::Store {
                        storage,
                        field,
                        span,
                    }));
                frame.tasks.push(EvaluationTask::Rvalue(value));
            }
            mir::ForeignFieldAccess::Region { target, length } => {
                let (kind, nullable) = match local_in(function, target)?.ty {
                    mir::Type::Region(kind) => (kind, false),
                    mir::Type::NullableRegion(kind) => (kind, true),
                    _ => {
                        return Err(InterpreterError::new(
                            "a fixed-length array field is viewed as a region",
                        ))
                    }
                };
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::Assign(target));
                self.push_local_value(LocalValue::ByteRegion(ByteRegionValue {
                    kind,
                    nullable,
                    source: Some(RegionSource::Inline(storage)),
                    length: length as usize,
                }))?;
            }
        }
        Ok(StepOutcome::Continue)
    }

    /// The C bytes of a field's evaluated `value`: the value itself, or
    /// the elements of the region a fixed-length array field copies, or
    /// the P1704 panic of a region of another length.
    fn field_bytes(
        &mut self,
        field: FieldStore,
        value: LocalValue,
        span: Span,
    ) -> Result<Result<Vec<u8>, StepOutcome>, InterpreterError> {
        let Some(count) = field.copied_elements else {
            let mut bytes = vec![0; c_value::size_of(field.ty)];
            c_value::encode(Some(&value), field.ty, &mut bytes);
            return Ok(Ok(bytes));
        };
        let LocalValue::ByteRegion(region) = value else {
            return Err(InterpreterError::new(
                "a copied fixed-length array field is supplied from a region",
            ));
        };
        if region.length as u64 != count {
            return self
                .runtime_panic_step_at(crate::foreign::ARRAY_FIELD_LENGTH_PANIC_CODE, span)
                .map(Err);
        }
        let (element, stride) = self.region_element(region.kind)?;
        let mut bytes = vec![0; count as usize * stride];
        let values = self.region_elements(region, 0, count as usize)?;
        for (slot, value) in bytes.chunks_exact_mut(stride.max(1)).zip(&values) {
            c_value::encode(Some(value), element, slot);
        }
        Ok(Ok(bytes))
    }

    pub(super) fn execute_foreign_aggregate_task(
        &mut self,
        task: Task,
    ) -> Result<StepOutcome, InterpreterError> {
        match task {
            Task::Value(expression) => self.expand_value(expression)?,
            Task::Nullable(expression) => self.expand_nullable(expression)?,
            Task::Build { ty, fields, span } => {
                let values = self.take_call_arguments(fields.len())?;
                let mut aggregate = ForeignAggregateValue::zeroed(ty);
                for (field, value) in fields.into_iter().zip(values) {
                    let bytes = match self.field_bytes(field, value, span)? {
                        Ok(bytes) => bytes,
                        Err(panic) => return Ok(panic),
                    };
                    let start = field.offset as usize;
                    aggregate
                        .bytes
                        .get_mut(start..start + bytes.len())
                        .ok_or_else(|| {
                            InterpreterError::new("a foreign field lies inside its value")
                        })?
                        .copy_from_slice(&bytes);
                }
                self.push_local_value(LocalValue::ForeignAggregate(aggregate))?;
            }
            Task::Store {
                storage,
                field,
                span,
            } => {
                let value = self.pop_local_value()?;
                let bytes = match self.field_bytes(field, value, span)? {
                    Ok(bytes) => bytes,
                    Err(panic) => return Ok(panic),
                };
                self.write_inline_bytes(storage, &bytes)?;
            }
            Task::Wrap => {
                let LocalValue::ForeignAggregate(value) = self.pop_local_value()? else {
                    return Err(InterpreterError::new(
                        "MIR foreign aggregate evaluation produced another type",
                    ));
                };
                self.push_local_value(LocalValue::NullableForeignAggregate {
                    ty: value.ty,
                    value: Some(value),
                })?;
            }
            Task::IsPresent => {
                let LocalValue::NullableForeignAggregate { value, .. } = self.pop_local_value()?
                else {
                    return Err(InterpreterError::new(
                        "MIR nullable foreign aggregate evaluation produced another type",
                    ));
                };
                self.push_scalar(mir::ScalarValue::Bool(value.is_some()))?;
            }
            Task::Box => {
                let LocalValue::ForeignAggregate(value) = self.pop_local_value()? else {
                    return Err(InterpreterError::new(
                        "MIR foreign aggregate evaluation produced another type",
                    ));
                };
                self.push_mixed(MixedValue::ForeignAggregate(value))?;
            }
        }
        Ok(StepOutcome::Continue)
    }

    fn expand_value(
        &mut self,
        expression: mir::ForeignAggregateExpression,
    ) -> Result<(), InterpreterError> {
        let expected = mir::Type::ForeignAggregate(expression.ty());
        let value = match expression {
            mir::ForeignAggregateExpression::Construct { ty, fields, span } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::ForeignAggregate(Task::Build {
                        ty,
                        fields: fields
                            .iter()
                            .map(|field| FieldStore {
                                offset: field.offset,
                                ty: field.value.ty(),
                                copied_elements: field.copied_elements,
                            })
                            .collect(),
                        span,
                    }));
                for field in fields.into_iter().rev() {
                    frame.tasks.push(EvaluationTask::Rvalue(field.value));
                }
                return Ok(());
            }
            mir::ForeignAggregateExpression::Local { local, .. } => self.read_local_value(local)?,
            mir::ForeignAggregateExpression::Property {
                object, property, ..
            } => self.read_property(object, property)?,
            mir::ForeignAggregateExpression::Static { id, .. } => self.static_value(id)?,
            mir::ForeignAggregateExpression::Call {
                function,
                args,
                span,
                ..
            } => return self.queue_call(function, args, ReturnExpectation::Value(expected), span),
            mir::ForeignAggregateExpression::NullableLocalAssumeNonNull { local, .. } => {
                LocalValue::ForeignAggregate(payload(&self.read_local_value(local)?)?.clone())
            }
            mir::ForeignAggregateExpression::CollectionIndex {
                collection,
                index,
                index_span,
                positional,
                remove,
                ..
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::LoadCollectionValue {
                    collection,
                    index_span,
                    transfer: remove,
                    positional,
                });
                frame.tasks.push(EvaluationTask::SetPanicSite(index_span));
                frame.tasks.push(EvaluationTask::Rvalue(*index));
                return Ok(());
            }
            mir::ForeignAggregateExpression::RegionIndex {
                region,
                index,
                index_span,
                ..
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::ByteRegion(byte_region::Task::Index {
                        region,
                        span: index_span,
                    }));
                frame.tasks.push(EvaluationTask::Rvalue(*index));
                return Ok(());
            }
            mir::ForeignAggregateExpression::MixedPayload { mixed, .. } => {
                match self.read_local_value(mixed)? {
                    LocalValue::Mixed(MixedValue::ForeignAggregate(value))
                    | LocalValue::NullableMixed(Some(MixedValue::ForeignAggregate(value))) => {
                        LocalValue::ForeignAggregate(value)
                    }
                    _ => {
                        return Err(InterpreterError::new(
                            "MIR mixed payload is not a foreign struct or union",
                        ))
                    }
                }
            }
        };
        if !local_value_matches_type(self.program, expected, &value) {
            return Err(InterpreterError::new(
                "MIR foreign aggregate source has another type",
            ));
        }
        self.push_local_value(value)
    }

    fn expand_nullable(
        &mut self,
        expression: mir::NullableForeignAggregateExpression,
    ) -> Result<(), InterpreterError> {
        let expected = mir::Type::NullableForeignAggregate(expression.ty());
        let value = match expression {
            mir::NullableForeignAggregateExpression::Null(ty) => {
                LocalValue::NullableForeignAggregate { ty, value: None }
            }
            mir::NullableForeignAggregateExpression::Present(value) => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::ForeignAggregate(Task::Wrap));
                frame
                    .tasks
                    .push(EvaluationTask::ForeignAggregate(Task::Value(value)));
                return Ok(());
            }
            mir::NullableForeignAggregateExpression::Local { local, .. } => {
                self.read_local_value(local)?
            }
            mir::NullableForeignAggregateExpression::Property {
                object, property, ..
            } => self.read_property(object, property)?,
            mir::NullableForeignAggregateExpression::Static { id, .. } => self.static_value(id)?,
            mir::NullableForeignAggregateExpression::Call {
                function,
                args,
                span,
                ..
            } => return self.queue_call(function, args, ReturnExpectation::Value(expected), span),
            mir::NullableForeignAggregateExpression::CollectionIndex {
                collection,
                index,
                index_span,
                positional,
                remove,
                ..
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::LoadCollectionValue {
                    collection,
                    index_span,
                    transfer: remove,
                    positional,
                });
                frame.tasks.push(EvaluationTask::SetPanicSite(index_span));
                frame.tasks.push(EvaluationTask::Rvalue(*index));
                return Ok(());
            }
            mir::NullableForeignAggregateExpression::CollectionGet {
                ty,
                collection,
                key,
                access,
                index_span,
                ..
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::DictionaryGet {
                    collection,
                    expected: mir::Type::ForeignAggregate(ty),
                    access,
                });
                frame.tasks.push(EvaluationTask::SetPanicSite(index_span));
                frame.tasks.push(EvaluationTask::Rvalue(*key));
                return Ok(());
            }
        };
        if !local_value_matches_type(self.program, expected, &value) {
            return Err(InterpreterError::new(
                "MIR nullable foreign aggregate source has another type",
            ));
        }
        self.push_local_value(value)
    }

    fn static_value(&self, id: mir::StaticId) -> Result<LocalValue, InterpreterError> {
        self.statics
            .get(id.0)
            .cloned()
            .ok_or_else(|| InterpreterError::new("a static property exists"))
    }
}
