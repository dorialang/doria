//! Stable homes for field-backed values and borrowed descriptors.
//! Boxed slots keep their real reference; field-backed homes share cell identity
//! across captures and writable callbacks.

use super::*;
use crate::ast::PropertyHookKind;

pub(in crate::codegen_php) fn cell(expr: &Expr, scopes: &PhpNameScopes) -> Option<String> {
    Shape::for_expr(expr, scopes)?;
    match expr {
        Expr::Grouped { expr, .. } => cell(expr, scopes),
        Expr::Variable { span, .. } => scopes.place_for_use(*span)?.cell(),
        Expr::PropertyAccess {
            object,
            property,
            span,
            null_safe: false,
            ..
        } => {
            let method = property_cell(object, property, *span, scopes)?;
            Some(format!(
                "{}->{method}()",
                property_hooks::receiver(object, false, scopes)
            ))
        }
        _ => None,
    }
}

fn property_cell<'a>(
    object: &Expr,
    property: &str,
    span: Span,
    scopes: &'a PhpNameScopes,
) -> Option<&'a str> {
    for kind in [PropertyHookKind::Get, PropertyHookKind::Set] {
        if scopes
            .specialization
            .accessor(span, kind, &scopes.substitutions)
            .is_some()
        {
            return None;
        }
    }
    let class = scopes.specialization.property_receiver_class(
        scopes.expression_types.get(&object.span())?,
        &scopes.substitutions,
    )?;
    scopes
        .specialization
        .property_storage_symbols(class, property)
        .map(|(_, method)| method)
}

pub(super) fn load(
    expr: &Expr,
    destination: &Place,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    let expr = expression_region::ungrouped(expr);
    if let Expr::PropertyAccess {
        object,
        property,
        span,
        null_safe,
        ..
    } = expr
    {
        let Some(method) = property_cell(object, property, *span, scopes).map(str::to_string)
        else {
            return false;
        };
        let load = |output: &mut String, indent, scopes: &mut PhpNameScopes| {
            let receiver = property_hooks::receiver(object, *null_safe, scopes);
            expression_region::consume(output, indent, scopes);
            if *null_safe {
                clear(output, indent);
            }
            let access = if *null_safe { "?->" } else { "->" };
            writeln(
                output,
                indent,
                &format!("{receiver}{access}{method}(){access}__doriaLoadFields();"),
            );
            // Capture before the nested receiver region can release its value.
            destination.capture(output, indent);
        };
        // The enclosing property region may already own the evaluated receiver.
        // Entering another expression emitter would discard that snapshot.
        if scopes
            .prepared_receiver
            .as_ref()
            .is_some_and(|receiver| receiver.value(object).is_some())
        {
            load(output, indent, scopes);
        } else {
            expression_region::emit(object, output, indent, scopes, load);
        }
        return true;
    }
    let Some(cell) = cell(expr, scopes) else {
        return false;
    };
    writeln(output, indent, &format!("{cell}->__doriaLoadFields();"));
    destination.capture(output, indent);
    true
}

pub(in crate::codegen_php) fn emit_property(
    name: &str,
    ty: &TypeRef,
    access: &MemberAccess,
    source: crate::source::SourceId,
    output: &mut String,
    indent: usize,
    scopes: &PhpNameScopes,
) -> bool {
    let Some((storage, method)) = scopes
        .current_class_id
        .and_then(|class| scopes.specialization.property_storage_symbols(class, name))
    else {
        return false;
    };
    let ty = scopes
        .specialization
        .resolve_type(ty, &scopes.substitutions)
        .unwrap();
    let shape = Shape::for_type(&ty, scopes).unwrap();
    let cell = cell_name(&shape.name);
    writeln(
        output,
        indent,
        &format!("private ?{cell} ${storage} = null;"),
    );
    writeln(
        output,
        indent,
        &format!("public function {method}(): {cell}"),
    );
    writeln(output, indent, "{");
    writeln(
        output,
        indent + 1,
        &format!("return $this->{storage} ??= new {cell}();"),
    );
    writeln(output, indent, "}");
    writeln(
        output,
        indent,
        &format!(
            "{} {} ${name} {{",
            emit_member_access(access, source),
            php_resolved_type(&ty, scopes)
        ),
    );
    writeln(
        output,
        indent + 1,
        &format!("get => $this->{method}()->value;"),
    );
    writeln(
        output,
        indent + 1,
        &format!("set {{ $this->{method}()->value = $value; }}"),
    );
    writeln(output, indent, "}");
    true
}

pub(in crate::codegen_php) fn cell_name(name: &str) -> String {
    format!("__DoriaValueCell_{}", hex_bytes(name.as_bytes()))
}

pub(in crate::codegen_php) fn initialize_property(
    property: &PropertyDecl,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    let Some((_, method)) = scopes.current_class_id.and_then(|class| {
        scopes
            .specialization
            .property_storage_symbols(class, &property.name)
    }) else {
        return false;
    };
    let method = method.to_string();
    let ty = scopes
        .specialization
        .resolve_type(&property.ty, &scopes.substitutions)
        .unwrap();
    let staged = Place::new(Shape::for_type(&ty, scopes).unwrap(), scopes);
    emit_into(
        property.initializer.as_ref().unwrap(),
        &staged,
        output,
        indent,
        scopes,
    );
    staged.publish(output, indent);
    writeln(
        output,
        indent,
        &format!("$this->{method}()->__doriaStoreFields();"),
    );
    true
}

fn cell_place(mut shape: Shape, name: &str) -> Place {
    shape.nullable = true;
    let fields = (0..shape.fields)
        .map(|index| format!("{name}->field{index}"))
        .collect();
    Place {
        shape,
        tag: format!("{name}->tag"),
        present: format!("{name}->present"),
        fields,
    }
}

pub(super) fn initialize(name: &str, source: &Place, output: &mut String, indent: usize) {
    let value = copy_fields(source);
    writeln(output, indent, &format!("${name} = {value};"));
}

fn copy_fields(source: &Place) -> String {
    let arguments = source
        .names()
        .map(|name| format!("${name}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("new {}({arguments})", cell_name(&source.shape.name))
}

pub(in crate::codegen_php) fn copy_capture(
    binding: BindingId,
    place: &PhpBindingPlace,
    scopes: &PhpNameScopes,
) -> Option<String> {
    let shape = Shape::for_type(scopes.source_type(binding)?, scopes)?;
    match place {
        PhpBindingPlace::Fields(fields) => Some(copy_fields(fields)),
        PhpBindingPlace::Cell(name) => Some(format!(
            "{}::__doriaCopyFrom(${name})",
            cell_name(&shape.name)
        )),
        PhpBindingPlace::Direct(name) => Some(format!(
            "{}::__doriaFromValue(${name})",
            cell_name(&shape.name)
        )),
    }
}

pub(in crate::codegen_php) fn initialize_parameter(
    name: &str,
    binding: BindingId,
    fields: Option<&Place>,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    let Some(shape) = scopes
        .source_type(binding)
        .and_then(|ty| Shape::for_type(ty, scopes))
    else {
        return false;
    };
    if let Some(fields) = fields {
        initialize(name, fields, output, indent);
    } else {
        let fields = Place::new(shape, scopes);
        unpack_stable(name, &fields, output, indent, &scopes.specialization);
        initialize(name, &fields, output, indent);
    }
    true
}

pub(in crate::codegen_php) fn emit_boxed_writer(
    declaration: &EnumDecl,
    output: &mut String,
    indent: usize,
    scopes: &PhpNameScopes,
) {
    let Some(definition) =
        definition(declaration, scopes).filter(|value| value.capabilities.trivial_copy)
    else {
        return;
    };
    if !definition.cases.iter().any(|case| !case.payload.is_empty()) {
        return;
    }
    writeln(
        output,
        indent,
        "public function __doriaPublishFields(): void",
    );
    writeln(output, indent, "{");
    let mut inner = scopes.clone();
    let shape = Shape::for_definition(definition, &scopes.specialization).unwrap();
    let place = Place::new(shape, &mut inner);
    unpack_stable("this", &place, output, indent + 1, &scopes.specialization);
    place.publish(output, indent + 1);
    writeln(output, indent, "}");
}

pub(super) fn emit_boxer(program: &Program, output: &mut String, plan: &specialization::Plan) {
    writeln(output, 1, "public static function boxed(): mixed");
    writeln(output, 1, "{");
    writeln(output, 2, "if (!self::$present) { return null; }");
    writeln(output, 2, "return match (self::$type) {");
    for kind in [
        crate::types::ByteRegionKind::Readonly,
        crate::types::ByteRegionKind::Writable,
    ] {
        let shape = Shape::for_concrete_type(&ResolvedType::ByteRegion(kind), plan).unwrap();
        writeln(
            output,
            3,
            &format!(
                "{} => {}(self::$tag, self::$field0, self::$field1, self::$field2),",
                shape.result_type(),
                boxer_name(&shape.name),
            ),
        );
    }
    for definition in &program.semantic_info.enums {
        // Symbolic applications of generic bodies are never materialized.
        if !definition.capabilities.trivial_copy
            || definition
                .source_type
                .arguments
                .iter()
                .any(crate::types::resolved_type_is_symbolic)
        {
            continue;
        }
        let fields = result_fields(definition, plan);
        if fields == 0 {
            continue;
        }
        let arguments = (0..fields)
            .map(|index| format!("self::$field{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln(
            output,
            3,
            &format!(
                "{} => {}(self::$tag, {arguments}),",
                definition.id.0,
                boxer_name(&enum_shape_name(definition, plan))
            ),
        );
    }
    writeln(
        output,
        3,
        "default => throw new LogicException('invalid Doria field result type'),",
    );
    writeln(output, 2, "};");
    writeln(output, 1, "}");
}

pub(in crate::codegen_php) fn emit_cell(
    declaration: &EnumDecl,
    output: &mut String,
    indent: usize,
    scopes: &PhpNameScopes,
) {
    let ty = instance_definition(declaration, scopes).unwrap();
    let ty = ResolvedType::Enum(ty.source_type.clone());
    let Some(shape) = Shape::for_type(&ty, scopes) else {
        return;
    };
    emit_shape_cell(shape, output, indent, &scopes.specialization);
}

pub(super) fn emit_shape_cell(
    shape: Shape,
    output: &mut String,
    indent: usize,
    plan: &specialization::Plan,
) {
    let name = cell_name(&shape.name);
    let place = cell_place(shape, "this");
    writeln(
        output,
        indent,
        &format!("final class {} extends __DoriaCell", name),
    );
    writeln(output, indent, "{");
    writeln(output, indent + 1, "public int $tag = -1;");
    writeln(output, indent + 1, "public bool $present = false;");
    for index in 0..place.shape.fields {
        writeln(
            output,
            indent + 1,
            &format!("public mixed $field{index} = null;"),
        );
    }
    let parameters = std::iter::once("int $tag = -1".to_string())
        .chain(std::iter::once("bool $present = false".to_string()))
        .chain((0..place.shape.fields).map(|index| format!("mixed $field{index} = null")))
        .collect::<Vec<_>>()
        .join(", ");
    writeln(
        output,
        indent + 1,
        &format!("public function __construct({parameters})"),
    );
    writeln(output, indent + 1, "{");
    writeln(output, indent + 2, "$this->tag = $tag;");
    writeln(output, indent + 2, "$this->present = $present;");
    for index in 0..place.shape.fields {
        writeln(
            output,
            indent + 2,
            &format!("$this->field{index} = $field{index};"),
        );
    }
    writeln(output, indent + 1, "}");
    writeln(
        output,
        indent + 1,
        "public static function __doriaCopyFrom(__DoriaCell $source): self",
    );
    writeln(output, indent + 1, "{");
    writeln(output, indent + 2, "$source->__doriaLoadFields();");
    let arguments = std::iter::once(format!("{RESULT}::$tag"))
        .chain(std::iter::once(format!("{RESULT}::$present")))
        .chain((0..place.shape.fields).map(|index| format!("{RESULT}::$field{index}")))
        .collect::<Vec<_>>()
        .join(", ");
    writeln(
        output,
        indent + 2,
        &format!("$copy = new self({arguments});"),
    );
    clear(output, indent + 2);
    writeln(output, indent + 2, "return $copy;");
    writeln(output, indent + 1, "}");
    writeln(
        output,
        indent + 1,
        "public static function __doriaFromValue(mixed $value): self",
    );
    writeln(output, indent + 1, "{");
    writeln(output, indent + 2, "$copy = new self();");
    writeln(output, indent + 2, "$copy->value = $value;");
    writeln(output, indent + 2, "return $copy;");
    writeln(output, indent + 1, "}");
    writeln(output, indent + 1, "public mixed $value {");
    writeln(output, indent + 2, &format!("get => {};", place.read()));
    writeln(output, indent + 2, "set {");
    unpack_stable("value", &place, output, indent + 3, plan);
    writeln(output, indent + 2, "}");
    writeln(output, indent + 1, "}");
    writeln(
        output,
        indent + 1,
        "public function __doriaProjectCell(): __DoriaCell { return $this; }",
    );
    writeln(
        output,
        indent + 1,
        "public function __doriaIsPresent(): bool { return $this->present; }",
    );
    writeln(
        output,
        indent + 1,
        "public function __doriaDropValue(): void",
    );
    writeln(output, indent + 1, "{");
    place.absent(output, indent + 2);
    writeln(output, indent + 1, "}");
    writeln(
        output,
        indent + 1,
        "public function __doriaLoadFields(): void",
    );
    writeln(output, indent + 1, "{");
    place.publish(output, indent + 2);
    writeln(output, indent + 1, "}");
    writeln(
        output,
        indent + 1,
        "public function __doriaStoreFields(): void",
    );
    writeln(output, indent + 1, "{");
    place.capture(output, indent + 2);
    writeln(output, indent + 2, "$this->live = true;");
    writeln(output, indent + 1, "}");
    writeln(output, indent, "}");
}
