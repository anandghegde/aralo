/// How an adaptive-case snippet is re-cased to follow what the user typed
/// (PRD E3): "ty" gives the text as defined, "Ty" capitalises it, "TY" shouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CaseTransform {
    #[default]
    AsDefined,
    /// Upper-case the first letter, leave the rest alone.
    Title,
    Upper,
}

impl CaseTransform {
    pub fn apply(self, text: &str) -> String {
        match self {
            CaseTransform::AsDefined => text.to_owned(),
            CaseTransform::Upper => text.to_uppercase(),
            CaseTransform::Title => {
                let mut out = String::with_capacity(text.len());
                let mut chars = text.chars();
                for c in chars.by_ref() {
                    if c.is_alphabetic() {
                        out.extend(c.to_uppercase());
                        break;
                    }
                    out.push(c);
                }
                out.push_str(chars.as_str());
                out
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ty_goldens() {
        assert_eq!(CaseTransform::AsDefined.apply("thank you"), "thank you");
        assert_eq!(CaseTransform::Title.apply("thank you"), "Thank you");
        assert_eq!(CaseTransform::Upper.apply("thank you"), "THANK YOU");
    }

    #[test]
    fn title_skips_leading_punctuation_and_keeps_existing_capitals() {
        assert_eq!(CaseTransform::Title.apply("(see below)"), "(See below)");
        assert_eq!(CaseTransform::Title.apply("iPhone order"), "IPhone order");
        assert_eq!(CaseTransform::Title.apply("Already"), "Already");
        assert_eq!(CaseTransform::Title.apply(""), "");
        assert_eq!(CaseTransform::Title.apply("123"), "123");
    }

    #[test]
    fn handles_letters_whose_upper_case_is_longer() {
        assert_eq!(CaseTransform::Title.apply("ßtraße"), "SStraße");
        assert_eq!(CaseTransform::Upper.apply("straße"), "STRASSE");
    }
}
