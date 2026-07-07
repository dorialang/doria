# Stage 11 Primitive Helper Branch Review

Reviewed branch:

    feature/stage-11-native-primitive-helper-signatures

Result:

    Not merged as-is.

Reason:

    The updated Doria End-to-End Development Plan defines Stage 11 as MIR + interpreter oracle. The old branch extended NativeSmokeModule with bool and string helper support, but the new plan requires retiring NativeSmokeModule instead.

Kept:

    - General semantic regression tests that are valid Doria.
    - PHP backend regression tests for valid Doria source.
    - Future MIR/runtime fixtures for bool helpers and string-parameter helper examples.

Not kept:

    - NativeSmokeModule expansion.
    - Decision file claiming Stage 11 is primitive helper signatures.
    - Current native examples that depend on the old smoke expansion.
    - Current native tests that require bool/string helper support before MIR.

Future landing points:

    - MIR + interpreter oracle: Stage 11.
    - Bool values and conditions in MIR/runtime: later plan stage.
    - Runtime strings and string parameters: later plan stage.
    - Cranelift parity for those features: after MIR/interpreter support exists.

This note is not an accepted language decision.
