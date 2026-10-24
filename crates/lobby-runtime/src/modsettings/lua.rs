//! A widget handler's config as data: BAR's `LuaUI/Config/<game>.lua` is a
//! `return { … }` of literals, and that is all this reads. A call, a
//! variable, an operator -- anything a mod could plant to run -- rejects the
//! whole file, and what is written back is modlobby's own text in BAR's
//! shape, never a byte a mod wrote.

use full_moon::ast::{Expression, Field, LastStmt, TableConstructor, UnOp};
use full_moon::tokenizer::{Lexer, LexerResult, StringLiteralQuoteType, Symbol, TokenType};

/// How deep a config's tables may nest: BAR's go four deep. Counted in the
/// token stream, before the parser -- which recurses once a level and has
/// no limit of its own -- ever runs.
const MOST_DEPTH: usize = 32;

/// A Lua value a config may hold.
#[derive(Debug, Clone, PartialEq)]
pub enum Lua {
	Bool(bool),
	Number(f64),
	Str(String),
	/// Fields in the order written; a list's entries are numbered from 1.
	Table(Vec<(Key, Lua)>),
}

/// A table field's key.
#[derive(Debug, Clone, PartialEq)]
pub enum Key {
	Name(String),
	Str(String),
	Number(f64),
}

/// The table `text` returns, when `text` is nothing but `return { … }` of
/// literals; otherwise why it is not, naming the first thing found that
/// is not a literal.
pub fn read(text: &str) -> Result<Lua, String> {
	scan(text)?;
	let ast = full_moon::parse(text).map_err(|errors| {
		errors
			.iter()
			.map(ToString::to_string)
			.collect::<Vec<_>>()
			.join("; ")
	})?;
	let block = ast.nodes();
	if block.stmts().next().is_some() {
		return Err("a statement before the return: only `return { … }` is read".into());
	}
	let Some(LastStmt::Return(returned)) = block.last_stmt() else {
		return Err("no `return` at the end".into());
	};
	let mut handed = returned.returns().iter();
	let (Some(expression), None) = (handed.next(), handed.next()) else {
		return Err("the return hands back one table, or nothing is read".into());
	};
	match expression {
		Expression::TableConstructor(table) => Ok(Lua::Table(fields(table)?)),
		other => Err(format!(
			"the return hands back {} rather than a table",
			what(other)
		)),
	}
}

/// Refuses, from the tokens alone, what no literal config holds and what
/// would make a parser recurse without end: any symbol but the brackets,
/// `=`, `,`, `;`, `return`, `true`, `false` and one minus before a number;
/// nesting deeper than [`MOST_DEPTH`]. The lexer runs in a loop, so a file
/// of any shape is scanned in bounded stack.
fn scan(text: &str) -> Result<(), String> {
	let tokens = match Lexer::new(text, full_moon::LuaVersion::lua51()).collect() {
		LexerResult::Ok(tokens) => tokens,
		LexerResult::Recovered(_, errors) | LexerResult::Fatal(errors) => {
			let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
			return Err(errors.join("; "));
		}
	};
	let mut depth = 0_usize;
	let mut minuses = 0_usize;
	for token in &tokens {
		match token.token_type() {
			TokenType::Whitespace { .. }
			| TokenType::SingleLineComment { .. }
			| TokenType::MultiLineComment { .. }
			| TokenType::Eof => continue,
			TokenType::Number { .. }
			| TokenType::StringLiteral { .. }
			| TokenType::Identifier { .. } => {}
			TokenType::Symbol { symbol } => match symbol {
				Symbol::LeftBrace | Symbol::LeftBracket | Symbol::LeftParen => {
					depth += 1;
					if depth > MOST_DEPTH {
						return Err(format!("tables nested deeper than {MOST_DEPTH}"));
					}
				}
				Symbol::RightBrace | Symbol::RightBracket | Symbol::RightParen => {
					depth = depth.saturating_sub(1);
				}
				Symbol::Minus => {
					minuses += 1;
					if minuses > 1 {
						return Err("a minus before a minus".into());
					}
					continue;
				}
				Symbol::Return
				| Symbol::True
				| Symbol::False
				| Symbol::Equal
				| Symbol::Comma
				| Symbol::Semicolon => {}
				other => return Err(format!("`{other}`, which is not a value this keeps")),
			},
			_ => return Err(format!("`{token}`, which is not a value this keeps")),
		}
		minuses = 0;
	}
	Ok(())
}

fn fields(table: &TableConstructor) -> Result<Vec<(Key, Lua)>, String> {
	let mut out = Vec::new();
	let mut next_index = 1.0;
	for field in table.fields() {
		let (key, value) = match field {
			Field::NoKey(expression) => {
				let key = Key::Number(next_index);
				next_index += 1.0;
				(key, expression)
			}
			Field::NameKey { key, value, .. } => match key.token().token_type() {
				TokenType::Identifier { identifier } => {
					(Key::Name(identifier.as_str().to_owned()), value)
				}
				_ => return Err("a key that is not a name".into()),
			},
			Field::ExpressionKey { key, value, .. } => {
				let key = match literal(key)? {
					Lua::Str(text) => Key::Str(text),
					Lua::Number(number) => Key::Number(number),
					_ => return Err("a key that is neither a string nor a number".into()),
				};
				(key, value)
			}
			_ => return Err("a field of a kind this does not read".into()),
		};
		out.push((key, literal(value)?));
	}
	Ok(out)
}

/// `expression` as a literal, or why it is not one.
fn literal(expression: &Expression) -> Result<Lua, String> {
	match expression {
		Expression::String(token) => match token.token().token_type() {
			TokenType::StringLiteral {
				literal,
				quote_type: StringLiteralQuoteType::Brackets,
				..
			} => Ok(Lua::Str(literal.as_str().to_owned())),
			TokenType::StringLiteral { literal, .. } => Ok(Lua::Str(unescape(literal.as_str()))),
			_ => Err("a string token that is not a literal".into()),
		},
		Expression::Number(token) => match token.token().token_type() {
			TokenType::Number { text } => number(text.as_str()).map(Lua::Number),
			_ => Err("a number token that is not a literal".into()),
		},
		Expression::Symbol(token) => match token.token().to_string().trim() {
			"true" => Ok(Lua::Bool(true)),
			"false" => Ok(Lua::Bool(false)),
			other => Err(format!("`{other}`, which is not a value this keeps")),
		},
		Expression::UnaryOperator {
			unop: UnOp::Minus(_),
			expression,
		} => match literal(expression)? {
			Lua::Number(number) => Ok(Lua::Number(-number)),
			_ => Err("a minus before something that is not a number".into()),
		},
		Expression::Parentheses { expression, .. } => literal(expression),
		Expression::TableConstructor(table) => Ok(Lua::Table(fields(table)?)),
		other => Err(what(other)),
	}
}

/// What an expression that is not a literal is, in words.
fn what(expression: &Expression) -> String {
	match expression {
		Expression::FunctionCall(_) => "a call".into(),
		Expression::Var(_) => "a variable".into(),
		Expression::Function(_) => "a function".into(),
		Expression::BinaryOperator { .. } => "an operator".into(),
		Expression::UnaryOperator { .. } => "an operator".into(),
		_ => "something that is not a literal".into(),
	}
}

fn number(text: &str) -> Result<f64, String> {
	let lower = text.to_ascii_lowercase();
	if let Some(hex) = lower.strip_prefix("0x") {
		return i64::from_str_radix(hex, 16)
			.map(|n| n as f64)
			.map_err(|_| format!("{text} is not a number"));
	}
	text.parse::<f64>()
		.ok()
		.filter(|number| number.is_finite())
		.ok_or_else(|| format!("{text} is not a number a double holds"))
}

/// A quoted literal's text with Lua's escapes undone.
fn unescape(literal: &str) -> String {
	let mut out = String::new();
	let mut chars = literal.chars().peekable();
	while let Some(c) = chars.next() {
		if c != '\\' {
			out.push(c);
			continue;
		}
		match chars.next() {
			Some('n') => out.push('\n'),
			Some('t') => out.push('\t'),
			Some('r') => out.push('\r'),
			Some('a') => out.push('\u{7}'),
			Some('b') => out.push('\u{8}'),
			Some('f') => out.push('\u{c}'),
			Some('v') => out.push('\u{b}'),
			Some('\\') => out.push('\\'),
			Some('"') => out.push('"'),
			Some('\'') => out.push('\''),
			Some('\n') => out.push('\n'),
			Some(digit) if digit.is_ascii_digit() => {
				let mut code = digit.to_digit(10).unwrap_or(0);
				for _ in 0..2 {
					match chars.peek().and_then(|c| c.to_digit(10)) {
						Some(next) => {
							code = code * 10 + next;
							chars.next();
						}
						None => break,
					}
				}
				out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
			}
			Some(other) => {
				out.push('\\');
				out.push(other);
			}
			None => out.push('\\'),
		}
	}
	out
}

/// `value` as BAR's `tableSave` writes a config: `return {` and the fields
/// one a line, a tab per level, trailing commas, names bare where Lua
/// allows and bracketed strings elsewhere.
pub fn write(value: &Lua) -> String {
	format!("return {}\n", text(value))
}

/// `value` as Lua source on its own: a table over its lines, a scalar as
/// written.
pub fn text(value: &Lua) -> String {
	let mut out = String::new();
	emit(value, 0, &mut out);
	out
}

fn emit(value: &Lua, depth: usize, out: &mut String) {
	match value {
		Lua::Bool(held) => out.push_str(if *held { "true" } else { "false" }),
		Lua::Number(number) => out.push_str(&number.to_string()),
		Lua::Str(text) => {
			out.push('"');
			out.push_str(&escape(text));
			out.push('"');
		}
		Lua::Table(fields) => {
			out.push_str("{\n");
			for (key, value) in fields {
				for _ in 0..=depth {
					out.push('\t');
				}
				match key {
					Key::Name(name) if is_name(name) => out.push_str(name),
					Key::Name(name) | Key::Str(name) => {
						out.push_str("[\"");
						out.push_str(&escape(name));
						out.push_str("\"]");
					}
					Key::Number(number) => {
						out.push('[');
						out.push_str(&number.to_string());
						out.push(']');
					}
				}
				out.push_str(" = ");
				emit(value, depth + 1, out);
				out.push_str(",\n");
			}
			for _ in 0..depth {
				out.push('\t');
			}
			out.push('}');
		}
	}
}

const KEYWORDS: &[&str] = &[
	"and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
	"local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// Whether `name` can stand bare before `=` in a table.
fn is_name(name: &str) -> bool {
	let mut chars = name.chars();
	chars
		.next()
		.is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
		&& chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
		&& !KEYWORDS.contains(&name)
}

fn escape(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	for c in text.chars() {
		match c {
			'\\' => out.push_str("\\\\"),
			'"' => out.push_str("\\\""),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if c.is_control() => out.push_str(&format!("\\{}", c as u32)),
			c => out.push(c),
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	const BYAR: &str = "-- Widget Custom data and order, order = 0 disabled widget\nreturn {\n\tallowUserWidgets = true,\n\tdata = {\n\t\tAdvPlayersList = {\n\t\t\tabsoluteResbarValues = false,\n\t\t\tdist = 14086.7168,\n\t\t\tdx = -0.03141068,\n\t\t\tname = \"spring\",\n\t\t},\n\t\t[\"Commander Name Tags\"] = {\n\t\t\tnameScaling = true,\n\t\t},\n\t},\n\torder = {\n\t\t[\"API Builder Queue\"] = 130,\n\t\tChat = 7,\n\t},\n}\n";

	#[test]
	fn a_bar_config_round_trips_in_bars_shape() {
		let read = read(BYAR).unwrap();
		assert_eq!(write(&read), BYAR.split_once('\n').unwrap().1);
		assert_eq!(super::read(&write(&read)).unwrap(), read);
	}

	#[test]
	fn a_table_with_a_call_is_rejected_whole() {
		let call = read("return { data = { x = print(\"y\") } }").unwrap_err();
		assert!(call.contains("call"), "{call}");
		let variable = read("return { a = b }").unwrap_err();
		assert!(variable.contains("variable"), "{variable}");
		assert!(read("return { a = 1 + 2 }").is_err());
		assert!(read("local x = 1\nreturn { a = x }").is_err());
		assert!(read("return 5").is_err());
		assert!(read("").is_err());
		assert_eq!(
			read("return { n = -1 }").unwrap(),
			Lua::Table(vec![(Key::Name("n".into()), Lua::Number(-1.0))])
		);
	}

	#[test]
	fn strings_with_escapes_and_long_brackets_are_kept() {
		let text =
			"return {\n\ta = \"q\\\"b\\nc\",\n\tb = [[long\nbracket]],\n\t[3] = \"third\",\n}\n";
		let read = read(text).unwrap();
		assert_eq!(
			read,
			Lua::Table(vec![
				(Key::Name("a".into()), Lua::Str("q\"b\nc".into())),
				(Key::Name("b".into()), Lua::Str("long\nbracket".into())),
				(Key::Number(3.0), Lua::Str("third".into())),
			])
		);
		assert_eq!(super::read(&write(&read)).unwrap(), read);
	}

	/// Nesting is counted in the token stream before anything recursive
	/// runs: a deep table is refused, never a stack overflow.
	#[test]
	fn a_deep_table_is_refused_before_it_is_parsed() {
		let deep = format!("return {}{}", "{".repeat(300), "}".repeat(300));
		let err = read(&deep).unwrap_err();
		assert!(err.contains("deep"), "{err}");
		let fine = format!("return {}{}", "{".repeat(20), "}".repeat(20));
		assert!(read(&fine).is_ok());
		let parens = format!("return {{ a = {}1{} }}", "(".repeat(300), ")".repeat(300));
		assert!(read(&parens).is_err());
	}

	/// An operator never belongs in a config, and a long chain of them
	/// recurses in any parser: refused from the tokens alone.
	#[test]
	fn an_operator_chain_is_refused_before_it_is_parsed() {
		let chain = format!("return {{ a = \"x\"{} }}", " .. \"x\"".repeat(20_000));
		assert!(read(&chain).is_err());
		let minus = format!("return {{ a = {}1 }}", "- ".repeat(20_000));
		assert!(read(&minus).is_err());
		assert!(read("return { a = -1 }").is_ok());
	}

	/// What a double cannot hold would be written back as `inf`, which no
	/// Lua reads as a number.
	#[test]
	fn a_number_past_what_a_double_holds_is_refused() {
		assert!(read("return { a = 1e309 }").is_err());
		assert!(read("return { a = 1e308 }").is_ok());
	}
}
