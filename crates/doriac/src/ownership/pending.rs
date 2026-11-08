//! A pending request retains selected physical storage, even when preparation returns void.

use super::*;
use crate::class_layout::PropertyId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Input(BorrowSource),
    Capture(BindingId),
    Context(CallbackCapturePlace),
    Local(Span),
    Unresolved(Span),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub request: Origin,
    pub storage: Origin,
    pub inherited: bool,
    pub writable: bool,
    /// The first owned field denotes a finite subtree, not an unbounded recursive path.
    pub owned_path: Vec<PropertyId>,
    pub site: Span,
}

#[derive(Debug, Clone)]
pub(super) struct Loan {
    pub request: BorrowRoot,
    pub storage: RetainedLoan,
}

enum Input {
    Loan(RetainedLoan),
    Deferred(Origin),
}

impl Input {
    fn origin(&self, checker: &Checker<'_>) -> Origin {
        match self {
            Self::Loan(loan) => checker.pending_origin(loan),
            Self::Deferred(origin) => origin.clone(),
        }
    }
}

pub(super) fn append_selections(destination: &mut Vec<Selection>, incoming: Vec<Selection>) {
    for selection in incoming {
        if !destination.contains(&selection) {
            destination.push(selection);
        }
    }
}

pub(super) fn append_loans(destination: &mut Vec<Loan>, incoming: &[Loan]) {
    for loan in incoming {
        if !destination.iter().any(|old| {
            old.request == loan.request
                && old.storage.root == loan.storage.root
                && old.storage.access == loan.storage.access
                && old.storage.root_key == loan.storage.root_key
        }) {
            destination.push(loan.clone());
        }
    }
}

impl Checker<'_> {
    pub(super) fn index_pending_expressions(&mut self, program: &ast::Program) {
        let wanted = self
            .callback_capture_targets
            .values()
            .flatten()
            .flat_map(|target| &target.captures)
            .flat_map(|(_, sources)| sources)
            .filter_map(|place| match place.source {
                CallbackCaptureSource::Expression(span) => Some(span),
                CallbackCaptureSource::Binding(_) => None,
            })
            .collect::<HashSet<_>>();
        let mut visit = |expr: &Expr| {
            if wanted.contains(&expr.span()) {
                self.retained
                    .pending_expressions
                    .insert(expr.span(), expr.clone());
            }
        };
        for (function, _) in program_callables(program) {
            if let Some(body) = function.body.as_block() {
                ast::visit::block(body, &mut visit);
            }
        }
    }

    pub(super) fn seed_pending_file_selection(&mut self, function: &ast::FunctionDecl) {
        use crate::compiler_known_stream::FileFunction;
        let ast::FunctionBody::Intrinsic {
            implementation: ast::CompilerIntrinsic::File(implementation),
            ..
        } = function.body
        else {
            return;
        };
        if !matches!(
            implementation,
            FileFunction::PrepareRead | FileFunction::PrepareWrite
        ) {
            return;
        }
        self.seed_pending_selection(function.span, implementation == FileFunction::PrepareRead);
    }

    /// The canonical `prepareRead` and `prepareWrite` requirements select their
    /// storage argument by contract (Decision 0110). A call through the
    /// requirement carries that selection whichever implementations the
    /// program reaches; std pruning can leave it with none.
    pub(super) fn seed_pending_requirement_selection(&mut self, requirement: &ast::FunctionDecl) {
        use crate::compiler_known_stream::StreamOperation;
        let writable = match StreamOperation::from_requirement(requirement.span) {
            Some(StreamOperation::PrepareRead) => true,
            Some(StreamOperation::PrepareWrite) => false,
            _ => return,
        };
        self.seed_pending_selection(requirement.span, writable);
    }

    fn seed_pending_selection(&mut self, declaration: Span, writable: bool) {
        let selection = Selection {
            request: Origin::Input(BorrowSource::Parameter(0)),
            storage: Origin::Input(BorrowSource::Parameter(1)),
            inherited: true,
            writable,
            owned_path: Vec::new(),
            site: declaration,
        };
        append_selections(
            &mut self
                .retained
                .callables
                .entry(declaration)
                .or_default()
                .pending_selections,
            vec![selection],
        );
    }

    fn pending_origin(&self, loan: &RetainedLoan) -> Origin {
        if let Some(capture) = loan.pending_capture {
            return Origin::Capture(capture);
        }
        let Some(current) = self.retained.current.as_ref() else {
            return Origin::Unresolved(loan.capture_span);
        };
        if loan.root == BorrowRoot::Receiver && current.closure {
            if let Some(capture) = self
                .closures
                .get(&ClosureId::from_span(current.declaration))
                .into_iter()
                .flat_map(|info| &info.captures)
                .find(|capture| {
                    self.binding_resolution
                        .declarations_by_id
                        .get(&capture.source_binding_id)
                        .is_some_and(|binding| {
                            matches!(
                                binding.kind,
                                BindingKind::MethodReceiver | BindingKind::ClosureCapture
                            ) && binding.name == "this"
                        })
                })
            {
                return Origin::Capture(capture.environment_binding_id);
            }
        }
        match loan.root {
            BorrowRoot::Receiver if !current.closure && !current.owns_receiver => {
                Origin::Input(BorrowSource::Receiver)
            }
            BorrowRoot::Binding(binding) => {
                if let Some((index, (_, take))) = current
                    .parameters
                    .iter()
                    .enumerate()
                    .find(|(_, (id, _))| *id == Some(binding))
                {
                    if !take || loan.inherited {
                        return Origin::Input(BorrowSource::Parameter(index));
                    }
                }
                let Some(declaration) = self.binding_resolution.declarations_by_id.get(&binding)
                else {
                    return Origin::Unresolved(loan.capture_span);
                };
                if declaration.kind == BindingKind::ClosureCapture {
                    Origin::Capture(binding)
                } else {
                    Origin::Local(declaration.span.unwrap_or(loan.capture_span))
                }
            }
            BorrowRoot::Temporary | BorrowRoot::Receiver => Origin::Local(loan.capture_span),
            BorrowRoot::EnclosingEnvironment(_) => Origin::Unresolved(loan.capture_span),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pending_inputs(
        &self,
        origin: &Origin,
        inherited: bool,
        access: BorrowAccess,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        callback: Option<(&Expr, &CallbackCaptureTarget)>,
        site: Span,
        scopes: &Scopes,
    ) -> Vec<Input> {
        match origin {
            Origin::Input(source) => {
                let input = self.call_borrow_source_expr(
                    ReturnBorrow {
                        kind: ReturnBorrowKind::Retained,
                        source: *source,
                        writable: access == BorrowAccess::Writable,
                        lifetime: RetainedLifetime::Cleanup,
                    },
                    receiver,
                    signature,
                    args,
                );
                let Some(input) = input else {
                    return vec![Input::Deferred(Origin::Unresolved(site))];
                };
                self.pending_expression_inputs(input, inherited, access, scopes)
            }
            Origin::Capture(captured) => {
                let capture = callback
                    .and_then(|(_, target)| {
                        self.closures.get(&ClosureId::from_span(target.declaration))
                    })
                    .and_then(|info| {
                        info.captures
                            .iter()
                            .find(|capture| capture.environment_binding_id == *captured)
                    });
                let Some(capture) = capture else {
                    return vec![Input::Deferred(origin.clone())];
                };
                let (callee, target) = callback.expect("capture has a callback target");
                if capture.mode == ast::ClosureCaptureMode::Take && !inherited {
                    return self.pending_expression_inputs(callee, false, access, scopes);
                }
                if let Some((_, places)) = target.captures.iter().find(|(id, _)| *id == *captured) {
                    if !places.is_empty() {
                        return places
                            .iter()
                            .flat_map(|place| {
                                self.pending_context_inputs(place, inherited, access, site, scopes)
                            })
                            .collect();
                    }
                }
                self.pending_context_inputs(
                    &CallbackCapturePlace {
                        source: CallbackCaptureSource::Binding(capture.source_binding_id),
                        callers: Vec::new(),
                    },
                    inherited,
                    access,
                    site,
                    scopes,
                )
            }
            Origin::Context(place) => {
                let Some((next, remaining)) = place.callers.split_first() else {
                    return self.pending_context_inputs(place, inherited, access, site, scopes);
                };
                if *next != site {
                    return Vec::new();
                }
                self.pending_context_inputs(
                    &CallbackCapturePlace {
                        source: place.source,
                        callers: remaining.to_vec(),
                    },
                    inherited,
                    access,
                    site,
                    scopes,
                )
            }
            Origin::Local(_) | Origin::Unresolved(_) => vec![Input::Deferred(origin.clone())],
        }
    }

    fn pending_expression_inputs(
        &self,
        expr: &Expr,
        inherited: bool,
        access: BorrowAccess,
        scopes: &Scopes,
    ) -> Vec<Input> {
        if inherited
            && self
                .retained_value_from_expr(expr, scopes)
                .is_some_and(|value| value.leases.is_empty())
        {
            return Vec::new();
        }
        let loans = if inherited {
            self.reborrow_source_loans(expr, scopes, access)
        } else {
            self.retained_source_loans(expr, scopes, access)
        };
        if loans.is_empty() {
            if inherited
                && self
                    .retained_call(expr, scopes)
                    .is_some_and(|(signature, _, _)| {
                        signature.declaration.is_some_and(|declaration| {
                            self.retained.analyzed_callables.contains(&declaration)
                        })
                    })
            {
                return Vec::new();
            }
            vec![Input::Deferred(Origin::Unresolved(expr.span()))]
        } else {
            loans.into_iter().map(Input::Loan).collect()
        }
    }

    fn pending_context_inputs(
        &self,
        place: &CallbackCapturePlace,
        inherited: bool,
        access: BorrowAccess,
        site: Span,
        scopes: &Scopes,
    ) -> Vec<Input> {
        if !place.callers.is_empty() {
            return vec![Input::Deferred(Origin::Context(place.clone()))];
        }
        match place.source {
            CallbackCaptureSource::Expression(span) => self
                .retained
                .pending_expressions
                .get(&span)
                .map(|expr| self.pending_expression_inputs(expr, inherited, access, scopes))
                .unwrap_or_else(|| vec![Input::Deferred(Origin::Unresolved(span))]),
            CallbackCaptureSource::Binding(id) => {
                let Some((name, binding)) = scopes.get_by_canonical(id) else {
                    return vec![Input::Deferred(Origin::Unresolved(
                        self.binding_resolution
                            .declarations_by_id
                            .get(&id)
                            .and_then(|binding| binding.span)
                            .unwrap_or(site),
                    ))];
                };
                if inherited {
                    if let Some(value) = &binding.retained_value {
                        return value.leases.iter().cloned().map(Input::Loan).collect();
                    }
                }
                let declaration = self.binding_resolution.declarations_by_id.get(&id);
                let span = declaration.and_then(|binding| binding.span).unwrap_or(site);
                let source = if declaration
                    .is_some_and(|binding| binding.kind == BindingKind::MethodReceiver)
                {
                    BorrowSourceRoot::Place("$this".into())
                } else {
                    binding.borrow_root.clone().unwrap_or_else(|| {
                        BorrowSourceRoot::from_places(binding_roots(binding, name))
                    })
                };
                let loans = self.retained_loans_at_source(Some(source), span, scopes, access);
                if loans.is_empty() {
                    vec![Input::Deferred(Origin::Unresolved(span))]
                } else {
                    loans.into_iter().map(Input::Loan).collect()
                }
            }
        }
    }

    pub(super) fn check_pending_storage_access(
        &mut self,
        expr: &Expr,
        mode: UseMode,
        scopes: &Scopes,
    ) {
        if scopes.2.is_empty()
            || matches!(ungroup_expr(expr), Expr::This { .. }) && mode == UseMode::Read
        {
            return;
        }
        if mode == UseMode::Read {
            if let Expr::PropertyAccess {
                object, property, ..
            } = ungroup_expr(expr)
            {
                if self.resolved_type(object).is_some_and(|ty| {
                    crate::types::ByteRegionKind::from_resolved(ty).is_some_and(|kind| {
                        crate::byte_intrinsics::member(&ResolvedType::ByteRegion(kind), property)
                            .is_some_and(|member| member.descriptor_metadata)
                    })
                }) {
                    return;
                }
            }
        }
        if !matches!(
            ungroup_expr(expr),
            Expr::Variable { .. } | Expr::This { .. } | Expr::PropertyAccess { .. }
        ) {
            return;
        }
        let loans = self.reborrow_source_loans(expr, scopes, BorrowAccess::Readonly);
        let conflict = scopes.2.iter().find(|selected| {
            loans.iter().any(|requested| {
                requested.root == selected.storage.root
                    && requested.root_key.overlaps(&selected.storage.root_key)
                    && (mode != UseMode::Read || selected.storage.access == BorrowAccess::Writable)
            })
        });
        if let Some(conflict) = conflict {
            self.diagnostics.push(Diagnostic::new("E0774", "selected storage remains in use by a pending request", expr.span())
                .with_title("Pending Request Keeps Selected Storage In Use")
                .with_related(conflict.storage.capture_span, "Pending Storage Is Selected Here")
                .with_help("retire the pending operation before accessing, transferring or replacing its selected storage"));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn retain_pending_call_inputs(
        &mut self,
        site: Span,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        callback: Option<(&Expr, &CallbackCaptureTarget)>,
        scopes: &mut Scopes,
    ) {
        let Some(current) = self.retained.current.as_ref() else {
            return;
        };
        let declaration = current.declaration;
        let boundary = self.retained.pending_boundaries.contains(&declaration);
        let selections = signature
            .declaration
            .and_then(|declaration| self.retained.callables.get(&declaration))
            .map(|info| info.for_callers().pending_selections.clone())
            .unwrap_or_default();
        for selection in selections {
            let requests = self.pending_inputs(
                &selection.request,
                false,
                BorrowAccess::Writable,
                receiver,
                args,
                signature,
                callback,
                site,
                scopes,
            );
            let access = if selection.writable {
                BorrowAccess::Writable
            } else {
                BorrowAccess::Readonly
            };
            let mut storage = self.pending_inputs(
                &selection.storage,
                selection.inherited,
                access,
                receiver,
                args,
                signature,
                callback,
                site,
                scopes,
            );
            for input in &mut storage {
                if let Input::Loan(loan) = input {
                    loan.access = access;
                    self.extend_retained_place(
                        &mut loan.root_key,
                        &selection.owned_path,
                        signature,
                    );
                }
            }
            for request in &requests {
                for input in &storage {
                    let origin = input.origin(self);
                    let (inherited, projection, source) = match input {
                        Input::Loan(loan) => (
                            loan.inherited,
                            loan.root_key.fields.clone(),
                            loan.capture_span,
                        ),
                        Input::Deferred(_) => (
                            selection.inherited,
                            selection.owned_path.clone(),
                            selection.site,
                        ),
                    };
                    let selected = Selection {
                        request: request.origin(self),
                        storage: origin.clone(),
                        inherited,
                        writable: selection.writable,
                        owned_path: projection.clone(),
                        site: selection.site,
                    };
                    append_selections(
                        &mut self
                            .retained
                            .callables
                            .entry(declaration)
                            .or_default()
                            .pending_selections,
                        vec![selected],
                    );
                    let permitted = matches!(origin, Origin::Input(BorrowSource::Parameter(1)))
                        && inherited
                        || matches!(origin, Origin::Input(BorrowSource::Receiver))
                            && (inherited || !projection.is_empty());
                    if matches!(origin, Origin::Local(_)) || boundary && !permitted {
                        self.pending_storage_escape(site, source);
                    }
                    if let (Input::Loan(request), Input::Loan(storage)) = (request, input) {
                        append_loans(
                            &mut scopes.2,
                            &[Loan {
                                request: request.root.clone(),
                                storage: storage.clone(),
                            }],
                        );
                    }
                }
            }
        }
    }

    fn pending_storage_escape(&mut self, span: Span, source: Span) {
        self.diagnostics.push(Diagnostic::new("E0773",
            "a pending request cannot retain storage that ends with its preparation call", span)
            .with_title("Pending Request Cannot Outlive Selected Storage")
            .with_related(source, "Selected Storage Originates Here")
            .with_help("select storage owned by the complete registered adapter or retained by the transfer group"));
    }
}
