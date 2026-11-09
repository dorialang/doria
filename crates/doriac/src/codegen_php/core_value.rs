//! Capture-free core adapters are named once and shared by all emission scopes.

use super::*;
use crate::compiler_known_contracts::CoreValueOperation;

#[derive(Debug, Default)]
pub(super) struct Helpers {
    indices: HashMap<(ResolvedType, CoreValueOperation), usize>,
    declarations: Vec<String>,
}

impl Helpers {
    pub(super) fn emit(&self, output: &mut String) {
        output.push_str("function __doria_core_identical($left, $right): bool\n{\n    return $left === $right;\n}\n\n");
        for declaration in &self.declarations {
            output.push_str(declaration);
        }
    }
}

pub(super) fn helper(
    ty: &ResolvedType,
    operation: CoreValueOperation,
    scopes: &PhpNameScopes,
) -> String {
    let ty = crate::types::substitute_resolved_type(ty, &scopes.substitutions);
    let ty = match ty {
        ResolvedType::Nullable(inner) => *inner,
        ty => ty,
    };
    let key = (ty, operation);
    let symbol = |index: usize| format!("__doria_core_callback_{index}");
    if let Some(index) = scopes.core_helpers.borrow().indices.get(&key) {
        return symbol(*index);
    }
    let binary = matches!(
        operation,
        CoreValueOperation::Equal | CoreValueOperation::Compare
    );
    let receiver = if binary { "$left" } else { "$value" };
    let arguments: &[&str] = if binary { &["$right"] } else { &[] };
    let call = emit_core_method(receiver, arguments, &key.0, operation, scopes);
    let body = match operation {
        CoreValueOperation::Equal => {
            format!("return $left === null || $right === null ? $left === $right : {call};")
        }
        CoreValueOperation::Hash => format!("return {call};"),
        CoreValueOperation::Clone => format!("return $value === null ? null : {call};"),
        CoreValueOperation::Compare => {
            let ordering = php_type_symbol("Ordering");
            format!("$order = {call}; return $order === {ordering}::Less ? -1 : ($order === {ordering}::Greater ? 1 : 0);")
        }
    };
    let parameters = if binary { "$left, $right" } else { "$value" };
    let mut helpers = scopes.core_helpers.borrow_mut();
    let index = helpers.declarations.len();
    let name = symbol(index);
    helpers.declarations.push(format!(
        "function {name}({parameters})\n{{\n    {body}\n}}\n\n"
    ));
    helpers.indices.insert(key, index);
    name
}
