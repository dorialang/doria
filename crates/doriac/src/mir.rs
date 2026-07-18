//! Unstable native-oriented MIR.
//!
//! MIR is the compiler-internal, backend-independent representation that Stage
//! 11 grows into the debug/interpreter oracle and future native lowering input.
//! The text dump is deterministic but not a stable public format.

use std::fmt;

use crate::class_layout::{ClassId, ClassLayout, PropertyId};
use crate::format_string::FormatPiece;
use crate::numeric::{FloatType, FloatValue, IntegerType, IntegerValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StaticId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub classes: Vec<Class>,
    pub statics: Vec<StaticProperty>,
    pub functions: Vec<Function>,
    pub entry: FunctionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticProperty {
    pub id: StaticId,
    pub class: ClassId,
    pub name: String,
    pub ty: Type,
    pub writable: bool,
    pub initializer: StaticValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticValue {
    Scalar(ScalarValue),
    String(String),
    Null,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Class {
    pub id: ClassId,
    pub name: String,
    pub properties: Vec<Property>,
    pub layout: ClassLayout,
    pub constructor: Option<FunctionId>,
    pub destructor: Option<FunctionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub id: PropertyId,
    pub name: String,
    pub ty: Type,
    pub writable: bool,
    pub promoted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub id: FunctionId,
    pub name: String,
    pub method: Option<MethodIdentity>,
    pub receiver_mode: Option<ReceiverMode>,
    pub params: Vec<LocalId>,
    pub return_type: ReturnType,
    pub locals: Vec<Local>,
    pub blocks: Vec<BasicBlock>,
    pub entry_block: BlockId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodIdentity {
    pub class: ClassId,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverMode {
    Readonly,
    Writable,
    /// Reserved for a future accepted consuming-receiver design.
    UnsupportedConsuming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnBorrow {
    pub source: BorrowSource,
    pub writable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowSource {
    Receiver,
    Parameter(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnType {
    Value(Type),
    Void,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarType {
    Integer(IntegerType),
    Float(FloatType),
    Bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarValue {
    Integer(IntegerValue),
    Float(FloatValue),
    Bool(bool),
}

impl ScalarValue {
    pub const fn ty(self) -> ScalarType {
        match self {
            Self::Integer(value) => ScalarType::Integer(value.ty),
            Self::Float(value) => ScalarType::Float(value.ty),
            Self::Bool(_) => ScalarType::Bool,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    pub id: LocalId,
    pub name: String,
    pub ty: Type,
    pub writable: bool,
    pub owned: bool,
    pub synthetic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Scalar(ScalarType),
    String,
    NullableString,
    Class(ClassId),
}

impl From<ScalarType> for Type {
    fn from(value: ScalarType) -> Self {
        Self::Scalar(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicBlock {
    pub id: BlockId,
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Scalar(ScalarValue),
    Local(LocalId),
    Static(StaticId),
    Property {
        object: LocalId,
        property: PropertyId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rvalue {
    Value(ValueExpression),
    String(StringExpression),
    NullableString(NullableStringExpression),
    Class(ClassExpression),
}

impl Rvalue {
    pub const fn ty(&self) -> Type {
        match self {
            Self::Value(value) => Type::Scalar(value.ty()),
            Self::String(_) => Type::String,
            Self::NullableString(_) => Type::NullableString,
            Self::Class(value) => Type::Class(value.class()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassExpression {
    Local {
        class: ClassId,
        local: LocalId,
        transfer: bool,
    },
    Property {
        class: ClassId,
        object: LocalId,
        property: PropertyId,
    },
    Call {
        class: ClassId,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    New {
        class: ClassId,
        properties: Vec<PropertyValue>,
        constructor: Option<FunctionId>,
        args: Vec<Rvalue>,
    },
}

impl ClassExpression {
    pub const fn class(&self) -> ClassId {
        match self {
            Self::Local { class, .. }
            | Self::Property { class, .. }
            | Self::Call { class, .. }
            | Self::New { class, .. } => *class,
        }
    }

    pub const fn owned_temporary_class(&self) -> Option<ClassId> {
        match self {
            Self::New { class, .. }
            | Self::Call {
                class,
                return_borrow: None,
                ..
            } => Some(*class),
            Self::Local { .. }
            | Self::Property { .. }
            | Self::Call {
                return_borrow: Some(_),
                ..
            } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyValue {
    pub property: PropertyId,
    pub source: PropertyValueSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyValueSource {
    Expression(Rvalue),
    ConstructorArgument(usize),
    ConstructorBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueExpression {
    Integer(IntegerExpression),
    Float(FloatExpression),
    Bool(BoolExpression),
}

impl ValueExpression {
    pub const fn ty(&self) -> ScalarType {
        match self {
            Self::Integer(value) => ScalarType::Integer(value.ty()),
            Self::Float(value) => ScalarType::Float(value.ty()),
            Self::Bool(_) => ScalarType::Bool,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerUnaryOp {
    Negate,
    BitwiseNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerBinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    ShiftLeft,
    ShiftRight,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegerExpression {
    Use {
        ty: IntegerType,
        operand: Operand,
    },
    Unary {
        ty: IntegerType,
        op: IntegerUnaryOp,
        operand: Box<IntegerExpression>,
    },
    Binary {
        ty: IntegerType,
        op: IntegerBinaryOp,
        left: Box<IntegerExpression>,
        right: Box<IntegerExpression>,
    },
    Convert {
        ty: IntegerType,
        value: Box<IntegerExpression>,
    },
    FloatToInt {
        value: Box<FloatExpression>,
    },
    Call {
        ty: IntegerType,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
}

impl IntegerExpression {
    pub const fn ty(&self) -> IntegerType {
        match self {
            Self::Use { ty, .. }
            | Self::Unary { ty, .. }
            | Self::Binary { ty, .. }
            | Self::Convert { ty, .. }
            | Self::Call { ty, .. } => *ty,
            Self::FloatToInt { .. } => IntegerType::Int64,
        }
    }

    pub const fn use_operand(ty: IntegerType, operand: Operand) -> Self {
        Self::Use { ty, operand }
    }

    pub const fn constant(value: IntegerValue) -> Self {
        Self::Use {
            ty: value.ty,
            operand: Operand::Scalar(ScalarValue::Integer(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatBinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FloatExpression {
    Use {
        ty: FloatType,
        operand: Operand,
    },
    Negate {
        ty: FloatType,
        operand: Box<FloatExpression>,
    },
    Binary {
        ty: FloatType,
        op: FloatBinaryOp,
        left: Box<FloatExpression>,
        right: Box<FloatExpression>,
    },
    IntToFloat {
        value: Box<IntegerExpression>,
    },
    Call {
        ty: FloatType,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
}

impl FloatExpression {
    pub const fn ty(&self) -> FloatType {
        match self {
            Self::Use { ty, .. }
            | Self::Negate { ty, .. }
            | Self::Binary { ty, .. }
            | Self::Call { ty, .. } => *ty,
            Self::IntToFloat { .. } => FloatType::Float64,
        }
    }

    pub const fn constant(value: FloatValue) -> Self {
        Self::Use {
            ty: value.ty,
            operand: Operand::Scalar(ScalarValue::Float(value)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringExpression {
    Literal(String),
    Local(LocalId),
    NullableLocalAssumeNonNull(LocalId),
    Property {
        object: LocalId,
        property: PropertyId,
    },
    Static(StaticId),
    Concat(Vec<StringExpression>),
    Display(ValueExpression),
    Call {
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    ReadFile(Box<StringExpression>),
    Format(Box<FormatExpression>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullableStringExpression {
    Null,
    String(StringExpression),
    Local(LocalId),
    Property {
        object: LocalId,
        property: PropertyId,
    },
    Static(StaticId),
    ReadLine,
    Call {
        function: FunctionId,
        args: Vec<Rvalue>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatArgument {
    Value(ValueExpression),
    String(StringExpression),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatExpression {
    pub pieces: Vec<FormatPiece>,
    pub arguments: Vec<FormatArgument>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoolExpression {
    Use {
        operand: Operand,
    },
    Compare {
        op: CompareOp,
        left: Box<ValueExpression>,
        right: Box<ValueExpression>,
    },
    StringCompare {
        op: CompareOp,
        left: Box<StringExpression>,
        right: Box<StringExpression>,
    },
    NullableStringCompare {
        op: CompareOp,
        left: Box<NullableStringExpression>,
        right: Box<NullableStringExpression>,
    },
    Not(Box<BoolExpression>),
    Binary {
        op: BoolBinaryOp,
        left: Box<BoolExpression>,
        right: Box<BoolExpression>,
    },
    Call {
        function: FunctionId,
        args: Vec<Rvalue>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolBinaryOp {
    And,
    Or,
    Xor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    AssignLocal {
        target: LocalId,
        value: Rvalue,
    },
    EchoStringLiteral(String),
    EchoString(StringExpression),
    CallVoid {
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    CallBorrowed {
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    Printf(FormatExpression),
    WriteFile {
        path: StringExpression,
        contents: StringExpression,
    },
    WriteStderr(StringExpression),
    AssignProperty {
        object: LocalId,
        property: PropertyId,
        value: Rvalue,
    },
    AssignStatic {
        target: StaticId,
        value: Rvalue,
    },
    DropClass {
        local: LocalId,
        class: ClassId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminator {
    Return(Rvalue),
    ReturnVoid,
    Panic(StringExpression),
    Unreachable,
    Jump(BlockId),
    Branch {
        condition: BoolExpression,
        then_block: BlockId,
        else_block: BlockId,
    },
}

pub(crate) fn class_temporary_capacity(function: &Function) -> usize {
    function
        .blocks
        .iter()
        .map(|block| {
            block
                .statements
                .iter()
                .map(statement_class_temporary_capacity)
                .sum::<usize>()
                + terminator_class_temporary_capacity(&block.terminator)
        })
        .sum()
}

fn statement_class_temporary_capacity(statement: &Statement) -> usize {
    match statement {
        Statement::AssignLocal { value, .. }
        | Statement::AssignProperty { value, .. }
        | Statement::AssignStatic { value, .. } => rvalue_class_temporary_capacity(value),
        Statement::EchoStringLiteral(_) | Statement::DropClass { .. } => 0,
        Statement::EchoString(value) | Statement::WriteStderr(value) => {
            string_class_temporary_capacity(value)
        }
        Statement::CallVoid { args, .. } | Statement::CallBorrowed { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        Statement::Printf(format) => format_class_temporary_capacity(format),
        Statement::WriteFile { path, contents } => {
            string_class_temporary_capacity(path) + string_class_temporary_capacity(contents)
        }
    }
}

fn terminator_class_temporary_capacity(terminator: &Terminator) -> usize {
    match terminator {
        Terminator::Return(value) => rvalue_class_temporary_capacity(value),
        Terminator::Panic(value) => string_class_temporary_capacity(value),
        Terminator::Branch { condition, .. } => bool_class_temporary_capacity(condition),
        Terminator::ReturnVoid | Terminator::Unreachable | Terminator::Jump(_) => 0,
    }
}

fn rvalue_class_temporary_capacity(value: &Rvalue) -> usize {
    match value {
        Rvalue::Value(value) => value_class_temporary_capacity(value),
        Rvalue::String(value) => string_class_temporary_capacity(value),
        Rvalue::NullableString(value) => nullable_string_class_temporary_capacity(value),
        Rvalue::Class(value) => class_expression_temporary_capacity(value),
    }
}

fn value_class_temporary_capacity(value: &ValueExpression) -> usize {
    match value {
        ValueExpression::Integer(value) => integer_class_temporary_capacity(value),
        ValueExpression::Float(value) => float_class_temporary_capacity(value),
        ValueExpression::Bool(value) => bool_class_temporary_capacity(value),
    }
}

fn integer_class_temporary_capacity(value: &IntegerExpression) -> usize {
    match value {
        IntegerExpression::Use { .. } => 0,
        IntegerExpression::Unary { operand, .. }
        | IntegerExpression::Convert { value: operand, .. } => {
            integer_class_temporary_capacity(operand)
        }
        IntegerExpression::Binary { left, right, .. } => {
            integer_class_temporary_capacity(left) + integer_class_temporary_capacity(right)
        }
        IntegerExpression::FloatToInt { value } => float_class_temporary_capacity(value),
        IntegerExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
    }
}

fn float_class_temporary_capacity(value: &FloatExpression) -> usize {
    match value {
        FloatExpression::Use { .. } => 0,
        FloatExpression::Negate { operand, .. } => float_class_temporary_capacity(operand),
        FloatExpression::Binary { left, right, .. } => {
            float_class_temporary_capacity(left) + float_class_temporary_capacity(right)
        }
        FloatExpression::IntToFloat { value } => integer_class_temporary_capacity(value),
        FloatExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
    }
}

fn string_class_temporary_capacity(value: &StringExpression) -> usize {
    match value {
        StringExpression::Concat(parts) => parts.iter().map(string_class_temporary_capacity).sum(),
        StringExpression::Display(value) => value_class_temporary_capacity(value),
        StringExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        StringExpression::ReadFile(path) => string_class_temporary_capacity(path),
        StringExpression::Format(format) => format_class_temporary_capacity(format),
        StringExpression::Literal(_)
        | StringExpression::Local(_)
        | StringExpression::NullableLocalAssumeNonNull(_)
        | StringExpression::Static(_)
        | StringExpression::Property { .. } => 0,
    }
}

fn nullable_string_class_temporary_capacity(value: &NullableStringExpression) -> usize {
    match value {
        NullableStringExpression::String(value) => string_class_temporary_capacity(value),
        NullableStringExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullableStringExpression::Null
        | NullableStringExpression::Local(_)
        | NullableStringExpression::Static(_)
        | NullableStringExpression::Property { .. }
        | NullableStringExpression::ReadLine => 0,
    }
}

fn class_expression_temporary_capacity(value: &ClassExpression) -> usize {
    match value {
        ClassExpression::Local { .. } | ClassExpression::Property { .. } => 0,
        ClassExpression::Call { args, .. } => {
            usize::from(value.owned_temporary_class().is_some())
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
        ClassExpression::New {
            properties, args, ..
        } => {
            1 + properties
                .iter()
                .filter_map(|property| match &property.source {
                    PropertyValueSource::Expression(value) => {
                        Some(rvalue_class_temporary_capacity(value))
                    }
                    PropertyValueSource::ConstructorArgument(_)
                    | PropertyValueSource::ConstructorBody => None,
                })
                .sum::<usize>()
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
    }
}

pub(crate) fn bool_class_temporary_capacity(value: &BoolExpression) -> usize {
    match value {
        BoolExpression::Use { .. } => 0,
        BoolExpression::Compare { left, right, .. } => {
            value_class_temporary_capacity(left) + value_class_temporary_capacity(right)
        }
        BoolExpression::StringCompare { left, right, .. } => {
            string_class_temporary_capacity(left) + string_class_temporary_capacity(right)
        }
        BoolExpression::NullableStringCompare { left, right, .. } => {
            nullable_string_class_temporary_capacity(left)
                + nullable_string_class_temporary_capacity(right)
        }
        BoolExpression::Not(value) => bool_class_temporary_capacity(value),
        BoolExpression::Binary { left, right, .. } => {
            bool_class_temporary_capacity(left) + bool_class_temporary_capacity(right)
        }
        BoolExpression::Call { args, .. } => args.iter().map(rvalue_class_temporary_capacity).sum(),
    }
}

fn format_class_temporary_capacity(format: &FormatExpression) -> usize {
    format
        .arguments
        .iter()
        .map(|argument| match argument {
            FormatArgument::Value(value) => value_class_temporary_capacity(value),
            FormatArgument::String(value) => string_class_temporary_capacity(value),
        })
        .sum()
}

impl fmt::Display for Program {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, function) in self.functions.iter().enumerate() {
            if index > 0 {
                writeln!(formatter)?;
            }
            write!(formatter, "{function}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Function {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "function {}(", self.name)?;
        for (index, parameter) in self.params.iter().enumerate() {
            if index > 0 {
                write!(formatter, ", ")?;
            }
            if let Some(local) = self
                .locals
                .get(parameter.0)
                .filter(|local| local.id == *parameter)
            {
                write!(formatter, "${}: {}", local.name, local.ty)?;
            } else {
                write!(formatter, "local{}", parameter.0)?;
            }
        }
        writeln!(formatter, "): {} {{", self.return_type)?;
        if !self.locals.is_empty() {
            writeln!(formatter, "locals:")?;
            for local in &self.locals {
                writeln!(formatter, "    {local}")?;
            }
        }
        for block in &self.blocks {
            write!(formatter, "{block}")?;
        }
        writeln!(formatter, "}}")
    }
}

impl fmt::Display for ReturnType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReturnType::Value(ty) => write!(formatter, "{ty}"),
            ReturnType::Void => write!(formatter, "void"),
        }
    }
}

impl fmt::Display for Local {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let role = if self.synthetic {
            "temp"
        } else if self.writable {
            "writable"
        } else {
            "readonly"
        };
        let name = if self.synthetic {
            self.name.clone()
        } else {
            format!("${}", self.name)
        };
        write!(
            formatter,
            "local{} {} {}: {}",
            self.id.0, role, name, self.ty
        )
    }
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Scalar(ty) => write!(formatter, "{ty}"),
            Type::String => write!(formatter, "string"),
            Type::NullableString => write!(formatter, "?string"),
            Type::Class(class) => write!(formatter, "class#{}", class.0),
        }
    }
}

impl fmt::Display for BasicBlock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "block{}:", self.id.0)?;
        for statement in &self.statements {
            writeln!(formatter, "    {statement}")?;
        }
        writeln!(formatter, "    {}", self.terminator)
    }
}

impl fmt::Display for Operand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Scalar(value) => write!(formatter, "{value}"),
            Operand::Local(id) => write!(formatter, "local{}", id.0),
            Operand::Static(id) => write!(formatter, "static{}", id.0),
            Operand::Property { object, property } => {
                write!(
                    formatter,
                    "local{}->property#{}:{}",
                    object.0, property.class.0, property.index
                )
            }
        }
    }
}

impl fmt::Display for Rvalue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rvalue::Value(expression) => write!(formatter, "{expression}"),
            Rvalue::String(value) => write!(formatter, "{value}"),
            Rvalue::NullableString(value) => write!(formatter, "{value}"),
            Rvalue::Class(value) => write!(formatter, "{value}"),
        }
    }
}

impl fmt::Display for ClassExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local {
                local,
                transfer: true,
                ..
            } => write!(formatter, "move local{}", local.0),
            Self::Local {
                local,
                transfer: false,
                ..
            } => write!(formatter, "borrow local{}", local.0),
            Self::Property {
                object, property, ..
            } => write!(
                formatter,
                "borrow local{}->property#{}:{}",
                object.0, property.class.0, property.index
            ),
            Self::Call {
                class, function, ..
            } => {
                write!(formatter, "call fn{} -> class#{}", function.0, class.0)
            }
            Self::New { class, .. } => write!(formatter, "new class#{}", class.0),
        }
    }
}

impl fmt::Display for ScalarType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(ty) => write!(formatter, "{ty}"),
            Self::Float(ty) => write!(formatter, "{ty}"),
            Self::Bool => formatter.write_str("bool"),
        }
    }
}

impl fmt::Display for ScalarValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(value) => write!(formatter, "{value}: {}", value.ty),
            Self::Float(value) => match value.ty {
                FloatType::Float32 => write!(formatter, "0x{:08x}: float32", value.bits),
                FloatType::Float64 => write!(formatter, "0x{:016x}: float", value.bits),
            },
            Self::Bool(value) => write!(formatter, "{value}: bool"),
        }
    }
}

impl fmt::Display for ValueExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(value) => write!(formatter, "{value}"),
            Self::Float(value) => write!(formatter, "{value}"),
            Self::Bool(value) => write!(formatter, "{value}"),
        }
    }
}

impl fmt::Display for IntegerUnaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntegerUnaryOp::Negate => write!(formatter, "-"),
            IntegerUnaryOp::BitwiseNot => write!(formatter, "~"),
        }
    }
}

impl fmt::Display for IntegerBinaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntegerBinaryOp::Add => write!(formatter, "+"),
            IntegerBinaryOp::Subtract => write!(formatter, "-"),
            IntegerBinaryOp::Multiply => write!(formatter, "*"),
            IntegerBinaryOp::Divide => write!(formatter, "/"),
            IntegerBinaryOp::Remainder => write!(formatter, "%"),
            IntegerBinaryOp::ShiftLeft => write!(formatter, "<<"),
            IntegerBinaryOp::ShiftRight => write!(formatter, ">>"),
            IntegerBinaryOp::BitwiseAnd => write!(formatter, "&"),
            IntegerBinaryOp::BitwiseXor => write!(formatter, "^"),
            IntegerBinaryOp::BitwiseOr => write!(formatter, "|"),
        }
    }
}

impl fmt::Display for IntegerExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntegerExpression::Use { ty, operand } => match operand {
                Operand::Scalar(ScalarValue::Integer(value)) => write!(formatter, "{value}: {ty}"),
                Operand::Local(id) => write!(formatter, "local{}: {ty}", id.0),
                Operand::Static(id) => write!(formatter, "static{}: {ty}", id.0),
                Operand::Property { object, property } => write!(
                    formatter,
                    "local{}->property{}: {ty}",
                    object.0, property.index
                ),
                Operand::Scalar(_) => write!(formatter, "<malformed scalar>: {ty}"),
            },
            IntegerExpression::Unary { ty, op, operand } => {
                write!(formatter, "({op}{operand}): {ty}")
            }
            IntegerExpression::Binary {
                ty,
                op,
                left,
                right,
            } => write!(formatter, "({left} {op} {right}): {ty}"),
            IntegerExpression::Convert { ty, value } => {
                write!(formatter, "convert<{ty}>({value}): {ty}")
            }
            IntegerExpression::FloatToInt { value } => {
                write!(formatter, "Float::toInt({value}): int")
            }
            IntegerExpression::Call { ty, function, args } => {
                write_call(formatter, *function, args)?;
                write!(formatter, ": {ty}")
            }
        }
    }
}

impl fmt::Display for FloatBinaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
        })
    }
}

impl fmt::Display for FloatExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Use { ty, operand } => match operand {
                Operand::Scalar(ScalarValue::Float(value)) => write!(formatter, "{value}: {ty}"),
                Operand::Local(id) => write!(formatter, "local{}: {ty}", id.0),
                Operand::Static(id) => write!(formatter, "static{}: {ty}", id.0),
                Operand::Property { object, property } => write!(
                    formatter,
                    "local{}->property{}: {ty}",
                    object.0, property.index
                ),
                Operand::Scalar(_) => write!(formatter, "<malformed scalar>: {ty}"),
            },
            Self::Negate { ty, operand } => write!(formatter, "(-{operand}): {ty}"),
            Self::Binary {
                ty,
                op,
                left,
                right,
            } => write!(formatter, "({left} {op} {right}): {ty}"),
            Self::IntToFloat { value } => write!(formatter, "Int::toFloat({value}): float"),
            Self::Call { ty, function, args } => {
                write_call(formatter, *function, args)?;
                write!(formatter, ": {ty}")
            }
        }
    }
}

impl fmt::Display for StringExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StringExpression::Literal(value) => {
                write!(formatter, "\"{}\"", escape_debug_string(value))
            }
            StringExpression::Local(id) => write!(formatter, "local{}", id.0),
            StringExpression::NullableLocalAssumeNonNull(id) => {
                write!(formatter, "nonnull(local{})", id.0)
            }
            StringExpression::Property { object, property } => {
                write!(formatter, "local{}->property{}", object.0, property.index)
            }
            StringExpression::Static(id) => write!(formatter, "static{}", id.0),
            StringExpression::Concat(parts) => {
                write!(formatter, "(")?;
                for (index, part) in parts.iter().enumerate() {
                    if index > 0 {
                        write!(formatter, " . ")?;
                    }
                    write!(formatter, "{part}")?;
                }
                write!(formatter, ")")
            }
            StringExpression::Display(value) => write!(formatter, "display({value})"),
            StringExpression::Call { function, args } => write_call(formatter, *function, args),
            StringExpression::ReadFile(path) => write!(formatter, "read_file({path})"),
            StringExpression::Format(format) => write!(formatter, "format({format})"),
        }
    }
}

impl fmt::Display for NullableStringExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => formatter.write_str("null"),
            Self::String(value) => write!(formatter, "some({value})"),
            Self::Local(local) => write!(formatter, "local{}", local.0),
            Self::Property { object, property } => {
                write!(formatter, "local{}->property{}", object.0, property.index)
            }
            Self::Static(id) => write!(formatter, "static{}", id.0),
            Self::ReadLine => formatter.write_str("read_line()"),
            Self::Call { function, args } => write_call(formatter, *function, args),
        }
    }
}

impl fmt::Display for FormatArgument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(value) => write!(formatter, "{value}"),
            Self::String(value) => write!(formatter, "{value}"),
        }
    }
}

impl fmt::Display for FormatExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "plan[{} pieces]", self.pieces.len())?;
        write!(formatter, "(")?;
        for (index, argument) in self.arguments.iter().enumerate() {
            if index != 0 {
                write!(formatter, ", ")?;
            }
            write!(formatter, "{argument}")?;
        }
        write!(formatter, ")")
    }
}

impl fmt::Display for BoolExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Use { operand } => match operand {
                Operand::Scalar(ScalarValue::Bool(value)) => write!(formatter, "{value}: bool"),
                Operand::Local(id) => write!(formatter, "local{}: bool", id.0),
                Operand::Static(id) => write!(formatter, "static{}: bool", id.0),
                Operand::Property { object, property } => {
                    write!(
                        formatter,
                        "local{}->property{}: bool",
                        object.0, property.index
                    )
                }
                Operand::Scalar(_) => formatter.write_str("<malformed scalar>: bool"),
            },
            Self::Compare { op, left, right } => write!(formatter, "{left} {op} {right}"),
            Self::StringCompare { op, left, right } => write!(formatter, "{left} {op} {right}"),
            Self::NullableStringCompare { op, left, right } => {
                write!(formatter, "{left} {op} {right}")
            }
            Self::Not(condition) => write!(formatter, "!({condition})"),
            Self::Binary { op, left, right } => {
                write!(formatter, "({left}) {op} ({right})")
            }
            Self::Call { function, args } => write_call(formatter, *function, args),
        }
    }
}

impl fmt::Display for CompareOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompareOp::Equal => write!(formatter, "=="),
            CompareOp::NotEqual => write!(formatter, "!="),
            CompareOp::Less => write!(formatter, "<"),
            CompareOp::LessEqual => write!(formatter, "<="),
            CompareOp::Greater => write!(formatter, ">"),
            CompareOp::GreaterEqual => write!(formatter, ">="),
        }
    }
}

impl fmt::Display for BoolBinaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoolBinaryOp::And => write!(formatter, "&&"),
            BoolBinaryOp::Or => write!(formatter, "||"),
            BoolBinaryOp::Xor => write!(formatter, "xor"),
        }
    }
}

impl fmt::Display for Statement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Statement::AssignLocal { target, value } => {
                write!(formatter, "local{} = {value}", target.0)
            }
            Statement::EchoStringLiteral(value) => {
                write!(formatter, "echo \"{}\"", escape_debug_string(value))
            }
            Statement::EchoString(value) => write!(formatter, "echo {value}"),
            Statement::CallVoid { function, args } | Statement::CallBorrowed { function, args } => {
                write_call(formatter, *function, args)
            }
            Statement::Printf(format) => write!(formatter, "printf {format}"),
            Statement::WriteFile { path, contents } => {
                write!(formatter, "write_file({path}, {contents})")
            }
            Statement::WriteStderr(value) => write!(formatter, "write_stderr({value})"),
            Statement::AssignProperty {
                object,
                property,
                value,
            } => write!(
                formatter,
                "local{}->property{} = {value}",
                object.0, property.index
            ),
            Statement::AssignStatic { target, value } => {
                write!(formatter, "static{} = {value}", target.0)
            }
            Statement::DropClass { local, class } => {
                write!(formatter, "drop class#{} local{}", class.0, local.0)
            }
        }
    }
}

impl fmt::Display for Terminator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Terminator::Return(operand) => write!(formatter, "return {operand}"),
            Terminator::ReturnVoid => write!(formatter, "return"),
            Terminator::Panic(message) => write!(formatter, "panic {message}"),
            Terminator::Unreachable => write!(formatter, "unreachable"),
            Terminator::Jump(target) => write!(formatter, "jump block{}", target.0),
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => write!(
                formatter,
                "branch {condition} -> block{}, block{}",
                then_block.0, else_block.0
            ),
        }
    }
}

fn escape_debug_string(value: &str) -> String {
    value.escape_default().collect()
}

fn write_call<T: fmt::Display>(
    formatter: &mut fmt::Formatter<'_>,
    function: FunctionId,
    args: &[T],
) -> fmt::Result {
    write!(formatter, "call function{}(", function.0)?;
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            write!(formatter, ", ")?;
        }
        write!(formatter, "{arg}")?;
    }
    write!(formatter, ")")
}
