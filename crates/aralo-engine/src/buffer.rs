//! The keystroke buffer: a fixed ring of characters that never leaves memory.

use zeroize::Zeroize;

/// How many typed characters the engine remembers (PRD P1).
pub const CAPACITY: usize = 64;

/// A fixed-size ring of the most recently typed characters.
///
/// Every slot that stops holding a live character is overwritten with `'\0'`
/// through `zeroize`, so the optimiser cannot drop the write.
pub(crate) struct RingBuffer {
    chars: [char; CAPACITY],
    /// Index of the slot the next character is written to.
    head: usize,
    len: usize,
}

impl RingBuffer {
    pub(crate) const fn new() -> Self {
        Self {
            chars: ['\0'; CAPACITY],
            head: 0,
            len: 0,
        }
    }

    pub(crate) fn push(&mut self, c: char) {
        self.chars[self.head] = c;
        self.head = (self.head + 1) % CAPACITY;
        if self.len < CAPACITY {
            self.len += 1;
        }
    }

    /// Drops the most recent character, as a Backspace does.
    pub(crate) fn pop(&mut self) {
        if self.len == 0 {
            return;
        }
        self.head = (self.head + CAPACITY - 1) % CAPACITY;
        self.chars[self.head].zeroize();
        self.len -= 1;
    }

    pub(crate) fn clear(&mut self) {
        self.chars.zeroize();
        self.head = 0;
        self.len = 0;
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// The character `back` positions before the most recent one; `0` is the
    /// most recent. `None` means the buffer does not reach that far.
    pub(crate) fn get_back(&self, back: usize) -> Option<char> {
        if back >= self.len {
            return None;
        }
        Some(self.chars[(self.head + CAPACITY - 1 - back) % CAPACITY])
    }

    /// Characters from the most recent backwards.
    pub(crate) fn iter_back(&self) -> impl Iterator<Item = char> + '_ {
        (0..self.len).filter_map(|back| self.get_back(back))
    }

    /// True when every slot of the backing storage is `'\0'`.
    pub(crate) fn is_zeroed(&self) -> bool {
        self.len == 0 && self.chars.iter().all(|&c| c == '\0')
    }
}

impl Drop for RingBuffer {
    fn drop(&mut self) {
        self.clear();
    }
}

// Deliberately no `Debug` that prints contents: the buffer must never reach a log.
impl core::fmt::Debug for RingBuffer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RingBuffer")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn tail(buffer: &RingBuffer) -> String {
        let mut chars: alloc::vec::Vec<char> = buffer.iter_back().collect();
        chars.reverse();
        chars.into_iter().collect()
    }

    #[test]
    fn keeps_only_the_last_capacity_chars() {
        let mut buffer = RingBuffer::new();
        for i in 0..(CAPACITY + 10) {
            buffer.push(char::from(b'a' + (i % 26) as u8));
        }
        assert_eq!(buffer.len(), CAPACITY);
        let expected: String = (10..CAPACITY + 10)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        assert_eq!(tail(&buffer), expected);
    }

    #[test]
    fn pop_removes_and_zeroes_the_newest_char() {
        let mut buffer = RingBuffer::new();
        buffer.push('a');
        buffer.push('b');
        buffer.pop();
        assert_eq!(tail(&buffer), "a");
        buffer.pop();
        buffer.pop(); // popping an empty buffer is a no-op
        assert!(buffer.is_zeroed());
    }

    #[test]
    fn clear_zeroes_every_slot_even_after_wrapping() {
        let mut buffer = RingBuffer::new();
        for _ in 0..(CAPACITY * 2 + 3) {
            buffer.push('x');
        }
        assert!(!buffer.is_zeroed());
        buffer.clear();
        assert!(buffer.is_zeroed());
    }

    #[test]
    fn debug_output_never_contains_typed_text() {
        let mut buffer = RingBuffer::new();
        for c in "hunter2".chars() {
            buffer.push(c);
        }
        let printed = alloc::format!("{buffer:?}");
        assert!(!printed.contains("hunter2"));
        assert_eq!(printed, "RingBuffer { len: 7, .. }");
    }
}
