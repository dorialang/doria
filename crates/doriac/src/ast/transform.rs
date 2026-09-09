//! Exhaustive syntax transformation, shared by compiler-owned expansions.

use super::*;
use crate::source::{NameSegmentRef, QualifiedNameRef};
use crate::types::*;

pub trait Transform: Sized {
    fn span(&mut self, _span: &mut Span) {}
    fn type_ref(&mut self, ty: &mut TypeRef) {
        ty.walk(self);
    }
    fn function(&mut self, function: &mut FunctionDecl) {
        function.walk(self);
    }
}

pub trait Transformable {
    fn transform<T: Transform>(&mut self, transform: &mut T);
}

impl Transformable for Span {
    fn transform<T: Transform>(&mut self, transform: &mut T) {
        transform.span(self);
    }
}
impl Transformable for TypeRef {
    fn transform<T: Transform>(&mut self, transform: &mut T) {
        transform.type_ref(self);
    }
}
impl Transformable for FunctionDecl {
    fn transform<T: Transform>(&mut self, transform: &mut T) {
        transform.function(self);
    }
}
impl<V: Transformable> Transformable for Vec<V> {
    fn transform<T: Transform>(&mut self, transform: &mut T) {
        for value in self {
            value.transform(transform);
        }
    }
}
impl<V: Transformable> Transformable for Option<V> {
    fn transform<T: Transform>(&mut self, transform: &mut T) {
        if let Some(value) = self {
            value.transform(transform);
        }
    }
}
impl<V: Transformable> Transformable for Box<V> {
    fn transform<T: Transform>(&mut self, transform: &mut T) {
        self.as_mut().transform(transform);
    }
}

macro_rules! leaves {
    ($($ty:ty),* $(,)?) => { $(impl Transformable for $ty {
        fn transform<T: Transform>(&mut self, _: &mut T) {}
    })* };
}
leaves!(
    String,
    bool,
    MemberAccess,
    AssignOp,
    IncrementOp,
    IncrementPosition,
    ClosureForm,
    ClosureCaptureMode,
    StaticQualifier,
    UnaryOp,
    BinaryOp,
    MatchOrigin,
    FunctionInvocationMode,
    FunctionTypeParameterMode
);

// Destructuring without `..` makes a newly added syntax field a compile error
// until its traversal is deliberately accounted for.
macro_rules! walk_struct {
    ($ty:ident { $($field:ident),* $(,)? }) => {
        impl $ty {
            pub fn walk<T: Transform>(&mut self, transform: &mut T) {
                let Self { $($field),* } = self;
                $($field.transform(transform);)*
            }
        }
    };
}
macro_rules! structure {
    ($ty:ident { $($field:ident),* $(,)? }) => {
        walk_struct!($ty { $($field),* });
        impl Transformable for $ty {
            fn transform<T: Transform>(&mut self, transform: &mut T) { self.walk(transform); }
        }
    };
}
macro_rules! enumeration {
    ($ty:ident { $($variant:ident $(($value:ident))? $({$($field:ident),* $(,)?})?),* $(,)? }) => {
        impl Transformable for $ty {
            fn transform<T: Transform>(&mut self, transform: &mut T) {
                match self { $(Self::$variant $(($value))? $({$($field),*})? => {
                    $($value.transform(transform);)?
                    $($($field.transform(transform);)*)?
                }),* }
            }
        }
    };
}

structure!(NameSegmentRef { text, span });
structure!(QualifiedNameRef {
    segments,
    separator_spans,
    span
});
structure!(NameRef { text, span });
walk_struct!(TypeRef {
    name,
    source_name,
    arguments,
    nullable,
    function,
    grouped
});
enumeration!(TypeArgumentRef { Type(value), Value(value) });
structure!(FunctionTypeRef {
    keyword_span,
    invocation_mode,
    invocation_modifier_span,
    parameter_list_open_span,
    parameter_list_close_span,
    parameter_list_span,
    parameters,
    colon_span,
    return_type,
    return_type_span,
    throws_clause,
    span
});
structure!(FunctionTypeParameterRef {
    ownership_mode,
    ownership_modifier_span,
    ty,
    type_span,
    span
});
structure!(FunctionTypeThrowsRef {
    keyword_span,
    entries,
    span
});
structure!(FunctionTypeEffectRef {
    ty,
    type_span,
    span
});
structure!(GroupedTypeRef {
    open_span,
    inner,
    close_span,
    span
});

enumeration!(ClassMember { Property(value), Method(value), Constant(value), Uses(value) });
structure!(PropertyDecl {
    access,
    is_static,
    writable,
    ty,
    name,
    name_span,
    initializer,
    span
});
structure!(ConstDecl {
    access,
    access_span,
    ty,
    name,
    name_span,
    initializer,
    span
});
structure!(TraitUse {
    keyword_span,
    traits,
    type_spans,
    comma_spans,
    adaptations,
    open_brace_span,
    close_brace_span,
    semicolon_span,
    span
});
structure!(TraitAdaptation {
    origin,
    origin_span,
    separator_span,
    method,
    kind,
    semicolon_span,
    span
});
enumeration!(TraitAdaptationKind {
    InsteadOf { keyword_span, excluded, type_spans, comma_spans },
    Alias { keyword_span, internal_span, alias },
});
walk_struct!(FunctionDecl {
    access,
    access_span,
    is_open,
    open_span,
    is_override,
    override_span,
    writable_this,
    writable_span,
    is_static,
    static_span,
    name,
    name_span,
    type_params,
    params,
    return_type,
    throws,
    body,
    syntax,
    modifier_prefix_span,
    span
});
enumeration!(FunctionBody { Block(value), Requirement { semicolon_span } });
structure!(FunctionSyntax {
    keyword_span,
    type_parameters,
    parameters,
    return_colon_span,
    return_type_span
});
structure!(DelimitedListSpans {
    open_span,
    comma_spans,
    close_span
});
structure!(ThrowsClause {
    keyword_span,
    entries,
    span
});
structure!(ThrowsEntry { ty, span });
structure!(TypeParamDecl {
    name,
    constraints,
    default_type,
    span
});
structure!(Param {
    constructor_role,
    role_and_mode_prefix_span,
    borrow_span,
    take,
    take_span,
    writable,
    writable_span,
    ownership_modifier_insert,
    ty,
    type_span,
    name,
    name_span,
    default,
    default_span,
    span
});
enumeration!(ConstructorParameterRole { Ordinary,
    Promoted { access, access_span }, InheritedPropertyOverride { override_span },
    ConstructorOnly { parameter_span } });
structure!(ArgumentName { text, span });
structure!(Argument { name, value, span });
structure!(Block { statements, span });
enumeration!(Stmt { Block(value), VarDecl(value), Assignment(value), Echo { expr, span },
    Return { expr, span }, Throw(value), Try(value), If(value), While(value), DoWhile(value),
    For(value), Break { span }, Continue { span }, Foreach(value), Increment(value), Expr { expr, span } });
structure!(ThrowStmt {
    keyword_span,
    expr,
    semicolon_span,
    span
});
structure!(TryStmt {
    keyword_span,
    body,
    catches,
    finally,
    span
});
structure!(CatchClause {
    keyword_span,
    ty,
    ty_span,
    binding,
    body,
    span
});
structure!(CatchBinding { name, span });
structure!(TryFinally {
    keyword_span,
    body,
    span
});
structure!(VarDecl {
    writable,
    ty,
    bindings,
    initializer,
    span
});
structure!(VarBinding { name, span });
structure!(Assignment {
    target,
    op,
    value,
    span
});
structure!(IfStmt {
    given,
    condition,
    then_block,
    else_branch,
    finally,
    span
});
enumeration!(ElseBranch { If(value), Block(value) });
structure!(WhileStmt {
    given,
    condition,
    body,
    finally,
    span
});
structure!(DoWhileStmt {
    body,
    condition,
    semicolon_span,
    finally,
    span
});
structure!(GivenPrelude { block, span });
structure!(ControlFlowFinally {
    keyword_span,
    block,
    span
});
structure!(ForStmt {
    initializer,
    condition,
    increment,
    body,
    span
});
enumeration!(ForInitializer { VarDecl(value), Assignment(value) });
enumeration!(ForIncrement { Increment(value), Assignment(value) });
structure!(IncrementStmt {
    target,
    op,
    position,
    span
});
structure!(ForeachStmt {
    iterable,
    first_binding,
    value_binding,
    body,
    span
});
structure!(ForeachBinding {
    writable,
    writable_span,
    ty,
    type_span,
    name,
    name_span,
    span
});
structure!(ClosureExpression {
    form,
    keyword_span,
    parameter_list_span,
    parameters,
    return_type,
    captures,
    body,
    span
});
structure!(ClosureParameter {
    take,
    take_span,
    writable,
    writable_span,
    ty,
    type_span,
    name,
    name_span,
    span
});
structure!(ClosureReturnType {
    colon_span,
    ty,
    type_span,
    span
});
structure!(ClosureCaptureClause {
    keyword_span,
    open_span,
    close_span,
    captures,
    span
});
structure!(ClosureCapture {
    mode,
    modifier_span,
    name,
    name_span,
    span
});
enumeration!(ClosureBody { Expression { arrow_span, expression }, Block(value) });
enumeration!(Expr {
    Variable { name, span }, This { span }, Identifier { name, span }, String { value, span },
    InterpolatedString { parts, span }, Int { value, span }, Float { value, span },
    Bool { value, span }, Null { span }, Array { elements, span },
    ArrayRepeat { value, count, span }, Index { collection, index, span },
    PropertyAccess { object, property, member_span, null_safe, span },
    MethodCall { object, method, member_span, args, argument_list_span, null_safe, span },
    IsType { expr, ty, span }, FunctionCall { name, args, span },
    CallableCall { callee, open_span, args, close_span, argument_list_span, span },
    StaticCall { qualifier, qualifier_span, method, member_span, member_sigil_span,
        args, argument_list_span, span },
    StaticMember { qualifier, qualifier_span, member, member_span, member_sigil_span, span },
    New { class_type, args, shared, span }, Grouped { expr, span }, Unary { op, expr, span },
    Binary { left, op, right, span }, Range { start, end, inclusive, span },
    Match { scrutinee, mode, arms, origin, span }, When(value), Closure(value),
});
structure!(WhenExpression {
    given,
    result_type,
    branches,
    finally,
    span
});
structure!(WhenBranch {
    condition,
    block,
    span
});
enumeration!(MatchMode { Borrowed, Consumed { take_span } });
structure!(MatchArm {
    pattern,
    guard,
    value,
    span
});
structure!(MatchGuard {
    condition,
    keyword_span,
    span
});
enumeration!(MatchPattern { Default { span },
    EnumCase { qualifier, qualifier_span, case, case_span, bindings, span },
    TypeBinding { ty, binding, span }, Expression(value) });
structure!(MatchBinding { name, span });
enumeration!(InterpolatedStringPart { Text { value, span }, Expr(value) });
structure!(ArrayElement { key, value });
