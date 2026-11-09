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

pub(super) fn convert_collection(
    expr: &Expr,
    value: String,
    owned: bool,
    scopes: &PhpNameScopes,
) -> String {
    let Some(expected) = scopes.interface_conversion_types.get(&expr.span()) else {
        return value;
    };
    let expected = crate::types::substitute_resolved_type(expected, &scopes.substitutions);
    let Some(actual) = scopes.expression_types.get(&expr.span()) else {
        return value;
    };
    let actual = crate::types::substitute_resolved_type(actual, &scopes.substitutions);
    convert_typed_collection(&actual, &expected, value, owned, false)
}

pub(super) fn convert_typed_collection(
    actual: &ResolvedType,
    expected: &ResolvedType,
    value: String,
    owned: bool,
    borrowed_cell: bool,
) -> String {
    let expected = if let ResolvedType::Nullable(inner) = expected {
        inner.as_ref()
    } else {
        expected
    };
    let ResolvedType::Interface(interface) = expected else {
        return value;
    };
    if interface.name != "Iterable" {
        return value;
    }
    let actual = if let ResolvedType::Nullable(inner) = actual {
        inner.as_ref()
    } else {
        actual
    };
    if !matches!(
        actual,
        ResolvedType::TypedArray(_)
            | ResolvedType::List(_)
            | ResolvedType::Set(_)
            | ResolvedType::SortedSet(_)
            | ResolvedType::Deque(_)
            | ResolvedType::Dictionary(_, _)
            | ResolvedType::SortedDictionary(_, _)
    ) {
        return value;
    }
    if borrowed_cell {
        return format!(
            "{}Borrow::fromCell({value})",
            collection_adapter_name(interface)
        );
    }
    format!(
        "{}::from({value}, {})",
        collection_adapter_name(interface),
        if owned { "true" } else { "false" }
    )
}

fn collection_adapter_name(interface: &InterfaceType<ResolvedType>) -> String {
    format!("{}Collection", specialization_name(interface))
}

pub(super) fn builtin_acquire(expr: &Expr, scopes: &PhpNameScopes) -> Option<String> {
    let Expr::MethodCall { object, span, .. } = expr else {
        return None;
    };
    let crate::semantics::CallableTarget::ConstrainedMethod { requirement, .. } =
        scopes.specialization.target(*span, &scopes.substitutions)?
    else {
        return None;
    };
    if crate::compiler_known_contracts::IterationOperation::from_requirement(requirement)
        != Some(crate::compiler_known_contracts::IterationOperation::Acquire)
    {
        return None;
    }
    let ResolvedType::Interface(iterator) = crate::types::substitute_resolved_type(
        scopes.expression_types.get(span)?,
        &scopes.substitutions,
    ) else {
        return None;
    };
    let iterable = InterfaceType::new("Iterable", iterator.arguments);
    Some(format!(
        "{}::from({}, false)->iterator()",
        collection_adapter_name(&iterable),
        emit_expr(object, scopes)
    ))
}

pub(super) fn nominal_type_name(ty: &ResolvedType, scopes: &PhpNameScopes) -> Option<String> {
    match ty {
        ResolvedType::Class(class) => Some(
            scopes
                .specialization
                .class_symbols
                .get(class)
                .cloned()
                .unwrap_or_else(|| php_type_symbol(&class.name)),
        ),
        ResolvedType::Interface(interface) => Some(specialization_name(interface)),
        ResolvedType::Error => Some(declaration_name("Error")),
        _ => None,
    }
}

pub(super) fn emit_declarations(
    semantic: &SemanticInfo,
    specialization: &specialization::Plan,
    output: &mut String,
) {
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
    output.push_str(r#"
interface __DoriaCollectionView
{
    public function __doriaCollectionValues(): iterable;
}
abstract class __DoriaBuiltinIterable implements __DoriaOwnedObject, __DoriaCollectionView
{
    public mixed $source;
    private bool $owns;
    public function __construct(mixed $source, bool $owns) { $this->source = $source; $this->owns = $owns; }
    public function __doriaCollectionValues(): iterable { return $this->source; }
    public static function from(mixed $source, bool $owns): ?static
    {
        if ($source === null || $source instanceof static) { return $source; }
        return new static($source, $owns);
    }
    public function __destruct()
    {
        if ($this->owns && __doria_cleanup_enabled()) {
            $this->owns = false;
            __doria_drop_value($this->source);
        }
    }
}
abstract class __DoriaBuiltinIterator implements __DoriaOwnedObject
{
    private ?Generator $cursor;
    private int $position = 0;
    public function __construct(__DoriaCollectionView $source)
    {
        $this->cursor = (static function() use ($source) {
            foreach ($source->__doriaCollectionValues() as $value) { yield $value; }
        })();
        $this->cursor->rewind();
    }
    public function hasCurrent(): bool { return $this->cursor !== null && $this->cursor->valid(); }
    protected function currentValue(): mixed
    {
        if (!$this->hasCurrent()) { __doria_panic("P1310", 0, 0, null, "Iterator::getCurrent", ["index" => [1, $this->position], "length" => [2, $this->position]]); }
        return $this->cursor->current();
    }
    public function advance(): void { if ($this->hasCurrent()) { ++$this->position; $this->cursor->next(); } }
    public function __destruct() { $this->cursor = null; }
}
"#);
    for fact in &semantic.contracts.interface_specializations {
        if !fact.valid
            || fact.specialization.name != "Iterable"
            || !fact.requirements.iter().any(|requirement| {
                requirement.origins.iter().any(|origin| {
                    crate::compiler_known_contracts::IterationOperation::from_requirement(
                        origin.declaration,
                    ) == Some(crate::compiler_known_contracts::IterationOperation::Acquire)
                })
            })
        {
            continue;
        }
        let iterable = &fact.specialization;
        let iterator = InterfaceType::new("Iterator", iterable.arguments.clone());
        let carrier = collection_adapter_name(iterable);
        let cursor = collection_adapter_name(&iterator);
        let (current, plan, current_symbol) = specialization
            .iteration_callable(
                &ResolvedType::Interface(iterator.clone()),
                crate::compiler_known_contracts::IterationOperation::GetCurrent,
            )
            .expect("checked collection iterator has a current-value callable");
        let home = if plan.caller_receiver_home() {
            "__DoriaCell $__doriaReceiver"
        } else {
            ""
        };
        let body =
            if plan.returns_borrow && borrowed_result_uses_cell(&current.signature.return_type) {
                "$value = $this->currentValue(); return __DoriaCell::borrowed($value);"
            } else if !plan.returns_borrow {
                "return __doria_copy_value($this->currentValue());"
            } else {
                "return $this->currentValue();"
            };
        let (_, _, acquire_symbol) = specialization
            .iteration_callable(
                &ResolvedType::Interface(iterable.clone()),
                crate::compiler_known_contracts::IterationOperation::Acquire,
            )
            .expect("checked iterable has an acquisition entry");
        let acquire_alias = if acquire_symbol == "iterator" {
            String::new()
        } else {
            format!("public function {acquire_symbol}(): {cursor} {{ return $this->iterator(); }}")
        };
        let mut cursor_aliases = String::new();
        for (operation, original, returned) in [
            (
                crate::compiler_known_contracts::IterationOperation::HasCurrent,
                "hasCurrent",
                "bool",
            ),
            (
                crate::compiler_known_contracts::IterationOperation::Advance,
                "advance",
                "void",
            ),
        ] {
            let (_, _, symbol) = specialization
                .iteration_callable(&ResolvedType::Interface(iterator.clone()), operation)
                .expect("checked iterator has its protocol entry");
            if symbol != original {
                cursor_aliases.push_str(&format!(
                    "public function {symbol}(): {returned} {{ {}parent::{original}(); }}",
                    if returned == "void" { "" } else { "return " }
                ));
            }
        }
        output.push_str(&format!("final class {carrier} extends __DoriaBuiltinIterable implements {} {{ public function iterator(): {cursor} {{ return new {cursor}($this); }} {acquire_alias} }}\nfinal class {cursor} extends __DoriaBuiltinIterator implements {} {{ public function {current_symbol}({home}): mixed {{ {body} }} {cursor_aliases} }}\n", specialization_name(iterable), specialization_name(&iterator)));
        output.push_str(&format!(r#"final class {carrier}Borrow extends __DoriaCell implements {}, __DoriaCollectionView
{{
    public mixed $value {{
        get => $this->source->value === null ? null : $this;
        set {{ throw new LogicException('cannot replace a borrowed collection view'); }}
    }}
    private function __construct(private readonly __DoriaCell $source) {{ $this->borrowed = true; }}
    public static function fromCell(__DoriaCell $source): static {{ return $source instanceof static ? $source : new static($source); }}
    public function __doriaCollectionValues(): iterable {{ return $this->source->value; }}
    public function iterator(): {cursor} {{ return new {cursor}($this); }}
    {acquire_alias}
}}
"#, specialization_name(iterable)));
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
