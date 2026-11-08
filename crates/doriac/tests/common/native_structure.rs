use std::collections::{BTreeMap, BTreeSet};

use cranelift_object::object::{
    Object, ObjectSection, ObjectSymbol, RelocationTarget, SectionKind, SymbolKind,
};

pub type FunctionReferences = BTreeMap<String, BTreeSet<String>>;

/// The symbol a relocation refers to. Cranelift's COFF output reaches an
/// external symbol through a `.refptr.` pointer cell; what the code references
/// is the symbol behind it.
pub fn relocation_target_name(name: &str) -> String {
    name.strip_prefix(".refptr.")
        .unwrap_or(name)
        .trim_start_matches('_')
        .to_owned()
}

pub fn authored_symbols(program: &doriac::mir::Program, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .map(|name| {
            let function = program
                .functions
                .iter()
                .find(|function| function.name == *name)
                .unwrap();
            doriac::native_abi::function_symbol(function)
                .trim_start_matches('_')
                .to_owned()
        })
        .collect()
}

// These are static references, not dynamic call counts. Unresolved targets
// remain explicit so a caller requiring complete reachability must reject them.
pub fn reachable_references(
    functions: &FunctionReferences,
    roots: Vec<String>,
) -> BTreeSet<String> {
    for root in &roots {
        assert!(
            functions.contains_key(root),
            "authored function has no measured body: {root}"
        );
    }
    let mut pending = roots;
    let mut visited = BTreeSet::new();
    let mut references = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        for target in &functions[&name] {
            references.insert(target.clone());
            if functions.contains_key(target) {
                pending.push(target.clone());
            }
        }
    }
    references
}

pub fn cranelift_references(program: &doriac::mir::Program) -> FunctionReferences {
    let bytes = doriac::codegen_cranelift::lower_mir_to_object(program).unwrap();
    let object = cranelift_object::object::File::parse(bytes.as_slice()).unwrap();
    let functions = object
        .symbols()
        .filter(|symbol| symbol.kind() == SymbolKind::Text)
        .filter_map(|symbol| symbol.section_index().map(|section| (symbol, section)))
        .filter(|(_, section)| {
            object.section_by_index(*section).unwrap().kind() == SectionKind::Text
        })
        .collect::<Vec<_>>();
    let mut graph = BTreeMap::new();
    for (symbol, section_index) in &functions {
        let section = object.section_by_index(*section_index).unwrap();
        let start = symbol.address();
        // Mach-O omits function sizes; text-function symbols bound their code.
        let end = if symbol.size() != 0 {
            start + symbol.size()
        } else {
            functions
                .iter()
                .filter(|(next, next_section)| {
                    next_section == section_index && next.address() > start
                })
                .map(|(next, _)| next.address())
                .min()
                .unwrap_or(section.address() + section.size())
        };
        let mut references = BTreeSet::new();
        for (offset, relocation) in section.relocations().filter(|(offset, _)| {
            let address = section.address() + offset;
            start <= address && address < end
        }) {
            if let RelocationTarget::Symbol(symbol) = relocation.target() {
                let symbol = object.symbol_by_index(symbol).unwrap();
                references.insert(relocation_target_name(symbol.name().unwrap()));
            } else if let RelocationTarget::Section(target) = relocation.target() {
                if object.section_by_index(target).unwrap().kind() == SectionKind::Text {
                    references.insert(format!("unresolved text target at {offset:#x}"));
                }
            }
        }
        graph.insert(
            symbol.name().unwrap().trim_start_matches('_').to_owned(),
            references,
        );
    }
    graph
}

#[cfg(feature = "llvm-backend")]
pub fn llvm_references(program: &doriac::mir::Program) -> (String, FunctionReferences) {
    let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(program).unwrap();
    let mut graph = BTreeMap::new();
    let mut current = None;
    for line in ir.lines() {
        if line.starts_with("define ") {
            let name = line.split_once('@').unwrap().1.split_once('(').unwrap().0;
            let name = name.trim_matches('"').trim_start_matches('_').to_owned();
            graph.insert(name.clone(), BTreeSet::new());
            current = Some(name);
        } else if line == "}" {
            current = None;
        } else if let Some(function) = &current {
            if line.contains("call ") {
                // Error descriptors can select any checked error's drop glue.
                // Include the whole finite domain, not only the observed error.
                if line.contains("call void %error.drop.function") {
                    graph.get_mut(function).unwrap().extend(
                        program
                            .classes
                            .iter()
                            .filter(|class| class.error_descriptor.is_some())
                            .map(|class| format!("doria_drop_class_{}", class.id.0)),
                    );
                    continue;
                }
                let target = if let Some((_, target)) = line.split_once('@') {
                    target
                        .split_once('(')
                        .unwrap()
                        .0
                        .trim_matches('"')
                        .trim_start_matches('_')
                        .to_owned()
                } else {
                    format!("unresolved text target in indirect LLVM call: {line}")
                };
                graph.get_mut(function).unwrap().insert(target);
            }
        }
    }
    (ir, graph)
}
