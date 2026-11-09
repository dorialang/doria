use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=DORIA_BUILD_COMMIT");
    println!("cargo:rerun-if-changed=RUNTIME_ABI_VERSION");
    if env::var_os("CARGO_FEATURE_HOSTED_SHARED_RUNTIME").is_none() {
        return;
    }
    let abi = fs::read_to_string("RUNTIME_ABI_VERSION").expect("runtime ABI version");
    let revision = env::var("DORIA_BUILD_COMMIT").unwrap_or_else(|_| "unknown".into());
    let target = env::var("TARGET").expect("runtime target");
    let profile = env::var("PROFILE").expect("runtime profile");
    let features = if env::var_os("CARGO_FEATURE_STANDALONE_WINDOWS_SUPPORT").is_some() {
        "[\"hosted-shared-runtime\",\"standalone-windows-support\"]"
    } else {
        "[\"hosted-shared-runtime\"]"
    };
    let identity = format!(
        "{{\"abiVersion\":{},\"runtimeRevision\":{},\"targetTriple\":{},\"profile\":{},\"featureSet\":{features},\"artifactKind\":\"shared\"}}",
        json_string(abi.trim()), json_string(&revision), json_string(&target), json_string(&profile),
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("runtime output directory"));
    fs::write(output.join("runtime-identity.json"), identity).expect("runtime identity export");
}

fn json_string(value: &str) -> String {
    let mut result = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            character if character.is_control() => {
                result.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => result.push(character),
        }
    }
    result.push('"');
    result
}
