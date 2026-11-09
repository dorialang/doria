//! Case construction shares generic inference without creating callable targets.

use super::*;

impl Checker<'_> {
    pub(super) fn admit_constant_enum_applications(
        &mut self,
        value: &crate::const_eval::ConstValue,
        span: Span,
        declaring_class: Option<&str>,
    ) -> bool {
        use crate::const_eval::ConstValue;
        let (value, fields) = match value {
            ConstValue::Enum(value) => (value, &[][..]),
            ConstValue::PayloadEnum(value) => (&value.value, value.fields.as_slice()),
            _ => return true,
        };
        let ty = &value.enum_type;
        if !ty.arguments.is_empty() {
            let Some(definition) = self
                .enums
                .get(&ty.name)
                .filter(|definition| {
                    definition.source_type.id == ty.id
                        && value.case.declaration == ty.id
                        && definition.cases.get(value.case.index).is_some()
                        && definition.type_params.len() == ty.arguments.len()
                })
                .cloned()
            else {
                self.diagnostics.push(Diagnostic::new(
                    "E0485",
                    "folded enum case does not match its checked declaration",
                    span,
                ));
                return false;
            };
            let application = ty.map_arguments(|arg| self.types.intern_resolved(arg));
            if !self.check_nominal_type_constraints(
                &ty.name,
                &definition.type_params,
                &application.arguments,
                span,
            ) {
                return false;
            }
            self.record_nominal_instantiation(
                instantiation::NominalInstance::Enum(application),
                declaring_class,
            );
        }
        let mut valid = true;
        for field in fields {
            valid &= self.admit_constant_enum_applications(field, span, declaring_class);
        }
        valid
    }

    pub(super) fn begin_enum_construction(
        &mut self,
        definition: &EnumDefinition,
        case: &str,
        span: Span,
        bindings: HashMap<String, TypeId>,
    ) {
        if definition.type_params.is_empty() {
            return;
        }
        let return_ty = self
            .types
            .intern(TypeKind::Enum(definition.source_type.clone()));
        self.pending_enum_constructions.insert(
            span,
            GenericInference {
                callee: format!("enum case `{}::{case}`", definition.name),
                type_params: definition.type_params.clone(),
                bindings,
                return_ty,
            },
        );
        if let Some(expected) = self.contextual_expression_types.get(&span).copied() {
            self.complete_enum_construction_from_expected(span, expected);
        } else {
            self.publish_enum_construction(span);
        }
    }

    pub(super) fn complete_enum_construction_from_expected(
        &mut self,
        span: Span,
        expected: TypeId,
    ) -> bool {
        let Some(mut inference) = self.pending_enum_constructions.get(&span).cloned() else {
            return false;
        };
        let expected = match self.types.kind(expected) {
            TypeKind::Nullable(inner) => *inner,
            _ => expected,
        };
        self.infer_type_parameter_bindings(
            &inference.callee,
            inference.return_ty,
            expected,
            span,
            &mut inference.bindings,
            true,
        );
        self.pending_enum_constructions.insert(span, inference);
        self.publish_enum_construction(span);
        true
    }

    fn publish_enum_construction(&mut self, span: Span) {
        let Some(inference) = self.pending_enum_constructions.get(&span).cloned() else {
            return;
        };
        if !inference
            .type_params
            .iter()
            .all(|parameter| inference.bindings.contains_key(&parameter.name))
        {
            return;
        }
        self.check_generic_type_constraints(
            &inference.callee,
            &inference.type_params,
            &inference.bindings,
            span,
        );
        let result = self.substitute_type(inference.return_ty, &inference.bindings);
        self.expression_types
            .insert(span, self.types.resolved(result));
    }

    pub(super) fn enum_construction_type(&mut self, span: Span, fallback: TypeId) -> TypeId {
        let Some(inference) = self.pending_enum_constructions.get(&span).cloned() else {
            return fallback;
        };
        if inference
            .type_params
            .iter()
            .all(|parameter| inference.bindings.contains_key(&parameter.name))
        {
            self.substitute_type(inference.return_ty, &inference.bindings)
        } else {
            self.types.unknown()
        }
    }

    pub(super) fn enum_case_payload(
        &mut self,
        source_type: &EnumType<TypeId>,
        index: usize,
    ) -> Option<Vec<EnumPayloadDefinition>> {
        let definition = self.enums.get(&source_type.name)?;
        if definition.source_type.id != source_type.id
            || definition.type_params.len() != source_type.arguments.len()
        {
            return None;
        }
        let substitutions = definition
            .type_params
            .iter()
            .zip(&source_type.arguments)
            .map(|(parameter, argument)| (parameter.name.clone(), *argument))
            .collect::<HashMap<_, _>>();
        let mut payload = definition.cases.get(index)?.payload.clone();
        for field in &mut payload {
            field.ty = self.substitute_type_id(field.ty, &substitutions);
        }
        Some(payload)
    }
}
