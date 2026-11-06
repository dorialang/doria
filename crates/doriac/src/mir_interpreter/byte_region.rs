//! Non-owning region descriptors over the interpreter's existing Bytes storage.
use super::*;
use crate::types::ByteRegionKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ByteRegionValue {
    pub kind: ByteRegionKind,
    pub nullable: bool,
    // Presence is independent of length; an empty region still has provenance.
    pub source: Option<CollectionElementPlace>,
    pub length: usize,
}

impl ByteRegionValue {
    pub fn ty(self) -> mir::Type {
        if self.nullable {
            mir::Type::NullableByteRegion(self.kind)
        } else {
            mir::Type::ByteRegion(self.kind)
        }
    }

    pub fn absent(kind: ByteRegionKind) -> Self {
        Self {
            kind,
            nullable: true,
            source: None,
            length: 0,
        }
    }

    fn present(mut self) -> Result<Self, InterpreterError> {
        if self.source.is_none() {
            return Err(InterpreterError::new(
                "MIR assumed an absent byte region was present",
            ));
        }
        self.nullable = false;
        Ok(self)
    }
}

#[derive(Debug, Clone)]
pub(super) enum Task {
    Value(mir::ByteRegionExpression),
    Nullable(mir::NullableByteRegionExpression),
    Build {
        kind: ByteRegionKind,
        span: Span,
    },
    Wrap,
    AssumePresent,
    Coalesce(mir::ByteRegionExpression),
    IsPresent,
    Length(mir::LocalId),
    Index {
        region: mir::LocalId,
        span: Span,
    },
    Set {
        region: mir::LocalId,
        span: Span,
    },
    /// Pops the source then the destination region; pushes the copied count.
    Copy,
    /// Pops destination, length and offset; checks both ranges, source first.
    CopyWithin {
        region: mir::LocalId,
        span: Span,
    },
}

fn region_bounds_facts(offset: i64, length: i64, source_length: usize) -> Vec<RuntimeFact> {
    vec![
        RuntimeFact {
            name: doria_diagnostic_catalogue::BYTE_REGION_OFFSET_FACT.into(),
            value: RuntimeFactValue::Signed(offset),
        },
        RuntimeFact {
            name: doria_diagnostic_catalogue::BYTE_REGION_REQUESTED_LENGTH_FACT.into(),
            value: RuntimeFactValue::Signed(length),
        },
        RuntimeFact {
            name: doria_diagnostic_catalogue::BYTE_BUFFER_LENGTH_FACT.into(),
            value: RuntimeFactValue::Unsigned(source_length as u64),
        },
    ]
}

fn checked_range(offset: i64, length: i64, source_length: usize) -> Option<(usize, usize)> {
    usize::try_from(offset)
        .ok()
        .zip(usize::try_from(length).ok())
        .filter(|(offset, length)| *offset <= source_length && *length <= source_length - *offset)
}

impl Interpreter<'_> {
    fn push_region(&mut self, region: ByteRegionValue) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::ByteRegion(region));
        Ok(())
    }

    fn pop_region(&mut self) -> Result<ByteRegionValue, InterpreterError> {
        match self.pop_local_value()? {
            LocalValue::ByteRegion(value) => Ok(value),
            _ => Err(InterpreterError::new(
                "MIR byte region evaluation produced another type",
            )),
        }
    }

    pub(super) fn region_local(
        &self,
        local: mir::LocalId,
    ) -> Result<ByteRegionValue, InterpreterError> {
        match self.read_local_value(local)? {
            LocalValue::ByteRegion(value) => value.present(),
            _ => Err(InterpreterError::new(
                "MIR byte region access uses another type",
            )),
        }
    }

    pub(super) fn execute_region_task(
        &mut self,
        task: Task,
    ) -> Result<StepOutcome, InterpreterError> {
        match task {
            Task::Value(expression) => match expression {
                mir::ByteRegionExpression::CollectionIndex {
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
                }
                mir::ByteRegionExpression::Local {
                    kind,
                    local,
                    transfer,
                } => {
                    let LocalValue::ByteRegion(value) = self.read_or_take_local(local, transfer)?
                    else {
                        return Err(InterpreterError::new("MIR region local has another type"));
                    };
                    if value.kind != kind || value.nullable {
                        return Err(InterpreterError::new(
                            "MIR region local changes its carrier type",
                        ));
                    }
                    self.push_region(value.present()?)?;
                }
                mir::ByteRegionExpression::Region {
                    kind,
                    source,
                    offset,
                    length,
                    span,
                } => {
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::ByteRegion(Task::Build { kind, span }));
                    frame.tasks.push(EvaluationTask::Rvalue(*length));
                    frame.tasks.push(EvaluationTask::Rvalue(*offset));
                    frame.tasks.push(EvaluationTask::Rvalue(*source));
                }
                mir::ByteRegionExpression::Call {
                    kind,
                    function,
                    args,
                    span,
                    ..
                } => {
                    self.queue_call(
                        function,
                        args,
                        ReturnExpectation::Value(mir::Type::ByteRegion(kind)),
                        span,
                    )?;
                }
                mir::ByteRegionExpression::AssumePresent { value, .. } => {
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::ByteRegion(Task::AssumePresent));
                    frame
                        .tasks
                        .push(EvaluationTask::ByteRegion(Task::Nullable(*value)));
                }
                mir::ByteRegionExpression::Coalesce { left, right, .. } => {
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::ByteRegion(Task::Coalesce(*right)));
                    frame
                        .tasks
                        .push(EvaluationTask::ByteRegion(Task::Nullable(*left)));
                }
            },
            Task::Nullable(expression) => match expression {
                mir::NullableByteRegionExpression::CollectionIndex {
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
                }
                mir::NullableByteRegionExpression::CollectionGet {
                    kind,
                    collection,
                    key,
                    index_span,
                    access,
                } => {
                    let frame = self.current_frame_mut()?;
                    frame.tasks.push(EvaluationTask::DictionaryGet {
                        collection,
                        expected: mir::Type::ByteRegion(kind),
                        access,
                    });
                    frame.tasks.push(EvaluationTask::SetPanicSite(index_span));
                    frame.tasks.push(EvaluationTask::Rvalue(*key));
                }
                mir::NullableByteRegionExpression::Null { kind } => {
                    self.push_region(ByteRegionValue::absent(kind))?
                }
                mir::NullableByteRegionExpression::Present(value) => {
                    let frame = self.current_frame_mut()?;
                    frame.tasks.push(EvaluationTask::ByteRegion(Task::Wrap));
                    frame
                        .tasks
                        .push(EvaluationTask::ByteRegion(Task::Value(value)));
                }
                mir::NullableByteRegionExpression::Local {
                    kind,
                    local,
                    transfer,
                } => {
                    let LocalValue::ByteRegion(value) = self.read_or_take_local(local, transfer)?
                    else {
                        return Err(InterpreterError::new(
                            "MIR nullable region local has another type",
                        ));
                    };
                    if value.kind != kind || !value.nullable {
                        return Err(InterpreterError::new(
                            "MIR nullable region changes its carrier type",
                        ));
                    }
                    self.push_region(value)?;
                }
                mir::NullableByteRegionExpression::Call {
                    kind,
                    function,
                    args,
                    span,
                    ..
                } => {
                    self.queue_call(
                        function,
                        args,
                        ReturnExpectation::Value(mir::Type::NullableByteRegion(kind)),
                        span,
                    )?;
                }
            },
            Task::Build { kind, span } => {
                let length = self.pop_integer()?.signed_value();
                let offset = self.pop_integer()?.signed_value();
                let source = self.pop_local_value()?;
                let (base, source_length) = match &source {
                    LocalValue::Collection(bytes)
                        if self.program.collection_types[bytes.ty.0].kind
                            == mir::CollectionKind::Bytes =>
                    {
                        let (identity, generation) =
                            self.register_borrowed_collection(Rc::downgrade(&bytes.entries));
                        (
                            CollectionElementPlace {
                                collection: identity,
                                generation,
                                index: 0,
                                ty: mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8)),
                            },
                            bytes.entries().len(),
                        )
                    }
                    LocalValue::ByteRegion(region) => {
                        let region = region.present()?;
                        (region.source.expect("present region"), region.length)
                    }
                    LocalValue::String(text) => {
                        let entries = self.string_region_storage(text);
                        let (identity, generation) = self.register_borrowed_collection(entries);
                        (
                            CollectionElementPlace {
                                collection: identity,
                                generation,
                                index: 0,
                                ty: mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8)),
                            },
                            text.len(),
                        )
                    }
                    _ => {
                        return Err(InterpreterError::new(
                            "MIR byte region has no Bytes or string provenance",
                        ))
                    }
                };
                self.current_frame_mut()?.active_panic_site = span;
                let checked = usize::try_from(offset)
                    .ok()
                    .zip(usize::try_from(length).ok())
                    .filter(|(offset, length)| {
                        *offset <= source_length && *length <= source_length - *offset
                    });
                let Some((offset, length)) = checked else {
                    return self.runtime_panic_step_with_facts_at(
                        "P1304",
                        span,
                        vec![
                            RuntimeFact {
                                name: doria_diagnostic_catalogue::BYTE_REGION_OFFSET_FACT.into(),
                                value: RuntimeFactValue::Signed(offset as i64),
                            },
                            RuntimeFact {
                                name: doria_diagnostic_catalogue::BYTE_REGION_REQUESTED_LENGTH_FACT
                                    .into(),
                                value: RuntimeFactValue::Signed(length as i64),
                            },
                            RuntimeFact {
                                name: doria_diagnostic_catalogue::BYTE_BUFFER_LENGTH_FACT.into(),
                                value: RuntimeFactValue::Unsigned(source_length as u64),
                            },
                        ],
                    );
                };
                self.push_region(ByteRegionValue {
                    kind,
                    nullable: false,
                    source: Some(CollectionElementPlace {
                        index: base.index + offset,
                        ..base
                    }),
                    length,
                })?;
            }
            Task::Wrap => {
                let mut value = self.pop_region()?.present()?;
                value.nullable = true;
                self.push_region(value)?;
            }
            Task::AssumePresent => {
                let value = self.pop_region()?.present()?;
                self.push_region(value)?;
            }
            Task::Coalesce(right) => {
                let value = self.pop_region()?;
                if value.source.is_some() {
                    self.push_region(value.present()?)?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::ByteRegion(Task::Value(right)));
                }
            }
            Task::IsPresent => {
                let value = self.pop_region()?;
                self.push_scalar(mir::ScalarValue::Bool(value.source.is_some()))?;
            }
            Task::Length(local) => {
                let length = self.region_local(local)?.length;
                self.push_scalar(mir::ScalarValue::Integer(
                    IntegerValue::from_i128(IntegerType::Int64, length as i128)
                        .expect("region length fits int"),
                ))?;
            }
            Task::Copy => {
                let source = self.pop_region()?.present()?;
                let destination = self.pop_region()?.present()?;
                if !destination.kind.is_writable() {
                    return Err(InterpreterError::new(
                        "MIR region copy writes through readonly access",
                    ));
                }
                let count = source.length.min(destination.length);
                if count != 0 {
                    let from = source.source.expect("present region");
                    let to = destination.source.expect("present region");
                    // Snapshot first: both regions may view one Bytes value.
                    let bytes = {
                        let (entries, _) = self.collection_element_storage(from)?;
                        let entries = entries.borrow();
                        entries[from.index..from.index + count]
                            .iter()
                            .map(|(_, value)| value.clone())
                            .collect::<Vec<_>>()
                    };
                    let (entries, _) = self.collection_element_storage(to)?;
                    let mut entries = entries.borrow_mut();
                    for (offset, value) in bytes.into_iter().enumerate() {
                        entries[to.index + offset].1 = value;
                    }
                }
                self.push_scalar(mir::ScalarValue::Integer(
                    IntegerValue::from_i128(IntegerType::Int64, count as i128)
                        .expect("region length fits int"),
                ))?;
            }
            Task::CopyWithin { region, span } => {
                let destination = self.pop_integer()?.signed_value() as i64;
                let length = self.pop_integer()?.signed_value() as i64;
                let offset = self.pop_integer()?.signed_value() as i64;
                let region = self.region_local(region)?;
                if !region.kind.is_writable() {
                    return Err(InterpreterError::new(
                        "MIR region copy writes through readonly access",
                    ));
                }
                self.current_frame_mut()?.active_panic_site = span;
                let Some((from, count)) = checked_range(offset, length, region.length) else {
                    return self.runtime_panic_step_with_facts_at(
                        "P1304",
                        span,
                        region_bounds_facts(offset, length, region.length),
                    );
                };
                let Some((to, _)) = checked_range(destination, length, region.length) else {
                    return self.runtime_panic_step_with_facts_at(
                        "P1304",
                        span,
                        region_bounds_facts(destination, length, region.length),
                    );
                };
                if count != 0 {
                    let base = region.source.expect("present region");
                    let (entries, _) = self.collection_element_storage(base)?;
                    let mut entries = entries.borrow_mut();
                    let moved = entries[base.index + from..base.index + from + count]
                        .iter()
                        .map(|(_, value)| value.clone())
                        .collect::<Vec<_>>();
                    for (offset, value) in moved.into_iter().enumerate() {
                        entries[base.index + to + offset].1 = value;
                    }
                }
            }
            Task::Index { region, span } | Task::Set { region, span } => {
                let write = matches!(task, Task::Set { .. });
                let replacement = if write {
                    Some(self.pop_scalar()?)
                } else {
                    None
                };
                let index = self.pop_integer()?.signed_value();
                let region = self.region_local(region)?;
                self.current_frame_mut()?.active_panic_site = span;
                if index < 0 || index as u128 >= region.length as u128 {
                    return self.collection_access_panic_step_at(
                        CollectionAccessError::Bounds {
                            code: "P1301",
                            index: index as i64,
                            length: region.length,
                        },
                        span,
                    );
                }
                let base = region.source.expect("present region");
                let (entries, _) = self.collection_element_storage(base)?;
                let position = base.index + index as usize;
                if let Some(value) = replacement {
                    if !region.kind.is_writable()
                        || value.ty() != mir::ScalarType::Integer(IntegerType::UInt8)
                    {
                        return Err(InterpreterError::new(
                            "MIR region write has an invalid capability or byte type",
                        ));
                    }
                    entries.borrow_mut()[position].1 = LocalValue::Scalar(value);
                } else {
                    let value = entries.borrow()[position].1.clone();
                    self.push_local_value(value)?;
                    self.current_frame_mut()?
                        .values
                        .set_place(Some(InterpreterPlace {
                            storage: InterpreterPlaceStorage::CollectionElement(
                                CollectionElementPlace {
                                    index: position,
                                    ..base
                                },
                            ),
                            ty: base.ty,
                            payload_projection: None,
                        }));
                }
            }
        }
        Ok(StepOutcome::Continue)
    }
}
