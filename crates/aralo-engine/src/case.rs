//! Case folding for the trie and detection of the typed case pattern (PRD E3).

/// How the user cased an abbreviation whose snippet uses adaptive case.
///
/// The engine only detects the pattern. The template evaluator applies it to
/// the expansion, so `ty` gives "thank you", `Ty` gives "Thank you" and `TY`
/// gives "THANK YOU".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CasePattern {
    /// Typed exactly as defined, or the snippet is not adaptive: no transform.
    AsDefined,
    /// First cased letter typed in upper case: capitalise the expansion.
    Title,
    /// Every cased letter typed in upper case (at least two): upper-case the expansion.
    Upper,
}

/// Folds one character for case-insensitive trie lookup.
///
/// The mapping is always one character to one character so that a match of
/// depth `n` in the trie is exactly `n` typed characters. Characters whose
/// lower-case form is more than one character (such as `İ`) fold to themselves.
pub(crate) fn fold(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

/// Detects the case pattern of `typed` against the abbreviation `defined`.
///
/// Both slices are in typing order and already known to be equal after folding.
pub(crate) fn detect_pattern(typed: &[char], defined: &[char]) -> CasePattern {
    if typed == defined {
        return CasePattern::AsDefined;
    }
    let mut cased = typed
        .iter()
        .filter(|c| c.is_uppercase() || c.is_lowercase());
    let Some(first) = cased.next() else {
        return CasePattern::AsDefined;
    };
    if !first.is_uppercase() {
        return CasePattern::AsDefined;
    }
    let mut rest = cased.peekable();
    if rest.peek().is_some() && rest.all(|c| c.is_uppercase()) {
        CasePattern::Upper
    } else {
        CasePattern::Title
    }
}

/// Word characters for the whole-word rule (PRD E2).
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn pattern(typed: &str, defined: &str) -> CasePattern {
        let typed: Vec<char> = typed.chars().collect();
        let defined: Vec<char> = defined.chars().collect();
        detect_pattern(&typed, &defined)
    }

    #[test]
    fn ty_goldens() {
        assert_eq!(pattern("ty", "ty"), CasePattern::AsDefined);
        assert_eq!(pattern("Ty", "ty"), CasePattern::Title);
        assert_eq!(pattern("TY", "ty"), CasePattern::Upper);
    }

    #[test]
    fn punctuation_and_digits_are_ignored() {
        assert_eq!(pattern(";Rf", ";rf"), CasePattern::Title);
        assert_eq!(pattern(";RF", ";rf"), CasePattern::Upper);
        assert_eq!(pattern(";r2", ";r2"), CasePattern::AsDefined);
    }

    #[test]
    fn a_single_letter_cannot_be_upper() {
        assert_eq!(pattern("A", "a"), CasePattern::Title);
        assert_eq!(pattern(";A", ";a"), CasePattern::Title);
    }

    #[test]
    fn mixed_case_that_starts_lower_is_left_alone() {
        assert_eq!(pattern("tY", "ty"), CasePattern::AsDefined);
    }

    #[test]
    fn typed_as_defined_wins_even_when_defined_has_capitals() {
        assert_eq!(pattern("TY", "TY"), CasePattern::AsDefined);
        assert_eq!(pattern("Ty", "Ty"), CasePattern::AsDefined);
    }

    #[test]
    fn fold_is_one_to_one() {
        assert_eq!(fold('A'), 'a');
        assert_eq!(fold('ß'), 'ß');
        assert_eq!(fold('İ'), 'İ'); // lower-cases to two chars, so it folds to itself
        assert_eq!(fold('Ä'), 'ä');
        assert_eq!(fold(';'), ';');
    }
}
