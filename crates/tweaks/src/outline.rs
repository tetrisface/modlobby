//! The names at the top of a payload, for jumping about a long one.
//!
//! A `tweakunits` table names hundreds of units; a `tweakdefs` script names
//! its helpers and the things it assigns. Nothing deeper: the point is to
//! find `corgolt4` in twelve thousand characters, not to index Lua.

use full_moon::ast::{Ast, Expression, Field, LastStmt, Prefix, Stmt, Var};
use full_moon::tokenizer::{TokenReference, TokenType};

use crate::Kind;
use crate::check::Symbol;

pub fn symbols(ast: &Ast, kind: Kind) -> Vec<Symbol> {
    match kind {
        Kind::Units => units(ast),
        Kind::Defs => defs(ast),
    }
}

/// The keys of the returned table constructor, in order.
fn units(ast: &Ast) -> Vec<Symbol> {
    let Some(LastStmt::Return(ret)) = ast.nodes().last_stmt() else {
        return Vec::new();
    };
    let Some(Expression::TableConstructor(table)) = ret.returns().iter().next() else {
        return Vec::new();
    };
    table.fields().iter().filter_map(key_of).collect()
}

fn key_of(field: &Field) -> Option<Symbol> {
    match field {
        Field::NameKey { key, .. } => Some(named(key)),
        Field::ExpressionKey {
            key: Expression::String(token),
            brackets,
            ..
        } => {
            let TokenType::StringLiteral { literal, .. } = token.token_type() else {
                return None;
            };
            Some(Symbol {
                name: literal.to_string(),
                line: line_of(brackets.tokens().0),
            })
        }
        _ => None,
    }
}

/// Top-level locals, assignments and functions, by their first name.
fn defs(ast: &Ast) -> Vec<Symbol> {
    ast.nodes()
        .stmts()
        .filter_map(|stmt| match stmt {
            Stmt::LocalAssignment(local) => local.names().iter().next().map(named),
            Stmt::LocalFunction(function) => Some(named(function.name())),
            Stmt::Assignment(assignment) => assignment.variables().iter().next().and_then(assigned),
            Stmt::FunctionDeclaration(function) => {
                let names = function.name().names();
                let first = names.iter().next()?;
                Some(Symbol {
                    name: names
                        .iter()
                        .map(|name| name.token().to_string())
                        .collect::<Vec<_>>()
                        .join("."),
                    line: line_of(first),
                })
            }
            _ => None,
        })
        .collect()
}

/// `UnitDefs.armcom = …` reads as one name; `t[k] = …` is left out.
fn assigned(var: &Var) -> Option<Symbol> {
    match var {
        Var::Name(name) => Some(named(name)),
        Var::Expression(expression) => {
            let Prefix::Name(first) = expression.prefix() else {
                return None;
            };
            let text = expression.to_string();
            let trimmed = text.trim();
            if trimmed.contains(['[', '(']) {
                return None;
            }
            Some(Symbol {
                name: trimmed.to_owned(),
                line: line_of(first),
            })
        }
        _ => None,
    }
}

fn named(token: &TokenReference) -> Symbol {
    Symbol {
        name: token.token().to_string(),
        line: line_of(token),
    }
}

fn line_of(token: &TokenReference) -> u32 {
    u32::try_from(token.token().start_position().line()).unwrap_or(u32::MAX)
}
