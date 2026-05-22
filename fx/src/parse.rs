//! Text parser — Hydra-shaped surface syntax that lowers to the same AST
//! `builder.rs` constructs. Grammar (informal):
//!
//! ```text
//! program    := statement (NEWLINE | SEMI)* ...
//! statement  := 'let' IDENT '=' expr
//!             | expr        // must end in `.out()` or `.out("buf")` to
//!                           // contribute to the composition
//!
//! expr       := primary ('.' IDENT '(' arglist? ')' )*
//! primary    := NUMBER | STRING | IDENT | IDENT '(' arglist? ')' | '(' expr ')'
//! arglist    := expr (',' expr)* ','?
//! ```
//!
//! Inside an expression a bound `let` identifier resolves to the [`Chain`]
//! it was bound to. Numeric and string literals only appear as argument
//! values. There is no operator syntax — everything is method-chain calls,
//! matching Hydra.

use std::collections::HashMap;

use crate::ast::{Combinator, Source, Transform};
use crate::builder::{Chain, Composition};
use crate::diagnostics::Diagnostic;
use crate::value::{UniformRef, Value};

pub fn parse(input: &str) -> Result<Composition, Diagnostic> {
    let tokens = tokenize(input)?;
    let mut parser = Parser::new(&tokens);
    parser.parse_program()
}

// ---- tokens ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Number(f64),
    String(String),
    LParen,
    RParen,
    Dot,
    Comma,
    Semi,
    Equals,
    Newline,
    Eof,
}

#[derive(Debug, Clone)]
struct Token {
    kind: Tok,
    line: usize,
    col: usize,
}

fn tokenize(input: &str) -> Result<Vec<Token>, Diagnostic> {
    let mut out = Vec::new();
    let mut chars = input.char_indices().peekable();
    let mut line = 1usize;
    let mut line_start = 0usize;

    while let Some(&(idx, c)) = chars.peek() {
        let col = idx - line_start + 1;
        match c {
            ' ' | '\t' | '\r' => {
                chars.next();
            }
            '\n' => {
                out.push(Token { kind: Tok::Newline, line, col });
                chars.next();
                line += 1;
                line_start = idx + 1;
            }
            '/' if peek_next_is(&chars, '/') => {
                while let Some(&(_, c)) = chars.peek() {
                    if c == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '(' => {
                chars.next();
                out.push(Token { kind: Tok::LParen, line, col });
            }
            ')' => {
                chars.next();
                out.push(Token { kind: Tok::RParen, line, col });
            }
            '.' if !peek_next_is_digit(&chars) => {
                chars.next();
                out.push(Token { kind: Tok::Dot, line, col });
            }
            ',' => {
                chars.next();
                out.push(Token { kind: Tok::Comma, line, col });
            }
            ';' => {
                chars.next();
                out.push(Token { kind: Tok::Semi, line, col });
            }
            '=' => {
                chars.next();
                out.push(Token { kind: Tok::Equals, line, col });
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                let start_col = col;
                loop {
                    match chars.next() {
                        Some((_, '"')) => break,
                        Some((_, '\\')) => match chars.next() {
                            Some((_, esc)) => s.push(esc),
                            None => {
                                return Err(Diagnostic::ParseError {
                                    line,
                                    col: start_col,
                                    message: "unterminated string literal".into(),
                                })
                            }
                        },
                        Some((_, ch)) => s.push(ch),
                        None => {
                            return Err(Diagnostic::ParseError {
                                line,
                                col: start_col,
                                message: "unterminated string literal".into(),
                            })
                        }
                    }
                }
                out.push(Token { kind: Tok::String(s), line, col });
            }
            d if d.is_ascii_digit() || d == '-' || d == '.' => {
                let mut num = String::new();
                let start_col = col;
                if d == '-' {
                    let mut peeked = chars.clone();
                    peeked.next();
                    match peeked.peek() {
                        Some(&(_, c2)) if c2.is_ascii_digit() || c2 == '.' => {
                            num.push('-');
                            chars.next();
                        }
                        _ => {
                            return Err(Diagnostic::ParseError {
                                line,
                                col: start_col,
                                message: "unexpected '-'".into(),
                            });
                        }
                    }
                }
                while let Some(&(_, c)) = chars.peek() {
                    if c.is_ascii_digit() || c == '.' {
                        num.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let n: f64 = num.parse().map_err(|_| Diagnostic::ParseError {
                    line,
                    col: start_col,
                    message: format!("invalid number '{}'", num),
                })?;
                out.push(Token { kind: Tok::Number(n), line, col: start_col });
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut name = String::new();
                let start_col = col;
                while let Some(&(_, c)) = chars.peek() {
                    if c.is_alphanumeric() || c == '_' {
                        name.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push(Token { kind: Tok::Ident(name), line, col: start_col });
            }
            other => {
                return Err(Diagnostic::ParseError {
                    line,
                    col,
                    message: format!("unexpected character '{}'", other),
                })
            }
        }
    }

    out.push(Token { kind: Tok::Eof, line, col: 1 });
    Ok(out)
}

fn peek_next_is(chars: &std::iter::Peekable<std::str::CharIndices>, target: char) -> bool {
    let mut clone = chars.clone();
    clone.next();
    clone.peek().map(|&(_, c)| c == target).unwrap_or(false)
}

fn peek_next_is_digit(chars: &std::iter::Peekable<std::str::CharIndices>) -> bool {
    let mut clone = chars.clone();
    clone.next();
    clone.peek().map(|&(_, c)| c.is_ascii_digit()).unwrap_or(false)
}

// ---- parser ------------------------------------------------------------

// Chain/Composition variants dominate the enum size because they wrap the
// AST. Boxing every variant would noise up the matchers throughout
// apply_function/apply_method for a parser-internal type that never escapes
// the call stack.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
enum ParseVal {
    Chain(Chain),
    Composition(Composition),
    Number(f64),
    String(String),
    /// A non-constant Value (today: a uniform reference like
    /// `audio_rms()`). Tweens never come from the parser — they're
    /// inherently Animato-built and require live structs the surface
    /// syntax can't express; pass them through the builder API.
    DynValue(Value),
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    env: HashMap<String, Chain>,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, pos: 0, env: HashMap::new() }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn eat_newlines_and_semis(&mut self) {
        while matches!(self.peek().kind, Tok::Newline | Tok::Semi) {
            self.advance();
        }
    }

    fn parse_program(&mut self) -> Result<Composition, Diagnostic> {
        self.eat_newlines_and_semis();
        let mut comp: Option<Composition> = None;
        while self.peek().kind != Tok::Eof {
            self.parse_statement(&mut comp)?;
            self.eat_newlines_and_semis();
        }
        comp.ok_or_else(|| {
            Diagnostic::InvalidComposition(
                "program has no `.out(...)` call — nothing to compile".into(),
            )
        })
    }

    fn parse_statement(&mut self, comp: &mut Option<Composition>) -> Result<(), Diagnostic> {
        if let Tok::Ident(name) = &self.peek().kind {
            if name == "let" {
                self.advance();
                let bind_name = self.expect_ident("identifier after `let`")?;
                self.expect(Tok::Equals, "expected '=' in let binding")?;
                let val = self.parse_expr()?;
                let chain = match val {
                    ParseVal::Chain(c) => c,
                    other => {
                        return Err(self.err(format!(
                            "right-hand side of `let {}` must be a chain expression, got {:?}",
                            bind_name, other
                        )))
                    }
                };
                self.env.insert(bind_name, chain);
                return Ok(());
            }
        }

        let val = self.parse_expr()?;
        match val {
            ParseVal::Composition(c) => {
                *comp = Some(match comp.take() {
                    Some(existing) => existing.and_then(c),
                    None => c,
                });
                Ok(())
            }
            ParseVal::Chain(_) => Err(self.err(
                "expression at top level must end in `.out()` or `.out(\"buffer\")`".into(),
            )),
            other => Err(self.err(format!(
                "top-level statement must be `let` or a chain ending in `.out(...)`, got {:?}",
                other
            ))),
        }
    }

    fn parse_expr(&mut self) -> Result<ParseVal, Diagnostic> {
        let mut head = self.parse_primary()?;
        while self.peek().kind == Tok::Dot {
            self.advance();
            let method = self.expect_ident("method name after '.'")?;
            let args = if self.peek().kind == Tok::LParen {
                self.advance();
                let args = self.parse_arglist()?;
                self.expect(Tok::RParen, "expected ')' after method args")?;
                args
            } else {
                Vec::new()
            };
            head = self.apply_method(head, &method, args)?;
        }
        Ok(head)
    }

    fn parse_primary(&mut self) -> Result<ParseVal, Diagnostic> {
        match &self.peek().kind {
            Tok::Number(n) => {
                let n = *n;
                self.advance();
                Ok(ParseVal::Number(n))
            }
            Tok::String(_) => {
                let tok = self.advance();
                if let Tok::String(s) = tok.kind {
                    Ok(ParseVal::String(s))
                } else {
                    unreachable!()
                }
            }
            Tok::LParen => {
                self.advance();
                let inner = self.parse_expr()?;
                self.expect(Tok::RParen, "expected ')'")?;
                Ok(inner)
            }
            Tok::Ident(_) => {
                let tok = self.advance();
                let name = if let Tok::Ident(n) = tok.kind { n } else { unreachable!() };
                if self.peek().kind == Tok::LParen {
                    self.advance();
                    let args = self.parse_arglist()?;
                    self.expect(Tok::RParen, "expected ')' after function args")?;
                    self.apply_function(&name, args)
                } else if let Some(chain) = self.env.get(&name) {
                    Ok(ParseVal::Chain(chain.clone()))
                } else if let Some(implicit) = implicit_uniform(&name) {
                    // Well-known transition-pipeline uniforms — `progress` is
                    // the obvious one: agents writing wavelet_fx for crossfades /
                    // wipes / dip-to-black reach for it by name (CSS / GLSL
                    // muscle memory) rather than spelling out
                    // `prop("progress")`. Auto-bind it as the equivalent
                    // CssProp reference so the parse succeeds and the
                    // pipeline's existing uniform plumbing fills the slot.
                    Ok(ParseVal::DynValue(implicit))
                } else {
                    Err(self.err(format!(
                        "unknown identifier `{}` (no let binding and not called as a function)",
                        name
                    )))
                }
            }
            other => Err(self.err(format!("unexpected token in expression: {:?}", other))),
        }
    }

    fn parse_arglist(&mut self) -> Result<Vec<ParseVal>, Diagnostic> {
        let mut args = Vec::new();
        if self.peek().kind == Tok::RParen {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expr()?);
            if self.peek().kind == Tok::Comma {
                self.advance();
                if self.peek().kind == Tok::RParen {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(args)
    }

    fn apply_function(&self, name: &str, args: Vec<ParseVal>) -> Result<ParseVal, Diagnostic> {
        let n = args.len();

        // Dynamic-uniform constructors: these return Values, not Chains, so
        // they can sit anywhere a numeric literal sits — e.g. `noise(audio_rms())`.
        match name {
            "audio_rms" => {
                if n != 0 {
                    return Err(self.err(format!("audio_rms() takes no arguments, got {}", n)));
                }
                return Ok(ParseVal::DynValue(Value::Uniform(UniformRef::AudioRms)));
            }
            "audio_fft" => {
                if n != 1 {
                    return Err(self.err(format!("audio_fft(n) expects 1 argument, got {}", n)));
                }
                let bin = get_u32(&args, 0, "audio_fft", 0)?;
                return Ok(ParseVal::DynValue(Value::Uniform(UniformRef::AudioFftBin(bin))));
            }
            "time_beat" => {
                if n != 0 {
                    return Err(self.err(format!("time_beat() takes no arguments, got {}", n)));
                }
                return Ok(ParseVal::DynValue(Value::Uniform(UniformRef::Beat)));
            }
            "seed" => {
                if n != 0 {
                    return Err(self.err(format!("seed() takes no arguments, got {}", n)));
                }
                return Ok(ParseVal::DynValue(Value::Uniform(UniformRef::Seed)));
            }
            "prop" => {
                if n != 1 {
                    return Err(self.err(format!("prop(name) expects 1 argument, got {}", n)));
                }
                let pname = get_string(&args, 0, "prop")?;
                return Ok(ParseVal::DynValue(Value::Uniform(UniformRef::CssProp(pname))));
            }
            _ => {}
        }

        let chain = match name {
            "noise" => Chain(crate::ast::Node::Source(Source::Noise {
                scale: get_value(&args, 0, "noise", 1.0)?,
                offset: get_value(&args, 1, "noise", 0.0)?,
            })),
            "osc" => Chain(crate::ast::Node::Source(Source::Osc {
                frequency: get_value(&args, 0, "osc", 60.0)?,
                sync: get_value(&args, 1, "osc", 0.1)?,
                offset: get_value(&args, 2, "osc", 0.0)?,
            })),
            "solid" => Chain(crate::ast::Node::Source(Source::Solid {
                r: get_value(&args, 0, "solid", 0.0)?,
                g: get_value(&args, 1, "solid", 0.0)?,
                b: get_value(&args, 2, "solid", 0.0)?,
                a: get_value(&args, 3, "solid", 1.0)?,
            })),
            "gradient" => Chain(crate::ast::Node::Source(Source::Gradient {
                speed: get_value(&args, 0, "gradient", 0.0)?,
            })),
            "voronoi" => Chain(crate::ast::Node::Source(Source::Voronoi {
                scale: get_value(&args, 0, "voronoi", 5.0)?,
                speed: get_value(&args, 1, "voronoi", 0.3)?,
                blending: get_value(&args, 2, "voronoi", 0.3)?,
            })),
            "shape" => Chain(crate::ast::Node::Source(Source::Shape {
                sides: get_u32(&args, 0, "shape", 3)?,
                radius: get_value(&args, 1, "shape", 0.3)?,
                smoothing: get_value(&args, 2, "shape", 0.01)?,
            })),
            "src" => {
                if n != 1 {
                    return Err(self.err(format!("src() expects 1 argument, got {}", n)));
                }
                Chain(crate::ast::Node::Source(Source::Src {
                    channel: get_u32(&args, 0, "src", 0)?,
                }))
            }
            "prev" => {
                if n != 0 {
                    return Err(self.err(format!("prev() takes no arguments, got {}", n)));
                }
                Chain(crate::ast::Node::Source(Source::Prev))
            }
            "from_buffer" => {
                if n != 1 {
                    return Err(self.err(format!("from_buffer() expects 1 argument, got {}", n)));
                }
                let bname = get_string(&args, 0, "from_buffer")?;
                Chain(crate::ast::Node::Source(Source::Buffer { name: bname }))
            }
            other => {
                return Err(self.err(format!("unknown function `{}`", other)));
            }
        };
        Ok(ParseVal::Chain(chain))
    }

    fn apply_method(
        &self,
        head: ParseVal,
        method: &str,
        args: Vec<ParseVal>,
    ) -> Result<ParseVal, Diagnostic> {
        let chain = match head {
            ParseVal::Chain(c) => c,
            other => {
                return Err(self.err(format!(
                    "method `.{}` requires a chain on the left-hand side, got {:?}",
                    method, other
                )))
            }
        };

        if method == "out" {
            return match args.len() {
                0 => Ok(ParseVal::Composition(chain.output())),
                1 => {
                    let buf = get_string(&args, 0, "out")?;
                    Ok(ParseVal::Composition(chain.output_to(buf)))
                }
                n => Err(self.err(format!(".out() expects 0 or 1 args, got {}", n))),
            };
        }

        let new_chain = match method {
            "rotate" => chain.rotate(
                get_value(&args, 0, "rotate", 0.0)?,
                get_value(&args, 1, "rotate", 0.0)?,
            ),
            "scale" => chain.scale(get_value(&args, 0, "scale", 1.0)?),
            "color" => chain.color(
                get_value(&args, 0, "color", 1.0)?,
                get_value(&args, 1, "color", 1.0)?,
                get_value(&args, 2, "color", 1.0)?,
                get_value(&args, 3, "color", 1.0)?,
            ),
            "brightness" => chain.brightness(get_value(&args, 0, "brightness", 0.0)?),
            "contrast" => chain.contrast(get_value(&args, 0, "contrast", 1.0)?),
            "invert" => chain.invert(get_value(&args, 0, "invert", 1.0)?),
            "scroll" => chain.scroll(
                get_value(&args, 0, "scroll", 0.5)?,
                get_value(&args, 1, "scroll", 0.5)?,
                get_value(&args, 2, "scroll", 0.0)?,
                get_value(&args, 3, "scroll", 0.0)?,
            ),
            "pixelate" => chain.pixelate(
                get_value(&args, 0, "pixelate", 20.0)?,
                get_value(&args, 1, "pixelate", 20.0)?,
            ),
            "blur" => chain.blur(get_value(&args, 0, "blur", 8.0)?),
            "repeat" => chain.repeat(
                get_value(&args, 0, "repeat", 3.0)?,
                get_value(&args, 1, "repeat", 3.0)?,
                get_value(&args, 2, "repeat", 0.0)?,
                get_value(&args, 3, "repeat", 0.0)?,
            ),
            "add" => chain.add(
                require_chain(&args, 0, "add")?,
                get_value(&args, 1, "add", 1.0)?,
            ),
            "mult" => chain.mult(
                require_chain(&args, 0, "mult")?,
                get_value(&args, 1, "mult", 1.0)?,
            ),
            "blend" => chain.blend(
                require_chain(&args, 0, "blend")?,
                get_value(&args, 1, "blend", 0.5)?,
            ),
            "modulate" => chain.modulate(
                require_chain(&args, 0, "modulate")?,
                get_value(&args, 1, "modulate", 0.1)?,
            ),
            "modulateScale" | "modulate_scale" => chain.modulate_scale(
                require_chain(&args, 0, "modulateScale")?,
                get_value(&args, 1, "modulateScale", 1.0)?,
                get_value(&args, 2, "modulateScale", 1.0)?,
            ),
            "modulateRotate" | "modulate_rotate" => chain.modulate_rotate(
                require_chain(&args, 0, "modulateRotate")?,
                get_value(&args, 1, "modulateRotate", 1.0)?,
                get_value(&args, 2, "modulateRotate", 0.0)?,
            ),
            "diff" => chain.diff(require_chain(&args, 0, "diff")?),
            "mask" => chain.mask(require_chain(&args, 0, "mask")?),
            "kaleid" => wrap_transform(chain, Transform::Kaleid {
                sides: get_u32(&args, 0, "kaleid", 4)?,
            }),
            "posterize" => wrap_transform(chain, Transform::Posterize {
                bins: get_value(&args, 0, "posterize", 3.0)?,
            }),
            "thresh" => wrap_transform(chain, Transform::Thresh {
                threshold: get_value(&args, 0, "thresh", 0.5)?,
                tolerance: get_value(&args, 1, "thresh", 0.04)?,
            }),
            "luma" => wrap_transform(chain, Transform::Luma {
                threshold: get_value(&args, 0, "luma", 0.5)?,
                tolerance: get_value(&args, 1, "luma", 0.1)?,
            }),
            "saturate" => wrap_transform(chain, Transform::Saturate {
                amount: get_value(&args, 0, "saturate", 2.0)?,
            }),
            "hue" => wrap_transform(chain, Transform::Hue {
                amount: get_value(&args, 0, "hue", 0.4)?,
            }),
            "modulateHue" | "modulate_hue" => wrap_combine(
                chain,
                require_chain(&args, 0, "modulateHue")?,
                Combinator::ModulateHue {
                    amount: get_value(&args, 1, "modulateHue", 1.0)?,
                },
            ),
            "modulatePixelate" | "modulate_pixelate" => wrap_combine(
                chain,
                require_chain(&args, 0, "modulatePixelate")?,
                Combinator::ModulatePixelate {
                    multiple: get_value(&args, 1, "modulatePixelate", 10.0)?,
                    offset: get_value(&args, 2, "modulatePixelate", 3.0)?,
                },
            ),
            other => return Err(self.err(format!("unknown method `.{}`", other))),
        };
        Ok(ParseVal::Chain(new_chain))
    }

    fn err(&self, message: String) -> Diagnostic {
        let t = self.peek();
        Diagnostic::ParseError {
            line: t.line,
            col: t.col,
            message,
        }
    }

    fn expect(&mut self, want: Tok, msg: &str) -> Result<(), Diagnostic> {
        if self.peek().kind == want {
            self.advance();
            Ok(())
        } else {
            Err(self.err(format!("{}: got {:?}", msg, self.peek().kind)))
        }
    }

    fn expect_ident(&mut self, msg: &str) -> Result<String, Diagnostic> {
        if let Tok::Ident(_) = &self.peek().kind {
            let tok = self.advance();
            if let Tok::Ident(n) = tok.kind {
                Ok(n)
            } else {
                unreachable!()
            }
        } else {
            Err(self.err(format!("expected {}, got {:?}", msg, self.peek().kind)))
        }
    }
}

/// Bare identifiers that resolve to a per-frame uniform without an
/// explicit `prop(...)` call. Today this is only `progress`, the
/// normalized 0..1 transition-window progress the transition pipeline
/// already writes into `CssProp("progress")` (see
/// `packages/wavelet/src/shader/transition.rs::pack_uniforms`).
///
/// Adding entries here is a deliberate vocabulary expansion — only do it
/// when the consumer side has matching plumbing. `t` / `width` / `height`
/// look obvious but `UniformRef` has no slots for them today, so binding
/// them implicitly would silently read zero.
fn implicit_uniform(name: &str) -> Option<Value> {
    match name {
        "progress" => Some(Value::Uniform(UniformRef::CssProp("progress".into()))),
        _ => None,
    }
}

fn get_value(args: &[ParseVal], idx: usize, fname: &str, default: f64) -> Result<Value, Diagnostic> {
    match args.get(idx) {
        None => Ok(Value::Const(default as f32)),
        Some(ParseVal::Number(n)) => Ok(Value::Const(*n as f32)),
        Some(ParseVal::DynValue(v)) => Ok(v.clone()),
        Some(other) => Err(Diagnostic::TypeMismatch {
            context: format!("argument {} of {}", idx + 1, fname),
            expected: "number or uniform reference (audio_rms(), prop(...), ...)".into(),
            actual: format!("{:?}", other),
        }),
    }
}

fn get_u32(args: &[ParseVal], idx: usize, fname: &str, default: u32) -> Result<u32, Diagnostic> {
    match args.get(idx) {
        None => Ok(default),
        Some(ParseVal::Number(n)) => {
            if *n < 0.0 || n.fract() != 0.0 {
                Err(Diagnostic::TypeMismatch {
                    context: format!("argument {} of {}", idx + 1, fname),
                    expected: "non-negative integer".into(),
                    actual: format!("{}", n),
                })
            } else {
                Ok(*n as u32)
            }
        }
        Some(other) => Err(Diagnostic::TypeMismatch {
            context: format!("argument {} of {}", idx + 1, fname),
            expected: "integer".into(),
            actual: format!("{:?}", other),
        }),
    }
}

fn get_string(args: &[ParseVal], idx: usize, fname: &str) -> Result<String, Diagnostic> {
    match args.get(idx) {
        Some(ParseVal::String(s)) => Ok(s.clone()),
        other => Err(Diagnostic::TypeMismatch {
            context: format!("argument {} of {}", idx + 1, fname),
            expected: "string".into(),
            actual: format!("{:?}", other),
        }),
    }
}

fn require_chain(args: &[ParseVal], idx: usize, fname: &str) -> Result<Chain, Diagnostic> {
    match args.get(idx) {
        Some(ParseVal::Chain(c)) => Ok(c.clone()),
        Some(other) => Err(Diagnostic::TypeMismatch {
            context: format!("argument {} of {}", idx + 1, fname),
            expected: "chain expression".into(),
            actual: format!("{:?}", other),
        }),
        None => Err(Diagnostic::InvalidComposition(format!(
            "{} requires a chain argument",
            fname
        ))),
    }
}

fn wrap_transform(chain: Chain, op: Transform) -> Chain {
    Chain(crate::ast::Node::Transform {
        input: Box::new(chain.0),
        op,
    })
}

fn wrap_combine(lhs: Chain, rhs: Chain, op: Combinator) -> Chain {
    Chain(crate::ast::Node::Combine {
        lhs: Box::new(lhs.0),
        rhs: Box::new(rhs.0),
        op,
    })
}
