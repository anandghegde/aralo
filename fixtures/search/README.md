# The search library

A small library that looks like a real one, for testing search by meaning
(PRD A4, task 4.8). Its snippets are what a support person, a salesperson and
anyone with an inbox keep, and several sit close to each other on purpose: the
refund reply has an invoice, a cancellation and a payment in its neighbourhood.

The test that holds search to its promise asks for things in words the
snippets do not use. "money back" has to find `Support/refund.md`, which never
says "money" or "back", with the model loaded from disk and nothing sent
anywhere (`crates/aralo-core/tests/meaning.rs`).
