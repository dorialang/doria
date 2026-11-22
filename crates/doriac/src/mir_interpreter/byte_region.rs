//! Non-owning region descriptors over the interpreter's existing Bytes storage.
use super::*;
use crate::mir::RegionType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ByteRegionValue {
    pub kind: RegionType,
    pub nullable: bool,
    // Presence is independent of length; an empty region still has provenance.
    pub source: Option<RegionSource>,
    pub length: usize,
}

/// The storage a region views from its first element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegionSource {
    /// Elements of host collection storage: `Bytes`, a `T[]`, or a
    /// string's bytes.
    Elements(CollectionElementPlace),
    /// Elements in their C layout, in place: a fixed-length array field of
    /// a foreign struct or union (Decision 0148 section 6).
    Inline(super::foreign_aggregate::InlineStorage),
}

impl RegionSource {
    /// The host collection storage the region views; `None` for elements
    /// in their C layout.
    pub fn elements(self) -> Option<CollectionElementPlace> {
        match self {
            Self::Elements(place) => Some(place),
            Self::Inline(_) => None,
        }
    }

    pub fn elements_mut(&mut self) -> Option<&mut CollectionElementPlace> {
        match self {
            Self::Elements(place) => Some(place),
            Self::Inline(_) => None,
        }
    }

    /// The same storage `count` elements of `stride` bytes further on.
    pub fn advance(self, count: usize, stride: usize) -> Self {
        match self {
            Self::Elements(place) => Self::Elements(CollectionElementPlace {
                index: place.index + count,
                ..place
            }),
            Self::Inline(storage) => Self::Inline(storage.at((count * stride) as u64)),
        }
    }
}

impl ByteRegionValue {
    pub fn ty(self) -> mir::Type {
        if self.nullable {
            mir::Type::NullableRegion(self.kind)
        } else {
            mir::Type::Region(self.kind)
        }
    }

    pub fn absent(kind: RegionType) -> Self {
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
    Value(mir::RegionExpression),
    Nullable(mir::NullableRegionExpression),
    Build {
        kind: RegionType,
        span: Span,
    },
    Wrap,
    AssumePresent,
    Coalesce(mir::RegionExpression),
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
                mir::RegionExpression::CollectionIndex {
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
                mir::RegionExpression::Local {
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
                mir::RegionExpression::Region {
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
                // A host operation that builds a region over foreign memory
                // (Decision 0148 section 10): the range a `ForeignRange`
                // checked, viewed as address and length. Reading the owner
                // has no effect.
                mir::RegionExpression::Foreign {
                    kind, data, length, ..
                } => {
                    let address = match self.read_local_value(data)? {
                        LocalValue::Scalar(mir::ScalarValue::Inline(pointer)) => {
                            pointer.bits as u64
                        }
                        _ => {
                            return Err(InterpreterError::new(
                                "a region over foreign memory starts at a pointer",
                            ))
                        }
                    };
                    let LocalValue::Scalar(mir::ScalarValue::Integer(length)) =
                        self.read_local_value(length)?
                    else {
                        return Err(InterpreterError::new(
                            "a region over foreign memory has an integer length",
                        ));
                    };
                    self.push_region(ByteRegionValue {
                        kind,
                        nullable: false,
                        source: Some(RegionSource::Inline(
                            super::foreign_aggregate::InlineStorage::Foreign { address },
                        )),
                        length: length.signed_value() as usize,
                    })?;
                }
                mir::RegionExpression::Call {
                    kind,
                    function,
                    args,
                    span,
                    ..
                } => {
                    self.queue_call(
                        function,
                        args,
                        ReturnExpectation::Value(mir::Type::Region(kind)),
                        span,
                    )?;
                }
                mir::RegionExpression::AssumePresent { value, .. } => {
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::ByteRegion(Task::AssumePresent));
                    frame
                        .tasks
                        .push(EvaluationTask::ByteRegion(Task::Nullable(*value)));
                }
                mir::RegionExpression::Coalesce { left, right, .. } => {
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
                mir::NullableRegionExpression::CollectionIndex {
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
                mir::NullableRegionExpression::CollectionGet {
                    kind,
                    collection,
                    key,
                    index_span,
                    access,
                } => {
                    let frame = self.current_frame_mut()?;
                    frame.tasks.push(EvaluationTask::DictionaryGet {
                        collection,
                        expected: mir::Type::Region(kind),
                        access,
                    });
                    frame.tasks.push(EvaluationTask::SetPanicSite(index_span));
                    frame.tasks.push(EvaluationTask::Rvalue(*key));
                }
                mir::NullableRegionExpression::Null { kind } => {
                    self.push_region(ByteRegionValue::absent(kind))?
                }
                mir::NullableRegionExpression::Present(value) => {
                    let frame = self.current_frame_mut()?;
                    frame.tasks.push(EvaluationTask::ByteRegion(Task::Wrap));
                    frame
                        .tasks
                        .push(EvaluationTask::ByteRegion(Task::Value(value)));
                }
                mir::NullableRegionExpression::Local {
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
                mir::NullableRegionExpression::Call {
                    kind,
                    function,
                    args,
                    span,
                    ..
                } => {
                    self.queue_call(
                        function,
                        args,
                        ReturnExpectation::Value(mir::Type::NullableRegion(kind)),
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
                            RegionSource::Elements(CollectionElementPlace {
                                collection: identity,
                                generation,
                                index: 0,
                                ty: mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8)),
                            }),
                            bytes.entries().len(),
                        )
                    }
                    // A `T[]` gives a typed array region over its elements.
                    LocalValue::Collection(array)
                        if kind.array == Some(array.ty)
                            && self.program.collection_types[array.ty.0].kind
                                == mir::CollectionKind::TypedArray =>
                    {
                        let (identity, generation) =
                            self.register_borrowed_collection(Rc::downgrade(&array.entries));
                        (
                            RegionSource::Elements(CollectionElementPlace {
                                collection: identity,
                                generation,
                                index: 0,
                                ty: self.program.collection_types[array.ty.0].value,
                            }),
                            array.entries().len(),
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
                            RegionSource::Elements(CollectionElementPlace {
                                collection: identity,
                                generation,
                                index: 0,
                                ty: mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8)),
                            }),
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
                        if kind.array.is_some() {
                            "P1305"
                        } else {
                            "P1304"
                        },
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
                    source: Some(base.advance(offset, self.region_element(kind)?.1)),
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
                    // Snapshot first: both regions may view one storage.
                    let values = self.region_elements(source, 0, count)?;
                    self.write_region_elements(destination, 0, values)?;
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
                    let moved = self.region_elements(region, from, count)?;
                    self.write_region_elements(region, to, moved)?;
                }
            }
            Task::Index { region, span } | Task::Set { region, span } => {
                let write = matches!(task, Task::Set { .. });
                // An element is a scalar or a nullable pointer.
                let replacement = if write {
                    Some(self.pop_local_value()?)
                } else {
                    None
                };
                let index = self.pop_integer()?.signed_value();
                let region = self.region_local(region)?;
                self.current_frame_mut()?.active_panic_site = span;
                if index < 0 || index as u128 >= region.length as u128 {
                    // A typed array region panics as `T[]` indexing does.
                    return self.collection_access_panic_step_at(
                        CollectionAccessError::Bounds {
                            code: if region.kind.array.is_some() {
                                "P1310"
                            } else {
                                "P1301"
                            },
                            index: index as i64,
                            length: region.length,
                        },
                        span,
                    );
                }
                let index = index as usize;
                if let Some(value) = replacement {
                    let (element, _) = self.region_element(region.kind)?;
                    if !region.kind.is_writable()
                        || !local_value_matches_type(self.program, element, &value)
                    {
                        return Err(InterpreterError::new(
                            "MIR region write has an invalid capability or element type",
                        ));
                    }
                    self.write_region_elements(region, index, vec![value])?;
                } else {
                    let value = self
                        .region_elements(region, index, 1)?
                        .pop()
                        .expect("one element");
                    self.push_local_value(value)?;
                    // An element of host collection storage is a place a
                    // borrow of the result can name.
                    if let Some(RegionSource::Elements(base)) = region.source {
                        self.current_frame_mut()?
                            .values
                            .set_place(Some(InterpreterPlace {
                                storage: InterpreterPlaceStorage::CollectionElement(
                                    CollectionElementPlace {
                                        index: base.index + index,
                                        ..base
                                    },
                                ),
                                ty: base.ty,
                                payload_projection: None,
                            }));
                    }
                }
            }
        }
        Ok(StepOutcome::Continue)
    }
}

impl Interpreter<'_> {
    /// The element type a region of `kind` views and the bytes between
    /// its elements in their C layout: a byte, or its typed array's
    /// element.
    pub(super) fn region_element(
        &self,
        kind: RegionType,
    ) -> Result<(mir::Type, usize), InterpreterError> {
        match kind.array {
            None => Ok((
                mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8)),
                1,
            )),
            Some(_) => crate::native_abi::array_region_element(self.program, kind)
                .ok_or_else(|| InterpreterError::new("a typed array region views built elements")),
        }
    }

    /// `count` elements of the present region `region` from element `from`.
    pub(super) fn region_elements(
        &self,
        region: ByteRegionValue,
        from: usize,
        count: usize,
    ) -> Result<Vec<LocalValue>, InterpreterError> {
        let (element, stride) = self.region_element(region.kind)?;
        let source = region.present()?.source.expect("present region");
        self.source_elements(source, element, stride, from, count)
    }

    /// Stores `values` as the elements of the present region `region` from
    /// element `from`.
    pub(super) fn write_region_elements(
        &mut self,
        region: ByteRegionValue,
        from: usize,
        values: Vec<LocalValue>,
    ) -> Result<(), InterpreterError> {
        let (element, stride) = self.region_element(region.kind)?;
        let source = region.present()?.source.expect("present region");
        self.write_source_elements(source, element, stride, from, values)
    }

    /// `count` elements of type `element`, `stride` bytes apart in their C
    /// layout, of the storage `source` views, from element `from`.
    pub(super) fn source_elements(
        &self,
        source: RegionSource,
        element: mir::Type,
        stride: usize,
        from: usize,
        count: usize,
    ) -> Result<Vec<LocalValue>, InterpreterError> {
        match source {
            RegionSource::Elements(base) => {
                let (entries, index) = self.collection_element_storage(base)?;
                let entries = entries.borrow();
                let elements = entries
                    .get(index + from..index + from + count)
                    .ok_or_else(|| InterpreterError::new("a region lies inside its storage"))?;
                Ok(elements.iter().map(|(_, value)| value.clone()).collect())
            }
            RegionSource::Inline(storage) => {
                let bytes =
                    self.inline_bytes(storage.at((from * stride) as u64), count * stride)?;
                bytes
                    .chunks_exact(stride.max(1))
                    .take(count)
                    .map(|slot| c_value::decode(element, slot))
                    .collect()
            }
        }
    }

    /// Stores `values`, of type `element` and `stride` bytes apart in their
    /// C layout, as the elements of the storage `source` views from element
    /// `from`.
    pub(super) fn write_source_elements(
        &mut self,
        source: RegionSource,
        element: mir::Type,
        stride: usize,
        from: usize,
        values: Vec<LocalValue>,
    ) -> Result<(), InterpreterError> {
        match source {
            RegionSource::Elements(base) => {
                let (entries, index) = self.collection_element_storage(base)?;
                let mut entries = entries.borrow_mut();
                for (offset, value) in values.into_iter().enumerate() {
                    entries
                        .get_mut(index + from + offset)
                        .ok_or_else(|| InterpreterError::new("a region lies inside its storage"))?
                        .1 = value;
                }
                Ok(())
            }
            RegionSource::Inline(storage) => {
                let mut bytes = vec![0; values.len() * stride];
                for (slot, value) in bytes.chunks_exact_mut(stride.max(1)).zip(&values) {
                    c_value::encode(Some(value), element, slot);
                }
                self.write_inline_bytes(storage.at((from * stride) as u64), &bytes)
            }
        }
    }

    /// Whether the storage `source` views still exists: a collection a
    /// region borrowed may have been released after the borrow ended.
    pub(super) fn source_is_live(&self, source: RegionSource) -> bool {
        match source {
            RegionSource::Elements(base) => self.collection_element_storage(base).is_ok(),
            RegionSource::Inline(_) => true,
        }
    }
}

impl Interpreter<'_> {
    /// The first `count` bytes of the present byte region `region`, from
    /// whatever storage it views: host bytes or memory a foreign library
    /// owns (Decision 0148 section 10).
    pub(super) fn region_bytes(
        &self,
        region: ByteRegionValue,
        count: usize,
    ) -> Result<Vec<u8>, InterpreterError> {
        self.region_elements(region, 0, count)?
            .into_iter()
            .map(|value| match value {
                LocalValue::Scalar(mir::ScalarValue::Integer(value)) => {
                    Ok(value.unsigned_value() as u8)
                }
                _ => Err(InterpreterError::new("a byte region holds a non-byte")),
            })
            .collect()
    }

    /// Stores `bytes` as the first bytes of the present byte region
    /// `region`.
    pub(super) fn write_region_bytes(
        &mut self,
        region: ByteRegionValue,
        bytes: &[u8],
    ) -> Result<(), InterpreterError> {
        let values = bytes
            .iter()
            .map(|byte| {
                LocalValue::Scalar(mir::ScalarValue::Integer(
                    IntegerValue::from_i128(IntegerType::UInt8, i128::from(*byte))
                        .expect("a byte fits uint8"),
                ))
            })
            .collect();
        self.write_region_elements(region, 0, values)
    }
}
