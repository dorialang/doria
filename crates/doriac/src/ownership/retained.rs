//! Source loans carried by an owned iterator, independently of cursor ownership.

use super::*;

#[derive(Default)]
pub(super) struct Analysis {
    pub callables: HashMap<Span, RetainedCallableInfo>,
    pub current: Option<FunctionContext>,
    pub expression_values: HashMap<Span, RetainedValueState>,
    pub yielded_values: Vec<Option<RetainedValueState>>,
    source_carriers: HashSet<String>,
    iterator_interfaces: HashSet<String>,
    requirements: HashMap<Span, Signature>,
    implementations: HashMap<Span, Vec<Span>>,
    ancestors: HashMap<String, Vec<String>>,
}

pub(super) struct FunctionContext {
    declaration: Span,
    parameters: Vec<(String, bool)>,
    constructor: bool,
}

impl Analysis {
    pub fn new(
        classes: &[crate::semantics::ClassSemanticInfo],
        contracts: &crate::semantics::contracts::ContractFacts,
        move_enums: &HashSet<String>,
    ) -> Self {
        let mut analysis = Self {
            source_carriers: classes
                .iter()
                .filter(|class| {
                    class
                        .properties
                        .iter()
                        .any(|property| property.borrowed_source)
                })
                .map(|class| class.declaration_name.clone())
                .collect(),
            ancestors: classes
                .iter()
                .map(|class| {
                    (
                        class.declaration_name.clone(),
                        class
                            .ancestors
                            .iter()
                            .map(|ancestor| ancestor.name.clone())
                            .collect(),
                    )
                })
                .collect(),
            ..Self::default()
        };
        for interface in contracts
            .interface_specializations
            .iter()
            .filter(|interface| interface.valid)
        {
            if interface.requirements.iter().any(|requirement| {
                requirement.origins.iter().any(|origin| {
                    origin.interface.name == "Iterator"
                        && origin.declaration.source == crate::compiler_known_contracts::SOURCE_ID
                })
            }) {
                analysis
                    .iterator_interfaces
                    .insert(interface.specialization.name.clone());
            }
            for requirement in &interface.requirements {
                for origin in &requirement.origins {
                    let signature = Signature {
                        declaration: Some(origin.declaration),
                        params: requirement
                            .signature
                            .parameters
                            .iter()
                            .map(|parameter| Parameter {
                                name: parameter.name.clone(),
                                move_type: resolved_type_is_move_type(
                                    &parameter.r#type,
                                    move_enums,
                                ) || resolved_type_requires_conservative_move(
                                    &parameter.r#type,
                                ),
                                class_type: matches!(parameter.r#type, ResolvedType::Class(_)),
                                generic: resolved_type_requires_conservative_move(
                                    &parameter.r#type,
                                ),
                                take: parameter.take,
                                writable: parameter.writable,
                                borrow: false,
                            })
                            .collect(),
                        returns_move_type: resolved_type_is_move_type(
                            &requirement.signature.return_type,
                            move_enums,
                        ),
                        return_borrow: requirement.return_borrow,
                        receiver: Some(if requirement.writable_receiver {
                            UseMode::Write
                        } else {
                            UseMode::Read
                        }),
                        ..Signature::default()
                    };
                    analysis
                        .requirements
                        .entry(origin.declaration)
                        .or_insert(signature);
                    if requirement.name == "iterator"
                        && requirement.origins.iter().any(|origin| {
                            origin.declaration.source == crate::compiler_known_contracts::SOURCE_ID
                                && origin.interface.name == "Iterable"
                        })
                    {
                        analysis.callables.insert(
                            origin.declaration,
                            RetainedCallableInfo {
                                returns: vec![RetainedSource {
                                    source: BorrowSource::Receiver,
                                    inherited: false,
                                }],
                                ..RetainedCallableInfo::default()
                            },
                        );
                    }
                }
            }
        }
        for conformance in &contracts.conformances {
            if conformance.status != crate::semantics::contracts::ConformanceStatus::Checked {
                continue;
            }
            for implementation in &conformance.implementations {
                let Some(declaration) = implementation.implementation else {
                    continue;
                };
                for origin in &implementation.requirement_origins {
                    let implementations = analysis
                        .implementations
                        .entry(origin.declaration)
                        .or_default();
                    if !implementations.contains(&declaration) {
                        implementations.push(declaration);
                    }
                }
            }
        }
        analysis
    }

    pub fn enter_function(&mut self, function: &ast::FunctionDecl) -> Option<FunctionContext> {
        if function.name == "__construct" {
            for (index, _) in function
                .params
                .iter()
                .enumerate()
                .filter(|(_, parameter)| parameter.borrow_span.is_some())
            {
                let source = RetainedSource {
                    source: BorrowSource::Parameter(index),
                    inherited: false,
                };
                let info = self.callables.entry(function.span).or_default();
                if !info.constructs.contains(&source) {
                    info.constructs.push(source);
                }
            }
        }
        self.current.replace(FunctionContext {
            declaration: function.span,
            parameters: function
                .params
                .iter()
                .map(|parameter| (parameter.name.clone(), parameter.take))
                .collect(),
            constructor: function.name == "__construct",
        })
    }

    fn carries_source(&self, ty: &ResolvedType) -> bool {
        match ty {
            ResolvedType::Nullable(inner) => self.carries_source(inner),
            ResolvedType::Class(class) => self.source_carriers.contains(&class.name),
            ResolvedType::Interface(interface) => {
                self.iterator_interfaces.contains(&interface.name)
            }
            _ => false,
        }
    }
}

impl Checker<'_> {
    pub(super) fn infer_retained_callables(&mut self, program: &ast::Program) {
        if self.retained.source_carriers.is_empty() && self.retained.iterator_interfaces.is_empty()
        {
            return;
        }
        for item in &program.items {
            let Item::Class(class) = item else { continue };
            for member in &class.members {
                let ClassMember::Method(method) = member else {
                    continue;
                };
                if !method.is_override {
                    continue;
                }
                for ancestor in self
                    .retained
                    .ancestors
                    .get(&class.name)
                    .into_iter()
                    .flatten()
                {
                    if let Some(base) = self
                        .methods
                        .get(&(ancestor.clone(), method.name.clone()))
                        .and_then(|signature| signature.declaration)
                    {
                        let implementations =
                            self.retained.implementations.entry(base).or_default();
                        if !implementations.contains(&method.span) {
                            implementations.push(method.span);
                        }
                    }
                }
            }
        }
        // Reuse the ownership pass's control-flow joins and canonical bindings.
        // Summaries grow monotonically over a finite set of parameter roots.
        loop {
            let before = self.retained.callables.clone();
            for item in &program.items {
                match item {
                    Item::Function(function) => self.check_function(function, None),
                    Item::Class(class) => {
                        for member in &class.members {
                            if let ClassMember::Method(method) = member {
                                self.check_function(method, Some(&class.name));
                            }
                        }
                    }
                    _ => {}
                }
            }
            // A call through a checked requirement can select any of its
            // implementations. Keep their source dependencies at that boundary.
            for (requirement, implementations) in &self.retained.implementations {
                let summaries = implementations
                    .iter()
                    .filter_map(|implementation| {
                        self.retained.callables.get(implementation).cloned()
                    })
                    .collect::<Vec<_>>();
                let summary = self.retained.callables.entry(*requirement).or_default();
                for candidate in summaries {
                    for source in candidate.returns {
                        if !summary.returns.contains(&source) {
                            summary.returns.push(source);
                        }
                    }
                    for source in candidate.requires_independent {
                        if !summary.requires_independent.contains(&source) {
                            summary.requires_independent.push(source);
                        }
                    }
                }
            }
            self.diagnostics.clear();
            self.analyzed_closures.clear();
            self.prepared_closure_evaluations.clear();
            self.closure_values.clear();
            self.closure_ownership.clear();
            self.retained.expression_values.clear();
            if self.retained.callables == before {
                break;
            }
        }
    }

    pub(super) fn parameter_retained_value(
        &self,
        id: Option<BindingId>,
    ) -> Option<RetainedValueState> {
        let id = id?;
        let declaration = self.binding_resolution.declarations_by_id.get(&id)?;
        let ty = declaration.source_type.as_ref()?;
        let carries_source = self.retained.carries_source(ty)
            || matches!(ty, ResolvedType::TypeParameter(name)
            if self.current_type_params.iter().any(|parameter| parameter.name == *name && parameter.constraints.iter()
                .any(|constraint| self.retained.iterator_interfaces.contains(&constraint.name))));
        carries_source.then(|| {
            iterator_value(vec![RetainedLoan {
                root: BorrowRoot::Binding(id),
                root_key: format!("retained-parameter:{}", id.0),
                access: BorrowAccess::Readonly,
                capture_span: declaration.span.unwrap_or_default(),
                source_depth: 0,
                inherited: true,
            }])
        })
    }

    pub(super) fn retained_call<'a>(
        &self,
        expr: &'a Expr,
        scopes: &Scopes,
    ) -> Option<(Signature, Option<&'a Expr>, &'a [Argument])> {
        let (receiver, args) = match ungroup_expr(expr) {
            Expr::New {
                class_type, args, ..
            } => {
                return Some((self.constructors.get(&class_type.name)?.clone(), None, args));
            }
            Expr::FunctionCall { args, .. } | Expr::StaticCall { args, .. } => (None, args),
            Expr::MethodCall { object, args, .. } => (Some(object.as_ref()), args),
            _ => return None,
        };
        let signature = match self.call_targets.get(&expr.span()) {
            Some(crate::semantics::CallableTarget::Function { name }) => self.signatures.get(name),
            Some(crate::semantics::CallableTarget::Method {
                class_type,
                method_name,
                ..
            }) => self.retained_method(&class_type.name, method_name),
            Some(
                crate::semantics::CallableTarget::InterfaceMethod { requirement, .. }
                | crate::semantics::CallableTarget::ConstrainedMethod { requirement, .. },
            ) => self.retained.requirements.get(requirement),
            _ => match ungroup_expr(expr) {
                Expr::FunctionCall { name, .. } => self.signatures.get(name),
                Expr::MethodCall { object, method, .. } => self
                    .expr_class(object, scopes)
                    .and_then(|class| self.retained_method(&class, method)),
                Expr::StaticCall {
                    qualifier, method, ..
                } => self
                    .qualifier_class(qualifier)
                    .and_then(|class| self.methods.get(&(class, method.clone()))),
                _ => None,
            },
        }?;
        Some((signature.clone(), receiver, args))
    }

    fn retained_method(&self, class: &str, method: &str) -> Option<&Signature> {
        std::iter::once(class)
            .chain(
                self.retained
                    .ancestors
                    .get(class)
                    .into_iter()
                    .flatten()
                    .map(String::as_str),
            )
            .find_map(|class| self.methods.get(&(class.to_owned(), method.to_owned())))
    }

    fn constructor_sources(&self, signature: &Signature) -> Vec<RetainedSource> {
        let mut sources = signature
            .declaration
            .and_then(|declaration| self.retained.callables.get(&declaration))
            .map(|info| info.constructs.clone())
            .unwrap_or_default();
        for (index, _) in signature
            .params
            .iter()
            .enumerate()
            .filter(|(_, parameter)| parameter.borrow)
        {
            let source = RetainedSource {
                source: BorrowSource::Parameter(index),
                inherited: false,
            };
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        sources
    }

    pub(super) fn retain_parent_constructor_sources(
        &mut self,
        signature: &Signature,
        args: &[Argument],
        scopes: &Scopes,
    ) {
        if !self
            .retained
            .current
            .as_ref()
            .is_some_and(|current| current.constructor)
        {
            return;
        }
        let mut sources = Vec::new();
        for source in self.constructor_sources(signature) {
            let Some(input) = self.call_borrow_source_expr(
                ReturnBorrow {
                    source: source.source,
                    writable: false,
                },
                None,
                signature,
                args,
            ) else {
                continue;
            };
            let loan = self.retained_source_loan(input, scopes);
            if let Some(source) = self.parameter_source(&loan) {
                sources.push(source);
            } else {
                self.diagnostics.push(iterator_escape(
                    input.span(),
                    loan.capture_span,
                    "constructed iterator would outlive its constructor-local source",
                ));
            }
        }
        let current = self.retained.current.as_ref().expect("constructor context");
        let info = self
            .retained
            .callables
            .entry(current.declaration)
            .or_default();
        for source in sources {
            if !info.constructs.contains(&source) {
                info.constructs.push(source);
            }
        }
    }

    pub(super) fn iterator_value_from_expr(
        &self,
        expr: &Expr,
        scopes: &Scopes,
    ) -> Option<RetainedValueState> {
        if let Some(value) = self.retained.expression_values.get(&expr.span()) {
            return Some(value.clone());
        }
        if matches!(ungroup_expr(expr), Expr::This { .. }) {
            return self
                .receiver_class
                .as_ref()
                .filter(|class| self.retained.source_carriers.contains(*class))
                .map(|_| {
                    iterator_value(vec![RetainedLoan {
                        root: BorrowRoot::Receiver,
                        root_key: "retained-receiver".into(),
                        access: BorrowAccess::Readonly,
                        capture_span: expr.span(),
                        source_depth: 0,
                        inherited: true,
                    }])
                });
        }
        if let Expr::Binary {
            left,
            op: BinaryOp::Coalesce,
            right,
            ..
        } = ungroup_expr(expr)
        {
            return join_retained_value(
                self.retained_value_from_expr(left, scopes).as_ref(),
                self.retained_value_from_expr(right, scopes).as_ref(),
            );
        }
        let (signature, receiver, args) = self.retained_call(expr, scopes)?;
        let sources = if matches!(ungroup_expr(expr), Expr::New { .. }) {
            self.constructor_sources(&signature)
        } else {
            signature
                .declaration
                .and_then(|declaration| self.retained.callables.get(&declaration))
                .map(|info| info.returns.clone())
                .unwrap_or_default()
        };
        if sources.is_empty() {
            return None;
        }
        let mut loans = Vec::new();
        for source in sources {
            let input = self.call_borrow_source_expr(
                ReturnBorrow {
                    source: source.source,
                    writable: false,
                },
                receiver,
                &signature,
                args,
            )?;
            if source.inherited {
                if let Some(value) = self.retained_value_from_expr(input, scopes) {
                    append_loans(&mut loans, &value.leases);
                }
            } else {
                append_loans(&mut loans, &[self.retained_source_loan(input, scopes)]);
            }
        }
        Some(iterator_value(loans))
    }

    fn retained_source_loan(&self, expr: &Expr, scopes: &Scopes) -> RetainedLoan {
        let key = self.borrow_root_key(expr, scopes);
        let binding = key.as_ref().and_then(|key| {
            scopes
                .0
                .iter()
                .rev()
                .flat_map(|scope| scope.iter())
                .find_map(|(name, binding)| {
                    let binding = binding.as_ref()?;
                    (binding_identity_key(binding.id, name) == *key).then_some(binding)
                })
        });
        let (root, source_depth) = if key.as_deref() == Some("$this") {
            (BorrowRoot::Receiver, 0)
        } else if let Some(binding) = binding {
            (
                binding
                    .canonical_id
                    .map(BorrowRoot::Binding)
                    .unwrap_or(BorrowRoot::Temporary),
                binding.scope_depth,
            )
        } else {
            (BorrowRoot::Temporary, scopes.lexical_depth() + 1)
        };
        RetainedLoan {
            root,
            root_key: key.unwrap_or_else(|| format!("temporary:{}", expr.span().start)),
            access: BorrowAccess::Readonly,
            capture_span: expr.span(),
            source_depth,
            inherited: false,
        }
    }

    fn parameter_source(&self, loan: &RetainedLoan) -> Option<RetainedSource> {
        let source = match loan.root {
            BorrowRoot::Receiver => BorrowSource::Receiver,
            BorrowRoot::Binding(id) => {
                let declaration = self.binding_resolution.declarations_by_id.get(&id)?;
                if !matches!(
                    declaration.kind,
                    BindingKind::FunctionParameter | BindingKind::MethodParameter
                ) {
                    return None;
                }
                let current = self.retained.current.as_ref()?;
                let (index, (_, take)) = current
                    .parameters
                    .iter()
                    .enumerate()
                    .find(|(_, (name, _))| name == &declaration.name)?;
                if *take && !loan.inherited {
                    return None;
                }
                BorrowSource::Parameter(index)
            }
            BorrowRoot::Temporary | BorrowRoot::EnclosingEnvironment(_) => return None,
        };
        Some(RetainedSource {
            source,
            inherited: loan.inherited,
        })
    }

    pub(super) fn validate_returned_iterator(
        &mut self,
        value: &RetainedValueState,
        span: Span,
    ) -> bool {
        let mut sources = Vec::new();
        for loan in &value.leases {
            let Some(source) = self.parameter_source(loan) else {
                self.diagnostics.push(iterator_escape(
                    span,
                    loan.capture_span,
                    "returned iterator would outlive its source",
                ));
                return false;
            };
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        if let Some(current) = &self.retained.current {
            let info = self
                .retained
                .callables
                .entry(current.declaration)
                .or_default();
            for source in sources {
                if !info.returns.contains(&source) {
                    info.returns.push(source);
                }
            }
        }
        true
    }

    pub(super) fn validate_iterator_storage(
        &mut self,
        value: &RetainedValueState,
        span: Span,
        destination: &str,
    ) -> bool {
        for loan in &value.leases {
            if loan.inherited {
                if let Some(source) = self.parameter_source(loan) {
                    if let Some(current) = &self.retained.current {
                        let info = self
                            .retained
                            .callables
                            .entry(current.declaration)
                            .or_default();
                        if !info.requires_independent.contains(&source.source) {
                            info.requires_independent.push(source.source);
                        }
                        continue;
                    }
                }
            }
            self.diagnostics.push(iterator_escape(
                span,
                loan.capture_span,
                &format!("borrow-bound iterator cannot be retained in {destination}"),
            ));
            return false;
        }
        true
    }

    pub(super) fn validate_retained_call_inputs(
        &mut self,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        scopes: &Scopes,
    ) {
        let required = signature
            .declaration
            .and_then(|declaration| self.retained.callables.get(&declaration))
            .map(|info| info.requires_independent.clone())
            .unwrap_or_default();
        for source in required {
            let Some(input) = self.call_borrow_source_expr(
                ReturnBorrow {
                    source,
                    writable: false,
                },
                receiver,
                signature,
                args,
            ) else {
                continue;
            };
            if let Some(value) = self.retained_value_from_expr(input, scopes) {
                self.validate_iterator_storage(
                    &value,
                    input.span(),
                    "storage retained by the callee",
                );
            }
        }
    }
}

fn iterator_value(leases: Vec<RetainedLoan>) -> RetainedValueState {
    let mut roots = leases
        .iter()
        .map(|loan| loan.root.clone())
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    RetainedValueState {
        kind: RetainedValueKind::Iterator,
        closure_id: None,
        provenance: if roots.is_empty() {
            ValueProvenance::Owned
        } else {
            ValueProvenance::BorrowBound(roots)
        },
        leases,
        nonescaping_parameter: false,
        take_parameter_insertion: None,
    }
}

fn append_loans(destination: &mut Vec<RetainedLoan>, incoming: &[RetainedLoan]) {
    for loan in incoming {
        if !destination.iter().any(|found| {
            found.root == loan.root
                && found.inherited == loan.inherited
                && found.access == loan.access
        }) {
            destination.push(loan.clone());
        }
    }
}

pub(super) fn iterator_escape(span: Span, source: Span, message: &str) -> Diagnostic {
    Diagnostic::new("E0762", message, span)
        .with_title("Iterator Cannot Outlive Borrowed Source")
        .with_related(source, "Source Loan Starts Here")
        .with_help("keep the iterator within its source's lifetime, or construct an iterator that owns its source")
}
