//! The engine-neutral `neo:strings` implementation.
//!
//! This module is Rust, not ECMAScript. Both runtimes bind these functions
//! directly: the V8 runtime installs them as native V8 function objects, and the
//! QuickJS runtime exports them from a native QuickJS module. There is no
//! JavaScript implementation and no JavaScript module wrapper in this crate.
//!
//! The functions operate on UTF-16 code units, not Rust `str` values. A
//! JavaScript string can contain a lone surrogate, and this module preserves
//! it: a lone surrogate is one code point that folds to itself and never
//! matches half of a surrogate pair. Indices are UTF-16 code unit offsets, like
//! `String.prototype.indexOf`, not Unicode code point offsets.
//!
//! Folded matching compares Unicode simple case-folding equivalence from the
//! generated table in [`case_fold`]. That is the same relation the previous
//! ECMAScript implementation obtained from the engine's `/iu`
//! canonicalization. It is locale-independent and is not a `toLowerCase()`
//! comparison: `"K"` equals U+212A KELVIN SIGN and `"s"` equals U+017F LATIN
//! SMALL LETTER LONG S, while `"ss"` does not equal `"ß"` (that is full
//! folding) and `"I"` does not equal U+0131 DOTLESS I. The table is Unicode
//! 16.0, fixed at build time, so both engines agree instead of following their
//! own Unicode release.
//!
//! Searching is linear in the text plus the search string (KMP over the
//! fold-equivalence relation) and uses O(search) extra memory, so a repeated
//! prefix cannot make it quadratic. `last_index_of_fold` stops as soon as no
//! pending or future match can begin at or before its limit, so a small limit
//! does not scan a long tail.
//!
//! With no `chars`, trimming is exactly JavaScript's `String.prototype.trim`,
//! `trimStart`, and `trimEnd`: that whitespace set contains U+FEFF and excludes
//! U+0085. It is deliberately not the Go-derived `neo:chars` `isSpace`. The
//! module has no host operation, no capability, and no I/O. It is not a
//! sandbox.

use std::collections::HashSet;

#[cfg(test)]
use std::cell::Cell;

use unicode_general_category::{GeneralCategory, get_general_category};

mod case_fold;

/// The canonical import specifier and registry key.
pub const SPECIFIER: &str = "neo:strings";

/// The named exports, in the module object's property order.
pub const EXPORTS: &[&str] = &[
    "equalFold",
    "startsWithFold",
    "endsWithFold",
    "indexOfFold",
    "lastIndexOfFold",
    "trim",
    "trimStart",
    "trimEnd",
];

/// The error message a binding reports when `text` is not a string.
pub const TEXT_MESSAGE: &str = "text must be a string";

/// The error message a binding reports when the second comparison argument is
/// not a string.
pub const OTHER_MESSAGE: &str = "other must be a string";

/// The error message a binding reports when `prefix` is not a string.
pub const PREFIX_MESSAGE: &str = "prefix must be a string";

/// The error message a binding reports when `suffix` is not a string.
pub const SUFFIX_MESSAGE: &str = "suffix must be a string";

/// The error message a binding reports when `search` is not a string.
pub const SEARCH_MESSAGE: &str = "search must be a string";

/// The error message a binding reports when `chars` is not a string.
pub const CHARS_MESSAGE: &str = "chars must be a string";

/// The error message a binding reports for a non-number `fromIndex`.
pub const FROM_INDEX_NOT_NUMBER: &str = "fromIndex must be a number";

/// The error message a binding reports for a non-integer `fromIndex`.
///
/// This is also the implementation marker the artifact scans look for: a build
/// with the `strings` feature cannot contain the Rust binding without this
/// string.
pub const FROM_INDEX_NOT_INTEGER: &str = "fromIndex must be an integer";

/// The two ways a `fromIndex` argument can be invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexError {
    /// The value is present but is not a JavaScript number.
    NotNumber,
    /// The value is a number but is not an integer; `NaN` is in this class.
    NotInteger,
}

impl IndexError {
    /// The message a binding reports for this error.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NotNumber => FROM_INDEX_NOT_NUMBER,
            Self::NotInteger => FROM_INDEX_NOT_INTEGER,
        }
    }
}

/// Clamps an optional `fromIndex` argument to `0..=length`.
///
/// `None` is an absent or `undefined` argument and uses `fallback`. `Infinity`
/// and `-Infinity` clamp like the corresponding `String.prototype` arguments.
/// Any other non-integer, including `NaN`, is [`IndexError::NotInteger`]; this
/// module does not coerce its arguments.
#[must_use = "an out-of-range index is a documented error"]
pub fn to_index(value: Option<f64>, length: usize, fallback: usize) -> Result<usize, IndexError> {
    let Some(value) = value else {
        return Ok(fallback);
    };
    if value == f64::INFINITY {
        return Ok(length);
    }
    if value == f64::NEG_INFINITY {
        return Ok(0);
    }
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(IndexError::NotInteger);
    }
    if value <= 0.0 {
        return Ok(0);
    }
    if value >= length as f64 {
        return Ok(length);
    }
    Ok(value as usize)
}

// Counts code point reads in this thread during a unit test. The production
// build has no counter, hook, or test-only branch; only the performance-bound
// test in this module observes it.
#[cfg(test)]
thread_local! {
    static CODE_POINT_READS: Cell<usize> = const { Cell::new(0) };
}

/// Decodes the code point at a UTF-16 index.
///
/// A well-formed surrogate pair is one code point of width two. A lone
/// surrogate is the code unit itself, of width one. The caller must pass an
/// index below `units.len()`.
fn code_point_at(units: &[u16], index: usize) -> (u32, usize) {
    #[cfg(test)]
    CODE_POINT_READS.with(|reads| reads.set(reads.get() + 1));
    let unit = units[index];
    if (0xD800..=0xDBFF).contains(&unit)
        && index + 1 < units.len()
        && (0xDC00..=0xDFFF).contains(&units[index + 1])
    {
        let high = u32::from(unit) - 0xD800;
        let low = u32::from(units[index + 1]) - 0xDC00;
        (0x1_0000 + (high << 10) + low, 2)
    } else {
        (u32::from(unit), 1)
    }
}

/// Decodes the code point that ends just before `end`, returning its start.
///
/// The caller must pass `end > 0`.
fn code_point_before(units: &[u16], end: usize) -> (u32, usize) {
    let unit = units[end - 1];
    if (0xDC00..=0xDFFF).contains(&unit) && end >= 2 {
        let high = units[end - 2];
        if (0xD800..=0xDBFF).contains(&high) {
            let high = u32::from(high) - 0xD800;
            let low = u32::from(unit) - 0xDC00;
            return (0x1_0000 + (high << 10) + low, end - 2);
        }
    }
    (u32::from(unit), end - 1)
}

/// Moves `index` forward off the trailing half of a surrogate pair.
///
/// No well-formed match can start in the middle of a pair, so this changes no
/// result; it only keeps the scan at code point boundaries.
fn code_point_start(units: &[u16], index: usize) -> usize {
    if index == 0 || index >= units.len() {
        return index;
    }
    let unit = units[index];
    if (0xDC00..=0xDFFF).contains(&unit) {
        let previous = units[index - 1];
        if (0xD800..=0xDBFF).contains(&previous) {
            return index + 1;
        }
    }
    index
}

/// True when two code points are equivalent under simple case folding.
fn fold_equal(left: u32, right: u32) -> bool {
    left == right || case_fold::simple_fold(left) == case_fold::simple_fold(right)
}

/// Counts the code points in `text`.
fn code_point_count(text: &[u16]) -> usize {
    let mut count = 0;
    let mut index = 0;
    while index < text.len() {
        let (_, width) = code_point_at(text, index);
        index += width;
        count += 1;
    }
    count
}

/// Copies the code points of `text` into a vector.
fn code_points_of(text: &[u16]) -> Vec<u32> {
    let mut points = Vec::with_capacity(code_point_count(text));
    let mut index = 0;
    while index < text.len() {
        let (point, width) = code_point_at(text, index);
        points.push(point);
        index += width;
    }
    points
}

/// Computes the KMP prefix function under fold equivalence.
///
/// Fold equivalence is established by a single canonical mapping, so it is
/// reflexive, symmetric, and transitive. The usual prefix-function recurrence
/// is therefore valid even when the pattern contains different code points
/// from one fold class, such as `"K"` and U+212A.
fn fold_prefix(pattern: &[u32]) -> Vec<usize> {
    let mut prefix = vec![0; pattern.len()];
    let mut matched = 0;
    for index in 1..pattern.len() {
        let point = pattern[index];
        while matched > 0 && !fold_equal(point, pattern[matched]) {
            matched = prefix[matched - 1];
        }
        if fold_equal(point, pattern[matched]) {
            matched += 1;
        }
        prefix[index] = matched;
    }
    prefix
}

/// True when two strings are equal under simple case folding.
///
/// Locale-independent: this is not a `toLowerCase()` comparison and does not
/// apply Turkish or other locale rules.
#[must_use]
pub fn equal_fold(text: &[u16], other: &[u16]) -> bool {
    if text == other {
        return true;
    }
    let mut text_index = 0;
    let mut other_index = 0;
    while text_index < text.len() && other_index < other.len() {
        let (text_point, text_width) = code_point_at(text, text_index);
        let (other_point, other_width) = code_point_at(other, other_index);
        if !fold_equal(text_point, other_point) {
            return false;
        }
        text_index += text_width;
        other_index += other_width;
    }
    text_index == text.len() && other_index == other.len()
}

/// True when `text` begins with `prefix`, ignoring case.
#[must_use]
pub fn starts_with_fold(text: &[u16], prefix: &[u16]) -> bool {
    let mut text_index = 0;
    let mut prefix_index = 0;
    while prefix_index < prefix.len() {
        if text_index >= text.len() {
            return false;
        }
        let (text_point, text_width) = code_point_at(text, text_index);
        let (prefix_point, prefix_width) = code_point_at(prefix, prefix_index);
        if !fold_equal(text_point, prefix_point) {
            return false;
        }
        text_index += text_width;
        prefix_index += prefix_width;
    }
    true
}

/// True when `text` ends with `suffix`, ignoring case.
///
/// A final newline is part of the text: `"a\n"` does not end with `"a"`.
#[must_use]
pub fn ends_with_fold(text: &[u16], suffix: &[u16]) -> bool {
    let mut text_index = text.len();
    let mut suffix_index = suffix.len();
    while suffix_index > 0 {
        if text_index == 0 {
            return false;
        }
        let (text_point, text_start) = code_point_before(text, text_index);
        let (suffix_point, suffix_start) = code_point_before(suffix, suffix_index);
        if !fold_equal(text_point, suffix_point) {
            return false;
        }
        text_index = text_start;
        suffix_index = suffix_start;
    }
    true
}

/// Finds the first occurrence of `search` in `text`, ignoring case.
///
/// Returns a UTF-16 code unit index, or `None` when no match starts at or after
/// the clamped `from_index`. An empty search returns the clamped index. The
/// scan uses KMP over code points, so the result stays linear when the search
/// string has a repeated prefix.
///
/// # Errors
///
/// Returns [`IndexError`] when `from_index` is present but is not an integer
/// number. `Infinity` and `-Infinity` are accepted and clamp.
pub fn index_of_fold(
    text: &[u16],
    search: &[u16],
    from_index: Option<f64>,
) -> Result<Option<usize>, IndexError> {
    let from = to_index(from_index, text.len(), 0)?;
    if search.is_empty() {
        return Ok(Some(from));
    }
    let mut offset = code_point_start(text, from);
    if offset >= text.len() {
        return Ok(None);
    }
    let pattern = code_points_of(search);
    let length = pattern.len();
    let prefix = fold_prefix(&pattern);
    let mut starts = vec![0usize; length];
    let mut matched = 0;
    let mut slot = 0;
    while offset < text.len() {
        let (point, width) = code_point_at(text, offset);
        starts[slot] = offset;
        slot += 1;
        if slot == length {
            slot = 0;
        }
        while matched > 0 && !fold_equal(pattern[matched], point) {
            matched = prefix[matched - 1];
        }
        if fold_equal(pattern[matched], point) {
            matched += 1;
        }
        // The ring holds the last `length` start offsets. After the increment
        // above, `starts[slot]` is the oldest of them, which is the start of
        // the match that has just completed.
        if matched == length {
            return Ok(Some(starts[slot]));
        }
        offset += width;
    }
    Ok(None)
}

/// Finds the last occurrence of `search` in `text`, ignoring case.
///
/// Overlapping occurrences are found, like `String.prototype.lastIndexOf`:
/// `last_index_of_fold("aaa", "aa", None)` is `Some(1)`. A match must start at
/// or before the clamped `from_index`; the default is the end of the text.
///
/// The scan is the same KMP walk as [`index_of_fold`]. It stops as soon as no
/// pending or future match can begin at or before the limit, so a small limit
/// does not read the rest of a long text.
///
/// # Errors
///
/// Returns [`IndexError`] when `from_index` is present but is not an integer
/// number. `Infinity` and `-Infinity` are accepted and clamp.
pub fn last_index_of_fold(
    text: &[u16],
    search: &[u16],
    from_index: Option<f64>,
) -> Result<Option<usize>, IndexError> {
    let limit = to_index(from_index, text.len(), text.len())?;
    if search.is_empty() {
        return Ok(Some(limit));
    }
    let pattern = code_points_of(search);
    let length = pattern.len();
    let prefix = fold_prefix(&pattern);
    let mut starts = vec![0usize; length];
    let mut last = None;
    let mut matched = 0;
    let mut offset = 0;
    let mut slot = 0;
    while offset < text.len() {
        let (point, width) = code_point_at(text, offset);
        starts[slot] = offset;
        slot += 1;
        if slot == length {
            slot = 0;
        }
        while matched > 0 && !fold_equal(pattern[matched], point) {
            matched = prefix[matched - 1];
        }
        if fold_equal(pattern[matched], point) {
            matched += 1;
        }
        if matched == length {
            let start = starts[slot];
            if start > limit {
                break;
            }
            last = Some(start);
            matched = prefix[matched - 1];
        }
        // The earliest match that can still complete begins at the start of
        // the surviving partial match, or at the next code point when no
        // partial match survives. Start offsets only increase with the scan,
        // so once that candidate is past the limit no match can be recorded
        // and the rest of the text need not be read.
        if matched > 0 {
            let candidate = (slot + length - matched) % length;
            if starts[candidate] > limit {
                break;
            }
        } else if offset + width > limit {
            break;
        }
        offset += width;
    }
    Ok(last)
}

/// The set of code points a trim operation removes, or JavaScript whitespace.
enum TrimSet<'a> {
    /// The set `String.prototype.trim` uses when no `chars` argument is given.
    Whitespace,
    /// The caller-supplied code point set.
    Points(&'a HashSet<u32>),
}

impl TrimSet<'_> {
    fn contains(&self, point: u32) -> bool {
        match self {
            Self::Whitespace => is_js_whitespace(point),
            Self::Points(points) => points.contains(&point),
        }
    }
}

/// True for the ECMAScript `WhiteSpace` and `LineTerminator` code points.
///
/// This includes U+FEFF and excludes U+0085. It is not `neo:chars`' Go-derived
/// `is_space`.
fn is_js_whitespace(point: u32) -> bool {
    match point {
        0x09..=0x0D | 0x20 | 0xA0 | 0x2028 | 0x2029 | 0xFEFF => true,
        _ => char::from_u32(point).is_some_and(|character| {
            get_general_category(character) == GeneralCategory::SpaceSeparator
        }),
    }
}

/// Builds the caller-supplied code point set.
fn code_point_set(text: &[u16]) -> HashSet<u32> {
    let mut points = HashSet::new();
    let mut index = 0;
    while index < text.len() {
        let (point, width) = code_point_at(text, index);
        points.insert(point);
        index += width;
    }
    points
}

/// Returns the first code unit that is not in the set.
fn trim_start_offset(text: &[u16], set: &TrimSet<'_>) -> usize {
    let mut start = 0;
    while start < text.len() {
        let (point, width) = code_point_at(text, start);
        if !set.contains(point) {
            break;
        }
        start += width;
    }
    start
}

/// Returns the end offset after the last code unit that is in the set.
fn trim_end_offset(text: &[u16], set: &TrimSet<'_>, floor: usize) -> usize {
    let mut end = text.len();
    while end > floor {
        let (point, start) = code_point_before(text, end);
        if !set.contains(point) {
            break;
        }
        end = start;
    }
    end
}

/// Removes leading and trailing code points that occur in `chars`.
///
/// With no `chars`, this is exactly `String.prototype.trim`, including
/// JavaScript's whitespace set. With `chars`, the argument is a set of Unicode
/// code points, not a prefix: `trim("xyhelloyx", "xy")` is `"hello"`. An empty
/// `chars` string removes nothing.
#[must_use]
pub fn trim(text: &[u16], chars: Option<&[u16]>) -> Vec<u16> {
    match chars {
        None => {
            let set = TrimSet::Whitespace;
            let start = trim_start_offset(text, &set);
            let end = trim_end_offset(text, &set, start);
            text[start..end].to_vec()
        }
        Some([]) => text.to_vec(),
        Some(chars) => {
            let points = code_point_set(chars);
            let set = TrimSet::Points(&points);
            let start = trim_start_offset(text, &set);
            let end = trim_end_offset(text, &set, start);
            text[start..end].to_vec()
        }
    }
}

/// Removes leading code points that occur in `chars`.
///
/// With no `chars`, this is exactly `String.prototype.trimStart`.
#[must_use]
pub fn trim_start(text: &[u16], chars: Option<&[u16]>) -> Vec<u16> {
    match chars {
        None => {
            let set = TrimSet::Whitespace;
            let start = trim_start_offset(text, &set);
            text[start..].to_vec()
        }
        Some([]) => text.to_vec(),
        Some(chars) => {
            let points = code_point_set(chars);
            let set = TrimSet::Points(&points);
            let start = trim_start_offset(text, &set);
            text[start..].to_vec()
        }
    }
}

/// Removes trailing code points that occur in `chars`.
///
/// With no `chars`, this is exactly `String.prototype.trimEnd`.
#[must_use]
pub fn trim_end(text: &[u16], chars: Option<&[u16]>) -> Vec<u16> {
    match chars {
        None => {
            let set = TrimSet::Whitespace;
            let end = trim_end_offset(text, &set, 0);
            text[..end].to_vec()
        }
        Some([]) => text.to_vec(),
        Some(chars) => {
            let points = code_point_set(chars);
            let set = TrimSet::Points(&points);
            let end = trim_end_offset(text, &set, 0);
            text[..end].to_vec()
        }
    }
}

/// The shared module descriptor.
pub const MODULE: crate::BuiltinModule = crate::BuiltinModule {
    specifier: SPECIFIER,
    exports: EXPORTS,
    backend: crate::Backend::Rust,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn text(units: &[u16]) -> String {
        String::from_utf16(units).expect("test output is well formed")
    }

    #[test]
    fn simple_folding_pins_the_documented_equivalences() {
        assert_eq!(case_fold::simple_fold(0x41), 0x61);
        assert_eq!(case_fold::simple_fold(0x61), 0x61);
        assert_eq!(case_fold::simple_fold(0x212A), 0x6B);
        assert_eq!(case_fold::simple_fold(0x17F), 0x73);
        assert_eq!(case_fold::simple_fold(0x3C2), 0x3C3);
        assert_eq!(case_fold::simple_fold(0x3C3), 0x3C3);
        assert_eq!(case_fold::simple_fold(0x1E9E), 0xDF);
        assert_eq!(case_fold::simple_fold(0xDF), 0xDF);
        assert_eq!(case_fold::simple_fold(0x130), 0x130);
        assert_eq!(case_fold::simple_fold(0x131), 0x131);
        assert_eq!(case_fold::simple_fold(0xB5), 0x3BC);
        assert_eq!(case_fold::simple_fold(0xD800), 0xD800);
        assert_eq!(case_fold::simple_fold(0x1F600), 0x1F600);
    }

    #[test]
    fn simple_folding_is_idempotent_and_covers_the_table() {
        for &(start, end, _) in case_fold::RANGES {
            assert!(start <= end);
            let mut point = start;
            loop {
                let folded = case_fold::simple_fold(point);
                assert_eq!(case_fold::simple_fold(folded), folded, "{point:#X}");
                if point == end {
                    break;
                }
                point += 1;
            }
        }
        let mut previous_end = 0;
        for &(start, end, _) in case_fold::RANGES {
            assert!(start > previous_end, "ranges overlap at {start:#X}");
            previous_end = end;
        }
    }

    #[test]
    fn equal_fold_matches_the_documented_cases() {
        let cases = [
            ("Hello", "HELLO", true),
            ("café", "CAFÉ", true),
            ("K", "\u{212A}", true),
            ("\u{212A}", "k", true),
            ("S", "\u{17F}", true),
            ("\u{3C3}", "\u{3C2}", true),
            ("I", "\u{131}", false),
            ("i", "\u{130}", false),
            ("ss", "\u{DF}", false),
            ("a\n", "A\n", true),
            ("\u{1F600}", "\u{1F600}", true),
            ("", "", true),
            ("", "\0", false),
            ("a.b", "a.b", true),
            ("a.b", "axb", false),
            ("[a-z]", "[a-z]", true),
            ("\\d", "\\d", true),
            ("a*", "aaa", false),
        ];
        for (left, right, expected) in cases {
            assert_eq!(
                equal_fold(&units(left), &units(right)),
                expected,
                "{left:?}"
            );
            assert_eq!(
                equal_fold(&units(right), &units(left)),
                expected,
                "symmetry for {left:?}"
            );
        }

        let lone = vec![u16::from(b'a'), 0xD800];
        let a = vec![u16::from(b'a')];
        assert!(equal_fold(&lone, &lone));
        assert!(!equal_fold(&lone, &a));
        assert!(!equal_fold(&[0xD83D], &units("\u{1F600}")));
    }

    #[test]
    fn prefix_and_suffix_matching_keep_newline_rules() {
        assert!(starts_with_fold(&units("Hello World"), &units("HELLO")));
        assert!(!starts_with_fold(&units("Hello"), &units("World")));
        assert!(starts_with_fold(&units("abc"), &units("")));
        assert!(starts_with_fold(&units("(x)"), &units("(x)")));
        assert!(starts_with_fold(&units("|x"), &units("|")));

        assert!(ends_with_fold(&units("Hello World"), &units("WORLD")));
        assert!(!ends_with_fold(&units("Hello"), &units("Hell")));
        assert!(ends_with_fold(&units("abc"), &units("")));
        assert!(!ends_with_fold(&units("a\n"), &units("a")));
        assert!(ends_with_fold(&units("a\n"), &units("\n")));
        assert!(ends_with_fold(&units("a\n"), &units("a\n")));
        assert!(!ends_with_fold(&units("a\r\n"), &units("a")));
    }

    #[test]
    fn index_arguments_clamp_and_reject_non_integers() {
        assert_eq!(to_index(None, 3, 0), Ok(0));
        assert_eq!(to_index(None, 3, 3), Ok(3));
        assert_eq!(to_index(Some(2.0), 3, 0), Ok(2));
        assert_eq!(to_index(Some(2.9), 3, 0), Err(IndexError::NotInteger));
        assert_eq!(to_index(Some(1.5), 3, 0), Err(IndexError::NotInteger));
        assert_eq!(to_index(Some(f64::NAN), 3, 0), Err(IndexError::NotInteger));
        assert_eq!(to_index(Some(f64::INFINITY), 3, 0), Ok(3));
        assert_eq!(to_index(Some(f64::NEG_INFINITY), 3, 0), Ok(0));
        assert_eq!(to_index(Some(-5.0), 3, 0), Ok(0));
        assert_eq!(to_index(Some(-0.0), 3, 0), Ok(0));
        assert_eq!(to_index(Some(99.0), 3, 0), Ok(3));
    }

    #[test]
    fn indices_are_utf16_and_last_index_finds_overlaps() {
        let cases = [
            ("\u{1F600}a", "A", None, Some(2)),
            ("a\u{1F600}b", "B", None, Some(3)),
            ("abc", "z", None, None),
            ("abc", "", None, Some(0)),
            ("abc", "", Some(2.0), Some(2)),
            ("\u{1F600}", "", Some(1.0), Some(1)),
            ("\u{1F600}a", "a", Some(1.0), Some(2)),
            ("abc", "b", Some(-5.0), Some(1)),
            ("abc", "c", Some(f64::INFINITY), None),
            ("abc", "", Some(f64::INFINITY), Some(3)),
            ("abc", "a", Some(99.0), None),
            ("x.a.y", "a.", None, Some(2)),
        ];
        for (haystack, needle, from, expected) in cases {
            assert_eq!(
                index_of_fold(&units(haystack), &units(needle), from),
                Ok(expected),
                "{haystack:?} {needle:?} {from:?}"
            );
        }

        let last_cases = [
            ("aaa", "aa", None, Some(1)),
            ("aaaa", "aa", None, Some(2)),
            ("aAa", "a", None, Some(2)),
            ("a\u{1F600}b\u{1F600}", "B", None, Some(3)),
            ("abc", "z", None, None),
            ("abc", "", None, Some(3)),
            ("abc", "", Some(1.0), Some(1)),
            ("aaaa", "aa", Some(2.0), Some(2)),
            ("aaaa", "aa", Some(1.0), Some(1)),
            ("aaa", "a", Some(0.0), Some(0)),
            ("aaa", "a", Some(-4.0), Some(0)),
            ("\u{212A}\u{212A}", "k", None, Some(1)),
        ];
        for (haystack, needle, from, expected) in last_cases {
            assert_eq!(
                last_index_of_fold(&units(haystack), &units(needle), from),
                Ok(expected),
                "{haystack:?} {needle:?} {from:?}"
            );
        }

        let overlap = units("aaaa");
        for (limit, expected) in [
            (-1.0, 0),
            (0.0, 0),
            (1.0, 1),
            (2.0, 2),
            (3.0, 2),
            (4.0, 2),
            (5.0, 2),
        ] {
            assert_eq!(
                last_index_of_fold(&overlap, &units("aa"), Some(limit)),
                Ok(Some(expected)),
                "limit {limit}"
            );
        }
    }

    #[test]
    fn trim_removes_a_code_point_set_and_defaults_to_js_whitespace() {
        let trim_cases = [
            ("...hi...", ".", "hi"),
            ("##a##", "#", "a"),
            ("xyhelloyx", "xy", "hello"),
            ("\u{1F600}x\u{1F600}", "\u{1F600}", "x"),
            ("  a  ", "", "  a  "),
            ("....", ".", ""),
            ("abc", ".", "abc"),
            ("", ".", ""),
            ("a.b", ".", "a.b"),
        ];
        for (value, chars, expected) in trim_cases {
            assert_eq!(
                text(&trim(&units(value), Some(&units(chars)))),
                expected,
                "{value:?} {chars:?}"
            );
        }

        let lone_text = vec![0xD800, u16::from(b'x'), 0xD800];
        assert_eq!(trim(&lone_text, Some(&[0xD800])), vec![u16::from(b'x')]);

        assert_eq!(
            text(&trim_start(&units("///a/b"), Some(&units("/")))),
            "a/b"
        );
        assert_eq!(text(&trim_end(&units("a/b///"), Some(&units("/")))), "a/b");
        assert_eq!(
            text(&trim_start(&units("\u{1F600}a"), Some(&units("\u{1F600}")))),
            "a"
        );
        assert_eq!(
            text(&trim_end(&units("a\u{1F600}"), Some(&units("\u{1F600}")))),
            "a"
        );

        let js_whitespace = units("\u{FEFF}\t a \n\u{85}");
        assert_eq!(trim(&js_whitespace, None), units("a \n\u{85}"));
        assert_eq!(trim(&units("\u{FEFF} a \u{FEFF}"), None), units("a"));
        assert_eq!(trim(&units("\u{85}a\u{85}"), None), units("\u{85}a\u{85}"));
        assert_eq!(trim_start(&units("  a  "), None), units("a  "));
        assert_eq!(trim_end(&units("  a  "), None), units("  a"));
    }

    fn reset_reads() {
        CODE_POINT_READS.with(|reads| reads.set(0));
    }

    fn read_count() -> usize {
        CODE_POINT_READS.with(Cell::get)
    }

    #[test]
    fn last_index_reads_only_through_the_limit() {
        // The needle costs two reads per code point (count and fill), and the
        // text costs one read per code point examined. Removing the early
        // termination reads the whole million-code-point text and fails every
        // bound.
        let long = units(&format!("{}needle", "\u{1F600}".repeat(1_000_000)));
        reset_reads();
        assert_eq!(
            last_index_of_fold(&long, &units("zzz"), Some(0.0)),
            Ok(None)
        );
        assert!(
            read_count() <= 16,
            "small-limit no-match read {}",
            read_count()
        );

        reset_reads();
        assert_eq!(
            last_index_of_fold(&long, &units("NEEDLE"), Some(0.0)),
            Ok(None)
        );
        assert!(
            read_count() <= 16,
            "a match after the limit read {}",
            read_count()
        );

        let at_limit = units(&format!("{}z{}", "a".repeat(10_000), "a".repeat(1_000_000)));
        reset_reads();
        assert_eq!(
            last_index_of_fold(&at_limit, &units("z"), Some(10_000.0)),
            Ok(Some(10_000))
        );
        assert!(
            read_count() <= 10_016,
            "a limit of 10000 read {}",
            read_count()
        );

        reset_reads();
        assert_eq!(
            last_index_of_fold(
                &units(&format!("{}z", "a".repeat(1_000_000))),
                &units("q"),
                Some(10_000.0)
            ),
            Ok(None)
        );
        assert!(
            read_count() <= 10_016,
            "a no-match at a limit of 10000 read {}",
            read_count()
        );

        reset_reads();
        assert_eq!(
            last_index_of_fold(
                &units(&format!("ab{}", "x".repeat(1_000_000))),
                &units("abc"),
                Some(0.0)
            ),
            Ok(None)
        );
        assert!(read_count() <= 32, "a pending match read {}", read_count());
    }

    #[test]
    fn folded_search_stays_linear_on_repeated_prefixes() {
        let ascii_prefix = units(&"a".repeat(100_000));
        let mut miss = ascii_prefix.clone();
        miss.push(b'c' as u16);
        let mut needle = ascii_prefix.clone();
        needle.push(b'b' as u16);
        assert_eq!(index_of_fold(&miss, &needle, None), Ok(None));
        assert_eq!(last_index_of_fold(&miss, &needle, None), Ok(None));

        let mut hit = ascii_prefix.clone();
        hit.extend_from_slice(&ascii_prefix);
        hit.push(b'b' as u16);
        assert_eq!(index_of_fold(&hit, &needle, None), Ok(Some(100_000)));
        assert_eq!(last_index_of_fold(&hit, &needle, None), Ok(Some(100_000)));

        let astral = units(&"\u{1F600}".repeat(50_000));
        let mut astral_text = astral.clone();
        astral_text.push(b'x' as u16);
        let mut astral_needle = astral.clone();
        astral_needle.push(b'y' as u16);
        assert_eq!(index_of_fold(&astral_text, &astral_needle, None), Ok(None));
        assert_eq!(
            last_index_of_fold(&astral_text, &astral_needle, None),
            Ok(None)
        );

        let folded_block = units(&"\u{3C3}".repeat(50_000));
        let mut folded_needle = units(&"\u{3A3}".repeat(50_000));
        folded_needle.push(b'y' as u16);
        let mut folded_hit = folded_block.clone();
        folded_hit.extend_from_slice(&folded_needle[..folded_needle.len() - 1]);
        folded_hit.push(b'y' as u16);
        assert_eq!(
            index_of_fold(&folded_hit, &folded_needle, None),
            Ok(Some(50_000))
        );
        assert_eq!(
            last_index_of_fold(&folded_hit, &folded_needle, None),
            Ok(Some(50_000))
        );
    }

    #[test]
    fn folded_search_keeps_unicode_and_overlap_semantics() {
        let index_cases = [
            ("aaaa", "aa", Some(0)),
            ("aaab", "aab", Some(1)),
            ("xabab", "abA", Some(1)),
            ("kK", "K\u{212A}", Some(0)),
            ("\u{3A3}\u{3A3}x\u{3A3}\u{3A3}y", "\u{3A3}\u{3A3}y", Some(3)),
            ("\u{1F600}a\u{1F600}a", "A\u{1F600}", Some(2)),
        ];
        for (haystack, needle, expected) in index_cases {
            assert_eq!(
                index_of_fold(&units(haystack), &units(needle), None),
                Ok(expected),
                "{haystack:?} {needle:?}"
            );
        }
        assert_eq!(
            index_of_fold(&units("\u{1F600}a"), &units("a"), Some(1.0)),
            Ok(Some(2))
        );
        assert_eq!(
            index_of_fold(&units("\u{1F600}"), &[0xDE00], None),
            Ok(None)
        );

        let last_cases = [
            ("aaaa", "aa", Some(2)),
            ("aaa", "aa", Some(1)),
            ("xababA", "abA", Some(3)),
            ("\u{1F600}\u{1F600}", "\u{1F600}", Some(2)),
        ];
        for (haystack, needle, expected) in last_cases {
            assert_eq!(
                last_index_of_fold(&units(haystack), &units(needle), None),
                Ok(expected),
                "{haystack:?} {needle:?}"
            );
        }
        let folded = units(&"\u{3C3}".repeat(1000));
        assert_eq!(
            last_index_of_fold(&folded, &units("\u{3C3}\u{3C3}"), None),
            Ok(Some(998))
        );
    }

    #[test]
    fn last_index_stops_at_a_small_limit() {
        let astral = units(&format!("{}needle", "\u{1F600}".repeat(1_000_000)));
        assert_eq!(
            last_index_of_fold(&astral, &units("zzz"), Some(0.0)),
            Ok(None)
        );
        assert_eq!(
            last_index_of_fold(&astral, &units("NEEDLE"), Some(0.0)),
            Ok(None)
        );

        let trailing = units(&format!("needle{}", "x".repeat(1_000_000)));
        assert_eq!(
            last_index_of_fold(&trailing, &units("NEEDLE"), Some(0.0)),
            Ok(Some(0))
        );
        let at_limit = units(&format!("{}b", "a".repeat(1_000_000)));
        assert_eq!(
            last_index_of_fold(&at_limit, &units("B"), Some(1_000_000.0)),
            Ok(Some(1_000_000))
        );
        assert_eq!(
            last_index_of_fold(&at_limit, &units("B"), Some(999_999.0)),
            Ok(None)
        );

        let pending = units(&format!("ab{}", "x".repeat(1_000_000)));
        assert_eq!(
            last_index_of_fold(&pending, &units("abc"), Some(0.0)),
            Ok(None)
        );
        let mid_pair = units(&format!("\u{1F600}{}", "a".repeat(1_000_000)));
        assert_eq!(
            last_index_of_fold(&mid_pair, &units("a"), Some(1.0)),
            Ok(None)
        );
        assert_eq!(
            last_index_of_fold(&pending, &units("ab"), Some(0.0)),
            Ok(Some(0))
        );

        let folded = units(&format!("{}x", "\u{3C3}".repeat(1_000_000)));
        assert_eq!(
            last_index_of_fold(&folded, &units("\u{3A3}"), Some(0.0)),
            Ok(Some(0))
        );
        let folded_tail = units(&format!("\u{3C3}{}", "\u{3A3}".repeat(1_000_000)));
        assert_eq!(
            last_index_of_fold(&folded_tail, &units("z"), Some(0.0)),
            Ok(None)
        );
    }
}
