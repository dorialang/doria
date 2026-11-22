//! The external symbols a COFF object (Windows) references without defining
//! them, read from its symbol table, so tests can check what an object
//! imports without a linker.

/// The names of the undefined external symbols of the COFF object `object`.
pub fn undefined_external_symbols(object: &[u8]) -> Vec<String> {
    let word = |offset: usize| u32::from_le_bytes(object[offset..offset + 4].try_into().unwrap());
    let table = word(8) as usize;
    let count = word(12) as usize;
    let strings = table + 18 * count;
    let mut names = Vec::new();
    let mut index = 0;
    while index < count {
        let record = &object[table + 18 * index..table + 18 * (index + 1)];
        let section = i16::from_le_bytes([record[12], record[13]]);
        let external = record[16] == 2;
        if section == 0 && external {
            let name = if record[..4] == [0, 0, 0, 0] {
                let start = strings + u32::from_le_bytes(record[4..8].try_into().unwrap()) as usize;
                let end = start + object[start..].iter().position(|byte| *byte == 0).unwrap();
                &object[start..end]
            } else {
                let end = record[..8].iter().position(|byte| *byte == 0).unwrap_or(8);
                &record[..end]
            };
            names.push(String::from_utf8_lossy(name).into_owned());
        }
        index += 1 + usize::from(record[17]);
    }
    names
}
