use std::process::ExitCode;

fn main() -> ExitCode {
    match doriac::lsp::run_stdio() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
