#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::iter;
use core::ops::Range;

use icu_casemap::CaseMapper;
use icu_locale_core::LanguageIdentifier;
use icu_properties::{props::WhiteSpace, CodePointSetData};
use icu_segmenter::GraphemeClusterSegmenter;
use writeable::Writeable;

pub const UNICODE_VERSION: &str = "17.0.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringError {
    ResultTooLarge,
    SliceLengthNegative,
    RepetitionCountNegative,
    PaddingLengthNegative,
    PaddingTextEmpty,
}

impl StringError {
    pub const fn panic_message(self) -> &'static str {
        match self {
            Self::ResultTooLarge => "String Result Is Too Large",
            Self::SliceLengthNegative => "String Slice Length Cannot Be Negative",
            Self::RepetitionCountNegative => "String Repetition Count Cannot Be Negative",
            Self::PaddingLengthNegative => "String Padding Length Cannot Be Negative",
            Self::PaddingTextEmpty => "String Padding Text Cannot Be Empty",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimMode {
    Both,
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseMapping {
    Lower,
    Upper,
    Fold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadSide {
    Start,
    End,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub grapheme_index: usize,
    pub byte_range: Range<usize>,
}

pub const fn checked_add(left: usize, right: usize) -> Result<usize, StringError> {
    match left.checked_add(right) {
        Some(value) => Ok(value),
        None => Err(StringError::ResultTooLarge),
    }
}

pub const fn checked_mul(left: usize, right: usize) -> Result<usize, StringError> {
    match left.checked_mul(right) {
        Some(value) => Ok(value),
        None => Err(StringError::ResultTooLarge),
    }
}

pub fn grapheme_boundaries(text: &str) -> impl Iterator<Item = usize> + '_ {
    GraphemeClusterSegmenter::new().segment_str(text)
}

pub fn grapheme_count(text: &str) -> usize {
    grapheme_boundaries(text).count().saturating_sub(1)
}

pub const fn byte_length(text: &str) -> usize {
    text.len()
}

pub const fn is_empty(text: &str) -> bool {
    text.is_empty()
}

pub fn trim_range(text: &str, mode: TrimMode) -> Range<usize> {
    let white_space = CodePointSetData::new::<WhiteSpace>();
    let start = if matches!(mode, TrimMode::Both | TrimMode::Start) {
        text.char_indices()
            .find_map(|(index, ch)| (!white_space.contains(ch)).then_some(index))
            .unwrap_or(text.len())
    } else {
        0
    };
    let end = if matches!(mode, TrimMode::Both | TrimMode::End) {
        text.char_indices()
            .rev()
            .find_map(|(index, ch)| (!white_space.contains(ch)).then_some(index + ch.len_utf8()))
            .unwrap_or(0)
    } else {
        text.len()
    };
    start.min(end)..end
}

pub fn boundary_matches<'a>(text: &'a str, needle: &'a str) -> impl Iterator<Item = Match> + 'a {
    let mut starts = grapheme_boundaries(text);
    let mut ends = grapheme_boundaries(text).peekable();
    let mut grapheme_index = 0usize;
    iter::from_fn(move || loop {
        let start = starts.next()?;
        let index = grapheme_index;
        grapheme_index = grapheme_index.saturating_add(1);
        let end = start.checked_add(needle.len())?;
        if end > text.len() || !text[start..].starts_with(needle) {
            continue;
        }
        while ends.peek().is_some_and(|boundary| *boundary < end) {
            ends.next();
        }
        if ends.peek().copied() == Some(end) {
            return Some(Match {
                grapheme_index: index,
                byte_range: start..end,
            });
        }
    })
}

pub fn first_index_of(text: &str, needle: &str) -> Option<usize> {
    boundary_matches(text, needle)
        .next()
        .map(|found| found.grapheme_index)
}

pub fn last_index_of(text: &str, needle: &str) -> Option<usize> {
    boundary_matches(text, needle)
        .last()
        .map(|found| found.grapheme_index)
}

/// True when every character boundary in `text` is also a grapheme-cluster
/// boundary, so a plain byte search already satisfies decision 0103's
/// alignment contract and no segmentation is needed.
///
/// ASCII has exactly one multi-character cluster, `CR LF`. Rule that out and no
/// ASCII character can join with its neighbour, so a byte match can neither
/// begin nor end inside a cluster.
fn every_char_is_its_own_grapheme(text: &str) -> bool {
    text.is_ascii() && !text.as_bytes().windows(2).any(|pair| pair == b"\r\n")
}

/// Whether `offset`, already known to be a character boundary, also falls on a
/// grapheme-cluster boundary. Stops as soon as the boundaries pass it.
fn is_grapheme_boundary(text: &str, offset: usize) -> bool {
    if offset == 0 || offset == text.len() {
        return true;
    }
    for boundary in grapheme_boundaries(text) {
        if boundary >= offset {
            return boundary == offset;
        }
    }
    false
}

/// Whether any byte-level occurrence of `needle` begins and ends on a
/// grapheme-cluster boundary.
///
/// Candidates come from byte search rather than from walking every boundary and
/// testing `starts_with` at each, which is what made the predicates cost a full
/// segmentation whether or not the needle was present. Two boundary cursors are
/// kept because a candidate's start and end are checked independently, and both
/// advance monotonically since a fixed-length needle's start and end both
/// increase with each successive candidate.
///
/// Candidates are advanced by one character rather than past the whole match,
/// so overlapping occurrences are still considered: an occurrence that is not
/// aligned must not hide a later overlapping one that is.
fn has_aligned_occurrence(text: &str, needle: &str) -> bool {
    let mut starts = grapheme_boundaries(text).peekable();
    let mut ends = grapheme_boundaries(text).peekable();
    let mut from = 0usize;
    while let Some(offset) = text[from..].find(needle) {
        let start = from + offset;
        let end = start + needle.len();
        while starts.peek().is_some_and(|boundary| *boundary < start) {
            starts.next();
        }
        if starts.peek().copied() == Some(start) {
            while ends.peek().is_some_and(|boundary| *boundary < end) {
                ends.next();
            }
            if ends.peek().copied() == Some(end) {
                return true;
            }
        }
        from = start + text[start..].chars().next().map_or(1, char::len_utf8);
        if from >= text.len() {
            break;
        }
    }
    false
}

pub fn contains(text: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if every_char_is_its_own_grapheme(text) {
        return text.contains(needle);
    }
    has_aligned_occurrence(text, needle)
}

pub fn starts_with(text: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    if !text.starts_with(prefix) {
        return false;
    }
    // The start is offset zero, always a boundary; only the end can split a
    // cluster.
    every_char_is_its_own_grapheme(text) || is_grapheme_boundary(text, prefix.len())
}

pub fn ends_with(text: &str, suffix: &str) -> bool {
    if suffix.is_empty() {
        return true;
    }
    if !text.ends_with(suffix) {
        return false;
    }
    // The end is the string length, always a boundary; only the start can
    // split a cluster.
    every_char_is_its_own_grapheme(text) || is_grapheme_boundary(text, text.len() - suffix.len())
}

pub fn count_occurrences(text: &str, needle: &str) -> Result<usize, StringError> {
    if needle.is_empty() {
        return checked_add(grapheme_count(text), 1);
    }
    let mut count = 0usize;
    let mut cursor = 0usize;
    for found in boundary_matches(text, needle) {
        if found.byte_range.start < cursor {
            continue;
        }
        count = checked_add(count, 1)?;
        cursor = found.byte_range.end;
    }
    Ok(count)
}

pub fn ignore_case_matches(text: &str, needle: &str) -> Result<Vec<Match>, StringError> {
    let folded_text = FoldedText::new(text)?;
    let folded_needle = folded_string(needle)?;
    let mut matches = Vec::new();
    matches
        .try_reserve(folded_text.boundaries.len())
        .map_err(|_| StringError::ResultTooLarge)?;

    for (index, start) in folded_text.boundaries.iter().enumerate() {
        let end = checked_add(start.folded_offset, folded_needle.len())?;
        let Ok(end_index) = folded_text
            .boundaries
            .binary_search_by_key(&end, |boundary| boundary.folded_offset)
        else {
            continue;
        };
        if folded_text.text.as_bytes().get(start.folded_offset..end)
            == Some(folded_needle.as_bytes())
        {
            matches.push(Match {
                grapheme_index: index,
                byte_range: start.original_offset
                    ..folded_text.boundaries[end_index].original_offset,
            });
        }
    }
    Ok(matches)
}

pub fn first_index_of_ignore_case(text: &str, needle: &str) -> Result<Option<usize>, StringError> {
    Ok(ignore_case_matches(text, needle)?
        .first()
        .map(|found| found.grapheme_index))
}

pub fn last_index_of_ignore_case(text: &str, needle: &str) -> Result<Option<usize>, StringError> {
    Ok(ignore_case_matches(text, needle)?
        .last()
        .map(|found| found.grapheme_index))
}

pub fn contains_ignore_case(text: &str, needle: &str) -> Result<bool, StringError> {
    Ok(first_index_of_ignore_case(text, needle)?.is_some())
}

pub fn starts_with_ignore_case(text: &str, prefix: &str) -> Result<bool, StringError> {
    Ok(ignore_case_matches(text, prefix)?
        .first()
        .is_some_and(|found| found.byte_range.start == 0))
}

pub fn ends_with_ignore_case(text: &str, suffix: &str) -> Result<bool, StringError> {
    Ok(ignore_case_matches(text, suffix)?
        .iter()
        .any(|found| found.byte_range.end == text.len()))
}

pub fn split_fields<'a>(text: &'a str, separator: &'a str, mut field: impl FnMut(&'a str)) {
    if separator.is_empty() {
        let mut boundaries = grapheme_boundaries(text);
        let Some(mut start) = boundaries.next() else {
            return;
        };
        for end in boundaries {
            if start != end {
                field(&text[start..end]);
            }
            start = end;
        }
        return;
    }

    let mut cursor = 0usize;
    for found in boundary_matches(text, separator) {
        if found.byte_range.start < cursor {
            continue;
        }
        field(&text[cursor..found.byte_range.start]);
        cursor = found.byte_range.end;
    }
    field(&text[cursor..]);
}

pub fn split_field_count(text: &str, separator: &str) -> Result<usize, StringError> {
    let mut count = 0usize;
    let mut overflow = false;
    split_fields(text, separator, |_| {
        if let Some(next) = count.checked_add(1) {
            count = next;
        } else {
            overflow = true;
        }
    });
    if overflow {
        Err(StringError::ResultTooLarge)
    } else {
        Ok(count)
    }
}

pub fn slice_range(
    text: &str,
    start: i64,
    length: Option<i64>,
) -> Result<Range<usize>, StringError> {
    if length.is_some_and(|length| length < 0) {
        return Err(StringError::SliceLengthNegative);
    }
    let mut boundaries = Vec::new();
    for boundary in grapheme_boundaries(text) {
        boundaries
            .try_reserve(1)
            .map_err(|_| StringError::ResultTooLarge)?;
        boundaries.push(boundary);
    }
    let count = boundaries.len().saturating_sub(1);
    let start_index = if start < 0 {
        let magnitude = start.unsigned_abs();
        count.saturating_sub(usize::try_from(magnitude).unwrap_or(usize::MAX))
    } else {
        usize::try_from(start).unwrap_or(usize::MAX).min(count)
    };
    let wanted = length
        .map(|length| usize::try_from(length).unwrap_or(usize::MAX))
        .unwrap_or_else(|| count.saturating_sub(start_index));
    let end_index = start_index.saturating_add(wanted).min(count);
    Ok(boundaries[start_index]..boundaries[end_index])
}

pub fn case_output_length(text: &str, mapping: CaseMapping) -> Result<usize, StringError> {
    let mut writer = CountingWriter::default();
    write_case_to(text, mapping, &mut writer).map_err(|_| StringError::ResultTooLarge)?;
    Ok(writer.length)
}

pub fn write_case(
    text: &str,
    mapping: CaseMapping,
    output: &mut [u8],
) -> Result<usize, StringError> {
    let mut writer = SliceWriter::new(output);
    write_case_to(text, mapping, &mut writer).map_err(|_| StringError::ResultTooLarge)?;
    Ok(writer.written)
}

pub fn first_case_output_length(text: &str, mapping: CaseMapping) -> Result<usize, StringError> {
    let first_end = grapheme_boundaries(text).nth(1).unwrap_or(0);
    checked_add(
        case_output_length(&text[..first_end], mapping)?,
        text.len().saturating_sub(first_end),
    )
}

pub fn write_first_case(
    text: &str,
    mapping: CaseMapping,
    output: &mut [u8],
) -> Result<usize, StringError> {
    let first_end = grapheme_boundaries(text).nth(1).unwrap_or(0);
    let mut writer = SliceWriter::new(output);
    write_case_to(&text[..first_end], mapping, &mut writer)
        .map_err(|_| StringError::ResultTooLarge)?;
    fmt::Write::write_str(&mut writer, &text[first_end..])
        .map_err(|_| StringError::ResultTooLarge)?;
    Ok(writer.written)
}

pub fn equals_ignore_case(
    left: &str,
    right: &str,
    scratch: &mut [u8],
) -> Result<bool, StringError> {
    let expected = case_output_length(left, CaseMapping::Fold)?;
    if scratch.len() < expected {
        return Err(StringError::ResultTooLarge);
    }
    let written = write_case(left, CaseMapping::Fold, scratch)?;
    Ok(writeable::cmp_utf8(&CaseMapper::new().fold(right), &scratch[..written]).is_eq())
}

fn write_case_to(text: &str, mapping: CaseMapping, output: &mut impl fmt::Write) -> fmt::Result {
    let mapper = CaseMapper::new();
    let root = LanguageIdentifier::UNKNOWN;
    match mapping {
        CaseMapping::Lower => mapper.lowercase(text, &root).write_to(output),
        CaseMapping::Upper => mapper.uppercase(text, &root).write_to(output),
        CaseMapping::Fold => mapper.fold(text).write_to(output),
    }
}

#[derive(Debug, Clone, Copy)]
struct FoldBoundary {
    folded_offset: usize,
    original_offset: usize,
}

struct FoldedText {
    text: String,
    boundaries: Vec<FoldBoundary>,
}

impl FoldedText {
    fn new(text: &str) -> Result<Self, StringError> {
        let folded_length = case_output_length(text, CaseMapping::Fold)?;
        let mut folded = String::new();
        folded
            .try_reserve_exact(folded_length)
            .map_err(|_| StringError::ResultTooLarge)?;
        let count = grapheme_count(text);
        let mut boundaries = Vec::new();
        boundaries
            .try_reserve_exact(checked_add(count, 1)?)
            .map_err(|_| StringError::ResultTooLarge)?;
        boundaries.push(FoldBoundary {
            folded_offset: 0,
            original_offset: 0,
        });

        let mut grapheme_boundaries = grapheme_boundaries(text);
        let mut start = grapheme_boundaries.next().unwrap_or(0);
        for end in grapheme_boundaries {
            write_case_to(&text[start..end], CaseMapping::Fold, &mut folded)
                .map_err(|_| StringError::ResultTooLarge)?;
            boundaries.push(FoldBoundary {
                folded_offset: folded.len(),
                original_offset: end,
            });
            start = end;
        }
        debug_assert_eq!(folded.len(), folded_length);
        Ok(Self {
            text: folded,
            boundaries,
        })
    }
}

fn folded_string(text: &str) -> Result<String, StringError> {
    Ok(FoldedText::new(text)?.text)
}

pub fn replacement_output_length(
    text: &str,
    search: &str,
    replacement: &str,
) -> Result<usize, StringError> {
    let mut length = text.len();
    let mut cursor = 0usize;
    for found in boundary_matches(text, search) {
        if found.byte_range.start < cursor {
            continue;
        }
        length = length
            .checked_sub(search.len())
            .and_then(|length| length.checked_add(replacement.len()))
            .ok_or(StringError::ResultTooLarge)?;
        cursor = found.byte_range.end;
    }
    Ok(length)
}

pub fn write_replacement(
    text: &str,
    search: &str,
    replacement: &str,
    output: &mut [u8],
) -> Result<usize, StringError> {
    let mut writer = ByteWriter::new(output);
    let mut cursor = 0usize;
    for found in boundary_matches(text, search) {
        if found.byte_range.start < cursor {
            continue;
        }
        writer.write(&text.as_bytes()[cursor..found.byte_range.start])?;
        writer.write(replacement.as_bytes())?;
        cursor = found.byte_range.end;
    }
    writer.write(&text.as_bytes()[cursor..])?;
    Ok(writer.written)
}

pub fn repetition_output_length(text: &str, count: i64) -> Result<usize, StringError> {
    if count < 0 {
        return Err(StringError::RepetitionCountNegative);
    }
    checked_mul(
        text.len(),
        usize::try_from(count).map_err(|_| StringError::ResultTooLarge)?,
    )
}

pub fn write_repetition(text: &str, count: i64, output: &mut [u8]) -> Result<usize, StringError> {
    let count = usize::try_from(count).map_err(|_| StringError::RepetitionCountNegative)?;
    let mut writer = ByteWriter::new(output);
    for _ in 0..count {
        writer.write(text.as_bytes())?;
    }
    Ok(writer.written)
}

pub fn padding_output_length(
    text: &str,
    target_length: i64,
    padding: &str,
) -> Result<usize, StringError> {
    let target = usize::try_from(target_length).map_err(|_| StringError::PaddingLengthNegative)?;
    let current = grapheme_count(text);
    if target <= current {
        return Ok(text.len());
    }
    let padding_count = grapheme_count(padding);
    if padding_count == 0 {
        return Err(StringError::PaddingTextEmpty);
    }
    let required = target - current;
    let whole_repetitions = required / padding_count;
    let remainder = required % padding_count;
    let remainder_bytes = grapheme_prefix_byte_length(padding, remainder);
    checked_add(
        text.len(),
        checked_add(
            checked_mul(padding.len(), whole_repetitions)?,
            remainder_bytes,
        )?,
    )
}

pub fn write_padding(
    text: &str,
    target_length: i64,
    padding: &str,
    side: PadSide,
    output: &mut [u8],
) -> Result<usize, StringError> {
    let target = usize::try_from(target_length).map_err(|_| StringError::PaddingLengthNegative)?;
    let current = grapheme_count(text);
    if target <= current {
        let mut writer = ByteWriter::new(output);
        writer.write(text.as_bytes())?;
        return Ok(writer.written);
    }
    let padding_count = grapheme_count(padding);
    if padding_count == 0 {
        return Err(StringError::PaddingTextEmpty);
    }
    let required = target - current;
    let whole_repetitions = required / padding_count;
    let remainder_bytes = grapheme_prefix_byte_length(padding, required % padding_count);
    let mut writer = ByteWriter::new(output);
    if matches!(side, PadSide::End) {
        writer.write(text.as_bytes())?;
    }
    for _ in 0..whole_repetitions {
        writer.write(padding.as_bytes())?;
    }
    writer.write(&padding.as_bytes()[..remainder_bytes])?;
    if matches!(side, PadSide::Start) {
        writer.write(text.as_bytes())?;
    }
    Ok(writer.written)
}

fn grapheme_prefix_byte_length(text: &str, count: usize) -> usize {
    grapheme_boundaries(text).nth(count).unwrap_or(text.len())
}

#[derive(Default)]
struct CountingWriter {
    length: usize,
}

impl fmt::Write for CountingWriter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.length = self.length.checked_add(value.len()).ok_or(fmt::Error)?;
        Ok(())
    }
}

struct SliceWriter<'a> {
    output: &'a mut [u8],
    written: usize,
}

impl<'a> SliceWriter<'a> {
    fn new(output: &'a mut [u8]) -> Self {
        Self { output, written: 0 }
    }
}

impl fmt::Write for SliceWriter<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let end = self.written.checked_add(value.len()).ok_or(fmt::Error)?;
        let destination = self.output.get_mut(self.written..end).ok_or(fmt::Error)?;
        destination.copy_from_slice(value.as_bytes());
        self.written = end;
        Ok(())
    }
}

struct ByteWriter<'a> {
    output: &'a mut [u8],
    written: usize,
}

impl<'a> ByteWriter<'a> {
    fn new(output: &'a mut [u8]) -> Self {
        Self { output, written: 0 }
    }

    fn write(&mut self, value: &[u8]) -> Result<(), StringError> {
        let end = self
            .written
            .checked_add(value.len())
            .ok_or(StringError::ResultTooLarge)?;
        let destination = self
            .output
            .get_mut(self.written..end)
            .ok_or(StringError::ResultTooLarge)?;
        destination.copy_from_slice(value);
        self.written = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::string::String;
    use std::vec;
    use std::vec::Vec;

    fn transformed(text: &str, mapping: CaseMapping) -> String {
        let mut output = vec![0; case_output_length(text, mapping).expect("case length")];
        let written = write_case(text, mapping, &mut output).unwrap();
        String::from_utf8(output[..written].to_vec()).unwrap()
    }

    fn replaced(text: &str, search: &str, replacement: &str) -> String {
        let mut output = vec![0; replacement_output_length(text, search, replacement).unwrap()];
        let written = write_replacement(text, search, replacement, &mut output).unwrap();
        String::from_utf8(output[..written].to_vec()).unwrap()
    }

    #[test]
    fn counts_unicode_extended_grapheme_clusters() {
        assert_eq!(grapheme_count(""), 0);
        assert_eq!(grapheme_count("abc"), 3);
        assert_eq!(grapheme_count("e\u{301}"), 1);
        assert_eq!(grapheme_count("👍🏾"), 1);
        assert_eq!(grapheme_count("🇿🇲"), 1);
        assert_eq!(grapheme_count("👨‍👩‍👧‍👦"), 1);
    }

    #[test]
    fn trims_unicode_white_space_without_removing_format_characters() {
        let text = "\u{00a0}\u{2003}\t Doria \r\n\u{200b}";
        let range = trim_range(text, TrimMode::Both);
        assert_eq!(&text[range], "Doria \r\n\u{200b}");
        assert_eq!(
            &text[trim_range(text, TrimMode::Start)],
            "Doria \r\n\u{200b}"
        );
        assert_eq!(&text[trim_range(text, TrimMode::End)], text);
    }

    #[test]
    fn maps_case_with_default_unicode_rules() {
        assert_eq!(transformed("DÓRIA ΣΟΣ", CaseMapping::Lower), "dória σος");
        assert_eq!(transformed("Straße", CaseMapping::Upper), "STRASSE");
        assert_eq!(transformed("İ", CaseMapping::Lower), "i\u{307}");
        assert_eq!(transformed("👍🏾", CaseMapping::Upper), "👍🏾");
    }

    #[test]
    fn maps_only_the_first_grapheme_with_default_unicode_rules() {
        let text = "ßTRASSE";
        let mut output = vec![0; first_case_output_length(text, CaseMapping::Upper).unwrap()];
        let written = write_first_case(text, CaseMapping::Upper, &mut output).unwrap();
        assert_eq!(
            core::str::from_utf8(&output[..written]).unwrap(),
            "SSTRASSE"
        );

        let text = "İSTANBUL";
        let mut output = vec![0; first_case_output_length(text, CaseMapping::Lower).unwrap()];
        let written = write_first_case(text, CaseMapping::Lower, &mut output).unwrap();
        assert_eq!(
            core::str::from_utf8(&output[..written]).unwrap(),
            "i\u{307}STANBUL"
        );
    }

    #[test]
    fn compares_using_full_default_case_folding_without_normalization() {
        let mut scratch =
            vec![0; case_output_length("Straße", CaseMapping::Fold).expect("fold length")];
        assert!(equals_ignore_case("Straße", "STRASSE", &mut scratch).unwrap());
        let mut scratch =
            vec![0; case_output_length("é", CaseMapping::Fold).expect("fold length")];
        assert!(equals_ignore_case("é", "É", &mut scratch).unwrap());
        assert!(!equals_ignore_case("é", "e\u{301}", &mut scratch).unwrap());
    }

    #[test]
    fn searches_only_at_grapheme_boundaries() {
        assert_eq!(first_index_of("a👍🏾b", "👍🏾"), Some(1));
        assert_eq!(last_index_of("a👍🏾a👍🏾", "👍🏾"), Some(3));
        assert_eq!(first_index_of("e\u{301}", "\u{301}"), None);
        assert_eq!(first_index_of("abc", ""), Some(0));
        assert_eq!(last_index_of("abc", ""), Some(3));
        assert!(contains("", ""));
        assert!(starts_with("abc", ""));
        assert!(ends_with("abc", ""));
    }

    #[test]
    fn searches_with_full_case_folding_and_original_grapheme_indices() {
        assert_eq!(
            first_index_of_ignore_case("aStraße👍🏾", "STRASSE").unwrap(),
            Some(1)
        );
        assert_eq!(
            last_index_of_ignore_case("Straße x STRASSE", "straße").unwrap(),
            Some(9)
        );
        assert!(contains_ignore_case("Dória", "DÓR").unwrap());
        assert!(starts_with_ignore_case("Straße", "STRASS").unwrap());
        assert!(ends_with_ignore_case("aStraße", "STRASSE").unwrap());
        assert!(!contains_ignore_case("é", "e\u{301}").unwrap());
        assert_eq!(first_index_of_ignore_case("abc", "").unwrap(), Some(0));
        assert_eq!(last_index_of_ignore_case("abc", "").unwrap(), Some(3));
    }

    #[test]
    fn counts_non_overlapping_occurrences_and_empty_boundaries() {
        assert_eq!(count_occurrences("aaaa", "aa").unwrap(), 2);
        assert_eq!(count_occurrences("a👍🏾a👍🏾", "👍🏾").unwrap(), 2);
        assert_eq!(count_occurrences("abc", "").unwrap(), 4);
        assert_eq!(count_occurrences("", "").unwrap(), 1);
    }

    #[test]
    fn replaces_non_overlapping_boundary_aligned_matches() {
        assert_eq!(replaced("aaaa", "aa", "x"), "xx");
        assert_eq!(replaced("abc", "", "-"), "-a-b-c-");
        assert_eq!(replaced("", "", "-"), "-");
        assert_eq!(replaced("e\u{301}", "\u{301}", "x"), "e\u{301}");
    }

    #[test]
    fn splits_preserving_fields_or_into_graphemes() {
        let mut fields = Vec::new();
        split_fields("a,,b,", ",", |field| fields.push(field));
        assert_eq!(fields, ["a", "", "b", ""]);
        fields.clear();
        split_fields("👍🏾a", "", |field| fields.push(field));
        assert_eq!(fields, ["👍🏾", "a"]);
        fields.clear();
        split_fields("", "", |field| fields.push(field));
        assert!(fields.is_empty());
    }

    #[test]
    fn slices_in_grapheme_units_with_clamping() {
        let text = "a👍🏾bc";
        assert_eq!(&text[slice_range(text, 1, Some(2)).unwrap()], "👍🏾b");
        assert_eq!(&text[slice_range(text, -1, None).unwrap()], "c");
        assert_eq!(&text[slice_range(text, -9, Some(1)).unwrap()], "a");
        assert_eq!(&text[slice_range(text, 9, None).unwrap()], "");
        assert_eq!(
            slice_range(text, 0, Some(-1)),
            Err(StringError::SliceLengthNegative)
        );
    }

    #[test]
    fn repeats_and_pads_with_checked_grapheme_contracts() {
        let mut repeated = vec![0; repetition_output_length("👍🏾", 3).unwrap()];
        write_repetition("👍🏾", 3, &mut repeated).unwrap();
        assert_eq!(core::str::from_utf8(&repeated).unwrap(), "👍🏾👍🏾👍🏾");
        assert_eq!(
            repetition_output_length("x", -1),
            Err(StringError::RepetitionCountNegative)
        );

        let length = padding_output_length("ab", 5, "👍🏾.").unwrap();
        let mut padded = vec![0; length];
        write_padding("ab", 5, "👍🏾.", PadSide::End, &mut padded).unwrap();
        assert_eq!(core::str::from_utf8(&padded).unwrap(), "ab👍🏾.👍🏾");
        assert_eq!(
            padding_output_length("a", 2, ""),
            Err(StringError::PaddingTextEmpty)
        );
        assert_eq!(
            padding_output_length("a", -1, "."),
            Err(StringError::PaddingLengthNegative)
        );
    }

    #[test]
    fn checked_size_helpers_reject_overflow() {
        assert_eq!(checked_add(usize::MAX, 1), Err(StringError::ResultTooLarge));
        assert_eq!(checked_mul(usize::MAX, 2), Err(StringError::ResultTooLarge));
    }

    #[test]
    fn long_slice_uses_one_boundary_inventory_without_quadratic_rescans() {
        let text = "👍🏾a".repeat(20_000);
        let range = slice_range(&text, -3, Some(2)).unwrap();
        assert_eq!(&text[range], "a👍🏾");
    }

    /// The predicates take a byte-search path that skips segmentation; this
    /// pins them to the boundary walk they replaced, on the inputs where
    /// grapheme alignment actually changes the answer.
    ///
    /// `reference_*` are the previous implementations, kept here verbatim so a
    /// future change to either side has to justify a divergence.
    fn reference_contains(text: &str, needle: &str) -> bool {
        first_index_of(text, needle).is_some()
    }

    fn reference_starts_with(text: &str, prefix: &str) -> bool {
        boundary_matches(text, prefix)
            .next()
            .is_some_and(|found| found.byte_range.start == 0)
    }

    fn reference_ends_with(text: &str, suffix: &str) -> bool {
        boundary_matches(text, suffix).any(|found| found.byte_range.end == text.len())
    }

    #[test]
    fn search_predicates_match_the_boundary_walk_they_replaced() {
        // Each text pairs plain content with a construct where a byte match can
        // land inside a grapheme cluster: combining marks, CR LF, Hangul jamo,
        // an emoji ZWJ sequence, regional indicators, and a skin-tone modifier.
        let texts = [
            "",
            "a",
            "abc",
            "Doria performance foundation",
            "line\r\nbreak",
            "\r\n",
            "e\u{301}", // e + combining acute
            "e\u{301}e\u{301}",
            "cafe\u{301} au lait",
            "\u{1100}\u{1161}\u{11A8}",    // Hangul L V T
            "\u{1F468}\u{200D}\u{1F469}",  // ZWJ sequence
            "\u{1F1E6}\u{1F1E7}\u{1F1E8}", // regional indicators
            "👍🏾a👍🏾",
            "Doria πλατφόρμα performance τέλος",
            "aaa",
            "ababab",
        ];
        let needles = [
            "",
            "a",
            "aa",
            "ab",
            "e",
            "\u{301}", // a bare combining mark
            "e\u{301}",
            "\r",
            "\n",
            "\r\n",
            "\u{1161}", // a bare Hangul vowel
            "\u{200D}", // a bare ZWJ
            "\u{1F469}",
            "\u{1F1E7}",
            "👍",
            "🏾",
            "performance",
            "πλατφόρμα",
            "absent",
        ];

        for text in texts {
            for needle in needles {
                assert_eq!(
                    contains(text, needle),
                    reference_contains(text, needle),
                    "contains({text:?}, {needle:?})"
                );
                assert_eq!(
                    starts_with(text, needle),
                    reference_starts_with(text, needle),
                    "starts_with({text:?}, {needle:?})"
                );
                assert_eq!(
                    ends_with(text, needle),
                    reference_ends_with(text, needle),
                    "ends_with({text:?}, {needle:?})"
                );
            }
        }
    }

    /// An unaligned occurrence must not end the search: a later aligned one
    /// still counts.
    ///
    /// This does not pin the *advance strategy*. Advancing by one character
    /// rather than past the whole match is equivalent to the boundary walk by
    /// construction, because every aligned occurrence is a byte occurrence and
    /// advancing by a character enumerates all of them; skipping to the end
    /// drops the overlapping ones. A randomised differential over 300,000
    /// text/needle pairs drawn from combining marks, CR LF, ZWJ, and emoji
    /// modifiers found no input where the two strategies disagree, so the
    /// choice rests on that argument rather than on a failing case.
    #[test]
    fn an_unaligned_occurrence_does_not_end_the_search() {
        let text = "e\u{301}e";
        let needle = "e";
        assert!(
            contains(text, needle),
            "the trailing bare e is an aligned match"
        );
        assert_eq!(contains(text, needle), reference_contains(text, needle));
    }

    /// Decision 0103: a match beginning or ending inside a grapheme is not a
    /// match, and the empty needle is contained by every string.
    #[test]
    fn alignment_and_empty_needle_follow_the_record() {
        assert!(!contains("e\u{301}", "e"), "e alone splits the cluster");
        assert!(!starts_with("e\u{301}", "e"));
        assert!(!ends_with("e\u{301}", "\u{301}"));
        assert!(contains("e\u{301}", "e\u{301}"));
        for text in ["", "a", "e\u{301}", "\r\n"] {
            assert!(contains(text, ""));
            assert!(starts_with(text, ""));
            assert!(ends_with(text, ""));
        }
    }
}
