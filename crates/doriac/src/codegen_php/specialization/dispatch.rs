//! Typed caller views of checked virtual families and nominal witnesses.

use super::*;
use crate::semantics::{CallableInstanceKey, CallableSignatureSemanticInfo, VirtualMethodFamily};

#[derive(Debug, Clone)]
enum Contract {
    Callable(mir::FunctionId),
    Requirement(RequirementKey),
}

#[derive(Debug, Clone)]
pub(super) struct Bridge {
    class: ClassId,
    contract: Contract,
    implementation: mir::FunctionId,
    pub(super) symbol: String,
    exact: bool,
}

impl Plan {
    fn callable_signature(&self, id: mir::FunctionId) -> CallableSignatureSemanticInfo {
        let callable = self.callable(id);
        let mut signature = self.semantic.callable_signatures[&callable.declaration].clone();
        specialize_signature(&mut signature, &callable.substitutions);
        signature
    }

    fn requirement_signature(&self, key: &RequirementKey) -> CallableSignatureSemanticInfo {
        let requirement = self
            .semantic
            .contracts
            .interface_specializations
            .iter()
            .find(|facts| facts.valid && facts.specialization == key.0)
            .and_then(|facts| {
                facts.requirements.iter().find(|requirement| {
                    requirement
                        .origins
                        .iter()
                        .any(|origin| origin.declaration == key.1)
                })
            })
            .expect("checked requirement has a canonical signature");
        let substitutions = requirement
            .generic_parameters
            .iter()
            .zip(&key.2)
            .map(|(parameter, GenericArgument::Type(ty))| (parameter.name.clone(), ty.clone()))
            .collect();
        let mut signature = requirement.signature.clone();
        specialize_signature(&mut signature, &substitutions);
        signature
    }

    pub(super) fn prepare_dispatch(
        &mut self,
        names: &mut HashSet<String>,
    ) -> Result<(), BackendError> {
        let mut families: HashMap<VirtualMethodFamily, Vec<mir::FunctionId>> = HashMap::new();
        let mut callable_families = HashMap::new();
        for callable in &self.callables {
            if !self
                .semantic
                .method_hierarchy
                .get(&callable.declaration)
                .is_some_and(|method| method.virtual_root.is_some())
            {
                continue;
            }
            let instance = CallableInstanceKey::new(callable.declaration, &callable.substitutions);
            let family = self
                .semantic
                .virtual_dispatch
                .family(&instance, callable.declaration)
                .map_err(BackendError::from_diagnostics)?
                .clone();
            families
                .entry(family.clone())
                .or_default()
                .push(callable.id);
            callable_families.insert(callable.id, family);
        }
        let adapted = families
            .iter()
            .filter_map(|(family, members)| {
                let returned = self.callable_signature(members[0]).return_type;
                members
                    .iter()
                    .any(|id| self.callable_signature(*id).return_type != returned)
                    .then_some(family.clone())
            })
            .collect::<HashSet<_>>();
        // PHP inheritance owns no result conversion. Each logical caller contract
        // gets a stable signature, and exact bodies keep distinct implementation IDs.
        for callable in &mut self.callables {
            if callable_families
                .get(&callable.id)
                .is_some_and(|family| adapted.contains(family))
            {
                callable.symbol = allocate_symbol("__doriaMethodView", names);
                self.exact_symbols
                    .entry(callable.id)
                    .or_insert_with(|| allocate_symbol("__doriaExactMethod", names));
            }
        }
        for class in &self.semantic.classes {
            let receiver = ClassType::new(class.declaration_name.clone(), class.arguments.clone());
            let site = self
                .semantic
                .class_hierarchy
                .values()
                .find(|declaration| declaration.name == class.declaration_name)
                .expect("checked class has a source declaration")
                .declaration;
            let slots = self
                .semantic
                .virtual_dispatch
                .slots(&receiver, site)
                .map_err(BackendError::from_diagnostics)?;
            for slot in slots.iter().filter(|slot| adapted.contains(&slot.family)) {
                let implementation = self
                    .callables
                    .iter()
                    .find(|callable| {
                        CallableInstanceKey::new(callable.declaration, &callable.substitutions)
                            == slot.implementation
                    })
                    .expect("checked virtual slot has a materialized implementation");
                if implementation.class != Some(class.id) {
                    continue;
                }
                for id in &families[&slot.family] {
                    let contract = self.callable(*id);
                    if *id == implementation.id
                        || !contract.class.is_some_and(|owner| {
                            class
                                .ancestors
                                .iter()
                                .any(|ancestor| self.classes.get(ancestor) == Some(&owner))
                        })
                    {
                        continue;
                    }
                    self.dispatch_bridges.push(Bridge {
                        class: class.id,
                        contract: Contract::Callable(*id),
                        implementation: implementation.id,
                        symbol: contract.symbol.clone(),
                        exact: true,
                    });
                }
            }
        }
        let mut keys = self.requirements.keys().cloned().collect::<Vec<_>>();
        keys.sort_by_cached_key(|key| {
            (
                resolved_type_identity(&ResolvedType::Interface(key.0.clone())),
                key.1,
                key.2
                    .iter()
                    .map(|GenericArgument::Type(ty)| resolved_type_identity(ty))
                    .collect::<Vec<_>>(),
            )
        });
        for key in &keys {
            if self.requirement_symbols.contains_key(key) {
                continue;
            }
            let requirement = self
                .semantic
                .contracts
                .interface_specializations
                .iter()
                .find(|facts| facts.valid && facts.specialization == key.0)
                .and_then(|facts| {
                    facts.requirements.iter().find(|requirement| {
                        requirement
                            .origins
                            .iter()
                            .any(|origin| origin.declaration == key.1)
                    })
                })
                .expect("checked requirement has an origin");
            let origins = requirement
                .origins
                .iter()
                .map(|origin| origin.declaration)
                .collect::<HashSet<_>>();
            let aliases = keys
                .iter()
                .filter(|other| other.0 == key.0 && other.2 == key.2 && origins.contains(&other.1))
                .cloned()
                .collect::<Vec<_>>();
            let returned = self.requirement_signature(key).return_type;
            if !self.interface_edges.iter().any(|(id, origin)| {
                aliases.contains(origin)
                    && (self.callable_signature(*id).return_type != returned
                        || callable_families
                            .get(id)
                            .is_some_and(|family| adapted.contains(family)))
            }) {
                continue;
            }
            let symbol = if key.0.name == "Displayable" {
                // The compiler's canonical display protocol retains its public host entry.
                requirement.name.clone()
            } else {
                allocate_symbol("__doriaRequirementView", names)
            };
            for alias in aliases {
                self.requirement_symbols.insert(alias, symbol.clone());
            }
        }
        for conformance in &self.semantic.contracts.conformances {
            if conformance.status != crate::semantics::contracts::ConformanceStatus::Checked {
                continue;
            }
            let ResolvedType::Class(class) = &conformance.implementing_type else {
                continue;
            };
            let Some(class) = self.classes.get(class).copied() else {
                continue;
            };
            let witnesses = self
                .semantic
                .interface_witnesses
                .conformance(conformance)
                .map_err(BackendError::from_diagnostics)?;
            for key in keys.iter().filter(|key| key.0 == conformance.interface) {
                let Some(symbol) = self.requirement_symbols.get(key) else {
                    continue;
                };
                if self
                    .dispatch_bridges
                    .iter()
                    .any(|bridge| bridge.class == class && &bridge.symbol == symbol)
                {
                    continue;
                }
                let witness = witnesses
                    .iter()
                    .find(|witness| {
                        witness
                            .origins
                            .iter()
                            .any(|origin| origin.declaration == key.1)
                    })
                    .expect("checked nominal witness");
                let instance = witness
                    .instance(&key.2, key.1)
                    .map_err(BackendError::from_diagnostics)?;
                let implementation = self
                    .callables
                    .iter()
                    .find(|callable| {
                        CallableInstanceKey::new(callable.declaration, &callable.substitutions)
                            == instance
                    })
                    .expect("checked witness is materialized")
                    .id;
                self.dispatch_bridges.push(Bridge {
                    class,
                    contract: Contract::Requirement(key.clone()),
                    implementation,
                    symbol: symbol.clone(),
                    exact: false,
                });
            }
        }
        Ok(())
    }

    pub(in crate::codegen_php) fn emit_dispatch_bridges(
        &self,
        output: &mut String,
        indent: usize,
        scopes: &PhpNameScopes,
    ) {
        for bridge in self
            .dispatch_bridges
            .iter()
            .filter(|bridge| Some(bridge.class) == scopes.current_class_id)
        {
            let (signature, plan, fields, declaration, result_fields) = match &bridge.contract {
                Contract::Callable(id) => {
                    let callable = self.callable(*id);
                    (
                        self.callable_signature(*id),
                        &self.callable_plans[id],
                        &callable.field_parameters,
                        callable.declaration,
                        callable.field_result.clone(),
                    )
                }
                Contract::Requirement(key) => {
                    let signature = self.requirement_signature(key);
                    let plan = &self.requirements[key];
                    let fields = (!plan.returns_borrow
                        && value_result::uses_fields(&signature.return_type, &self.semantic.enums))
                    .then(|| signature.return_type.clone());
                    (
                        signature,
                        plan,
                        &self.requirement_fields[key],
                        key.1,
                        fields,
                    )
                }
            };
            let mut transports = vec![value_argument::Transport::Boxed];
            if result_fields.is_some() {
                transports.push(value_argument::Transport::FieldResult);
            }
            if fields.iter().any(Option::is_some) {
                transports.push(value_argument::Transport::FieldArguments);
                if result_fields.is_some() {
                    transports.push(value_argument::Transport::FieldArgumentsAndResult);
                }
            }
            for transport in transports {
                let mut scopes = scopes.expression_scope();
                let mut parameters = Vec::new();
                let mut arguments = Vec::new();
                if plan.caller_receiver_home() {
                    parameters.push("__DoriaCell $__doriaViewReceiver".into());
                    arguments.push("$__doriaViewReceiver".into());
                }
                for (index, (parameter, home)) in signature
                    .parameters
                    .iter()
                    .zip(&plan.parameters)
                    .enumerate()
                {
                    if let Some(ty) = fields[index].as_ref().filter(|_| transport.arguments()) {
                        let place = value_result::Place::parameter(ty, index, &mut scopes);
                        parameters.extend(place.declarations());
                        arguments.extend(place.names().map(|name| format!("${name}")));
                    } else {
                        let default = (!transport.arguments())
                            .then(|| {
                                self.semantic.parameter_defaults.get(&ParameterDefaultKey {
                                    function: declaration,
                                    parameter_index: index,
                                })
                            })
                            .flatten();
                        parameters.push(view_parameter(parameter, home.cell, default, &scopes));
                        arguments.push(format!("${}", parameter.name));
                    }
                }
                let implementation = self.callable(bridge.implementation);
                let actual = self.callable_signature(implementation.id).return_type;
                let source = if bridge.exact {
                    self.exact_symbol(implementation.id).unwrap()
                } else {
                    &implementation.symbol
                };
                let symbol = transport_symbol(self, &bridge.symbol, transport);
                let source_transport = value_argument::Transport::for_call(
                    transport.arguments(),
                    transport.result() && implementation.field_result.is_some(),
                );
                let source = transport_symbol(self, source, source_transport);
                let call = format!(
                    "{}{}({})",
                    if bridge.exact { "self::" } else { "$this->" },
                    source,
                    arguments.join(", ")
                );
                let borrowed =
                    plan.returns_borrow && borrowed_result_uses_cell(&signature.return_type);
                let returned = if transport.result() {
                    "void".into()
                } else if borrowed {
                    "__DoriaCell".into()
                } else {
                    php_resolved_type(&signature.return_type, &scopes)
                };
                writeln(
                    output,
                    indent,
                    &format!(
                        "public function {symbol}({}): {returned}",
                        parameters.join(", ")
                    ),
                );
                writeln(output, indent, "{");
                if transport.result() && implementation.field_result.is_some() {
                    writeln(output, indent + 1, &format!("{call};"));
                    writeln(output, indent + 1, "return;");
                } else if transport.result() {
                    scopes.value_return = Some(value_result::Return::new(
                        result_fields.as_ref().unwrap(),
                        &mut scopes,
                    ));
                    value_result::emit_forwarded_result(&call, output, indent + 1, &mut scopes);
                    value_result::emit_return_epilogue(output, indent + 1, &scopes);
                } else if signature.return_type == ResolvedType::Void {
                    writeln(output, indent + 1, &format!("{call};"));
                } else {
                    let value = interface::convert_typed_collection(
                        &actual,
                        &signature.return_type,
                        call,
                        !plan.returns_borrow,
                        borrowed,
                    );
                    writeln(output, indent + 1, &format!("return {value};"));
                }
                writeln(output, indent, "}");
                output.push('\n');
            }
        }
    }
}

fn specialize_signature(
    signature: &mut CallableSignatureSemanticInfo,
    substitutions: &HashMap<String, ResolvedType>,
) {
    signature.return_type = substitute_resolved_type(&signature.return_type, substitutions);
    for parameter in &mut signature.parameters {
        parameter.r#type = substitute_resolved_type(&parameter.r#type, substitutions);
    }
}

fn transport_symbol<'a>(
    plan: &'a Plan,
    symbol: &'a str,
    transport: value_argument::Transport,
) -> &'a str {
    if transport.arguments() {
        plan.argument_symbol(symbol, transport.result())
    } else if transport.result() {
        plan.result_symbol(symbol)
    } else {
        symbol
    }
}

fn view_parameter(
    parameter: &crate::semantics::CallableParameterSemanticInfo,
    cell: bool,
    default: Option<&ConstValue>,
    scopes: &PhpNameScopes,
) -> String {
    let payload = matches!(default, Some(ConstValue::PayloadEnum(_)));
    let value = php_resolved_type(&parameter.r#type, scopes);
    let mut types = vec![value.trim_start_matches('?').to_string()];
    if payload && !types.iter().any(|ty| ty == "array") {
        types.push("array".into());
    }
    if value.starts_with('?') {
        types.push("null".into());
    }
    let ty = if cell && default.is_none() {
        "__DoriaCell".into()
    } else if value == "mixed" {
        value
    } else {
        if cell {
            types.insert(0, "__DoriaCell".into());
        }
        types.join("|")
    };
    let mut result = format!("{ty} ${}", parameter.name);
    if let Some(default) = default {
        result.push_str(" = ");
        result.push_str(&if payload {
            "[]".into()
        } else {
            emit_const_value(default, &scopes.specialization)
        });
    }
    result
}
