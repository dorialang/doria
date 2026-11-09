//! Pending request loans cannot enter storage that outlives a phase call.

use super::*;
use crate::compiler_known_stream::{contains_call_scoped_request, is_call_scoped_request};

impl Checker<'_> {
    pub(super) fn check_call_scoped_requests(
        &mut self,
        closure_ownership: &HashMap<ClosureId, crate::ownership::ClosureOwnershipInfo>,
        return_borrows: &HashMap<Span, ReturnBorrow>,
    ) {
        let mut violations = HashSet::new();
        for (span, signature) in &self.function_signatures {
            let result = self.types.resolved(signature.return_ty);
            if contains_call_scoped_request(&result)
                && !(is_call_scoped_request(&result) && return_borrows.contains_key(span))
            {
                violations.insert((*span, "be returned"));
            }
            for parameter in &signature.params {
                let ty = self.types.resolved(parameter.ty);
                if contains_call_scoped_request(&ty)
                    && (parameter.take || !is_call_scoped_request(&ty))
                {
                    violations.insert((*span, "be accepted with ownership or inside a container"));
                }
            }
        }
        for class in self.classes.values() {
            for property in class.properties.values() {
                if contains_call_scoped_request(&self.types.resolved(property.ty)) {
                    violations.insert((property.declaration_span, "be stored in a property"));
                }
            }
            for property in class.static_properties.values() {
                if contains_call_scoped_request(&self.types.resolved(property.ty)) {
                    violations.insert((class.declaration, "be stored in a static property"));
                }
            }
        }
        for interface in &self.contracts.interfaces {
            for requirement in &interface.requirements {
                let result = &requirement.signature.return_type;
                let returns_request = contains_call_scoped_request(result)
                    && !(is_call_scoped_request(result) && requirement.return_borrow.is_some());
                let owns_request = requirement.signature.parameters.iter().any(|parameter| {
                    contains_call_scoped_request(&parameter.r#type)
                        && (parameter.take || !is_call_scoped_request(&parameter.r#type))
                });
                for origin in &requirement.origins {
                    if returns_request {
                        violations.insert((
                            origin.declaration,
                            "be returned by an interface requirement",
                        ));
                    }
                    if owns_request {
                        violations
                            .insert((origin.declaration, "be owned by an interface requirement"));
                    }
                }
            }
        }
        for enumeration in self.enums.values() {
            for field in enumeration.cases.iter().flat_map(|case| &case.payload) {
                if contains_call_scoped_request(&self.types.resolved(field.ty)) {
                    violations.insert((field.span, "be stored in an enum payload"));
                }
            }
        }
        for (span, ty) in &self.expression_types {
            if !is_call_scoped_request(ty) && contains_call_scoped_request(ty) {
                violations.insert((*span, "be stored in a container or owning callable"));
            }
        }
        for (span, plan) in &self.mixed_box_plans {
            if contains_call_scoped_request(&plan.source_type) {
                violations.insert((*span, "be erased into mixed"));
            }
        }
        for binding in self.binding_resolution.declarations_by_id.values() {
            if let (Some(span), Some(ty)) = (binding.span, &binding.source_type) {
                if !is_call_scoped_request(ty) && contains_call_scoped_request(ty) {
                    violations.insert((span, "be declared inside a container or owning callable"));
                }
                if is_call_scoped_request(ty)
                    && binding.ownership == crate::symbols::BindingOwnership::Owned
                {
                    violations.insert((span, "be independently owned"));
                }
            }
        }
        for (id, closure) in &self.closures {
            let escapes = closure_ownership.get(id).is_some_and(|ownership| {
                ownership.escape != crate::ownership::ClosureEscapeClassification::Local
            });
            for capture in &closure.captures {
                if contains_call_scoped_request(&capture.source_type)
                    && (escapes || capture.mode == ClosureCaptureMode::Take)
                {
                    violations.insert((
                        capture.declaration_span,
                        "be retained by an escaping or owning closure",
                    ));
                }
            }
        }
        let mut violations = violations.into_iter().collect::<Vec<_>>();
        violations.sort_by_key(|(span, context)| (*span, *context));
        for (span, context) in violations {
            self.report_call_scoped_request(span, context);
        }
    }

    pub(super) fn report_call_scoped_request(&mut self, span: Span, context: &str) {
        self.diagnostics.push(
            Diagnostic::new("E0772", format!("a pending request view cannot {context}"), span)
                .with_title("Pending Request Cannot Escape Its Phase")
                .with_explanation("the transfer group owns and reuses the request slot; preparation and completion only borrow it for their call")
                .with_help("pass the view through a borrowed parameter and keep its use inside the preparation or completion call"),
        );
    }
}
