#![forbid(unsafe_code)]

pub mod bridge;
pub mod checker;
pub mod depth;
pub mod eval;
mod path;
pub mod printer;
pub mod shapes;
pub mod snippet;
pub mod surface;
mod walk;

pub use path::{ConfigPath, ConfigRoot, ElementPath, ElementRoot, Segment, ValuePath};

pub const REDUCE_ACCUMULATOR: &str = "acc";

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum EvalMode {
        Transform => "transform",
        Predicate => "predicate",
        Number => "number",
        Reduce => "reduce",
    }
}

impl EvalMode {
    #[must_use]
    pub const fn implicit_bindings(self) -> &'static [&'static str] {
        match self {
            Self::Reduce => &[REDUCE_ACCUMULATOR],
            Self::Transform | Self::Predicate | Self::Number => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EvalMode;

    #[test]
    fn evaluation_modes_have_the_four_canonical_names() {
        assert_eq!(
            EvalMode::ALL.map(EvalMode::as_str),
            ["transform", "predicate", "number", "reduce"]
        );
    }
}

mod comparison;
