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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CollectionTypeId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub classes: Vec<Class>,
    pub collection_types: Vec<CollectionType>,
    pub statics: Vec<StaticProperty>,
    pub functions: Vec<Function>,
    pub entry: FunctionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CollectionKind {
    Bytes,
    TypedArray,
    List,
    Dictionary,
    Set,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionType {
    pub id: CollectionTypeId,
    pub kind: CollectionKind,
    pub key: Option<Type>,
    pub value: Type,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    Scalar(ScalarType),
    String,
    NullableScalar(ScalarType),
    NullableString,
    Class(ClassId),
    NullableClass(ClassId),
    Collection(CollectionTypeId),
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
    NullablePayload(LocalId),
    Static(StaticId),
    Property {
        object: LocalId,
        property: PropertyId,
    },
    CollectionLength(LocalId),
    CollectionIndex {
        collection: LocalId,
        index: Box<Rvalue>,
        remove: bool,
    },
    CollectionKeyAt {
        collection: LocalId,
        offset: Box<Rvalue>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rvalue {
    Value(ValueExpression),
    String(StringExpression),
    NullableScalar(NullableScalarExpression),
    NullableString(NullableStringExpression),
    Class(ClassExpression),
    NullableClass(NullableClassExpression),
    Collection(CollectionExpression),
}

impl Rvalue {
    pub const fn ty(&self) -> Type {
        match self {
            Self::Value(value) => Type::Scalar(value.ty()),
            Self::String(_) => Type::String,
            Self::NullableScalar(value) => Type::NullableScalar(value.ty()),
            Self::NullableString(_) => Type::NullableString,
            Self::Class(value) => Type::Class(value.class()),
            Self::NullableClass(value) => Type::NullableClass(value.class()),
            Self::Collection(value) => Type::Collection(value.collection()),
        }
    }

    pub const fn owned_temporary_class(&self) -> Option<ClassId> {
        match self {
            Self::Class(value) => value.owned_temporary_class(),
            Self::NullableClass(value) => value.owned_temporary_class(),
            Self::Collection(_) => None,
            Self::Value(_)
            | Self::String(_)
            | Self::NullableScalar(_)
            | Self::NullableString(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionExpression {
    Local {
        collection: CollectionTypeId,
        local: LocalId,
        transfer: bool,
    },
    Literal {
        collection: CollectionTypeId,
        entries: Vec<CollectionEntry>,
    },
    Index {
        collection: CollectionTypeId,
        source: LocalId,
        index: Box<Rvalue>,
        transfer: bool,
    },
    Property {
        collection: CollectionTypeId,
        object: LocalId,
        property: PropertyId,
    },
    SetFrom {
        collection: CollectionTypeId,
        source: LocalId,
        transfer: bool,
        algebra: Option<(SetAlgebraOp, LocalId)>,
    },
    FromBytes {
        collection: CollectionTypeId,
        source: LocalId,
    },
    BytesFromArray {
        collection: CollectionTypeId,
        source: LocalId,
    },
    ReadFileBytes {
        collection: CollectionTypeId,
        path: Box<StringExpression>,
    },
    ReadStdinBytes {
        collection: CollectionTypeId,
    },
    Call {
        collection: CollectionTypeId,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetAlgebraOp {
    Union,
    Intersect,
    Difference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullableCollectionAccess {
    Get,
    Remove,
    First,
    Last,
    Pop,
}

impl CollectionExpression {
    pub const fn collection(&self) -> CollectionTypeId {
        match self {
            Self::Local { collection, .. }
            | Self::Literal { collection, .. }
            | Self::Index { collection, .. }
            | Self::Property { collection, .. }
            | Self::SetFrom { collection, .. }
            | Self::FromBytes { collection, .. }
            | Self::BytesFromArray { collection, .. }
            | Self::ReadFileBytes { collection, .. }
            | Self::ReadStdinBytes { collection }
            | Self::Call { collection, .. } => *collection,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionEntry {
    pub key: Option<Rvalue>,
    pub value: Rvalue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullableScalarExpression {
    Null(ScalarType),
    Value(ValueExpression),
    Local {
        ty: ScalarType,
        local: LocalId,
    },
    Property {
        ty: ScalarType,
        object: LocalId,
        property: PropertyId,
    },
    Static {
        ty: ScalarType,
        id: StaticId,
    },
    Call {
        ty: ScalarType,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    NullSafeProperty {
        ty: ScalarType,
        object: Box<NullableClassExpression>,
        property: PropertyId,
    },
    NullSafeCall {
        ty: ScalarType,
        object: Box<NullableClassExpression>,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    Coalesce {
        ty: ScalarType,
        left: Box<NullableScalarExpression>,
        right: Box<NullableScalarExpression>,
    },
    DictionaryGet {
        ty: ScalarType,
        collection: LocalId,
        key: Box<Rvalue>,
        access: NullableCollectionAccess,
    },
}

impl NullableScalarExpression {
    pub const fn ty(&self) -> ScalarType {
        match self {
            Self::Null(ty)
            | Self::Local { ty, .. }
            | Self::Property { ty, .. }
            | Self::Static { ty, .. }
            | Self::Call { ty, .. }
            | Self::NullSafeProperty { ty, .. }
            | Self::NullSafeCall { ty, .. }
            | Self::Coalesce { ty, .. }
            | Self::DictionaryGet { ty, .. } => *ty,
            Self::Value(value) => value.ty(),
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
    NullableLocalAssumeNonNull {
        class: ClassId,
        local: LocalId,
        transfer: bool,
    },
    Coalesce {
        class: ClassId,
        left: Box<NullableClassExpression>,
        right: Box<ClassExpression>,
        transfer: bool,
    },
    CollectionIndex {
        class: ClassId,
        collection: LocalId,
        index: Box<Rvalue>,
        transfer: bool,
    },
}

impl ClassExpression {
    pub const fn class(&self) -> ClassId {
        match self {
            Self::Local { class, .. }
            | Self::Property { class, .. }
            | Self::Call { class, .. }
            | Self::New { class, .. }
            | Self::NullableLocalAssumeNonNull { class, .. }
            | Self::Coalesce { class, .. }
            | Self::CollectionIndex { class, .. } => *class,
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
            | Self::NullableLocalAssumeNonNull { .. }
            | Self::Coalesce { .. }
            | Self::CollectionIndex { .. }
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
    Coalesce {
        ty: IntegerType,
        left: Box<NullableScalarExpression>,
        right: Box<IntegerExpression>,
    },
}

impl IntegerExpression {
    pub const fn ty(&self) -> IntegerType {
        match self {
            Self::Use { ty, .. }
            | Self::Unary { ty, .. }
            | Self::Binary { ty, .. }
            | Self::Convert { ty, .. }
            | Self::Call { ty, .. }
            | Self::Coalesce { ty, .. } => *ty,
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
    Coalesce {
        ty: FloatType,
        left: Box<NullableScalarExpression>,
        right: Box<FloatExpression>,
    },
}

impl FloatExpression {
    pub const fn ty(&self) -> FloatType {
        match self {
            Self::Use { ty, .. }
            | Self::Negate { ty, .. }
            | Self::Binary { ty, .. }
            | Self::Call { ty, .. }
            | Self::Coalesce { ty, .. } => *ty,
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
    Coalesce {
        left: Box<NullableStringExpression>,
        right: Box<StringExpression>,
    },
    CollectionIndex {
        collection: LocalId,
        index: Box<Rvalue>,
        remove: bool,
    },
    CollectionKeyAt {
        collection: LocalId,
        offset: Box<Rvalue>,
    },
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
    NullSafeProperty {
        object: Box<NullableClassExpression>,
        property: PropertyId,
    },
    NullSafeCall {
        object: Box<NullableClassExpression>,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    Coalesce {
        left: Box<NullableStringExpression>,
        right: Box<NullableStringExpression>,
    },
    DictionaryGet {
        collection: LocalId,
        key: Box<Rvalue>,
        access: NullableCollectionAccess,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullableClassExpression {
    Null(ClassId),
    Class(ClassExpression),
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
    NullSafeProperty {
        class: ClassId,
        object: Box<NullableClassExpression>,
        property: PropertyId,
    },
    NullSafeCall {
        class: ClassId,
        object: Box<NullableClassExpression>,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    Coalesce {
        class: ClassId,
        left: Box<NullableClassExpression>,
        right: Box<NullableClassExpression>,
        transfer: bool,
    },
    DictionaryGet {
        class: ClassId,
        collection: LocalId,
        key: Box<Rvalue>,
        access: NullableCollectionAccess,
    },
}

impl NullableClassExpression {
    pub const fn class(&self) -> ClassId {
        match self {
            Self::Null(class)
            | Self::Local { class, .. }
            | Self::Property { class, .. }
            | Self::Call { class, .. }
            | Self::NullSafeProperty { class, .. }
            | Self::NullSafeCall { class, .. }
            | Self::Coalesce { class, .. }
            | Self::DictionaryGet { class, .. } => *class,
            Self::Class(value) => value.class(),
        }
    }

    pub const fn owned_temporary_class(&self) -> Option<ClassId> {
        match self {
            Self::Class(value) => value.owned_temporary_class(),
            Self::Call {
                class,
                return_borrow: None,
                ..
            }
            | Self::NullSafeCall {
                class,
                return_borrow: None,
                ..
            } => Some(*class),
            Self::Null(_)
            | Self::Local { .. }
            | Self::Property { .. }
            | Self::Call {
                return_borrow: Some(_),
                ..
            }
            | Self::NullSafeProperty { .. }
            | Self::Coalesce { .. }
            | Self::DictionaryGet { .. }
            | Self::NullSafeCall {
                return_borrow: Some(_),
                ..
            } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatArgument {
    Value(ValueExpression),
    String(StringExpression),
    ClassDisplay(StringExpression),
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
    NullableScalarIsPresent(Box<NullableScalarExpression>),
    NullableClassIsPresent(Box<NullableClassExpression>),
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
    Coalesce {
        left: Box<NullableScalarExpression>,
        right: Box<BoolExpression>,
    },
    CollectionHas {
        collection: LocalId,
        value: Box<Rvalue>,
        op: CollectionMembershipOp,
    },
    CollectionIsEmpty {
        collection: LocalId,
    },
    CollectionEqual {
        left: LocalId,
        right: LocalId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionMembershipOp {
    Contains,
    Add,
    Remove,
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
    CallNullSafe {
        object: NullableClassExpression,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    Printf(FormatExpression),
    WriteFile {
        path: StringExpression,
        contents: StringExpression,
    },
    AppendFile {
        path: StringExpression,
        contents: StringExpression,
    },
    WriteStderr(StringExpression),
    WriteFileBytes {
        path: StringExpression,
        contents: LocalId,
        append: bool,
    },
    WriteStreamBytes {
        contents: LocalId,
        stderr: bool,
    },
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
    DropString {
        local: LocalId,
    },
    CollectionAdd {
        collection: LocalId,
        value: Rvalue,
        index: Option<Rvalue>,
        op: CollectionMutationOp,
    },
    CollectionSet {
        collection: LocalId,
        key: Rvalue,
        value: Rvalue,
    },
    AssignCollectionIndex {
        collection: LocalId,
        index: Rvalue,
        value: Rvalue,
    },
    DropCollection {
        local: LocalId,
        collection: CollectionTypeId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionMutationOp {
    Add,
    InsertAt,
    Remove,
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
        Statement::CollectionAdd { value, .. } => rvalue_class_temporary_capacity(value),
        Statement::CollectionSet { key, value, .. } => {
            rvalue_class_temporary_capacity(key) + rvalue_class_temporary_capacity(value)
        }
        Statement::AssignCollectionIndex { index, value, .. } => {
            rvalue_class_temporary_capacity(index) + rvalue_class_temporary_capacity(value)
        }
        Statement::EchoStringLiteral(_)
        | Statement::DropClass { .. }
        | Statement::DropString { .. }
        | Statement::DropCollection { .. }
        | Statement::WriteStreamBytes { .. } => 0,
        Statement::EchoString(value) | Statement::WriteStderr(value) => {
            string_class_temporary_capacity(value)
        }
        Statement::CallVoid { args, .. } | Statement::CallBorrowed { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        Statement::CallNullSafe { object, args, .. } => {
            nullable_class_temporary_capacity(object)
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
        Statement::Printf(format) => format_class_temporary_capacity(format),
        Statement::WriteFile { path, contents } | Statement::AppendFile { path, contents } => {
            string_class_temporary_capacity(path) + string_class_temporary_capacity(contents)
        }
        Statement::WriteFileBytes { path, .. } => string_class_temporary_capacity(path),
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
        Rvalue::NullableScalar(value) => nullable_scalar_class_temporary_capacity(value),
        Rvalue::NullableString(value) => nullable_string_class_temporary_capacity(value),
        Rvalue::Class(value) => class_expression_temporary_capacity(value),
        Rvalue::NullableClass(value) => nullable_class_temporary_capacity(value),
        Rvalue::Collection(value) => collection_class_temporary_capacity(value),
    }
}

fn collection_class_temporary_capacity(value: &CollectionExpression) -> usize {
    match value {
        CollectionExpression::Local { .. }
        | CollectionExpression::SetFrom { .. }
        | CollectionExpression::FromBytes { .. }
        | CollectionExpression::BytesFromArray { .. }
        | CollectionExpression::ReadStdinBytes { .. } => 0,
        CollectionExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        CollectionExpression::Literal { entries, .. } => entries
            .iter()
            .map(|entry| {
                entry
                    .key
                    .as_ref()
                    .map_or(0, rvalue_class_temporary_capacity)
                    + rvalue_class_temporary_capacity(&entry.value)
            })
            .sum(),
        CollectionExpression::Index { index, .. } => rvalue_class_temporary_capacity(index),
        CollectionExpression::Property { .. } => 0,
        CollectionExpression::ReadFileBytes { path, .. } => string_class_temporary_capacity(path),
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
        IntegerExpression::Coalesce { left, right, .. } => {
            nullable_scalar_class_temporary_capacity(left) + integer_class_temporary_capacity(right)
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
        FloatExpression::Coalesce { left, right, .. } => {
            nullable_scalar_class_temporary_capacity(left) + float_class_temporary_capacity(right)
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
        StringExpression::Coalesce { left, right } => {
            nullable_string_class_temporary_capacity(left) + string_class_temporary_capacity(right)
        }
        StringExpression::CollectionIndex { index, .. } => rvalue_class_temporary_capacity(index),
        StringExpression::CollectionKeyAt { offset, .. } => rvalue_class_temporary_capacity(offset),
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
        NullableStringExpression::NullSafeProperty { object, .. } => {
            nullable_class_temporary_capacity(object)
        }
        NullableStringExpression::NullSafeCall { object, args, .. } => {
            nullable_class_temporary_capacity(object)
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
        NullableStringExpression::Coalesce { left, right } => {
            nullable_string_class_temporary_capacity(left)
                + nullable_string_class_temporary_capacity(right)
        }
        NullableStringExpression::DictionaryGet { key, .. } => rvalue_class_temporary_capacity(key),
        NullableStringExpression::Null
        | NullableStringExpression::Local(_)
        | NullableStringExpression::Static(_)
        | NullableStringExpression::Property { .. }
        | NullableStringExpression::ReadLine => 0,
    }
}

fn class_expression_temporary_capacity(value: &ClassExpression) -> usize {
    match value {
        ClassExpression::Local { .. }
        | ClassExpression::Property { .. }
        | ClassExpression::NullableLocalAssumeNonNull { .. } => 0,
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
        ClassExpression::Coalesce { left, right, .. } => {
            nullable_class_temporary_capacity(left) + class_expression_temporary_capacity(right)
        }
        ClassExpression::CollectionIndex { index, .. } => rvalue_class_temporary_capacity(index),
    }
}

fn nullable_scalar_class_temporary_capacity(value: &NullableScalarExpression) -> usize {
    match value {
        NullableScalarExpression::Value(value) => value_class_temporary_capacity(value),
        NullableScalarExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullableScalarExpression::NullSafeProperty { object, .. } => {
            nullable_class_temporary_capacity(object)
        }
        NullableScalarExpression::NullSafeCall { object, args, .. } => {
            nullable_class_temporary_capacity(object)
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
        NullableScalarExpression::Coalesce { left, right, .. } => {
            nullable_scalar_class_temporary_capacity(left)
                + nullable_scalar_class_temporary_capacity(right)
        }
        NullableScalarExpression::DictionaryGet { key, .. } => rvalue_class_temporary_capacity(key),
        NullableScalarExpression::Null(_)
        | NullableScalarExpression::Local { .. }
        | NullableScalarExpression::Property { .. }
        | NullableScalarExpression::Static { .. } => 0,
    }
}

fn nullable_class_temporary_capacity(value: &NullableClassExpression) -> usize {
    match value {
        NullableClassExpression::Class(value) => class_expression_temporary_capacity(value),
        NullableClassExpression::Call { args, .. } => {
            usize::from(value.owned_temporary_class().is_some())
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
        NullableClassExpression::NullSafeCall { object, args, .. } => {
            usize::from(value.owned_temporary_class().is_some())
                + nullable_class_temporary_capacity(object)
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
        NullableClassExpression::NullSafeProperty { object, .. } => {
            nullable_class_temporary_capacity(object)
        }
        NullableClassExpression::Coalesce { left, right, .. } => {
            nullable_class_temporary_capacity(left) + nullable_class_temporary_capacity(right)
        }
        NullableClassExpression::DictionaryGet { key, .. } => rvalue_class_temporary_capacity(key),
        NullableClassExpression::Null(_)
        | NullableClassExpression::Local { .. }
        | NullableClassExpression::Property { .. } => 0,
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
        BoolExpression::NullableScalarIsPresent(value) => {
            nullable_scalar_class_temporary_capacity(value)
        }
        BoolExpression::NullableClassIsPresent(value) => nullable_class_temporary_capacity(value),
        BoolExpression::Not(value) => bool_class_temporary_capacity(value),
        BoolExpression::Binary { left, right, .. } => {
            bool_class_temporary_capacity(left) + bool_class_temporary_capacity(right)
        }
        BoolExpression::Call { args, .. } => args.iter().map(rvalue_class_temporary_capacity).sum(),
        BoolExpression::Coalesce { left, right } => {
            nullable_scalar_class_temporary_capacity(left) + bool_class_temporary_capacity(right)
        }
        BoolExpression::CollectionHas { value, .. } => rvalue_class_temporary_capacity(value),
        BoolExpression::CollectionIsEmpty { .. } => 0,
        BoolExpression::CollectionEqual { .. } => 0,
    }
}

fn format_class_temporary_capacity(format: &FormatExpression) -> usize {
    format
        .arguments
        .iter()
        .map(|argument| match argument {
            FormatArgument::Value(value) => value_class_temporary_capacity(value),
            FormatArgument::String(value) | FormatArgument::ClassDisplay(value) => {
                string_class_temporary_capacity(value)
            }
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
            Type::NullableScalar(ty) => write!(formatter, "?{ty}"),
            Type::NullableString => write!(formatter, "?string"),
            Type::Class(class) => write!(formatter, "class#{}", class.0),
            Type::NullableClass(class) => write!(formatter, "?class#{}", class.0),
            Type::Collection(collection) => write!(formatter, "collection#{}", collection.0),
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
            Operand::NullablePayload(id) => write!(formatter, "payload(local{})", id.0),
            Operand::Static(id) => write!(formatter, "static{}", id.0),
            Operand::Property { object, property } => {
                write!(
                    formatter,
                    "local{}->property#{}:{}",
                    object.0, property.class.0, property.index
                )
            }
            Operand::CollectionLength(local) => {
                write!(formatter, "length(local{})", local.0)
            }
            Operand::CollectionIndex {
                collection, index, ..
            } => {
                write!(formatter, "local{}[{index}]", collection.0)
            }
            Operand::CollectionKeyAt { collection, offset } => {
                write!(formatter, "key_at(local{}, {offset})", collection.0)
            }
        }
    }
}

impl fmt::Display for Rvalue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rvalue::Value(expression) => write!(formatter, "{expression}"),
            Rvalue::String(value) => write!(formatter, "{value}"),
            Rvalue::NullableScalar(value) => write!(formatter, "{value}"),
            Rvalue::NullableString(value) => write!(formatter, "{value}"),
            Rvalue::Class(value) => write!(formatter, "{value}"),
            Rvalue::NullableClass(value) => write!(formatter, "{value}"),
            Rvalue::Collection(value) => write!(formatter, "{value}"),
        }
    }
}

impl fmt::Display for CollectionExpression {
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
            Self::Literal {
                collection,
                entries,
            } => {
                write!(formatter, "collection#{}[", collection.0)?;
                for (index, entry) in entries.iter().enumerate() {
                    if index != 0 {
                        formatter.write_str(", ")?;
                    }
                    if let Some(key) = &entry.key {
                        write!(formatter, "{key} => ")?;
                    }
                    write!(formatter, "{}", entry.value)?;
                }
                formatter.write_str("]")
            }
            Self::Index {
                source,
                index,
                transfer,
                ..
            } => write!(
                formatter,
                "{} local{}[{index}]",
                if *transfer { "move" } else { "borrow" },
                source.0
            ),
            Self::Property {
                object, property, ..
            } => write!(
                formatter,
                "borrow local{}->property{}",
                object.0, property.index
            ),
            Self::SetFrom {
                source, transfer, ..
            } => write!(
                formatter,
                "Set::from({} local{})",
                if *transfer { "move" } else { "borrow" },
                source.0
            ),
            Self::FromBytes { source, .. } => {
                write!(formatter, "local{}->toArray()", source.0)
            }
            Self::BytesFromArray { source, .. } => {
                write!(formatter, "Bytes::fromArray(local{})", source.0)
            }
            Self::ReadFileBytes { path, .. } => write!(formatter, "read_file_bytes({path})"),
            Self::ReadStdinBytes { .. } => formatter.write_str("read_stdin_bytes()"),
            Self::Call { function, args, .. } => write_call(formatter, *function, args),
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
            Self::NullableLocalAssumeNonNull {
                class,
                local,
                transfer,
            } => write!(
                formatter,
                "{} nonnull(local{}): class#{}",
                if *transfer { "move" } else { "borrow" },
                local.0,
                class.0
            ),
            Self::Coalesce { left, right, .. } => write!(formatter, "({left} ?? {right})"),
            Self::CollectionIndex {
                class,
                collection,
                index,
                transfer,
            } => write!(
                formatter,
                "{} local{}[{index}]: class#{}",
                if *transfer { "move" } else { "borrow" },
                collection.0,
                class.0
            ),
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
                Operand::NullablePayload(id) => {
                    write!(formatter, "payload(local{}): {ty}", id.0)
                }
                Operand::Static(id) => write!(formatter, "static{}: {ty}", id.0),
                Operand::Property { object, property } => write!(
                    formatter,
                    "local{}->property{}: {ty}",
                    object.0, property.index
                ),
                Operand::Scalar(_) => write!(formatter, "<malformed scalar>: {ty}"),
                Operand::CollectionLength(local) => {
                    write!(formatter, "length(local{}): {ty}", local.0)
                }
                Operand::CollectionIndex {
                    collection, index, ..
                } => {
                    write!(formatter, "local{}[{index}]: {ty}", collection.0)
                }
                Operand::CollectionKeyAt { collection, offset } => {
                    write!(formatter, "key_at(local{}, {offset}): {ty}", collection.0)
                }
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
            IntegerExpression::Coalesce { ty, left, right } => {
                write!(formatter, "({left} ?? {right}): {ty}")
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
                Operand::NullablePayload(id) => {
                    write!(formatter, "payload(local{}): {ty}", id.0)
                }
                Operand::Static(id) => write!(formatter, "static{}: {ty}", id.0),
                Operand::Property { object, property } => write!(
                    formatter,
                    "local{}->property{}: {ty}",
                    object.0, property.index
                ),
                Operand::Scalar(_) => write!(formatter, "<malformed scalar>: {ty}"),
                Operand::CollectionLength(local) => {
                    write!(formatter, "length(local{}): {ty}", local.0)
                }
                Operand::CollectionIndex {
                    collection, index, ..
                } => {
                    write!(formatter, "local{}[{index}]: {ty}", collection.0)
                }
                Operand::CollectionKeyAt { collection, offset } => {
                    write!(formatter, "key_at(local{}, {offset}): {ty}", collection.0)
                }
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
            Self::Coalesce { ty, left, right } => {
                write!(formatter, "({left} ?? {right}): {ty}")
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
            StringExpression::Coalesce { left, right } => {
                write!(formatter, "({left} ?? {right})")
            }
            StringExpression::CollectionIndex {
                collection, index, ..
            } => {
                write!(formatter, "local{}[{index}]", collection.0)
            }
            StringExpression::CollectionKeyAt { collection, offset } => {
                write!(formatter, "key_at(local{}, {offset})", collection.0)
            }
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
            Self::NullSafeProperty { object, property } => {
                write!(formatter, "{object}?->property{}", property.index)
            }
            Self::NullSafeCall {
                object,
                function,
                args,
            } => {
                write!(formatter, "{object}?->")?;
                write_call(formatter, *function, args)
            }
            Self::Coalesce { left, right } => write!(formatter, "({left} ?? {right})"),
            Self::DictionaryGet {
                collection, key, ..
            } => {
                write!(formatter, "local{}.get({key})", collection.0)
            }
        }
    }
}

impl fmt::Display for FormatArgument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(value) => write!(formatter, "{value}"),
            Self::String(value) => write!(formatter, "{value}"),
            Self::ClassDisplay(value) => write!(formatter, "class-display({value})"),
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
                Operand::NullablePayload(id) => {
                    write!(formatter, "payload(local{}): bool", id.0)
                }
                Operand::Static(id) => write!(formatter, "static{}: bool", id.0),
                Operand::Property { object, property } => {
                    write!(
                        formatter,
                        "local{}->property{}: bool",
                        object.0, property.index
                    )
                }
                Operand::Scalar(_) => formatter.write_str("<malformed scalar>: bool"),
                Operand::CollectionLength(local) => {
                    write!(formatter, "length(local{}): bool", local.0)
                }
                Operand::CollectionIndex {
                    collection, index, ..
                } => {
                    write!(formatter, "local{}[{index}]: bool", collection.0)
                }
                Operand::CollectionKeyAt { collection, offset } => {
                    write!(formatter, "key_at(local{}, {offset}): bool", collection.0)
                }
            },
            Self::Compare { op, left, right } => write!(formatter, "{left} {op} {right}"),
            Self::StringCompare { op, left, right } => write!(formatter, "{left} {op} {right}"),
            Self::NullableStringCompare { op, left, right } => {
                write!(formatter, "{left} {op} {right}")
            }
            Self::NullableScalarIsPresent(value) => write!(formatter, "present({value})"),
            Self::NullableClassIsPresent(value) => write!(formatter, "present({value})"),
            Self::Not(condition) => write!(formatter, "!({condition})"),
            Self::Binary { op, left, right } => {
                write!(formatter, "({left}) {op} ({right})")
            }
            Self::Call { function, args } => write_call(formatter, *function, args),
            Self::Coalesce { left, right } => write!(formatter, "({left} ?? {right})"),
            Self::CollectionHas {
                collection, value, ..
            } => {
                write!(formatter, "local{}.has({value})", collection.0)
            }
            Self::CollectionIsEmpty { collection } => {
                write!(formatter, "local{}.isEmpty", collection.0)
            }
            Self::CollectionEqual { left, right } => {
                write!(formatter, "local{} == local{}", left.0, right.0)
            }
        }
    }
}

impl fmt::Display for NullableScalarExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null(ty) => write!(formatter, "null: ?{ty}"),
            Self::Value(value) => write!(formatter, "some({value})"),
            Self::Local { local, .. } => write!(formatter, "local{}", local.0),
            Self::Property {
                object, property, ..
            } => {
                write!(formatter, "local{}->property{}", object.0, property.index)
            }
            Self::Static { id, .. } => write!(formatter, "static{}", id.0),
            Self::Call { function, args, .. } => write_call(formatter, *function, args),
            Self::NullSafeProperty {
                object, property, ..
            } => {
                write!(formatter, "{object}?->property{}", property.index)
            }
            Self::NullSafeCall {
                object,
                function,
                args,
                ..
            } => {
                write!(formatter, "{object}?->")?;
                write_call(formatter, *function, args)
            }
            Self::Coalesce { left, right, .. } => write!(formatter, "({left} ?? {right})"),
            Self::DictionaryGet {
                collection, key, ..
            } => {
                write!(formatter, "local{}.get({key})", collection.0)
            }
        }
    }
}

impl fmt::Display for NullableClassExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null(class) => write!(formatter, "null: ?class#{}", class.0),
            Self::Class(value) => write!(formatter, "some({value})"),
            Self::Local { local, .. } => write!(formatter, "local{}", local.0),
            Self::Property {
                object, property, ..
            } => {
                write!(formatter, "local{}->property{}", object.0, property.index)
            }
            Self::Call { function, args, .. } => write_call(formatter, *function, args),
            Self::NullSafeProperty {
                object, property, ..
            } => {
                write!(formatter, "{object}?->property{}", property.index)
            }
            Self::NullSafeCall {
                object,
                function,
                args,
                ..
            } => {
                write!(formatter, "{object}?->")?;
                write_call(formatter, *function, args)
            }
            Self::Coalesce { left, right, .. } => write!(formatter, "({left} ?? {right})"),
            Self::DictionaryGet {
                collection, key, ..
            } => {
                write!(formatter, "local{}.get({key})", collection.0)
            }
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
            Statement::CallNullSafe {
                object,
                function,
                args,
            } => {
                write!(formatter, "null_safe {object} -> ")?;
                write_call(formatter, *function, args)
            }
            Statement::Printf(format) => write!(formatter, "printf {format}"),
            Statement::WriteFile { path, contents } => {
                write!(formatter, "write_file({path}, {contents})")
            }
            Statement::AppendFile { path, contents } => {
                write!(formatter, "append_file({path}, {contents})")
            }
            Statement::WriteStderr(value) => write!(formatter, "write_stderr({value})"),
            Statement::WriteFileBytes {
                path,
                contents,
                append,
            } => write!(
                formatter,
                "{}({path}, local{})",
                if *append {
                    "append_file_bytes"
                } else {
                    "write_file_bytes"
                },
                contents.0
            ),
            Statement::WriteStreamBytes { contents, stderr } => write!(
                formatter,
                "{}(local{})",
                if *stderr {
                    "write_stderr_bytes"
                } else {
                    "write_stdout_bytes"
                },
                contents.0
            ),
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
            Statement::DropString { local } => write!(formatter, "drop string local{}", local.0),
            Statement::CollectionAdd {
                collection, value, ..
            } => {
                write!(formatter, "local{}.add({value})", collection.0)
            }
            Statement::CollectionSet {
                collection,
                key,
                value,
            } => write!(formatter, "local{}.set({key}, {value})", collection.0),
            Statement::AssignCollectionIndex {
                collection,
                index,
                value,
            } => write!(formatter, "local{}[{index}] = {value}", collection.0),
            Statement::DropCollection { local, collection } => {
                write!(
                    formatter,
                    "drop collection#{} local{}",
                    collection.0, local.0
                )
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
