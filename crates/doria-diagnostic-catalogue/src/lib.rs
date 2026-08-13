#![no_std]

/// Every stable compiler-owned diagnostic identity.
///
/// This crate is the single catalogue authority shared by the compiler and
/// the freestanding runtime. Rich runtime entries below are the compact subset
/// needed by standalone executables; they are not a second catalogue.
pub const DIAGNOSTIC_CODES: &[&str] = &[
    "B0001", "B0002", "B0003", "B1301", "B1901", "B2001", "B2301", "B2401", "B2501", "B2601",
    "E0101", "E0102", "E0103", "E0201", "E0202", "E0203", "E0204", "E0300", "E0303", "E0304",
    "E0305", "E0306", "E0307", "E0308", "E0309", "E0310", "E0401", "E0402", "E0403", "E0404",
    "E0405", "E0406", "E0407", "E0408", "E0409", "E0410", "E0411", "E0412", "E0413", "E0414",
    "E0415", "E0416", "E0417", "E0419", "E0420", "E0421", "E0422", "E0423", "E0424", "E0425",
    "E0426", "E0430", "E0431", "E0432", "E0433", "E0434", "E0435", "E0436", "E0440", "E0441",
    "E0442", "E0443", "E0444", "E0445", "E0450", "E0451", "E0452", "E0453", "E0454", "E0455",
    "E0456", "E0457", "E0461", "E0462", "E0463", "E0464", "E0465", "E0466", "E0467", "E0468",
    "E0470", "E0471", "E0472", "E0473", "E0474", "E0475", "E0476", "E0477", "E0478", "E0479",
    "E0480", "E0481", "E0482", "E0483", "E0484", "E0485", "E0486", "E0487", "E0488", "E0489",
    "E0490", "E0491", "E0492", "E0493", "E0494", "E0495", "E0496", "E0497", "E0498", "E0500",
    "E0501", "E0502", "E0503", "E0504", "E0505", "E0506", "E0507", "E0508", "E0509", "E0510",
    "E0511", "E0512", "E0513", "E0515", "E0516", "E0517", "E0518", "E0519", "E0520", "E0521",
    "E0522", "E0523", "E0524", "E0525", "E0526", "E0527", "E0528", "E0529", "E0530", "E0531",
    "E0532", "E0533", "E0534", "E0535", "E0536", "E0537", "E0538", "E0539", "E0540", "E0541",
    "E0542", "E0543", "E0544", "E0545", "E0546", "E0547", "E0548", "E0549", "E0550", "E0551",
    "E0552", "E0553", "E0554", "E0555", "E0556", "E0557", "E0558", "E0559", "E0560", "E0561",
    "E0562", "E0563", "E0564", "E0565", "E0566", "E0567", "E0568", "E0569", "E0570", "E0571",
    "E0572", "E0573", "E0574", "E0575", "E0576", "E0577", "E0578", "E0579", "E0580", "E0581",
    "E0582", "E0583", "E0584", "E0585", "E0586", "E0587", "E0588", "E0589", "E0590", "E0591",
    "E0592", "E0593", "E0594", "E0595", "E0596", "E0597", "E0598", "E0599", "E0600", "E0601",
    "E0602", "E0603", "E0604", "I0001", "I1101", "I1301", "I1302", "I1401", "I2001", "I2002",
    "I2003", "I2201", "I2401", "I2601", "I2701", "I2702", "I2801", "L0001", "L0002", "M1101",
    "M1102", "P0001", "P0002", "P0017", "P1000", "P1001", "P1101", "P1102", "P1103", "P1104",
    "P1105", "P1106", "P1107", "P1108", "P1109", "P1110", "P1111", "P1201", "P1202", "P1203",
    "P1204", "P1205", "P1206", "P1301", "P1302", "P1310", "P1311", "P1312", "P1313", "P1320",
    "P1321", "P1322", "P1401", "P1402", "P1403", "P1404", "P1405", "P1406", "P1407", "P1410",
    "P1501", "P1502", "P1503", "P1504", "P1505", "P1601",
];

/// One stable runtime-outcome identity shared by the compiler and `doria-rt`.
///
/// Presentation belongs to the compiler-owned diagnostic model. The runtime
/// consumes these immutable entries only when a standalone executable has no
/// compiler host available to render its structured outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeCatalogueEntry {
    pub code: &'static str,
    pub title: &'static str,
    pub domain: &'static str,
    pub primary_label: &'static str,
    pub explanation: &'static str,
    pub process_status: i32,
    pub fact_names: &'static [&'static str],
}

pub const SHARED_ACCESS_CONFLICT_REASON_FACT: &str = "conflictReason";
pub const READONLY_THEN_WRITABLE_CONFLICT: &str =
    "Cannot Acquire Writable Access While Readonly Access Is Active";
pub const WRITABLE_THEN_READONLY_CONFLICT: &str =
    "Cannot Acquire Readonly Access While Writable Access Is Active";
pub const WRITABLE_THEN_WRITABLE_CONFLICT: &str =
    "Cannot Acquire Writable Access While Writable Access Is Active";

pub fn is_shared_access_conflict_reason(value: &str) -> bool {
    matches!(
        value,
        READONLY_THEN_WRITABLE_CONFLICT
            | WRITABLE_THEN_READONLY_CONFLICT
            | WRITABLE_THEN_WRITABLE_CONFLICT
    )
}

pub const STRING_PADDING_OPERATION_FACT: &str = "operation";
pub const PROCESS_STATUS_FACT: &str = "status";
pub const STRING_SLICE_LENGTH_FACT: &str = "length";
pub const STRING_PADDING_REQUESTED_LENGTH_FACT: &str = "requestedLength";
pub const STRING_REPETITION_COUNT_FACT: &str = "count";
pub const STRING_PADDING_VALUE_FACT: &str = "value";
pub const STRING_PADDING_CURRENT_LENGTH_FACT: &str = "currentGraphemeLength";
pub const STRING_PADDING_REQUESTED_GRAPHEME_LENGTH_FACT: &str = "requestedGraphemeLength";
pub const STRING_PADDING_PADDING_LENGTH_FACT: &str = "paddingGraphemeLength";
pub const INDEX_FACT: &str = "index";
pub const INDEXED_LENGTH_FACT: &str = "length";
pub const COLLECTION_FILL_COUNT_FACT: &str = "count";
pub const STRING_PADDING_OPERATIONS: &[&str] = &["padStart", "padEnd"];

pub fn is_string_padding_operation(value: &str) -> bool {
    STRING_PADDING_OPERATIONS.contains(&value)
}

pub fn runtime_entry(code: &str) -> Option<&'static RuntimeCatalogueEntry> {
    RUNTIME_CATALOGUE.iter().find(|entry| entry.code == code)
}

pub const RUNTIME_CATALOGUE: &[RuntimeCatalogueEntry] = &[
    RuntimeCatalogueEntry {
        code: "P1000",
        title: "Program Panicked",
        domain: "language",
        primary_label: "Panic Raised Here",
        explanation: "The program explicitly raised a fatal panic.",
        process_status: 101,
        fact_names: &["message"],
    },
    RuntimeCatalogueEntry {
        code: "P1001",
        title: "Runtime Diagnostic Failed",
        domain: "runtime",
        primary_label: "Runtime Failure",
        explanation: "The runtime could not construct the intended diagnostic and terminated safely.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1101",
        title: "Integer Addition Overflowed",
        domain: "numeric",
        primary_label: "Addition Overflowed Here",
        explanation: "The mathematical addition result cannot be represented by the operation's integer type.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1102",
        title: "Integer Subtraction Overflowed",
        domain: "numeric",
        primary_label: "Subtraction Overflowed Here",
        explanation: "The mathematical subtraction result cannot be represented by the operation's integer type.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1103",
        title: "Integer Multiplication Overflowed",
        domain: "numeric",
        primary_label: "Multiplication Overflowed Here",
        explanation: "The mathematical multiplication result cannot be represented by the operation's integer type.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1104",
        title: "Integer Negation Overflowed",
        domain: "numeric",
        primary_label: "Negation Overflowed Here",
        explanation: "The negated value cannot be represented by the operation's signed integer type.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1105",
        title: "Integer Division By Zero",
        domain: "numeric",
        primary_label: "Zero Divisor",
        explanation: "Integer division requires a non-zero divisor.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1106",
        title: "Integer Division Overflowed",
        domain: "numeric",
        primary_label: "Division Overflowed Here",
        explanation: "Dividing the minimum signed value by negative one cannot be represented by the same integer type.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1107",
        title: "Integer Remainder By Zero",
        domain: "numeric",
        primary_label: "Zero Divisor",
        explanation: "Integer remainder requires a non-zero divisor.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1108",
        title: "Integer Shift Count Is Out Of Range",
        domain: "numeric",
        primary_label: "Invalid Shift Count",
        explanation: "The shift count must be non-negative and smaller than the integer type's bit width.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1109",
        title: "Integer Conversion Is Out Of Range",
        domain: "numeric",
        primary_label: "Value Cannot Be Represented",
        explanation: "The source value cannot be represented by the requested integer type.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1110",
        title: "Float To Integer Conversion Is Out Of Range",
        domain: "numeric",
        primary_label: "Value Cannot Be Converted",
        explanation: "The float is NaN, infinite, or outside the canonical signed integer range after truncation.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1111",
        title: "Main Returned An Invalid Process Status",
        domain: "process",
        primary_label: "Invalid Process Status Returned Here",
        explanation: "A Doria entry point may return only a process status from 0 through 125.",
        process_status: 101,
        fact_names: &[PROCESS_STATUS_FACT],
    },
    RuntimeCatalogueEntry {
        code: "P1201",
        title: "String Slice Length Cannot Be Negative",
        domain: "string",
        primary_label: "Negative Slice Length",
        explanation: "A string slice length describes a count of graphemes and therefore cannot be negative.",
        process_status: 101,
        fact_names: &[STRING_SLICE_LENGTH_FACT],
    },
    RuntimeCatalogueEntry {
        code: "P1202",
        title: "String Padding Length Cannot Be Negative",
        domain: "string",
        primary_label: "Negative Padding Length",
        explanation: "A string padding target is a grapheme length and therefore cannot be negative.",
        process_status: 101,
        fact_names: &[STRING_PADDING_REQUESTED_LENGTH_FACT],
    },
    RuntimeCatalogueEntry {
        code: "P1203",
        title: "String Padding Text Cannot Be Empty",
        domain: "string",
        primary_label: "Empty Padding Text",
        explanation: "The padding operation must add graphemes, but empty padding text cannot add any graphemes.",
        process_status: 101,
        fact_names: &[
            STRING_PADDING_OPERATION_FACT,
            STRING_PADDING_VALUE_FACT,
            STRING_PADDING_CURRENT_LENGTH_FACT,
            STRING_PADDING_REQUESTED_GRAPHEME_LENGTH_FACT,
            STRING_PADDING_PADDING_LENGTH_FACT,
        ],
    },
    RuntimeCatalogueEntry {
        code: "P1204",
        title: "String Repetition Count Cannot Be Negative",
        domain: "string",
        primary_label: "Negative Repetition Count",
        explanation: "A string repetition count describes how many copies to produce and therefore cannot be negative.",
        process_status: 101,
        fact_names: &[STRING_REPETITION_COUNT_FACT],
    },
    RuntimeCatalogueEntry {
        code: "P1205",
        title: "String Result Is Too Large",
        domain: "string",
        primary_label: "String Operation Exceeded Runtime Limits",
        explanation: "The requested string result is too large for the runtime representation.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1206",
        title: "String Allocation Failed",
        domain: "string",
        primary_label: "String Value Requested Here",
        explanation: "The runtime could not allocate storage for the requested string value.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1301",
        title: "Byte Index Is Out Of Bounds",
        domain: "bytes",
        primary_label: "Invalid Byte Index",
        explanation: "The byte index is outside the valid range of this Bytes value.",
        process_status: 101,
        fact_names: &[INDEX_FACT, INDEXED_LENGTH_FACT],
    },
    RuntimeCatalogueEntry {
        code: "P1302",
        title: "Byte Buffer Allocation Failed",
        domain: "bytes",
        primary_label: "Byte Buffer Requested Here",
        explanation: "The runtime could not allocate storage for the requested byte buffer.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1310",
        title: "Collection Index Is Out Of Bounds",
        domain: "collection",
        primary_label: "Invalid Collection Index",
        explanation: "The collection index is outside the valid range of this collection.",
        process_status: 101,
        fact_names: &[INDEX_FACT, INDEXED_LENGTH_FACT],
    },
    RuntimeCatalogueEntry {
        code: "P1311",
        title: "Collection Fill Count Cannot Be Negative",
        domain: "collection",
        primary_label: "Negative Fill Count",
        explanation: "A collection fill count describes how many elements to create and therefore cannot be negative.",
        process_status: 101,
        fact_names: &[COLLECTION_FILL_COUNT_FACT],
    },
    RuntimeCatalogueEntry {
        code: "P1312",
        title: "Dictionary Key Was Not Found",
        domain: "collection",
        primary_label: "Missing Dictionary Key",
        explanation: "The requested key does not exist in this dictionary.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1313",
        title: "Collection Allocation Failed",
        domain: "collection",
        primary_label: "Collection Created Here",
        explanation: "The runtime could not allocate or grow storage for the requested collection.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1321",
        title: "Mixed Value Does Not Have Unique Ownership",
        domain: "ownership",
        primary_label: "Ownership Transfer Requested Here",
        explanation: "A move-type value can be taken from mixed only when that mixed box holds the final owning claim.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1320",
        title: "Mixed Value Allocation Failed",
        domain: "mixed",
        primary_label: "Mixed Value Created Here",
        explanation: "The runtime could not allocate storage for the mixed value.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1322",
        title: "Mixed Ownership Clone Failed",
        domain: "mixed",
        primary_label: "Mixed Ownership Cloned Here",
        explanation: "The runtime could not create the requested mixed ownership claim.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1401",
        title: "File Read Failed",
        domain: "io",
        primary_label: "File Read Requested Here",
        explanation: "The runtime could not read the requested file.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1402",
        title: "File Write Failed",
        domain: "io",
        primary_label: "File Write Requested Here",
        explanation: "The runtime could not write the requested file.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1403",
        title: "Standard Input Read Failed",
        domain: "io",
        primary_label: "Input Read Requested Here",
        explanation: "The runtime could not read from standard input.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1404",
        title: "Input Text Is Not Valid UTF-8",
        domain: "io",
        primary_label: "Text Input Requested Here",
        explanation: "Doria text input must contain valid UTF-8.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1405",
        title: "File Path Contains A NUL Byte",
        domain: "io",
        primary_label: "Invalid File Path Used Here",
        explanation: "A file path cannot contain an embedded NUL byte.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1406",
        title: "File Text Is Not Valid UTF-8",
        domain: "io",
        primary_label: "Text File Read Here",
        explanation: "Doria text-file reads require valid UTF-8 contents.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1407",
        title: "Standard Device Write Failed",
        domain: "io",
        primary_label: "Output Written Here",
        explanation: "The runtime could not write to the requested standard device.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1410",
        title: "Program Argument Is Not Valid UTF-8",
        domain: "process",
        primary_label: "Invalid Program Argument",
        explanation: "Doria program arguments must be valid UTF-8 text.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1501",
        title: "Writable Shared Access Conflicted",
        domain: "ownership",
        primary_label: "Conflicting Access Requested Here",
        explanation: "The requested shared access overlaps an incompatible active access to the same allocation.",
        process_status: 101,
        fact_names: &[SHARED_ACCESS_CONFLICT_REASON_FACT],
    },
    RuntimeCatalogueEntry {
        code: "P1502",
        title: "Shared Ownership Allocation Failed",
        domain: "ownership",
        primary_label: "Shared Ownership Requested Here",
        explanation: "The runtime could not allocate the shared-ownership control block.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1503",
        title: "Shared Ownership Count Overflowed",
        domain: "ownership",
        primary_label: "Shared Ownership Retained Here",
        explanation: "The shared-ownership reference count cannot represent another owner.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1504",
        title: "Weak Ownership Count Overflowed",
        domain: "ownership",
        primary_label: "Weak Ownership Retained Here",
        explanation: "The weak-reference count cannot represent another observer.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1505",
        title: "Shared Access Count Overflowed",
        domain: "ownership",
        primary_label: "Shared Access Requested Here",
        explanation: "The readonly shared-access count cannot represent another active access.",
        process_status: 101,
        fact_names: &[],
    },
    RuntimeCatalogueEntry {
        code: "P1601",
        title: "Class Allocation Failed",
        domain: "class",
        primary_label: "Class Constructed Here",
        explanation: "The runtime could not allocate storage for the requested class instance.",
        process_status: 101,
        fact_names: &[],
    },
];
