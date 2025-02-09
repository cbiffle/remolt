//! Tokenizer is a type used for parsing a `&str` into slices in a way not easily
//! supported by the `Peekable<Chars>` iterator.  The basic procedure is as follows:
//!
//! * Use `next` and `peek` to query the iterator in the usual way.
//! * Detect the beginning of a token and get its index from `mark`.
//! * Skip just past the end of the token using `next`, `skip`, etc.
//! * Use `token` to retrieve a slice from the mark to the index.

use core::iter::Peekable;
use core::str::Chars;

/// The Tokenizer type.  See the module-level documentation.
#[derive(Clone, Debug)]
pub struct Tokenizer<'a> {
    // The string being parsed.
    input: &'a str,

    // The starting index of the next character.
    index: usize,

    // The iterator used to extract characters from the input
    chars: Peekable<Chars<'a>>,
}

impl<'a> Tokenizer<'a> {
    /// Creates a new tokenizer for the given input.
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            index: 0,
            chars: input.chars().peekable(),
        }
    }

    /// Returns the entire input.
    #[allow(dead_code)]
    pub fn input(&self) -> &str {
        self.input
    }

    // Returns the remainder of the input starting at the index.
    pub fn as_str(&self) -> &str {
        &self.input[self.index..]
    }

    // Returns the current index as a mark, for later use.
    pub fn mark(&self) -> usize {
        self.index
    }

    // Returns the remainder of the input starting at the given mark.
    #[allow(dead_code)]
    pub fn tail(&self, mark: usize) -> &str {
        &self.input[mark..]
    }

    /// Returns the next character and updates the index.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<char> {
        let ch = self.chars.next();

        if let Some(c) = ch {
            self.index += c.len_utf8();
        }

        ch
    }

    /// Returns the next character without updating the index.
    pub fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    /// Get the token between the mark and the index.  Returns "" if we're at the
    /// end or mark == index.
    pub fn token(&self, mark: usize) -> &str {
        assert!(mark <= self.index, "mark follows index");
        &self.input[mark..self.index]
    }

    /// Get the token between the mark and the index.  Returns "" if
    /// mark == index.
    #[allow(dead_code)]
    pub fn token2(&self, mark: usize, index: usize) -> &str {
        assert!(mark <= index, "mark follows index");
        &self.input[mark..index]
    }

    /// Resets the index to the given mark.  For internal use only.
    fn reset_to(&mut self, mark: usize) {
        self.index = mark;
        self.chars = self.input[self.index..].chars().peekable();
    }

    /// Is the next character the given character?  Does not update the index.
    pub fn is(&mut self, ch: char) -> bool {
        if let Some(c) = self.chars.peek() {
            *c == ch
        } else {
            false
        }
    }

    /// Is the predicate true for the next character? Does not update the index.
    pub fn has<P>(&mut self, predicate: P) -> bool
    where
        P: Fn(char) -> bool,
    {
        if let Some(ch) = self.chars.peek().copied() {
            predicate(ch)
        } else {
            false
        }
    }

    /// Is there anything left in the input?
    pub fn at_end(&mut self) -> bool {
        // &mut is needed because peek() can mutate the iterator
        self.chars.peek().is_none()
    }

    /// Skip over the next character, updating the index.  This is equivalent to
    /// `next`, but communicates better.
    pub fn skip(&mut self) {
        self.next();
    }

    /// Skip over the given character, updating the index.  This is equivalent to
    /// `next`, but communicates better.  Panics if the character is not matched.
    pub fn skip_char(&mut self, ch: char) {
        assert!(self.is(ch));
        self.next();
    }

    /// Skips the given number of characters, updating the index.
    /// It is not an error if the iterator doesn't contain that many.
    pub fn skip_over(&mut self, num_chars: usize) {
        for _ in 0..num_chars {
            self.next();
        }
    }

    /// Skips over characters while the predicate is true.  Updates the index.
    pub fn skip_while<P>(&mut self, predicate: P)
    where
        P: Fn(char) -> bool,
    {
        while let Some(ch) = self.chars.peek().copied() {
            if predicate(ch) {
                self.next();
            } else {
                break;
            }
        }
    }

    /// Parses a backslash-escape and returns its value. If the escape is valid,
    /// the value will be the substituted character.  If the escape is not valid,
    /// it will be the single character following the backslash.  Either way, the
    /// the index will point at what's next.  If there's nothing following the backslash,
    /// return the backslash.
    pub fn backslash_subst(&mut self) -> char {
        // FIRST, skip the backslash.
        self.skip_char('\\');

        let start = self.mark(); // Mark the character following the backslash.

        // NEXT, get the next character.
        if let Some(c) = self.next() {
            // FIRST, match the character.
            match c {
                // Single character escapes
                'a' => '\x07', // Audible Alarm
                'b' => '\x08', // Backspace
                'f' => '\x0c', // Form Feed
                'n' => '\n',   // New Line
                'r' => '\r',   // Carriage Return
                't' => '\t',   // Tab
                'v' => '\x0b', // Vertical Tab

                // 1 to 3 octal digits
                '0'..='7' => {
                    // Note: only works because these digits are single bytes.
                    // TODO: count instead.
                    while self.has(|ch| ch.is_digit(8)) && self.index - start < 3 {
                        self.next();
                    }

                    let octal = &self.input[start..self.index];

                    let val = u8::from_str_radix(octal, 8).map_err(|_| ()).unwrap();
                    val as char
                }

                // \xhh, \uhhhh, \Uhhhhhhhh
                'x' | 'u' | 'U' => {
                    let mark = self.mark();

                    let max = match c {
                        'x' => 2,
                        'u' => 4,
                        'U' => 8,
                        _ => unreachable!(),
                    };

                    // Note: only works because these digits are single bytes.
                    // TODO: count instead.
                    while self.has(|ch| ch.is_ascii_hexdigit()) && self.index - mark < max {
                        self.next();
                    }

                    if self.index == mark {
                        return c;
                    }

                    let hex = &self.input[mark..self.index];

                    let val = u32::from_str_radix(hex, 16).map_err(|_| ()).unwrap();
                    if let Some(ch) = char::from_u32(val) {
                        ch
                    } else {
                        self.reset_to(mark);
                        c
                    }
                }

                // Arbitrary single characters
                _ => c,
            }
        } else {
            // Return the backslash; no escape, since no following character.
            '\\'
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::util;

    use super::*;

    #[test]
    fn test_basics() {
        // Create the iterator
        let mut ptr = Tokenizer::new("abc");

        // Initial state
        assert_eq!(ptr.input(), "abc");
        assert_eq!(ptr.as_str(), "abc");
        assert_eq!(ptr.peek(), Some('a'));
    }

    #[test]
    fn test_next() {
        // Create the iterator
        let mut ptr = Tokenizer::new("abc");

        assert_eq!(ptr.next(), Some('a'));
        assert_eq!(ptr.as_str(), "bc");

        assert_eq!(ptr.next(), Some('b'));
        assert_eq!(ptr.as_str(), "c");

        assert_eq!(ptr.next(), Some('c'));
        assert_eq!(ptr.as_str(), "");

        assert_eq!(ptr.next(), None);
    }

    #[test]
    fn test_token() {
        // Create the iterator
        let mut ptr = Tokenizer::new("abcdef");

        ptr.next();
        ptr.next();
        assert_eq!(ptr.as_str(), "cdef");

        let start = ptr.mark();
        ptr.next();
        ptr.next();

        assert_eq!(ptr.token(start), "cd");
        assert_eq!(ptr.as_str(), "ef");

        let ptr = Tokenizer::new("abc");
        let start = ptr.mark();
        assert_eq!(ptr.token(start), "");
    }

    #[test]
    fn test_peek() {
        let mut ptr = Tokenizer::new("abcdef");

        assert_eq!(ptr.peek(), Some('a'));
        assert_eq!(ptr.as_str(), "abcdef");

        ptr.next();
        ptr.next();

        assert_eq!(ptr.peek(), Some('c'));
        assert_eq!(ptr.as_str(), "cdef");
    }

    #[test]
    fn test_reset_to() {
        let mut ptr = Tokenizer::new("abcdef");

        ptr.next();
        ptr.next();
        ptr.reset_to(0);

        assert_eq!(ptr.as_str(), "abcdef");
        assert_eq!(ptr.peek(), Some('a'));

        ptr.next();
        ptr.next();
        let start = ptr.mark();
        ptr.next();
        ptr.next();
        ptr.reset_to(start);

        assert_eq!(ptr.as_str(), "cdef");
        assert_eq!(ptr.peek(), Some('c'));
    }

    #[test]
    fn test_is() {
        let mut ptr = Tokenizer::new("a");
        assert!(ptr.is('a'));
        assert!(!ptr.is('b'));
        ptr.next();
        assert!(!ptr.is('a'));
    }

    #[test]
    fn test_has() {
        let mut ptr = Tokenizer::new("a1");
        assert!(ptr.has(util::is_alphabetic));
        ptr.skip();
        assert!(!ptr.has(util::is_alphabetic));
        ptr.skip();
        assert!(!ptr.has(util::is_alphabetic));
    }

    #[test]
    fn test_skip() {
        let mut ptr = Tokenizer::new("abc");

        assert_eq!(ptr.peek(), Some('a'));
        assert_eq!(ptr.as_str(), "abc");

        ptr.skip();
        assert_eq!(ptr.peek(), Some('b'));
        assert_eq!(ptr.as_str(), "bc");

        ptr.skip();
        assert_eq!(ptr.peek(), Some('c'));
        assert_eq!(ptr.as_str(), "c");

        ptr.skip();
        assert_eq!(ptr.peek(), None);
        assert_eq!(ptr.as_str(), "");
    }

    #[test]
    fn test_skip_over() {
        let mut ptr = Tokenizer::new("abc");
        ptr.skip_over(2);
        assert_eq!(ptr.peek(), Some('c'));
        assert_eq!(ptr.as_str(), "c");

        let mut ptr = Tokenizer::new("abc");
        ptr.skip_over(3);
        assert_eq!(ptr.peek(), None);
        assert_eq!(ptr.as_str(), "");

        let mut ptr = Tokenizer::new("abc");
        ptr.skip_over(6);
        assert_eq!(ptr.peek(), None);
        assert_eq!(ptr.as_str(), "");
    }

    #[test]
    fn test_skip_while() {
        let mut ptr = Tokenizer::new("aaabc");
        ptr.skip_while(|ch| ch == 'a');
        assert_eq!(ptr.peek(), Some('b'));
        assert_eq!(ptr.as_str(), "bc");

        let mut ptr = Tokenizer::new("aaa");
        ptr.skip_while(|ch| ch == 'a');
        assert_eq!(ptr.peek(), None);
        assert_eq!(ptr.as_str(), "");
    }

    #[test]
    fn test_backslash_subst_single() {
        // Single Character Escapes
        assert_eq!(bsubst("\\a-"), ('\x07', Some('-')));
        assert_eq!(bsubst("\\b-"), ('\x08', Some('-')));
        assert_eq!(bsubst("\\f-"), ('\x0c', Some('-')));
        assert_eq!(bsubst("\\n-"), ('\n', Some('-')));
        assert_eq!(bsubst("\\r-"), ('\r', Some('-')));
        assert_eq!(bsubst("\\t-"), ('\t', Some('-')));
        assert_eq!(bsubst("\\v-"), ('\x0b', Some('-')));
    }

    #[test]
    fn test_backslash_subst_octal() {
        // Octals
        assert_eq!(bsubst("\\1-"), ('\x01', Some('-')));
        assert_eq!(bsubst("\\17-"), ('\x0f', Some('-')));
        assert_eq!(bsubst("\\177-"), ('\x7f', Some('-')));
        assert_eq!(bsubst("\\1772-"), ('\x7f', Some('2')));
        assert_eq!(bsubst("\\18-"), ('\x01', Some('8')));
        assert_eq!(bsubst("\\8-"), ('8', Some('-')));
    }

    #[test]
    fn test_backslash_subst_hex2() {
        // \xhh: One or two hex digits.
        assert_eq!(bsubst("\\x-"), ('x', Some('-')));
        assert_eq!(bsubst("\\x1-"), ('\x01', Some('-')));
        assert_eq!(bsubst("\\x7f-"), ('\x7f', Some('-')));
    }

    #[test]
    fn test_backslash_subst_hex4() {
        // \uhhhh: 1-4 hex digits.
        assert_eq!(bsubst("\\u-"), ('u', Some('-')));
        assert_eq!(bsubst("\\u7-"), ('\x07', Some('-')));
        assert_eq!(bsubst("\\u77-"), ('w', Some('-')));
        assert_eq!(bsubst("\\u077-"), ('w', Some('-')));
        assert_eq!(bsubst("\\u0077-"), ('w', Some('-')));
        assert_eq!(bsubst("\\u00077-"), ('\x07', Some('7')));
    }

    #[test]
    fn test_backslash_subst_hex8() {
        // \Uhhhhhhhh: 1-8 hex digits.
        assert_eq!(bsubst("\\U-"), ('U', Some('-')));
        assert_eq!(bsubst("\\U7-"), ('\x07', Some('-')));
        assert_eq!(bsubst("\\U77-"), ('w', Some('-')));
        assert_eq!(bsubst("\\U077-"), ('w', Some('-')));
        assert_eq!(bsubst("\\U0077-"), ('w', Some('-')));
        assert_eq!(bsubst("\\U00077-"), ('w', Some('-')));
        assert_eq!(bsubst("\\U000077-"), ('w', Some('-')));
        assert_eq!(bsubst("\\U0000077-"), ('w', Some('-')));
        assert_eq!(bsubst("\\U00000077-"), ('w', Some('-')));
        assert_eq!(bsubst("\\U000000077-"), ('\x07', Some('7')));
    }

    #[test]
    fn test_backslash_subst_other() {
        // Arbitrary Character
        assert_eq!(bsubst("\\*-"), ('*', Some('-')));

        // backslash only
        assert_eq!(bsubst("\\"), ('\\', None));
    }

    fn bsubst(input: &str) -> (char, Option<char>) {
        let mut ctx = Tokenizer::new(input);
        (ctx.backslash_subst(), ctx.as_str().chars().next())
    }
}
