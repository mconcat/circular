
use circular_core::{PortId, PortIdError, Value};
use circular_expr::Segment;

use crate::config::{PayloadPath, PayloadPathError, payload_path_from_value};

pub const ROUTE_PREFIX: &str = "route_";

pub const UNMATCHED_PORT: &str = "unmatched";

#[derive(Clone, Debug, PartialEq)]
pub struct RouteCases {
    cases: Vec<RouteCase>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RouteCase {
    name: String,
    port: PortId,
    match_key: Value,
}

impl RouteCase {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn port(&self) -> &PortId {
        &self.port
    }

    #[must_use]
    pub const fn match_key(&self) -> &Value {
        &self.match_key
    }
}

impl RouteCases {
    pub fn try_new<I, S>(cases: I) -> Result<Self, RouteCasesError>
    where
        I: IntoIterator<Item = (S, Value)>,
        S: Into<String>,
    {
        let mut cases: Vec<RouteCase> = cases
            .into_iter()
            .map(|(name, match_key)| {
                let name = name.into();
                validate_case_key(&name)?;
                let port =
                    PortId::try_derived(format!("{ROUTE_PREFIX}{name}")).map_err(|source| {
                        RouteCasesError::PortName {
                            case: name.clone(),
                            source,
                        }
                    })?;
                Ok(RouteCase {
                    name,
                    port,
                    match_key,
                })
            })
            .collect::<Result<_, RouteCasesError>>()?;

        cases.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));

        for (index, case) in cases.iter().enumerate() {
            if let Some(earlier) = cases[..index]
                .iter()
                .find(|earlier| earlier.match_key == case.match_key)
            {
                return Err(RouteCasesError::DuplicateMatchKey {
                    first: earlier.name.clone(),
                    second: case.name.clone(),
                });
            }
        }

        Ok(Self { cases })
    }

    #[must_use]
    pub const fn empty() -> Self {
        Self { cases: Vec::new() }
    }

    #[must_use]
    pub fn cases(&self) -> &[RouteCase] {
        &self.cases
    }

    #[must_use]
    pub fn match_case(&self, value: &Value) -> Option<&RouteCase> {
        self.cases.iter().find(|case| &case.match_key == value)
    }
}

fn validate_case_key(key: &str) -> Result<(), RouteCasesError> {
    if key.is_empty() {
        return Err(RouteCasesError::CaseKeyEmpty);
    }
    if let Some(at) = key
        .bytes()
        .position(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'))
    {
        return Err(RouteCasesError::CaseKeyByte {
            case: key.to_owned(),
            at,
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RouteCasesError {
    CaseKeyEmpty,
    CaseKeyByte {
        case: String,
        at: usize,
    },
    PortName { case: String, source: PortIdError },
    DuplicateMatchKey { first: String, second: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RouteConfig {
    at: PayloadPath,
    cases: RouteCases,
}

impl RouteConfig {
    pub const AT: &'static str = "at";
    pub const CASES: &'static str = "cases";

    pub fn from_value(value: &Value) -> Result<Self, RouteConfigError> {
        let Some(root) = value.as_object() else {
            return Err(RouteConfigError::NotAnObject);
        };

        let at = root
            .get(Self::AT)
            .ok_or(RouteConfigError::MissingAt)
            .and_then(|at| payload_path_from_value(at).map_err(RouteConfigError::At))?;

        let cases = root
            .get(Self::CASES)
            .ok_or(RouteConfigError::MissingCases)?;
        let Some(cases) = cases.as_object() else {
            return Err(RouteConfigError::CasesNotAnObject);
        };
        let cases = RouteCases::try_new(
            cases
                .iter()
                .map(|(name, value)| (name.to_owned(), value.clone())),
        )
        .map_err(RouteConfigError::Cases)?;

        Ok(Self { at, cases })
    }

    #[must_use]
    pub const fn at(&self) -> &PayloadPath {
        &self.at
    }

    #[must_use]
    pub const fn cases(&self) -> &RouteCases {
        &self.cases
    }

    #[must_use]
    pub fn into_parts(self) -> (PayloadPath, RouteCases) {
        (self.at, self.cases)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RouteConfigError {
    NotAnObject,
    MissingAt,
    At(PayloadPathError),
    MissingCases,
    CasesNotAnObject,
    Cases(RouteCasesError),
}

impl core::fmt::Display for RouteConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAnObject => f.write_str("route config must be an object"),
            Self::MissingAt => f.write_str("config.at is missing"),
            Self::At(reason) => write!(f, "config.at is not a payload path: {reason}"),
            Self::MissingCases => f.write_str("config.cases is missing"),
            Self::CasesNotAnObject => f.write_str("config.cases must be an object"),
            Self::Cases(reason) => match reason {
                RouteCasesError::CaseKeyEmpty => {
                    f.write_str("config.cases contains an empty case name")
                }
                RouteCasesError::CaseKeyByte { case, at } => write!(
                    f,
                    "config.cases.{case} has an invalid case-name byte at {at}"
                ),
                RouteCasesError::PortName { case, source } => write!(
                    f,
                    "config.cases.{case} cannot form an outlet name: {source}"
                ),
                RouteCasesError::DuplicateMatchKey { first, second } => write!(
                    f,
                    "config.cases.{first} and config.cases.{second} select the same value"
                ),
            },
        }
    }
}

#[must_use]
pub fn select<'value>(payload: &'value Value, at: &PayloadPath) -> Option<&'value Value> {
    let mut current = payload;
    for segment in at.segments() {
        current = match segment {
            Segment::Key(key) => current.as_object()?.get(key.as_str())?,
            Segment::Index(index) => {
                let index = usize::try_from(*index).ok()?;
                current.as_array()?.get(index)?
            }
        };
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PayloadRoot, payload_path_from_value};
    use circular_expr::ValuePath;

    fn path(segments: &[Value]) -> PayloadPath {
        payload_path_from_value(&Value::array(segments.to_vec())).expect("the test path is valid")
    }

    fn object(entries: &[(&str, Value)]) -> Value {
        Value::object(entries.iter().map(|(k, v)| ((*k).to_owned(), v.clone())))
            .expect("test keys are unique")
    }

    fn route_config(at: Value, cases: &[(&str, Value)]) -> Value {
        object(&[
            ("at", at),
            (
                "cases",
                Value::object(cases.iter().map(|(k, v)| ((*k).to_owned(), v.clone())))
                    .expect("test case keys are unique"),
            ),
        ])
    }

    #[test]
    fn a_well_formed_config_yields_the_path_and_the_case_table() {
        let value = route_config(
            Value::array([Value::string("kind")]),
            &[("even", Value::int(0)), ("odd", Value::int(1))],
        );

        let config = RouteConfig::from_value(&value).expect("valid config");

        assert_eq!(config.at().segments().len(), 1);
        let names: Vec<&str> = config.cases().cases().iter().map(RouteCase::name).collect();
        assert_eq!(names, vec!["even", "odd"]);
    }

    #[test]
    fn an_empty_path_is_a_valid_selector_not_a_missing_one() {
        let value = route_config(Value::array([]), &[("all", Value::Null)]);

        let config = RouteConfig::from_value(&value).expect("an empty path is valid");

        assert!(config.at().segments().is_empty());
    }

    #[test]
    fn an_empty_case_object_is_a_valid_config() {
        let value = route_config(Value::array([]), &[]);

        let config = RouteConfig::from_value(&value).expect("empty cases are valid");

        assert!(config.cases().cases().is_empty());
    }

    #[test]
    fn each_malformed_place_names_itself_in_the_rejection() {
        use RouteConfigError as E;

        for (value, expected) in [
            (Value::int(1), E::NotAnObject),
            (object(&[("cases", object(&[]))]), E::MissingAt),
            (object(&[("at", Value::array([]))]), E::MissingCases),
            (
                object(&[("at", Value::array([])), ("cases", Value::int(1))]),
                E::CasesNotAnObject,
            ),
        ] {
            assert_eq!(RouteConfig::from_value(&value).unwrap_err(), expected);
        }

        let bad_path = object(&[("at", Value::string("a.b")), ("cases", object(&[]))]);
        assert!(matches!(
            RouteConfig::from_value(&bad_path).unwrap_err(),
            E::At(PayloadPathError::NotAnArray { .. })
        ));

        let dupe = route_config(
            Value::array([]),
            &[("one", Value::int(1)), ("two", Value::int(1))],
        );
        assert!(matches!(
            RouteConfig::from_value(&dupe).unwrap_err(),
            E::Cases(RouteCasesError::DuplicateMatchKey { .. })
        ));
    }

    #[test]
    fn values_of_different_kinds_are_different_keys() {
        let cases = RouteCases::try_new([
            ("int", Value::int(1)),
            ("float", Value::float(1.0)),
            ("text", Value::string("1")),
            ("flag", Value::bool(true)),
        ])
        .expect("the four values differ under structural equality");

        assert_eq!(cases.cases().len(), 4);
        for (name, value) in [
            ("int", Value::int(1)),
            ("float", Value::float(1.0)),
            ("text", Value::string("1")),
            ("flag", Value::bool(true)),
        ] {
            assert_eq!(cases.match_case(&value).expect("it matches").name(), name);
        }
    }

    #[test]
    fn the_float_special_values_behave_as_the_value_model_decided() {
        let cases = RouteCases::try_new([
            ("nan", Value::float(f64::NAN)),
            ("plus_zero", Value::float(0.0)),
            ("minus_zero", Value::float(-0.0)),
        ])
        .expect("the three differ bitwise");

        assert_eq!(
            cases.match_case(&Value::float(f64::NAN)).unwrap().name(),
            "nan"
        );
        assert_eq!(
            cases.match_case(&Value::float(0.0)).unwrap().name(),
            "plus_zero"
        );
        assert_eq!(
            cases.match_case(&Value::float(-0.0)).unwrap().name(),
            "minus_zero"
        );
    }

    #[test]
    fn two_cases_claiming_the_same_value_reject_the_whole_config() {
        let error = RouteCases::try_new([
            ("first", object(&[("a", Value::int(1))])),
            ("second", object(&[("a", Value::int(1))])),
        ])
        .expect_err("two values equal under structural equality are a duplicate declaration");

        assert_eq!(
            error,
            RouteCasesError::DuplicateMatchKey {
                first: "first".to_owned(),
                second: "second".to_owned(),
            }
        );
    }

    #[test]
    fn array_order_matters_and_object_construction_order_does_not() {
        let reversed = RouteCases::try_new([
            ("ascending", Value::array([Value::int(1), Value::int(2)])),
            ("descending", Value::array([Value::int(2), Value::int(1)])),
        ])
        .expect("a different element order is a different value");
        assert_eq!(reversed.cases().len(), 2);

        let duplicate = RouteCases::try_new([
            ("one", object(&[("a", Value::int(1)), ("b", Value::int(2))])),
            ("two", object(&[("b", Value::int(2)), ("a", Value::int(1))])),
        ]);
        assert!(
            duplicate.is_err(),
            "objects built in a different order are the same value, so this is a duplicate"
        );
    }

    #[test]
    fn expansion_order_is_the_case_name_byte_order_not_the_insertion_order() {
        let inserted = RouteCases::try_new([
            ("z", Value::int(4)),
            ("ab", Value::int(3)),
            ("a1", Value::int(1)),
            ("a_b", Value::int(2)),
        ])
        .unwrap();

        let names: Vec<&str> = inserted.cases().iter().map(RouteCase::name).collect();
        assert_eq!(names, vec!["a1", "a_b", "ab", "z"]);

        let shuffled = RouteCases::try_new([
            ("a1", Value::int(1)),
            ("z", Value::int(4)),
            ("a_b", Value::int(2)),
            ("ab", Value::int(3)),
        ])
        .unwrap();
        assert_eq!(inserted, shuffled);
    }

    #[test]
    fn a_case_name_that_cannot_become_a_port_rejects_the_config() {
        for rejected in ["Not A Port", "Mango", "with space", "café", "a-b", "a.b"] {
            let error = RouteCases::try_new([(rejected, Value::int(1))]).unwrap_err();
            assert!(
                matches!(error, RouteCasesError::CaseKeyByte { ref case, .. } if case == rejected),
                "{rejected:?} is outside the no-conversion check, yet {error:?} came out"
            );
        }

        assert!(PortId::try_derived(ROUTE_PREFIX.to_owned()).is_ok());
        assert_eq!(
            RouteCases::try_new([("", Value::int(1))]).unwrap_err(),
            RouteCasesError::CaseKeyEmpty
        );

        let long = "a".repeat(33);
        assert!(matches!(
            RouteCases::try_new([(long.as_str(), Value::int(1))]).unwrap_err(),
            RouteCasesError::PortName { .. }
        ));
    }

    #[test]
    fn every_case_outlet_carries_the_route_prefix() {
        let cases = RouteCases::try_new([("even", Value::int(0)), ("odd", Value::int(1))]).unwrap();

        let ports: Vec<&str> = cases
            .cases()
            .iter()
            .map(|case| case.port().as_str())
            .collect();
        assert_eq!(ports, vec!["route_even", "route_odd"]);
        assert!(!ports.contains(&UNMATCHED_PORT));
    }

    #[test]
    fn an_empty_case_table_sends_everything_to_unmatched() {
        let cases = RouteCases::empty();

        assert!(cases.cases().is_empty());
        assert!(cases.match_case(&Value::Null).is_none());
        assert!(cases.match_case(&Value::int(7)).is_none());
    }

    #[test]
    fn the_empty_path_selects_the_whole_payload() {
        let payload = object(&[("a", Value::int(1))]);
        let whole = ValuePath::new(PayloadRoot, []);

        assert_eq!(select(&payload, &whole), Some(&payload));
    }

    #[test]
    fn a_path_walks_keys_and_indices_to_the_exact_place() {
        let payload = object(&[(
            "items",
            Value::array([object(&[("id", Value::string("x"))]), Value::int(9)]),
        )]);

        let at = path(&[Value::string("items"), Value::int(0), Value::string("id")]);
        assert_eq!(select(&payload, &at), Some(&Value::string("x")));

        let second = path(&[Value::string("items"), Value::int(1)]);
        assert_eq!(select(&payload, &second), Some(&Value::int(9)));
    }

    #[test]
    fn a_selected_null_is_a_value_not_an_absence() {
        let payload = object(&[("maybe", Value::Null)]);
        let at = path(&[Value::string("maybe")]);

        assert_eq!(select(&payload, &at), Some(&Value::Null));
        let cases = RouteCases::try_new([("nothing", Value::Null)]).unwrap();
        assert_eq!(
            cases
                .match_case(select(&payload, &at).unwrap())
                .unwrap()
                .name(),
            "nothing"
        );
    }

    #[test]
    fn a_large_index_that_no_platform_can_address_is_a_miss_not_a_panic() {
        let payload = Value::array([Value::int(1)]);
        let at = ValuePath::new(PayloadRoot, [Segment::index(u64::MAX)]);

        assert_eq!(select(&payload, &at), None);
    }
}
