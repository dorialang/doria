//! Program-wide PHP standard I/O uses the canonical native device coordinator.

use super::*;
use crate::monomorphization::{CallableDecl, CallableInstance};
use crate::semantics::CallableTarget;
use crate::types::substitute_resolved_type;

#[derive(Default)]
struct Usage {
    erased_standard: bool,
    stream_dispatch: bool,
    native_operation: bool,
}

impl Usage {
    fn observe(
        &mut self,
        expr: &Expr,
        semantic: &SemanticInfo,
        plan: &specialization::Plan,
        substitutions: &HashMap<String, ResolvedType>,
    ) {
        if let Some(ty) = semantic.expression_types.get(&expr.span()) {
            let ty = substitute_resolved_type(ty, substitutions);
            let ty = match &ty {
                ResolvedType::Nullable(inner) => inner.as_ref(),
                ty => ty,
            };
            self.erased_standard |= matches!(ty, ResolvedType::Class(class)
                if crate::compiler_known_stream::standard_device(&class.name).is_some())
                && (semantic
                    .interface_conversion_types
                    .contains_key(&expr.span())
                    || semantic.mixed_box_plans.contains_key(&expr.span()));
        }
        if matches!(expr, Expr::FunctionCall { name, .. }
            if Builtin::from_name(name).is_some_and(Builtin::uses_bytes))
        {
            self.native_operation = true;
        }
        let target = semantic
            .property_accessor_calls
            .get(&expr.span())
            .and_then(|calls| calls.getter.as_ref())
            .and_then(|getter| {
                getter
                    .target
                    .specialize(|ty| substitute_resolved_type(ty, substitutions))
            })
            .or_else(|| plan.target(expr.span(), substitutions));
        match target {
            Some(CallableTarget::Method { class_type, .. }) => {
                self.native_operation |=
                    crate::compiler_known_stream::standard_device(&class_type.name).is_some();
            }
            Some(CallableTarget::InterfaceMethod {
                interface,
                method_name,
                ..
            }) => {
                use crate::compiler_known_stream::StreamOperation;
                let receiver = ResolvedType::Interface(interface);
                self.stream_dispatch |= [
                    StreamOperation::Read,
                    StreamOperation::Write,
                    StreamOperation::PrepareRead,
                    StreamOperation::PrepareWrite,
                    StreamOperation::CompleteRead,
                    StreamOperation::CompleteWrite,
                ]
                .into_iter()
                .any(|operation| {
                    plan.stream_callable(&receiver, operation)
                        .is_some_and(|(name, _, _)| name == method_name)
                });
            }
            _ => {}
        }
    }
}

pub(super) fn required(
    program: &Program,
    plan: &specialization::Plan,
    declarations: &[CallableDecl<'_>],
    instances: &[CallableInstance],
) -> bool {
    let mut usage = Usage::default();
    let substitutions = HashMap::new();
    crate::hir::visit::expressions(program, &mut |expr| {
        if !program.sources.iter().any(|source| {
            source.id == expr.span().source
                && source.package == crate::names::PackageIdentity::CompilerKnown
        }) {
            usage.observe(expr, &program.semantic_info, plan, &substitutions);
        }
    });
    for (callable, instance) in plan.callables.iter().zip(instances) {
        if callable.compiler_owned {
            continue;
        }
        let function = declarations[instance.declaration].function;
        let semantic = program
            .semantic_info
            .with_callable_ownership(callable.declaration, &callable.substitutions);
        let mut observe =
            |expr: &Expr| usage.observe(expr, &semantic, plan, &callable.substitutions);
        crate::hir::visit::block(&function.body, &mut observe);
        if function.name == "__construct" {
            if let Some(class) = callable.class.and_then(|id| {
                program
                    .semantic_info
                    .classes
                    .iter()
                    .find(|class| class.id == id)
            }) {
                for item in &program.items {
                    let Item::Class(declaration) = item else {
                        continue;
                    };
                    if declaration.name != class.declaration_name {
                        continue;
                    }
                    for member in &declaration.members {
                        if let ClassMember::Property(property) = member {
                            if let Some(value) = &property.initializer {
                                crate::hir::visit::expression(value, &mut observe);
                            }
                        }
                    }
                }
            }
        }
    }
    // Erased dispatch can select a standard view supplied by another callable.
    // Select one transport for the entire program before any input is read.
    usage.native_operation || (usage.erased_standard && usage.stream_dispatch)
}

pub(super) fn emit_byte_intrinsic(
    name: &str,
    args: &[Argument],
    span: Span,
    scopes: &PhpNameScopes,
) -> String {
    let (operation, path, contents) = match Builtin::from_name(name).unwrap() {
        Builtin::ReadFileBytes => (2, emit_expr(&args[0].value, scopes), "null".into()),
        Builtin::ReadStdinBytes => (3, "null".into(), "null".into()),
        Builtin::WriteFileBytes | Builtin::AppendFileBytes => (
            if name == "write_file_bytes" { 4 } else { 5 },
            emit_expr(&args[0].value, scopes),
            emit_expr(&args[1].value, scopes),
        ),
        Builtin::WriteStdoutBytes | Builtin::WriteStderrBytes => (
            if name == "write_stdout_bytes" { 6 } else { 7 },
            "null".into(),
            emit_expr(&args[0].value, scopes),
        ),
        _ => unreachable!("checked byte intrinsic"),
    };
    format!(
        "__DoriaNativeStandardIo::get()->intrinsic({operation}, {path}, {contents}, {}, {}, {})",
        scopes.source_location(span, span.start),
        scopes.source_location(span, span.end),
        scopes.callable_identity()
    )
}

pub(super) fn emit_runtime(output: &mut String, native: bool) {
    output.push_str(if native {
        "const __DORIA_NATIVE_STANDARD_IO = true;\n"
    } else {
        "const __DORIA_NATIVE_STANDARD_IO = false;\n"
    });
    let operations = crate::compiler_known_io::IoOperation::ALL
        .iter()
        .map(|operation| {
            format!(
                "{} => __DoriaStdIoIoOperation::{},",
                *operation as u8,
                operation.case_name()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut runtime = RUNTIME.replace("__DORIA_OPERATIONS__", &operations);
    for name in ["ReadOutcome", "WriteOutcome"] {
        runtime = runtime.replace(
            &format!("__Doria{name}"),
            &php_type_symbol(&format!("Doria\\Std\\Io\\{name}")),
        );
    }
    output.push_str(&value_result::bind_native_results(runtime));
}

const RUNTIME: &str = r#"
final class __DoriaNativeStandardIo
{
    private static ?self $instance = null;
    private readonly FFI $runtime;
    private array $slots;
    private array $sources = [];

    private function __construct()
    {
        $this->runtime = __DoriaNativeIo::get();
        $this->slots = [];
        foreach (['result' => 'void *[1]', 'message' => 'DrStringV1 *[1]',
            'path' => 'DrStringV1 *[1]', 'system' => 'int64_t[1]',
            'valid' => 'size_t[1]', 'invalid' => 'size_t[1]',
            'transferred' => 'size_t[1]', 'meta' => 'uint64_t[1]',
            'tag' => 'uint64_t[1]', 'count' => 'size_t[1]'] as $name => $type) {
            $this->slots[$name] = $this->runtime->new($type);
        }
    }

    public static function get(): self { return self::$instance ??= new self(); }

    public function source(int $stream, string $class): __DoriaCell
    {
        return $this->sources[$stream] ??= new __DoriaCell($class::__doriaFromReadinessSource(
            $this->runtime->dr_v1_standard_stream_readiness_source($stream)));
    }

    private function text(?FFI\CData $value): string
    {
        if ($value === null || FFI::isNull($value)) { return ''; }
        $length = $this->runtime->dr_v1_string_length($value);
        return $length === 0 ? '' : FFI::string($this->runtime->dr_v1_string_data($value), $length);
    }

    private function releaseErrors(): void
    {
        foreach (['message', 'path'] as $name) {
            $value = $this->slots[$name][0];
            $this->slots[$name][0] = null;
            if ($value !== null && !FFI::isNull($value)) { $this->runtime->dr_v1_string_release($value); }
        }
    }

    private function failure(int $start, int $end, string $callable): never
    {
        $s = &$this->slots;
        $meta = $s['meta'][0];
        $targetTag = ($meta >> 8) & 255;
        $message = $this->text($s['message'][0]);
        $path = $this->text($s['path'][0]);
        if ((($meta >> 24) & 255) === 1) {
            $source = $targetTag === 0 ? __DoriaStdIoUtf8InputSource::File($path)
                : __DoriaStdIoUtf8InputSource::__doriaCaseStandardInput();
            __doria_throw(new __DoriaStdIoInvalidUtf8Error($message, $source, $s['valid'][0],
                ($meta & (1 << 33)) === 0 ? null : $s['invalid'][0]),
                __doria_checked_origin($start, $end), $callable);
        }
        $operation = match ($meta & 255) { __DORIA_OPERATIONS__
            default => __DoriaNativeIo::fail('protocol'),
        };
        $target = match ($targetTag) {
            0 => __DoriaStdIoIoTarget::File($path),
            1 => __DoriaStdIoIoTarget::__doriaCaseStandardInput(),
            2 => __DoriaStdIoIoTarget::__doriaCaseStandardOutput(),
            3 => __DoriaStdIoIoTarget::__doriaCaseStandardError(),
            default => __DoriaNativeIo::fail('protocol'),
        };
        __doria_throw(new __DoriaStdIoIoError($message, $operation, $target,
            __doria_native_io_reason(($meta >> 16) & 255),
            ($meta & (1 << 32)) === 0 ? null : $s['system'][0],
            ($meta & (1 << 34)) === 0 ? null : $s['transferred'][0]),
            __doria_checked_origin($start, $end), $callable);
    }

    public function transfer(int $stream, bool $reading, __DoriaByteRegion $region): mixed
    {
        $this->transferResult($stream, $reading, $region);
        $value = $reading
            ? __DORIA_READ_BOX__(__DoriaValueResult::$tag, __DoriaValueResult::$field0)
            : __DORIA_WRITE_BOX__(__DoriaValueResult::$tag, __DoriaValueResult::$field0);
        __DoriaValueResult::clear();
        return $value;
    }

    public function transferResult(int $stream, bool $reading, __DoriaByteRegion $region): void
    {
        $this->transferRange($stream, $reading, $region->__doriaNativeStorage($reading),
            $region->__doriaRegionOffset(), count($region));
    }

    public function transferFields(int $stream, bool $reading, WeakReference $storage,
        int $offset, int $length): void
    {
        $this->transferRange($stream, $reading, __DoriaByteRegion::__doriaNativeSource($storage), $offset, $length);
    }

    private function transferRange(int $stream, bool $reading, __DoriaBytes $source, int $offset, int $length): void
    {
        __DoriaValueResult::clear();
        $s = &$this->slots;
        $code = $this->runtime->dr_v1_host_standard_stream_io(null, $stream, $reading ? 1 : 2,
            $source->__doriaNativeBase(), count($source), $offset, $length, $s['tag'], $s['count'],
            $s['message'], $s['path'], $s['system'], $s['transferred'], $s['meta']);
        try {
            if ($code === 1) { $this->failure(0, 0, ''); }
            if ($code !== 0) { __DoriaNativeIo::fail('protocol'); }
            $tag = $s['tag'][0];
            $count = $s['count'][0];
        } finally { $this->releaseErrors(); }
        if ($reading) {
            match ($tag) {
                0 => __DORIA_READ_DATA_RESULT__($count),
                1 => __DORIA_READ_BLOCK_RESULT__(),
                2 => __DORIA_READ_EOF_RESULT__(),
                default => __DoriaNativeIo::fail('protocol'),
            };
        } else {
            match ($tag) {
                0 => __DORIA_WRITE_DATA_RESULT__($count),
                1 => __DORIA_WRITE_BLOCK_RESULT__(),
                default => __DoriaNativeIo::fail('protocol'),
            };
        }
    }

    public function intrinsic(int $operation, ?string $path, string|__DoriaBytes|null $contents,
        int $start, int $end, string $callable): mixed
    {
        $s = &$this->slots;
        $runtime = $this->runtime;
        $nativePath = $path === null ? null : $runtime->dr_v1_string_from_utf8($path, strlen($path));
        $length = $contents instanceof __DoriaBytes ? count($contents) : strlen($contents ?? '');
        $data = $contents instanceof __DoriaBytes
            ? ($length === 0 ? null : $runtime->cast('char *', $contents->__doriaNativeBase()))
            : $contents;
        try {
            $code = $runtime->dr_v4_checked_io(null, $operation, $nativePath, $data, $length,
                $s['result'], $s['message'], $s['path'], $s['system'], $s['valid'],
                $s['invalid'], $s['transferred'], $s['meta']);
            if ($code === 1) { $this->failure($start, $end, $callable); }
            if ($code !== 0) { __DoriaNativeIo::fail('protocol'); }
            if ($operation === 2 || $operation === 3) {
                $owner = $runtime->cast('DrBytesV1 *', $s['result'][0]);
                $value = __DoriaBytes::__doriaFromNativeBytes($owner);
                $s['result'][0] = null;
                return $value;
            }
            if ($operation === 0 || $operation === 1) {
                $value = $s['result'][0];
                return $value === null || FFI::isNull($value) ? null
                    : $this->text($runtime->cast('DrStringV1 *', $value));
            }
            return null;
        } finally {
            if ($nativePath !== null) { $runtime->dr_v1_string_release($nativePath); }
            $value = $s['result'][0];
            $s['result'][0] = null;
            if ($value !== null && !FFI::isNull($value)) {
                if ($operation === 2 || $operation === 3) { $runtime->dr_v1_bytes_free($runtime->cast('DrBytesV1 *', $value)); }
                else { $runtime->dr_v1_string_release($runtime->cast('DrStringV1 *', $value)); }
            }
            $this->releaseErrors();
        }
    }
}
"#;
