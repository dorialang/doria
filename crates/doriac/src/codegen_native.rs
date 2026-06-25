use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use cranelift_codegen::ir::{types, AbiParam, InstBuilder};
use cranelift_codegen::settings;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{default_libcall_names, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::backend::BackendError;
use crate::hir::{self, Expr, Item, Stmt};

pub fn generate_executable(program: &hir::Program) -> Result<Vec<u8>, BackendError> {
    let exit_code = validate_stage_2b(program)?;
    let object_bytes = generate_object(exit_code)?;
    link_object(&object_bytes)
}

pub fn validate_stage_2b(program: &hir::Program) -> Result<i32, BackendError> {
    let mut main_functions = Vec::new();

    for item in &program.items {
        match item {
            Item::Function(function) if function.name == "main" => {
                main_functions.push(function);
            }
            Item::Function(function) => {
                return Err(BackendError::new(format!(
                    "unsupported top-level item for native Stage 2b: extra top-level function `{}`",
                    function.name
                )));
            }
            Item::Class(class_decl) => {
                return Err(BackendError::new(format!(
                    "unsupported top-level item for native Stage 2b: class `{}`",
                    class_decl.name
                )));
            }
            Item::Statement(statement) => {
                return Err(BackendError::new(format!(
                    "unsupported top-level item for native Stage 2b: {}",
                    describe_statement(statement)
                )));
            }
        }
    }

    let [main] = main_functions.as_slice() else {
        return Err(match main_functions.len() {
            0 => BackendError::new(
                "no native entrypoint found; Stage 2b native output requires exactly one top-level `function main(): int`",
            ),
            _ => BackendError::new(
                "multiple native entrypoints found; Stage 2b native output requires exactly one top-level `function main(): int`",
            ),
        });
    };

    if !main.params.is_empty() {
        return Err(BackendError::new(
            "wrong main signature for native Stage 2b: `main` must not declare parameters",
        ));
    }

    if !matches!(
        main.return_type.as_ref(),
        Some(return_type) if return_type.name == "int" && return_type.args.is_empty()
    ) {
        return Err(BackendError::new(
            "wrong main signature for native Stage 2b: expected `function main(): int`",
        ));
    }

    let Some((return_statement, local_statements)) = main.body.statements.split_last() else {
        return Err(BackendError::new(
            "unsupported native statement for Stage 2b: `main` must end with `return <portable integer literal or readonly integer local>;`",
        ));
    };

    let mut local_values = HashMap::new();
    for statement in local_statements {
        match statement {
            Stmt::VarDecl(decl) => validate_stage_2b_local(decl, &mut local_values)?,
            Stmt::Return { .. } => {
                return Err(BackendError::new(
                    "unsupported native statement for Stage 2b: no statements may follow `return <portable integer literal or readonly integer local>;`",
                ));
            }
            other => {
                return Err(BackendError::new(format!(
                    "unsupported native statement for Stage 2b: expected readonly `int` local declaration or final return, found {}",
                    describe_statement(other)
                )));
            }
        }
    }

    match return_statement {
        Stmt::Return { expr: Some(expr), .. } => {
            validate_stage_2b_return_expr(expr, &local_values)
        }
        Stmt::Return { expr: None, .. } => Err(BackendError::new(
            "unsupported native statement for Stage 2b: expected `return <portable integer literal or readonly integer local>;`, found bare `return;`",
        )),
        other => Err(BackendError::new(format!(
            "unsupported native statement for Stage 2b: `main` must end with `return <portable integer literal or readonly integer local>;`, found {}",
            describe_statement(other)
        ))),
    }
}

fn validate_stage_2b_local(
    decl: &hir::VarDecl,
    local_values: &mut HashMap<String, i64>,
) -> Result<(), BackendError> {
    if decl.writable {
        return Err(unsupported_stage_2b_local());
    }

    if let Some(ty) = &decl.ty {
        if ty.name != "int" || !ty.args.is_empty() {
            return Err(unsupported_stage_2b_local());
        }
    }

    let Expr::Int { value, .. } = &decl.initializer else {
        return Err(unsupported_stage_2b_local());
    };

    let parsed = parse_doria_int_literal(value)?;
    local_values.insert(decl.name.clone(), parsed);
    Ok(())
}

fn unsupported_stage_2b_local() -> BackendError {
    BackendError::new(
        "unsupported native local for Stage 2b: expected readonly `int` local initialized from an integer literal",
    )
}

fn validate_stage_2b_return_expr(
    expr: &Expr,
    local_values: &HashMap<String, i64>,
) -> Result<i32, BackendError> {
    match expr {
        Expr::Int { value, .. } => parse_stage_2b_exit_code(parse_doria_int_literal(value)?),
        Expr::Variable { name, .. } => {
            let Some(value) = local_values.get(name) else {
                return Err(BackendError::new(
                    "unsupported native expression for Stage 2b: expected integer literal or readonly integer local",
                ));
            };
            parse_stage_2b_exit_code(*value)
        }
        other => Err(BackendError::new(format!(
            "unsupported native expression for Stage 2b: expected integer literal or readonly integer local, found `{}`",
            describe_expression(other)
        ))),
    }
}

fn parse_doria_int_literal(value: &str) -> Result<i64, BackendError> {
    value
        .parse::<i64>()
        .map_err(|_| BackendError::new("integer literal is outside the Doria `int` range"))
}

fn parse_stage_2b_exit_code(value: i64) -> Result<i32, BackendError> {
    if !(0..=125).contains(&value) {
        return Err(BackendError::new(
            "native Stage 2b exit code must be in the range 0..125",
        ));
    }

    Ok(value as i32)
}

fn generate_object(exit_code: i32) -> Result<Vec<u8>, BackendError> {
    let isa_builder = cranelift_native::builder()
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
    let isa = isa_builder
        .finish(settings::Flags::new(settings::builder()))
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
    let mut module = ObjectModule::new(
        ObjectBuilder::new(isa, "doria_stage_2b", default_libcall_names())
            .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?,
    );

    let mut signature = module.make_signature();
    signature.returns.push(AbiParam::new(types::I32));

    let function_id = module
        .declare_function("main", Linkage::Export, &signature)
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;

    let mut context = module.make_context();
    context.func.signature = signature;
    let mut function_context = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut context.func, &mut function_context);
        let entry_block = builder.create_block();
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);
        let exit_value = builder.ins().iconst(types::I32, i64::from(exit_code));
        builder.ins().return_(&[exit_value]);
        builder.finalize();
    }

    module
        .define_function(function_id, &mut context)
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
    module.clear_context(&mut context);

    module
        .finish()
        .emit()
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))
}

fn link_object(object_bytes: &[u8]) -> Result<Vec<u8>, BackendError> {
    let temp_stem = unique_temp_stem();
    let object_path = temp_stem.with_extension(object_extension());
    let executable_path = temp_stem.with_extension(executable_extension());

    fs::write(&object_path, object_bytes)
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;

    let link_result = invoke_linker(&object_path, &executable_path);
    let executable_bytes = match link_result {
        Ok(()) => fs::read(&executable_path)
            .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?,
        Err(error) => {
            cleanup_temp_artifacts(&object_path, &executable_path);
            return Err(error);
        }
    };

    cleanup_temp_artifacts(&object_path, &executable_path);
    Ok(executable_bytes)
}

fn invoke_linker(object_path: &Path, executable_path: &Path) -> Result<(), BackendError> {
    // Stage 2b emits a Cranelift object file and asks the host toolchain to link
    // it. This is not a C backend: Doria never generates C source or uses C
    // semantics as an oracle here.
    let cc_is_set = env::var_os("CC").is_some();
    let linker = env::var("CC").unwrap_or_else(|_| default_linker().to_string());
    let mut command = Command::new(&linker);
    command.args(linker_arguments(
        &linker,
        cc_is_set,
        cfg!(windows),
        object_path,
        executable_path,
    ));

    let output = command.output().map_err(|error| {
        BackendError::new(format!(
            "linker/toolchain failure: failed to run `{linker}`: {error}"
        ))
    })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = [stderr.trim(), stdout.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    if details.is_empty() {
        Err(BackendError::new(format!(
            "linker/toolchain failure: `{linker}` exited with status {}",
            output.status
        )))
    } else {
        Err(BackendError::new(format!(
            "linker/toolchain failure: `{linker}` exited with status {}\n{}",
            output.status, details
        )))
    }
}

fn cleanup_temp_artifacts(object_path: &Path, executable_path: &Path) {
    let _ = fs::remove_file(object_path);
    let _ = fs::remove_file(executable_path);
}

fn unique_temp_stem() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    env::temp_dir().join(format!("doriac-native-{}-{nanos}", std::process::id()))
}

fn object_extension() -> &'static str {
    if cfg!(windows) {
        "obj"
    } else {
        "o"
    }
}

fn executable_extension() -> &'static str {
    if cfg!(windows) {
        "exe"
    } else {
        "out"
    }
}

fn default_linker() -> &'static str {
    if cfg!(windows) {
        "cl.exe"
    } else {
        "cc"
    }
}

fn linker_arguments(
    linker: &str,
    cc_is_set: bool,
    windows: bool,
    object_path: &Path,
    executable_path: &Path,
) -> Vec<OsString> {
    if windows && (!cc_is_set || is_msvc_style_compiler_driver(linker)) {
        // Cranelift-generated objects do not carry MSVC /DEFAULTLIB directives.
        // For Stage 2b's tiny main, make Doria's main the executable entrypoint
        // instead of relying on CRT startup to discover and call it.
        return vec![
            OsString::from("/nologo"),
            object_path.as_os_str().to_os_string(),
            OsString::from(format!("/Fe:{}", executable_path.display())),
            OsString::from("/link"),
            OsString::from("/ENTRY:main"),
            OsString::from("/SUBSYSTEM:CONSOLE"),
        ];
    }

    vec![
        object_path.as_os_str().to_os_string(),
        OsString::from("-o"),
        executable_path.as_os_str().to_os_string(),
    ]
}

fn is_msvc_style_compiler_driver(linker: &str) -> bool {
    let Some(name) = Path::new(linker).file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        "cl" | "cl.exe" | "clang-cl" | "clang-cl.exe"
    )
}

fn describe_statement(statement: &Stmt) -> &'static str {
    match statement {
        Stmt::VarDecl(_) => "local variable declaration",
        Stmt::Assignment(_) => "assignment",
        Stmt::Echo { .. } => "echo statement",
        Stmt::Return { .. } => "return statement",
        Stmt::If(_) => "if statement",
        Stmt::While(_) => "while statement",
        Stmt::Foreach(_) => "foreach statement",
        Stmt::Expr { .. } => "expression statement",
    }
}

fn describe_expression(expr: &Expr) -> &'static str {
    match expr {
        Expr::Variable { .. } => "variable",
        Expr::This { .. } => "$this",
        Expr::Identifier { .. } => "identifier",
        Expr::String { .. } => "string literal",
        Expr::InterpolatedString { .. } => "interpolated string",
        Expr::Int { .. } => "integer literal",
        Expr::Float { .. } => "float literal",
        Expr::Bool { .. } => "bool literal",
        Expr::Null { .. } => "null literal",
        Expr::Array { .. } => "collection literal",
        Expr::PropertyAccess { .. } => "property access",
        Expr::MethodCall { .. } => "method call",
        Expr::FunctionCall { .. } => "function call",
        Expr::StaticCall { .. } => "static call",
        Expr::New { .. } => "object construction",
        Expr::Binary { .. } => "binary expression",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_default_uses_msvc_compiler_driver_arguments() {
        let args = linker_arguments(
            "cl.exe",
            false,
            true,
            Path::new("main.obj"),
            Path::new("main.exe"),
        );

        assert_eq!(
            args,
            vec![
                OsString::from("/nologo"),
                OsString::from("main.obj"),
                OsString::from("/Fe:main.exe"),
                OsString::from("/link"),
                OsString::from("/ENTRY:main"),
                OsString::from("/SUBSYSTEM:CONSOLE"),
            ]
        );
    }

    #[test]
    fn windows_clang_cl_uses_msvc_compiler_driver_arguments() {
        let args = linker_arguments(
            "clang-cl.exe",
            true,
            true,
            Path::new("main.obj"),
            Path::new("main.exe"),
        );

        assert_eq!(
            args,
            vec![
                OsString::from("/nologo"),
                OsString::from("main.obj"),
                OsString::from("/Fe:main.exe"),
                OsString::from("/link"),
                OsString::from("/ENTRY:main"),
                OsString::from("/SUBSYSTEM:CONSOLE"),
            ]
        );
    }

    #[test]
    fn unix_style_compiler_driver_uses_dash_o() {
        let args = linker_arguments(
            "clang",
            true,
            true,
            Path::new("main.obj"),
            Path::new("main.exe"),
        );

        assert_eq!(
            args,
            vec![
                OsString::from("main.obj"),
                OsString::from("-o"),
                OsString::from("main.exe"),
            ]
        );
    }
}
