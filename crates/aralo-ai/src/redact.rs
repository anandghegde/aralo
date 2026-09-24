//! Redaction: the third stage, and an empty slot until v1.
//!
//! In v1 on-device detectors mask card numbers, government IDs, keys,
//! passwords and email addresses with stable tokens, and the tokens are put
//! back in the model's answer (PRD P5). The stage exists now so that it is a
//! new stage when it arrives, not a new path (ADR-0007).

use crate::context::ContextItem;

/// Returns the context as it will be sent. Today that is unchanged.
pub fn redact(items: Vec<ContextItem>) -> Vec<ContextItem> {
    items
}
