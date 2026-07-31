# Unicode String Runtime

Documentation role: implementation note.

Doria strings contain valid UTF-8. The compiler's MIR interpreter and
`doria-rt` both use the workspace `doria-unicode` crate for the executable
String surface, so Cranelift and LLVM cannot acquire different text semantics.
The crate uses pinned ICU4X 2.2 data corresponding to Unicode 17.0.0.

The shared layer defines:

- extended grapheme-cluster boundaries for length, search, replacement, split,
  slice, and padding;
- the Unicode White_Space property for trimming;
- locale-independent default lower/upper mappings, first-grapheme casing, and
  full case folding for equality and boundary-aligned search;
- non-overlapping occurrence counting with empty needles matching grapheme
  boundaries;
- checked output-size planning before allocation; and
- the canonical String panic messages.

It performs no implicit normalization. Canonically equivalent but differently
encoded strings remain distinct under equality, search, and replacement.
`byteLength` and `isEmpty` read the private runtime String header in O(1);
grapheme-aware operations traverse UTF-8 without permanent per-string caches.

The PHP compatibility backend refuses this surface when it cannot preserve the
same contract without optional extensions. PHP behavior never defines Doria
String behavior.
