use super::*;

impl Checker<'_> {
    pub(super) fn use_property_setter(
        &mut self,
        target: &Expr,
        value: &Expr,
        scopes: &mut Scopes,
    ) -> bool {
        let target = ungroup_expr(target);
        let Expr::PropertyAccess { object, .. } = target else {
            return false;
        };
        let Some(call) = self
            .property_accessor_calls
            .get(&target.span())
            .and_then(|calls| calls.setter.as_ref())
            .cloned()
        else {
            return false;
        };
        let signature = self.accessor_signature(&call);
        let argument = Argument {
            name: None,
            value: value.clone(),
            span: value.span(),
        };
        self.use_call_args(
            target.span(),
            Some(object),
            &[argument],
            &signature,
            CallExecution::Always,
            scopes,
        );
        self.record_exceptional_effects_at(target.span(), &call.checked_effects, scopes);
        true
    }

    pub(super) fn use_property_update(
        &mut self,
        target: &Expr,
        value: Option<&Expr>,
        scopes: &mut Scopes,
    ) -> bool {
        let target = ungroup_expr(target);
        let Expr::PropertyAccess { object, .. } = target else {
            return false;
        };
        let Some(calls) = self.property_accessor_calls.get(&target.span()).cloned() else {
            return false;
        };
        let (Some(getter), Some(setter)) = (calls.getter, calls.setter) else {
            return false;
        };
        let borrow_depth = self.active_borrows.len();
        self.use_expr(object, scopes, UseMode::Read);
        self.activate_place_input_borrows(object, scopes);
        // Read-modify-write holds the receiver once through the getter, RHS,
        // and setter, just as an ordinary property update holds its owner.
        self.activate_borrow(object, UseMode::Write, scopes);
        self.record_exceptional_effects_at(target.span(), &getter.checked_effects, scopes);
        if let Some(value) = value {
            self.use_expr(value, scopes, UseMode::Read);
        }
        self.record_exceptional_effects_at(target.span(), &setter.checked_effects, scopes);
        self.active_borrows.truncate(borrow_depth);
        true
    }

    pub(super) fn getter_signature(&self, expr: &Expr) -> Option<Signature> {
        let call = self
            .property_accessor_calls
            .get(&ungroup_expr(expr).span())?
            .getter
            .as_ref()?;
        Some(self.accessor_signature(call))
    }

    fn accessor_signature(
        &self,
        call: &crate::semantics::PropertyAccessorCallSemanticInfo,
    ) -> Signature {
        let return_borrow = self
            .return_borrows
            .get(&call.declaration)
            .copied()
            .unwrap_or(call.return_borrow);
        Signature {
            declaration: Some(call.declaration),
            params: call
                .parameter
                .iter()
                .map(|parameter| Parameter {
                    name: parameter.name.clone(),
                    move_type: resolved_type_is_move_type(&parameter.r#type, &self.move_enum_names),
                    class_type: resolved_type_class(&parameter.r#type).is_some(),
                    generic: resolved_type_requires_conservative_move(&parameter.r#type),
                    take: parameter.take,
                    writable: parameter.writable,
                    borrow: parameter.borrow,
                })
                .collect(),
            returns: resolved_type_class(&call.return_type).map(str::to_owned),
            returns_collection: resolved_collection_info(&call.return_type, &self.move_enum_names),
            returns_move_type: resolved_type_is_move_type(&call.return_type, &self.move_enum_names)
                && return_borrow.is_none_or(|borrow| borrow.kind == ReturnBorrowKind::Retained),
            return_borrow,
            receiver: Some(if call.receiver_mode.is_writable() {
                UseMode::Write
            } else {
                UseMode::Read
            }),
        }
    }
}
