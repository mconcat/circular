
use cel::common::ast::IdedExpr;
use cel::parser::{ParseErrors, Parser};

use crate::walk::{Structure, structure};

pub const MAX_TERM_DEPTH: usize = 64;

pub const PARSER_RECURSION_LIMIT: u16 = (MAX_TERM_DEPTH - 1) as u16;

#[derive(Debug)]
pub enum ParseRejection {
    Grammar(ParseErrors),
    TooDeep {
        depth: usize,
        limit: usize,
    },
}

impl std::fmt::Display for ParseRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Grammar(errors) => write!(formatter, "{errors}"),
            Self::TooDeep { depth, limit } => {
                write!(
                    formatter,
                    "expression nesting depth {depth} exceeds the limit {limit}"
                )
            }
        }
    }
}

pub fn parse_bounded(source: &str) -> Result<IdedExpr, ParseRejection> {
    let term = Parser::new()
        .max_recursion_depth(PARSER_RECURSION_LIMIT)
        .parse(source)
        .map_err(ParseRejection::Grammar)?;
    if exceeds_max_depth(&term) {
        return Err(ParseRejection::TooDeep {
            depth: MAX_TERM_DEPTH + 1,
            limit: MAX_TERM_DEPTH,
        });
    }
    Ok(term)
}

#[must_use]
pub fn exceeds_max_depth(term: &IdedExpr) -> bool {
    let mut work = vec![(term, 1usize)];
    while let Some((node, depth)) = work.pop() {
        if depth > MAX_TERM_DEPTH {
            return true;
        }
        let child = depth + 1;
        match structure(node) {
            Structure::Leaf => {}
            Structure::Node(children) => {
                work.extend(children.into_iter().map(|node| (node, child)));
            }
            Structure::Comprehension {
                iter_range,
                accu_init,
                loop_cond,
                loop_step,
                result,
                ..
            } => {
                work.push((iter_range, child));
                work.push((accu_init, child));
                work.push((loop_cond, child));
                work.push((loop_step, child));
                work.push((result, child));
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{MAX_TERM_DEPTH, exceeds_max_depth, parse_bounded};

    #[test]
    fn left_recursive_nesting_is_bounded() {
        for build in [
            (|n: usize| format!("a{}", ".f".repeat(n))) as fn(usize) -> String,
            |n| format!("a{}", "[0]".repeat(n)),
        ] {
            assert!(
                parse_bounded(&build(MAX_TERM_DEPTH - 4)).is_ok(),
                "inner refusal"
            );
            assert!(
                parse_bounded(&build(MAX_TERM_DEPTH + 4)).is_err(),
                "outer pass"
            );
        }
    }

    #[test]
    fn rule_recursive_nesting_is_bounded() {
        for build in [
            (|n: usize| format!("{}1{}", "[".repeat(n), "]".repeat(n))) as fn(usize) -> String,
            |n| format!("{}1{}", "{'k': ".repeat(n), "}".repeat(n)),
            |n| format!("{}1{}", "f(".repeat(n), ")".repeat(n)),
        ] {
            assert!(
                parse_bounded(&build(MAX_TERM_DEPTH - 4)).is_ok(),
                "inner refusal"
            );
            assert!(
                parse_bounded(&build(MAX_TERM_DEPTH + 4)).is_err(),
                "outer pass"
            );
        }
    }

    #[test]
    fn a_wide_real_term_fits_with_room() {
        let source = r#"((((("message" in event) && ("usage" in event.message)) && ("input_tokens" in event.message.usage)) && ("output_tokens" in event.message.usage)) ? {"vendor": "claude", "sessionId": (("sessionId" in event) ? event.sessionId : ""), "type": "usage", "input_tokens": event.message.usage.input_tokens, "output_tokens": event.message.usage.output_tokens} : (((((("type" in event) && (event.type == "assistant")) && ("message" in event)) && ("content" in event.message)) && event.message.content.exists(b, (b.type == "tool_use"))) ? {"vendor": "claude", "sessionId": (("sessionId" in event) ? event.sessionId : ""), "type": "tool_use", "name": event.message.content.filter(b, (b.type == "tool_use"))[0].name} : ((("type" in event) && (event.type == "error")) ? {"vendor": "claude", "sessionId": (("sessionId" in event) ? event.sessionId : ""), "type": "error"} : {"vendor": "claude", "sessionId": (("sessionId" in event) ? event.sessionId : ""), "type": "label", "label": (("repo" in event) ? event.repo : "claude")})))"#;
        assert!(parse_bounded(source).is_ok(), "a real term was refused");
    }

    #[test]
    fn wide_but_shallow_terms_are_accepted() {
        let elements = (0..500).map(|_| "1").collect::<Vec<_>>().join(", ");
        assert!(parse_bounded(&format!("[{elements}]")).is_ok());
        assert!(parse_bounded(&format!("f({elements})")).is_ok());

        let pairs = (0..300)
            .map(|index| format!("'k{index}': {index}"))
            .collect::<Vec<_>>()
            .join(", ");
        assert!(parse_bounded(&format!("{{{pairs}}}")).is_ok());
    }

    #[test]
    fn the_term_gate_passes_shallow_terms() {
        let shallow = parse_bounded("l.map(x, x * 2)").expect("parse");
        assert!(!exceeds_max_depth(&shallow));
    }

    #[test]
    fn far_over_limit_inputs_reject_on_the_default_stack() {
        for source in [
            format!("a{}", ".f".repeat(10_000)),
            format!("a{}", "[0]".repeat(10_000)),
            format!("1{}", " + 1".repeat(10_000)),
            format!("1{}", " < 1".repeat(10_000)),
            format!(
                "{}a{}{}",
                "f(".repeat(60),
                ".f".repeat(10_000),
                ")".repeat(60)
            ),
            format!("{}1{}", "[".repeat(10_000), "]".repeat(10_000)),
        ] {
            assert!(parse_bounded(&source).is_err());
        }
    }

    #[test]
    fn long_shallow_syntax_is_not_a_left_recursion_limit() {
        for source in [
            vec!["true"; 10_000].join(" || "),
            format!("pkg.{}Message {{}}", "qualified.".repeat(10_000)),
            format!("'{}'", ".field[0]+1".repeat(10_000)),
            format!("1 // {}", ".field[0]+1".repeat(10_000)),
            format!("{}true", "!".repeat(10_000)),
        ] {
            assert!(parse_bounded(&source).is_ok());
        }
        let mixed = format!("{}a{}{}", "f(".repeat(30), ".f".repeat(33), ")".repeat(30));
        assert!(parse_bounded(&mixed).is_ok());
        let mixed = format!("{}a{}{}", "f(".repeat(30), ".f".repeat(34), ")".repeat(30));
        assert!(parse_bounded(&mixed).is_err());
    }

    #[test]
    fn parse_28877_terms_benchmark() {
        let start = std::time::Instant::now();
        for _ in 0..28_877 {
            assert!(parse_bounded("event.body == 'ok' ? event.count + 1 : 0").is_ok());
        }
        eprintln!(
            "parse-bench terms=28877 elapsed_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }

    #[test]
    fn ordinary_terms_pass() {
        for source in [
            "a",
            "a + b * c",
            "l.map(x, x * 2)",
            "l.all(x, x.size() > 0)",
            "{'a': [1, 2], 'b': {'c': 3}}",
            "has(m.a) ? m.a : 'none'",
        ] {
            assert!(parse_bounded(source).is_ok(), "{source}");
        }
    }
}
