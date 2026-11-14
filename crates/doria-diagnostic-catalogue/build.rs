//! Lists the documentation entries under `entries/` so the catalogue can
//! embed each one. An entry is one Markdown file named for its code; adding
//! the file is the whole registration.

use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=entries");
    println!("cargo:rerun-if-changed=build.rs");
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let out = env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    let mut codes = Vec::new();
    if env::var_os("CARGO_FEATURE_EXPLANATIONS").is_some() {
        let entries = Path::new(&manifest).join("entries");
        for file in fs::read_dir(&entries).expect("the catalogue has an entries directory") {
            let path = file.expect("an entries directory item can be read").path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
                continue;
            }
            let code = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .expect("an entry file is named for its code")
                .to_string();
            codes.push(code);
        }
        codes.sort();
    }
    let mut table = String::from("&[\n");
    for code in &codes {
        table.push_str(&format!(
            "    (\"{code}\", include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/entries/{code}.md\"))),\n"
        ));
    }
    table.push_str("]\n");
    fs::write(Path::new(&out).join("entry_sources.rs"), table)
        .expect("the entry table can be written");
}
