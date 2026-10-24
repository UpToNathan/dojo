//! 07 Inline calculator. See .scratch/launcher-features/issues/07-calculator-row.md.
//!
//! [`evaluate`] answers a root-search query only when the whole query is clearly maths: an
//! arithmetic expression that contains an operator, a function call, a percentage or a unit
//! conversion. A bare number, a bare constant or ordinary words return `None`, so application
//! search is untouched. Division by zero, domain errors (`sqrt(-1)`, `ln(0)`), overflow and
//! malformed input also return `None` rather than an error row: a half-typed expression simply
//! shows nothing until it is complete.
//!
//! Supported:
//!
//! - `+ - * / % ^ **`, parentheses, unary minus, decimals, `1e3` exponents, thousands separators
//!   (`1,000`; commas inside a function call are argument separators), and `× ÷ −`.
//! - `sqrt abs sin cos tan ln log log2 exp round floor ceil min max` and the constants `pi e tau`.
//!   Trigonometry is in radians, `log` is base 10, `round(x, digits)` takes optional digits.
//! - Percentages: `20% of 150` is 30, `150 + 10%` is 165, `150 - 10%` is 135. Elsewhere `10%`
//!   is 0.1. `%` followed by an operand (`10 % 3`) is the remainder.
//! - Unit conversion `<expression> <unit> in|to|as|into <unit>` for length, mass, volume,
//!   temperature, data size (SI `kB`/`MB`, binary `KiB`/`MiB`, bits `Kb`/`Mb`), time, speed and
//!   area. Case matters only for data units: `MB` and `mb` are megabytes, `Mb` is megabits.
//!   Gallons, cups and fluid ounces are US measures and a year is 365.25 days. Time zones are not
//!   handled.
//!
//! Results keep up to 12 significant digits, so `0.1 + 0.2` is `0.3`.

/// A calculator result for the launcher's top row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// What the row shows, such as `= 1,234` or `= 3.10686 mi`. Large numbers are grouped.
    pub display: String,
    /// What Enter copies: the number without grouping, followed by the unit for a conversion.
    pub copy: String,
    /// A short row label: `Calculator`, `Percentage` or `<Family> conversion`.
    pub label: String,
}

const MAX_QUERY_CHARS: usize = 256;
const MAX_DEPTH: usize = 64;
const SIGNIFICANT_DIGITS: usize = 12;

/// Answers `query` when all of it is clearly maths, and `None` otherwise.
pub fn evaluate(query: &str) -> Option<Answer> {
    let query = query.trim().trim_end_matches('=').trim_end();
    if query.is_empty() || query.chars().count() > MAX_QUERY_CHARS {
        return None;
    }
    let lexed = lex(query)?;
    if lexed.tokens.is_empty() {
        return None;
    }
    let tree = parse(&lexed.tokens)?;
    match lexed.unit_start {
        Some(start) => convert(&tree, &query[start..]),
        None => {
            if is_trivial(&tree) {
                return None;
            }
            let value = eval(&tree)?;
            let plain = format_number(value)?;
            Some(Answer {
                display: format!("= {}", group_digits(&plain)),
                copy: plain,
                label: if has_percent(&tree) {
                    "Percentage"
                } else {
                    "Calculator"
                }
                .to_owned(),
            })
        }
    }
}

// ---------------------------------------------------------------- lexing

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Percent,
    LParen,
    RParen,
    Comma,
}

struct Lexed {
    tokens: Vec<Token>,
    /// Byte offset where the unit words of a conversion begin, if the query has any.
    unit_start: Option<usize>,
}

const CONSTANTS: [(&str, f64); 3] = [
    ("pi", std::f64::consts::PI),
    ("e", std::f64::consts::E),
    ("tau", std::f64::consts::TAU),
];

const FUNCTIONS: [&str; 12] = [
    "sqrt", "abs", "sin", "cos", "tan", "ln", "log", "log2", "exp", "round", "floor", "ceil",
];
const VARIADIC: [&str; 2] = ["min", "max"];

fn is_function(name: &str) -> bool {
    FUNCTIONS.contains(&name) || VARIADIC.contains(&name)
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '°' || c == 'µ'
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '°' | '²' | '³' | 'µ')
}

/// Splits the query into tokens. The first word that is not a function, constant or `of` ends
/// the expression: from there on the query is unit text, kept raw for [`convert`].
fn lex(query: &str) -> Option<Lexed> {
    let chars: Vec<(usize, char)> = query.char_indices().collect();
    let mut tokens = Vec::new();
    // For each open parenthesis: whether it opened a function call.
    let mut parens: Vec<bool> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (offset, c) = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '0'..='9' | '.' => {
                let in_call = parens.last().copied().unwrap_or(false);
                let (value, next) = lex_number(&chars, i, !in_call)?;
                tokens.push(Token::Number(value));
                i = next;
            }
            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            }
            '-' | '−' => {
                tokens.push(Token::Minus);
                i += 1;
            }
            '×' => {
                tokens.push(Token::Star);
                i += 1;
            }
            '*' => {
                if chars.get(i + 1).map(|&(_, next)| next) == Some('*') {
                    tokens.push(Token::Caret);
                    i += 2;
                } else {
                    tokens.push(Token::Star);
                    i += 1;
                }
            }
            '/' | '÷' => {
                tokens.push(Token::Slash);
                i += 1;
            }
            '^' => {
                tokens.push(Token::Caret);
                i += 1;
            }
            '%' => {
                tokens.push(Token::Percent);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            '(' => {
                let opens_call =
                    matches!(tokens.last(), Some(Token::Ident(name)) if is_function(name));
                parens.push(opens_call);
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                parens.pop()?;
                tokens.push(Token::RParen);
                i += 1;
            }
            c if is_ident_start(c) => {
                let mut end = i;
                while end < chars.len() && is_ident_continue(chars[end].1) {
                    end += 1;
                }
                let word: String = chars[i..end].iter().map(|&(_, c)| c).collect();
                let lower = word.to_lowercase();
                let calls = chars[end..]
                    .iter()
                    .find(|&&(_, c)| !c.is_whitespace())
                    .is_some_and(|&(_, c)| c == '(');
                let known = lower == "of"
                    || CONSTANTS.iter().any(|(name, _)| *name == lower)
                    || (calls && is_function(&lower));
                if !known {
                    if !parens.is_empty() {
                        return None;
                    }
                    return Some(Lexed {
                        tokens,
                        unit_start: Some(offset),
                    });
                }
                tokens.push(Token::Ident(lower));
                i = end;
            }
            _ => return None,
        }
    }
    if !parens.is_empty() {
        return None;
    }
    Some(Lexed {
        tokens,
        unit_start: None,
    })
}

/// Reads a number starting at `start`. With `grouping`, `1,234,567` is one number.
fn lex_number(chars: &[(usize, char)], start: usize, grouping: bool) -> Option<(f64, usize)> {
    let at = |index: usize| chars.get(index).map(|&(_, c)| c);
    let digits_from = |mut index: usize| {
        while at(index).is_some_and(|c| c.is_ascii_digit()) {
            index += 1;
        }
        index
    };
    let mut text = String::new();
    let mut i = start;
    let integer_end = digits_from(i);
    text.extend(chars[i..integer_end].iter().map(|&(_, c)| c));
    i = integer_end;
    // Thousands groups: one to three leading digits, then `,ddd` groups with no digit after.
    if grouping && (1..=3).contains(&(integer_end - start)) && at(i) == Some(',') {
        let mut probe = i;
        let mut groups = String::new();
        while at(probe) == Some(',')
            && (1..=3).all(|n| at(probe + n).is_some_and(|c| c.is_ascii_digit()))
            && !at(probe + 4).is_some_and(|c| c.is_ascii_digit())
        {
            groups.extend(chars[probe + 1..probe + 4].iter().map(|&(_, c)| c));
            probe += 4;
        }
        if !groups.is_empty() {
            text.push_str(&groups);
            i = probe;
        }
    }
    if at(i) == Some('.') {
        let fraction_end = digits_from(i + 1);
        text.push('.');
        text.extend(chars[i + 1..fraction_end].iter().map(|&(_, c)| c));
        i = fraction_end;
    }
    if text == "." || text.is_empty() {
        return None;
    }
    if matches!(at(i), Some('e' | 'E')) {
        let sign = usize::from(matches!(at(i + 1), Some('+' | '-')));
        let exponent_end = digits_from(i + 1 + sign);
        if exponent_end > i + 1 + sign {
            text.push('e');
            text.extend(chars[i + 1..exponent_end].iter().map(|&(_, c)| c));
            i = exponent_end;
        }
    }
    let value: f64 = text.parse().ok()?;
    value.is_finite().then_some((value, i))
}

// --------------------------------------------------------------- parsing

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Number(f64),
    Constant(f64),
    Negate(Box<Node>),
    /// `n%`: a fraction of whatever it is added to, subtracted from or taken of.
    Percent(Box<Node>),
    Binary(Op, Box<Node>, Box<Node>),
    Call(String, Vec<Node>),
}

struct Parser<'a> {
    tokens: &'a [Token],
    position: usize,
    depth: usize,
}

fn parse(tokens: &[Token]) -> Option<Node> {
    let mut parser = Parser {
        tokens,
        position: 0,
        depth: 0,
    };
    let node = parser.additive()?;
    (parser.position == tokens.len()).then_some(node)
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn enter(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MAX_DEPTH).then_some(())
    }

    fn additive(&mut self) -> Option<Node> {
        let mut left = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => Op::Add,
                Some(Token::Minus) => Op::Sub,
                _ => return Some(left),
            };
            self.position += 1;
            let right = self.multiplicative()?;
            left = Node::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn multiplicative(&mut self) -> Option<Node> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => Op::Mul,
                Some(Token::Slash) => Op::Div,
                // A `%` the postfix rule left alone is a remainder.
                Some(Token::Percent) => Op::Rem,
                Some(Token::Ident(word)) if word == "of" => {
                    if !matches!(left, Node::Percent(_)) {
                        return None;
                    }
                    Op::Mul
                }
                _ => return Some(left),
            };
            self.position += 1;
            let right = self.unary()?;
            left = Node::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn unary(&mut self) -> Option<Node> {
        self.enter()?;
        let node = if self.eat(&Token::Minus) {
            self.unary().map(|inner| Node::Negate(Box::new(inner)))
        } else if self.eat(&Token::Plus) {
            self.unary()
        } else {
            self.power()
        };
        self.depth -= 1;
        node
    }

    /// `^` is right-associative and binds tighter than unary minus: `-2^2` is -4 and `2^3^2` is 512.
    fn power(&mut self) -> Option<Node> {
        let base = self.postfix()?;
        if self.eat(&Token::Caret) {
            let exponent = self.unary()?;
            return Some(Node::Binary(Op::Pow, Box::new(base), Box::new(exponent)));
        }
        Some(base)
    }

    fn postfix(&mut self) -> Option<Node> {
        let mut node = self.primary()?;
        // `%` is a percentage unless an operand follows it, which makes it a remainder.
        if self.peek() == Some(&Token::Percent) && !self.operand_follows(self.position + 1) {
            self.position += 1;
            node = Node::Percent(Box::new(node));
        }
        Some(node)
    }

    fn operand_follows(&self, index: usize) -> bool {
        match self.tokens.get(index) {
            Some(Token::Number(_) | Token::LParen) => true,
            Some(Token::Ident(word)) => word != "of",
            _ => false,
        }
    }

    fn primary(&mut self) -> Option<Node> {
        self.enter()?;
        let node = self.primary_inner();
        self.depth -= 1;
        node
    }

    fn primary_inner(&mut self) -> Option<Node> {
        match self.tokens.get(self.position)?.clone() {
            Token::Number(value) => {
                self.position += 1;
                Some(Node::Number(value))
            }
            Token::LParen => {
                self.position += 1;
                let inner = self.additive()?;
                self.eat(&Token::RParen).then_some(inner)
            }
            Token::Ident(name) => {
                self.position += 1;
                if let Some((_, value)) = CONSTANTS.iter().find(|(constant, _)| *constant == name) {
                    return Some(Node::Constant(*value));
                }
                if !is_function(&name) || !self.eat(&Token::LParen) {
                    return None;
                }
                let mut arguments = vec![self.additive()?];
                while self.eat(&Token::Comma) {
                    arguments.push(self.additive()?);
                }
                if !self.eat(&Token::RParen) {
                    return None;
                }
                let valid = match name.as_str() {
                    "round" => (1..=2).contains(&arguments.len()),
                    "min" | "max" => true,
                    _ => arguments.len() == 1,
                };
                valid.then_some(Node::Call(name, arguments))
            }
            _ => None,
        }
    }
}

/// A lone number, constant or percentage is not worth an answer row.
fn is_trivial(node: &Node) -> bool {
    match node {
        Node::Number(_) | Node::Constant(_) => true,
        Node::Negate(inner) | Node::Percent(inner) => is_trivial(inner),
        Node::Binary(..) | Node::Call(..) => false,
    }
}

fn has_percent(node: &Node) -> bool {
    match node {
        Node::Percent(_) => true,
        Node::Number(_) | Node::Constant(_) => false,
        Node::Negate(inner) => has_percent(inner),
        Node::Binary(_, left, right) => has_percent(left) || has_percent(right),
        Node::Call(_, arguments) => arguments.iter().any(has_percent),
    }
}

// ------------------------------------------------------------ evaluation

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

/// Sums that cancel to rounding noise (`0.1 + 0.2 - 0.3`) become exactly zero.
fn settle(result: f64, left: f64, right: f64) -> f64 {
    if result.abs() < 1e-12 * left.abs().max(right.abs()) {
        0.0
    } else {
        result
    }
}

fn eval(node: &Node) -> Option<f64> {
    match node {
        Node::Number(value) | Node::Constant(value) => Some(*value),
        Node::Negate(inner) => Some(-eval(inner)?),
        Node::Percent(inner) => finite(eval(inner)? / 100.0),
        Node::Binary(op, left_node, right_node) => {
            let left = eval(left_node)?;
            // `a + n%` and `a - n%` are relative to `a`.
            let relative = match (&**left_node, &**right_node, op) {
                (Node::Percent(_), _, _) => None,
                (_, Node::Percent(_), Op::Add | Op::Sub) => Some(left),
                _ => None,
            };
            let right = match (relative, &**right_node) {
                (Some(base), Node::Percent(_)) => base * eval(right_node)?,
                _ => eval(right_node)?,
            };
            let result = match op {
                Op::Add => settle(left + right, left, right),
                Op::Sub => settle(left - right, left, right),
                Op::Mul => left * right,
                Op::Div | Op::Rem if right == 0.0 => return None,
                Op::Div => left / right,
                Op::Rem => left % right,
                Op::Pow => left.powf(right),
            };
            finite(result)
        }
        Node::Call(name, arguments) => {
            let values = arguments.iter().map(eval).collect::<Option<Vec<_>>>()?;
            finite(call(name, &values)?)
        }
    }
}

fn call(name: &str, values: &[f64]) -> Option<f64> {
    let x = values[0];
    Some(match name {
        "sqrt" if x >= 0.0 => x.sqrt(),
        "abs" => x.abs(),
        "sin" => snap(x.sin()),
        "cos" => snap(x.cos()),
        "tan" => snap(x.tan()),
        "ln" if x > 0.0 => x.ln(),
        "log" if x > 0.0 => x.log10(),
        "log2" if x > 0.0 => x.log2(),
        "exp" => x.exp(),
        "floor" => x.floor(),
        "ceil" => x.ceil(),
        "round" => match values.get(1) {
            None => x.round(),
            Some(&digits) => {
                if digits.fract() != 0.0 || !(0.0..=15.0).contains(&digits) {
                    return None;
                }
                let scale = 10f64.powi(digits as i32);
                (x * scale).round() / scale
            }
        },
        "min" => values.iter().copied().fold(f64::INFINITY, f64::min),
        "max" => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        _ => return None,
    })
}

/// `sin(pi)` is 1.2e-16, which is zero for every purpose of this calculator.
fn snap(value: f64) -> f64 {
    if value.abs() < 1e-15 { 0.0 } else { value }
}

// ------------------------------------------------------------ formatting

/// Formats with up to 12 significant digits, no trailing zeros and no exponent for ordinary
/// magnitudes (`1e-7` up to just below `1e16`).
fn format_number(value: f64) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    if value == 0.0 {
        return Some("0".to_owned());
    }
    let scientific = format!("{:.*e}", SIGNIFICANT_DIGITS - 1, value.abs());
    let (mantissa, exponent) = scientific.split_once('e')?;
    let exponent: i32 = exponent.parse().ok()?;
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let sign = if value < 0.0 { "-" } else { "" };
    let body = if (-7..16).contains(&exponent) {
        if exponent >= 0 {
            let integer_len = exponent as usize + 1;
            if digits.len() <= integer_len {
                format!("{digits}{}", "0".repeat(integer_len - digits.len()))
            } else {
                format!("{}.{}", &digits[..integer_len], &digits[integer_len..])
            }
        } else {
            format!("0.{}{digits}", "0".repeat((-exponent - 1) as usize))
        }
    } else if digits.len() == 1 {
        format!("{digits}e{exponent}")
    } else {
        format!("{}.{}e{exponent}", &digits[..1], &digits[1..])
    };
    Some(format!("{sign}{body}"))
}

/// Adds commas to the integer part of a plain number: `1234567.5` becomes `1,234,567.5`.
fn group_digits(plain: &str) -> String {
    if plain.contains('e') {
        return plain.to_owned();
    }
    let (sign, unsigned) = plain
        .strip_prefix('-')
        .map_or(("", plain), |rest| ("-", rest));
    let (integer, fraction) = unsigned
        .split_once('.')
        .map_or((unsigned, None), |(integer, fraction)| {
            (integer, Some(fraction))
        });
    let mut grouped = String::new();
    for (index, digit) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    match fraction {
        Some(fraction) => format!("{sign}{grouped}.{fraction}"),
        None => format!("{sign}{grouped}"),
    }
}

// ------------------------------------------------------------------ units

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Length,
    Mass,
    Volume,
    Temperature,
    Data,
    Time,
    Speed,
    Area,
}

impl Family {
    fn name(self) -> &'static str {
        match self {
            Self::Length => "Length",
            Self::Mass => "Mass",
            Self::Volume => "Volume",
            Self::Temperature => "Temperature",
            Self::Data => "Data size",
            Self::Time => "Time",
            Self::Speed => "Speed",
            Self::Area => "Area",
        }
    }
}

/// A unit relates to its family's base unit by `base = (value + offset) * scale`. Only the
/// temperature scales have an offset.
struct Unit {
    family: Family,
    symbol: &'static str,
    scale: f64,
    offset: f64,
    /// Matched case-insensitively (after dropping a plural `s` if nothing matches).
    names: &'static [&'static str],
    /// Matched case-sensitively, and before `names`.
    exact: &'static [&'static str],
}

const fn unit(
    family: Family,
    symbol: &'static str,
    scale: f64,
    names: &'static [&'static str],
) -> Unit {
    Unit {
        family,
        symbol,
        scale,
        offset: 0.0,
        names,
        exact: &[],
    }
}

const fn exact_unit(
    family: Family,
    symbol: &'static str,
    scale: f64,
    names: &'static [&'static str],
    exact: &'static [&'static str],
) -> Unit {
    Unit {
        family,
        symbol,
        scale,
        offset: 0.0,
        names,
        exact,
    }
}

const fn temperature(
    symbol: &'static str,
    scale: f64,
    offset: f64,
    names: &'static [&'static str],
) -> Unit {
    Unit {
        family: Family::Temperature,
        symbol,
        scale,
        offset,
        names,
        exact: &[],
    }
}

use Family::{Area, Data, Length, Mass, Speed, Time, Volume};

/// Base units: metre, kilogram, litre, kelvin, byte, second, metre per second, square metre.
static UNITS: &[Unit] = &[
    unit(Length, "nm", 1e-9, &["nm", "nanometer", "nanometre"]),
    unit(
        Length,
        "µm",
        1e-6,
        &["um", "µm", "micron", "micrometer", "micrometre"],
    ),
    unit(Length, "mm", 1e-3, &["mm", "millimeter", "millimetre"]),
    unit(Length, "cm", 1e-2, &["cm", "centimeter", "centimetre"]),
    unit(Length, "m", 1.0, &["m", "meter", "metre"]),
    unit(Length, "km", 1e3, &["km", "kilometer", "kilometre"]),
    unit(Length, "in", 0.0254, &["in", "inch", "inches"]),
    unit(Length, "ft", 0.3048, &["ft", "foot", "feet"]),
    unit(Length, "yd", 0.9144, &["yd", "yard"]),
    unit(Length, "mi", 1609.344, &["mi", "mile"]),
    unit(Length, "nmi", 1852.0, &["nmi", "nautical mile"]),
    unit(Mass, "mg", 1e-6, &["mg", "milligram"]),
    unit(Mass, "g", 1e-3, &["g", "gram", "gramme"]),
    unit(Mass, "kg", 1.0, &["kg", "kilogram", "kilo", "kilogramme"]),
    unit(Mass, "t", 1e3, &["t", "tonne", "metric ton"]),
    unit(Mass, "oz", 0.028_349_523_125, &["oz", "ounce"]),
    unit(Mass, "lb", 0.453_592_37, &["lb", "lbs", "pound"]),
    unit(Mass, "st", 6.350_293_18, &["st", "stone"]),
    unit(
        Volume,
        "ml",
        1e-3,
        &["ml", "milliliter", "millilitre", "cc", "cm3", "cm³"],
    ),
    unit(Volume, "cl", 1e-2, &["cl", "centiliter", "centilitre"]),
    unit(Volume, "dl", 0.1, &["dl", "deciliter", "decilitre"]),
    unit(Volume, "l", 1.0, &["l", "liter", "litre"]),
    unit(
        Volume,
        "m³",
        1e3,
        &["m3", "m³", "cubic meter", "cubic metre"],
    ),
    unit(Volume, "tsp", 0.004_928_921_593_75, &["tsp", "teaspoon"]),
    unit(
        Volume,
        "tbsp",
        0.014_786_764_781_25,
        &["tbsp", "tablespoon"],
    ),
    unit(
        Volume,
        "fl oz",
        0.029_573_529_562_5,
        &["fl oz", "floz", "fluid ounce"],
    ),
    unit(Volume, "cup", 0.236_588_236_5, &["cup"]),
    unit(Volume, "pt", 0.473_176_473, &["pt", "pint"]),
    unit(Volume, "qt", 0.946_352_946, &["qt", "quart"]),
    unit(Volume, "gal", 3.785_411_784, &["gal", "gallon"]),
    temperature(
        "°C",
        1.0,
        273.15,
        &[
            "c",
            "°c",
            "celsius",
            "centigrade",
            "deg c",
            "degc",
            "degrees celsius",
            "degree celsius",
            "degrees c",
        ],
    ),
    temperature(
        "°F",
        5.0 / 9.0,
        459.67,
        &[
            "f",
            "°f",
            "fahrenheit",
            "deg f",
            "degf",
            "degrees fahrenheit",
            "degree fahrenheit",
            "degrees f",
        ],
    ),
    temperature("K", 1.0, 0.0, &["k", "kelvin"]),
    exact_unit(Data, "bit", 0.125, &["bit"], &["b"]),
    exact_unit(Data, "B", 1.0, &["byte"], &["B"]),
    unit(Data, "kB", 1e3, &["kb", "kilobyte"]),
    unit(Data, "MB", 1e6, &["mb", "megabyte"]),
    unit(Data, "GB", 1e9, &["gb", "gigabyte"]),
    unit(Data, "TB", 1e12, &["tb", "terabyte"]),
    unit(Data, "PB", 1e15, &["pb", "petabyte"]),
    unit(Data, "KiB", 1024.0, &["kib", "kibibyte"]),
    unit(Data, "MiB", 1_048_576.0, &["mib", "mebibyte"]),
    unit(Data, "GiB", 1_073_741_824.0, &["gib", "gibibyte"]),
    unit(Data, "TiB", 1_099_511_627_776.0, &["tib", "tebibyte"]),
    unit(Data, "PiB", 1_125_899_906_842_624.0, &["pib", "pebibyte"]),
    exact_unit(Data, "Kb", 125.0, &["kbit", "kilobit"], &["Kb"]),
    exact_unit(Data, "Mb", 125_000.0, &["mbit", "megabit"], &["Mb"]),
    exact_unit(Data, "Gb", 125_000_000.0, &["gbit", "gigabit"], &["Gb"]),
    exact_unit(Data, "Tb", 125_000_000_000.0, &["tbit", "terabit"], &["Tb"]),
    unit(Time, "ns", 1e-9, &["ns", "nanosecond"]),
    unit(Time, "µs", 1e-6, &["us", "µs", "microsecond"]),
    unit(Time, "ms", 1e-3, &["ms", "millisecond"]),
    unit(Time, "s", 1.0, &["s", "sec", "second"]),
    unit(Time, "min", 60.0, &["min", "minute"]),
    unit(Time, "h", 3600.0, &["h", "hr", "hour"]),
    unit(Time, "d", 86_400.0, &["d", "day"]),
    unit(Time, "wk", 604_800.0, &["w", "wk", "week"]),
    unit(Time, "yr", 31_557_600.0, &["y", "yr", "year"]),
    unit(
        Speed,
        "m/s",
        1.0,
        &["m/s", "mps", "meter per second", "metre per second"],
    ),
    unit(
        Speed,
        "km/h",
        1.0 / 3.6,
        &[
            "km/h",
            "kph",
            "kmh",
            "kmph",
            "kilometer per hour",
            "kilometre per hour",
        ],
    ),
    unit(Speed, "mph", 0.447_04, &["mph", "mi/h", "mile per hour"]),
    unit(Speed, "kn", 1852.0 / 3600.0, &["kn", "kt", "knot"]),
    unit(
        Speed,
        "ft/s",
        0.3048,
        &["ft/s", "fps", "foot per second", "feet per second"],
    ),
    unit(
        Area,
        "mm²",
        1e-6,
        &["mm2", "mm²", "square millimeter", "square millimetre"],
    ),
    unit(
        Area,
        "cm²",
        1e-4,
        &["cm2", "cm²", "square centimeter", "square centimetre"],
    ),
    unit(
        Area,
        "m²",
        1.0,
        &["m2", "m²", "sqm", "sq m", "square meter", "square metre"],
    ),
    unit(
        Area,
        "km²",
        1e6,
        &[
            "km2",
            "km²",
            "sq km",
            "square kilometer",
            "square kilometre",
        ],
    ),
    unit(Area, "ha", 1e4, &["ha", "hectare"]),
    unit(
        Area,
        "in²",
        0.000_645_16,
        &["in2", "in²", "sq in", "square inch", "square inches"],
    ),
    unit(
        Area,
        "ft²",
        0.092_903_04,
        &["ft2", "ft²", "sqft", "sq ft", "square foot", "square feet"],
    ),
    unit(
        Area,
        "yd²",
        0.836_127_36,
        &["yd2", "yd²", "sq yd", "square yard"],
    ),
    unit(
        Area,
        "mi²",
        2_589_988.110_336,
        &["mi2", "mi²", "sq mi", "square mile"],
    ),
    unit(Area, "ac", 4046.856_422_4, &["ac", "acre"]),
];

/// Looks a unit up by what the user typed: `km`, `Square Feet`, `km / h`, `kilometres per hour`.
fn find_unit(text: &str) -> Option<&'static Unit> {
    let joined = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" / ", "/")
        .replace(" /", "/")
        .replace("/ ", "/");
    if let Some(unit) = UNITS
        .iter()
        .find(|unit| unit.exact.contains(&joined.as_str()))
    {
        return Some(unit);
    }
    let lower = joined.to_lowercase();
    // Plurals: `miles`, `square feet` is listed, `kilometres per hour` pluralises the head only.
    let mut candidates = vec![lower.clone()];
    if let Some(singular) = lower.strip_suffix('s').filter(|rest| !rest.is_empty()) {
        candidates.push(singular.to_owned());
    }
    if let Some((head, tail)) = lower.split_once(" per ") {
        let head = head.strip_suffix('s').unwrap_or(head);
        candidates.push(format!("{head} per {tail}"));
    }
    candidates.iter().find_map(|candidate| {
        UNITS
            .iter()
            .find(|unit| unit.names.contains(&candidate.as_str()))
    })
}

const CONVERT_WORDS: [&str; 4] = ["in", "to", "as", "into"];

/// Handles `<expression> <unit> in <unit>`, where `unit_text` is everything after the expression.
fn convert(tree: &Node, unit_text: &str) -> Option<Answer> {
    let words: Vec<&str> = unit_text.split_whitespace().collect();
    for split in 1..words.len().saturating_sub(1) {
        if !CONVERT_WORDS.contains(&words[split].to_lowercase().as_str()) {
            continue;
        }
        let (Some(from), Some(to)) = (
            find_unit(&words[..split].join(" ")),
            find_unit(&words[split + 1..].join(" ")),
        ) else {
            continue;
        };
        if from.family != to.family {
            return None;
        }
        let value = eval(tree)?;
        let base = (value + from.offset) * from.scale;
        let converted = finite(base / to.scale - to.offset)?;
        // Temperature offsets leave rounding noise where the answer is zero (32 F is 0 C).
        let noise = 1e-12 * (base / to.scale).abs().max(to.offset.abs());
        let converted = if converted.abs() < noise {
            0.0
        } else {
            converted
        };
        let plain = format_number(converted)?;
        return Some(Answer {
            display: format!("= {} {}", group_digits(&plain), to.symbol),
            copy: format!("{plain} {}", to.symbol),
            label: format!("{} conversion", from.family.name()),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(query: &str) -> String {
        evaluate(query)
            .unwrap_or_else(|| panic!("{query:?} should be maths"))
            .display
    }

    fn copied(query: &str) -> String {
        evaluate(query)
            .unwrap_or_else(|| panic!("{query:?} should be maths"))
            .copy
    }

    #[test]
    fn answers_carry_display_copy_and_label() {
        let answer = evaluate("6 * 7").unwrap();
        assert_eq!(answer.display, "= 42");
        assert_eq!(answer.copy, "42");
        assert_eq!(answer.label, "Calculator");
    }

    #[test]
    fn follows_operator_precedence() {
        assert_eq!(copied("2 + 3 * 4"), "14");
        assert_eq!(copied("(2 + 3) * 4"), "20");
        assert_eq!(copied("10 - 4 - 3"), "3");
        assert_eq!(copied("100 / 10 / 5"), "2");
        assert_eq!(copied("2 * 3 % 4"), "2");
        assert_eq!(copied("2 + 3 ^ 2 * 2"), "20");
    }

    #[test]
    fn power_is_right_associative_and_binds_tighter_than_minus() {
        assert_eq!(copied("2^3^2"), "512");
        assert_eq!(copied("2 ** 3 ** 2"), "512");
        assert_eq!(copied("-2^2"), "-4");
        assert_eq!(copied("(-2)^2"), "4");
        assert_eq!(copied("2^-2"), "0.25");
        assert_eq!(copied("4^0.5"), "2");
    }

    #[test]
    fn handles_unary_minus_and_nested_parentheses() {
        assert_eq!(copied("-(3 + 4)"), "-7");
        assert_eq!(copied("--3 + 1"), "4");
        assert_eq!(copied("3 - -2"), "5");
        assert_eq!(copied("((1 + 2) * (3 + (4 - 1)))"), "18");
        assert_eq!(copied("+5 + 1"), "6");
    }

    #[test]
    fn accepts_unicode_operators() {
        assert_eq!(copied("6 × 7"), "42");
        assert_eq!(copied("84 ÷ 2"), "42");
        assert_eq!(copied("50 − 8"), "42");
    }

    #[test]
    fn remainder_needs_an_operand_after_the_percent_sign() {
        assert_eq!(copied("10 % 3"), "1");
        assert_eq!(copied("10 % (2 + 1)"), "1");
        assert_eq!(copied("-7 % 3"), "-1");
        assert_eq!(copied("5.5 % 2"), "1.5");
    }

    #[test]
    fn decimals_exponents_and_thousands_separators() {
        assert_eq!(copied("1.5 + .25"), "1.75");
        assert_eq!(copied("1,000 + 1"), "1001");
        assert_eq!(copied("1,234,567 * 2"), "2469134");
        assert_eq!(copied("1e3 + 1"), "1001");
        assert_eq!(copied("2.5e-3 * 2"), "0.005");
        assert_eq!(copied("1E2 / 4"), "25");
    }

    #[test]
    fn commas_in_calls_separate_arguments() {
        assert_eq!(copied("max(1,234)"), "234");
        assert_eq!(copied("min(1, 2, 3)"), "1");
        assert_eq!(copied("max(1,000, 5)"), "5");
    }

    #[test]
    fn malformed_thousands_groups_are_rejected() {
        assert_eq!(evaluate("1,00 + 1"), None);
        assert_eq!(evaluate("1,2345 + 1"), None);
        assert_eq!(evaluate("1,5 + 1"), None);
    }

    #[test]
    fn functions_and_constants() {
        assert_eq!(copied("sqrt(16)"), "4");
        assert_eq!(copied("abs(-3.5)"), "3.5");
        assert_eq!(copied("sin(0)"), "0");
        assert_eq!(copied("cos(0)"), "1");
        assert_eq!(copied("tan(0)"), "0");
        assert_eq!(copied("ln(e)"), "1");
        assert_eq!(copied("log(1000)"), "3");
        assert_eq!(copied("log2(8)"), "3");
        assert_eq!(copied("exp(0)"), "1");
        assert_eq!(copied("round(2.5)"), "3");
        assert_eq!(copied("round(-2.5)"), "-3");
        assert_eq!(copied("round(3.14159, 2)"), "3.14");
        assert_eq!(copied("floor(-1.5)"), "-2");
        assert_eq!(copied("ceil(1.2)"), "2");
        assert_eq!(copied("min(4, 2, 9)"), "2");
        assert_eq!(copied("max(4, 2, 9)"), "9");
        assert_eq!(copied("pi * 2"), "6.28318530718");
        assert_eq!(copied("tau / 2"), "3.14159265359");
        assert_eq!(copied("e ^ 2"), "7.38905609893");
        assert_eq!(copied("SQRT(9) + PI - pi"), "3");
    }

    #[test]
    fn trig_noise_snaps_to_zero() {
        assert_eq!(copied("sin(pi)"), "0");
        assert_eq!(copied("cos(pi / 2)"), "0");
        assert_eq!(copied("sin(pi / 6) * 2"), "1");
    }

    #[test]
    fn function_arity_is_checked() {
        assert_eq!(evaluate("sqrt(1, 2)"), None);
        assert_eq!(evaluate("sqrt()"), None);
        assert_eq!(evaluate("round(1, 2, 3)"), None);
        assert_eq!(evaluate("round(1, 2.5)"), None);
        assert_eq!(evaluate("max()"), None);
        assert_eq!(evaluate("sqrt 16"), None);
        assert_eq!(evaluate("unknown(3)"), None);
    }

    #[test]
    fn percentages() {
        assert_eq!(shown("20% of 150"), "= 30");
        assert_eq!(evaluate("20% of 150").unwrap().label, "Percentage");
        assert_eq!(copied("150 + 10%"), "165");
        assert_eq!(copied("150 - 10%"), "135");
        assert_eq!(copied("150 * 10%"), "15");
        assert_eq!(copied("10% * 50"), "5");
        assert_eq!(copied("200 + 50%"), "300");
        assert_eq!(copied("12.5% of 80"), "10");
        assert_eq!(copied("(10 + 10)% of 50"), "10");
        assert_eq!(copied("20% of 150 + 5"), "35");
        assert_eq!(copied("50% of 2^3"), "4");
        assert_eq!(copied("100 + 10% + 10%"), "121");
    }

    #[test]
    fn of_needs_a_percentage_on_its_left() {
        assert_eq!(evaluate("5 of 10"), None);
        assert_eq!(evaluate("20% of"), None);
    }

    #[test]
    fn no_float_noise() {
        assert_eq!(copied("0.1 + 0.2"), "0.3");
        assert_eq!(copied("0.1 + 0.2 - 0.3"), "0");
        assert_eq!(copied("1 / 3"), "0.333333333333");
        assert_eq!(copied("2 / 3"), "0.666666666667");
        assert_eq!(copied("0.7 * 3"), "2.1");
        assert_eq!(copied("1 - 0.9"), "0.1");
    }

    #[test]
    fn formats_magnitudes_without_exponents_until_they_are_extreme() {
        assert_eq!(copied("123456789 * 1000"), "123456789000");
        assert_eq!(copied("10 ^ 15"), "1000000000000000");
        assert_eq!(copied("10 ^ 16"), "1e16");
        assert_eq!(copied("1.5 * 10 ^ 20"), "1.5e20");
        assert_eq!(copied("0.0000001 * 1"), "0.0000001");
        assert_eq!(copied("1 / 10 ^ 8"), "1e-8");
        assert_eq!(copied("-1 / 10 ^ 9"), "-1e-9");
        assert_eq!(copied("0 * 5"), "0");
        assert_eq!(copied("-0 * 5"), "0");
    }

    #[test]
    fn display_groups_thousands_and_copy_does_not() {
        assert_eq!(shown("1000 * 1000"), "= 1,000,000");
        assert_eq!(copied("1000 * 1000"), "1000000");
        assert_eq!(shown("1234.5 * 10"), "= 12,345");
        assert_eq!(shown("-1234567.25 + 0"), "= -1,234,567.25");
        assert_eq!(shown("999 + 0"), "= 999");
        assert_eq!(shown("1.5 * 10 ^ 20"), "= 1.5e20");
    }

    #[test]
    fn bare_numbers_constants_and_words_are_not_maths() {
        for query in [
            "",
            " ",
            "42",
            "-5",
            "(5)",
            "3.14",
            "pi",
            "e",
            "50%",
            "firefox",
            "firefox 2",
            "7zip",
            "gimp 2.10",
            "2 firefox",
            "firefox + 2",
            "x264",
            "kitty",
            "vim",
            "of",
            "in",
            "to",
            "1,000",
            "tau",
            "5 km",
            "km",
            "=",
        ] {
            assert_eq!(evaluate(query), None, "{query:?}");
        }
    }

    #[test]
    fn malformed_input_is_rejected() {
        for query in [
            "2 +",
            "+",
            "* 3",
            "(1 + 2",
            "1 + 2)",
            "()",
            "2 3",
            "2 pi",
            "2(3)",
            "1..2",
            "1.2.3",
            ".",
            "1 + * 2",
            "$5 + 2",
            "2 + 2 + abc",
            "5 +- ",
            "^2",
            "2 ^",
            "1 2 + 3",
            "sin",
            "sin + 1",
            "1 + (",
            ") (",
        ] {
            assert_eq!(evaluate(query), None, "{query:?}");
        }
    }

    #[test]
    fn division_by_zero_domain_errors_and_overflow_are_rejected() {
        for query in [
            "1 / 0",
            "1 / (2 - 2)",
            "5 % 0",
            "sqrt(-1)",
            "ln(0)",
            "ln(-1)",
            "log(0)",
            "(-8) ^ 0.5",
            "0 ^ -1",
            "10 ^ 400",
            "9 ^ 9 ^ 9",
            "exp(1000)",
            "1e400 + 1",
            "round(1, 20)",
        ] {
            assert_eq!(evaluate(query), None, "{query:?}");
        }
    }

    #[test]
    fn deeply_nested_input_is_rejected_without_overflowing_the_stack() {
        let nested = format!("{}1{} + 1", "(".repeat(200), ")".repeat(200));
        assert_eq!(evaluate(&nested), None);
        let negations = format!("{}1 + 1", "-".repeat(200));
        assert_eq!(evaluate(&negations), None);
        assert_eq!(evaluate(&"1+".repeat(300)), None);
        let fine = format!("{}1{} + 1", "(".repeat(20), ")".repeat(20));
        assert_eq!(copied(&fine), "2");
    }

    #[test]
    fn a_trailing_equals_sign_is_ignored() {
        assert_eq!(copied("2+2="), "4");
        assert_eq!(copied("2+2 ="), "4");
    }

    #[test]
    fn converts_length() {
        assert_eq!(copied("5 km in miles"), "3.10685596119 mi");
        assert_eq!(shown("5 km in miles"), "= 3.10685596119 mi");
        assert_eq!(copied("1 mi to km"), "1.609344 km");
        assert_eq!(copied("12 inches to cm"), "30.48 cm");
        assert_eq!(copied("6 ft to m"), "1.8288 m");
        assert_eq!(copied("100 cm in m"), "1 m");
        assert_eq!(copied("1 nmi to km"), "1.852 km");
        assert_eq!(copied("3 feet to inches"), "36 in");
        assert_eq!(
            evaluate("5 km to miles").unwrap().label,
            "Length conversion"
        );
    }

    #[test]
    fn inch_is_both_a_unit_and_the_conversion_word() {
        assert_eq!(copied("1 in to cm"), "2.54 cm");
        assert_eq!(copied("2.54 cm in in"), "1 in");
        assert_eq!(copied("1 in in cm"), "2.54 cm");
    }

    #[test]
    fn converts_mass() {
        assert_eq!(copied("10 kg to lb"), "22.0462262185 lb");
        assert_eq!(copied("1 lb to g"), "453.59237 g");
        assert_eq!(copied("16 oz to lb"), "1 lb");
        assert_eq!(copied("1 tonne to kg"), "1000 kg");
        assert_eq!(copied("500 g in kg"), "0.5 kg");
        assert_eq!(copied("14 pounds to stone"), "1 st");
    }

    #[test]
    fn converts_volume() {
        assert_eq!(copied("1 gal to l"), "3.785411784 l");
        assert_eq!(copied("1 l to ml"), "1000 ml");
        assert_eq!(copied("2 cups to ml"), "473.176473 ml");
        assert_eq!(copied("1 tbsp to tsp"), "3 tsp");
        assert_eq!(copied("1 m3 to l"), "1000 l");
        assert_eq!(copied("8 fl oz to cup"), "1 cup");
        assert_eq!(copied("4 qt to gal"), "1 gal");
    }

    #[test]
    fn converts_temperature() {
        assert_eq!(copied("72 f to c"), "22.2222222222 °C");
        assert_eq!(copied("100 c to f"), "212 °F");
        assert_eq!(copied("0 c to k"), "273.15 K");
        assert_eq!(copied("0 k to c"), "-273.15 °C");
        assert_eq!(copied("-40 c to f"), "-40 °F");
        assert_eq!(copied("32 °F to °C"), "0 °C");
        assert_eq!(copied("37 celsius in fahrenheit"), "98.6 °F");
        assert_eq!(shown("72 f to c"), "= 22.2222222222 °C");
    }

    #[test]
    fn converts_data_sizes_in_si_binary_and_bits() {
        assert_eq!(copied("1.5 gb to mb"), "1500 MB");
        assert_eq!(copied("1 GiB to MiB"), "1024 MiB");
        assert_eq!(copied("1 GB to GiB"), "0.931322574615 GiB");
        assert_eq!(copied("1 KiB to bytes"), "1024 B");
        assert_eq!(copied("1 MB to kB"), "1000 kB");
        assert_eq!(copied("1 B to bit"), "8 bit");
        assert_eq!(copied("8 Mb to MB"), "1 MB");
        assert_eq!(copied("1 MB to Mb"), "8 Mb");
        assert_eq!(copied("100 Mbit to MB"), "12.5 MB");
        assert_eq!(copied("2 TB to GB"), "2000 GB");
        assert_eq!(copied("1 tib to gib"), "1024 GiB");
        assert_eq!(copied("1024 megabytes to gigabytes"), "1.024 GB");
    }

    #[test]
    fn converts_time() {
        assert_eq!(copied("2 h to min"), "120 min");
        assert_eq!(copied("90 min to h"), "1.5 h");
        assert_eq!(copied("1 day to hours"), "24 h");
        assert_eq!(copied("1 week in days"), "7 d");
        assert_eq!(copied("1500 ms to s"), "1.5 s");
        assert_eq!(copied("1 year to days"), "365.25 d");
        assert_eq!(copied("3600 seconds to hours"), "1 h");
        assert_eq!(copied("min(2, 3) min to s"), "120 s");
    }

    #[test]
    fn converts_speed() {
        assert_eq!(copied("100 km/h to mph"), "62.1371192237 mph");
        assert_eq!(copied("10 m/s to km/h"), "36 km/h");
        assert_eq!(copied("60 mph to kph"), "96.56064 km/h");
        assert_eq!(copied("1 knot to km/h"), "1.852 km/h");
        assert_eq!(copied("100 km / h to m/s"), "27.7777777778 m/s");
        assert_eq!(copied("60 miles per hour to km/h"), "96.56064 km/h");
    }

    #[test]
    fn converts_area() {
        assert_eq!(copied("1 ha to m2"), "10000 m²");
        assert_eq!(copied("1 acre to m2"), "4046.8564224 m²");
        assert_eq!(copied("1 km2 to ha"), "100 ha");
        assert_eq!(copied("100 sq ft to m2"), "9.290304 m²");
        assert_eq!(copied("1 sq mi to acres"), "640 ac");
        assert_eq!(copied("1 m² to cm²"), "10000 cm²");
        assert_eq!(
            copied("10 square meters to square feet"),
            "107.639104167 ft²"
        );
    }

    #[test]
    fn conversions_accept_expressions_and_percentages() {
        assert_eq!(copied("(2 + 3) km to m"), "5000 m");
        assert_eq!(copied("2 * 5 km to m"), "10000 m");
        assert_eq!(copied("0.5 * 4 h to min"), "120 min");
        assert_eq!(copied("pi km to m"), "3141.59265359 m");
        assert_eq!(copied("-10 c to f"), "14 °F");
    }

    #[test]
    fn conversion_words_and_units_are_case_insensitive_except_data_symbols() {
        assert_eq!(copied("5 KM TO MILES"), "3.10685596119 mi");
        assert_eq!(copied("5 Km In Miles"), "3.10685596119 mi");
        assert_eq!(copied("5 km as mi"), "3.10685596119 mi");
        assert_eq!(copied("5 km into mi"), "3.10685596119 mi");
    }

    #[test]
    fn mismatched_unknown_or_incomplete_conversions_are_rejected() {
        for query in [
            "5 km to kg",
            "5 c to m",
            "5 km to",
            "5 km in",
            "to km",
            "km to m",
            "5 foo to bar",
            "5 km to foo",
            "5 foo to km",
            "5 mb to ms",
            "5 km to miles to m",
            "1 / 0 km to m",
            "5 km miles",
            "pdf to png",
            "firefox in c",
            "node 20 to 22",
            "5 km to 3 miles",
        ] {
            assert_eq!(evaluate(query), None, "{query:?}");
        }
    }

    #[test]
    fn long_queries_are_rejected() {
        assert_eq!(evaluate(&format!("1{}", " + 1".repeat(100))), None);
    }

    #[test]
    fn number_formatting_trims_and_rounds() {
        assert_eq!(format_number(0.1 + 0.2).unwrap(), "0.3");
        assert_eq!(format_number(100.0).unwrap(), "100");
        assert_eq!(format_number(-0.5).unwrap(), "-0.5");
        assert_eq!(format_number(123456.789).unwrap(), "123456.789");
        assert_eq!(format_number(f64::NAN), None);
        assert_eq!(format_number(f64::INFINITY), None);
        assert_eq!(group_digits("1234567.891"), "1,234,567.891");
        assert_eq!(group_digits("123"), "123");
        assert_eq!(group_digits("-1000"), "-1,000");
    }
}
