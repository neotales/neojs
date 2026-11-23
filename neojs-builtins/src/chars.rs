//! The engine-neutral `neo:chars` implementation.
//!
//! This module is Rust, not ECMAScript. Both runtimes bind these functions
//! directly: the V8 runtime installs them as native V8 function objects, and the
//! QuickJS runtime exports them from a native QuickJS module. There is no
//! JavaScript implementation and no JavaScript module wrapper in this crate.
//!
//! Character classification uses the Unicode General_Category tables from
//! [`unicode_general_category`], fixed at Unicode 16.0. Case mapping uses Rust's
//! `char::to_lowercase` and `char::to_uppercase`; that is the Unicode data shipped
//! with the Rust compiler, which is not guaranteed to be the same release as the
//! category table and can differ from the tables the engines used before this
//! binding. A mapping that expands to more than one code point leaves the input
//! unchanged, because the API returns one code point.
//!
//! Every predicate is total: a non-number, a non-integer, a surrogate, an
//! out-of-range value, or `NaN` is not a Unicode scalar value and returns
//! `false`. `to_lower` and `to_upper` reject such a value; the engine binding
//! maps a non-number to `TypeError` and a non-scalar number to `RangeError`.
//!
//! The module has no host operation, no capability, and no I/O. It is not a
//! sandbox.

use unicode_general_category::{GeneralCategory, get_general_category};

/// The canonical import specifier and registry key.
pub const SPECIFIER: &str = "neo:chars";

/// The named exports, in the module object's property order.
pub const EXPORTS: &[&str] = &[
    "isChar",
    "isAscii",
    "isLatin1",
    "isControl",
    "isDigit",
    "isLetter",
    "isLetterOrDigit",
    "isLower",
    "isUpper",
    "isPunc",
    "isSpace",
    "isSymbol",
    "toLower",
    "toUpper",
];

/// The error message a binding reports for a non-scalar case-mapping input.
///
/// The bindings use the same text so the two engines report the same cause. It
/// is also the implementation marker the artifact scans look for: a build with
/// the `chars` feature cannot contain the Rust binding without this string.
pub const NON_SCALAR_MESSAGE: &str = "value must be a Unicode scalar value";

/// The error message a binding reports for a non-number case-mapping input.
pub const NON_NUMBER_MESSAGE: &str = "value must be a number";

/// A validated Unicode scalar value.
///
/// A scalar is an integer in `0..=0x10FFFF` that is not a UTF-16 surrogate.
/// Rust cannot represent a surrogate in a `char`, so a `Scalar` is exactly the
/// set of values that can become a `char`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Scalar(u32);

impl Scalar {
    /// The largest Unicode scalar value.
    pub const MAX: u32 = 0x10_FFFF;

    /// Returns a scalar for an integer code point, or `None` for a surrogate or
    /// an out-of-range value.
    #[must_use]
    pub fn from_u32(value: u32) -> Option<Self> {
        if value > Self::MAX || (0xD800..=0xDFFF).contains(&value) {
            return None;
        }
        Some(Self(value))
    }

    /// Returns a scalar for a numeric JavaScript value, or `None` for a
    /// non-number, a non-integer, a non-finite value, a surrogate, or an
    /// out-of-range value.
    ///
    /// `-0.0` is the scalar zero. The caller is responsible for the engine-level
    /// `typeof value === "number"` check; this function treats a non-number it
    /// is handed as a non-scalar.
    #[must_use]
    pub fn from_f64(value: f64) -> Option<Self> {
        if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > f64::from(Self::MAX)
        {
            return None;
        }
        // The range checks above make this cast exact and in range.
        Self::from_u32(value as u32)
    }

    /// Returns the code point.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Returns the Unicode `char` for this scalar.
    #[must_use]
    pub fn to_char(self) -> char {
        char::from_u32(self.0).expect("a Scalar always names a Unicode scalar value")
    }
}

/// The general category of a scalar's character.
#[must_use]
fn category(value: Scalar) -> GeneralCategory {
    get_general_category(value.to_char())
}

/// True for a Unicode scalar value.
#[must_use]
pub fn is_char(value: f64) -> bool {
    Scalar::from_f64(value).is_some()
}

/// True for an ASCII scalar value, `0` through `0x7F`.
#[must_use]
pub fn is_ascii(value: f64) -> bool {
    Scalar::from_f64(value).is_some_and(|value| value.get() <= 0x7F)
}

/// True for a Latin-1 scalar value, `0` through `0xFF`.
#[must_use]
pub fn is_latin1(value: f64) -> bool {
    Scalar::from_f64(value).is_some_and(|value| value.get() <= 0xFF)
}

/// True for a control character (Unicode category `Cc`).
#[must_use]
pub fn is_control(value: f64) -> bool {
    Scalar::from_f64(value).is_some_and(|value| category(value) == GeneralCategory::Control)
}

/// True for a decimal digit (Unicode category `Nd`), not other numerals.
#[must_use]
pub fn is_digit(value: f64) -> bool {
    Scalar::from_f64(value).is_some_and(|value| category(value) == GeneralCategory::DecimalNumber)
}

/// True for a letter (Unicode category `L`).
#[must_use]
pub fn is_letter(value: f64) -> bool {
    Scalar::from_f64(value).is_some_and(|value| {
        matches!(
            category(value),
            GeneralCategory::UppercaseLetter
                | GeneralCategory::LowercaseLetter
                | GeneralCategory::TitlecaseLetter
                | GeneralCategory::ModifierLetter
                | GeneralCategory::OtherLetter
        )
    })
}

/// True for a letter or a decimal digit.
#[must_use]
pub fn is_letter_or_digit(value: f64) -> bool {
    is_letter(value) || is_digit(value)
}

/// True for a lowercase letter (Unicode category `Ll`).
#[must_use]
pub fn is_lower(value: f64) -> bool {
    Scalar::from_f64(value).is_some_and(|value| category(value) == GeneralCategory::LowercaseLetter)
}

/// True for an uppercase letter (Unicode category `Lu`).
#[must_use]
pub fn is_upper(value: f64) -> bool {
    Scalar::from_f64(value).is_some_and(|value| category(value) == GeneralCategory::UppercaseLetter)
}

/// True for punctuation (Unicode category `P`).
#[must_use]
pub fn is_punc(value: f64) -> bool {
    Scalar::from_f64(value).is_some_and(|value| {
        matches!(
            category(value),
            GeneralCategory::ConnectorPunctuation
                | GeneralCategory::DashPunctuation
                | GeneralCategory::OpenPunctuation
                | GeneralCategory::ClosePunctuation
                | GeneralCategory::InitialPunctuation
                | GeneralCategory::FinalPunctuation
                | GeneralCategory::OtherPunctuation
        )
    })
}

/// True for whitespace: ASCII tab, line feed, vertical tab, form feed,
/// carriage return, and space, plus U+0085 and categories `Zs`, `Zl`, and `Zp`.
///
/// This is Go's `unicode.IsSpace` contract, not JavaScript `\s`: U+FEFF and
/// U+200B are not whitespace.
#[must_use]
pub fn is_space(value: f64) -> bool {
    Scalar::from_f64(value).is_some_and(|value| match value.get() {
        0x09..=0x0D | 0x20 | 0x85 => true,
        _ => matches!(
            category(value),
            GeneralCategory::SpaceSeparator
                | GeneralCategory::LineSeparator
                | GeneralCategory::ParagraphSeparator
        ),
    })
}

/// True for a symbol (Unicode category `S`), including emoji.
#[must_use]
pub fn is_symbol(value: f64) -> bool {
    Scalar::from_f64(value).is_some_and(|value| {
        matches!(
            category(value),
            GeneralCategory::MathSymbol
                | GeneralCategory::CurrencySymbol
                | GeneralCategory::ModifierSymbol
                | GeneralCategory::OtherSymbol
        )
    })
}

/// Returns the one-code-point lowercase mapping, or the input when the full
/// mapping expands to more than one code point.
#[must_use]
pub fn to_lower(value: Scalar) -> u32 {
    one_code_point(value, value.to_char().to_lowercase())
}

/// Returns the one-code-point uppercase mapping, or the input when the full
/// mapping expands to more than one code point.
#[must_use]
pub fn to_upper(value: Scalar) -> u32 {
    one_code_point(value, value.to_char().to_uppercase())
}

/// Returns the single code point of an iterator, or the original when the
/// iterator is empty or has more than one item.
fn one_code_point(value: Scalar, mut mapped: impl Iterator<Item = char>) -> u32 {
    let Some(first) = mapped.next() else {
        return value.get();
    };
    if mapped.next().is_some() {
        value.get()
    } else {
        first as u32
    }
}

/// Validates a numeric case-mapping argument and returns its lowercase mapping.
///
/// The engine binding checks `typeof value === "number"` first. A number that
/// is not a Unicode scalar value returns [`None`]; the binding maps that to
/// `RangeError`.
#[must_use]
pub fn to_lower_number(value: f64) -> Option<u32> {
    Scalar::from_f64(value).map(to_lower)
}

/// Validates a numeric case-mapping argument and returns its uppercase mapping.
///
/// See [`to_lower_number`].
#[must_use]
pub fn to_upper_number(value: f64) -> Option<u32> {
    Scalar::from_f64(value).map(to_upper)
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

    fn scalar(value: u32) -> Scalar {
        Scalar::from_u32(value).expect("scalar")
    }

    #[test]
    fn scalars_exclude_surrogates_and_out_of_range_values() {
        assert_eq!(Scalar::from_u32(0), Some(scalar(0)));
        assert_eq!(Scalar::from_u32(0x10_FFFF), Some(scalar(0x10_FFFF)));
        assert_eq!(Scalar::from_u32(0xD7FF), Some(scalar(0xD7FF)));
        assert_eq!(Scalar::from_u32(0xE000), Some(scalar(0xE000)));
        assert_eq!(Scalar::from_u32(0xD800), None);
        assert_eq!(Scalar::from_u32(0xDFFF), None);
        assert_eq!(Scalar::from_u32(0x11_0000), None);
    }

    #[test]
    fn numeric_scalars_reject_fractions_non_finite_and_surrogates() {
        assert_eq!(Scalar::from_f64(0.0), Some(scalar(0)));
        assert_eq!(Scalar::from_f64(-0.0), Some(scalar(0)));
        assert_eq!(Scalar::from_f64(65.0), Some(scalar(65)));
        assert_eq!(Scalar::from_f64(0x10_FFFF as f64), Some(scalar(0x10_FFFF)));
        for invalid in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.0,
            1.5,
            0x11_0000 as f64,
            0xD800 as f64,
            0xDFFF as f64,
        ] {
            assert_eq!(Scalar::from_f64(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn predicates_are_total() {
        for invalid in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.0,
            1.5,
            0x11_0000 as f64,
            0xD800 as f64,
            0xDFFF as f64,
        ] {
            assert!(!is_char(invalid));
            assert!(!is_ascii(invalid));
            assert!(!is_latin1(invalid));
            assert!(!is_control(invalid));
            assert!(!is_digit(invalid));
            assert!(!is_letter(invalid));
            assert!(!is_letter_or_digit(invalid));
            assert!(!is_lower(invalid));
            assert!(!is_upper(invalid));
            assert!(!is_punc(invalid));
            assert!(!is_space(invalid));
            assert!(!is_symbol(invalid));
        }
    }

    #[test]
    fn classification_matches_the_documented_categories() {
        assert!(is_char(0x10_FFFF as f64));
        assert!(!is_char(0xD800 as f64));
        assert!(is_ascii(0x7F as f64));
        assert!(!is_ascii(0x80 as f64));
        assert!(is_latin1(0xFF as f64));
        assert!(!is_latin1(0x100 as f64));

        assert!(is_control(0x00 as f64));
        assert!(is_control(0x0A as f64));
        assert!(is_control(0x7F as f64));
        assert!(is_control(0x85 as f64));
        assert!(is_control(0x9F as f64));
        assert!(!is_control(0xA0 as f64));

        assert!(is_digit(0x30 as f64));
        assert!(is_digit(0x0663 as f64));
        assert!(!is_digit(0x2168 as f64));

        assert!(is_letter(0x41 as f64));
        assert!(is_letter(0x03BB as f64));
        assert!(is_letter(0x0416 as f64));
        assert!(!is_letter(0x30 as f64));
        assert!(is_letter_or_digit(0x30 as f64));
        assert!(is_letter_or_digit(0x41 as f64));
        assert!(!is_letter_or_digit(0x5F as f64));

        assert!(is_lower(0x61 as f64));
        assert!(is_lower(0x03B1 as f64));
        assert!(!is_lower(0x41 as f64));
        assert!(is_upper(0x41 as f64));
        assert!(is_upper(0x0391 as f64));
        assert!(!is_upper(0x61 as f64));

        assert!(is_punc(0x21 as f64));
        assert!(is_punc(0x2C as f64));
        assert!(!is_punc(0x24 as f64));

        assert!(is_space(0x20 as f64));
        assert!(is_space(0x09 as f64));
        assert!(is_space(0x85 as f64));
        assert!(is_space(0xA0 as f64));
        assert!(is_space(0x1680 as f64));
        assert!(is_space(0x3000 as f64));
        assert!(!is_space(0x200B as f64));
        assert!(!is_space(0xFEFF as f64));

        assert!(is_symbol(0x2B as f64));
        assert!(is_symbol(0x20AC as f64));
        assert!(is_symbol(0x1F600 as f64));
        assert!(!is_symbol(0x41 as f64));
    }

    #[test]
    fn one_code_point_case_mapping_keeps_expanding_mappings() {
        assert_eq!(to_lower(scalar(0x00)), 0x00);
        assert_eq!(to_upper(scalar(0x00)), 0x00);
        assert_eq!(to_lower(scalar(0x41)), 0x61);
        assert_eq!(to_lower(scalar(0x61)), 0x61);
        assert_eq!(to_upper(scalar(0x61)), 0x41);
        assert_eq!(to_upper(scalar(0x41)), 0x41);
        assert_eq!(to_lower(scalar(0x0391)), 0x03B1);
        assert_eq!(to_upper(scalar(0x03B1)), 0x0391);
        assert_eq!(to_lower(scalar(0x1E9E)), 0x00DF);
        assert_eq!(to_lower(scalar(0x0130)), 0x0130);
        assert_eq!(to_upper(scalar(0x00DF)), 0x00DF);
    }

    #[test]
    fn unicode_16_category_boundary_pins_the_intended_contract() {
        use unicode_general_category::{UNICODE_VERSION, get_general_category};

        assert_eq!(UNICODE_VERSION, (16, 0, 0));
        assert_eq!(
            get_general_category(char::from_u32(0x1C89).expect("scalar")),
            GeneralCategory::UppercaseLetter
        );
        assert_eq!(
            get_general_category(char::from_u32(0x1C8A).expect("scalar")),
            GeneralCategory::LowercaseLetter
        );
        assert_eq!(
            get_general_category(char::from_u32(0x0378).expect("scalar")),
            GeneralCategory::Unassigned
        );
        assert!(is_letter(0x1C89 as f64));
        assert!(is_upper(0x1C89 as f64));
        assert!(is_lower(0x1C8A as f64));
        assert!(!is_letter(0x0378 as f64));
    }

    #[test]
    fn case_mapping_documents_the_compiler_table_dependency() {
        // These code points are classified by the Unicode 16.0 category table.
        // The mappings here come from the Rust compiler's case tables, which are
        // a separate release stream. If a compiler upgrade changes them, update
        // this test and the documented limitation rather than the category
        // table.
        assert_eq!(to_lower(scalar(0x1C89)), 0x1C8A);
        assert_eq!(to_upper(scalar(0x1C8A)), 0x1C89);
        // An expanding mapping still returns the input unchanged.
        assert_eq!(to_upper(scalar(0x00DF)), 0x00DF);
    }

    #[test]
    fn numeric_case_mapping_returns_none_for_non_scalars() {
        assert_eq!(to_lower_number(0x41 as f64), Some(0x61));
        assert_eq!(to_upper_number(0x61 as f64), Some(0x41));
        for invalid in [
            f64::NAN,
            f64::INFINITY,
            -1.0,
            1.5,
            0x11_0000 as f64,
            0xD800 as f64,
        ] {
            assert_eq!(to_lower_number(invalid), None, "{invalid}");
            assert_eq!(to_upper_number(invalid), None, "{invalid}");
        }
    }
}
