//! Which locals each MIR statement and terminator touches, and how. One
//! walk over the executable MIR that every consumer shares, first the
//! inputs and outputs of unsafe regions and the host operations of the
//! companion plan (Decision 0148 section 23), which `HostReach` reports.
//!
//! A report is conservative toward writing: a local whose storage a
//! statement may change is reported written, including a local passed to a
//! writable parameter, or to a call whose parameters the caller cannot
//! name (an indirect call), a local whose address is taken, the storage a
//! writable region views, and a collection an element is moved out of. Plans (`ControlFlowPlan`,
//! `MatchResultPlan`) describe code the other statements already contain,
//! and are not reported.

use super::*;

/// How a statement or terminator touches a local.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalAccess {
    Read,
    Write,
    /// The value's ownership leaves the local, or the local is destroyed.
    Move,
}

/// The mode of a direct call's parameter, by callee and argument position,
/// when the caller knows it.
pub type ParameterModes<'m> = &'m dyn Fn(FunctionId, usize) -> Option<FunctionParameterMode>;

/// What a statement or terminator does that only the host can run inside
/// an unsafe region (Decision 0148 section 23, entry ABI version 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostReach {
    /// A call: of the function named, or through a closure or interface
    /// value when `None`. A call of an extern declaration's function is a
    /// foreign call, which the companion runs itself.
    Call(Option<FunctionId>),
    /// The construction of a class.
    Construct,
    /// A static property read or write.
    Static,
    /// Doria I/O: output, file and stream I/O, `read_line`, the terminal.
    Io,
}

/// Every local `statement` touches, in the order its parts are written.
pub fn statement_locals(
    statement: &Statement,
    modes: ParameterModes<'_>,
    visit: &mut dyn FnMut(LocalId, LocalAccess),
) {
    statement_touches(statement, modes, visit, &mut |_| {});
}

/// Every local `terminator` touches, in the order its parts are written.
pub fn terminator_locals(
    terminator: &Terminator,
    modes: ParameterModes<'_>,
    visit: &mut dyn FnMut(LocalId, LocalAccess),
) {
    terminator_touches(terminator, modes, visit, &mut |_| {});
}

/// `statement_locals`, also reporting each part only the host can run.
pub fn statement_touches(
    statement: &Statement,
    modes: ParameterModes<'_>,
    visit: &mut dyn FnMut(LocalId, LocalAccess),
    host: &mut dyn FnMut(HostReach),
) {
    Walker {
        visit,
        modes,
        host,
        argument: false,
    }
    .statement(statement);
}

/// `terminator_locals`, also reporting each part only the host can run.
pub fn terminator_touches(
    terminator: &Terminator,
    modes: ParameterModes<'_>,
    visit: &mut dyn FnMut(LocalId, LocalAccess),
    host: &mut dyn FnMut(HostReach),
) {
    Walker {
        visit,
        modes,
        host,
        argument: false,
    }
    .terminator(terminator);
}

struct Walker<'v, 'h> {
    visit: &'v mut dyn FnMut(LocalId, LocalAccess),
    modes: ParameterModes<'v>,
    host: &'h mut dyn FnMut(HostReach),
    /// Inside a call argument, where a read may be a writable borrow.
    argument: bool,
}

impl Walker<'_, '_> {
    fn report(&mut self, local: LocalId, access: LocalAccess) {
        let access = if self.argument && access == LocalAccess::Read {
            LocalAccess::Write
        } else {
            access
        };
        (self.visit)(local, access);
    }

    fn read(&mut self, local: LocalId) {
        self.report(local, LocalAccess::Read);
    }

    fn write(&mut self, local: LocalId) {
        self.report(local, LocalAccess::Write);
    }

    fn moved(&mut self, local: LocalId) {
        self.report(local, LocalAccess::Move);
    }

    fn local(&mut self, local: LocalId, transfer: bool) {
        if transfer {
            self.moved(local);
        } else {
            self.read(local);
        }
    }

    /// A collection an element is read from, removed from or moved out of.
    fn element_source(&mut self, collection: LocalId, changes: bool) {
        if changes {
            self.write(collection);
        } else {
            self.read(collection);
        }
    }

    /// A direct call: a readonly or taking parameter reads its argument (a
    /// move is the argument's own transfer), a writable one may write it.
    fn call(&mut self, function: FunctionId, args: &[Rvalue]) {
        (self.host)(HostReach::Call(Some(function)));
        for (position, arg) in args.iter().enumerate() {
            match (self.modes)(function, position) {
                Some(FunctionParameterMode::Readonly | FunctionParameterMode::Take) => {
                    self.rvalue(arg)
                }
                Some(FunctionParameterMode::Writable) | None => {
                    self.arguments(std::slice::from_ref(arg))
                }
            }
        }
    }

    fn arguments(&mut self, args: &[Rvalue]) {
        let outer = std::mem::replace(&mut self.argument, true);
        for arg in args {
            self.rvalue(arg);
        }
        self.argument = outer;
    }

    fn statement(&mut self, statement: &Statement) {
        match statement {
            Statement::EchoStringLiteral(_) => (self.host)(HostReach::Io),
            Statement::SetDiagnosticSite { .. }
            | Statement::ControlFlowPlan(_)
            | Statement::MatchResultPlan { .. } => {}
            Statement::CompletePendingRequest {
                request, outcome, ..
            } => {
                self.write(*request);
                self.write(*outcome);
            }
            Statement::EndBorrow { local } => self.read(*local),
            Statement::AssignRegionIndex {
                region,
                index,
                value,
                ..
            } => {
                self.rvalue(index);
                self.rvalue(value);
                self.write(*region);
            }
            Statement::CopyWithinByteRegion {
                region,
                offset,
                length,
                destination,
                ..
            } => {
                self.rvalue(offset);
                self.rvalue(length);
                self.rvalue(destination);
                self.write(*region);
            }
            Statement::CoreCollection {
                collection,
                operation,
            } => {
                self.write(*collection);
                self.core_collection(operation);
            }
            Statement::AdvanceCollectionIterator { receiver, .. } => self.write(*receiver),
            Statement::BindClosureEnvironment {
                environment,
                bindings,
            } => {
                self.read(*environment);
                for (_, local) in bindings {
                    self.write(*local);
                }
            }
            Statement::AssignLocal { target, value } => {
                self.rvalue(value);
                self.write(*target);
            }
            Statement::AssignLocalGroup { targets, value } => {
                self.rvalue(value);
                for target in targets {
                    self.write(*target);
                }
            }
            Statement::BindPayloadEnumFields {
                source, targets, ..
            } => {
                self.write(*source);
                for target in targets {
                    self.write(*target);
                }
            }
            Statement::EchoString(value) | Statement::WriteStderr(value) => {
                (self.host)(HostReach::Io);
                self.string(value);
            }
            Statement::CallVoid { function, args, .. }
            | Statement::CallBorrowed { function, args, .. } => self.call(*function, args),
            Statement::CallNullSafe {
                object,
                function,
                args,
                ..
            } => {
                self.nullable_class(object);
                self.call(*function, args);
            }
            Statement::Printf(format) => {
                (self.host)(HostReach::Io);
                self.format(format);
            }
            Statement::WriteFile { path, contents } | Statement::AppendFile { path, contents } => {
                (self.host)(HostReach::Io);
                self.string(path);
                self.string(contents);
            }
            Statement::WriteFileBytes { path, contents, .. } => {
                (self.host)(HostReach::Io);
                self.string(path);
                self.read(*contents);
            }
            Statement::WriteStreamBytes { contents, .. } => {
                (self.host)(HostReach::Io);
                self.read(*contents);
            }
            Statement::AssignProperty { object, value, .. } => {
                self.rvalue(value);
                self.write(*object);
            }
            Statement::AssignStatic { value, .. } => {
                (self.host)(HostReach::Static);
                self.rvalue(value);
            }
            Statement::DropClass { local, .. }
            | Statement::DropSharedReference { local, .. }
            | Statement::DropWeakReference { local, .. }
            | Statement::DropWritableSharedReference { local, .. }
            | Statement::DropWritableWeakReference { local, .. }
            | Statement::DropSharedReferenceAccess { local, .. }
            | Statement::DropString { local }
            | Statement::DropMixed { local }
            | Statement::DropError { local }
            | Statement::DropCollection { local, .. }
            | Statement::DropPayloadEnum { local, .. }
            | Statement::DropInlineValue { local, .. }
            | Statement::DropFunction { local, .. } => self.moved(*local),
            Statement::CleanupConstructorPhase { object, .. } => self.write(*object),
            Statement::EnsureErrorOrigin { error, .. } => self.write(*error),
            Statement::ExtractErrorObject { target, error, .. } => {
                self.write(*error);
                self.write(*target);
            }
            Statement::TakeProperty { target, object, .. } => {
                self.write(*object);
                self.write(*target);
            }
            Statement::CollectionAdd {
                collection,
                value,
                index,
                ..
            } => {
                self.rvalue(value);
                if let Some(index) = index {
                    self.rvalue(index);
                }
                self.write(*collection);
            }
            Statement::CollectionSet {
                collection,
                key,
                value,
                ..
            } => {
                self.rvalue(key);
                self.rvalue(value);
                self.write(*collection);
            }
            Statement::AssignCollectionIndex {
                collection,
                index,
                value,
                ..
            } => {
                self.rvalue(index);
                self.rvalue(value);
                self.write(*collection);
            }
            Statement::CollectionClear { collection, .. } => self.write(*collection),
            Statement::Pointer {
                target, operation, ..
            } => {
                self.pointer_operation(operation);
                if let Some(target) = target {
                    self.write(*target);
                }
            }
            Statement::ForeignCall {
                function,
                args,
                target,
                ..
            } => {
                if let ForeignCallee::Pointer(pointer) = function.callee {
                    self.read(pointer);
                }
                for arg in args {
                    self.read(*arg);
                }
                if let Some(target) = target {
                    self.write(*target);
                }
            }
            Statement::ForeignField {
                aggregate, access, ..
            } => match access {
                ForeignFieldAccess::Read { target } => {
                    self.inline_place(aggregate, false);
                    self.write(*target);
                }
                ForeignFieldAccess::Write { value, .. } => {
                    self.rvalue(value);
                    self.inline_place(aggregate, true);
                }
                ForeignFieldAccess::Region { target, .. } => {
                    self.inline_place(aggregate, true);
                    self.write(*target);
                }
            },
        }
    }

    fn terminator(&mut self, terminator: &Terminator) {
        match terminator {
            Terminator::Return(value) => self.rvalue(value),
            Terminator::ReturnVoid | Terminator::Unreachable | Terminator::Jump(_) => {}
            Terminator::Panic { message, .. } => self.string(message),
            Terminator::Branch { condition, .. } => self.bool(condition),
            Terminator::CheckedCall {
                function,
                args,
                result,
                error,
                ..
            } => {
                self.call(*function, args);
                self.results(*result, Some(*error));
            }
            Terminator::IndirectCall {
                callee,
                args,
                result,
                ..
            } => {
                self.indirect_callee(callee);
                self.arguments(args);
                self.results(*result, None);
            }
            Terminator::CheckedIndirectCall {
                callee,
                args,
                result,
                error,
                ..
            } => {
                self.indirect_callee(callee);
                self.arguments(args);
                self.results(*result, Some(*error));
            }
            Terminator::CheckedConstruct {
                properties,
                constructor,
                args,
                result,
                error,
                ..
            } => {
                (self.host)(HostReach::Construct);
                self.property_values(properties);
                self.call(*constructor, args);
                self.results(Some(*result), Some(*error));
            }
            Terminator::CheckedIo {
                operation,
                result,
                error,
                ..
            } => {
                (self.host)(HostReach::Io);
                self.checked_io(operation);
                self.results(*result, Some(*error));
            }
            Terminator::CheckedDeadlineFromNow {
                duration,
                result,
                error,
                ..
            } => {
                self.read(*duration);
                self.results(*result, Some(*error));
            }
            Terminator::CheckedCancellationRequest {
                source,
                result,
                error,
                ..
            } => {
                self.write(*source);
                self.results(*result, Some(*error));
            }
            Terminator::ErrorSwitch { error, .. } => self.read(*error),
            Terminator::PropagateError { error } => self.moved(*error),
        }
    }

    fn results(&mut self, result: Option<LocalId>, error: Option<LocalId>) {
        if let Some(result) = result {
            self.write(result);
        }
        if let Some(error) = error {
            self.write(error);
        }
    }

    fn indirect_callee(&mut self, callee: &IndirectCallee) {
        (self.host)(HostReach::Call(None));
        match callee {
            IndirectCallee::Closure(function) => self.function(function),
            IndirectCallee::InterfaceMethod { receiver, .. } => self.write(*receiver),
        }
    }

    fn property_values(&mut self, properties: &[PropertyValue]) {
        for property in properties {
            match &property.source {
                PropertyValueSource::Expression(value) => self.rvalue(value),
                PropertyValueSource::ConstructorArgument(_)
                | PropertyValueSource::ConstructorBody => {}
            }
        }
    }

    fn core_collection(&mut self, operation: &CoreCollectionOperation) {
        match operation {
            CoreCollectionOperation::Insert {
                position,
                key,
                value,
                hash,
                ..
            } => {
                self.read(*position);
                if let Some(key) = key {
                    self.read(*key);
                }
                self.moved(*value);
                if let Some(hash) = hash {
                    self.read(*hash);
                }
            }
            CoreCollectionOperation::Remove {
                position,
                key,
                value,
            } => {
                self.read(*position);
                if let Some(key) = key {
                    self.write(*key);
                }
                self.write(*value);
            }
            CoreCollectionOperation::Swap { left, right } => {
                self.read(*left);
                self.read(*right);
            }
            CoreCollectionOperation::KeyAt { position, target }
            | CoreCollectionOperation::ValueAt { position, target } => {
                self.read(*position);
                self.write(*target);
            }
            CoreCollectionOperation::Exchange {
                position,
                value,
                previous,
                ..
            } => {
                self.read(*position);
                self.moved(*value);
                self.write(*previous);
            }
            CoreCollectionOperation::HashNext {
                hash,
                previous,
                target,
            } => {
                self.read(*hash);
                self.read(*previous);
                self.write(*target);
            }
            CoreCollectionOperation::HashPosition { slot, target } => {
                self.read(*slot);
                self.write(*target);
            }
            CoreCollectionOperation::RequireKey { position } => self.read(*position),
        }
    }

    fn pointer_operation(&mut self, operation: &PointerOperation) {
        match operation {
            PointerOperation::Read { pointer, .. }
            | PointerOperation::Address { pointer }
            | PointerOperation::ReadForeignString { pointer, .. } => self.read(*pointer),
            PointerOperation::Write { pointer, value, .. } => {
                self.read(*pointer);
                self.rvalue(value);
            }
            PointerOperation::Offset { pointer, count, .. } => {
                self.read(*pointer);
                self.integer(count);
            }
            PointerOperation::FromAddress { address } => self.integer(address),
            // The address of Doria storage lets foreign code write it.
            PointerOperation::AddressOf { place, .. } => self.inline_place(place, true),
            PointerOperation::RegionData { region, .. } => self.write(*region),
            PointerOperation::ForeignStringData { string } => self.write(*string),
            PointerOperation::CEntryAddress { .. } => {}
            PointerOperation::ForeignRange {
                pointer, length, ..
            } => {
                self.read(*pointer);
                self.integer(length);
            }
            PointerOperation::CreateHandle { value } => self.rvalue(value),
            PointerOperation::FromHandle { handle, .. } => self.read(*handle),
        }
    }

    fn inline_place(&mut self, place: &InlinePlace, writes: bool) {
        let storage = |walker: &mut Self, local: LocalId| {
            if writes {
                walker.write(local);
            } else {
                walker.read(local);
            }
        };
        match place {
            InlinePlace::Local(local) | InlinePlace::NullableLocalAssumeNonNull(local) => {
                storage(self, *local)
            }
            InlinePlace::Property { object, .. }
            | InlinePlace::NullablePropertyAssumeNonNull { object, .. } => storage(self, *object),
            InlinePlace::Static(_) | InlinePlace::NullableStaticAssumeNonNull(_) => {
                (self.host)(HostReach::Static)
            }
            InlinePlace::CollectionElement {
                collection, index, ..
            }
            | InlinePlace::NullableCollectionElementAssumeNonNull {
                collection, index, ..
            } => {
                storage(self, *collection);
                self.read(*index);
            }
            InlinePlace::RegionElement { region, index, .. } => {
                storage(self, *region);
                self.read(*index);
            }
        }
    }

    fn checked_io(&mut self, operation: &CheckedIoOperation) {
        // Every local an I/O operation names is runtime state it may change.
        match operation {
            CheckedIoOperation::ObserveTransferGroup { group, wait, .. } => {
                self.write(*group);
                self.write(*wait);
            }
            CheckedIoOperation::ResubmitTransfer {
                group,
                key,
                offset,
                length,
                ..
            } => {
                for local in [group, key, offset, length] {
                    self.write(*local);
                }
            }
            CheckedIoOperation::TransferGroupReadRegion { group, key }
            | CheckedIoOperation::ResubmitWatch { group, key } => {
                self.write(*group);
                self.write(*key);
            }
            CheckedIoOperation::BeginTransferGroup {
                set,
                plan,
                deadline,
                cancellation,
                ..
            } => {
                for local in [set, plan, deadline, cancellation] {
                    self.write(*local);
                }
            }
            CheckedIoOperation::TransferPlanKey { plan, position, .. } => {
                self.write(*plan);
                self.write(*position);
            }
            CheckedIoOperation::ConfigureTransferPlan {
                previous,
                key,
                region,
                ..
            } => {
                if let Some(previous) = previous {
                    self.write(*previous);
                }
                self.write(*key);
                self.write(*region);
            }
            CheckedIoOperation::WatchTransferPlan {
                previous,
                source,
                identity,
                ..
            } => {
                if let Some(previous) = previous {
                    self.write(*previous);
                }
                self.write(*source);
                self.write(*identity);
            }
            CheckedIoOperation::NewReadinessSet { .. } | CheckedIoOperation::ReadStdinBytes => {}
            CheckedIoOperation::RegisterReadinessStream { set, stream, .. } => {
                self.write(*set);
                self.write(*stream);
            }
            CheckedIoOperation::UnregisterReadinessStream { set, key, .. }
            | CheckedIoOperation::BorrowReadinessStream { set, key, .. } => {
                self.write(*set);
                self.write(*key);
            }
            CheckedIoOperation::PendingRequestOutcome { request, .. } => self.write(*request),
            CheckedIoOperation::PreparePendingFile {
                file,
                request,
                region,
            } => {
                for local in [file, request, region] {
                    self.write(*local);
                }
            }
            CheckedIoOperation::PreparePendingStandard {
                request, region, ..
            } => {
                self.write(*request);
                self.write(*region);
            }
            CheckedIoOperation::OpenOwnedFile {
                path,
                creation,
                mode,
                ..
            } => {
                self.string(path);
                for local in [creation, mode].into_iter().flatten() {
                    self.write(*local);
                }
            }
            CheckedIoOperation::ReadOwnedFile {
                file, destination, ..
            } => {
                self.write(*file);
                self.write(*destination);
            }
            CheckedIoOperation::WriteOwnedFile { file, source, .. } => {
                self.write(*file);
                self.write(*source);
            }
            CheckedIoOperation::TransferStandardBytes { region, .. } => self.write(*region),
            CheckedIoOperation::CloseOwnedFile { file } => self.write(*file),
            CheckedIoOperation::ControlOwnedFile { file, control } => {
                self.write(*file);
                self.file_control(control);
            }
            CheckedIoOperation::Process {
                process, arguments, ..
            } => {
                if let Some(process) = process {
                    self.write(*process);
                }
                for argument in arguments {
                    self.write(*argument);
                }
            }
            CheckedIoOperation::ReadLine { prompt } => self.string(prompt),
            CheckedIoOperation::ReadFile { path, .. } => self.string(path),
            CheckedIoOperation::WriteFile { path, contents, .. } => {
                self.string(path);
                self.io_contents(contents);
            }
            CheckedIoOperation::WriteStream { contents, .. } => self.io_contents(contents),
        }
    }

    fn file_control(&mut self, control: &FileControlOperation) {
        match control {
            FileControlOperation::SetLength { length } => self.read(*length),
            FileControlOperation::Seek { offset, origin } => {
                self.read(*offset);
                self.read(*origin);
            }
            FileControlOperation::Lock { mode } | FileControlOperation::TryLock { mode } => {
                self.read(*mode)
            }
            FileControlOperation::SynchronizeData
            | FileControlOperation::SynchronizeMetadata
            | FileControlOperation::Position
            | FileControlOperation::Length
            | FileControlOperation::Unlock
            | FileControlOperation::AbandonLock
            | FileControlOperation::CloseInput => {}
        }
    }

    fn io_contents(&mut self, contents: &IoContents) {
        match contents {
            IoContents::String(value) => self.string(value),
            IoContents::Format(format) => self.format(format),
            IoContents::Bytes(local) => self.read(*local),
        }
    }

    fn format(&mut self, format: &FormatExpression) {
        for argument in &format.arguments {
            match argument {
                FormatArgument::Value(value) => self.value(value),
                FormatArgument::String(value) | FormatArgument::ClassDisplay(value) => {
                    self.string(value)
                }
            }
        }
    }

    fn intrinsic(&mut self, call: &StringIntrinsicCall) {
        for arg in &call.args {
            self.rvalue(arg);
        }
    }

    fn rvalue(&mut self, value: &Rvalue) {
        match value {
            Rvalue::Value(value) => self.value(value),
            Rvalue::String(value) => self.string(value),
            Rvalue::Region(value) => self.region(value),
            Rvalue::NullableRegion(value) => self.nullable_region(value),
            Rvalue::Mixed(value) => self.mixed(value),
            Rvalue::NullableScalar(value) => self.nullable_scalar(value),
            Rvalue::NullableString(value) => self.nullable_string(value),
            Rvalue::NullableMixed(value) => self.nullable_mixed(value),
            Rvalue::Interface(value) => self.interface(&value.value),
            Rvalue::NullableInterface(value) => self.nullable_interface(&value.value),
            Rvalue::Class(value) => self.class(value),
            Rvalue::NullableClass(value) => self.nullable_class(value),
            Rvalue::SharedReference(value) => self.shared(value),
            Rvalue::WeakReference(value) => self.weak(value),
            Rvalue::NullableSharedReference(value) => self.nullable_shared(value),
            Rvalue::NullableWeakReference(value) => self.nullable_weak(value),
            Rvalue::WritableSharedReference(value) => self.writable_shared(value),
            Rvalue::WritableWeakReference(value) => self.writable_weak(value),
            Rvalue::NullableWritableSharedReference(value) => self.nullable_writable_shared(value),
            Rvalue::NullableWritableWeakReference(value) => self.nullable_writable_weak(value),
            Rvalue::SharedReferenceAccess(value) => self.access(value),
            Rvalue::NullableSharedReferenceAccess(value) => self.nullable_access(value),
            Rvalue::Collection(value) => self.collection(value),
            Rvalue::NullableCollection(value) => self.nullable_collection(value),
            Rvalue::PayloadEnum(value) => self.payload_enum(value),
            Rvalue::NullablePayloadEnum(value) => self.nullable_payload_enum(value),
            Rvalue::Function(value) => self.function(value),
            Rvalue::NullableFunction(value) => self.nullable_function(value),
            Rvalue::ForeignAggregate(value) => self.foreign_aggregate(value),
            Rvalue::NullableForeignAggregate(value) => self.nullable_foreign_aggregate(value),
        }
    }

    fn operand(&mut self, operand: &Operand) {
        match operand {
            Operand::RegionLength(local)
            | Operand::Local(local)
            | Operand::NullablePayload(local)
            | Operand::CollectionLength(local) => self.read(*local),
            Operand::RegionIndex { region, index, .. } => {
                self.rvalue(index);
                self.read(*region);
            }
            Operand::Scalar(_) => {}
            Operand::Static(_) => (self.host)(HostReach::Static),
            Operand::Property { object, .. } => self.read(*object),
            Operand::CollectionIteratorPosition { receiver, .. } => self.read(*receiver),
            Operand::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
            Operand::CollectionKeyAt {
                collection, offset, ..
            } => {
                self.rvalue(offset);
                self.read(*collection);
            }
            Operand::MixedPayload { mixed, .. } => self.read(*mixed),
            Operand::StringIntrinsic(call) => self.intrinsic(call),
        }
    }

    fn value(&mut self, value: &ValueExpression) {
        match value {
            ValueExpression::Integer(value) => self.integer(value),
            ValueExpression::Float(value) => self.float(value),
            ValueExpression::Bool(value) => self.bool(value),
            ValueExpression::Enum(value) => self.enumeration(value),
            ValueExpression::Inline(value) => self.inline(value),
        }
    }

    fn integer(&mut self, value: &IntegerExpression) {
        match value {
            IntegerExpression::InlineWord(value) => self.inline(value),
            IntegerExpression::LastSystemCode => {}
            IntegerExpression::Use { operand, .. } => self.operand(operand),
            IntegerExpression::Unary { operand, .. } => self.integer(operand),
            IntegerExpression::Binary { left, right, .. } => {
                self.integer(left);
                self.integer(right);
            }
            IntegerExpression::Convert { value, .. } => self.integer(value),
            IntegerExpression::FloatToInt { value, .. } => self.float(value),
            IntegerExpression::Call { function, args, .. } => self.call(*function, args),
            IntegerExpression::Coalesce { left, right, .. } => {
                self.nullable_scalar(left);
                self.integer(right);
            }
            IntegerExpression::EnumBacking { value, .. } => self.enumeration(value),
            IntegerExpression::CopyRegion {
                destination,
                source,
                ..
            } => {
                let outer = std::mem::replace(&mut self.argument, true);
                self.rvalue(destination);
                self.argument = outer;
                self.rvalue(source);
            }
            IntegerExpression::Terminal { call, .. } => {
                (self.host)(HostReach::Io);
                match call {
                    TerminalCall::Operate {
                        operation,
                        first,
                        second,
                        text,
                    } => {
                        for local in [operation, first, second, text] {
                            self.write(*local);
                        }
                    }
                    TerminalCall::AwaitEvent {
                        generation,
                        wait,
                        cancellation,
                    } => {
                        for local in [generation, wait, cancellation] {
                            self.write(*local);
                        }
                    }
                }
            }
        }
    }

    fn float(&mut self, value: &FloatExpression) {
        match value {
            FloatExpression::Use { operand, .. } => self.operand(operand),
            FloatExpression::Negate { operand, .. } => self.float(operand),
            FloatExpression::Binary { left, right, .. } => {
                self.float(left);
                self.float(right);
            }
            FloatExpression::IntToFloat { value } => self.integer(value),
            FloatExpression::Call { function, args, .. } => self.call(*function, args),
            FloatExpression::Coalesce { left, right, .. } => {
                self.nullable_scalar(left);
                self.float(right);
            }
        }
    }

    fn enumeration(&mut self, value: &EnumExpression) {
        match value {
            EnumExpression::Use { operand, .. } => self.operand(operand),
            EnumExpression::Case(_) => {}
            EnumExpression::Call { function, args, .. } => self.call(*function, args),
            EnumExpression::Coalesce { left, right, .. } => {
                self.nullable_scalar(left);
                self.enumeration(right);
            }
        }
    }

    fn inline(&mut self, value: &InlineValueExpression) {
        match value {
            InlineValueExpression::FromWord { word, .. } => self.integer(word),
            InlineValueExpression::InterfaceReceiver {
                receiver, transfer, ..
            } => self.local(*receiver, *transfer),
            InlineValueExpression::CancellationToken { source, .. } => self.read(*source),
            InlineValueExpression::Use { operand, .. } => self.operand(operand),
            InlineValueExpression::Call { function, args, .. } => self.call(*function, args),
            InlineValueExpression::Coalesce { left, right, .. } => {
                self.nullable_scalar(left);
                self.inline(right);
            }
        }
    }

    fn bool(&mut self, value: &BoolExpression) {
        match value {
            BoolExpression::CancellationRequested { source } => self.read(*source),
            BoolExpression::NominalIs { local, .. } => self.read(*local),
            BoolExpression::Use { operand } => self.operand(operand),
            BoolExpression::Compare { left, right, .. } => {
                self.value(left);
                self.value(right);
            }
            BoolExpression::StringCompare { left, right, .. } => {
                self.string(left);
                self.string(right);
            }
            BoolExpression::NullableStringCompare { left, right, .. } => {
                self.nullable_string(left);
                self.nullable_string(right);
            }
            BoolExpression::ClassIdentityCompare { left, right, .. }
            | BoolExpression::CollectionEqual { left, right } => {
                self.read(*left);
                self.read(*right);
            }
            BoolExpression::NullableScalarIsPresent(value) => self.nullable_scalar(value),
            BoolExpression::NullableClassIsPresent(value) => self.nullable_class(value),
            BoolExpression::ClassIs { value, .. } => self.nullable_class(value),
            BoolExpression::NullableCollectionIsPresent(value) => self.nullable_collection(value),
            BoolExpression::NullableByteRegionIsPresent(value) => self.nullable_region(value),
            BoolExpression::NullableSharedReferenceIsPresent(value) => self.nullable_shared(value),
            BoolExpression::NullableWeakReferenceIsPresent(value) => self.nullable_weak(value),
            BoolExpression::NullableWritableSharedReferenceIsPresent(value) => {
                self.nullable_writable_shared(value)
            }
            BoolExpression::NullableWritableWeakReferenceIsPresent(value) => {
                self.nullable_writable_weak(value)
            }
            BoolExpression::NullableSharedReferenceAccessIsPresent(value) => {
                self.nullable_access(value)
            }
            BoolExpression::NullableMixedIsPresent(value) => self.nullable_mixed(value),
            BoolExpression::NullableErrorIsPresent(value) => self.nullable_interface(&value.value),
            BoolExpression::NullablePayloadEnumIsPresent(value) => {
                self.nullable_payload_enum(value)
            }
            BoolExpression::NullableForeignAggregateIsPresent(value) => {
                self.nullable_foreign_aggregate(value)
            }
            BoolExpression::NullableFunctionIsPresent(value) => self.nullable_function(value),
            BoolExpression::PayloadEnumCompare { left, right, .. } => {
                self.payload_enum(left);
                self.payload_enum(right);
            }
            BoolExpression::PayloadEnumIsCase { local, .. } => self.read(*local),
            BoolExpression::NullablePayloadEnumCompare { left, right, .. } => {
                self.nullable_payload_enum(left);
                self.nullable_payload_enum(right);
            }
            BoolExpression::Not(value) => self.bool(value),
            BoolExpression::Binary { left, right, .. } => {
                self.bool(left);
                self.bool(right);
            }
            BoolExpression::Call { function, args, .. } => self.call(*function, args),
            BoolExpression::Coalesce { left, right } => {
                self.nullable_scalar(left);
                self.bool(right);
            }
            BoolExpression::CollectionHas {
                collection, value, ..
            } => {
                self.rvalue(value);
                self.read(*collection);
            }
            BoolExpression::CollectionIsEmpty { collection } => self.read(*collection),
            BoolExpression::MixedIs { mixed, .. } => self.mixed(mixed),
        }
    }

    fn string(&mut self, value: &StringExpression) {
        match value {
            StringExpression::Literal(_) => {}
            StringExpression::Static(_) => (self.host)(HostReach::Static),
            StringExpression::Local(local)
            | StringExpression::NullableLocalAssumeNonNull(local)
            | StringExpression::MixedPayload(local) => self.read(*local),
            StringExpression::Property { object, .. } => self.read(*object),
            StringExpression::ErrorMessage(value) => self.interface(value),
            StringExpression::Concat(parts) => {
                for part in parts {
                    self.string(part);
                }
            }
            StringExpression::Display(value) => self.value(value),
            StringExpression::Call { function, args, .. } => self.call(*function, args),
            StringExpression::ReadFile { path, .. } => self.string(path),
            StringExpression::Format(format) => self.format(format),
            StringExpression::Coalesce { left, right } => {
                self.nullable_string(left);
                self.string(right);
            }
            StringExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
            StringExpression::CollectionKeyAt {
                collection, offset, ..
            } => {
                self.rvalue(offset);
                self.read(*collection);
            }
            StringExpression::Intrinsic(call) => self.intrinsic(call),
            StringExpression::EnumBacking { value, .. } => self.enumeration(value),
        }
    }

    fn nullable_string(&mut self, value: &NullableStringExpression) {
        match value {
            NullableStringExpression::Null => {}
            NullableStringExpression::Static(_) => (self.host)(HostReach::Static),
            NullableStringExpression::String(value) => self.string(value),
            NullableStringExpression::Local(local) => self.read(*local),
            NullableStringExpression::Property { object, .. } => self.read(*object),
            NullableStringExpression::ReadLine { prompt, .. } => self.string(prompt),
            NullableStringExpression::Call { function, args, .. } => self.call(*function, args),
            NullableStringExpression::NullSafeProperty { object, .. } => {
                self.nullable_class(object)
            }
            NullableStringExpression::NullSafeCall {
                object,
                function,
                args,
                ..
            } => {
                self.nullable_class(object);
                self.call(*function, args);
            }
            NullableStringExpression::EnumBacking { value, .. } => self.nullable_scalar(value),
            NullableStringExpression::Coalesce { left, right } => {
                self.nullable_string(left);
                self.nullable_string(right);
            }
            NullableStringExpression::DictionaryGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.read(*collection);
            }
            NullableStringExpression::Intrinsic(call) => self.intrinsic(call),
        }
    }

    fn nullable_scalar(&mut self, value: &NullableScalarExpression) {
        match value {
            NullableScalarExpression::Null(_) => {}
            NullableScalarExpression::Static { .. } => (self.host)(HostReach::Static),
            NullableScalarExpression::InlineClaim { value, .. }
            | NullableScalarExpression::EnumBacking { value, .. } => self.nullable_scalar(value),
            NullableScalarExpression::TransferGroupKey { group, .. } => self.write(*group),
            NullableScalarExpression::Value(value) => self.value(value),
            NullableScalarExpression::Local { local, .. } => self.read(*local),
            NullableScalarExpression::Property { object, .. } => self.read(*object),
            NullableScalarExpression::Call { function, args, .. } => self.call(*function, args),
            NullableScalarExpression::NullSafeProperty { object, .. } => {
                self.nullable_class(object)
            }
            NullableScalarExpression::NullSafeCall {
                object,
                function,
                args,
                ..
            } => {
                self.nullable_class(object);
                self.call(*function, args);
            }
            NullableScalarExpression::Coalesce { left, right, .. } => {
                self.nullable_scalar(left);
                self.nullable_scalar(right);
            }
            NullableScalarExpression::DictionaryGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.read(*collection);
            }
            NullableScalarExpression::CollectionIndexOf { collection, value } => {
                self.rvalue(value);
                self.read(*collection);
            }
            NullableScalarExpression::RegionIndex { region, index, .. } => {
                self.rvalue(index);
                self.read(*region);
            }
            NullableScalarExpression::Parse { value, .. } => self.string(value),
            NullableScalarExpression::TryConvert { value, .. } => self.integer(value),
            NullableScalarExpression::StringIntrinsic(call) => self.intrinsic(call),
        }
    }

    fn region(&mut self, value: &RegionExpression) {
        match value {
            RegionExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
            RegionExpression::Region {
                kind,
                source,
                offset,
                length,
                ..
            } => {
                // A writable region lets its holder, or foreign code given its
                // pointer, write the storage it views.
                if kind.is_writable() {
                    self.arguments(std::slice::from_ref(source.as_ref()));
                } else {
                    self.rvalue(source);
                }
                self.rvalue(offset);
                self.rvalue(length);
            }
            RegionExpression::Local {
                local, transfer, ..
            } => self.local(*local, *transfer),
            RegionExpression::Call { function, args, .. } => self.call(*function, args),
            RegionExpression::AssumePresent { value, .. } => self.nullable_region(value),
            RegionExpression::Coalesce { left, right, .. } => {
                self.nullable_region(left);
                self.region(right);
            }
            RegionExpression::Foreign {
                owner,
                data,
                length,
                ..
            } => {
                self.rvalue(owner);
                self.read(*data);
                self.read(*length);
            }
        }
    }

    fn nullable_region(&mut self, value: &NullableRegionExpression) {
        match value {
            NullableRegionExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
            NullableRegionExpression::CollectionGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.read(*collection);
            }
            NullableRegionExpression::Null { .. } => {}
            NullableRegionExpression::Present(value) => self.region(value),
            NullableRegionExpression::Local {
                local, transfer, ..
            } => self.local(*local, *transfer),
            NullableRegionExpression::Call { function, args, .. } => self.call(*function, args),
        }
    }

    fn mixed(&mut self, value: &MixedExpression) {
        match value {
            MixedExpression::Null => {}
            MixedExpression::Local { local, transfer } => self.local(*local, *transfer),
            MixedExpression::Property { object, .. } => self.read(*object),
            MixedExpression::Call { function, args, .. } => self.call(*function, args),
            MixedExpression::BoxValue { value, .. } => self.value(value),
            MixedExpression::BoxForeignAggregate(value) => self.foreign_aggregate(value),
            MixedExpression::BoxString { value, .. } => self.string(value),
            MixedExpression::BoxClass { value, .. } => self.class(value),
            MixedExpression::BoxInterface { value, .. } => self.interface(&value.value),
            MixedExpression::BoxPayloadEnum { value } => self.payload_enum(value),
            MixedExpression::BoxFunction { value, .. } => self.function(value),
            MixedExpression::CollectionIndex {
                collection,
                index,
                transfer,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *transfer || *remove);
            }
        }
    }

    fn nullable_mixed(&mut self, value: &NullableMixedExpression) {
        match value {
            NullableMixedExpression::Null => {}
            NullableMixedExpression::Mixed(value) => self.mixed(value),
            NullableMixedExpression::BoxNullablePayloadEnum(value) => {
                self.nullable_payload_enum(value)
            }
            NullableMixedExpression::Local { local, transfer } => self.local(*local, *transfer),
            NullableMixedExpression::CollectionIndex {
                collection,
                index,
                transfer,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *transfer || *remove);
            }
            NullableMixedExpression::CollectionGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.read(*collection);
            }
            NullableMixedExpression::Property { object, .. } => self.read(*object),
            NullableMixedExpression::Call { function, args, .. } => self.call(*function, args),
            NullableMixedExpression::Coalesce { left, right, .. } => {
                self.nullable_mixed(left);
                self.nullable_mixed(right);
            }
        }
    }

    fn interface(&mut self, value: &InterfaceValue) {
        match value {
            InterfaceValue::SharedPayload { local } => self.read(*local),
            InterfaceValue::NarrowedLocal {
                local, transfer, ..
            }
            | InterfaceValue::Local { local, transfer }
            | InterfaceValue::NullableLocalAssumeNonNull { local, transfer } => {
                self.local(*local, *transfer)
            }
            InterfaceValue::Upcast { source, .. } => self.interface(&source.value),
            InterfaceValue::FromClass { object, .. } => self.class(object),
            InterfaceValue::FromNullableClass { object, .. } => self.nullable_class(object),
            InterfaceValue::FromValue { value, .. } => self.rvalue(value),
            InterfaceValue::NewCollectionIterator { source, .. } => self.read(*source),
            InterfaceValue::Property {
                object, transfer, ..
            } => self.element_source(*object, *transfer),
            InterfaceValue::Call { function, args, .. } => self.call(*function, args),
            InterfaceValue::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
        }
    }

    fn nullable_interface(&mut self, value: &NullableInterfaceValue) {
        match value {
            NullableInterfaceValue::SharedPayload { local } => self.read(*local),
            NullableInterfaceValue::Upcast { source, .. } => self.nullable_interface(&source.value),
            NullableInterfaceValue::Null => {}
            NullableInterfaceValue::Present(value) => self.interface(value),
            NullableInterfaceValue::Local { local, transfer } => self.local(*local, *transfer),
            NullableInterfaceValue::Property {
                object, transfer, ..
            } => self.element_source(*object, *transfer),
            NullableInterfaceValue::Call { function, args, .. } => self.call(*function, args),
            NullableInterfaceValue::DictionaryGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.read(*collection);
            }
            NullableInterfaceValue::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
        }
    }

    fn class(&mut self, value: &ClassExpression) {
        match value {
            ClassExpression::Upcast { value, .. } => self.class(value),
            ClassExpression::FileReadinessSource { file, .. } => self.write(*file),
            ClassExpression::StandardReadinessSource { borrowed_from, .. } => {
                self.write(*borrowed_from)
            }
            ClassExpression::InterfacePayload {
                local, transfer, ..
            }
            | ClassExpression::Local {
                local, transfer, ..
            }
            | ClassExpression::NullableLocalAssumeNonNull {
                local, transfer, ..
            }
            | ClassExpression::MixedPayload {
                mixed: local,
                transfer,
                ..
            } => self.local(*local, *transfer),
            ClassExpression::InterfaceReceiver {
                receiver, transfer, ..
            } => self.local(*receiver, *transfer),
            ClassExpression::Property { object, .. } => self.read(*object),
            ClassExpression::Call { function, args, .. } => self.call(*function, args),
            ClassExpression::New {
                properties,
                constructor,
                args,
                ..
            } => {
                (self.host)(HostReach::Construct);
                for property in properties {
                    match &property.source {
                        PropertyValueSource::Expression(value) => self.rvalue(value),
                        PropertyValueSource::ConstructorArgument(_)
                        | PropertyValueSource::ConstructorBody => {}
                    }
                }
                match constructor {
                    Some(constructor) => self.call(*constructor, args),
                    None => self.arguments(args),
                }
            }
            ClassExpression::Coalesce { left, right, .. } => {
                self.nullable_class(left);
                self.class(right);
            }
            ClassExpression::CollectionIndex {
                collection,
                index,
                transfer,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *transfer);
            }
            ClassExpression::SharedPayload { reference, .. } => self.shared(reference),
            ClassExpression::SharedAccessPayload { access, .. } => self.read(*access),
        }
    }

    fn nullable_class(&mut self, value: &NullableClassExpression) {
        match value {
            NullableClassExpression::Null(_) => {}
            NullableClassExpression::Class(value) => self.class(value),
            NullableClassExpression::Upcast { value, .. } => self.nullable_class(value),
            NullableClassExpression::SharedPayload { reference, .. } => {
                self.nullable_shared(reference)
            }
            NullableClassExpression::Local {
                local, transfer, ..
            } => self.local(*local, *transfer),
            NullableClassExpression::Property { object, .. } => self.read(*object),
            NullableClassExpression::Call { function, args, .. } => self.call(*function, args),
            NullableClassExpression::NullSafeProperty { object, .. } => self.nullable_class(object),
            NullableClassExpression::NullSafeCall {
                object,
                function,
                args,
                ..
            } => {
                self.nullable_class(object);
                self.call(*function, args);
            }
            NullableClassExpression::Coalesce { left, right, .. } => {
                self.nullable_class(left);
                self.nullable_class(right);
            }
            NullableClassExpression::DictionaryGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.read(*collection);
            }
        }
    }

    fn shared(&mut self, value: &SharedReferenceExpression) {
        match value {
            SharedReferenceExpression::New { value, .. } => self.rvalue(value),
            SharedReferenceExpression::Local {
                local, transfer, ..
            }
            | SharedReferenceExpression::NullableLocalAssumeNonNull {
                local, transfer, ..
            } => self.local(*local, *transfer),
            SharedReferenceExpression::Property { object, .. } => self.read(*object),
            SharedReferenceExpression::Call { function, args, .. } => self.call(*function, args),
            SharedReferenceExpression::Share { value, .. } => self.shared(value),
            SharedReferenceExpression::Coalesce { left, right, .. } => {
                self.nullable_shared(left);
                self.shared(right);
            }
            SharedReferenceExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
        }
    }

    fn weak(&mut self, value: &WeakReferenceExpression) {
        match value {
            WeakReferenceExpression::Local {
                local, transfer, ..
            }
            | WeakReferenceExpression::NullableLocalAssumeNonNull {
                local, transfer, ..
            } => self.local(*local, *transfer),
            WeakReferenceExpression::Property { object, .. } => self.read(*object),
            WeakReferenceExpression::Call { function, args, .. } => self.call(*function, args),
            WeakReferenceExpression::Create { value, .. } => self.shared(value),
            WeakReferenceExpression::Coalesce { left, right, .. } => {
                self.nullable_weak(left);
                self.weak(right);
            }
            WeakReferenceExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
        }
    }

    fn nullable_shared(&mut self, value: &NullableSharedReferenceExpression) {
        match value {
            NullableSharedReferenceExpression::Null(_) => {}
            NullableSharedReferenceExpression::Shared(value) => self.shared(value),
            NullableSharedReferenceExpression::Local {
                local, transfer, ..
            } => self.local(*local, *transfer),
            NullableSharedReferenceExpression::Property { object, .. } => self.read(*object),
            NullableSharedReferenceExpression::Call { function, args, .. } => {
                self.call(*function, args)
            }
            NullableSharedReferenceExpression::Acquire { value, .. } => self.weak(value),
            NullableSharedReferenceExpression::NullSafeShare { value, .. } => {
                self.nullable_shared(value)
            }
            NullableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
                self.nullable_weak(value)
            }
            NullableSharedReferenceExpression::Coalesce { left, right, .. } => {
                self.nullable_shared(left);
                self.nullable_shared(right);
            }
            NullableSharedReferenceExpression::DictionaryGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.read(*collection);
            }
            NullableSharedReferenceExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
        }
    }

    fn nullable_weak(&mut self, value: &NullableWeakReferenceExpression) {
        match value {
            NullableWeakReferenceExpression::Null(_) => {}
            NullableWeakReferenceExpression::Weak(value) => self.weak(value),
            NullableWeakReferenceExpression::Local {
                local, transfer, ..
            } => self.local(*local, *transfer),
            NullableWeakReferenceExpression::Property { object, .. } => self.read(*object),
            NullableWeakReferenceExpression::Call { function, args, .. } => {
                self.call(*function, args)
            }
            NullableWeakReferenceExpression::NullSafeCreate { value, .. } => {
                self.nullable_shared(value)
            }
            NullableWeakReferenceExpression::Coalesce { left, right, .. } => {
                self.nullable_weak(left);
                self.nullable_weak(right);
            }
            NullableWeakReferenceExpression::DictionaryGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.read(*collection);
            }
            NullableWeakReferenceExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
        }
    }

    fn writable_shared(&mut self, value: &WritableSharedReferenceExpression) {
        match value {
            WritableSharedReferenceExpression::New { value, .. } => self.rvalue(value),
            WritableSharedReferenceExpression::Local {
                local, transfer, ..
            }
            | WritableSharedReferenceExpression::NullableLocalAssumeNonNull {
                local,
                transfer,
                ..
            } => self.local(*local, *transfer),
            WritableSharedReferenceExpression::Property { object, .. } => self.read(*object),
            WritableSharedReferenceExpression::Call { function, args, .. } => {
                self.call(*function, args)
            }
            WritableSharedReferenceExpression::Share { value, .. } => self.writable_shared(value),
            WritableSharedReferenceExpression::Coalesce { left, right, .. } => {
                self.nullable_writable_shared(left);
                self.writable_shared(right);
            }
            WritableSharedReferenceExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
        }
    }

    fn writable_weak(&mut self, value: &WritableWeakReferenceExpression) {
        match value {
            WritableWeakReferenceExpression::Local {
                local, transfer, ..
            }
            | WritableWeakReferenceExpression::NullableLocalAssumeNonNull {
                local, transfer, ..
            } => self.local(*local, *transfer),
            WritableWeakReferenceExpression::Property { object, .. } => self.read(*object),
            WritableWeakReferenceExpression::Call { function, args, .. } => {
                self.call(*function, args)
            }
            WritableWeakReferenceExpression::Create { value, .. } => self.writable_shared(value),
            WritableWeakReferenceExpression::Coalesce { left, right, .. } => {
                self.nullable_writable_weak(left);
                self.writable_weak(right);
            }
            WritableWeakReferenceExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
        }
    }

    fn nullable_writable_shared(&mut self, value: &NullableWritableSharedReferenceExpression) {
        match value {
            NullableWritableSharedReferenceExpression::Null(_) => {}
            NullableWritableSharedReferenceExpression::Strong(value) => self.writable_shared(value),
            NullableWritableSharedReferenceExpression::Local {
                local, transfer, ..
            } => self.local(*local, *transfer),
            NullableWritableSharedReferenceExpression::Property { object, .. } => {
                self.read(*object)
            }
            NullableWritableSharedReferenceExpression::Call { function, args, .. } => {
                self.call(*function, args)
            }
            NullableWritableSharedReferenceExpression::Acquire { value, .. } => {
                self.writable_weak(value)
            }
            NullableWritableSharedReferenceExpression::NullSafeShare { value, .. } => {
                self.nullable_writable_shared(value)
            }
            NullableWritableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
                self.nullable_writable_weak(value)
            }
            NullableWritableSharedReferenceExpression::Coalesce { left, right, .. } => {
                self.nullable_writable_shared(left);
                self.nullable_writable_shared(right);
            }
            NullableWritableSharedReferenceExpression::DictionaryGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.read(*collection);
            }
        }
    }

    fn nullable_writable_weak(&mut self, value: &NullableWritableWeakReferenceExpression) {
        match value {
            NullableWritableWeakReferenceExpression::Null(_) => {}
            NullableWritableWeakReferenceExpression::Weak(value) => self.writable_weak(value),
            NullableWritableWeakReferenceExpression::Local {
                local, transfer, ..
            } => self.local(*local, *transfer),
            NullableWritableWeakReferenceExpression::Property { object, .. } => self.read(*object),
            NullableWritableWeakReferenceExpression::Call { function, args, .. } => {
                self.call(*function, args)
            }
            NullableWritableWeakReferenceExpression::NullSafeCreate { value, .. } => {
                self.nullable_writable_shared(value)
            }
            NullableWritableWeakReferenceExpression::Coalesce { left, right, .. } => {
                self.nullable_writable_weak(left);
                self.nullable_writable_weak(right);
            }
            NullableWritableWeakReferenceExpression::DictionaryGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.read(*collection);
            }
        }
    }

    fn access(&mut self, value: &SharedReferenceAccessExpression) {
        match value {
            SharedReferenceAccessExpression::Local {
                local, transfer, ..
            }
            | SharedReferenceAccessExpression::NullableLocalAssumeNonNull {
                local, transfer, ..
            } => self.local(*local, *transfer),
            SharedReferenceAccessExpression::Property { object, .. } => self.read(*object),
            SharedReferenceAccessExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
            SharedReferenceAccessExpression::Call { function, args, .. } => {
                self.call(*function, args)
            }
            SharedReferenceAccessExpression::Acquire { value, .. } => self.writable_shared(value),
        }
    }

    fn nullable_access(&mut self, value: &NullableSharedReferenceAccessExpression) {
        match value {
            NullableSharedReferenceAccessExpression::Null { .. } => {}
            NullableSharedReferenceAccessExpression::Access(value) => self.access(value),
            NullableSharedReferenceAccessExpression::Local {
                local, transfer, ..
            } => self.local(*local, *transfer),
            NullableSharedReferenceAccessExpression::Property { object, .. } => self.read(*object),
            NullableSharedReferenceAccessExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
            NullableSharedReferenceAccessExpression::CollectionGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.read(*collection);
            }
            NullableSharedReferenceAccessExpression::Call { function, args, .. } => {
                self.call(*function, args)
            }
            NullableSharedReferenceAccessExpression::NullSafeAcquire { value, .. } => {
                self.nullable_writable_shared(value)
            }
        }
    }

    fn collection(&mut self, value: &CollectionExpression) {
        match value {
            CollectionExpression::InterfaceReceiver { receiver, .. }
            | CollectionExpression::IteratorSource { receiver, .. } => self.read(*receiver),
            CollectionExpression::Local {
                local, transfer, ..
            } => self.local(*local, *transfer),
            CollectionExpression::Literal { entries, .. } => {
                for entry in entries {
                    if let Some(key) = &entry.key {
                        self.rvalue(key);
                    }
                    self.rvalue(&entry.value);
                }
            }
            CollectionExpression::Fill { value, count, .. } => {
                self.rvalue(value);
                self.integer(count);
            }
            CollectionExpression::ConstructionCapacity { count, .. } => self.integer(count),
            CollectionExpression::FinishConstruction { source, .. } => self.moved(*source),
            CollectionExpression::Index {
                source,
                index,
                transfer,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*source, *transfer);
            }
            CollectionExpression::Property { object, .. } => self.read(*object),
            CollectionExpression::SharedAccessPayload { access, .. } => self.read(*access),
            CollectionExpression::From {
                source,
                transfer,
                algebra,
                ..
            } => {
                self.local(*source, *transfer);
                if let Some((_, other)) = algebra {
                    self.read(*other);
                }
            }
            CollectionExpression::FromBytes { source, .. }
            | CollectionExpression::BytesFromArray { source, .. } => self.read(*source),
            CollectionExpression::BytesAllocate { length, .. } => self.integer(length),
            CollectionExpression::ReadFileBytes { path, .. } => self.string(path),
            CollectionExpression::ReadStdinBytes { .. } => {}
            CollectionExpression::ForeignString { text, .. } => self.string(text),
            CollectionExpression::StringIntrinsic(call) => self.intrinsic(call),
            CollectionExpression::Call { function, args, .. } => self.call(*function, args),
        }
    }

    fn nullable_collection(&mut self, value: &NullableCollectionExpression) {
        match value {
            NullableCollectionExpression::Null(_) => {}
            NullableCollectionExpression::TryForeignString { text, .. } => self.string(text),
            NullableCollectionExpression::Collection(value) => self.collection(value),
            NullableCollectionExpression::Local {
                local, transfer, ..
            } => self.local(*local, *transfer),
            NullableCollectionExpression::Index {
                source,
                index,
                transfer,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*source, *transfer);
            }
            NullableCollectionExpression::CollectionGet { source, key, .. } => {
                self.rvalue(key);
                self.read(*source);
            }
            NullableCollectionExpression::Property { object, .. } => self.read(*object),
            NullableCollectionExpression::Call { function, args, .. } => self.call(*function, args),
            NullableCollectionExpression::Coalesce { left, right, .. } => {
                self.nullable_collection(left);
                self.nullable_collection(right);
            }
        }
    }

    fn payload_place(&mut self, place: &PayloadEnumPlace) {
        match place {
            PayloadEnumPlace::Local(local)
            | PayloadEnumPlace::NullableLocalAssumeNonNull(local) => self.write(*local),
            PayloadEnumPlace::Static(_) => (self.host)(HostReach::Static),
            PayloadEnumPlace::Property { object, .. } => self.write(*object),
            PayloadEnumPlace::CollectionIndex {
                collection, index, ..
            } => {
                self.rvalue(index);
                self.write(*collection);
            }
            PayloadEnumPlace::MixedPayload { mixed } => self.write(*mixed),
        }
    }

    fn payload_enum(&mut self, value: &PayloadEnumExpression) {
        match value {
            PayloadEnumExpression::Construct { fields, .. } => {
                for field in fields {
                    self.rvalue(field);
                }
            }
            PayloadEnumExpression::Use { place, .. } => self.payload_place(place),
            PayloadEnumExpression::Call { function, args, .. } => self.call(*function, args),
            PayloadEnumExpression::Coalesce { left, right, .. } => {
                self.nullable_payload_enum(left);
                self.payload_enum(right);
            }
        }
    }

    fn nullable_payload_enum(&mut self, value: &NullablePayloadEnumExpression) {
        match value {
            NullablePayloadEnumExpression::Null(_) => {}
            NullablePayloadEnumExpression::Value(value) => self.payload_enum(value),
            NullablePayloadEnumExpression::Use { place, .. } => self.payload_place(place),
            NullablePayloadEnumExpression::Call { function, args, .. } => {
                self.call(*function, args)
            }
            NullablePayloadEnumExpression::CollectionGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.write(*collection);
            }
            NullablePayloadEnumExpression::Coalesce { left, right, .. } => {
                self.nullable_payload_enum(left);
                self.nullable_payload_enum(right);
            }
        }
    }

    fn function(&mut self, value: &FunctionExpression) {
        match value {
            FunctionExpression::Create { captures, .. } => {
                for capture in captures {
                    match capture {
                        ClosureCaptureOperand::BorrowLocal { local, writable } => {
                            if *writable {
                                self.write(*local);
                            } else {
                                self.read(*local);
                            }
                        }
                        ClosureCaptureOperand::CopyValue(value)
                        | ClosureCaptureOperand::MoveValue(value) => self.rvalue(value),
                    }
                }
            }
            FunctionExpression::Local {
                local, transfer, ..
            }
            | FunctionExpression::MixedPayload {
                mixed: local,
                transfer,
                ..
            } => self.local(*local, *transfer),
            FunctionExpression::Property { object, .. } => self.read(*object),
            FunctionExpression::Call { function, args, .. } => self.call(*function, args),
            FunctionExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
            FunctionExpression::AssumePresent { value, .. } => self.nullable_function(value),
        }
    }

    fn nullable_function(&mut self, value: &NullableFunctionExpression) {
        match value {
            NullableFunctionExpression::Null { .. } => {}
            NullableFunctionExpression::Present(value) => self.function(value),
            NullableFunctionExpression::Local {
                local, transfer, ..
            } => self.local(*local, *transfer),
            NullableFunctionExpression::Property { object, .. } => self.read(*object),
            NullableFunctionExpression::Call { function, args, .. } => self.call(*function, args),
            NullableFunctionExpression::DictionaryGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.read(*collection);
            }
            NullableFunctionExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
        }
    }

    fn foreign_aggregate(&mut self, value: &ForeignAggregateExpression) {
        match value {
            ForeignAggregateExpression::Construct { fields, .. } => {
                for field in fields {
                    self.rvalue(&field.value);
                }
            }
            ForeignAggregateExpression::Local { local, .. }
            | ForeignAggregateExpression::NullableLocalAssumeNonNull { local, .. }
            | ForeignAggregateExpression::MixedPayload { mixed: local, .. } => self.read(*local),
            ForeignAggregateExpression::Property { object, .. } => self.read(*object),
            ForeignAggregateExpression::Static { .. } => (self.host)(HostReach::Static),
            ForeignAggregateExpression::Call { function, args, .. } => self.call(*function, args),
            ForeignAggregateExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
            ForeignAggregateExpression::RegionIndex { region, index, .. } => {
                self.rvalue(index);
                self.read(*region);
            }
        }
    }

    fn nullable_foreign_aggregate(&mut self, value: &NullableForeignAggregateExpression) {
        match value {
            NullableForeignAggregateExpression::Null(_) => {}
            NullableForeignAggregateExpression::Static { .. } => (self.host)(HostReach::Static),
            NullableForeignAggregateExpression::Present(value) => self.foreign_aggregate(value),
            NullableForeignAggregateExpression::Local { local, .. } => self.read(*local),
            NullableForeignAggregateExpression::Property { object, .. } => self.read(*object),
            NullableForeignAggregateExpression::Call { function, args, .. } => {
                self.call(*function, args)
            }
            NullableForeignAggregateExpression::CollectionIndex {
                collection,
                index,
                remove,
                ..
            } => {
                self.rvalue(index);
                self.element_source(*collection, *remove);
            }
            NullableForeignAggregateExpression::CollectionGet {
                collection, key, ..
            } => {
                self.rvalue(key);
                self.read(*collection);
            }
        }
    }
}
