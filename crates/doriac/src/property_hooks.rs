//! Shared declaration facts for property hooks, not an executable representation.
//!
//! These facts borrow authored syntax. Declaration validation uses the same
//! facts; return ownership, layout/dispatch slots, and calls are checked later.

use std::borrow::Cow;

use crate::ast::{self, Expr, PropertyDecl, PropertyHook, PropertyHookKind};
use crate::diagnostics::Diagnostic;
use crate::source::Span;
use crate::symbols::ReceiverMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyHookStorage {
    Computed,
    Backed,
}

/// The one physical field shared by every backed override in a property family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyBackingField {
    pub declaring_class: String,
    pub property_name: String,
    pub declaration: Span,
}

/// Compiler-only callable name. Source member lookup always uses the property
/// and accessor kind, never this name as a user-declared method.
pub fn accessor_name(property: &str, kind: PropertyHookKind) -> String {
    let kind = match kind {
        PropertyHookKind::Get => "get",
        PropertyHookKind::Set => "set",
    };
    format!("{property}::{kind}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyHookContext {
    Class,
    /// Storage here describes the contribution to the eventual composing class,
    /// not a runtime trait instance.
    Trait,
    Interface,
}

/// Source identity of an accessor, independent of generated callable names.
///
/// Preserve the complete property span, including its source and expansion
/// identities. Composition/specialization owns any concrete owner or dispatch
/// identity; this is not a substitute for those existing facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertyAccessorIdentity {
    pub property_span: Span,
    pub kind: PropertyHookKind,
}

#[derive(Debug, Clone, Copy)]
pub struct PropertyAccessorFacts<'a> {
    pub identity: PropertyAccessorIdentity,
    /// Retains kind, source spans, typed parameter, throws clause, and body.
    pub hook: &'a PropertyHook,
    pub receiver_mode: ReceiverMode,
}

#[derive(Debug, Clone, Copy)]
pub struct PropertyHookFacts<'a> {
    /// Retains authored name/type/access/writability, initializer, modifiers,
    /// hooks, and source spans without copying or restating AST data.
    pub property: &'a PropertyDecl,
    pub context: PropertyHookContext,
    /// Interface requirements have no instance storage, even if malformed
    /// source contains an implementation body. Semantic validation is separate.
    pub storage: Option<PropertyHookStorage>,
}

/// Enumerate member bodies through the same callable model. Ordinary methods
/// stay borrowed; hook views retain the authored accessor's source identity.
pub fn member_callables(
    member: &ast::ClassMember,
    context: PropertyHookContext,
) -> impl Iterator<Item = Cow<'_, ast::FunctionDecl>> {
    let method = match member {
        ast::ClassMember::Method(method) => Some(Cow::Borrowed(method)),
        _ => None,
    };
    let hooks = match member {
        ast::ClassMember::Property(property) => declaration_facts(property, context),
        _ => None,
    };
    method.into_iter().chain(
        hooks
            .into_iter()
            .flat_map(|facts| facts.callables().map(Cow::Owned)),
    )
}

impl<'a> PropertyHookFacts<'a> {
    pub fn symbols(&self) -> crate::symbols::PropertyHooksInfo {
        let accessor = |kind| {
            self.accessors()
                .find(|accessor| accessor.hook.kind == kind)
                .map(|accessor| crate::symbols::PropertyAccessorInfo {
                    declaration: accessor.hook.span,
                    receiver_mode: accessor.receiver_mode,
                })
        };
        crate::symbols::PropertyHooksInfo {
            storage: self.storage,
            is_open: self.property.open_span.is_some(),
            is_override: self.property.override_span.is_some(),
            getter: accessor(PropertyHookKind::Get),
            setter: accessor(PropertyHookKind::Set),
        }
    }

    /// A callable view for the shared signature, body, ownership, and effect
    /// checkers. Identity is the authored hook span; the display name is never
    /// registered as a source-callable method.
    pub fn callables(&self) -> impl Iterator<Item = ast::FunctionDecl> + 'a {
        let property = self.property;
        self.accessors().map(move |accessor| {
            let hook = accessor.hook;
            ast::FunctionDecl {
                access: property.access,
                access_span: None,
                is_open: property.open_span.is_some(),
                open_span: property.open_span,
                is_override: property.override_span.is_some(),
                override_span: property.override_span,
                writable_this: accessor.receiver_mode == ReceiverMode::Writable,
                writable_span: hook.writable_span,
                is_static: property.is_static,
                static_span: None,
                name: accessor_name(&property.name, hook.kind),
                name_span: hook.keyword_span,
                type_params: Vec::new(),
                params: hook.parameter.iter().cloned().collect(),
                return_type: Some(match hook.kind {
                    PropertyHookKind::Get => property.ty.clone(),
                    PropertyHookKind::Set => crate::types::TypeRef::named("void"),
                }),
                throws: hook.throws.clone(),
                body: hook.body.clone(),
                syntax: Box::new(ast::FunctionSyntax::synthetic(hook.keyword_span)),
                modifier_prefix_span: hook.span.at(hook.span.start, hook.keyword_span.start),
                span: hook.span,
            }
        })
    }

    pub fn validate_declaration(&self, diagnostics: &mut Vec<Diagnostic>) {
        if self.property.writable {
            return;
        }
        for accessor in self.accessors() {
            if accessor.hook.kind == PropertyHookKind::Set {
                diagnostics.push(
                    Diagnostic::new(
                        "E0765",
                        format!("setter for `${}` requires a writable property", self.property.name),
                        accessor.hook.keyword_span,
                    )
                    .with_title("Setter Requires A Writable Property")
                    .with_related(self.property.name_span, "Property Declared Readonly Here")
                    .with_explanation("a setter lets callers assign the property, so the property must be declared `writable`")
                    .with_help("declare the property `writable`, or remove the setter to keep it readonly"),
                );
            }
        }
    }

    /// Accessors in authored order; no AST clones or synthetic wrappers.
    pub fn accessors(&self) -> impl ExactSizeIterator<Item = PropertyAccessorFacts<'a>> + 'a {
        let property = self.property;
        property
            .hooks
            .iter()
            .map(move |hook| PropertyAccessorFacts {
                identity: PropertyAccessorIdentity {
                    property_span: property.span,
                    kind: hook.kind,
                },
                hook,
                receiver_mode: match hook.kind {
                    PropertyHookKind::Set => ReceiverMode::Writable,
                    PropertyHookKind::Get if hook.writable_span.is_some() => ReceiverMode::Writable,
                    PropertyHookKind::Get => ReceiverMode::Readonly,
                },
            })
    }
}

/// Classify one hooked declaration. Ordinary stored properties return `None`.
///
/// Only a direct `$this->sameProperty` occurrence in a hook body establishes
/// automatic backing; parentheses around `$this` are transparent. Initializers
/// and parameter defaults are not hook bodies.
/// A computed declaration with an initializer remains classified as computed;
/// deciding whether that combination is legal belongs to semantic checking.
pub fn declaration_facts(
    property: &PropertyDecl,
    context: PropertyHookContext,
) -> Option<PropertyHookFacts<'_>> {
    if property.hooks.is_empty() {
        return None;
    }
    let storage = match context {
        PropertyHookContext::Interface => None,
        PropertyHookContext::Class | PropertyHookContext::Trait => {
            let backed = property.hooks.iter().any(|hook| {
                let Some(body) = hook.body.as_block() else {
                    return false;
                };
                let mut references_backing = false;
                ast::visit::block(body, &mut |expression| {
                    if let Expr::PropertyAccess {
                        object,
                        property: name,
                        ..
                    } = expression
                    {
                        references_backing |= name == &property.name && is_direct_this(object);
                    }
                });
                references_backing
            });
            Some(if backed {
                PropertyHookStorage::Backed
            } else {
                PropertyHookStorage::Computed
            })
        }
    };
    Some(PropertyHookFacts {
        property,
        context,
        storage,
    })
}

fn is_direct_this(mut expression: &Expr) -> bool {
    while let Expr::Grouped { expr, .. } = expression {
        expression = expr;
    }
    matches!(expression, Expr::This { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{ClassMember, FunctionBody, Item};
    use crate::source::{ExpansionId, SourceId};
    use crate::types::TypeArgumentRef;

    fn class_properties(members: &str) -> Vec<PropertyDecl> {
        let source = format!("class Example<T> {{ {members} }}");
        let program = crate::parse_source("hooks.doria", &source).expect("hook syntax parses");
        let Item::Class(class) = program.items.into_iter().next().unwrap() else {
            panic!("expected class");
        };
        class
            .members
            .into_iter()
            .filter_map(|member| match member {
                ClassMember::Property(property) => Some(property),
                _ => None,
            })
            .collect()
    }

    fn storage(members: &str) -> Option<PropertyHookStorage> {
        let properties = class_properties(members);
        declaration_facts(&properties[0], PropertyHookContext::Class)
            .expect("expected hooked property")
            .storage
    }

    #[test]
    fn backing_requires_own_property_on_direct_this() {
        for body in [
            "get => $this->value;",
            "set (int $next) => $this->value = $next;",
            "get => $this->value->other;",
            "get => ($this->value);",
            "get => ($this)->value;",
            "get => ((($this)))->value;",
            "set (int $next) => (($this))->value = $next;",
            "get => $this->value[0];",
        ] {
            assert_eq!(
                storage(&format!("writable int $value {{ {body} }}")),
                Some(PropertyHookStorage::Backed),
                "{body}"
            );
        }
        for body in [
            "get => $this->other;",
            "get => $other->value;",
            "get => (($other))->value;",
            "get => $this->other->value;",
            "get => (($this)->other)->value;",
            "get => 42;",
            r#"get => "$this->value";"#,
            "get; set (int $next);",
        ] {
            assert_eq!(
                storage(&format!("writable int $value {{ {body} }}")),
                Some(PropertyHookStorage::Computed),
                "{body}"
            );
        }
    }

    #[test]
    fn initializers_do_not_create_backing_for_computed_hooks() {
        for initializer in ["0", "$this->value"] {
            assert_eq!(
                storage(&format!("int $value = {initializer} {{ get => 42; }}")),
                Some(PropertyHookStorage::Computed)
            );
        }
        assert_eq!(
            storage(
                r#"writable string $text = "" {
                get => $this->text;
                set (string $value) => $this->text = String::trim($value);
            }"#
            ),
            Some(PropertyHookStorage::Backed)
        );
    }

    #[test]
    fn shared_traversal_finds_backing_in_nested_bodies_and_closures() {
        for body in [
            "get { if (true) { return $this->value; } return 0; }",
            "get { try { return 0; } finally { echo $this->value; } }",
            "get { while (false) { echo $this->value; } return 0; }",
            "get { let $read = fn() with ($this) => $this->value; return 0; }",
            "get { let $read = function (): int with ($this) { return $this->value; }; return 0; }",
        ] {
            assert_eq!(
                storage(&format!("int $value {{ {body} }}")),
                Some(PropertyHookStorage::Backed),
                "{body}"
            );
        }
    }

    #[test]
    fn parameter_defaults_are_not_hook_bodies() {
        // Defaults are parser-invalid for setters. Keep the classification
        // boundary explicit even for an AST presented before validation.
        let mut properties =
            class_properties("writable int $value { set (int $next); } int $probe = $this->value;");
        let default = properties[1].initializer.take();
        properties[0].hooks[0].parameter.as_mut().unwrap().default = default;
        let facts = declaration_facts(&properties[0], PropertyHookContext::Class).unwrap();
        assert_eq!(facts.storage, Some(PropertyHookStorage::Computed));
    }

    #[test]
    fn interface_context_never_contributes_instance_storage() {
        let program = crate::parse_source(
            "hooks.doria",
            "interface Example<T> { T $value { get; } T $invalid { get => $this->invalid; } }",
        )
        .expect("parser preserves bodies for later semantic checking");
        let Item::Interface(interface) = &program.items[0] else {
            panic!("expected interface");
        };
        for property in &interface.properties {
            let facts = declaration_facts(property, PropertyHookContext::Interface).unwrap();
            assert_eq!(facts.context, PropertyHookContext::Interface);
            assert_eq!(facts.storage, None);
            assert_eq!(facts.accessors().len(), 1);
        }
        assert!(matches!(
            interface.properties[0].hooks[0].body,
            FunctionBody::Requirement { .. }
        ));
    }

    #[test]
    fn trait_context_classifies_its_contribution_without_a_runtime_trait_owner() {
        let program = crate::parse_source(
            "hooks.doria",
            "trait Example<T> { T $value { get => $this->value; } }",
        )
        .expect("trait hook parses");
        let Item::Trait(declaration) = &program.items[0] else {
            panic!("expected trait");
        };
        let ClassMember::Property(property) = &declaration.members[0] else {
            panic!("expected property");
        };
        let facts = declaration_facts(property, PropertyHookContext::Trait).unwrap();
        assert_eq!(facts.context, PropertyHookContext::Trait);
        assert_eq!(facts.storage, Some(PropertyHookStorage::Backed));
    }

    #[test]
    fn generic_facts_borrow_authored_contract_and_distinguish_accessor_identities() {
        let properties = class_properties(
            "internal open writable List<T> $values {
                get throws ReadError<T> => $this->values;
                set (take List<T> $next) throws WriteError<T> => $this->values = $next;
            }
            override T $cached { writable get => $this->cached; }",
        );
        let property = &properties[0];
        let facts = declaration_facts(property, PropertyHookContext::Class).unwrap();
        assert!(std::ptr::eq(facts.property, property));
        assert_eq!(facts.property.name, "values");
        assert_eq!(facts.property.access, ast::MemberAccess::Internal);
        assert!(facts.property.writable);
        assert!(facts.property.open_span.is_some());
        assert_eq!(facts.property.ty.name, "List");
        let TypeArgumentRef::Type(element) = &facts.property.ty.arguments[0] else {
            panic!("expected generic type argument");
        };
        assert_eq!(element.name, "T");
        let mut accessors = facts.accessors();
        let get = accessors.next().unwrap();
        let set = accessors.next().unwrap();
        assert!(accessors.next().is_none());
        assert!(std::ptr::eq(get.hook, &property.hooks[0]));
        assert!(std::ptr::eq(set.hook, &property.hooks[1]));
        assert_eq!(get.receiver_mode, ReceiverMode::Readonly);
        assert_eq!(set.receiver_mode, ReceiverMode::Writable);
        assert_eq!(get.identity.property_span, property.span);
        assert_eq!(get.identity.kind, PropertyHookKind::Get);
        assert_eq!(set.identity.kind, PropertyHookKind::Set);
        assert_ne!(get.identity, set.identity);
        assert_eq!(
            get.hook.throws.as_ref().unwrap().entries[0].ty.name,
            "ReadError"
        );
        assert_eq!(
            set.hook.throws.as_ref().unwrap().entries[0].ty.name,
            "WriteError"
        );
        assert!(set.hook.parameter.as_ref().unwrap().take);
        let parameter_type = &set.hook.parameter.as_ref().unwrap().ty;
        assert_eq!(parameter_type.name, "List");
        let TypeArgumentRef::Type(parameter_element) = &parameter_type.arguments[0] else {
            panic!("expected setter generic type argument");
        };
        assert_eq!(parameter_element.name, "T");
        let cached = declaration_facts(&properties[1], PropertyHookContext::Class).unwrap();
        assert!(cached.property.override_span.is_some());
        assert!(!cached.property.writable);
        assert_eq!(
            cached.accessors().next().unwrap().receiver_mode,
            ReceiverMode::Writable
        );
        assert_eq!(cached.accessors().len(), 1);
    }

    #[test]
    fn accessor_identity_preserves_source_and_expansion_identity() {
        let mut properties = class_properties("int $value { get => $this->value; }");
        properties[0].span.source = SourceId(17);
        properties[0].span.expansion = ExpansionId(23);
        let facts = declaration_facts(&properties[0], PropertyHookContext::Class).unwrap();
        let accessor = facts.accessors().next().unwrap();
        assert_eq!(accessor.identity.property_span, properties[0].span);
    }

    #[test]
    fn ordinary_stored_properties_are_not_hook_declarations() {
        let properties = class_properties("writable int $value = 0;");
        for context in [
            PropertyHookContext::Class,
            PropertyHookContext::Trait,
            PropertyHookContext::Interface,
        ] {
            assert!(declaration_facts(&properties[0], context).is_none());
        }
    }
}
