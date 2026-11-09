//! Reusable result slots for inline aggregate and borrowed descriptor producers.
//!
//! A caller must capture the fields before another call and clear the channel
//! after moving its result into local storage. Existing boxed boundaries keep
//! their ownership contract while callers migrate to direct field transport.

use super::*;

pub(super) mod storage;

const RESULT: &str = "__DoriaValueResult";
pub(super) const REGION_FIELDS: usize = 3;

pub(super) fn region_result_type(kind: crate::types::ByteRegionKind) -> i64 {
    match kind {
        crate::types::ByteRegionKind::Readonly => -2,
        crate::types::ByteRegionKind::Writable => -3,
    }
}

pub(super) fn uses_fields(ty: &ResolvedType, enums: &[crate::semantics::EnumSemanticInfo]) -> bool {
    let inner = match ty {
        ResolvedType::Nullable(inner) => inner.as_ref(),
        ty => ty,
    };
    if matches!(inner, ResolvedType::ByteRegion(_)) {
        return true;
    }
    let ResolvedType::Enum(ty) = inner else {
        return false;
    };
    let Some(facts) = enums
        .iter()
        .find(|definition| definition.source_type == *ty)
    else {
        return false;
    };
    facts.capabilities.trivial_copy && facts.cases.iter().any(|case| !case.payload.is_empty())
}

pub(super) fn supports_intrinsic(intrinsic: Option<crate::ast::CompilerIntrinsic>) -> bool {
    use crate::compiler_known_stream::FileFunction;
    matches!(
        intrinsic,
        None | Some(crate::ast::CompilerIntrinsic::File(
            FileFunction::Read
                | FileFunction::Write
                | FileFunction::CompleteRead
                | FileFunction::CompleteWrite
                | FileFunction::ReadRequestOutcome
                | FileFunction::WriteRequestOutcome
        )) | Some(crate::ast::CompilerIntrinsic::TransferGroup(
            crate::compiler_known_stream::TransferGroupFunction::NextCompletion
                | crate::compiler_known_stream::TransferGroupFunction::ReadRegion
        ))
    )
}

pub(super) fn supports_intrinsic_arguments(
    intrinsic: Option<crate::ast::CompilerIntrinsic>,
) -> bool {
    matches!(
        intrinsic,
        None | Some(crate::ast::CompilerIntrinsic::TransferGroup(
            crate::compiler_known_stream::TransferGroupFunction::NextCompletion
        )) | Some(crate::ast::CompilerIntrinsic::File(
            crate::compiler_known_stream::FileFunction::Read
                | crate::compiler_known_stream::FileFunction::Write
                | crate::compiler_known_stream::FileFunction::PrepareRead
                | crate::compiler_known_stream::FileFunction::PrepareWrite
                | crate::compiler_known_stream::FileFunction::CompleteRead
                | crate::compiler_known_stream::FileFunction::CompleteWrite
        ))
    )
}

pub(super) enum CaseArgument<'a> {
    Scalar(&'a str),
    Result(&'a str),
}

pub(super) fn case_call(
    name: &str,
    case: &str,
    arguments: &[CaseArgument<'_>],
    plan: &specialization::Plan,
) -> String {
    let definition = plan.enum_definition(name).expect("checked intrinsic enum");
    let index = definition
        .cases
        .iter()
        .position(|entry| entry.name == case)
        .expect("checked intrinsic enum case");
    let shape = Shape::for_definition(definition, plan).expect("field-producing intrinsic enum");
    assert_eq!(shape.cases[index].len(), arguments.len());
    let arguments = shape.cases[index]
        .iter()
        .zip(arguments)
        .flat_map(|(payload, argument)| match argument {
            CaseArgument::Scalar(value) => {
                assert!(
                    payload.nested.is_none(),
                    "scalar intrinsic payload is a leaf"
                );
                vec![value.to_string()]
            }
            CaseArgument::Result(name) => {
                let nested = payload.nested.as_ref().expect("nested intrinsic result");
                assert_eq!(nested.name, *name);
                std::iter::once(format!("{RESULT}::$tag"))
                    .chain(std::iter::once(format!("{RESULT}::$present")))
                    .chain((0..nested.fields).map(|index| format!("{RESULT}::$field{index}")))
                    .collect()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}({arguments});", writer_name(name, case))
}

pub(super) fn intrinsic_parameter(
    function: &FunctionDecl,
    index: usize,
    fields: Option<&Place>,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> Place {
    if let Some(place) = fields {
        return place.clone();
    }
    let parameter = &function.params[index];
    let source = if callable_parameter_uses_cell(
        scopes.closure_plan.callable_definition(function.span),
        index,
    ) {
        PhpBindingPlace::Cell(parameter.name.clone())
    } else {
        PhpBindingPlace::Direct(parameter.name.clone())
    };
    let ty = scopes
        .specialization
        .resolve_type(&parameter.ty, &scopes.substitutions)
        .expect("typed intrinsic parameter");
    let shape =
        Shape::for_concrete_type(&ty, &scopes.specialization).expect("field intrinsic parameter");
    let place = Place::new(shape, scopes);
    unpack(&source.read(), &place, output, indent, scopes);
    place
}

#[derive(Debug, Clone)]
pub(super) struct Return {
    destination: Destination,
    label: String,
}

impl Return {
    pub(super) fn new(ty: &ResolvedType, scopes: &mut PhpNameScopes) -> Self {
        Self {
            destination: Destination::Fields(Place::new(
                Shape::for_type(ty, scopes).expect("checked field return type"),
                scopes,
            )),
            label: scopes.fresh_label("__doria_return_fields"),
        }
    }
}

pub(super) fn emit_return(
    expr: &Expr,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    let Some(result) = scopes.value_return.clone() else {
        return false;
    };
    let temporaries = scopes.expression_temporaries.clone();
    if let Some(yield_temporaries) = &scopes.yield_temporaries {
        scopes.expression_temporaries = Some(Rc::clone(yield_temporaries));
    }
    if let Destination::Value { name, owned: true } = &result.destination {
        for pending in &scopes.pending_returns {
            let mut pending = pending.borrow_mut();
            if !pending.contains(name) {
                pending.push(name.clone());
            }
        }
    }
    result.destination.emit(expr, output, indent, scopes);
    scopes.expression_temporaries = temporaries;
    writeln(output, indent, &format!("goto {};", result.label));
    true
}

pub(super) fn emit_return_epilogue(output: &mut String, indent: usize, scopes: &PhpNameScopes) {
    let Some(result) = &scopes.value_return else {
        return;
    };
    let Destination::Fields(place) = &result.destination else {
        unreachable!("callable field epilogue requires field storage");
    };
    writeln(output, indent, &format!("{}:", result.label));
    place.publish(output, indent);
    writeln(output, indent, "return;");
}

pub(super) fn clear(output: &mut String, indent: usize) {
    writeln(output, indent, &format!("{RESULT}::clear();"));
}

pub(super) fn field_call(expr: &Expr, scopes: &PhpNameScopes) -> bool {
    if Shape::for_expr(expr, scopes).is_none() {
        return false;
    }
    if let Expr::CallableCall(call) = expr {
        let function = scopes
            .closure_plan
            .callable_value_type(scopes.closure_owner, call.span);
        return scopes
            .specialization
            .function_field_result(function)
            .is_some();
    }
    scopes
        .specialization
        .field_call_symbol(expr.span(), &scopes.substitutions)
        .is_some()
}

pub(super) fn emit_boxed_return(
    ty: &ResolvedType,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let shape = Shape::for_type(ty, scopes).unwrap();
    if shape.nullable {
        writeln(
            output,
            indent,
            &format!("if (!{RESULT}::$present) {{ {RESULT}::clear(); return null; }}"),
        );
    }
    let fields = (0..shape.fields)
        .map(|index| format!("{RESULT}::$field{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let temporary = scopes.fresh_temp("__doria_boxed_return");
    writeln(
        output,
        indent,
        &format!(
            "${temporary} = {}({RESULT}::$tag, {fields});",
            boxer_name(&shape.name)
        ),
    );
    writeln(output, indent, &format!("{RESULT}::clear();"));
    writeln(output, indent, &format!("return ${temporary};"));
}

pub(super) fn emit_forwarded_result(
    value: &str,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let Destination::Fields(place) = &scopes.value_return.as_ref().unwrap().destination else {
        unreachable!("forwarded field return requires field storage");
    };
    unpack(value, &place.clone(), output, indent, scopes);
}

pub(super) fn bind_native_results(mut runtime: String) -> String {
    for (token, enumeration, case) in [
        ("__DORIA_READ_DATA_RESULT__", "ReadOutcome", "Data"),
        ("__DORIA_READ_BLOCK_RESULT__", "ReadOutcome", "WouldBlock"),
        ("__DORIA_READ_EOF_RESULT__", "ReadOutcome", "EndOfStream"),
        ("__DORIA_READ_TIMEOUT_RESULT__", "ReadOutcome", "TimedOut"),
        ("__DORIA_WRITE_DATA_RESULT__", "WriteOutcome", "Accepted"),
        ("__DORIA_WRITE_BLOCK_RESULT__", "WriteOutcome", "WouldBlock"),
    ] {
        runtime = runtime.replace(
            token,
            &writer_name(&format!("Doria\\Std\\Io\\{enumeration}"), case),
        );
    }
    for (token, enumeration) in [
        ("__DORIA_READ_BOX__", "ReadOutcome"),
        ("__DORIA_WRITE_BOX__", "WriteOutcome"),
    ] {
        runtime = runtime.replace(
            token,
            &boxer_name(&format!("Doria\\Std\\Io\\{enumeration}")),
        );
    }
    runtime
}

#[derive(Debug, Clone)]
struct Shape {
    kind: ShapeKind,
    name: String,
    nullable: bool,
    fields: usize,
    cases: Vec<Vec<Payload>>,
}

#[derive(Debug, Clone, Copy)]
enum ShapeKind {
    Enum(crate::enums::EnumId),
    ByteRegion(crate::types::ByteRegionKind),
}

#[derive(Debug, Clone)]
struct Payload {
    offset: usize,
    nested: Option<Box<Shape>>,
}

impl Payload {
    fn width(&self) -> usize {
        self.nested.as_ref().map_or(1, |shape| 2 + shape.fields)
    }

    fn place(&self, fields: &[String]) -> Option<Place> {
        let shape = self.nested.as_deref()?.clone();
        Some(Place {
            tag: fields[self.offset].clone(),
            present: fields[self.offset + 1].clone(),
            fields: fields[self.offset + 2..self.offset + self.width()].to_vec(),
            shape,
        })
    }

    fn value(&self, fields: &[String]) -> String {
        self.place(fields)
            .map_or_else(|| format!("${}", fields[self.offset]), |place| place.read())
    }
}

impl Shape {
    fn for_type(ty: &ResolvedType, scopes: &PhpNameScopes) -> Option<Self> {
        let ty = crate::types::substitute_resolved_type(ty, &scopes.substitutions);
        Self::for_concrete_type(&ty, &scopes.specialization)
    }

    fn for_concrete_type(ty: &ResolvedType, plan: &specialization::Plan) -> Option<Self> {
        let (inner, nullable) = match ty {
            ResolvedType::Nullable(inner) => (inner.as_ref(), true),
            inner => (inner, false),
        };
        if let ResolvedType::ByteRegion(kind) = inner {
            return Some(Self {
                kind: ShapeKind::ByteRegion(*kind),
                name: kind.source_name().to_string(),
                nullable,
                fields: REGION_FIELDS,
                cases: Vec::new(),
            });
        }
        let ResolvedType::Enum(ty) = inner else {
            return None;
        };
        let definition = plan.enum_definition_for_type(ty)?;
        if !definition.capabilities.trivial_copy
            || definition.cases.iter().all(|case| case.payload.is_empty())
        {
            return None;
        }
        let cases = definition
            .cases
            .iter()
            .map(|case| {
                let mut offset = 0;
                case.payload
                    .iter()
                    .map(|field| {
                        let payload = Payload {
                            offset,
                            nested: Self::for_concrete_type(&field.ty, plan).map(Box::new),
                        };
                        offset += payload.width();
                        payload
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let fields = cases
            .iter()
            .map(|fields| fields.iter().map(Payload::width).sum())
            .max()?;
        Some(Self {
            kind: ShapeKind::Enum(definition.id),
            name: enum_shape_name(definition, plan),
            nullable,
            fields,
            cases,
        })
    }

    fn result_type(&self) -> i64 {
        match self.kind {
            ShapeKind::Enum(id) => id.0 as i64,
            ShapeKind::ByteRegion(kind) => region_result_type(kind),
        }
    }

    fn case_width(&self, case: usize) -> usize {
        self.cases[case].iter().map(Payload::width).sum()
    }

    fn for_definition(
        definition: &crate::semantics::EnumSemanticInfo,
        plan: &specialization::Plan,
    ) -> Option<Self> {
        Self::for_concrete_type(&ResolvedType::Enum(definition.source_type.clone()), plan)
    }

    fn for_expr(expr: &Expr, scopes: &PhpNameScopes) -> Option<Self> {
        Self::for_type(scopes.expression_types.get(&expr.span())?, scopes)
    }
}

#[derive(Debug, Clone)]
pub(super) struct Place {
    shape: Shape,
    tag: String,
    present: String,
    fields: Vec<String>,
}

impl Place {
    pub(super) fn argument(
        expr: &Expr,
        ty: &ResolvedType,
        output: &mut String,
        indent: usize,
        scopes: &mut PhpNameScopes,
    ) -> Self {
        Self::expression(
            expr,
            Shape::for_type(ty, scopes).expect("checked field argument type"),
            output,
            indent,
            scopes,
        )
    }

    fn expression(
        expr: &Expr,
        shape: Shape,
        output: &mut String,
        indent: usize,
        scopes: &mut PhpNameScopes,
    ) -> Self {
        let place = Self::new(shape, scopes);
        for name in place.names() {
            scopes.track_expression_temporary(PhpExpressionTemporary::Alias(name.to_string()));
        }
        expression_region::emit(expr, output, indent, scopes, |output, indent, scopes| {
            emit_into(expr, &place, output, indent, scopes);
        });
        place
    }

    fn equality(&self, other: &Self) -> String {
        assert_eq!(self.shape.name, other.shape.name);
        let cases = self
            .shape
            .cases
            .iter()
            .enumerate()
            .map(|(tag, fields)| {
                let payload = fields
                    .iter()
                    .map(|field| {
                        if let Some(left) = field.place(&self.fields) {
                            left.equality(
                                &field
                                    .place(&other.fields)
                                    .expect("matching nested enum shape"),
                            )
                        } else {
                            format!(
                                "__doria_equal(${}, ${})",
                                self.fields[field.offset], other.fields[field.offset]
                            )
                        }
                    })
                    .collect::<Vec<_>>();
                format!(
                    "(${} === {tag}{})",
                    self.tag,
                    if payload.is_empty() {
                        String::new()
                    } else {
                        format!(" && {}", payload.join(" && "))
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(" || ");
        format!(
            "(${} === ${} && (!${} || (${} === ${} && ({cases}))))",
            self.present, other.present, self.present, self.tag, other.tag
        )
    }

    pub(super) fn parameter(ty: &ResolvedType, index: usize, scopes: &mut PhpNameScopes) -> Self {
        let ty = crate::types::substitute_resolved_type(ty, &scopes.substitutions);
        let place = Self::parameter_in(&ty, index, &scopes.specialization);
        scopes
            .used_php_names
            .extend(place.names().map(str::to_string));
        place
    }

    pub(super) fn parameter_in(
        ty: &ResolvedType,
        index: usize,
        plan: &specialization::Plan,
    ) -> Self {
        let shape = Shape::for_concrete_type(ty, plan).expect("checked field parameter type");
        let names = component_names(&shape, index, plan);
        Self {
            shape,
            tag: names[0].clone(),
            present: names[1].clone(),
            fields: names[2..].to_vec(),
        }
    }

    pub(super) fn declarations(&self) -> Vec<String> {
        std::iter::once(format!("int ${}", self.tag))
            .chain(std::iter::once(format!("bool ${}", self.present)))
            .chain(self.fields.iter().map(|field| format!("mixed ${field}")))
            .collect()
    }

    fn new(shape: Shape, scopes: &mut PhpNameScopes) -> Self {
        let tag = scopes.fresh_temp("__doria_value_tag");
        let present = scopes.fresh_temp("__doria_value_present");
        let fields = (0..shape.fields)
            .map(|_| scopes.fresh_temp("__doria_value_field"))
            .collect();
        Self {
            shape,
            tag,
            present,
            fields,
        }
    }

    pub(super) fn names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.tag.as_str())
            .chain(std::iter::once(self.present.as_str()))
            .chain(self.fields.iter().map(String::as_str))
    }

    pub(super) fn tag(&self) -> String {
        format!("${}", self.tag)
    }

    pub(super) fn region_parts(&self) -> Option<[String; 3]> {
        matches!(self.shape.kind, ShapeKind::ByteRegion(_))
            .then(|| std::array::from_fn(|index| format!("${}", self.fields[index])))
    }

    pub(super) fn region_kind(&self) -> Option<crate::types::ByteRegionKind> {
        match self.shape.kind {
            ShapeKind::ByteRegion(kind) => Some(kind),
            ShapeKind::Enum(_) => None,
        }
    }

    pub(super) fn present(&self) -> String {
        format!("${}", self.present)
    }

    pub(super) fn scalar_payload(
        &self,
        case: &str,
        index: usize,
        plan: &specialization::Plan,
    ) -> String {
        let ShapeKind::Enum(id) = self.shape.kind else {
            unreachable!("checked enum payload")
        };
        let definition = plan
            .enum_definition_by_id(id)
            .expect("checked enum payload");
        let case = definition
            .cases
            .iter()
            .position(|entry| entry.name == case)
            .expect("checked enum payload case");
        let payload = &self.shape.cases[case][index];
        assert!(
            payload.nested.is_none(),
            "intrinsic payload is a scalar leaf"
        );
        payload.value(&self.fields)
    }

    pub(super) fn read(&self) -> String {
        let arguments = std::iter::once(&self.tag)
            .chain(&self.fields)
            .map(|name| format!("${name}"))
            .collect::<Vec<_>>()
            .join(", ");
        let value = format!("{}({arguments})", boxer_name(&self.shape.name));
        if self.shape.nullable {
            format!("(${} ? {value} : null)", self.present)
        } else {
            value
        }
    }

    fn copy_from(&self, source: &Self, output: &mut String, indent: usize) {
        assert_eq!(self.shape.name, source.shape.name);
        for (target, source) in self.fields.iter().zip(&source.fields) {
            writeln(output, indent, &format!("${target} = ${source};"));
        }
        writeln(
            output,
            indent,
            &format!("${} = ${};", self.present, source.present),
        );
        writeln(output, indent, &format!("${} = ${};", self.tag, source.tag));
    }

    pub(super) fn absent(&self, output: &mut String, indent: usize) {
        for field in &self.fields {
            writeln(output, indent, &format!("${field} = null;"));
        }
        writeln(output, indent, &format!("${} = false;", self.present));
        writeln(output, indent, &format!("${} = -1;", self.tag));
    }

    pub(super) fn capture(&self, output: &mut String, indent: usize) {
        for (index, field) in self.fields.iter().enumerate() {
            writeln(
                output,
                indent,
                &format!("${field} = {RESULT}::$field{index};"),
            );
        }
        writeln(
            output,
            indent,
            &format!("${} = {RESULT}::$present;", self.present),
        );
        writeln(output, indent, &format!("${} = {RESULT}::$tag;", self.tag));
        writeln(output, indent, &format!("{RESULT}::clear();"));
    }

    pub(super) fn publish(&self, output: &mut String, indent: usize) {
        clear(output, indent);
        writeln(output, indent, &format!("if (${})", self.present));
        writeln(output, indent, "{");
        for (index, field) in self.fields.iter().enumerate() {
            writeln(
                output,
                indent + 1,
                &format!("{RESULT}::$field{index} = ${field};"),
            );
        }
        if matches!(self.shape.kind, ShapeKind::Enum(_)) {
            writeln(output, indent + 1, &format!("switch (${}) {{", self.tag));
            for tag in 0..self.shape.cases.len() {
                writeln(
                    output,
                    indent + 2,
                    &format!(
                        "case {tag}: {RESULT}::$fields = {}; break;",
                        self.shape.case_width(tag)
                    ),
                );
            }
            writeln(output, indent + 1, "}");
        } else {
            writeln(
                output,
                indent + 1,
                &format!("{RESULT}::$fields = {};", self.shape.fields),
            );
        }
        writeln(
            output,
            indent + 1,
            &format!("{RESULT}::$tag = ${};", self.tag),
        );
        writeln(
            output,
            indent + 1,
            &format!("{RESULT}::$type = {};", self.shape.result_type()),
        );
        writeln(output, indent + 1, &format!("{RESULT}::$present = true;"));
        writeln(output, indent, "}");
    }
}

fn boxer_name(name: &str) -> String {
    format!("__DoriaValueBox_{}", hex_bytes(name.as_bytes()))
}

pub(super) fn parameter_names(
    ty: &ResolvedType,
    index: usize,
    scopes: &PhpNameScopes,
) -> Vec<String> {
    let shape = Shape::for_type(ty, scopes).expect("checked field parameter type");
    component_names(&shape, index, &scopes.specialization)
}

fn component_names(shape: &Shape, index: usize, plan: &specialization::Plan) -> Vec<String> {
    ["tag".to_string(), "present".to_string()]
        .into_iter()
        .chain((0..shape.fields).map(|field| format!("field{field}")))
        .map(|component| plan.argument_parameter_name(index, &component))
        .collect()
}

pub(super) fn argument_values(
    expr: &Expr,
    ty: &ResolvedType,
    scopes: &PhpNameScopes,
) -> Option<Vec<String>> {
    let shape = Shape::for_type(ty, scopes)?;
    if let Some(place) = flat_place(expr, scopes) {
        if place.shape.name != shape.name {
            return None;
        }
        return Some(place.names().map(|name| format!("${name}")).collect());
    }
    match expr {
        Expr::Grouped { expr, .. } => argument_values(expr, ty, scopes),
        Expr::Null { .. } if shape.nullable => Some(
            std::iter::once("-1".to_string())
                .chain(std::iter::once("false".to_string()))
                .chain((0..shape.fields).map(|_| "null".to_string()))
                .collect(),
        ),
        _ => None,
    }
}

pub(super) fn constant_arguments(
    value: &ConstValue,
    ty: &ResolvedType,
    scopes: &PhpNameScopes,
) -> Vec<String> {
    let shape = Shape::for_type(ty, scopes).expect("checked field default type");
    constant_components(value, &shape, scopes)
}

fn constant_components(value: &ConstValue, shape: &Shape, scopes: &PhpNameScopes) -> Vec<String> {
    let (tag, present, fields) = match value {
        ConstValue::Null => {
            assert!(shape.nullable, "absent field default has a nullable type");
            (-1, false, Vec::new())
        }
        ConstValue::PayloadEnum(value) => {
            let ShapeKind::Enum(id) = shape.kind else {
                unreachable!("checked field enum")
            };
            let definition = scopes
                .specialization
                .enum_definition_by_id(id)
                .expect("checked field enum");
            let case = definition
                .resolve_typed_case(&value.value.enum_type, value.value.case)
                .expect("checked field enum constant belongs to its application");
            assert_eq!(
                value.fields.len(),
                definition.cases[case.index].payload.len()
            );
            (
                i64::from(definition.cases[case.index].tag),
                true,
                value
                    .fields
                    .iter()
                    .zip(&shape.cases[case.index])
                    .flat_map(|(field, payload)| {
                        payload.nested.as_ref().map_or_else(
                            || vec![emit_const_value(field, &scopes.specialization)],
                            |shape| constant_components(field, shape, scopes),
                        )
                    })
                    .collect(),
            )
        }
        _ => unreachable!("checked field default is an enum or null"),
    };
    std::iter::once(tag.to_string())
        .chain(std::iter::once(present.to_string()))
        .chain((0..shape.fields).map(|index| {
            fields
                .get(index)
                .cloned()
                .unwrap_or_else(|| "null".to_string())
        }))
        .collect()
}

fn flat_place<'a>(expr: &Expr, scopes: &'a PhpNameScopes) -> Option<&'a Place> {
    match expr {
        Expr::Grouped { expr, .. } => flat_place(expr, scopes),
        Expr::Variable { span, .. } => match scopes.place_for_use(*span)? {
            PhpBindingPlace::Fields(place) => Some(place),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn region_argument(
    expr: &Expr,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> Option<Place> {
    let shape = Shape::for_expr(expr, scopes)?;
    matches!(shape.kind, ShapeKind::ByteRegion(_))
        .then(|| Place::expression(expr, shape, output, indent, scopes))
}

pub(super) fn flat_region<'a>(expr: &Expr, scopes: &'a PhpNameScopes) -> Option<&'a Place> {
    flat_place(expr, scopes).filter(|place| place.region_kind().is_some())
}

pub(super) fn emit_region_runtime(output: &mut String, plan: &specialization::Plan) {
    for kind in [
        crate::types::ByteRegionKind::Readonly,
        crate::types::ByteRegionKind::Writable,
    ] {
        let shape = Shape::for_concrete_type(&ResolvedType::ByteRegion(kind), plan).unwrap();
        writeln(
            output,
            0,
            &format!(
                "function {}(int $tag, mixed $storage, mixed $offset, mixed $length): __DoriaByteRegion {{ return __DoriaByteRegion::__doriaFromFields($storage, $offset, $length, {}); }}",
                boxer_name(&shape.name),
                if kind.is_writable() { "true" } else { "false" },
            ),
        );
        storage::emit_shape_cell(shape, output, 0, plan);
    }
}

pub(super) fn prepare_comparison(
    expr: &Expr,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> Option<String> {
    let Expr::Binary {
        left, op, right, ..
    } = expr
    else {
        return None;
    };
    if !matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) {
        return None;
    }
    let left_shape = Shape::for_expr(left, scopes);
    let right_shape = Shape::for_expr(right, scopes);
    let shape = left_shape.as_ref().or(right_shape.as_ref())?.clone();
    if matches!(shape.kind, ShapeKind::ByteRegion(_))
        && ![left.as_ref(), right.as_ref()]
            .into_iter()
            .any(|expr| matches!(expression_region::ungrouped(expr), Expr::Null { .. }))
    {
        return None;
    }
    for (expr, candidate) in [
        (left.as_ref(), left_shape.as_ref()),
        (right.as_ref(), right_shape.as_ref()),
    ] {
        if let Some(candidate) = candidate {
            if candidate.name != shape.name {
                return None;
            }
        } else if !matches!(expression_region::ungrouped(expr), Expr::Null { .. }) {
            return None;
        }
    }
    let left = Place::expression(left, shape.clone(), output, indent, scopes);
    let right = Place::expression(right, shape, output, indent, scopes);
    let equal = if matches!(left.shape.kind, ShapeKind::ByteRegion(_)) {
        format!("(${} === ${})", left.present, right.present)
    } else {
        left.equality(&right)
    };
    Some(if *op == BinaryOp::NotEqual {
        format!("!{equal}")
    } else {
        equal
    })
}

pub(super) fn null_comparison(expr: &Expr, scopes: &PhpNameScopes) -> Option<String> {
    let Expr::Binary {
        left, op, right, ..
    } = expr
    else {
        return None;
    };
    if !matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) {
        return None;
    }
    let value = if matches!(expression_region::ungrouped(left), Expr::Null { .. }) {
        right
    } else if matches!(expression_region::ungrouped(right), Expr::Null { .. }) {
        left
    } else {
        return None;
    };
    Shape::for_expr(value, scopes)?;
    let present = if let Some(place) = flat_place(value, scopes) {
        format!("${}", place.present)
    } else {
        format!("{}->__doriaIsPresent()", storage::cell(value, scopes)?)
    };
    Some(if *op == BinaryOp::Equal {
        format!("!({present})")
    } else {
        present
    })
}

pub(super) fn emit_local(
    declaration: &VarDecl,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    let bindings = declaration
        .bindings
        .iter()
        .map(|binding| scopes.binding_for_declaration(&binding.name, binding.span))
        .collect::<Option<Vec<_>>>();
    let Some(bindings) = bindings else {
        return false;
    };
    if bindings.iter().any(|id| !scopes.owns_binding(*id)) {
        return false;
    }
    let Some(shape) = bindings
        .first()
        .and_then(|id| scopes.source_type(*id))
        .and_then(|ty| Shape::for_type(ty, scopes))
    else {
        return false;
    };
    let first = Place::new(shape.clone(), scopes);
    emit_into(&declaration.initializer, &first, output, indent, scopes);
    for (index, (binding, id)) in declaration.bindings.iter().zip(bindings).enumerate() {
        let mut place = if index == 0 {
            first.clone()
        } else {
            let place = Place::new(shape.clone(), scopes);
            place.copy_from(&first, output, indent);
            place
        };
        let name = scopes.declare(&binding.name);
        if scopes.needs_cell(id) {
            storage::initialize(&name, &place, output, indent);
            scopes.bind_place(id, PhpBindingPlace::Cell(name));
            continue;
        }
        writeln(output, indent, &format!("${name} = ${};", place.tag));
        place.tag = name;
        scopes.bind_place(id, PhpBindingPlace::Fields(place));
    }
    true
}

pub(super) fn emit_assignment(
    assignment: &Assignment,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    if assignment.op != AssignOp::Assign {
        return false;
    }
    if storage::cell(&assignment.target, scopes).is_some() {
        let name = scopes.fresh_temp("__doria_assignment_cell");
        scopes.track_expression_temporary(PhpExpressionTemporary::Alias(name.clone()));
        let render = |output: &mut String, indent, scopes: &mut PhpNameScopes| {
            let cell = storage::cell(&assignment.target, scopes).unwrap();
            expression_region::consume(output, indent, scopes);
            writeln(output, indent, &format!("${name} = {cell};"));
        };
        if let Expr::PropertyAccess { object, .. } =
            expression_region::ungrouped(&assignment.target)
        {
            expression_region::emit(object, output, indent, scopes, render);
        } else {
            render(output, indent, scopes);
        }
        let shape = Shape::for_expr(&assignment.target, scopes).unwrap();
        let staged = Place::new(shape, scopes);
        emit_into(&assignment.value, &staged, output, indent, scopes);
        staged.publish(output, indent);
        writeln(output, indent, &format!("${name}->__doriaStoreFields();"));
        return true;
    }
    let Some(target) = flat_place(&assignment.target, scopes).cloned() else {
        return emit_stable_assignment(assignment, output, indent, scopes);
    };
    // Acquire the replacement, including its finalizers, before committing.
    let staged = Place::new(target.shape.clone(), scopes);
    emit_into(&assignment.value, &staged, output, indent, scopes);
    target.copy_from(&staged, output, indent);
    true
}

fn emit_stable_assignment(
    assignment: &Assignment,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    let target = expression_region::ungrouped(&assignment.target);
    // A pure value cannot observe or replace the receiver, so the stored place
    // needs no statement staging; ordinary emission evaluates it once.
    if matches!(target, Expr::PropertyAccess { .. })
        && !assignment_target_needs_php_drop(target, scopes)
        && expression_region::simple_scalar(&assignment.value)
    {
        return false;
    }
    if let Some(destination) = assignment_place::stored_property(target, output, indent, scopes) {
        let owned = assignment_target_needs_php_drop(target, scopes);
        let value = stage_value(&assignment.value, owned, output, indent, scopes);
        if owned {
            let previous = scopes.fresh_temp("__doria_assignment_previous");
            scopes.track_expression_temporary(PhpExpressionTemporary::Alias(previous.clone()));
            writeln(
                output,
                indent,
                &format!("${previous} = {destination} ?? null;"),
            );
            writeln(output, indent, &format!("{destination} = {value};"));
            writeln(output, indent, &format!("unset({value});"));
            writeln(output, indent, &format!("__doria_drop_value(${previous});"));
        } else {
            writeln(output, indent, &format!("{destination} = {value};"));
        }
        return true;
    }
    if !matches!(target, Expr::Variable { .. } | Expr::StaticMember { .. }) {
        return false;
    }
    let owned = assignment_target_needs_php_drop(target, scopes);
    if !owned && !has_statement_value(&assignment.value, scopes) {
        return false;
    }
    let cell = owned
        .then(|| assignment_target_cell(target, scopes))
        .flatten();
    if owned && cell.is_none() {
        return false;
    }
    let destination = emit_assignment_target(target, scopes);
    let value = stage_value(&assignment.value, owned, output, indent, scopes);
    if let Some(cell) = cell {
        writeln(
            output,
            indent,
            &format!("__doria_replace_cell({cell}, {value});"),
        );
        writeln(output, indent, &format!("unset({value});"));
    } else {
        writeln(output, indent, &format!("{destination} = {value};"));
    }
    true
}

pub(super) fn assignment_expression(
    assignment: &Assignment,
    scopes: &PhpNameScopes,
) -> Option<String> {
    if flat_place(&assignment.target, scopes).is_none()
        && storage::cell(&assignment.target, scopes).is_none()
    {
        return None;
    }
    let mut body = String::new();
    let mut inner = scopes.clone();
    let temporaries = Rc::new(RefCell::new(Vec::new()));
    inner.expression_temporaries = Some(Rc::clone(&temporaries));
    if !emit_assignment(assignment, &mut body, 0, &mut inner) {
        return None;
    }
    body.push_str("return null;");
    let temporaries = temporaries.borrow();
    Some(emit_expression_body(scopes, &body, &temporaries))
}

pub(super) fn statement_value(
    expr: &Expr,
    owned: bool,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> Option<String> {
    if !has_statement_value(expr, scopes) {
        return None;
    }
    Some(stage_value(expr, owned, output, indent, scopes))
}

pub(super) fn stage_value(
    expr: &Expr,
    owned: bool,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> String {
    let name = scopes.fresh_temp("__doria_statement_value");
    emit_value_into(expr, &name, owned, output, indent, scopes);
    format!("${name}")
}

pub(super) fn statement_borrowed_cell(
    expr: &Expr,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> Option<String> {
    if !has_statement_value(expr, scopes) {
        return None;
    }
    let name = scopes.fresh_temp("__doria_statement_cell");
    let destination = Destination::BorrowedCell { name: name.clone() };
    if expression_region::is_root(expr, scopes) {
        destination.emit(expr, output, indent, scopes);
    } else {
        expression_region::emit(expr, output, indent, scopes, |output, indent, scopes| {
            destination.emit(expr, output, indent, scopes);
        });
    }
    Some(format!("${name}"))
}

pub(super) fn borrowed_cell_expression(expr: &Expr, scopes: &PhpNameScopes) -> Option<String> {
    if !has_statement_value(expr, scopes) {
        return None;
    }
    let mut inner = scopes.clone();
    let mut body = String::new();
    let value = statement_borrowed_cell(expr, &mut body, 0, &mut inner)?;
    writeln(&mut body, 0, &format!("return {value};"));
    Some(emit_expression_body(&inner, &body, &[]))
}

pub(super) fn emit_value_into(
    expr: &Expr,
    name: &str,
    owned: bool,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let destination = Destination::Value {
        name: name.to_string(),
        owned,
    };
    if expression_region::is_root(expr, scopes) {
        destination.emit(expr, output, indent, scopes);
    } else {
        expression_region::emit(expr, output, indent, scopes, |output, indent, scopes| {
            destination.emit(expr, output, indent, scopes);
        });
    }
}

pub(super) fn has_statement_value(expr: &Expr, scopes: &PhpNameScopes) -> bool {
    direct_match(expr, scopes) || direct_when(expr).is_some()
}

pub(super) fn has_field_value(expr: &Expr, scopes: &PhpNameScopes) -> bool {
    Shape::for_expr(expr, scopes).is_some()
}

pub(super) fn match_expression(expr: &Expr, owned: bool, scopes: &PhpNameScopes) -> Option<String> {
    if !direct_match(expr, scopes) {
        return None;
    }
    let mut inner = scopes.clone();
    inner.pending_returns.clear();
    let name = inner.fresh_temp("__doria_match_value");
    let mut body = String::new();
    emit_match(
        expr,
        &Destination::Value {
            name: name.clone(),
            owned,
        },
        &mut body,
        0,
        &mut inner,
    );
    writeln(&mut body, 0, &format!("return ${name};"));
    Some(emit_expression_body(scopes, &body, &[]))
}

fn convert_statement_value(expr: &Expr, name: &str, owned: bool, scopes: &PhpNameScopes) -> String {
    interface::convert_collection(
        expr,
        emit_mixed_box_plan(expr, format!("${name}"), scopes),
        owned,
        scopes,
    )
}

#[derive(Debug, Clone)]
enum Destination {
    Fields(Place),
    Value { name: String, owned: bool },
    BorrowedCell { name: String },
}

impl Destination {
    fn emit(&self, expr: &Expr, output: &mut String, indent: usize, scopes: &mut PhpNameScopes) {
        if let Some(when) = direct_when(expr) {
            emit_when(when, self, output, indent, scopes);
            if let Self::Value { name, owned } = self {
                Self::convert(expr, name, *owned, output, indent, scopes);
            } else if let Self::BorrowedCell { name } = self {
                Self::convert_cell(expr, name, output, indent, scopes);
            }
            return;
        }
        match self {
            Self::Fields(place) => emit_into(expr, place, output, indent, scopes),
            Self::Value { name, owned } => {
                if direct_match(expr, scopes) {
                    emit_match(expr, self, output, indent, scopes);
                    Self::convert(expr, name, *owned, output, indent, scopes);
                } else {
                    let value = if *owned {
                        emit_owned_expr(expr, scopes)
                    } else {
                        emit_expr(expr, scopes)
                    };
                    expression_region::consume(output, indent, scopes);
                    writeln(output, indent, &format!("${name} = {value};"));
                }
            }
            Self::BorrowedCell { name } => {
                if direct_match(expr, scopes) {
                    emit_match(expr, self, output, indent, scopes);
                    Self::convert_cell(expr, name, output, indent, scopes);
                } else {
                    let cell =
                        value_argument::Input::borrowed_cell(expr, scopes).expression(expr, scopes);
                    expression_region::consume(output, indent, scopes);
                    writeln(output, indent, &format!("${name} = {cell};"));
                }
            }
        }
    }

    fn convert_cell(
        expr: &Expr,
        name: &str,
        output: &mut String,
        indent: usize,
        scopes: &PhpNameScopes,
    ) {
        let source = format!("${name}->value");
        let value = interface::convert_collection(
            expr,
            emit_mixed_box_plan(expr, source.clone(), scopes),
            false,
            scopes,
        );
        if value != source {
            writeln(
                output,
                indent,
                &format!("${name} = new __DoriaCell({value});"),
            );
        }
    }

    fn convert(
        expr: &Expr,
        name: &str,
        owned: bool,
        output: &mut String,
        indent: usize,
        scopes: &PhpNameScopes,
    ) {
        let value = convert_statement_value(expr, name, owned, scopes);
        if value != format!("${name}") {
            writeln(output, indent, &format!("${name} = {value};"));
        }
    }
}

fn direct_when(expr: &Expr) -> Option<&WhenExpression> {
    match expr {
        Expr::When(when) => Some(when),
        Expr::Grouped { expr, .. } => direct_when(expr),
        _ => None,
    }
}

fn emit_when(
    when: &WhenExpression,
    destination: &Destination,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    scopes.whens.get(&when.span).expect("checked when result");
    let label = scopes.fresh_label("__doria_when_merge");
    let owned = matches!(destination, Destination::Value { owned: true, .. });
    if let Destination::Value { name, owned: true } = destination {
        // A repeated statement must not abandon an earlier transferred result.
        writeln(output, indent, &format!("${name} = null;"));
    }
    let mut inner = scoped_when(
        scopes,
        owned,
        matches!(destination, Destination::BorrowedCell { .. }),
    );
    inner.value_return = Some(Return {
        destination: destination.clone(),
        label: label.clone(),
    });
    emit_when_body(when, output, indent, &mut inner);
    writeln(output, indent, &format!("{label}:"));
}

fn direct_match(expr: &Expr, scopes: &PhpNameScopes) -> bool {
    matches!(expression_region::ungrouped(expr), Expr::Match { span, .. } if scopes.matches.contains_key(span))
}

fn field_match(expr: &Expr, scopes: &PhpNameScopes) -> bool {
    if let Expr::Grouped { expr, .. } = expr {
        return field_match(expr, scopes);
    }
    let Expr::Match {
        scrutinee, span, ..
    } = expr
    else {
        return false;
    };
    Shape::for_expr(scrutinee, scopes).is_some()
        && scopes.matches.get(span).is_some_and(|info| {
            info.arms.iter().all(|arm| {
                matches!(
                    arm.pattern,
                    ResolvedMatchPattern::EnumCase { .. }
                        | ResolvedMatchPattern::Null
                        | ResolvedMatchPattern::Default
                )
            })
        })
}

fn emit_match(
    expr: &Expr,
    destination: &Destination,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    if let Expr::Grouped { expr, .. } = expr {
        emit_match(expr, destination, output, indent, scopes);
        return;
    }
    if !field_match(expr, scopes) {
        emit_boxed_match(expr, destination, output, indent, scopes);
        return;
    }
    let Expr::Match {
        scrutinee,
        arms,
        span,
        ..
    } = expr
    else {
        unreachable!();
    };
    let info = scopes.matches[span].clone();
    let scrutinee_place = Place::new(Shape::for_expr(scrutinee, scopes).unwrap(), scopes);
    emit_into(scrutinee, &scrutinee_place, output, indent, scopes);
    let matched = scopes.fresh_temp("__doria_match_done");
    writeln(output, indent, &format!("${matched} = false;"));
    for (arm, facts) in arms.iter().zip(&info.arms) {
        let condition = match &facts.pattern {
            ResolvedMatchPattern::EnumCase { case_id, .. } => format!(
                "${} && ${} === {}",
                scrutinee_place.present, scrutinee_place.tag, scopes.payload_case_tags[case_id]
            ),
            ResolvedMatchPattern::Null => format!("!${}", scrutinee_place.present),
            ResolvedMatchPattern::Default => "true".into(),
            _ => unreachable!("direct match patterns are checked before emission"),
        };
        writeln(
            output,
            indent,
            &format!("if (!${matched} && ({condition}))"),
        );
        writeln(output, indent, "{");
        let mut arm_scopes = scopes.clone();
        arm_scopes.push();
        emit_owned_scope(
            output,
            indent + 1,
            &mut arm_scopes,
            |output, indent, arm_scopes| {
                if let MatchPattern::EnumCase {
                    bindings: Some(bindings),
                    ..
                } = &arm.pattern
                {
                    let ResolvedMatchPattern::EnumCase { case_id, .. } = &facts.pattern else {
                        unreachable!("checked field match has an enum case");
                    };
                    for (binding, field) in bindings
                        .iter()
                        .zip(&scrutinee_place.shape.cases[case_id.index])
                    {
                        let name = arm_scopes.declare(&binding.name);
                        if let Some(place) = field.place(&scrutinee_place.fields) {
                            let id = arm_scopes
                                .binding_for_declaration(&binding.name, binding.span)
                                .expect("checked nested payload binding");
                            if arm_scopes.needs_cell(id) {
                                storage::initialize(&name, &place, output, indent);
                                arm_scopes.bind_place(id, PhpBindingPlace::Cell(name));
                            } else {
                                arm_scopes.bind_place(id, PhpBindingPlace::Fields(place));
                            }
                            continue;
                        }
                        emit_php_match_binding(
                            output,
                            &name,
                            binding,
                            field.value(&scrutinee_place.fields),
                            false,
                            arm_scopes,
                        );
                    }
                }
                if let Some(guard) = &arm.guard {
                    let value =
                        expression_region::value(&guard.condition, output, indent, arm_scopes);
                    writeln(output, indent, &format!("if ({value})"));
                    writeln(output, indent, "{");
                }
                let body_indent = indent + usize::from(arm.guard.is_some());
                writeln(output, body_indent, &format!("${matched} = true;"));
                expression_region::emit(
                    &arm.value,
                    output,
                    body_indent,
                    arm_scopes,
                    |output, indent, scopes| destination.emit(&arm.value, output, indent, scopes),
                );
                if arm.guard.is_some() {
                    writeln(output, indent, "}");
                }
            },
        );
        writeln(output, indent, "}");
    }
}

fn emit_boxed_match(
    expr: &Expr,
    destination: &Destination,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let Expr::Match {
        scrutinee,
        arms,
        span,
        ..
    } = expr
    else {
        unreachable!()
    };
    let info = scopes.matches[span].clone();
    let consumed = matches!(info.mode, crate::ast::MatchMode::Consumed { .. });
    let drop_scrutinee = consumed && resolved_type_needs_php_drop(&info.scrutinee_type, scopes);
    let temporary = scopes.fresh_temp("__doria_match_subject");
    scopes.track_expression_temporary(PhpExpressionTemporary::Alias(temporary.clone()));
    expression_region::emit(
        scrutinee,
        output,
        indent,
        scopes,
        |output, indent, scopes| {
            let value = if consumed {
                emit_owned_expr(scrutinee, scopes)
            } else {
                emit_expr(scrutinee, scopes)
            };
            expression_region::consume(output, indent, scopes);
            writeln(output, indent, &format!("${temporary} = {value};"));
        },
    );
    if drop_scrutinee {
        writeln(output, indent, "try {");
    }
    let body_indent = indent + usize::from(drop_scrutinee);
    let matched = scopes.fresh_temp("__doria_match_done");
    writeln(output, body_indent, &format!("${matched} = false;"));
    for (index, (arm, facts)) in arms.iter().zip(&info.arms).enumerate() {
        writeln(output, body_indent, &format!("if (!${matched}) {{"));
        let mut arm_scopes = scopes.clone();
        arm_scopes.push();
        let condition = if index + 1 == arms.len() {
            None
        } else {
            Some(if facts.pattern == ResolvedMatchPattern::Condition {
                let MatchPattern::Expression(condition) = &arm.pattern else {
                    unreachable!()
                };
                let value =
                    expression_region::value(condition, output, body_indent + 1, &mut arm_scopes);
                format!("({value}) === true")
            } else {
                emit_php_match_condition(
                    &temporary,
                    &arm.pattern,
                    &facts.pattern,
                    &info.scrutinee_type,
                    &arm_scopes,
                )
                .expect("only the final checked match arm may be unconditional")
            })
        };
        if let Some(condition) = &condition {
            writeln(output, body_indent + 1, &format!("if ({condition}) {{"));
        }
        let arm_indent = body_indent + 1 + usize::from(condition.is_some());
        let guard_passed = if let Some(guard) = &arm.guard {
            let passed = arm_scopes.fresh_temp("__doria_match_guard");
            let mut guard_scopes = arm_scopes.clone();
            guard_scopes.push();
            emit_owned_scope(
                output,
                arm_indent,
                &mut guard_scopes,
                |output, indent, scopes| {
                    emit_php_match_bindings(
                        output,
                        &temporary,
                        &arm.pattern,
                        facts,
                        &info.scrutinee_type,
                        scopes,
                        false,
                        false,
                    );
                    let value = expression_region::value(&guard.condition, output, indent, scopes);
                    writeln(output, indent, &format!("${passed} = {value};"));
                },
            );
            writeln(output, arm_indent, &format!("if (${passed} === true) {{"));
            true
        } else {
            false
        };
        emit_owned_scope(
            output,
            arm_indent + usize::from(guard_passed),
            &mut arm_scopes,
            |output, indent, scopes| {
                emit_php_match_bindings(
                    output,
                    &temporary,
                    &arm.pattern,
                    facts,
                    &info.scrutinee_type,
                    scopes,
                    false,
                    consumed,
                );
                writeln(output, indent, &format!("${matched} = true;"));
                expression_region::emit(
                    &arm.value,
                    output,
                    indent,
                    scopes,
                    |output, indent, scopes| {
                        destination.emit(&arm.value, output, indent, scopes);
                    },
                );
            },
        );
        if guard_passed {
            writeln(output, arm_indent, "}");
        }
        if condition.is_some() {
            writeln(output, body_indent + 1, "}");
        }
        writeln(output, body_indent, "}");
    }
    if drop_scrutinee {
        writeln(output, indent, "} finally {");
        writeln(
            output,
            indent + 1,
            &format!("__doria_drop_value(${temporary});"),
        );
        writeln(output, indent, "}");
    }
}

fn emit_into(
    expr: &Expr,
    destination: &Place,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    if !expression_region::is_root(expr, scopes) {
        expression_region::emit(expr, output, indent, scopes, |output, indent, scopes| {
            emit_into(expr, destination, output, indent, scopes);
        });
        return;
    }
    if let Some(source) = flat_place(expr, scopes) {
        destination.copy_from(source, output, indent);
        return;
    }
    if storage::load(expr, destination, output, indent, scopes) {
        return;
    }
    if let Expr::Grouped { expr, .. } = expr {
        emit_into(expr, destination, output, indent, scopes);
        return;
    }
    if matches!(expr, Expr::Null { .. }) {
        destination.absent(output, indent);
        return;
    }
    if direct_match(expr, scopes) {
        emit_match(
            expr,
            &Destination::Fields(destination.clone()),
            output,
            indent,
            scopes,
        );
        return;
    }
    if let Expr::When(when) = expr {
        emit_when(
            when,
            &Destination::Fields(destination.clone()),
            output,
            indent,
            scopes,
        );
        return;
    }
    if field_call(expr, scopes) {
        let mut call_scopes = scopes.clone();
        call_scopes.field_call = Some(expr.span());
        writeln(output, indent, &format!("{RESULT}::clear();"));
        let value = emit_expr(expr, &call_scopes);
        expression_region::consume(output, indent, &call_scopes);
        writeln(output, indent, &format!("{value};"));
        destination.capture(output, indent);
        return;
    }
    if let Expr::Binary {
        left,
        op: BinaryOp::Coalesce,
        right,
        ..
    } = expr
    {
        if let Some(shape) = Shape::for_expr(left, scopes) {
            let left_place = Place::new(shape, scopes);
            emit_into(left, &left_place, output, indent, scopes);
            writeln(output, indent, &format!("if (${})", left_place.present));
            writeln(output, indent, "{");
            destination.copy_from(&left_place, output, indent + 1);
            writeln(output, indent, "}");
            writeln(output, indent, "else");
            writeln(output, indent, "{");
            emit_into(right, destination, output, indent + 1, scopes);
            writeln(output, indent, "}");
            return;
        }
    }
    if matches!(destination.shape.kind, ShapeKind::ByteRegion(_)) {
        if bytes::emit_fields(expr, destination, output, indent, scopes) {
            return;
        }
        let value = emit_expr(expr, scopes);
        expression_region::consume(output, indent, scopes);
        unpack(&value, destination, output, indent, scopes);
        return;
    }
    let ShapeKind::Enum(id) = destination.shape.kind else {
        unreachable!("checked enum destination")
    };
    let definition = scopes
        .specialization
        .enum_definition_by_id(id)
        .unwrap()
        .clone();
    let constructor = scopes
        .specialization
        .enum_case(expr.span())
        .filter(|case| definition.resolve_case(*case).is_some())
        .map(|case| {
            (
                case.index,
                match expr {
                    Expr::StaticCall { args, .. } => args.as_slice(),
                    Expr::StaticMember { .. } => &[][..],
                    _ => unreachable!("checked enum construction preserves case syntax"),
                },
            )
        });
    if let Some((tag, args)) = constructor {
        let case = &definition.cases[tag];
        let names = case
            .payload
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>();
        let written_names = args
            .iter()
            .map(|argument| argument.name.as_ref().map(|name| name.text.as_str()))
            .collect::<Vec<_>>();
        let bound =
            crate::arg_binding::bind_arguments(&names, &vec![false; names.len()], &written_names);
        let call = {
            let arguments = args
                .iter()
                .enumerate()
                .flat_map(|(written, argument)| {
                    let index = bound.arg_to_param[written].expect("checked enum argument binding");
                    let (names, values) = if destination.shape.cases[tag][index].nested.is_some() {
                        (
                            parameter_names(&case.payload[index].ty, index, scopes),
                            Place::argument(
                                &argument.value,
                                &case.payload[index].ty,
                                output,
                                indent,
                                scopes,
                            )
                            .names()
                            .map(|name| format!("${name}"))
                            .collect::<Vec<_>>(),
                        )
                    } else {
                        (
                            vec![case.payload[index].name.clone()],
                            vec![expression_region::value(
                                &argument.value,
                                output,
                                indent,
                                scopes,
                            )],
                        )
                    };
                    values
                        .into_iter()
                        .zip(names)
                        .map(|(value, name)| {
                            if argument.name.is_some() {
                                format!("{name}: {value}")
                            } else {
                                value
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{}({arguments})",
                writer_name(
                    &enum_shape_name(&definition, &scopes.specialization),
                    &definition.cases[tag].name
                )
            )
        };
        writeln(output, indent, &format!("{call};"));
        destination.capture(output, indent);
        return;
    }
    let value = emit_expr(expr, scopes);
    expression_region::consume(output, indent, scopes);
    unpack(&value, destination, output, indent, scopes);
}

fn unpack(
    value: &str,
    destination: &Place,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let boxed = scopes.fresh_temp("__doria_boxed_boundary");
    writeln(output, indent, &format!("${boxed} = {value};"));
    unpack_stable(&boxed, destination, output, indent, &scopes.specialization);
    writeln(output, indent, &format!("unset(${boxed});"));
}

pub(super) fn unpack_stable(
    boxed: &str,
    destination: &Place,
    output: &mut String,
    indent: usize,
    plan: &specialization::Plan,
) {
    unpack_value(&format!("${boxed}"), destination, output, indent, plan);
}

fn unpack_value(
    value: &str,
    destination: &Place,
    output: &mut String,
    indent: usize,
    plan: &specialization::Plan,
) {
    if matches!(destination.shape.kind, ShapeKind::ByteRegion(_)) {
        destination.absent(output, indent);
        writeln(output, indent, &format!("if ({value} !== null) {{"));
        for (field, getter) in
            destination
                .fields
                .iter()
                .zip(["__doriaRegionStorage", "__doriaRegionOffset", "count"])
        {
            let access = if getter == "count" {
                format!("count({value})")
            } else {
                format!("({value})->{getter}()")
            };
            writeln(output, indent + 1, &format!("${field} = {access};"));
        }
        writeln(
            output,
            indent + 1,
            &format!("${} = true;", destination.present),
        );
        writeln(output, indent + 1, &format!("${} = 0;", destination.tag));
        writeln(output, indent, "}");
        return;
    }
    let ShapeKind::Enum(id) = destination.shape.kind else {
        unreachable!("checked enum destination")
    };
    let definition = plan.enum_definition_by_id(id).unwrap();
    destination.absent(output, indent);
    writeln(output, indent, &format!("if ({value} !== null)"));
    writeln(output, indent, "{");
    writeln(
        output,
        indent + 1,
        &format!("${} = true;", destination.present),
    );
    for tag in 0..definition.cases.len() {
        writeln(
            output,
            indent + 1,
            &format!(
                "{} (({value})->__doriaMatchesCase({tag}))",
                if tag == 0 { "if" } else { "else if" }
            ),
        );
        writeln(output, indent + 1, "{");
        writeln(
            output,
            indent + 2,
            &format!("${} = {tag};", destination.tag),
        );
        for (index, field) in destination.shape.cases[tag].iter().enumerate() {
            let value = format!("({value})->__doriaPayloadAt({index})");
            if let Some(place) = field.place(&destination.fields) {
                unpack_value(&value, &place, output, indent + 2, plan);
                continue;
            }
            writeln(
                output,
                indent + 2,
                &format!("${} = {value};", destination.fields[field.offset]),
            );
        }
        writeln(output, indent + 1, "}");
    }
    writeln(output, indent, "}");
}

pub(super) fn writer_name(enum_name: &str, case: &str) -> String {
    format!(
        "__DoriaValueCase_{}_{}",
        hex_bytes(enum_name.as_bytes()),
        hex_bytes(case.as_bytes())
    )
}

fn definition<'a>(
    declaration: &EnumDecl,
    scopes: &'a PhpNameScopes,
) -> Option<&'a crate::semantics::EnumSemanticInfo> {
    let definition = instance_definition(declaration, scopes)
        .expect("checked payload enum has shared capability facts");
    definition.capabilities.copy.then_some(definition)
}

/// The specialization a declaration is being emitted for. A concrete
/// declaration has exactly one; a generic one is emitted per application.
pub(super) fn instance_definition<'a>(
    declaration: &EnumDecl,
    scopes: &'a PhpNameScopes,
) -> Option<&'a crate::semantics::EnumSemanticInfo> {
    match &scopes.enum_instance {
        Some(instance) => scopes.specialization.enum_definition_for_type(instance),
        None => scopes.specialization.enum_definition(&declaration.name),
    }
}

/// Layout, cell and boxer identity. Applications of one generic declaration
/// can have different payload layouts, so each has its own.
pub(super) fn enum_shape_name(
    definition: &crate::semantics::EnumSemanticInfo,
    plan: &specialization::Plan,
) -> String {
    if definition.source_type.arguments.is_empty() {
        definition.name.clone()
    } else {
        plan.enum_symbol(&definition.source_type)
    }
}

fn result_fields(
    definition: &crate::semantics::EnumSemanticInfo,
    plan: &specialization::Plan,
) -> usize {
    Shape::for_definition(definition, plan).map_or_else(
        || {
            definition
                .cases
                .iter()
                .map(|case| case.payload.len())
                .max()
                .unwrap_or(0)
        },
        |shape| shape.fields,
    )
}

pub(super) fn emit_runtime(program: &Program, output: &mut String, plan: &specialization::Plan) {
    let fields = program
        .semantic_info
        .enums
        .iter()
        .filter(|definition| definition.capabilities.copy)
        .map(|definition| result_fields(definition, plan))
        .filter(|fields| *fields != 0)
        .max()
        .unwrap_or(0)
        .max(REGION_FIELDS);
    writeln(output, 0, &format!("final class {RESULT}"));
    writeln(output, 0, "{");
    writeln(output, 1, "public static int $type = -1;");
    writeln(output, 1, "public static int $tag = -1;");
    writeln(output, 1, "public static bool $present = false;");
    writeln(output, 1, "public static int $fields = 0;");
    for index in 0..fields {
        writeln(
            output,
            1,
            &format!("public static mixed $field{index} = null;"),
        );
    }
    writeln(output, 1, "public static function clear(): void");
    writeln(output, 1, "{");
    writeln(output, 2, "self::$type = -1;");
    writeln(output, 2, "self::$tag = -1;");
    writeln(output, 2, "self::$present = false;");
    writeln(output, 2, "switch (self::$fields) {");
    for index in (0..fields).rev() {
        writeln(
            output,
            3,
            &format!("case {}: self::$field{index} = null;", index + 1),
        );
    }
    writeln(output, 2, "}");
    writeln(output, 2, "self::$fields = 0;");
    writeln(output, 1, "}");
    storage::emit_boxer(program, output, plan);
    writeln(output, 0, "}");
    output.push('\n');
}

pub(super) fn emit_boxed_factory(
    declaration: &EnumDecl,
    case: &EnumCaseDecl,
    tag: usize,
    output: &mut String,
    indent: usize,
    scopes: &PhpNameScopes,
) -> bool {
    let Some(definition) = definition(declaration, scopes) else {
        return false;
    };
    let arguments = case
        .payload
        .iter()
        .map(|field| format!("${}", field.name))
        .collect::<Vec<_>>()
        .join(", ");
    if definition.capabilities.trivial_copy {
        clear(output, indent);
        writeln(
            output,
            indent,
            &format!("return new self({tag}, [{arguments}]);"),
        );
        return true;
    }
    writeln(
        output,
        indent,
        &format!(
            "{}({arguments});",
            writer_name(
                &enum_shape_name(definition, &scopes.specialization),
                &case.name
            )
        ),
    );
    let fields = (0..case.payload.len())
        .map(|index| format!("{RESULT}::$field{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    writeln(
        output,
        indent,
        &format!("$result = new self({tag}, [{fields}]);"),
    );
    writeln(output, indent, &format!("{RESULT}::clear();"));
    writeln(output, indent, "return $result;");
    true
}

pub(super) fn emit_writers(
    declaration: &EnumDecl,
    output: &mut String,
    indent: usize,
    scopes: &PhpNameScopes,
) {
    let Some(definition) = definition(declaration, scopes) else {
        return;
    };
    let shape = Shape::for_definition(definition, &scopes.specialization);
    for (tag, case) in declaration.cases.iter().enumerate() {
        let mut inner = scopes.clone();
        for field in &case.payload {
            inner.declare_unmangled(&field.name);
        }
        let field_parameters = definition.cases[tag]
            .payload
            .iter()
            .enumerate()
            .map(|(index, field)| {
                shape
                    .as_ref()
                    .filter(|shape| shape.cases[tag][index].nested.is_some())
                    .map(|_| Place::parameter(&field.ty, index, &mut inner))
            })
            .collect::<Vec<_>>();
        let parameters = case
            .payload
            .iter()
            .enumerate()
            .flat_map(|(index, field)| {
                field_parameters[index].as_ref().map_or_else(
                    || vec![format!("{} ${}", php_type(&field.ty, scopes), field.name)],
                    Place::declarations,
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        output.push('\n');
        writeln(
            output,
            indent,
            &format!(
                "function {}({parameters}): void",
                writer_name(
                    &enum_shape_name(definition, &scopes.specialization),
                    &case.name
                )
            ),
        );
        writeln(output, indent, "{");
        if let Some(shape) = &shape {
            let place = Place::new(shape.clone(), &mut inner);
            place.absent(output, indent + 1);
            for (index, field) in case.payload.iter().enumerate() {
                let payload = &shape.cases[tag][index];
                if let Some(source) = &field_parameters[index] {
                    payload
                        .place(&place.fields)
                        .unwrap()
                        .copy_from(source, output, indent + 1);
                } else {
                    writeln(
                        output,
                        indent + 1,
                        &format!("${} = ${};", place.fields[payload.offset], field.name),
                    );
                }
            }
            writeln(output, indent + 1, &format!("${} = {tag};", place.tag));
            writeln(output, indent + 1, &format!("${} = true;", place.present));
            place.publish(output, indent + 1);
            writeln(output, indent, "}");
            continue;
        }
        writeln(output, indent + 1, &format!("{RESULT}::clear();"));
        for (index, field) in case.payload.iter().enumerate() {
            writeln(
                output,
                indent + 1,
                &format!("{RESULT}::$field{index} = ${};", field.name),
            );
        }
        writeln(output, indent + 1, &format!("{RESULT}::$tag = {tag};"));
        writeln(output, indent + 1, &format!("{RESULT}::$present = true;"));
        writeln(
            output,
            indent + 1,
            &format!("{RESULT}::$fields = {};", case.payload.len()),
        );
        writeln(
            output,
            indent + 1,
            &format!("{RESULT}::$type = {};", definition.id.0),
        );
        writeln(output, indent, "}");
    }
    if !definition.capabilities.trivial_copy {
        return;
    }
    let Some(shape) = shape else {
        return;
    };
    let fields = shape.fields;
    let parameters = (0..fields)
        .map(|index| format!("mixed $field{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    writeln(
        output,
        indent,
        &format!(
            "function {}(int $tag, {parameters}): {}",
            boxer_name(&enum_shape_name(definition, &scopes.specialization)),
            scopes.specialization.enum_symbol(&definition.source_type)
        ),
    );
    writeln(output, indent, "{");
    writeln(output, indent + 1, "return match ($tag) {");
    for (tag, case) in declaration.cases.iter().enumerate() {
        let names = (0..fields)
            .map(|index| format!("field{index}"))
            .collect::<Vec<_>>();
        let arguments = shape.cases[tag]
            .iter()
            .map(|payload| payload.value(&names))
            .collect::<Vec<_>>()
            .join(", ");
        writeln(
            output,
            indent + 2,
            &format!(
                "{tag} => {}::{}({arguments}),",
                scopes.specialization.enum_symbol(&definition.source_type),
                php_payload_case_method(&case.name, !case.payload.is_empty())
            ),
        );
    }
    writeln(output, indent + 1, "};");
    writeln(output, indent, "}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ClassType;
    use std::process::Command;

    #[test]
    fn enum_field_shapes_preserve_concrete_identity_and_emitted_names() {
        let mut program = crate::lower_source(
            "enum-instance-fields.doria",
            r#"
enum First { case Value(bool $value); }
enum Second { case Value(int $left, int $right); }
class Owner { function __construct() {} }
enum Third { case Value(Owner $owner); }
function main(): void {}
"#,
        )
        .unwrap();
        let declaration = crate::enums::EnumId(program.semantic_info.enums.len() + 1);
        let application =
            |ty| crate::enums::EnumType::with_arguments(declaration, "Packet", vec![ty]);
        // Exercise materialized metadata independently of source activation.
        for (name, argument) in [
            ("First", ResolvedType::Bool),
            ("Second", ResolvedType::Integer(IntegerType::Int64)),
            (
                "Third",
                ResolvedType::Class(ClassType::new("Owner", Vec::new())),
            ),
        ] {
            program
                .semantic_info
                .enums
                .iter_mut()
                .find(|definition| definition.name == name)
                .unwrap()
                .source_type = application(argument);
        }
        let closures = PhpClosurePlan::build(&program, None);
        let plan = specialization::Plan::build(&program, &closures).unwrap();
        let mut result_types = Vec::new();
        for (name, fields) in [("First", 1), ("Second", 2), ("Third", 0)] {
            let definition = plan.enum_definition(name).unwrap();
            let ty = ResolvedType::Enum(definition.source_type.clone());
            assert_eq!(
                plan.enum_definition_for_type(&definition.source_type),
                Some(definition)
            );
            assert_eq!(uses_fields(&ty, &program.semantic_info.enums), fields != 0);
            let shape = Shape::for_concrete_type(&ty, &plan);
            if fields == 0 {
                assert!(shape.is_none());
                continue;
            }
            let shape = shape.unwrap();
            assert_eq!(shape.name, plan.enum_symbol(&definition.source_type));
            assert_ne!(shape.name, name);
            assert_eq!(shape.fields, fields);
            assert_eq!(shape.result_type(), definition.id.0 as i64);
            result_types.push(shape.result_type());
            let nullable = Shape::for_concrete_type(&crate::types::nullable_of(ty), &plan).unwrap();
            assert!(nullable.nullable);
            assert_eq!(nullable.result_type(), shape.result_type());
        }
        assert_ne!(result_types[0], result_types[1]);
        let missing = application(ResolvedType::String);
        assert!(plan.enum_definition_for_type(&missing).is_none());
        let missing = ResolvedType::Enum(missing);
        assert!(!uses_fields(&missing, &program.semantic_info.enums));
        assert!(Shape::for_concrete_type(&missing, &plan).is_none());
    }

    #[test]
    fn scheduled_match_keeps_owned_arguments_until_statement_result_binding() {
        let source = r#"
enum Packet { case Number(int $count); case Empty; }
class Drops { static writable int $count = 0; }
class Watch { function __construct() {} function __destruct(): void { Drops::count++; } }
function number(Watch $source): int { return 7; }
function immediate(): int {
    Drops::count = 0;
    let $value = match (Packet::Number(number(new Watch()))) {
        Packet::Number($number) => $number + Drops::count * 100,
        Packet::Empty => 0,
    };
    return $value + Drops::count * 10;
}
function main(): int { return immediate(); }
"#;
        let mir = crate::lower_source_to_mir("match-statement-temporaries.doria", source).unwrap();
        let interpreted = crate::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(interpreted.exit_status, 17, "{interpreted:?}");
        assert!(interpreted.runtime_diagnostic.is_none());
        let php =
            crate::compile_source_to_php("match-statement-temporaries.doria", source).unwrap();
        let script = format!(
            "{}\nexit({}());",
            php.strip_prefix("<?php").unwrap(),
            php_function_name("main")
        );
        let result = php_execution::Script::new(&script)
            .command(&["-d", "ffi.enable=0"])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(17), "{result:?}\n{php}");
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
    }

    fn execute(source: &str, expected: i64, function: &str) -> String {
        let php = crate::compile_source_to_php("copy-enum-consumers.doria", source).unwrap();
        let script = format!(
            "{}\nif ({}() !== {expected}) {{ throw new LogicException('field-backed value changed'); }}\necho 'passed';",
            php.strip_prefix("<?php").unwrap(),
            php_function_name(function)
        );
        let result = php_execution::Script::new(&script)
            .command(&["-d", "ffi.enable=0"])
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}\n{php}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"passed");
        php
    }

    fn execute_main(source: &str, expected: &[u8]) -> String {
        let php = crate::compile_source_to_php("statement-when.doria", source).unwrap();
        let script = format!(
            "{}\n{}();",
            php.strip_prefix("<?php").unwrap(),
            php_function_name("main")
        );
        let result = php_execution::Script::new(&script)
            .command(&["-d", "ffi.enable=0"])
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}\n{php}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, expected);
        php
    }

    #[test]
    fn numeric_compound_matches_keep_places_and_read_before_rhs_mutation() {
        let source = r#"
enum Packet { case Number(int $count); case Empty; }
enum BytePacket { case Number(uint8 $count); }
class State { static writable int $order = 0; static writable int $slot = 10; }
class Counter {
    writable int $stored = 10;
    writable int $getters = 0;
    writable int $setters = 0;
    writable int $value {
        writable get { $this->getters++; State::order = State::order * 10 + 1; return $this->stored; }
        set (int $next) { $this->setters++; State::order = State::order * 10 + 3; $this->stored = $next; }
    }
}
function right(): int { State::order = State::order * 10 + 2; return 3; }
function changed(): int { State::slot = 100; return 3; }
function hot(): int {
    let writable $sum = 0;
    let writable $index = 0;
    while ($index < 1000) {
        $sum += ((match (Packet::Number($index)) {
            Packet::Number($count) => $count,
            Packet::Empty => -100,
        }));
        $index++;
    }
    return $sum;
}
function main(): void {
    let writable $local = 10;
    $local += when (true): int { $local = 100; return 3; } else { return 0; };
    writable float64 $real = 1.0;
    $real += match (Packet::Number(1)) { Packet::Number($count) => 2.0, Packet::Empty => -1.0, };
    let writable $counter = new Counter();
    State::order = 0;
    $counter->value += match (Packet::Number(1)) { Packet::Number($count) => right(), Packet::Empty => 0, };
    if (State::order != 123 || $counter->getters != 1 || $counter->setters != 1 || $counter->stored != 13) { panic("hook order"); }
    State::slot = 10;
    State::slot += match (Packet::Number(1)) { Packet::Number($count) => changed(), Packet::Empty => 0, };
    writable int[] $array = [10];
    $array[0] += match (Packet::Number(3)) { Packet::Number($count) => $count, Packet::Empty => 0, };
    writable List<int> $list = [10];
    $list[0] *= match (Packet::Number(3)) { Packet::Number($count) => $count, Packet::Empty => 0, };
    writable Bytes $bytes = Bytes::allocate(1);
    $bytes[0] = 10;
    uint8 $three = 3;
    $bytes[0] += match (BytePacket::Number($three)) { BytePacket::Number($count) => $count, };
    if ($local != 13 || $real != 3.0 || State::slot != 13 || $array[0] != 13 || $list[0] != 30 || $bytes[0] != 13) { panic("numeric places"); }
    echo hot();
}
"#;
        let mir = crate::lower_source_to_mir("numeric-match-statements.doria", source).unwrap();
        let interpreted = crate::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(interpreted.stdout, b"499500");
        assert!(interpreted.runtime_diagnostic.is_none(), "{interpreted:?}");
        let php = execute_main(source, b"499500");
        let body = php
            .split_once(&format!("function {}(", php_function_name("hot")))
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0;
        assert!(
            !body.contains("(function"),
            "numeric match created a closure: {body}"
        );
        assert!(body.contains("__doria_numeric_previous"));
    }

    #[test]
    fn stored_assignments_stage_values_and_commit_ownership_without_loop_wrappers() {
        let source = include_str!("../../tests/fixtures/stored_assignment_regions.doria");
        let expected = b"6;6;6;6;5;new 1;new 2;drop 1;fail;drop 2;new 3;fail;drop 3;";
        let php = execute_main(source, expected);
        let script = format!(
            r#"{}
$counts = new ReflectionProperty(__DoriaConstructorProbe::class, 'counts');
foreach ({:?} as $name) {{
    $function = '__DoriaFunction_' . bin2hex($name);
    $baseline = null;
    foreach ([0, 1, 1000] as $iterations) {{
        __DoriaConstructorProbe::reset();
        if ($function($iterations) !== intdiv($iterations * ($iterations - 1), 2)) {{
            throw new LogicException("stored assignment result changed: $name");
        }}
        $actual = $counts->getValue();
        if ($name === 'owned') {{
            if ({}() !== $iterations + 1 || __DoriaConstructorProbe::count('{}') !== $iterations + 1) {{
                throw new LogicException('explicit owner acquisition or release changed');
            }}
            unset($actual['{}']);
        }}
        ksort($actual); $baseline ??= $actual;
        if ($actual !== $baseline || __DoriaConstructorProbe::live() !== 0) {{
            throw new LogicException("stored assignment wrappers grew: $name at $iterations " . json_encode($actual));
        }}
    }}
    $tokens = __doria_test_loop_tokens({}, $name === 'hidden' ? 'exercise' : $function);
    foreach ([T_FUNCTION, T_FN] as $token) {{
        if (($tokens[$token] ?? 0) !== 0) {{ throw new LogicException('stored assignment loop allocated a closure'); }}
    }}
}}
echo 'stored assignments passed';
"#,
            php.strip_prefix("<?php").unwrap(),
            ["hot", "hidden", "generic", "owned"],
            php_function_name("drops"),
            php_type_symbol("Token"),
            php_type_symbol("Token"),
            emit_php_string_literal(&php),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"stored assignments passed");
        let mir = crate::lower_source_to_mir("stored-assignment-regions.doria", source)
            .unwrap_or_else(|errors| {
                panic!("PHP matched exact cleanup and flat allocation checks; native MIR rejected the same source: {errors:?}")
            });
        let interpreted = crate::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(interpreted.stdout, expected);
        assert!(interpreted.runtime_diagnostic.is_none(), "{interpreted:?}");
    }

    #[test]
    fn numeric_stored_updates_preserve_order_cleanup_and_flat_allocations() {
        let source = include_str!("../../tests/fixtures/numeric_stored_updates.doria");
        let mir = crate::lower_source_to_mir("numeric-stored-updates.doria", source).unwrap();
        let interpreted = crate::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(interpreted.stdout, b"4;4;4;13;1245;1245;15");
        assert!(interpreted.runtime_diagnostic.is_none(), "{interpreted:?}");
        let php = execute_main(source, b"4;4;4;13;1245;1245;15");
        let script = format!(
            r#"{}
$counts = new ReflectionProperty(__DoriaConstructorProbe::class, 'counts');
foreach ({:?} as $name) {{
    $function = '__DoriaFunction_' . bin2hex($name);
    $baseline = null;
    foreach ([0, 1, 1000] as $iterations) {{
        __DoriaConstructorProbe::reset();
        if ($function($iterations) !== $iterations) {{ throw new LogicException('stored update result changed'); }}
        $actual = $counts->getValue(); ksort($actual); $baseline ??= $actual;
        if ($actual !== $baseline || __DoriaConstructorProbe::live() !== 0) {{
            throw new LogicException("stored update allocation grew: $name at $iterations " . json_encode($actual));
        }}
    }}
    $tokens = __doria_test_loop_tokens({}, $name === 'hidden' ? 'exercise' : $function);
    foreach ([T_FUNCTION, T_FN, T_NEW] as $token) {{
        if (($tokens[$token] ?? 0) !== 0) {{ throw new LogicException('allocating stored update loop'); }}
    }}
}}
echo 'stored updates passed';
"#,
            php.strip_prefix("<?php").unwrap(),
            ["hot", "hidden", "forHeader"],
            emit_php_string_literal(&php),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"stored updates passed");
        for (body, code) in [
            ("make()->value++;", "E0204"),
            (
                "let writable $counter = make(); $counter->value += when (true): int { $counter->value = 100; return 3; } else { return 0; };",
                "E0477",
            ),
        ] {
            let source = format!(
                "class Counter {{ writable int $value = 10; }}\n\
                 function make(): Counter {{ return new Counter(); }}\n\
                 function main(): void {{ {body} }}"
            );
            let errors =
                crate::compile_source_to_php("numeric-stored-boundary.doria", &source).unwrap_err();
            assert!(errors.iter().any(|error| error.code == code), "{errors:?}");
        }
    }

    #[test]
    fn numeric_match_updates_release_operands_and_receivers_on_checked_failures() {
        let source = include_str!("../../tests/fixtures/numeric_match_cleanup.doria");
        execute_main(source, b"12345;1245;15");
        let mir = crate::lower_source_to_mir("numeric-match-cleanup.doria", source)
            .unwrap_or_else(|errors| {
                panic!("PHP matched exact cleanup output; native MIR rejected the same source: {errors:?}")
            });
        let interpreted = crate::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(interpreted.stdout, b"12345;1245;15");
        assert!(interpreted.runtime_diagnostic.is_none(), "{interpreted:?}");
    }

    fn execute_with_constructor_probe(source: &str) -> std::process::Output {
        let probe = include_str!("../../tests/fixtures/php_constructor_probe.php");
        let script = format!(
            "{}\n$source = {};\n$path = __DIR__ . DIRECTORY_SEPARATOR . 'instrumented.php';\nfile_put_contents($path, __doria_test_instrument_constructors($source));\nrequire $path;",
            probe.strip_prefix("<?php").unwrap(),
            emit_php_string_literal(&format!("<?php\n{source}")),
        );
        php_execution::Script::new(&script)
            .command(&["-d", "ffi.enable=0"])
            .output()
            .unwrap()
    }

    #[test]
    fn constructor_probe_counts_objects_once_and_preserves_weak_lifetimes() {
        let script = r#"
class ProbeParent {
    public function text(): string { $name = 'value'; return "{$name}}"; }
    public function __construct() {}
}
class ProbeChild extends ProbeParent {
    public function __construct() { parent::__construct(); }
}
class ProbeFailure {
    public function __construct() { throw new LogicException('construction failed'); }
}
interface ProbeContract { public function __construct(); }
function __construct(): int { return 7; }
__DoriaConstructorProbe::reset();
$parent = new ProbeParent();
if ($parent->text() !== 'value}' || __DoriaConstructorProbe::count(ProbeParent::class) !== 1) {
    throw new LogicException('interpolated text changed constructor instrumentation');
}
unset($parent);
__DoriaConstructorProbe::reset();
$value = new ProbeChild();
$reference = WeakReference::create($value);
if (__DoriaConstructorProbe::count(ProbeChild::class) !== 1
    || __DoriaConstructorProbe::count(ProbeParent::class) !== 0
    || __DoriaConstructorProbe::live() !== 1 || __construct() !== 7) {
    throw new LogicException('constructor phases were counted as separate objects');
}
unset($value);
if ($reference->get() !== null || __DoriaConstructorProbe::live() !== 0) {
    throw new LogicException('constructor probe retained an object');
}
try { new ProbeFailure(); } catch (LogicException $error) { unset($error); }
if (__DoriaConstructorProbe::count(ProbeFailure::class) !== 1
    || __DoriaConstructorProbe::live() !== 0) {
    throw new LogicException('failed construction was not counted or released');
}
$literal = 'function __construct() { new ProbeChild(); }';
$value = new ProbeChild();
unset($value);
if (__DoriaConstructorProbe::count(ProbeChild::class) !== 2
    || __DoriaConstructorProbe::live() !== 0) {
    throw new LogicException('constructor probe lost a later instance');
}
echo "constructor probe passed\n";
"#;
        let result = execute_with_constructor_probe(script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"constructor probe passed\n");
    }

    #[test]
    fn loop_probe_counts_body_and_condition_constructs_without_counting_other_functions() {
        let source = r#"<?php
function unrelated(): void { $value = new stdClass(); }
function withoutLoop(): void {}
function measured(): void {
    while ((fn(): bool => false)()) {
        $text = 'function fn new {';
        $block = function(): int { return 1; };
        $arrow = fn(): int => 2;
        $value = new stdClass();
    }
}
"#;
        let script = format!(
            r#"
$source = {};
$counts = __doria_test_loop_tokens($source, 'measured');
if (($counts[T_FUNCTION] ?? 0) !== 1 || ($counts[T_FN] ?? 0) !== 2 || ($counts[T_NEW] ?? 0) !== 1) {{
    throw new LogicException('loop token counts changed');
}}
$rejected = false;
try {{ __doria_test_loop_tokens($source, 'withoutLoop'); }}
catch (LogicException) {{ $rejected = true; }}
if (!$rejected) {{ throw new LogicException('loop probe escaped its function'); }}
echo "loop tokens passed\n";
"#,
            emit_php_string_literal(source),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"loop tokens passed\n");
    }

    #[test]
    fn field_result_loop_constructs_no_outcome_objects_with_a_measured_boxed_control() {
        let source = r#"
enum Packet { case Number(int $count); case Empty; }
function make(int $count): Packet { return Packet::Number($count); }
function hot(): int {
    let writable $index = 0;
    let writable $sum = 0;
    while ($index < 1000) {
        let $value = make($index);
        let $copy = $value;
        $sum += match ($copy) { Packet::Number($number) => $number, Packet::Empty => -1, };
        $index++;
    }
    return $sum;
}
function main(): void {}
"#;
        let php = crate::compile_source_to_php("measured-field-results.doria", source).unwrap();
        let script = format!(
            r#"{}
__DoriaConstructorProbe::reset();
if ({}() !== 499500 || __DoriaConstructorProbe::count('{}') !== 0
    || __DoriaConstructorProbe::count('__DoriaCell') !== 0) {{
    throw new LogicException('field result loop constructed transport objects');
}}
__DoriaConstructorProbe::reset();
for ($index = 0; $index < 1000; ++$index) {{
    $value = {}($index);
    if ($value->__doriaPayloadAt(0) !== $index) {{
        throw new LogicException('boxed control changed the outcome');
    }}
    unset($value);
}}
if (__DoriaConstructorProbe::count('{}') !== 1000
    || __DoriaConstructorProbe::live() !== 0) {{
    throw new LogicException('boxed control was not counted or released');
}}
echo "measured field results passed\n";
"#,
            php.strip_prefix("<?php").unwrap(),
            php_function_name("hot"),
            php_type_symbol("Packet"),
            php_function_name("make"),
            php_type_symbol("Packet"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"measured field results passed\n");
    }

    #[test]
    fn field_equality_preserves_active_payloads_nulls_order_and_lazy_operands() {
        let source = r#"
enum Leaf { case Empty; case Pair(float $value, int $stamp); }
enum Packet { case Empty; case Nested(?Leaf $value, int $stamp); }
class Trace { static writable string $log = ""; }
function probe(string $name, int $stamp): Packet {
    Trace::log = Trace::log . $name;
    return Packet::Nested(Leaf::Pair(1.5, $stamp), $stamp);
}
function sameFloat(float $value): bool { return Leaf::Pair($value, 1) == Leaf::Pair($value, 1); }
function hotEquality(int $iterations): void {
    let writable $index = 0;
    while ($index < $iterations) {
        Trace::log = "";
        if (probe("L", $index) != probe("R", $index) || Trace::log != "LR") { panic("equality evaluation order changed"); }
        if (false && probe("X", 0) == Packet::Empty) { panic("lazy and changed"); }
        if (!(true || probe("Y", 0) == Packet::Empty) || Trace::log != "LR") { panic("lazy or changed"); }
        writable ?Packet $optional = null;
        if ($optional != null || null != $optional) { panic("absent value changed"); }
        $optional = Packet::Nested(null, $index);
        if ($optional == null || null == $optional || $optional != Packet::Nested(null, $index)
            || $optional == Packet::Nested(Leaf::Empty, $index)) { panic("nested presence changed"); }
        $optional = Packet::Empty;
        if ($optional != Packet::Empty || $optional == Packet::Nested(null, 0)) { panic("inactive payload compared"); }
        $index++;
    }
}
function main(): void {}
"#;
        let php = crate::compile_source_to_php("measured-field-equality.doria", source).unwrap();
        let script = format!(
            r#"{}
foreach ([0, 1, 1000] as $iterations) {{
    __DoriaConstructorProbe::reset();
    {}($iterations);
    $counts = (new ReflectionProperty(__DoriaConstructorProbe::class, 'counts'))->getValue();
    if ($counts !== [] || __DoriaConstructorProbe::live() !== 0) {{
        throw new LogicException('field equality boxed values: ' . json_encode($counts));
    }}
}}
if ({}(NAN) || !{}(INF) || !{}(-0.0)) {{ throw new LogicException('floating equality changed'); }}
echo "measured field equality passed\n";
"#,
            php.strip_prefix("<?php").unwrap(),
            php_function_name("hotEquality"),
            php_function_name("sameFloat"),
            php_function_name("sameFloat"),
            php_function_name("sameFloat")
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"measured field equality passed\n");
    }

    #[test]
    fn field_arguments_construct_no_outcome_objects_across_callable_boundaries() {
        let source = include_str!("../../tests/fixtures/field_argument_transport.doria");
        let php = crate::compile_source_to_php("measured-field-arguments.doria", source).unwrap();
        let script = format!(
            r#"{}
$counter = new ReflectionProperty(__DoriaConstructorProbe::class, 'counts');
$loop = __doria_test_loop_tokens({}, '{}');
foreach ([T_FUNCTION, T_FN, T_NEW] as $token) {{
    if (($loop[$token] ?? 0) !== 0) {{ throw new LogicException('argument loop contains an allocating construct'); }}
}}
$observed = [];
foreach ({}::cases() as $route) {{
    $baseline = null;
    foreach ([0, 1, 1000] as $iterations) {{
        __DoriaConstructorProbe::reset();
        $actual = {}($iterations, $route);
        if ($actual !== intdiv($iterations * ($iterations - 1), 2)) {{
            throw new LogicException('argument transport changed the result');
        }}
        $observed[$route->name][$iterations] = __DoriaConstructorProbe::count('{}');
        $counts = $counter->getValue();
        ksort($counts);
        $baseline ??= $counts;
        if ($counts !== $baseline || __DoriaConstructorProbe::live() !== 0) {{
            throw new LogicException('argument transport allocated or retained objects: ' . $route->name . ' ' . json_encode($counts, JSON_THROW_ON_ERROR));
        }}
    }}
}}
$failures = array_filter($observed, static fn(array $counts): bool => array_sum($counts) !== 0);
if ($failures !== []) {{
    throw new LogicException('outcome objects at argument boundaries: ' . json_encode($failures, JSON_THROW_ON_ERROR));
}}
if ({}() !== 0 || {}(offset: 3) !== 3) {{
    throw new LogicException('boxed default arguments changed');
}}
$reader = new {}();
if ($reader->read() !== 0 || {}::readDefault(offset: 4) !== 4) {{
    throw new LogicException('boxed inherited or static defaults changed');
}}
unset($reader);
echo "measured field arguments passed\n";
"#,
            php.strip_prefix("<?php").unwrap(),
            emit_php_string_literal(&php),
            php_function_name("hot"),
            php_type_symbol("ArgumentRoute"),
            php_function_name("hot"),
            php_type_symbol("Packet"),
            php_function_name("consumeDefault"),
            php_function_name("consumeNullable"),
            php_type_symbol("ConcreteReader"),
            php_type_symbol("StaticReader"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"measured field arguments passed\n");
    }

    #[test]
    fn nested_copy_payload_transport_constructs_no_outcome_objects_and_preserves_boxed_snapshots() {
        let source = format!(
            "{}\nenum Collision {{ case Value(int $__doriaFieldParameter0_1_tag, Deep $nested); }}\nfunction collision(): Collision {{ return Collision::Value(5, make(7)); }}",
            include_str!("../../tests/fixtures/nested_copy_transport.doria"),
        );
        let php = crate::compile_source_to_php("nested-copy-transport.doria", &source).unwrap();
        let enums = format!(
            "[{}]",
            ["Leaf", "Envelope", "Deep"]
                .map(|name| emit_php_string_literal(&php_type_symbol(name)))
                .join(", "),
        );
        let script = format!(
            r#"{}
$counter = new ReflectionProperty(__DoriaConstructorProbe::class, 'counts');
$loop = __doria_test_loop_tokens({}, '{}');
foreach ([T_FUNCTION, T_FN, T_NEW] as $token) {{
    if (($loop[$token] ?? 0) !== 0) {{ throw new LogicException('nested loop contains an allocating construct'); }}
}}
foreach ({}::cases() as $route) {{
    $baseline = null;
    foreach ([0, 1, 1000] as $iterations) {{
        __DoriaConstructorProbe::reset();
        if ({}($iterations, $route) !== intdiv($iterations * ($iterations - 1), 2)) {{
            throw new LogicException('nested transport changed the result: ' . $route->name);
        }}
        $counts = $counter->getValue(); ksort($counts);
        $baseline ??= $counts;
        if ($counts !== $baseline || __DoriaConstructorProbe::live() !== 0) {{
            throw new LogicException('nested transport allocated or retained objects: ' . $route->name . ' ' . json_encode($counts, JSON_THROW_ON_ERROR));
        }}
    }}
}}
__DoriaConstructorProbe::reset();
$source = {}(7);
$home = new {}();
$home->value = $source;
$snapshot = $home->value;
$home->value = {}(9);
if ({}($source) !== 7 || {}($snapshot) !== 7 || {}($home->value) !== 9) {{
    throw new LogicException('boxed nested values lost Copy isolation');
}}
foreach ({} as $class) {{
    if (__DoriaConstructorProbe::count($class) === 0) {{ throw new LogicException('boxed positive control was not measured: ' . $class); }}
}}
$home->value = null;
if ({}($home->value) !== -1 || {}({}::{}()) !== -2) {{
    throw new LogicException('null and empty nested cases were conflated');
}}
$collision = {}();
if ($collision->__doriaPayloadAt(0) !== 5 || {}($collision->__doriaPayloadAt(1)) !== 7) {{
    throw new LogicException('flattened parameter names collided with authored payload names');
}}
__DoriaValueResult::clear();
unset($collision, $source, $snapshot, $home);
if (__DoriaConstructorProbe::live() !== 0) {{ throw new LogicException('boxed nested objects were retained'); }}
echo "measured nested transport passed\n";
"#,
            php.strip_prefix("<?php").unwrap(),
            emit_php_string_literal(&php),
            php_function_name("hot"),
            php_type_symbol("NestedRoute"),
            php_function_name("hot"),
            php_function_name("make"),
            php_type_symbol("Holder"),
            php_function_name("make"),
            php_function_name("number"),
            php_function_name("number"),
            php_function_name("number"),
            enums,
            php_function_name("number"),
            php_function_name("number"),
            php_type_symbol("Deep"),
            php_payload_case_method("Empty", false),
            php_function_name("collision"),
            php_function_name("number"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"measured nested transport passed\n");
    }

    #[test]
    fn callback_results_construct_no_outcome_objects_and_preserve_boxed_entries() {
        let source = include_str!("../../tests/fixtures/field_callback_results.doria");
        let php = crate::compile_source_to_php("measured-callback-results.doria", source).unwrap();
        let script = format!(
            r#"{}
$loop = __doria_test_loop_tokens({}, '{}');
foreach ([T_FUNCTION, T_FN, T_NEW] as $token) {{
    if (($loop[$token] ?? 0) !== 0) {{ throw new LogicException('callback loop contains an allocating construct'); }}
}}
$observed = [];
foreach ({}::cases() as $route) {{
    foreach ([0, 1, 1000] as $iterations) {{
        __DoriaConstructorProbe::reset();
        if ({}($iterations, $route) !== intdiv($iterations * ($iterations - 1), 2)) {{
            throw new LogicException('callback result transport changed the value');
        }}
        $observed[$route->name][$iterations] = __DoriaConstructorProbe::count('{}');
    }}
}}
$failures = array_filter($observed, static fn(array $counts): bool => array_sum($counts) !== 0);
if ($failures !== []) {{
    throw new LogicException('boxed callback results: ' . json_encode($failures, JSON_THROW_ON_ERROR));
}}
$callback = {}();
$combined = {}();
__DoriaConstructorProbe::reset();
for ($index = 0; $index < 1000; ++$index) {{
    $boxed = $callback($index);
    $fieldArguments = $combined->__doriaFieldArguments(0, true, $index, null);
    if ($boxed->__doriaPayloadAt(0) !== $index || $fieldArguments->__doriaPayloadAt(0) !== $index) {{
        throw new LogicException('boxed callback entry changed the value');
    }}
    unset($boxed, $fieldArguments);
}}
if (__DoriaConstructorProbe::count('{}') !== 2000 || __DoriaConstructorProbe::live() !== 0) {{
    throw new LogicException('boxed callback controls were not counted or released');
}}
$callback->__doriaDrop();
$combined->__doriaDrop();
$once = {}(21);
if ({}() !== -1) {{ throw new LogicException('once callback capture was released before invocation'); }}
__DoriaConstructorProbe::reset();
$boxed = $once();
if ($boxed->__doriaPayloadAt(0) !== 21 || {}() !== 21
    || __DoriaConstructorProbe::count('{}') !== 1) {{
    throw new LogicException('once callback boxed result was overwritten during cleanup');
}}
unset($boxed, $once);
if (__DoriaConstructorProbe::live() !== 0) {{
    throw new LogicException('once callback boxed result was not released');
}}
echo "measured callback results passed\n";
"#,
            php.strip_prefix("<?php").unwrap(),
            emit_php_string_literal(&php),
            php_function_name("hot"),
            php_type_symbol("CallbackResultRoute"),
            php_function_name("hot"),
            php_type_symbol("Packet"),
            php_function_name("resultCallback"),
            php_function_name("combineCallback"),
            php_type_symbol("Packet"),
            php_function_name("onceCallback"),
            php_function_name("lastCleanup"),
            php_function_name("lastCleanup"),
            php_type_symbol("Packet"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"drop 21;measured callback results passed\n");
    }

    #[test]
    fn copy_storage_reuses_stable_homes_and_preserves_boxed_borrows() {
        let source = include_str!("../../tests/fixtures/outcome_storage.doria");
        let php = crate::compile_source_to_php("measured-outcome-storage.doria", source).unwrap();
        let script = format!(
            r#"{}
$counts = new ReflectionProperty(__DoriaConstructorProbe::class, 'counts');
foreach ({:?} as $name) {{
    $function = '__DoriaFunction_' . bin2hex($name);
    $loop = __doria_test_loop_tokens({}, $function);
    foreach ([T_FUNCTION, T_FN, T_NEW] as $token) {{
        if (($loop[$token] ?? 0) !== 0) {{ throw new LogicException("allocating storage loop: $name"); }}
    }}
    $baseline = null;
    foreach ([0, 1, 1000] as $iterations) {{
        __DoriaConstructorProbe::reset();
        if ($function($iterations) !== intdiv($iterations * ($iterations - 1), 2)) {{
            throw new LogicException("storage value changed: $name");
        }}
        $actual = $counts->getValue(); ksort($actual);
        $baseline ??= $actual;
        // Invariant checks use the same field transport as loop consumers.
        if ($actual !== $baseline || __DoriaConstructorProbe::count('{}') !== 0
            || __DoriaConstructorProbe::live() !== 0) {{
            throw new LogicException("storage constructors grew: $name at $iterations " . json_encode($actual));
        }}
    }}
}}
$boxedSource = {}::Number(3);
$callback = {}($boxedSource);
__DoriaConstructorProbe::reset();
for ($index = 0; $index < 1000; ++$index) {{
    if ($callback($index) !== $index) {{ throw new LogicException('boxed capture factory changed the value'); }}
}}
if ($counts->getValue() !== [] || $boxedSource->__doriaPayloadAt(0) !== 3) {{
    throw new LogicException('boxed capture factory allocated per update or changed its source');
}}
$callback->__doriaDrop();
unset($callback, $boxedSource);
$home = new {}();
{}(7); $home->__doriaStoreFields();
$snapshot = $home->value;
$alias = $home;
{}(9); $alias->__doriaStoreFields();
if ($snapshot->__doriaPayloadAt(0) !== 7 || $home->value->__doriaPayloadAt(0) !== 9) {{
    throw new LogicException('field home lost identity or Copy isolation');
}}
$home->value = null;
$home->__doriaLoadFields();
if (__DoriaValueResult::$present) {{ throw new LogicException('nullable field home stayed present'); }}
__DoriaValueResult::clear();
$slot = $snapshot;
$borrow = __DoriaCell::borrowed($slot);
$borrow->__doriaLoadFields();
if (__DoriaValueResult::$field0 !== 7) {{ throw new LogicException('boxed borrow load changed'); }}
__DoriaValueResult::clear();
{}(11); $borrow->__doriaStoreFields();
if ($slot->__doriaPayloadAt(0) !== 11 || $snapshot->__doriaPayloadAt(0) !== 7) {{
    throw new LogicException('boxed borrow did not update its original slot');
}}
__doria_drop_cell($home);
__doria_drop_cell($borrow);
unset($home, $alias, $snapshot, $slot, $borrow);
if (__DoriaConstructorProbe::live() !== 0) {{ throw new LogicException('storage was retained'); }}
echo "measured storage passed\n";
"#,
            php.strip_prefix("<?php").unwrap(),
            [
                "localStorage",
                "parameterStorage",
                "capturedStorage",
                "callbackStorage",
                "ownedCaptureStorage",
                "propertyStorage"
            ],
            emit_php_string_literal(&php),
            php_type_symbol("Packet"),
            php_type_symbol("Packet"),
            php_function_name("ownedCapture"),
            storage::cell_name("Packet"),
            writer_name("Packet", "Number"),
            writer_name("Packet", "Number"),
            writer_name("Packet", "Number"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"measured storage passed\n");
    }

    #[test]
    fn copy_property_storage_preserves_checked_owners_and_boxed_inspection() {
        let source = include_str!("../../tests/fixtures/outcome_property_storage.doria");
        let php = crate::compile_source_to_php("outcome-property-storage.doria", source).unwrap();
        let script = format!(
            r#"{}
$counts = new ReflectionProperty(__DoriaConstructorProbe::class, 'counts');
foreach ({:?} as $name) {{
    $function = '__DoriaFunction_' . bin2hex($name);
    $tokens = __doria_test_loop_tokens({}, $name === 'internalStorage' ? 'exercise' : $function);
    foreach ([T_FUNCTION, T_FN, T_NEW] as $token) {{
        if (($tokens[$token] ?? 0) !== 0) {{ throw new LogicException("allocating property loop: $name"); }}
    }}
    $baseline = null;
    foreach ([0, 1, 1000] as $iterations) {{
        __DoriaConstructorProbe::reset();
        if ($function($iterations) !== intdiv($iterations * ($iterations - 1), 2)) {{
            throw new LogicException("property value changed: $name");
        }}
        $actual = $counts->getValue(); ksort($actual); $baseline ??= $actual;
        if ($actual !== $baseline || __DoriaConstructorProbe::live() !== 0) {{
            throw new LogicException("property constructors grew: $name at $iterations " . json_encode($actual));
        }}
    }}
}}
$holder = new {}();
$snapshot = $holder->value;
$holder->value = {}::Number(7);
if (!$snapshot->__doriaMatchesCase(1) || $holder->value->__doriaPayloadAt(0) !== 7) {{
    throw new LogicException('boxed property inspection lost Copy isolation');
}}
unset($snapshot, $holder);
if (__DoriaConstructorProbe::live() !== 0) {{ throw new LogicException('boxed property inspection retained storage'); }}
echo "measured property storage passed\n";
"#,
            php.strip_prefix("<?php").unwrap(),
            [
                "physicalStorage",
                "promotedStorage",
                "genericStorage",
                "nullableStorage",
                "callbackPropertyStorage",
                "internalStorage",
                "shadowedStorage",
                "hookedStorage",
                "genericHookedStorage",
                "virtualHookedStorage",
                "erasedHookedStorage",
                "nullSafeStorage",
                "nullableNullSafeStorage",
                "genericNullSafeStorage"
            ],
            emit_php_string_literal(&php),
            php_type_symbol("Plain"),
            php_type_symbol("Packet"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"measured property storage passed\n");
    }

    #[test]
    fn copy_enum_locals_assignments_and_guarded_matches_use_fields_without_boxing() {
        let php = execute(
            r#"
enum Packet { case Number(int $count); case Pair(int $left, int $right); case Empty; }
function hot(): int {
    let writable $index = 0;
    let writable $total = 0;
    while ($index < 1000) {
        Packet $one, $saved = Packet::Number($index);
        let writable $value = $one;
        $value = Packet::Pair(right: 1, left: $index);
        let $current = match ($value) {
            Packet::Pair($left, $right) if $right == 0 => -100,
            Packet::Pair($left, $right) => $left + $right,
            Packet::Number($number) => $number,
            Packet::Empty => -200,
        };
        let $previous = match ($saved) {
            Packet::Number($number) => $number,
            Packet::Pair($left, $right) => -300,
            Packet::Empty => -400,
        };
        $total += $current + $previous;
        $index++;
    }
    return $total;
}
function main(): void { hot(); }
"#,
            1_000_000,
            "hot",
        );
        let function = php
            .split_once(&format!("function {}(", php_function_name("hot")))
            .unwrap()
            .1;
        let body = function.split_once("\n}\n").unwrap().0;
        for boxed in [
            "new ",
            "(function($__doriaMatch",
            "::Number(",
            "::Pair(",
            "__DoriaValueBox_",
        ] {
            assert!(
                !body.contains(boxed),
                "unexpected boxed consumer {boxed}: {body}"
            );
        }
        assert!(body.contains("__doria_match_done"));
        assert!(body.contains(&writer_name("Packet", "Pair")));
    }

    #[test]
    fn copy_enum_nullable_call_return_error_finalizer_and_for_boundaries_preserve_values() {
        let source = r#"
enum Packet { case Number(int $count); case Pair(int $left, int $right); case Empty; }
class Failure implements Error { function __construct(string $message) {} }
class State { static writable int $order = 0; }
function argument(int $value): int { State::order = State::order * 10 + $value; return $value; }
function identity(Packet $value): Packet { return $value; }
function failed(): Packet throws Failure { throw new Failure("failed"); }
function replace(writable Packet $value): void { $value = Packet::Number(13); }
function inspect(?Packet $value): int {
    return match ($value) {
        null => -1,
        Packet::Number($number) => $number,
        Packet::Pair($left, $right) => $left + $right,
        Packet::Empty => -2,
    };
}
function finalized(): Packet {
    try { return Packet::Number(7); }
    finally { Packet $noise = Packet::Pair(8, 9); }
}
function boundaries(): int {
    State::order = 0;
    ?Packet $absent = null;
    Packet $zero = $absent ?? Packet::Number(0);
    let $saved = $zero;
    let writable $changing = $zero;
    $changing = Packet::Number(9);
    try { $changing = failed(); } catch (Failure) {}
    ?Packet $lifted = $saved;
    Packet $fromCall = identity($changing);
    Packet $returned = finalized();
    Packet $ordered = Packet::Pair(right: argument(2), left: argument(1));
    mixed $dynamic = match ($ordered) {
        Packet::Pair($left, $right) => $left + $right,
        Packet::Number($number) => $number,
        Packet::Empty => 0,
    };
    let $unboxed = match ($dynamic) { int $number => $number, default => -100, };
    let writable $borrowed = Packet::Number(4);
    replace($borrowed);
    let writable $captured = Packet::Number(2);
    let writable $edit = function (): void with (writable $captured) { $captured = Packet::Number(17); };
    $edit();
    let writable $counter = 0;
    let writable $loop = Packet::Empty;
    for (; $counter < 5; $loop = Packet::Number($counter)) { $counter++; continue; }
    return inspect($lifted) + inspect($fromCall) + inspect($returned) + inspect($ordered)
        + inspect($loop) + State::order + $unboxed + inspect($borrowed) + inspect($captured);
}
function main(): void { echo boundaries(); }
"#;
        let mir = crate::lower_source_to_mir("copy-enum-boundaries.doria", source).unwrap();
        let expected = crate::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(expected.stdout, b"69");
        assert!(expected.runtime_diagnostic.is_none(), "{expected:?}");
        execute(source, 69, "boundaries");
    }

    #[test]
    fn expression_matches_keep_owners_until_the_enclosing_statement_finishes() {
        let source = r#"
enum Packet { case Number(int $count); case Empty; }
class Drops { static writable int $count = 0; }
class Watch { function __construct() {} function __destruct(): void { Drops::count++; } }
function number(Watch $source): int { return 7; }
function embedded(): int {
    Drops::count = 0;
    let writable $value = 1;
    $value += match (Packet::Number(number(new Watch()))) {
        Packet::Number($number) => $number + Drops::count * 100,
        Packet::Empty => 0,
    };
    return $value + Drops::count * 10;
}
function main(): int { return embedded(); }
"#;
        let mir = crate::lower_source_to_mir("expression-match-temporaries.doria", source).unwrap();
        let interpreted = crate::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(interpreted.exit_status, 18, "{interpreted:?}");
        assert!(interpreted.runtime_diagnostic.is_none());
        execute(source, 18, "embedded");
    }

    #[test]
    fn inline_match_guards_use_statement_regions_without_expression_closures() {
        let source = r#"
enum Packet { case Number(int $count); case Empty; }
class State { static writable int $trace = 0; static writable int $drops = 0; }
class Failure implements Error { function __construct(string $message) {} }
class Token { function __destruct(): void { State::drops++; } }
function gate(int $digit, bool $accept): bool {
    State::trace = State::trace * 10 + $digit;
    return $accept;
}
function reject(Token $token): bool { return false; }
function fail(Token $token): bool throws Failure { throw new Failure("guard"); }
function hot(int $iterations): int {
    let writable $sum = 0;
    foreach (0..<$iterations as int $index) {
        State::trace = 0;
        let $packet = Packet::Number(7);
        let $value = match ($packet) {
            Packet::Empty if gate(9, true) => -100,
            Packet::Number($count) if gate(1, false) => -200,
            Packet::Number($count) if gate(2, true) => $count,
            default => -300,
        };
        ?Packet $absent = null;
        let $zero = match ($absent) {
            Packet::Number($count) if gate(9, true) => -400,
            null if gate(3, true) => 0,
            default => -500,
        };
        if (State::trace != 123) { panic("guard evaluation order changed"); }
        $sum += $value + $zero;
    }
    return $sum;
}
function cleanup(): int {
    State::drops = 0;
    let $value = match (Packet::Number(9)) {
        Packet::Number($count) if reject(new Token()) => -1,
        default => 2,
    };
    if (State::drops != 1) { panic("false guard temporary was retained"); }
    try {
        let $unused = match (Packet::Number(0)) {
            Packet::Number($count) if fail(new Token()) => $count,
            default => -1,
        };
        panic("guard error was lost");
    } catch (Failure) {}
    return State::drops * 10 + $value;
}
function main(): void {}
"#;
        let php = crate::compile_source_to_php("inline-guard-regions.doria", source).unwrap();
        let script = format!(
            r#"{}
foreach ([0, 1, 1000] as $iterations) {{
    if ({}($iterations) !== $iterations * 7) {{
        throw new LogicException('inline match guard value changed');
    }}
}}
if ({}() !== 22) {{ throw new LogicException('guard temporary cleanup changed'); }}
echo 'functional guards;';
$tokens = __doria_test_loop_tokens(file_get_contents(__FILE__), '{}');
foreach ([T_FUNCTION, T_FN] as $token) {{
    if (($tokens[$token] ?? 0) !== 0) {{
        throw new LogicException('inline match guard allocated an expression closure');
    }}
}}
echo 'guard regions';
"#,
            php.strip_prefix("<?php").unwrap(),
            php_function_name("hot"),
            php_function_name("cleanup"),
            php_function_name("hot"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"functional guards;guard regions");
    }

    #[test]
    fn prepared_control_values_preserve_lazy_regions_without_expression_closures() {
        let source = r#"
enum Packet { case Number(int $count); case Empty; }
class State { static writable int $trace = 0; static writable int $drops = 0; }
class Failure implements Error { function __construct(string $message) {} }
class Token { function __destruct(): void { State::drops++; } }
function gate(int $digit, bool $accept): bool {
    State::trace = State::trace * 10 + $digit;
    return $accept;
}
function reject(Token $token): bool { return false; }
function fail(Token $token): bool throws Failure { throw new Failure("condition"); }
function hot(int $iterations): int {
    let writable $sum = 0;
    foreach (0..<$iterations as int $index) {
        State::trace = 0;
        if ((match (Packet::Number(3)) {
            Packet::Number($count) if gate(1, true) => $count == 3,
            default => false,
        }) && (when (gate(2, true)): bool {
            return gate(3, true);
        } else {
            return false;
        } finally {
            State::trace = State::trace * 10 + 4;
        })) { $sum++; }
        if (false && (match (true) { true => gate(9, true), default => false, })) {
            panic("lazy match operand ran");
        }
        if (!(true || (when (true): bool { return gate(9, true); } else { return false; }))) {
            panic("lazy when operand changed");
        }
        if (State::trace != 1234) { panic("condition evaluation order changed"); }
        let writable $remaining = 1;
        while (match ($remaining) { 1 => true, default => false, }) { $remaining--; }
        do { $remaining++; } while (when (false): bool { return true; } else { return false; });
        $sum += $remaining;
    }
    return $sum;
}
function cleanup(): int {
    State::drops = 0;
    if (when (true): bool { return reject(new Token()); } else { return true; }) {
        panic("false condition changed");
    }
    if (State::drops != 1) { panic("condition temporary was retained"); }
    try {
        if (match (true) { true => fail(new Token()), default => false, }) {
            panic("throwing condition returned");
        }
    } catch (Failure) {}
    return State::drops;
}
function loopExits(): int {
    State::drops = 0;
    let writable $steps = 0;
    let writable $checks = 0;
    do {
        let $token = new Token();
        $steps++;
        if ($steps == 1) { continue; }
        if ($steps == 3) { break; }
    } while (when (true): bool {
        if (State::drops != $steps) { panic("loop body cleanup followed its condition"); }
        $checks++;
        return true;
    } else { return false; });
    return $steps * 100 + $checks * 10 + State::drops;
}
function conditionFailure(): int {
    State::drops = 0;
    try {
        do { let $token = new Token(); }
        while (match (true) { true => fail(new Token()), default => false, });
    } catch (Failure) {}
    return State::drops;
}
function main(): void {}
"#;
        let php = crate::compile_source_to_php("prepared-control-values.doria", source).unwrap();
        let script = format!(
            r#"{}
foreach ([0, 1, 1000] as $iterations) {{
    if ({}($iterations) !== $iterations * 2) {{
        throw new LogicException('prepared condition result changed');
    }}
}}
if ({}() !== 2) {{ throw new LogicException('condition temporary cleanup changed'); }}
if ({}() !== 323) {{ throw new LogicException('do-loop condition or cleanup order changed'); }}
if ({}() !== 2) {{ throw new LogicException('do-loop throwing condition cleanup changed'); }}
echo 'functional conditions;';
foreach (['{}', '{}'] as $function) {{
    $tokens = __doria_test_loop_tokens(file_get_contents(__FILE__), $function);
    foreach ([T_FUNCTION, T_FN] as $token) {{
        if (($tokens[$token] ?? 0) !== 0) {{
            throw new LogicException('prepared control value allocated an expression closure');
        }}
    }}
}}
echo 'statement conditions';
"#,
            php.strip_prefix("<?php").unwrap(),
            php_function_name("hot"),
            php_function_name("cleanup"),
            php_function_name("loopExits"),
            php_function_name("conditionFailure"),
            php_function_name("hot"),
            php_function_name("loopExits"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"functional conditions;statement conditions");
    }

    #[test]
    fn for_increments_use_statement_destinations_and_preserve_exit_ownership() {
        let source = r#"
enum Step { case Next(int $value); }
class Trace {
    static writable int $bodyDrops = 0;
    static writable int $increments = 0;
    static writable int $position = 0;
    static writable int $ownedDrops = 0;
}
class Guard { function __destruct(): void { Trace::bodyDrops++; } }
class Product {
    function __construct(int $value) {}
    function __destruct(): void { Trace::ownedDrops++; }
}
class Failure implements Error { function __construct(string $message) {} }
function hot(int $iterations): int {
    let writable $sum = 0;
    foreach (0..<$iterations as int $repeat) {
        Trace::bodyDrops = 0;
        Trace::increments = 0;
        let writable $index = 0;
        for (; $index < 4; $index += when (true): int {
            Trace::increments++;
            if (Trace::bodyDrops != Trace::increments) { panic("increment preceded body cleanup"); }
            return match (Step::Next(1)) { Step::Next($value) => $value, };
        } else { return 0; }) {
            let $guard = new Guard();
            if ($index == 1) { continue; }
            if ($index == 3) { break; }
            $sum += $index;
        }
        if ($index != 3 || Trace::increments != 3 || Trace::bodyDrops != 4) {
            panic("for exit order changed");
        }
        for (; false; $index += when (true): int {
            Trace::increments++;
            return 1;
        } else { return 0; }) { panic("initial condition was skipped"); }
        let writable $slot = 0;
        for (; $slot < 3; $slot = match (Step::Next($slot + 1)) {
            Step::Next($value) => $value,
        }) { $sum++; }
        Trace::position = 0;
        for (; Trace::position < 2; Trace::position = when (true): int {
            return Trace::position + 1;
        } else { return 0; }) { $sum++; }
        if (Trace::increments != 3) { panic("zero-iteration increment ran"); }
    }
    return $sum;
}
function owners(): int {
    Trace::ownedDrops = 0;
    let writable $owner = new Product(0);
    let writable $index = 0;
    for (; $index < 2; $owner = when (true): Product {
        $index++;
        return new Product($index);
    } else { return new Product(9); }) {
        if ($index == 1) { break; }
    }
    return $owner->value * 10 + Trace::ownedDrops;
}
function failed(): int {
    Trace::ownedDrops = 0;
    let writable $owner = new Product(7);
    try {
        for (; true; $owner = when (true): Product {
            return new Product(8);
        } else { return new Product(9); } finally {
            throw when (true): Failure {
                return new Failure("increment");
            } else { return new Failure("unused"); };
        }) {}
    } catch (Failure) {}
    return $owner->value * 10 + Trace::ownedDrops;
}
function ownership(): int {
    let $first = owners();
    if ($first != 11 || Trace::ownedDrops != 2) { panic("replacement ownership changed"); }
    let $second = failed();
    if ($second != 71 || Trace::ownedDrops != 2) { panic("failed replacement lost its owner"); }
    return 1;
}
function main(): void {}
"#;
        let php = crate::compile_source_to_php("for-increment-statements.doria", source).unwrap();
        let script = format!(
            r#"{}
foreach ([0, 1, 1000] as $iterations) {{
    if ({}($iterations) !== $iterations * 7) {{
        throw new LogicException('for increment result changed');
    }}
}}
if ({}() !== 1) {{ throw new LogicException('for increment ownership changed'); }}
echo 'functional increments;';
foreach (['{}', '{}', '{}'] as $function) {{
    $tokens = __doria_test_loop_tokens(file_get_contents(__FILE__), $function);
    foreach ([T_FUNCTION, T_FN] as $token) {{
        if (($tokens[$token] ?? 0) !== 0) {{
            throw new LogicException("for increment allocated an expression closure in $function");
        }}
    }}
}}
echo 'statement increments';
"#,
            php.strip_prefix("<?php").unwrap(),
            php_function_name("hot"),
            php_function_name("ownership"),
            php_function_name("hot"),
            php_function_name("owners"),
            php_function_name("failed"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"functional increments;statement increments");
    }

    #[test]
    fn conditional_call_values_preserve_order_null_gates_and_pending_ownership() {
        let source = r#"
enum Step { case Number(int $value); }
class Trace {
    static writable int $order = 0;
    static writable int $drops = 0;
    static writable int $calls = 0;
}
class Product {
    function __construct(int $value) {}
    function __destruct(): void { Trace::drops++; }
}
class Failure implements Error { function __construct(string $message) {} }
function mark(int $digit): int { Trace::order = Trace::order * 10 + $digit; return $digit; }
function pair(int $left, int $right): int { return $left * 10 + $right; }
class Calculator {
    static function pair(int $left, int $right): int { return pair($left, $right); }
}
class Sink {
    writable int $stored = 0;
    writable int $writes = 0;
    writable int $value {
        get { return $this->stored; }
        set (int $next) { $this->stored = $next; $this->writes++; }
    }
    function pair(int $left, int $right): int { return pair($left, $right); }
}
function hot(int $iterations): int {
    let writable $sum = 0;
    let writable $sink = new Sink();
    ?Sink $absent = null;
    let $callback = fn(int $number) => $number + 1;
    foreach (0..<$iterations as int $index) {
        Trace::order = 0;
        let $first = pair(
            right: when (true): int { return mark(2); } else { return 0; },
            left: match (Step::Number(1)) { Step::Number($number) => mark($number), },
        );
        if (Trace::order != 21) { panic("free argument order changed"); }
        Trace::order = 0;
        let $second = Calculator::pair(
            when (true): int { return mark(3); } else { return 0; },
            match (Step::Number(4)) { Step::Number($number) => mark($number), },
        );
        if (Trace::order != 34) { panic("static argument order changed"); }
        Trace::order = 0;
        let $third = $sink->pair(
            right: match (Step::Number(6)) { Step::Number($number) => mark($number), },
            left: when (true): int { return mark(5); } else { return 0; },
        );
        let $fourth = $callback(when (true): int { return 7; } else { return 0; });
        $sink->value = match (Step::Number(9)) { Step::Number($number) => mark($number), };
        $absent?->pair(when (true): int { return mark(8); } else { return 0; }, 0);
        if (Trace::order != 659) { panic("method, setter or null gate changed"); }
        $sum += $first + $second + $third + $fourth + $sink->value;
    }
    if ($sink->writes != $iterations) { panic("setter invocation count changed"); }
    return $sum;
}
function consume(take Product $owner, int $later): int {
    Trace::calls++;
    return $owner->value + $later;
}
function fail(): int throws Failure { throw new Failure("argument"); }
function cleanup(): int {
    Trace::drops = 0;
    Trace::calls = 0;
    try {
        let $unused = consume(
            when (true): Product { return new Product(7); } else { return new Product(9); },
            fail(),
        );
    } catch (Failure) {}
    if (Trace::calls != 0) { panic("callee ran before its arguments completed"); }
    return Trace::drops;
}
function main(): void {}
"#;
        let php = crate::compile_source_to_php("conditional-call-values.doria", source).unwrap();
        let script = format!(
            r#"{}
foreach ([0, 1, 1000] as $iterations) {{
    if ({}($iterations) !== $iterations * 119) {{
        throw new LogicException('conditional call result changed');
    }}
}}
if ({}() !== 1) {{ throw new LogicException('pending conditional argument cleanup changed'); }}
echo 'functional arguments;';
$tokens = __doria_test_loop_tokens(file_get_contents(__FILE__), '{}');
foreach ([T_FUNCTION, T_FN] as $token) {{
    if (($tokens[$token] ?? 0) !== 0) {{
        throw new LogicException('conditional value argument allocated an expression closure');
    }}
}}
echo 'statement arguments';
"#,
            php.strip_prefix("<?php").unwrap(),
            php_function_name("hot"),
            php_function_name("cleanup"),
            php_function_name("hot"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"functional arguments;statement arguments");
    }

    #[test]
    fn conditional_receivers_preserve_projections_and_pending_owners() {
        let source = r#"
class Trace {
    static writable int $order = 0;
    static writable int $drops = 0;
    static writable int $calls = 0;
}
interface Reading { function read(int $number): int; }
class Reader implements Reading {
    function __construct(int $bias) {}
    function read(int $number): int { return $this->bias + $number; }
    function zero(): int { return $this->bias; }
}
class Token {
    function __construct(int $value) {}
    function __destruct(): void { Trace::drops++; }
}
class Consumer {
    function __construct(take Token $token) {}
    consuming function finish(int $later): int {
        Trace::calls++;
        return $this->token->value + $later;
    }
}
class Failure implements Error { function __construct(string $message) {} }
function mark(int $digit): int { Trace::order = Trace::order * 10 + $digit; return $digit; }
function fail(): int throws Failure { throw new Failure("operand"); }
function hot(int $iterations): int {
    let $reader = new Reader(10);
    SharedReference<Reading> $shared = shared new Reader(11);
    let writable $sum = 0;
    foreach (0..<$iterations as int $index) {
        Trace::order = 0;
        $sum += (when (true): Reader {
            mark(1); return $reader;
        } else { return $reader; } finally { mark(2); })->read(mark(3));
        $sum += (match (true) { true => $reader, default => $reader, })->zero();
        $sum += (when (true): Reader { return $reader; } else { return $reader; })->zero();
        (when (true): ?Reader {
            mark(7); return null;
        } else { return $reader; })?->read(mark(9));
        $sum += (when (true): SharedReference<Reading> {
            mark(8); return $shared;
        } else { return $shared; })->read(2);
        if (Trace::order != 12378) { panic("receiver order changed"); }
    }
    return $sum;
}
function cleanup(): int {
    Trace::drops = 0;
    Trace::calls = 0;
    try {
        (when (true): Consumer {
            return new Consumer(new Token(7));
        } else { return new Consumer(new Token(9)); })->finish(fail());
    } catch (Failure) {}
    try {
        (match (true) {
            true => new Consumer(new Token(7)), default => new Consumer(new Token(9)),
        })->finish(fail());
    } catch (Failure) {}
    if (Trace::drops != 2 || Trace::calls != 0) { panic("pending operand cleanup changed"); }
    let $receiver = (when (true): Consumer {
        return new Consumer(new Token(7));
    } else { return new Consumer(new Token(9)); })->finish(3);
    let $matched = (match (true) {
        true => new Consumer(new Token(7)), default => new Consumer(new Token(9)),
    })->finish(1);
    if ($receiver != 10 || $matched != 8 || Trace::calls != 2) { panic("handoff changed"); }
    try {
        (when (true): Consumer {
            return new Consumer(new Token(7));
        } else { return new Consumer(new Token(9)); } finally { fail(); })->finish(3);
    } catch (Failure) {}
    if (Trace::calls != 2) { panic("abandoned conditional result was invoked"); }
    return Trace::drops;
}
function main(): void {}
"#;
        let php = crate::compile_source_to_php("conditional-call-receivers.doria", source).unwrap();
        let script = format!(
            r#"{}
foreach ([0, 1, 1000] as $iterations) {{
    if ({}($iterations) !== $iterations * 46) {{
        throw new LogicException('conditional receiver result changed');
    }}
}}
if ({}() !== 5) {{ throw new LogicException('conditional receiver ownership changed'); }}
echo 'functional receivers;';
$tokens = __doria_test_loop_tokens(file_get_contents(__FILE__), '{}');
foreach ([T_FUNCTION, T_FN] as $token) {{
    if (($tokens[$token] ?? 0) !== 0) {{
        throw new LogicException('conditional receiver allocated an expression closure');
    }}
}}
echo 'statement receivers';
"#,
            php.strip_prefix("<?php").unwrap(),
            php_function_name("hot"),
            php_function_name("cleanup"),
            php_function_name("hot"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"functional receivers;statement receivers");
    }

    #[test]
    fn conditional_cell_arguments_preserve_loans_and_taking_handoffs() {
        let source = r#"
class Trace {
    static writable int $order = 0;
    static writable int $drops = 0;
    static writable int $calls = 0;
}
class Token {
    function __construct(int $value) {}
    function __destruct(): void { Trace::drops++; }
}
class Failure implements Error { function __construct(string $message) {} }
function mark(int $digit): int { Trace::order = Trace::order * 10 + $digit; return $digit; }
function fail(): int throws Failure { throw new Failure("argument"); }
function lend(Token $value, int $number = 2): Token { return $value; }
function consume(take Token $value, int $number): int {
    Trace::calls++;
    return $value->value + $number;
}
function drops(): int { return Trace::drops; }
function hot(int $iterations): int {
    Trace::drops = 0;
    let $owner = new Token(7);
    let $callback = function(Token $value, int $number): Token { return $value; };
    let writable $sum = 0;
    foreach (0..<$iterations as int $index) {
        Trace::order = 0;
        $sum += lend(
            number: mark(2),
            value: when (true): Token { mark(1); return $owner; } else { return $owner; },
        )->value;
        if (Trace::order != 21) { panic("named cell argument order changed"); }
        Trace::order = 0;
        $sum += $callback(
            match (true) { true => $owner, default => $owner, }, mark(3),
        )->value;
        $sum += lend(when (true): Token { return $owner; } else { return $owner; })->value;
        if (Trace::order != 3 || Trace::drops != 0 || $owner->value != 7) {
            panic("borrowed conditional cell consumed its owner");
        }
    }
    return $sum;
}
function owned(int $iterations): int {
    Trace::drops = 0;
    Trace::calls = 0;
    let $callback = function(take Token $value, int $number): int {
        Trace::calls++;
        return $value->value + $number;
    };
    let writable $sum = 0;
    foreach (0..<$iterations as int $index) {
        $sum += consume(
            when (true): Token { return new Token(5); } else { return new Token(9); }, 2,
        );
        $sum += $callback(match (true) { true => new Token(7), default => new Token(9), }, 3);
    }
    if (Trace::drops != $iterations * 2 || Trace::calls != $iterations * 2) {
        panic("taking cell handoff changed");
    }
    return $sum;
}
function cleanup(): int {
    Trace::drops = 0;
    Trace::calls = 0;
    let $callback = function(take Token $value, int $number): int {
        Trace::calls++;
        return $value->value + $number;
    };
    try {
        consume(when (true): Token { return new Token(1); } else { return new Token(9); }, fail());
    } catch (Failure) {}
    try {
        $callback(match (true) { true => new Token(2), default => new Token(9), }, fail());
    } catch (Failure) {}
    try {
        consume(when (true): Token {
            return new Token(3);
        } else { return new Token(9); } finally { fail(); }, 0);
    } catch (Failure) {}
    try {
        $callback(when (true): Token {
            return new Token(4);
        } else { return new Token(9); } finally { fail(); }, 0);
    } catch (Failure) {}
    if (Trace::calls != 0) { panic("callee ran before its conditional arguments completed"); }
    return Trace::drops;
}
function main(): void {}
"#;
        let php = crate::compile_source_to_php("conditional-cell-arguments.doria", source).unwrap();
        assert!(php.contains("new __DoriaCell($__doria_statement_value"));
        let script = format!(
            r#"{}
foreach ([0, 1, 1000] as $iterations) {{
    if ({}($iterations) !== $iterations * 21 || {}() !== 1) {{
        throw new LogicException('conditional cell loan changed');
    }}
    if ({}($iterations) !== $iterations * 17) {{
        throw new LogicException('conditional taking cell result changed');
    }}
}}
if ({}() !== 4) {{ throw new LogicException('pending cell argument cleanup changed'); }}
echo 'functional cells;';
foreach (['{}', '{}'] as $function) {{
    $tokens = __doria_test_loop_tokens(file_get_contents(__FILE__), $function);
    foreach ([T_FUNCTION, T_FN] as $token) {{
        if (($tokens[$token] ?? 0) !== 0) {{
            throw new LogicException('conditional cell value allocated an expression closure');
        }}
    }}
}}
echo 'statement cells';
"#,
            php.strip_prefix("<?php").unwrap(),
            php_function_name("hot"),
            php_function_name("drops"),
            php_function_name("owned"),
            php_function_name("cleanup"),
            php_function_name("hot"),
            php_function_name("owned"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"functional cells;statement cells");
    }

    #[test]
    fn conditional_borrowed_arguments_reuse_selected_storage_cells() {
        let source = r#"
class Trace {
    static writable int $drops = 0;
    static writable int $calls = 0;
    static writable int $finalizers = 0;
}
interface Reading { function read(): int; }
class Token implements Reading {
    function __construct(int $value) {}
    function read(): int { return $this->value; }
    function __destruct(): void { Trace::drops++; }
}
class Failure implements Error { function __construct(string $message) {} }
function fail(): int throws Failure { throw new Failure("argument"); }
function lend(Token $value, int $number = 0): Token { Trace::calls++; return $value; }
function erased(Reading $value): Reading { return $value; }
function dynamic(mixed $value): int {
    if ($value is int) { return $value; }
    panic("conditional argument lost its mixed conversion");
}
function hot(bool $left, int $iterations): int {
    Trace::drops = 0;
    Trace::calls = 0;
    Trace::finalizers = 0;
    let $first = new Token(5);
    let $second = new Token(9);
    let writable $sum = 0;
    foreach (0..<$iterations as int $index) {
        $sum += lend(when ($left): Token { return $first; } else { return $second; })->value;
        $sum += lend(match ($left) { true => $first, false => $second, })->value;
        $sum += lend(when ($left): Token {
            return match (true) { true => $first, default => $second, };
        } else {
            return match (false) { true => $first, false => $second, };
        } finally { Trace::finalizers++; })->value;
        if (Trace::drops != 0) { panic("borrowed argument dropped its owner"); }
    }
    if (Trace::calls != $iterations * 3 || Trace::finalizers != $iterations) {
        panic("borrowed argument evaluation changed");
    }
    return $sum;
}
function conversions(): int {
    let $owner = new Token(7);
    return erased(when (true): Token { return $owner; } else { return $owner; })->read()
        + dynamic(when (true): int { return 11; } else { return 13; });
}
function cleanup(): int {
    Trace::drops = 0;
    Trace::calls = 0;
    try {
        lend(when (true): Token { return new Token(1); } else { return new Token(2); }, fail());
    } catch (Failure) {}
    try {
        lend(when (true): Token { return new Token(3); } else { return new Token(4); }
            finally { fail(); });
    } catch (Failure) {}
    if (Trace::calls != 0) { panic("callee ran before its arguments completed"); }
    return Trace::drops;
}
function drops(): int { return Trace::drops; }
function main(): void {}
"#;
        let php = crate::compile_source_to_php("conditional-borrowed-cells.doria", source).unwrap();
        let script = format!(
            r#"{}
foreach ([false, true] as $left) {{
    $baseline = null;
    foreach ([0, 1, 1000] as $iterations) {{
        __DoriaConstructorProbe::reset();
        if ({}($left, $iterations) !== $iterations * ($left ? 15 : 27) || {}() !== 2) {{
            throw new LogicException('conditional borrowed argument result or cleanup changed');
        }}
        $cells = __DoriaConstructorProbe::count('__DoriaCell');
        $baseline ??= $cells;
        if ($cells !== $baseline) {{
            throw new LogicException("conditional borrowed arguments allocated $cells cells; baseline $baseline");
        }}
    }}
}}
if ({}() !== 18 || {}() !== 2) {{
    throw new LogicException('conditional argument conversion or pending cleanup changed');
}}
$tokens = __doria_test_loop_tokens(file_get_contents(__FILE__), '{}');
foreach ([T_FUNCTION, T_FN] as $token) {{
    if (($tokens[$token] ?? 0) !== 0) {{
        throw new LogicException('conditional borrowed argument allocated an expression closure');
    }}
}}
echo 'selected cells reused';
"#,
            php.strip_prefix("<?php").unwrap(),
            php_function_name("hot"),
            php_function_name("drops"),
            php_function_name("conversions"),
            php_function_name("cleanup"),
            php_function_name("hot"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"selected cells reused");
    }

    #[test]
    fn conditional_borrowed_parameters_forward_homes_through_calls_and_dispatch() {
        let source = r#"
class Trace { static writable int $drops = 0; }
interface Reading { function read(): int; }
class Token implements Reading {
    function __construct(int $value) {}
    function read(): int { return $this->value; }
    function __destruct(): void { Trace::drops++; }
}
function lend(Token $value): Token { return $value; }
function relay(Token $first, Token $second, bool $left): int {
    return lend(when ($left): Token { return $first; } else { return $second; })->value
        + lend(match ($left) { true => $first, false => $second, })->value;
}
function through(Token $first, Token $second, bool $left): int {
    return relay(second: $second, left: $left, first: $first);
}
function identity<T>(T $value): T { return $value; }
function generic<T implements Reading>(T $first, T $second, bool $left): int {
    return identity(when ($left): T { return $first; } else { return $second; })->read();
}
interface Reader { function read(Token $first, Token $second, bool $left): int; }
open class Base implements Reader {
    open function read(Token $first, Token $second, bool $left): int { return 0; }
}
class Derived extends Base {
    override function read(Token $first, Token $second, bool $left): int {
        return through($first, $second, $left);
    }
}
function virtualRead(Base $reader, Token $first, Token $second, bool $left): int {
    return $reader->read($first, $second, $left);
}
function erasedRead(Reader $reader, Token $first, Token $second, bool $left): int {
    return $reader->read($first, $second, $left);
}
function hot(bool $left, int $iterations): int {
    Trace::drops = 0;
    let $first = new Token(5);
    let $second = new Token(9);
    let $reader = new Derived();
    let writable $sum = 0;
    foreach (0..<$iterations as int $index) {
        $sum += through($first, $second, $left);
        $sum += generic($first, $second, $left);
        $sum += virtualRead($reader, $first, $second, $left);
        $sum += erasedRead($reader, $first, $second, $left);
        if (Trace::drops != 0) { panic("forwarded borrow dropped its owner"); }
    }
    return $sum;
}
function drops(): int { return Trace::drops; }
function main(): void {}
"#;
        let php = crate::compile_source_to_php("forwarded-borrowed-cells.doria", source).unwrap();
        let script = format!(
            r#"{}
foreach ([false, true] as $left) {{
    $baseline = null;
    foreach ([0, 1, 1000] as $iterations) {{
        __DoriaConstructorProbe::reset();
        if ({}($left, $iterations) !== $iterations * ($left ? 35 : 63) || {}() !== 2) {{
            throw new LogicException('forwarded borrowed argument result or cleanup changed');
        }}
        $cells = __DoriaConstructorProbe::count('__DoriaCell');
        $baseline ??= $cells;
        if ($cells !== $baseline) {{
            throw new LogicException("forwarded borrowed arguments allocated $cells cells; baseline $baseline");
        }}
    }}
}}
echo 'forwarded cells reused';
"#,
            php.strip_prefix("<?php").unwrap(),
            php_function_name("hot"),
            php_function_name("drops"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"forwarded cells reused");
    }

    #[test]
    fn conditional_borrowed_argument_returns_preserve_single_source() {
        let source = r#"
class Token { function __construct(int $value) {} }
function lend(Token $value): Token { return $value; }
function pick(Token $value, bool $left): Token {
    return lend(when ($left): Token { return $value; } else { return $value; });
}
function pickMatch(Token $value, bool $left): Token {
    return match ($left) { true => $value, false => lend($value), };
}
function pickWhen(Token $value, bool $left): Token {
    return when ($left): Token { return $value; } else { return lend($value); };
}
function main(): void {}
"#;
        let php = crate::compile_source_to_php("conditional-returned-cell.doria", source).unwrap();
        let script = format!(
            r#"{}
$first = new __DoriaCell(new {}(5));
$second = new __DoriaCell(new {}(9));
if ({}($first, true) !== $first || {}($second, false) !== $second) {{
    throw new LogicException('conditional argument replaced its returned storage home');
}}
foreach ([{}, {}] as $select) {{
    if ($select($first, true) !== $first || $select($second, false) !== $second) {{
        throw new LogicException('conditional return replaced its source storage home');
    }}
}}
__doria_drop_cell($second);
__doria_drop_cell($first);
echo 'returned cells preserved';
"#,
            php.strip_prefix("<?php").unwrap(),
            php_type_symbol("Token"),
            php_type_symbol("Token"),
            php_function_name("pick"),
            php_function_name("pick"),
            emit_php_string_literal(&php_function_name("pickMatch")),
            emit_php_string_literal(&php_function_name("pickWhen")),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"returned cells preserved");
    }

    #[test]
    fn conditional_pattern_returns_preserve_borrowed_payloads() {
        let source = crate::mir_lowering::conditional_results::PATTERN_RESULT_SOURCE;
        let php = crate::compile_source_to_php("pattern-returned-cell.doria", source).unwrap();
        let script = format!(
            "{}\n{}();",
            php.strip_prefix("<?php").unwrap(),
            php_function_name("main"),
        );
        let result = php_execution::Script::new(&script)
            .command(&["-d", "ffi.enable=0"])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"121034567080");
    }

    #[test]
    fn conditional_named_cells_preserve_borrowed_results_and_pending_owners() {
        let source = r#"
class Trace {
    static writable int $order = 0;
    static writable int $drops = 0;
    static writable int $calls = 0;
    static writable int $views = 0;
    static writable int $gets = 0;
}
class Token {
    function __construct(int $value) {}
    function __destruct(): void { Trace::drops++; }
}
class Box {
    function __construct(take Token $token) {}
    Token $item { get { Trace::gets++; return $this->token; } }
}
class Failure implements Error { function __construct(string $message) {} }
function mark(int $digit): int { Trace::order = Trace::order * 10 + $digit; return $digit; }
function fail(): int throws Failure { throw new Failure("argument"); }
function lend(Token $value, int $number = 2): Token { return $value; }
function boxView(Box $value): Box { Trace::views++; return $value; }
function listView(List<Token> $value): List<Token> { Trace::views++; return $value; }
function makeToken(): Token { return new Token(11); }
function makeBox(): Box { return new Box(new Token(13)); }
function consume(take Token $value, int $number): int {
    Trace::calls++;
    return $value->value + $number;
}
function drops(): int { return Trace::drops; }
function hot(int $iterations): int {
    Trace::drops = 0;
    Trace::views = 0;
    Trace::gets = 0;
    let $owner = new Token(7);
    let $box = new Box(new Token(11));
    List<Token> $list = [new Token(17)];
    let writable $sum = 0;
    foreach (0..<$iterations as int $index) {
        Trace::order = 0;
        $sum += lend(
            number: mark(2),
            value: when (true): Token { mark(1); return $owner; } else { return $owner; },
        )->value;
        $sum += lend(match (true) { true => $owner, default => $owner, })->value;
        $sum += lend(when (true): Token { return $owner; } else { return $owner; })->value;
        $sum += boxView(when (true): Box { return $box; } else { return $box; })->item->value;
        $sum += (when (true): Box { return $box; } else { return $box; })->item->value;
        let $first = ((listView($list)))->first;
        if ($first != null) { $sum += $first->value; }
        if (Trace::order != 21 || Trace::drops != 0 || $owner->value != 7) {
            panic("named conditional cell loan changed");
        }
    }
    if (Trace::views != $iterations * 2 || Trace::gets != $iterations * 2) { panic("receiver was evaluated twice"); }
    return $sum;
}
function temporary(int $iterations): int {
    Trace::drops = 0;
    Trace::gets = 0;
    let writable $sum = 0;
    foreach (0..<$iterations as int $index) {
        $sum += makeToken()->value;
        $sum += makeBox()->item->value;
        $sum += (when (true): Box { return makeBox(); } else { return makeBox(); })->item->value;
        if (Trace::drops != ($index + 1) * 3 || Trace::gets != ($index + 1) * 2) {
            panic("temporary receiver cleanup changed");
        }
    }
    return $sum;
}
function owned(int $iterations): int {
    Trace::drops = 0;
    Trace::calls = 0;
    let writable $sum = 0;
    foreach (0..<$iterations as int $index) {
        $sum += consume(
            when (true): Token { return new Token(5); } else { return new Token(9); }, 2,
        );
    }
    if (Trace::drops != $iterations || Trace::calls != $iterations) { panic("named cell handoff changed"); }
    return $sum;
}
function cleanup(): int {
    Trace::drops = 0;
    Trace::calls = 0;
    try {
        consume(when (true): Token { return new Token(1); } else { return new Token(9); }, fail());
    } catch (Failure) {}
    try {
        consume(when (true): Token {
            return new Token(2);
        } else { return new Token(9); } finally { fail(); }, 0);
    } catch (Failure) {}
    if (Trace::calls != 0) { panic("named callee ran before its arguments completed"); }
    return Trace::drops;
}
function main(): void {}
"#;
        let php = crate::compile_source_to_php("conditional-named-cells.doria", source).unwrap();
        assert!(php.contains("new __DoriaCell($__doria_statement_value"));
        let script = format!(
            r#"{}
foreach ([0, 1, 1000] as $iterations) {{
    if ({}($iterations) !== $iterations * 60 || {}() !== 3) {{
        throw new LogicException('conditional named cell loan changed');
    }}
    if ({}($iterations) !== $iterations * 7) {{
        throw new LogicException('conditional named cell result changed');
    }}
    if ({}($iterations) !== $iterations * 37) {{
        throw new LogicException('temporary receiver result changed');
    }}
}}
if ({}() !== 2) {{ throw new LogicException('pending named cell cleanup changed'); }}
echo 'functional named cells;';
foreach (['{}', '{}', '{}'] as $function) {{
    $tokens = __doria_test_loop_tokens(file_get_contents(__FILE__), $function);
    foreach ([T_FUNCTION, T_FN] as $token) {{
        if (($tokens[$token] ?? 0) !== 0) {{
            throw new LogicException('conditional named cell allocated an expression closure');
        }}
    }}
}}
echo 'statement named cells';
"#,
            php.strip_prefix("<?php").unwrap(),
            php_function_name("hot"),
            php_function_name("drops"),
            php_function_name("owned"),
            php_function_name("temporary"),
            php_function_name("cleanup"),
            php_function_name("hot"),
            php_function_name("owned"),
            php_function_name("temporary"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(
            result.stdout,
            b"functional named cells;statement named cells"
        );
    }

    #[test]
    fn conditional_callees_preserve_order_and_pending_owners() {
        let source = r#"
class Trace {
    static writable int $order = 0;
    static writable int $drops = 0;
    static writable int $calls = 0;
}
class Token {
    function __construct(int $value) {}
    function __destruct(): void { Trace::drops++; }
}
class Failure implements Error { function __construct(string $message) {} }
function mark(int $digit): int { Trace::order = Trace::order * 10 + $digit; return $digit; }
function fail(): int throws Failure { throw new Failure("operand"); }
function drop(take Token $token): void {}
function makeOnce(int $value): function once(int): int {
    let $token = new Token($value);
    return function(int $number): int with (take $token) {
        Trace::calls++;
        let $result = $token->value + $number;
        drop($token);
        return $result;
    };
}
function hot(int $iterations): int {
    let $first = fn(int $number) => $number + 1;
    let $second = fn(int $number) => $number + 2;
    let writable $sum = 0;
    foreach (0..<$iterations as int $index) {
        Trace::order = 0;
        $sum += (when (true): function(int): int {
            mark(1); return $first;
        } else { return $second; } finally { mark(2); })(mark(3));
        $sum += (match (false) { true => $first, false => $second, })(5);
        if (Trace::order != 123) { panic("callee order changed"); }
    }
    return $sum;
}
function cleanup(): int {
    Trace::drops = 0;
    Trace::calls = 0;
    try {
        (when (true): function once(int): int {
            return makeOnce(7);
        } else { return makeOnce(9); })(fail());
    } catch (Failure) {}
    try {
        (match (true) { true => makeOnce(7), default => makeOnce(9), })(fail());
    } catch (Failure) {}
    if (Trace::drops != 2 || Trace::calls != 0) { panic("pending callee cleanup changed"); }
    let $chosen = (when (true): function once(int): int {
        return makeOnce(7);
    } else { return makeOnce(9); })(3);
    let $matched = (match (true) { true => makeOnce(7), default => makeOnce(9), })(1);
    if ($chosen != 10 || $matched != 8 || Trace::calls != 2) { panic("callee handoff changed"); }
    try {
        (when (true): function once(int): int {
            return makeOnce(7);
        } else { return makeOnce(9); } finally { fail(); })(1);
    } catch (Failure) {}
    if (Trace::calls != 2) { panic("abandoned conditional callee was invoked"); }
    return Trace::drops;
}
function main(): void {}
"#;
        let php = crate::compile_source_to_php("conditional-call-callees.doria", source).unwrap();
        let script = format!(
            r#"{}
foreach ([0, 1, 1000] as $iterations) {{
    if ({}($iterations) !== $iterations * 11) {{
        throw new LogicException('conditional callee result changed');
    }}
}}
if ({}() !== 5) {{ throw new LogicException('conditional callee ownership changed'); }}
echo 'functional callees;';
$tokens = __doria_test_loop_tokens(file_get_contents(__FILE__), '{}');
foreach ([T_FUNCTION, T_FN] as $token) {{
    if (($tokens[$token] ?? 0) !== 0) {{
        throw new LogicException('conditional callee allocated an expression closure');
    }}
}}
echo 'statement callees';
"#,
            php.strip_prefix("<?php").unwrap(),
            php_function_name("hot"),
            php_function_name("cleanup"),
            php_function_name("hot"),
        );
        let result = execute_with_constructor_probe(&script);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"functional callees;statement callees");
    }

    #[test]
    fn conditional_enum_results_use_fields_across_nested_and_nullable_branches() {
        let source = include_str!("../../tests/fixtures/inline_copy_when_results.doria");
        let php = execute(source, 17, "hot");
        let start = php
            .find(&format!("function {}(", php_function_name("hot")))
            .unwrap();
        let body = php[start..].split_once("\n}\n").unwrap().0;
        for boxed in ["new ", "__DoriaValueBox_", "::Number(", "::Pair("] {
            assert!(
                !body.contains(boxed),
                "unexpected conditional boxing {boxed}: {body}"
            );
        }
        assert!(
            !body
                .lines()
                .any(|line| line.contains("(function()") && line.ends_with('{')),
            "conditional expression closure: {body}"
        );
        let labels = body
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("__doria_when_merge") && line.ends_with(':'))
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), 3, "{body}");
        assert_eq!(
            labels.iter().collect::<HashSet<_>>().len(),
            labels.len(),
            "duplicate conditional labels: {body}"
        );
    }

    #[test]
    fn conditional_enum_results_preserve_statement_owners_and_replacement() {
        let source = include_str!("../../tests/fixtures/inline_copy_when_results.doria");
        let mir = crate::lower_source_to_mir("conditional-fields.doria", source).unwrap();
        let result = crate::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(result.stdout, b"17|20|9\n", "{result:?}");
        assert!(result.stderr.is_empty());
        assert_eq!(result.exit_status, 0);
        assert!(result.runtime_diagnostic.is_none());
        execute(source, 20, "lifetime");
        execute(source, 9, "replacement");
    }

    #[test]
    fn conditional_enum_loop_exits_preserve_cleanup() {
        execute(
            include_str!("../../tests/fixtures/inline_copy_when_loop_exits.doria"),
            35,
            "loopExits",
        );
    }

    #[test]
    fn statement_when_results_use_direct_destinations_and_preserve_cleanup() {
        let source = include_str!("../../tests/fixtures/statement_when_results.doria");
        let php = execute_main(
            source,
            b"23|0|2,f,1,\n7|3,o,\n3,o,7,\nowned claim\n7|t,4,\nreplaced|a,8,\nouter|i,r,6,\n",
        );
        for function in [
            "scalar",
            "owned",
            "managed",
            "temporary",
            "abandoned",
            "outerAbandoned",
        ] {
            let start = php
                .find(&format!("function {}(", php_function_name(function)))
                .unwrap();
            let body = php[start..].split_once("\n}\n").unwrap().0;
            assert!(
                !body
                    .lines()
                    .any(|line| line.contains("(function()") && line.ends_with('{')),
                "conditional expression closure in {function}: {body}"
            );
            assert!(
                body.contains("__doria_when_merge"),
                "missing direct destination in {function}: {body}"
            );
            let labels = body
                .lines()
                .map(str::trim)
                .filter(|line| line.starts_with("__doria_when_merge") && line.ends_with(':'))
                .collect::<Vec<_>>();
            assert_eq!(
                labels.iter().collect::<HashSet<_>>().len(),
                labels.len(),
                "duplicate labels: {body}"
            );
        }
    }

    #[test]
    fn repeated_statement_when_results_do_not_drop_transferred_owners() {
        execute_main(
            include_str!("../../tests/fixtures/statement_when_reuse.doria"),
            b"5|u,0,u,c,5,\n",
        );
    }

    #[test]
    fn callable_results_use_field_bodies_and_preserve_boxed_entry_points() {
        let php = execute(
            r#"
enum Packet { case Number(int $count); case Empty; }
function make(int $count): Packet { return Packet::Number($count); }
function hot(): int {
    let writable $index = 0;
    let writable $sum = 0;
    while ($index < 1000) {
        let $value = make($index);
        $sum += match ($value) { Packet::Number($number) => $number, Packet::Empty => -1, };
        $index++;
    }
    return $sum;
}
function main(): void { hot(); }
"#,
            499_500,
            "hot",
        );
        let start = php
            .find(&format!("function {}(", php_function_name("hot")))
            .unwrap();
        let hot = php[start..].split_once("\n}\n").unwrap().0;
        assert!(hot.contains("__doriaFieldResult"), "{hot}");
        for boxed in ["new ", "__DoriaValueBox_", "::Number(", "::Empty("] {
            assert!(!hot.contains(boxed), "unexpected boxed call {boxed}: {hot}");
        }
        let wrapper = php
            .split_once(&format!("function {}(", php_function_name("make")))
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0;
        assert!(wrapper.contains("__DoriaValueBox_"));
        let script = format!(
            "{}\n$value = {}(PHP_INT_MIN);\n{}(PHP_INT_MAX);\nif ($value->__doriaPayloadAt(0) !== PHP_INT_MIN) {{ exit(1); }}",
            php.strip_prefix("<?php").unwrap(),
            php_function_name("make"),
            php_function_name("make")
        );
        let result = php_execution::Script::new(&script)
            .command(&["-d", "ffi.enable=0"])
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
    }

    #[test]
    fn callable_results_preserve_generic_virtual_erased_nullable_and_property_dispatch() {
        execute(
            r#"
enum Packet { case Number(int $count); case Empty; }
interface Emitter { function value(): Packet; }
open class Base implements Emitter {
    open function value(): Packet { return Packet::Number(1); }
}
class Child extends Base {
    override function value(): Packet { return Packet::Number(2); }
    Packet $packet { get => Packet::Number(3); }
    static Packet $common { get => Packet::Number(4); }
    static function create(): Packet { return Packet::Number(5); }
}
function copy<T>(T $value): T { return $value; }
function optional(bool $present): ?Child { if ($present) { return new Child(); } return null; }
function constrained<T implements Emitter>(T $value): Packet { return $value->value(); }
function erased(Emitter $value): Packet { return $value->value(); }
function virtualCall(Base $value): Packet { return $value->value(); }
function inspect(?Packet $value): int {
    return match ($value) { null => 0, Packet::Number($number) => $number, Packet::Empty => -100, };
}
function dispatch(): int {
    let $child = new Child();
    ?Child $absent = optional(false);
    ?Child $present = optional(true);
    let $first = erased($child);
    let $second = virtualCall($child);
    let $third = constrained($child);
    let $fourth = $child->packet;
    let $fifth = Child::common;
    let $sixth = Child::create();
    let $seventh = copy(Packet::Number(7));
    let $eighth = $absent?->value();
    let $ninth = $present?->value();
    return inspect($first) + inspect($second) + inspect($third) + inspect($fourth)
        + inspect($fifth) + inspect($sixth) + inspect($seventh) + inspect($eighth) + inspect($ninth);
}
function main(): void { dispatch(); }
"#,
            27,
            "dispatch",
        );
    }

    #[test]
    fn callable_fields_publish_after_consuming_cleanup_and_finalizers() {
        execute(
            r#"
enum Packet { case Number(int $count); case Pair(int $left, int $right); }
class State { static writable int $drops = 0; }
function make(int $count): Packet { return Packet::Number($count); }
class Owner {
    function __construct() {}
    consuming function finish(): Packet { return Packet::Number(7); }
    function __destruct(): void { let $noise = make(99); State::drops++; }
}
class Failure implements Error { function __construct(string $message) {} }
function finalized(take Owner $owner): Packet {
    try { return Packet::Number(11); }
    finally { let $noise = make(88); }
}
function replaced(): Packet throws Failure {
    try { return Packet::Number(1); }
    finally { let $noise = make(77); throw new Failure("replaced"); }
}
function selected(bool $first): Packet {
    return when ($first): Packet { return Packet::Number(13); }
        else { return Packet::Pair(5, 6); };
}
function inspect(Packet $value): int {
    return match ($value) { Packet::Number($number) => $number, Packet::Pair($left, $right) => $left + $right, };
}
function cleanup(): int {
    State::drops = 0;
    let $consumed = (new Owner())->finish();
    let $finalized = finalized(new Owner());
    let writable $unchanged = make(17);
    try { $unchanged = replaced(); } catch (Failure) {}
    let $yes = selected(true);
    let $no = selected(false);
    let $callback = fn() => Packet::Number(19);
    let $called = $callback();
    return inspect($consumed) + inspect($finalized) + inspect($unchanged)
        + inspect($yes) + inspect($no) + inspect($called) + State::drops;
}
function main(): void { cleanup(); }
"#,
            80,
            "cleanup",
        );
    }

    #[test]
    fn copy_enum_result_writers_reuse_complete_nominal_slots_and_keep_boxed_boundaries() {
        let source = r#"
enum Packet { case Number(int $count); case Pair(int $left, string $right); case Empty; }
enum Other { case Triple(int $first, int $second, int $third); }
class Owner { function __construct() {} }
enum Owned { case Value(Owner $owner); }
function main(): void {}
"#;
        let program = crate::lower_source("copy-enum-result-slots.doria", source).unwrap();
        let id = |name| {
            program
                .semantic_info
                .enums
                .iter()
                .find(|definition| definition.name == name)
                .unwrap()
                .id
                .0
        };
        let php = crate::compile_source_to_php("copy-enum-result-slots.doria", source).unwrap();
        for (name, case) in [
            ("Packet", "Number"),
            ("Packet", "Pair"),
            ("Packet", "Empty"),
            ("Other", "Triple"),
        ] {
            let start = php
                .find(&format!("function {}(", writer_name(name, case)))
                .unwrap();
            let body = php[start..].split_once("\n}").unwrap().0;
            assert!(!body.contains("new "), "{body}");
            assert!(!body.contains('['), "{body}");
        }
        assert!(!php.contains(&format!("function {}(", writer_name("Owned", "Value"))));
        let Ok(version) = Command::new("php").arg("--version").output() else {
            eprintln!("PHP unavailable; result-slot execution skipped");
            return;
        };
        assert!(version.status.success());
        let script = format!(
            r#"{}
function resultCheck(bool $condition): void {{
    if (!$condition) {{ throw new LogicException('result-slot transport changed'); }}
}}
function resultArgument(mixed $value): mixed {{
    $GLOBALS['resultOrder'][] = $value;
    return $value;
}}
{}(PHP_INT_MIN);
resultCheck({RESULT}::$type === {} && {RESULT}::$tag === 0 && {RESULT}::$field0 === PHP_INT_MIN);
$saved = {RESULT}::$field0;
{}(PHP_INT_MAX);
resultCheck($saved === PHP_INT_MIN && {RESULT}::$field0 === PHP_INT_MAX);
$GLOBALS['resultOrder'] = [];
{}(right: resultArgument('right'), left: resultArgument(7));
resultCheck($GLOBALS['resultOrder'] === ['right', 7]);
resultCheck({RESULT}::$tag === 1 && {RESULT}::$field0 === 7 && {RESULT}::$field1 === 'right');
{}(1, 2, 3);
resultCheck({RESULT}::$type === {} && {RESULT}::$field2 === 3);
{}();
resultCheck({RESULT}::$type === {} && {RESULT}::$tag === 2);
resultCheck({RESULT}::$field0 === null && {RESULT}::$field1 === null && {RESULT}::$field2 === null);
$old = {}::Number(5);
{}(9);
resultCheck($old->__doriaPayloadAt(0) === 5 && {RESULT}::$field0 === 9);
$next = {}::Number(11);
resultCheck($old->__doriaPayloadAt(0) === 5 && $next->__doriaPayloadAt(0) === 11);
resultCheck({RESULT}::$type === -1 && {RESULT}::$field0 === null);
for ($index = 0; $index < 1000; ++$index) {{ {}($index); }}
resultCheck({RESULT}::$field0 === 999);
{RESULT}::clear();
echo "result slots preserved\n";
"#,
            php.strip_prefix("<?php").unwrap(),
            writer_name("Packet", "Number"),
            id("Packet"),
            writer_name("Packet", "Number"),
            writer_name("Packet", "Pair"),
            writer_name("Other", "Triple"),
            id("Other"),
            writer_name("Packet", "Empty"),
            id("Packet"),
            php_type_symbol("Packet"),
            writer_name("Packet", "Number"),
            php_type_symbol("Packet"),
            writer_name("Packet", "Number"),
        );
        let result = php_execution::Script::new(&script)
            .command(&["-d", "ffi.enable=0"])
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"result slots preserved\n");
    }
}
