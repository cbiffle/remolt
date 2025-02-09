//! # Molt TCL Parser
//!
//! This is the Molt TCL Parser.  It parses a TCL script (e.g., the contents of a TCL file,
//! the body of a `proc`, the body of a loop, an `if` clause) into an internal form for later
//! evaluation.
//!
//! ## The Dodekalogue
//!
//! TCL syntax is governed by a set of rules called The Dodekalogue.  See the
//! [Tcl(n) man page for Tcl 8.7](https://www.tcl-lang.org/man/tcl8.7/TclCmd/Tcl.htm)
//! details.
//!
//! ## The Internal Form
//!
//! The internal form is as follows:
//!
//! * A `Script` represents a compiled script.
//! * A `Script` consists of list of `WordVec`'s, each of which represents a single command.
//! * A `WordVec` is a list of `Words` representing the command name and its arguments.
//! * A `Word` is an entity that can be evaluated by the interpreter to produce a single
//!   `Value`.
//!
//! ## Evaluation
//!
//! Thus, evaluation consists of looping over the commands in the script.  For each command
//!
//! *   Convert each `Word` in the command's `WordVec` into a `Value`
//! *   Look up the Molt command given its name.
//! *   Pass the list of `Value`'s to the command in the usual way.
//! *   If the command returns `Err(_)`, script execution terminates early and control is
//!     returned to the caller.
//!
//! ## Scripts and Values
//!
//! Script parsing is most usually performed by the `Value::as_script` method as part of
//! script evaluation by the `Interp`.  In this way, the script's internal form persists and
//! need not be recomputed for each evaluation.
//!
//! ## Other Parsing Functions
//!
//! The module provides a number lower-level parsing functions to the rest of the library.
//! For example, the `expr` parser sometimes need to parse quoted string and variable names.
//!
//! ## Variable Name Literals
//!
//! Variable names are parsed in two contexts: as part of "$-substitution", and as simple command
//! arguments, e.g., as in `set my_var 1`.  In the latter case, the variable name is parsed not by
//! the parser but by the command that interprets the argument as a variable name.  This module
//! provides `parse_varname_literal` for this case; it is usually used via `Value::as_var_name`.

use crate::eval_ptr::EvalPtr;
use crate::types::Exception;
use crate::types::VarName;
use crate::util::is_varname_char;
use crate::value::Value;

#[cfg(feature = "internals")]
use crate::types::MoltOptResult;
#[cfg(feature = "internals")]
use crate::interp::Interp;
#[cfg(feature = "internals")]
use crate::check_args;

use alloc::string::{String, ToString as _};
use alloc::vec::Vec;
use alloc::boxed::Box;

/// A compiled script, which can be executed in the context of an interpreter.
#[derive(Debug, PartialEq)]
pub(crate) struct Script {
    // A script is a list of one or more commands to execute.
    commands: Vec<WordVec>,
}

impl Script {
    /// Create a new script object, to which commands will be added during parsing.
    fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    /// Return the list of commands for evaluation.
    pub fn commands(&self) -> &[WordVec] {
        &self.commands
    }
}

/// A single command, consisting of a vector of `Word`'s for evaluation.
#[derive(Debug, PartialEq)]
pub(crate) struct WordVec {
    words: Vec<Word>,
}

impl WordVec {
    /// Create a new `WordVec`, to which `Word`'s can be added during parsing.
    fn new() -> Self {
        Self { words: Vec::new() }
    }

    /// Return the list of words for evaluation.
    pub fn words(&self) -> &[Word] {
        &self.words
    }
}

/// A single `Word` in a command.  A `Word` can be evaluated to produce a `Value`.
#[derive(Debug, PartialEq)]
pub(crate) enum Word {
    /// A `Value`, e.g., the braced word `{a b c}` parses to the value "a b c".
    Value(Value),

    /// VarRef(name): a scalar variable reference, e.g., `$name`
    VarRef(String),

    /// ArrayRef(name, index): an array variable reference, e.g., `$a(1)`.  The index is
    /// represented by a `Word` since it can include various substitutions.
    ArrayRef(String, Box<Word>),

    /// Script(script): A nested script, e.g., `[foo 1 2 3]`.
    Script(Script),

    /// Tokens(words...): A list of `Words` that will be concatenated into a single `Value`,
    /// e.g., `a $x [foo] bar` or `foo.$x`.
    Tokens(Vec<Word>),

    /// Expand(word): A word preceded by the expansion operator, e.g, `{*}...`.
    Expand(Box<Word>),

    /// String(string): A string literal.  This usually appears only as an element in
    /// a `Tokens` list, e.g., the `a` and `b` in `a[myproc]b`.
    ///
    String(String),
}

/// Parses a script, given as a string slice.  Returns a parsed `Script` (or an error).
pub(crate) fn parse(input: &str) -> Result<Script, Exception> {
    // FIRST, create an EvalPtr as a parsing aid; then parse the script.
    let mut ctx = EvalPtr::new(input);
    parse_script(&mut ctx)
}

/// Parses a script represented by an `EvalPtr`.  This form is also used by `expr`.
pub(crate) fn parse_script(ctx: &mut EvalPtr) -> Result<Script, Exception> {
    let mut script = Script::new();

    // Parse commands from the input until we've reach the end.
    while !ctx.at_end_of_script() {
        script.commands.push(parse_command(ctx)?);
    }

    Ok(script)
}

/// Parses a single command from the input, returning it as a `WordVec`.
fn parse_command(ctx: &mut EvalPtr) -> Result<WordVec, Exception> {
    let mut cmd: WordVec = WordVec::new();

    // FIRST, deal with whitespace and comments between "here" and the next command.
    while !ctx.at_end_of_script() {
        ctx.skip_block_white();

        // Either there's a comment, or we're at the beginning of the next command.
        // If the former, skip the comment; then check for more whitespace and comments.
        // Otherwise, go on to the command.
        if !ctx.skip_comment() {
            break;
        }
    }

    // NEXT, Read words until we get to the end of the line or hit an error
    // NOTE: parse_word() can always assume that it's at the beginning of a word.
    while !ctx.at_end_of_command() {
        // FIRST, get the next word; there has to be one, or there's an input error.
        cmd.words.push(parse_next_word(ctx)?);

        // NEXT, skip any whitespace.
        ctx.skip_line_white();
    }

    // NEXT, If we ended at a ";", consume the semi-colon.
    if ctx.next_is(';') {
        ctx.next();
    }

    // NEXT, return the parsed command.
    Ok(cmd)
}

/// Parse and return the next word from the input.
fn parse_next_word(ctx: &mut EvalPtr) -> Result<Word, Exception> {
    if ctx.next_is('{') {
        // FIRST, look for "{*}" operator
        if ctx.tok().as_str().starts_with("{*}") {
            ctx.skip();
            ctx.skip();
            ctx.skip();

            // If the next character is white space, this is just a normal braced
            // word; return its content.  Otherwise, parse what remains as a word
            // and box it in Expand.
            if ctx.at_end() || ctx.next_is_block_white() {
                return Ok(Word::Value(Value::from("*")));
            } else {
                return Ok(Word::Expand(Box::new(parse_next_word(ctx)?)));
            }
        }

        // NEXT, just a normal braced word containing an asterisk.
        parse_braced_word(ctx)
    } else if ctx.next_is('"') {
        parse_quoted_word(ctx)
    } else {
        parse_bare_word(ctx, false)
    }
}

/// Parses a braced word from the input.  It's an error if the there are any non-whitespace
/// characters following the close brace, or if the close brace is missing.
pub(crate) fn parse_braced_word(ctx: &mut EvalPtr) -> Result<Word, Exception> {
    // FIRST, skip the opening brace, and count it; non-escaped braces need to
    // balance.
    ctx.skip_char('{');
    let mut count = 1;

    // NEXT, add tokens to the word until we reach the close quote
    let mut text = String::new();
    let mut start = ctx.mark();

    while !ctx.at_end() {
        // Note: the while condition ensures that there's a character.
        if ctx.next_is('{') {
            count += 1;
            ctx.skip();
        } else if ctx.next_is('}') {
            count -= 1;

            if count > 0 {
                ctx.skip();
            } else {
                // We've found and consumed the closing brace.  We should either
                // see more more whitespace, or we should be at the end of the list
                // Otherwise, there are incorrect characters following the close-brace.
                text.push_str(ctx.token(start));
                let result = Ok(Word::Value(Value::from(text)));
                ctx.skip(); // Skip the closing brace

                if ctx.at_end_of_command() || ctx.next_is_line_white() {
                    return result;
                } else {
                    return molt_err!("extra characters after close-brace");
                }
            }
        } else if ctx.next_is('\\') {
            text.push_str(ctx.token(start));
            ctx.skip();

            // If there's no character it's because we're at the end; and there's
            // no close brace.
            if let Some(ch) = ctx.next() {
                if ch == '\n' {
                    text.push(' ');
                } else {
                    text.push('\\');
                    text.push(ch);
                }
            }
            start = ctx.mark();
        } else {
            ctx.skip();
        }
    }

    molt_err!("missing close-brace")
}

/// Parses a quoted word, handling backslash, variable, and command substitution. It's
/// an error if the there are any non-whitespace characters following the close quote, or
/// if the close quote is missing.
pub(crate) fn parse_quoted_word(ctx: &mut EvalPtr) -> Result<Word, Exception> {
    // FIRST, consume the the opening quote.
    ctx.next();

    // NEXT, add tokens to the word until we reach the close quote
    let mut tokens = Tokens::new();
    let mut start = ctx.mark();

    while !ctx.at_end() {
        // Note: the while condition ensures that there's a character.
        if ctx.next_is('[') {
            if start != ctx.mark() {
                tokens.push_str(ctx.token(start));
            }
            tokens.push(Word::Script(parse_brackets(ctx)?));
            start = ctx.mark();
        } else if ctx.next_is('$') {
            if start != ctx.mark() {
                tokens.push_str(ctx.token(start));
            }
            parse_dollar(ctx, &mut tokens)?;
            start = ctx.mark();
        } else if ctx.next_is('\\') {
            if start != ctx.mark() {
                tokens.push_str(ctx.token(start));
            }
            tokens.push_char(ctx.backslash_subst());
            start = ctx.mark();
        } else if ctx.next_is('"') {
            if start != ctx.mark() {
                tokens.push_str(ctx.token(start));
            }
            ctx.skip_char('"');
            if !ctx.at_end_of_command() && !ctx.next_is_line_white() {
                return molt_err!("extra characters after close-quote");
            } else {
                return Ok(tokens.take());
            }
        } else {
            ctx.skip();
        }
    }

    molt_err!("missing \"")
}

/// Parses a bare word, handling backslash, variable, and command substitution.
fn parse_bare_word(ctx: &mut EvalPtr, index_flag: bool) -> Result<Word, Exception> {
    let mut tokens = Tokens::new();
    let mut start = ctx.mark();

    while !ctx.at_end_of_command() && !ctx.next_is_line_white() {
        // Note: the while condition ensures that there's a character.
        if index_flag && ctx.next_is(')') {
            // Parsing an array index, and we're at the end.
            break;
        } else if ctx.next_is('[') {
            if start != ctx.mark() {
                tokens.push_str(ctx.token(start));
            }
            tokens.push(Word::Script(parse_brackets(ctx)?));
            start = ctx.mark();
        } else if ctx.next_is('$') {
            if start != ctx.mark() {
                tokens.push_str(ctx.token(start));
            }
            parse_dollar(ctx, &mut tokens)?;
            start = ctx.mark();
        } else if ctx.next_is('\\') {
            if start != ctx.mark() {
                tokens.push_str(ctx.token(start));
            }
            tokens.push_char(ctx.backslash_subst());
            start = ctx.mark();
        } else {
            ctx.skip();
        }
    }

    if start != ctx.mark() {
        tokens.push_str(ctx.token(start));
    }

    Ok(tokens.take())
}

/// Parses an embedded script in a bare or quoted word, returning the result as a
/// Script.  It's an error if the close-bracket is missing.
fn parse_brackets(ctx: &mut EvalPtr) -> Result<Script, Exception> {
    // FIRST, skip the '['
    ctx.skip_char('[');

    // NEXT, parse the script up to the matching ']'
    let old_flag = ctx.is_bracket_term();
    ctx.set_bracket_term(true);
    let result = parse_script(ctx);
    ctx.set_bracket_term(old_flag);

    // NEXT, make sure there's a closing bracket
    if result.is_ok() {
        if ctx.next_is(']') {
            ctx.next();
        } else {
            return molt_err!("missing close-bracket");
        }
    }

    result
}

/// Parses a "$" in the input, and pushes the result into a list of tokens.  Usually this
/// will be a variable reference, but it may simply be a bare "$".
fn parse_dollar(ctx: &mut EvalPtr, tokens: &mut Tokens) -> Result<(), Exception> {
    // FIRST, skip the '$'
    ctx.skip_char('$');

    // NEXT, make sure this is really a variable reference.  If it isn't
    // just return a "$".
    if !ctx.next_is_varname_char() && !ctx.next_is('{') {
        tokens.push_char('$');
    } else {
        tokens.push(parse_varname(ctx)?);
    }

    Ok(())
}

/// Parses a variable name; the "$" has already been consumed.  Handles both braced
/// and non-braced variable names, including array names.
///
/// Also used by expr.rs.
pub(crate) fn parse_varname(ctx: &mut EvalPtr) -> Result<Word, Exception> {
    // FIRST, is this a braced variable name?
    if ctx.next_is('{') {
        ctx.skip_char('{');
        let start = ctx.mark();
        ctx.skip_while(|ch| ch != '}');

        if ctx.at_end() {
            return molt_err!("missing close-brace for variable name");
        }

        let var_name = parse_varname_literal(ctx.token(start));
        ctx.skip_char('}');
        match var_name.index() {
            Some(index) => Ok(Word::ArrayRef(
                var_name.name().into(),
                Box::new(Word::String(index.into())),
            )),
            None => Ok(Word::VarRef(var_name.name().into())),
        }
    } else {
        let start = ctx.mark();
        ctx.skip_while(is_varname_char);
        let name = ctx.token(start).to_string();

        if !ctx.next_is('(') {
            // Scalar; just return it.
            Ok(Word::VarRef(name))
        } else {
            // Array; parse out the word that evaluates to the index.
            ctx.skip();
            let index = parse_bare_word(ctx, true)?;
            ctx.skip_char(')');
            Ok(Word::ArrayRef(name, Box::new(index)))
        }
    }
}

/// Parses a literal variable name: a string that is known to be a complete variable
/// name.
///
/// If it contains an opening parenthesis and ends with a closing parenthesis, then
/// it's an array reference; otherwise it's just a scalar name.
pub(crate) fn parse_varname_literal(literal: &str) -> VarName {
    let mut ctx = EvalPtr::new(literal);

    // FIRST, find the first open parenthesis.  If there is none, just return the literal
    // as a scalar.
    let start = ctx.mark();
    ctx.skip_while(|ch| ch != '(');

    if ctx.at_end() {
        return VarName::scalar(literal.into());
    }

    // NEXT, pluck out the name.
    let name = ctx.token(start).to_string();
    ctx.skip_char('(');

    if ctx.tok().as_str().is_empty() {
        return VarName::scalar(literal.into());
    }

    // NEXT, skip to the final character.
    let start = ctx.mark();
    let chars_left = ctx.tok().as_str().len() - 1;

    for _ in 0..chars_left {
        ctx.skip();
    }

    if ctx.next_is(')') {
        VarName::array(name, ctx.token(start).to_string())
    } else {
        VarName::scalar(literal.into())
    }
}

/// The Tokens structure.  This is used when parsing a bare or quoted word; the
/// intent is to accumulate the relevant words, while merging adjacent string literals.
struct Tokens {
    /// The list of words
    list: Vec<Word>,

    /// If true, we're accumulating a string literal, which will eventually become a `Word`.
    got_string: bool,

    /// The string literal we're accumulating, if any, or an empty string otherwise.
    string: String,
}

impl Tokens {
    /// Creates a new Tokens structure.
    fn new() -> Self {
        Self {
            list: Vec::new(),
            got_string: false,
            string: String::new(),
        }
    }

    /// Pushes an entire word into the list of tokens.  If a string literal is being
    /// accumulated, it is turned into a `Word` and pushed before the input word.
    fn push(&mut self, word: Word) {
        if self.got_string {
            let string = core::mem::take(&mut self.string);
            self.list.push(Word::String(string));
            self.got_string = false;
        }

        self.list.push(word);
    }

    /// Pushes a literal string onto the list of tokens.  It will be merged with any
    /// string literal that's being accumulated.
    fn push_str(&mut self, str: &str) {
        self.string.push_str(str);
        self.got_string = true;
    }

    /// Pushes a single character onto the list of tokens.  It will be merged with any
    /// string literal that's being accumulated.
    fn push_char(&mut self, ch: char) {
        self.string.push(ch);
        self.got_string = true;
    }

    /// Takes the accumulated tokens as a single `Word`, either `Word::Value` or
    /// `Word::Tokens`.
    fn take(mut self) -> Word {
        if self.got_string {
            // If there's nothing but the string, turn it into a value.
            // Otherwise, just add it to the list of tokens.
            if self.list.is_empty() {
                return Word::Value(Value::from(self.string));
            } else {
                let string = core::mem::take(&mut self.string);
                self.list.push(Word::String(string));
            }
        }

        if self.list.is_empty() {
            Word::Value(Value::empty())
        } else if self.list.len() == 1 {
            self.list.pop().unwrap()
        } else {
            Word::Tokens(self.list)
        }
    }
}

/// # parse *script*
///
/// A command for parsing an arbitrary script and outputting the parsed form.
/// This is an undocumented debugging aid.  The output can be greatly improved.
#[cfg(feature = "internals")]
pub fn cmd_parse(_interp: &mut Interp, argv: &[Value]) -> MoltOptResult {
    check_args(1, argv, 2, 2, "script")?;

    let script = &argv[1];

    molt_opt_ok!(alloc::format!("{:?}", parse(script.as_str())?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokens() {
        // No tokens pushed; get empty string.
        let tokens = Tokens::new();
        assert_eq!(tokens.take(), Word::Value(Value::empty()));

        // Push normal Word only; get it back.
        let mut tokens = Tokens::new();
        tokens.push(Word::Value(Value::from("abc")));
        assert_eq!(tokens.take(), Word::Value(Value::from("abc")));

        // Push a single str.  Get Value.
        let mut tokens = Tokens::new();
        tokens.push_str("xyz");
        assert_eq!(tokens.take(), Word::Value(Value::from("xyz")));

        // Push two strs.  Get one value.
        let mut tokens = Tokens::new();
        tokens.push_str("abc");
        tokens.push_str("def");
        assert_eq!(tokens.take(), Word::Value(Value::from("abcdef")));

        // Push strs and chars.  Get one value.
        let mut tokens = Tokens::new();
        tokens.push_str("abc");
        tokens.push_char('/');
        tokens.push_str("def");
        assert_eq!(tokens.take(), Word::Value(Value::from("abc/def")));

        // Push multiple normal words
        let mut tokens = Tokens::new();
        tokens.push(Word::VarRef("a".into()));
        tokens.push(Word::String("xyz".into()));
        assert_eq!(
            tokens.take(),
            Word::Tokens(vec![Word::VarRef("a".into()), Word::String("xyz".into())])
        );

        // Push a string, a word, and another string
        let mut tokens = Tokens::new();
        tokens.push_str("a");
        tokens.push_str("b");
        tokens.push(Word::VarRef("xyz".into()));
        tokens.push_str("c");
        tokens.push_str("d");
        assert_eq!(
            tokens.take(),
            Word::Tokens(vec![
                Word::String("ab".into()),
                Word::VarRef("xyz".into()),
                Word::String("cd".into())
            ])
        );
    }

    #[test]
    fn test_parse() {
        assert!(parse("").unwrap().commands.is_empty());

        let cmds = parse("a").unwrap().commands;
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].words, vec![Word::Value(Value::from("a"))]);

        let cmds = parse("a\nb").unwrap().commands;
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].words, vec![Word::Value(Value::from("a"))]);
        assert_eq!(cmds[1].words, vec![Word::Value(Value::from("b"))]);

        let cmds = parse("a;b").unwrap().commands;
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].words, vec![Word::Value(Value::from("a"))]);
        assert_eq!(cmds[1].words, vec![Word::Value(Value::from("b"))]);

        let cmds = parse(" a ; b ").unwrap().commands;
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].words, vec![Word::Value(Value::from("a"))]);
        assert_eq!(cmds[1].words, vec![Word::Value(Value::from("b"))]);

        assert_eq!(parse("a {"), molt_err!("missing close-brace"));
    }

    #[test]
    fn test_parse_next_word() {
        // NOTE: The point of this test is to make sure that parse_next_word is
        // calling the right functions to complete the job, not to verify what
        // those functions are doing; they have their own tests.

        // Normal Braced Word
        assert_eq!(
            pword("{abc}"),
            Ok((Word::Value(Value::from("abc")), "".into()))
        );

        // {*} at end of input
        assert_eq!(pword("{*}"), Ok((Word::Value(Value::from("*")), "".into())));

        // {*} followed by white-space
        assert_eq!(
            pword("{*} "),
            Ok((Word::Value(Value::from("*")), " ".into()))
        );

        // {*} followed by word
        assert_eq!(
            pword("{*}abc "),
            Ok((
                Word::Expand(Box::new(Word::Value(Value::from("abc")))),
                " ".into()
            ))
        );

        // Quoted Word
        assert_eq!(
            pword("\"abc\""),
            Ok((Word::Value(Value::from("abc")), "".into()))
        );

        // Bare word
        assert_eq!(
            pword("abc"),
            Ok((Word::Value(Value::from("abc")), "".into()))
        );
    }

    fn pword(input: &str) -> Result<(Word, String), Exception> {
        let mut ctx = EvalPtr::new(input);
        let word = parse_next_word(&mut ctx)?;
        Ok((word, ctx.tok().as_str().to_string()))
    }

    #[test]
    fn test_parse_braced_word() {
        // Simple string
        assert_eq!(
            pbrace("{abc}"),
            Ok((Word::Value(Value::from("abc")), "".into()))
        );

        // Simple string with following space
        assert_eq!(
            pbrace("{abc} "),
            Ok((Word::Value(Value::from("abc")), " ".into()))
        );

        // String with white space
        assert_eq!(
            pbrace("{a b c} "),
            Ok((Word::Value(Value::from("a b c")), " ".into()))
        );

        // String with $ and []space
        assert_eq!(
            pbrace("{a $b [c]} "),
            Ok((Word::Value(Value::from("a $b [c]")), " ".into()))
        );

        // String with balanced braces
        assert_eq!(
            pbrace("{a{b}c} "),
            Ok((Word::Value(Value::from("a{b}c")), " ".into()))
        );

        // String with escaped braces
        assert_eq!(
            pbrace("{a\\{bc} "),
            Ok((Word::Value(Value::from("a\\{bc")), " ".into()))
        );

        assert_eq!(
            pbrace("{ab\\}c} "),
            Ok((Word::Value(Value::from("ab\\}c")), " ".into()))
        );

        // String with escaped newline (a real newline with a \ in front)
        assert_eq!(
            pbrace("{ab\\\nc} "),
            Ok((Word::Value(Value::from("ab c")), " ".into()))
        );

        // Strings with missing close-brace
        assert_eq!(pbrace("{abc"), molt_err!("missing close-brace"));

        assert_eq!(pbrace("{a{b}c"), molt_err!("missing close-brace"));
    }

    fn pbrace(input: &str) -> Result<(Word, String), Exception> {
        let mut ctx = EvalPtr::new(input);
        let word = parse_braced_word(&mut ctx)?;
        Ok((word, ctx.tok().as_str().to_string()))
    }

    #[test]
    fn test_parse_quoted_word() {
        // Simple string
        assert_eq!(
            pqw("\"abc\""),
            Ok((Word::Value(Value::from("abc")), "".into()))
        );

        // Simple string with text following
        assert_eq!(
            pqw("\"abc\" "),
            Ok((Word::Value(Value::from("abc")), " ".into()))
        );

        // Backslash substitution at beginning, middle, and end
        assert_eq!(
            pqw("\"\\x77-\" "),
            Ok((Word::Value(Value::from("w-")), " ".into()))
        );

        assert_eq!(
            pqw("\"-\\x77-\" "),
            Ok((Word::Value(Value::from("-w-")), " ".into()))
        );

        assert_eq!(
            pqw("\"-\\x77\" "),
            Ok((Word::Value(Value::from("-w")), " ".into()))
        );

        // Variable reference
        assert_eq!(
            pqw("\"a$x.b\" "),
            Ok((
                Word::Tokens(vec![
                    Word::String("a".into()),
                    Word::VarRef("x".into()),
                    Word::String(".b".into()),
                ]),
                " ".into()
            ))
        );

        assert_eq!(
            pqw("\"a${x}b\" "),
            Ok((
                Word::Tokens(vec![
                    Word::String("a".into()),
                    Word::VarRef("x".into()),
                    Word::String("b".into()),
                ]),
                " ".into()
            ))
        );

        // Not actually a variable reference
        assert_eq!(
            pqw("\"a$.b\" "),
            Ok((Word::Value(Value::from("a$.b")), " ".into()))
        );

        // Brackets
        assert_eq!(
            pqw("\"a[list b]c\" "),
            Ok((
                Word::Tokens(vec![
                    Word::String("a".into()),
                    Word::Script(pbrack("[list b]").unwrap()),
                    Word::String("c".into()),
                ]),
                " ".into()
            ))
        );

        // Missing close quote
        assert_eq!(pqw("\"abc"), molt_err!("missing \""));

        // Extra characters after close-quote
        assert_eq!(
            pqw("\"abc\"x "),
            molt_err!("extra characters after close-quote")
        );
    }

    fn pqw(input: &str) -> Result<(Word, String), Exception> {
        let mut ctx = EvalPtr::new(input);
        let word = parse_quoted_word(&mut ctx)?;
        Ok((word, ctx.tok().as_str().to_string()))
    }

    #[test]
    fn test_parse_bare_word() {
        // Simple string
        assert_eq!(
            pbare("abc", false),
            Ok((Word::Value(Value::from("abc")), "".into()))
        );

        // Simple string with text following
        assert_eq!(
            pbare("abc ", false),
            Ok((Word::Value(Value::from("abc")), " ".into()))
        );

        // Backslash substitution at beginning, middle, and end
        assert_eq!(
            pbare("\\x77- ", false),
            Ok((Word::Value(Value::from("w-")), " ".into()))
        );

        assert_eq!(
            pbare("-\\x77- ", false),
            Ok((Word::Value(Value::from("-w-")), " ".into()))
        );

        assert_eq!(
            pbare("-\\x77 ", false),
            Ok((Word::Value(Value::from("-w")), " ".into()))
        );

        // Variable reference
        assert_eq!(
            pbare("a$x.b ", false),
            Ok((
                Word::Tokens(vec![
                    Word::String("a".into()),
                    Word::VarRef("x".into()),
                    Word::String(".b".into()),
                ]),
                " ".into()
            ))
        );

        assert_eq!(
            pbare("a${x}b ", false),
            Ok((
                Word::Tokens(vec![
                    Word::String("a".into()),
                    Word::VarRef("x".into()),
                    Word::String("b".into()),
                ]),
                " ".into()
            ))
        );

        // Not actually a variable reference
        assert_eq!(
            pbare("a$.b ", false),
            Ok((Word::Value(Value::from("a$.b")), " ".into()))
        );

        // Brackets
        assert_eq!(
            pbare("a[list b]c ", false),
            Ok((
                Word::Tokens(vec![
                    Word::String("a".into()),
                    Word::Script(pbrack("[list b]").unwrap()),
                    Word::String("c".into()),
                ]),
                " ".into()
            ))
        );

        // Array index
        assert_eq!(
            // Parse up to but not including the ")".
            pbare("a)b", true),
            Ok((Word::Value(Value::from("a")), ")b".into()))
        );
    }

    fn pbare(input: &str, index_flag: bool) -> Result<(Word, String), Exception> {
        let mut ctx = EvalPtr::new(input);
        let word = parse_bare_word(&mut ctx, index_flag)?;
        Ok((word, ctx.tok().as_str().to_string()))
    }

    #[test]
    fn test_parse_brackets() {
        let script = pbrack("[set a 5]").unwrap();
        assert_eq!(script.commands.len(), 1);
        let cmd = &script.commands[0];
        assert_eq!(
            cmd.words,
            vec![
                Word::Value(Value::from("set")),
                Word::Value(Value::from("a")),
                Word::Value(Value::from("5")),
            ]
        );

        assert_eq!(pbrack("[incomplete"), molt_err!("missing close-bracket"));
    }

    fn pbrack(input: &str) -> Result<Script, Exception> {
        let mut ctx = EvalPtr::new(input);
        parse_brackets(&mut ctx)
    }

    #[test]
    fn test_parse_dollar() {
        // Normal var names
        assert_eq!(pvar("$a"), Ok((Word::VarRef("a".into()), "".into())));
        assert_eq!(pvar("$abc"), Ok((Word::VarRef("abc".into()), "".into())));
        assert_eq!(pvar("$abc."), Ok((Word::VarRef("abc".into()), ".".into())));
        assert_eq!(pvar("$a.bc"), Ok((Word::VarRef("a".into()), ".bc".into())));
        assert_eq!(
            pvar("$a1_.bc"),
            Ok((Word::VarRef("a1_".into()), ".bc".into()))
        );

        // Array names
        assert_eq!(
            pvar("$a(1)"),
            Ok((
                Word::ArrayRef("a".into(), Box::new(Word::Value(Value::from("1")))),
                "".into()
            ))
        );

        // Braced var names
        assert_eq!(pvar("${a}b"), Ok((Word::VarRef("a".into()), "b".into())));
        assert_eq!(
            pvar("${ab"),
            molt_err!("missing close-brace for variable name")
        );

        // Braced var names with arrays
        assert_eq!(
            pvar("${a(1)}"),
            Ok((
                Word::ArrayRef("a".into(), Box::new(Word::String("1".into()))),
                "".into()
            ))
        );

        // Just a bare "$"
        assert_eq!(pvar("$"), Ok((Word::Value(Value::from("$")), "".into())));
        assert_eq!(pvar("$."), Ok((Word::Value(Value::from("$")), ".".into())));
    }

    fn pvar(input: &str) -> Result<(Word, String), Exception> {
        let mut ctx = EvalPtr::new(input);
        let mut tokens = Tokens::new();
        parse_dollar(&mut ctx, &mut tokens)?;
        Ok((tokens.take(), ctx.tok().as_str().to_string()))
    }

    #[test]
    fn test_parse_varname_literal() {
        // Scalars
        assert_eq!(parse_varname_literal(""), scalar(""));
        assert_eq!(parse_varname_literal("a"), scalar("a"));
        assert_eq!(parse_varname_literal("a(b"), scalar("a(b"));
        assert_eq!(parse_varname_literal("("), scalar("("));
        assert_eq!(parse_varname_literal(")"), scalar(")"));
        assert_eq!(parse_varname_literal("a(b)c"), scalar("a(b)c"));
        assert_eq!(parse_varname_literal("(b)c"), scalar("(b)c"));

        // Arrays
        assert_eq!(parse_varname_literal("a(b)"), array("a", "b"));
        assert_eq!(parse_varname_literal("a({)"), array("a", "{"));
        assert_eq!(parse_varname_literal("()"), array("", ""));
        assert_eq!(parse_varname_literal("(b)"), array("", "b"));
        assert_eq!(parse_varname_literal("a()"), array("a", ""));
        assert_eq!(parse_varname_literal("%(()"), array("%", "("));
        assert_eq!(parse_varname_literal("%())"), array("%", ")"));
    }

    fn scalar(name: &str) -> VarName {
        VarName::scalar(name.into())
    }

    fn array(name: &str, index: &str) -> VarName {
        VarName::array(name.into(), index.into())
    }
}
