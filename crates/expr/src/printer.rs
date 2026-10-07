
use std::fmt::Write as _;

use cel::common::ast::{
    CallExpr, ComprehensionExpr, EntryExpr, Expr, IdedExpr, ListExpr, LiteralValue, MapExpr,
    SelectExpr,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrintError {
    Unspecified,
    UnfoldableComprehension,
    UnrepresentableLiteral,
    UnsupportedConstruct(&'static str),
}

impl std::fmt::Display for PrintError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unspecified => formatter.write_str("an empty expression has no canonical text"),
            Self::UnfoldableComprehension => formatter
                .write_str("a comprehension that does not fold into a macro has no canonical text"),
            Self::UnrepresentableLiteral => {
                formatter.write_str("a non-finite number has no CEL literal spelling")
            }
            Self::UnsupportedConstruct(what) => {
                write!(
                    formatter,
                    "a construct this profile does not support: {what}"
                )
            }
        }
    }
}

impl std::error::Error for PrintError {}

pub fn print(term: &IdedExpr) -> Result<String, PrintError> {
    let mut out = String::new();
    write_expr(&mut out, term, Precedence::Lowest)?;
    Ok(out)
}

#[must_use]
pub fn terms_equal(left: &IdedExpr, right: &IdedExpr) -> bool {
    match (&left.expr, &right.expr) {
        (Expr::Unspecified, Expr::Unspecified) => true,
        (Expr::Ident(left), Expr::Ident(right)) => left == right,
        (Expr::Literal(left), Expr::Literal(right)) => left == right,
        (Expr::Select(left), Expr::Select(right)) => {
            left.field == right.field
                && left.test == right.test
                && terms_equal(&left.operand, &right.operand)
        }
        (Expr::Call(left), Expr::Call(right)) => {
            left.func_name == right.func_name
                && match (&left.target, &right.target) {
                    (None, None) => true,
                    (Some(left), Some(right)) => terms_equal(left, right),
                    _ => false,
                }
                && slices_equal(&left.args, &right.args)
        }
        (Expr::List(left), Expr::List(right)) => {
            left.optional_indices == right.optional_indices
                && slices_equal(&left.elements, &right.elements)
        }
        (Expr::Map(left), Expr::Map(right)) => entries_equal(left, right),
        (Expr::Comprehension(left), Expr::Comprehension(right)) => {
            left.iter_var == right.iter_var
                && left.iter_var2 == right.iter_var2
                && left.accu_var == right.accu_var
                && terms_equal(&left.iter_range, &right.iter_range)
                && terms_equal(&left.accu_init, &right.accu_init)
                && terms_equal(&left.loop_cond, &right.loop_cond)
                && terms_equal(&left.loop_step, &right.loop_step)
                && terms_equal(&left.result, &right.result)
        }
        _ => false,
    }
}

fn slices_equal(left: &[IdedExpr], right: &[IdedExpr]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| terms_equal(left, right))
}

fn entries_equal(left: &MapExpr, right: &MapExpr) -> bool {
    left.entries.len() == right.entries.len()
        && left
            .entries
            .iter()
            .zip(&right.entries)
            .all(|(left, right)| match (&left.expr, &right.expr) {
                (EntryExpr::MapEntry(left), EntryExpr::MapEntry(right)) => {
                    left.optional == right.optional
                        && terms_equal(&left.key, &right.key)
                        && terms_equal(&left.value, &right.value)
                }
                (EntryExpr::StructField(left), EntryExpr::StructField(right)) => {
                    left.field == right.field
                        && left.optional == right.optional
                        && terms_equal(&left.value, &right.value)
                }
                _ => false,
            })
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Precedence {
    Lowest,
    Conditional,
    Or,
    And,
    Relation,
    Additive,
    Multiplicative,
    Unary,
    Postfix,
}

fn write_expr(out: &mut String, term: &IdedExpr, context: Precedence) -> Result<(), PrintError> {
    match &term.expr {
        Expr::Unspecified => Err(PrintError::Unspecified),
        Expr::Ident(name) => {
            out.push_str(name);
            Ok(())
        }
        Expr::Literal(literal) => write_literal(out, literal),
        Expr::Select(select) => write_select(out, select),
        Expr::List(list) => write_list(out, list),
        Expr::Map(map) => write_map(out, map),
        Expr::Struct(_) => Err(PrintError::UnsupportedConstruct("struct literal")),
        Expr::Comprehension(comprehension) => write_comprehension(out, comprehension),
        Expr::Call(call) => write_call(out, call, context),
    }
}

fn wrap(
    out: &mut String,
    context: Precedence,
    own: Precedence,
    body: impl FnOnce(&mut String) -> Result<(), PrintError>,
) -> Result<(), PrintError> {
    let parenthesize = own < context;
    if parenthesize {
        out.push('(');
    }
    body(out)?;
    if parenthesize {
        out.push(')');
    }
    Ok(())
}

fn write_select(out: &mut String, select: &SelectExpr) -> Result<(), PrintError> {
    if select.test {
        out.push_str("has(");
        write_expr(out, &select.operand, Precedence::Postfix)?;
        let _ = write!(out, ".{})", select.field);
        return Ok(());
    }
    write_expr(out, &select.operand, Precedence::Postfix)?;
    let _ = write!(out, ".{}", select.field);
    Ok(())
}

fn write_list(out: &mut String, list: &ListExpr) -> Result<(), PrintError> {
    out.push('[');
    for (index, element) in list.elements.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        if list.optional_indices.contains(&index) {
            out.push('?');
        }
        write_expr(out, element, Precedence::Lowest)?;
    }
    out.push(']');
    Ok(())
}

fn write_map(out: &mut String, map: &MapExpr) -> Result<(), PrintError> {
    out.push('{');
    for (index, entry) in map.entries.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        match &entry.expr {
            EntryExpr::MapEntry(entry) => {
                if entry.optional {
                    out.push('?');
                }
                write_expr(out, &entry.key, Precedence::Lowest)?;
                out.push_str(": ");
                write_expr(out, &entry.value, Precedence::Lowest)?;
            }
            EntryExpr::StructField(_) => {
                return Err(PrintError::UnsupportedConstruct("struct field"));
            }
        }
    }
    out.push('}');
    Ok(())
}

fn binary(name: &str) -> Option<(&'static str, Precedence)> {
    Some(match name {
        "_||_" => ("||", Precedence::Or),
        "_&&_" => ("&&", Precedence::And),
        "_==_" => ("==", Precedence::Relation),
        "_!=_" => ("!=", Precedence::Relation),
        "_<_" => ("<", Precedence::Relation),
        "_<=_" => ("<=", Precedence::Relation),
        "_>_" => (">", Precedence::Relation),
        "_>=_" => (">=", Precedence::Relation),
        "@in" => ("in", Precedence::Relation),
        "_+_" => ("+", Precedence::Additive),
        "_-_" => ("-", Precedence::Additive),
        "_*_" => ("*", Precedence::Multiplicative),
        "_/_" => ("/", Precedence::Multiplicative),
        "_%_" => ("%", Precedence::Multiplicative),
        _ => return None,
    })
}

fn write_call(out: &mut String, call: &CallExpr, context: Precedence) -> Result<(), PrintError> {
    if call.target.is_none() {
        if let Some((symbol, precedence)) = binary(&call.func_name)
            && call.args.len() == 2
        {
            let left_context = if is_rebalanced(&call.func_name) {
                next_tighter(precedence)
            } else {
                precedence
            };
            return wrap(out, context, precedence, |out| {
                write_expr(out, &call.args[0], left_context)?;
                let _ = write!(out, " {symbol} ");
                write_expr(out, &call.args[1], next_tighter(precedence))
            });
        }
        match (call.func_name.as_str(), call.args.len()) {
            ("!_" | "-_", 1) => {
                let symbol = if call.func_name == "!_" { "!" } else { "-" };
                return wrap(out, context, Precedence::Unary, |out| {
                    out.push_str(symbol);
                    write_expr(out, &call.args[0], Precedence::Unary)
                });
            }
            ("_?_:_", 3) => {
                return wrap(out, context, Precedence::Conditional, |out| {
                    write_expr(out, &call.args[0], next_tighter(Precedence::Conditional))?;
                    out.push_str(" ? ");
                    write_expr(out, &call.args[1], next_tighter(Precedence::Conditional))?;
                    out.push_str(" : ");
                    write_expr(out, &call.args[2], Precedence::Conditional)
                });
            }
            ("_[_]", 2) => {
                write_expr(out, &call.args[0], Precedence::Postfix)?;
                out.push('[');
                write_expr(out, &call.args[1], Precedence::Lowest)?;
                out.push(']');
                return Ok(());
            }
            _ => {}
        }
    }

    if let Some(target) = &call.target {
        write_expr(out, target, Precedence::Postfix)?;
        out.push('.');
    }
    out.push_str(&call.func_name);
    out.push('(');
    for (index, argument) in call.args.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        write_expr(out, argument, Precedence::Lowest)?;
    }
    out.push(')');
    Ok(())
}

fn is_rebalanced(name: &str) -> bool {
    matches!(name, "_||_" | "_&&_")
}

const fn next_tighter(precedence: Precedence) -> Precedence {
    match precedence {
        Precedence::Lowest => Precedence::Conditional,
        Precedence::Conditional => Precedence::Or,
        Precedence::Or => Precedence::And,
        Precedence::And => Precedence::Relation,
        Precedence::Relation => Precedence::Additive,
        Precedence::Additive => Precedence::Multiplicative,
        Precedence::Multiplicative | Precedence::Unary => Precedence::Unary,
        Precedence::Postfix => Precedence::Postfix,
    }
}

fn write_literal(out: &mut String, literal: &LiteralValue) -> Result<(), PrintError> {
    match literal {
        LiteralValue::Null => out.push_str("null"),
        LiteralValue::Boolean(value) => out.push_str(if *value.inner() { "true" } else { "false" }),
        LiteralValue::Int(value) => {
            let _ = write!(out, "{}", value.inner());
        }
        LiteralValue::UInt(value) => {
            let _ = write!(out, "{}u", value.inner());
        }
        LiteralValue::Double(value) => {
            let value = *value.inner();
            if !value.is_finite() {
                return Err(PrintError::UnrepresentableLiteral);
            }
            let _ = write!(out, "{value:?}");
        }
        LiteralValue::String(value) => write_quoted(out, value.inner()),
        LiteralValue::Bytes(value) => {
            out.push_str("b\"");
            for byte in value.inner().iter() {
                match byte {
                    b'\\' => out.push_str("\\\\"),
                    b'"' => out.push_str("\\\""),
                    0x20..=0x7e => out.push(*byte as char),
                    _ => {
                        let _ = write!(out, "\\x{byte:02x}");
                    }
                }
            }
            out.push('"');
        }
    }
    Ok(())
}

fn write_quoted(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if control.is_control() => {
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

fn write_comprehension(out: &mut String, node: &ComprehensionExpr) -> Result<(), PrintError> {
    let accumulator = node.accu_var.as_str();
    let iterator = node.iter_var.as_str();

    let macro_call = fold_all(node, accumulator)
        .or_else(|| fold_exists(node, accumulator))
        .or_else(|| fold_exists_one(node, accumulator))
        .or_else(|| fold_filter(node, accumulator, iterator))
        .or_else(|| fold_map(node, accumulator))
        .ok_or(PrintError::UnfoldableComprehension)?;

    write_expr(out, &node.iter_range, Precedence::Postfix)?;
    let _ = write!(out, ".{}({iterator}, ", macro_call.name);
    for (index, argument) in macro_call.arguments.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        write_expr(out, argument, Precedence::Lowest)?;
    }
    out.push(')');
    Ok(())
}

struct MacroCall<'a> {
    name: &'static str,
    arguments: Vec<&'a IdedExpr>,
}

fn is_ident(term: &IdedExpr, name: &str) -> bool {
    matches!(&term.expr, Expr::Ident(found) if found == name)
}

fn call_of<'a>(term: &'a IdedExpr, name: &str, arity: usize) -> Option<&'a [IdedExpr]> {
    match &term.expr {
        Expr::Call(call)
            if call.target.is_none() && call.func_name == name && call.args.len() == arity =>
        {
            Some(&call.args)
        }
        _ => None,
    }
}

fn is_bool_literal(term: &IdedExpr, expected: bool) -> bool {
    matches!(&term.expr, Expr::Literal(LiteralValue::Boolean(value)) if *value.inner() == expected)
}

fn is_int_literal(term: &IdedExpr, expected: i64) -> bool {
    matches!(&term.expr, Expr::Literal(LiteralValue::Int(value)) if *value.inner() == expected)
}

fn is_empty_list(term: &IdedExpr) -> bool {
    matches!(&term.expr, Expr::List(list) if list.elements.is_empty())
}

fn fold_all<'a>(node: &'a ComprehensionExpr, accumulator: &str) -> Option<MacroCall<'a>> {
    if !is_bool_literal(&node.accu_init, true) || !is_ident(&node.result, accumulator) {
        return None;
    }
    let guard = call_of(&node.loop_cond, "@not_strictly_false", 1)?;
    if !is_ident(&guard[0], accumulator) {
        return None;
    }
    let step = call_of(&node.loop_step, "_&&_", 2)?;
    is_ident(&step[0], accumulator).then(|| MacroCall {
        name: "all",
        arguments: vec![&step[1]],
    })
}

fn fold_exists<'a>(node: &'a ComprehensionExpr, accumulator: &str) -> Option<MacroCall<'a>> {
    if !is_bool_literal(&node.accu_init, false) || !is_ident(&node.result, accumulator) {
        return None;
    }
    let guard = call_of(&node.loop_cond, "@not_strictly_false", 1)?;
    let negated = call_of(&guard[0], "!_", 1)?;
    if !is_ident(&negated[0], accumulator) {
        return None;
    }
    let step = call_of(&node.loop_step, "_||_", 2)?;
    is_ident(&step[0], accumulator).then(|| MacroCall {
        name: "exists",
        arguments: vec![&step[1]],
    })
}

fn fold_exists_one<'a>(node: &'a ComprehensionExpr, accumulator: &str) -> Option<MacroCall<'a>> {
    if !is_int_literal(&node.accu_init, 0) || !is_bool_literal(&node.loop_cond, true) {
        return None;
    }
    let result = call_of(&node.result, "_==_", 2)?;
    if !is_ident(&result[0], accumulator) || !is_int_literal(&result[1], 1) {
        return None;
    }
    let step = call_of(&node.loop_step, "_?_:_", 3)?;
    let increment = call_of(&step[1], "_+_", 2)?;
    (is_ident(&increment[0], accumulator)
        && is_int_literal(&increment[1], 1)
        && is_ident(&step[2], accumulator))
    .then(|| MacroCall {
        name: "exists_one",
        arguments: vec![&step[0]],
    })
}

fn conditional_append<'a>(
    node: &'a ComprehensionExpr,
    accumulator: &str,
) -> Option<(&'a IdedExpr, &'a IdedExpr)> {
    let step = call_of(&node.loop_step, "_?_:_", 3)?;
    let appended = call_of(&step[1], "_+_", 2)?;
    if !is_ident(&appended[0], accumulator) || !is_ident(&step[2], accumulator) {
        return None;
    }
    let Expr::List(list) = &appended[1].expr else {
        return None;
    };
    let [element] = list.elements.as_slice() else {
        return None;
    };
    Some((&step[0], element))
}

fn fold_filter<'a>(
    node: &'a ComprehensionExpr,
    accumulator: &str,
    iterator: &str,
) -> Option<MacroCall<'a>> {
    if !is_empty_list(&node.accu_init)
        || !is_bool_literal(&node.loop_cond, true)
        || !is_ident(&node.result, accumulator)
    {
        return None;
    }
    let (predicate, element) = conditional_append(node, accumulator)?;
    is_ident(element, iterator).then(|| MacroCall {
        name: "filter",
        arguments: vec![predicate],
    })
}

fn fold_map<'a>(node: &'a ComprehensionExpr, accumulator: &str) -> Option<MacroCall<'a>> {
    if !is_empty_list(&node.accu_init)
        || !is_bool_literal(&node.loop_cond, true)
        || !is_ident(&node.result, accumulator)
    {
        return None;
    }
    if let Some((predicate, element)) = conditional_append(node, accumulator) {
        return Some(MacroCall {
            name: "map",
            arguments: vec![predicate, element],
        });
    }
    let appended = call_of(&node.loop_step, "_+_", 2)?;
    if !is_ident(&appended[0], accumulator) {
        return None;
    }
    let Expr::List(list) = &appended[1].expr else {
        return None;
    };
    let [element] = list.elements.as_slice() else {
        return None;
    };
    Some(MacroCall {
        name: "map",
        arguments: vec![element],
    })
}

#[cfg(test)]
mod tests {
    use super::{print, terms_equal};
    use cel::parser::Parser;

    fn canonical(source: &str) -> String {
        let term = Parser::new().parse(source).expect("parse");
        print(&term).expect("print")
    }

    fn assert_roundtrips(source: &str) {
        let original = Parser::new().parse(source).expect("parse");
        let printed = print(&original).expect("print");
        let reparsed = Parser::new()
            .parse(&printed)
            .unwrap_or_else(|error| panic!("{source} → {printed} does not parse again: {error}"));
        assert!(
            terms_equal(&original, &reparsed),
            "{source} → {printed} became a different term"
        );
        assert_eq!(print(&reparsed).expect("reprint"), printed, "{source}");
    }

    #[test]
    fn law_expr_59_holds_across_the_surface() {
        for source in [
            "a",
            "a.b.c",
            "a[0]",
            "a[b]",
            "-a",
            "!a",
            "!!a",
            "1",
            "1u",
            "1.5",
            "1.0",
            "-1",
            "'s'",
            "\"s\"",
            "b'ab'",
            "null",
            "true",
            "false",
            "a + b",
            "a - b",
            "a * b",
            "a / b",
            "a % b",
            "a == b",
            "a != b",
            "a < b",
            "a <= b",
            "a > b",
            "a >= b",
            "a in l",
            "a && b",
            "a || b",
            "a ? b : c",
            "a + b * c",
            "(a + b) * c",
            "a || b && c",
            "(a || b) && c",
            "a - b - c",
            "a - (b - c)",
            "a ? b : c ? d : e",
            "[]",
            "[1, 2, 3]",
            "[[1], [2]]",
            "{}",
            "{'k': 1}",
            "{'k': {'j': 2}}",
            "size('abc')",
            "'abc'.size()",
            "f(a, b)",
            "a.f(b)",
            "has(m.a)",
            "has(m.a.b)",
            "l.all(x, x > 0)",
            "l.exists(x, x > 0)",
            "l.exists_one(x, x > 0)",
            "l.map(x, x * 2)",
            "l.map(x, x > 1, x * 2)",
            "l.filter(x, x > 1)",
            "l.map(x, x.map(y, y + 1))",
            "m.all(k, m[k] > 0)",
        ] {
            assert_roundtrips(source);
        }
    }

    #[test]
    fn spelling_differences_collapse() {
        assert_eq!(canonical("a  +   b"), "a + b");
        assert_eq!(canonical("a+b"), "a + b");
        assert_eq!(canonical("(a + b)"), "a + b");
        assert_eq!(canonical("((a))"), "a");
        assert_eq!(canonical("'s'"), "\"s\"");
        assert_eq!(canonical("[ 1 , 2 ]"), "[1, 2]");
    }

    #[test]
    fn literal_kind_survives_printing() {
        assert_eq!(canonical("1"), "1");
        assert_eq!(canonical("1.0"), "1.0");
        assert_eq!(canonical("2.0"), "2.0");
        assert_eq!(canonical("1u"), "1u");
        assert_eq!(canonical("l[1]"), "l[1]");
    }

    #[test]
    fn macro_binder_names_survive_printing() {
        assert_eq!(canonical("l.all(x, x > 0)"), "l.all(x, x > 0)");
        assert_eq!(canonical("l.all(item, item > 0)"), "l.all(item, item > 0)");
        assert_ne!(canonical("l.all(x, x > 0)"), canonical("l.all(v, v > 0)"));
    }

    #[test]
    fn string_contents_are_not_normalized() {
        assert_ne!(canonical("'caf\\u00e9'"), canonical("'cafe\\u0301'"));
        assert_eq!(canonical("'café'"), "\"café\"");
    }
}
