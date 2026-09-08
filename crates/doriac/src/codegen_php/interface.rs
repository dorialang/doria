//! PHP nominal markers encode checked conformance without host signature inference.

use super::*;
use crate::semantics::contracts::ConformanceStatus;
use crate::types::InterfaceType;

pub(super) fn declaration_name(name: &str) -> String {
    match name {
        "Error" => "__DoriaErrorValue".to_string(),
        "Displayable" => "__DoriaDisplayable".to_string(),
        _ => format!("__DoriaContract_{}", hex_bytes(name.as_bytes())),
    }
}

pub(super) fn specialization_name(interface: &InterfaceType<ResolvedType>) -> String {
    format!(
        "__DoriaInterface_{}",
        hex_bytes(resolved_type_identity(&ResolvedType::Interface(interface.clone())).as_bytes())
    )
}

pub(super) fn nominal_type_name(ty: &ResolvedType, scopes: &PhpNameScopes) -> Option<String> {
    match ty {
        ResolvedType::Class(class) => Some(
            scopes
                .specialization
                .class_symbols
                .get(class)
                .cloned()
                .unwrap_or_else(|| php_symbol_name(&class.name)),
        ),
        ResolvedType::Interface(interface) => Some(specialization_name(interface)),
        ResolvedType::Error => Some(declaration_name("Error")),
        _ => None,
    }
}

pub(super) fn emit_declarations(semantic: &SemanticInfo, output: &mut String) {
    for interface in &semantic.contracts.interfaces {
        if !interface.valid || matches!(interface.name.as_str(), "Error" | "Displayable") {
            continue;
        }
        let parents = interface
            .parents
            .iter()
            .map(|parent| declaration_name(&parent.specialization.name))
            .collect::<Vec<_>>();
        let extends = if parents.is_empty() {
            String::new()
        } else {
            format!(" extends {}", parents.join(", "))
        };
        output.push_str(&format!(
            "interface {}{extends} {{}}\n",
            declaration_name(&interface.name)
        ));
    }
    for interface in &semantic.contracts.interface_specializations {
        if !interface.valid {
            continue;
        }
        output.push_str(&format!(
            "interface {} extends {} {{}}\n",
            specialization_name(&interface.specialization),
            declaration_name(&interface.specialization.name)
        ));
    }
    output.push('\n');
}

pub(super) fn class_views(
    semantic: &SemanticInfo,
    ty: &crate::types::ClassType<ResolvedType>,
) -> Vec<String> {
    semantic
        .contracts
        .conformances
        .iter()
        .filter(|fact| {
            fact.status == ConformanceStatus::Checked
                && matches!(&fact.implementing_type, ResolvedType::Class(class) if class == ty)
        })
        .map(|fact| specialization_name(&fact.interface))
        .collect()
}
