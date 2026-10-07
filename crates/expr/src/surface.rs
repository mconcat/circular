
use std::borrow::Cow;

use cel::common::types::{self, CelBool, CelBytes, CelDouble, CelInt, CelString, CelUInt};
use cel::common::value::Val;
use cel::{Env, ExecutionError};

pub const PROFILE_SURFACE: [(&str, &[usize]); 9] = [
    ("size", &[1]),
    ("startsWith", &[2]),
    ("endsWith", &[2]),
    ("contains", &[2]),
    ("int", &[1]),
    ("double", &[1]),
    ("string", &[1]),
    ("bytes", &[1]),
    ("uint", &[1]),
];

#[must_use]
pub fn profile_env() -> Env {
    let mut env = Env::default();
    register_size(&mut env);
    register_string_predicates(&mut env);
    register_conversions(&mut env);
    crate::comparison::register(&mut env);
    env
}

pub(crate) static PROFILE_ENV: std::sync::LazyLock<std::sync::Arc<Env>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(profile_env()));

pub(crate) static SURFACE_NAMES: std::sync::LazyLock<
    std::collections::BTreeMap<String, Box<[usize]>>,
> = std::sync::LazyLock::new(|| {
    PROFILE_SURFACE
        .iter()
        .map(|(name, arities)| ((*name).to_owned(), Box::from(*arities)))
        .collect()
});

fn register_size(env: &mut Env) {
    env.add_overload("size", "size_string", vec![types::STRING_TYPE], size_string)
        .expect("size_string overload id is unique in this environment");
    env.add_member_overload(
        "size",
        "string_size",
        types::STRING_TYPE,
        Vec::new(),
        size_string,
    )
    .expect("string_size overload id is unique in this environment");

    env.add_overload("size", "size_list", vec![types::LIST_TYPE], size_sizer)
        .expect("size_list overload id is unique in this environment");
    env.add_member_overload(
        "size",
        "list_size",
        types::LIST_TYPE,
        Vec::new(),
        size_sizer,
    )
    .expect("list_size overload id is unique in this environment");

    env.add_overload("size", "size_map", vec![types::MAP_TYPE], size_sizer)
        .expect("size_map overload id is unique in this environment");
    env.add_member_overload("size", "map_size", types::MAP_TYPE, Vec::new(), size_sizer)
        .expect("map_size overload id is unique in this environment");

    env.add_overload("size", "size_bytes", vec![types::BYTES_TYPE], size_sizer)
        .expect("size_bytes overload id is unique in this environment");
    env.add_member_overload(
        "size",
        "bytes_size",
        types::BYTES_TYPE,
        Vec::new(),
        size_sizer,
    )
    .expect("bytes_size overload id is unique in this environment");
}

fn size_sizer(args: Vec<Cow<'_, dyn Val>>) -> Result<Cow<'_, dyn Val>, ExecutionError> {
    let target = args.first().ok_or_else(|| ExecutionError::UnexpectedType {
        got: "none".to_owned(),
        want: "sizer".to_owned(),
    })?;
    let sizer = target
        .as_sizer()
        .ok_or_else(|| ExecutionError::UnexpectedType {
            got: target.get_type().name().to_owned(),
            want: "list · map · bytes".to_owned(),
        })?;
    let result: Box<dyn Val> = Box::new(sizer.size());
    Ok(Cow::Owned(result))
}

fn register_string_predicates(env: &mut Env) {
    for (name, id, op) in [
        (
            "startsWith",
            "string_starts_with",
            string_starts_with as StringPredicate,
        ),
        ("endsWith", "string_ends_with", string_ends_with),
        ("contains", "string_contains", string_contains),
    ] {
        env.add_member_overload(name, id, types::STRING_TYPE, vec![types::STRING_TYPE], op)
            .expect("predicate overload id is unique in this environment");
    }
}

type StringPredicate = fn(Vec<Cow<'_, dyn Val>>) -> Result<Cow<'_, dyn Val>, ExecutionError>;

fn two_strings<'a>(args: &'a [Cow<'a, dyn Val>]) -> Result<(&'a str, &'a str), ExecutionError> {
    let text = |index: usize| -> Result<&'a str, ExecutionError> {
        args.get(index)
            .and_then(|arg| arg.downcast_ref::<CelString>())
            .map(CelString::inner)
            .ok_or_else(|| ExecutionError::UnexpectedType {
                got: args
                    .get(index)
                    .map_or_else(|| "none".to_owned(), |arg| arg.get_type().name().to_owned()),
                want: "string".to_owned(),
            })
    };
    Ok((text(0)?, text(1)?))
}

fn boolean(value: bool) -> Cow<'static, dyn Val> {
    let result: Box<dyn Val> = Box::new(CelBool::from(value));
    Cow::Owned(result)
}

fn string_starts_with(args: Vec<Cow<'_, dyn Val>>) -> Result<Cow<'_, dyn Val>, ExecutionError> {
    let (subject, prefix) = two_strings(&args)?;
    Ok(boolean(subject.starts_with(prefix)))
}

fn string_ends_with(args: Vec<Cow<'_, dyn Val>>) -> Result<Cow<'_, dyn Val>, ExecutionError> {
    let (subject, suffix) = two_strings(&args)?;
    Ok(boolean(subject.ends_with(suffix)))
}

fn string_contains(args: Vec<Cow<'_, dyn Val>>) -> Result<Cow<'_, dyn Val>, ExecutionError> {
    let (subject, needle) = two_strings(&args)?;
    Ok(boolean(subject.contains(needle)))
}

fn register_conversions(env: &mut Env) {
    let arms: [(&str, &str, types::Type, StringPredicate); 17] = [
        ("int", "int_int", types::INT_TYPE, to_int),
        ("int", "int_double", types::DOUBLE_TYPE, to_int),
        ("int", "int_uint", types::UINT_TYPE, to_int),
        ("int", "int_string", types::STRING_TYPE, to_int),
        ("uint", "uint_uint", types::UINT_TYPE, to_uint),
        ("uint", "uint_int", types::INT_TYPE, to_uint),
        ("uint", "uint_string", types::STRING_TYPE, to_uint),
        ("double", "double_double", types::DOUBLE_TYPE, to_double),
        ("double", "double_int", types::INT_TYPE, to_double),
        ("double", "double_uint", types::UINT_TYPE, to_double),
        ("double", "double_string", types::STRING_TYPE, to_double),
        ("string", "string_string", types::STRING_TYPE, to_string),
        ("string", "string_int", types::INT_TYPE, to_string),
        ("string", "string_uint", types::UINT_TYPE, to_string),
        ("string", "string_double", types::DOUBLE_TYPE, to_string),
        ("string", "string_bytes", types::BYTES_TYPE, string_of_bytes),
        ("bytes", "bytes_string", types::STRING_TYPE, to_bytes),
    ];
    for (name, id, argument, op) in arms {
        env.add_overload(name, id, vec![argument], op)
            .expect("conversion overload id is unique in this environment");
    }
}

fn to_int(args: Vec<Cow<'_, dyn Val>>) -> Result<Cow<'_, dyn Val>, ExecutionError> {
    let subject = args.first().ok_or_else(|| ExecutionError::UnexpectedType {
        got: "none".to_owned(),
        want: "int · uint · double".to_owned(),
    })?;
    let value = if let Some(inner) = subject.downcast_ref::<CelInt>() {
        *inner.inner()
    } else if let Some(inner) = subject.downcast_ref::<CelUInt>() {
        i64::try_from(*inner.inner()).map_err(|_| ExecutionError::UnexpectedType {
            got: "uint outside the int range".to_owned(),
            want: "int".to_owned(),
        })?
    } else if let Some(inner) = subject.downcast_ref::<CelDouble>() {
        let raw = *inner.inner();
        if !raw.is_finite() || raw <= -(2f64.powi(63)) || raw >= 2f64.powi(63) {
            return Err(ExecutionError::UnexpectedType {
                got: "double outside the int range".to_owned(),
                want: "int".to_owned(),
            });
        }
        raw.trunc() as i64
    } else if let Some(inner) = subject.downcast_ref::<CelString>() {
        parse_text("int", inner.inner())?
    } else {
        return Err(ExecutionError::UnexpectedType {
            got: subject.get_type().name().to_owned(),
            want: "int · uint · double · string".to_owned(),
        });
    };
    let result: Box<dyn Val> = Box::new(CelInt::from(value));
    Ok(Cow::Owned(result))
}

fn parse_text<T>(function: &str, text: &str) -> Result<T, ExecutionError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    text.parse::<T>()
        .map_err(|error| ExecutionError::FunctionError {
            function: function.to_owned(),
            message: format!("string parse error: {error}"),
        })
}

fn to_uint(args: Vec<Cow<'_, dyn Val>>) -> Result<Cow<'_, dyn Val>, ExecutionError> {
    let subject = args.first().ok_or_else(|| ExecutionError::UnexpectedType {
        got: "none".to_owned(),
        want: "int · uint".to_owned(),
    })?;
    let value = if let Some(inner) = subject.downcast_ref::<CelUInt>() {
        *inner.inner()
    } else if let Some(inner) = subject.downcast_ref::<CelInt>() {
        u64::try_from(*inner.inner()).map_err(|_| ExecutionError::UnexpectedType {
            got: "negative int".to_owned(),
            want: "uint".to_owned(),
        })?
    } else if let Some(inner) = subject.downcast_ref::<CelString>() {
        parse_text("uint", inner.inner())?
    } else {
        return Err(ExecutionError::UnexpectedType {
            got: subject.get_type().name().to_owned(),
            want: "int · uint · string".to_owned(),
        });
    };
    let result: Box<dyn Val> = Box::new(CelUInt::from(value));
    Ok(Cow::Owned(result))
}

fn to_double(args: Vec<Cow<'_, dyn Val>>) -> Result<Cow<'_, dyn Val>, ExecutionError> {
    let subject = args.first().ok_or_else(|| ExecutionError::UnexpectedType {
        got: "none".to_owned(),
        want: "int · uint · double".to_owned(),
    })?;
    let value = if let Some(inner) = subject.downcast_ref::<CelDouble>() {
        *inner.inner()
    } else if let Some(inner) = subject.downcast_ref::<CelInt>() {
        *inner.inner() as f64
    } else if let Some(inner) = subject.downcast_ref::<CelUInt>() {
        *inner.inner() as f64
    } else if let Some(inner) = subject.downcast_ref::<CelString>() {
        parse_text("double", inner.inner())?
    } else {
        return Err(ExecutionError::UnexpectedType {
            got: subject.get_type().name().to_owned(),
            want: "int · uint · double · string".to_owned(),
        });
    };
    let result: Box<dyn Val> = Box::new(CelDouble::from(value));
    Ok(Cow::Owned(result))
}

fn to_string(args: Vec<Cow<'_, dyn Val>>) -> Result<Cow<'_, dyn Val>, ExecutionError> {
    let subject = args.first().ok_or_else(|| ExecutionError::UnexpectedType {
        got: "none".to_owned(),
        want: "string · int · uint · double".to_owned(),
    })?;
    let value = if let Some(inner) = subject.downcast_ref::<CelString>() {
        inner.inner().to_owned()
    } else if let Some(inner) = subject.downcast_ref::<CelInt>() {
        inner.inner().to_string()
    } else if let Some(inner) = subject.downcast_ref::<CelUInt>() {
        inner.inner().to_string()
    } else if let Some(inner) = subject.downcast_ref::<CelDouble>() {
        inner.inner().to_string()
    } else {
        return Err(ExecutionError::UnexpectedType {
            got: subject.get_type().name().to_owned(),
            want: "string · int · uint · double".to_owned(),
        });
    };
    let result: Box<dyn Val> = Box::new(CelString::from(value));
    Ok(Cow::Owned(result))
}

fn string_of_bytes(args: Vec<Cow<'_, dyn Val>>) -> Result<Cow<'_, dyn Val>, ExecutionError> {
    let subject = args.first().ok_or_else(|| ExecutionError::UnexpectedType {
        got: "none".to_owned(),
        want: "bytes".to_owned(),
    })?;
    let Some(inner) = subject.downcast_ref::<CelBytes>() else {
        return Err(ExecutionError::UnexpectedType {
            got: subject.get_type().name().to_owned(),
            want: "bytes".to_owned(),
        });
    };
    let text = std::str::from_utf8(inner.inner()).map_err(|_| ExecutionError::UnexpectedType {
        got: "invalid UTF-8 bytes".to_owned(),
        want: "string".to_owned(),
    })?;
    let result: Box<dyn Val> = Box::new(CelString::from(text.to_owned()));
    Ok(Cow::Owned(result))
}

fn to_bytes(args: Vec<Cow<'_, dyn Val>>) -> Result<Cow<'_, dyn Val>, ExecutionError> {
    let subject = args
        .first()
        .and_then(|arg| arg.downcast_ref::<CelString>())
        .ok_or_else(|| ExecutionError::UnexpectedType {
            got: args
                .first()
                .map_or_else(|| "none".to_owned(), |arg| arg.get_type().name().to_owned()),
            want: "string".to_owned(),
        })?;
    let result: Box<dyn Val> = Box::new(CelBytes::from(subject.inner().as_bytes().to_vec()));
    Ok(Cow::Owned(result))
}

fn size_string(args: Vec<Cow<'_, dyn Val>>) -> Result<Cow<'_, dyn Val>, ExecutionError> {
    let subject = args
        .first()
        .and_then(|arg| arg.downcast_ref::<CelString>())
        .ok_or_else(|| ExecutionError::UnexpectedType {
            got: args
                .first()
                .map_or_else(|| "none".to_owned(), |arg| arg.get_type().name().to_owned()),
            want: "string".to_owned(),
        })?;

    let count = i64::try_from(subject.inner().chars().count()).map_err(|_| {
        ExecutionError::InternalError("string length exceeds the int range".to_owned())
    })?;

    let result: Box<dyn Val> = Box::new(CelInt::from(count));
    Ok(Cow::Owned(result))
}

#[cfg(test)]
mod tests {
    use super::profile_env;
    use cel::{Context, Program, Value};
    use std::sync::Arc;

    fn eval(source: &str) -> Result<Value, String> {
        let context = Context::with_env(Arc::new(profile_env()));
        Program::compile(source)
            .map_err(|error| error.to_string())?
            .execute(&context)
            .map_err(|error| error.to_string())
    }

    #[test]
    fn size_counts_unicode_code_points_not_utf8_bytes() {
        for (source, expected) in [
            ("size('')", 0i64),
            ("size('A')", 1),
            ("size('ÿ')", 1),
            ("size('four')", 4),
            ("size('πέντε')", 5),
            ("size('€→✓')", 3),
            ("size('😀')", 1),
        ] {
            assert_eq!(eval(source), Ok(expected.into()), "{source}");
        }
    }

    #[test]
    fn size_is_open_in_both_call_forms() {
        assert_eq!(eval("size('€→✓')"), eval("'€→✓'.size()"));
    }

    #[test]
    fn string_of_bytes_decodes_utf8_and_refuses_the_invalid() {
        assert_eq!(eval("string(b'four')"), Ok("four".into()));
        assert_eq!(eval("string(bytes('café'))"), Ok("café".into()));
        assert!(
            eval("string(b'\\xff\\xfe')").is_err(),
            "invalid UTF-8 is an error, not a value"
        );
    }

    #[test]
    fn unregistered_names_do_not_exist() {
        for source in [
            "'ab'.matches('a')",
            "timestamp('2026-01-01T00:00:00Z')",
            "duration('1h')",
            "optional.of(1)",
            "[1, 2].max()",
        ] {
            let outcome = eval(source);
            assert!(outcome.is_err(), "{source} is open: {outcome:?}");
        }
    }

    #[test]
    fn every_arm_of_an_open_name_has_a_body() {
        for (source, expected) in [
            ("size([1, 2])", 2i64),
            ("[1, 2].size()", 2),
            ("size({'a': 1})", 1),
            ("{'a': 1}.size()", 1),
            ("size(b'abc')", 3),
            ("b'abc'.size()", 3),
        ] {
            assert_eq!(eval(source), Ok(expected.into()), "{source}");
        }
    }

    #[test]
    fn the_opened_seven_answer() {
        for (source, expected) in [
            ("'foobar'.startsWith('foo')", true.into()),
            ("'foobar'.startsWith('bar')", false.into()),
            ("'foobar'.endsWith('bar')", true.into()),
            ("'foobar'.contains('oob')", true.into()),
            ("'foobar'.contains('zzz')", false.into()),
            ("int(1.9)", 1i64.into()),
            ("int(-1.9)", (-1i64).into()),
            ("double(2)", 2.0f64.into()),
            ("string(3)", "3".into()),
            ("string('already')", "already".into()),
            (
                "bytes('ab')",
                cel::Value::Bytes(std::sync::Arc::new(vec![97, 98])),
            ),
        ] {
            assert_eq!(eval(source), Ok(expected), "{source}");
        }
    }

    #[test]
    fn a_double_outside_int_range_is_refused_not_folded() {
        for source in ["int(1.0e19)", "int(-1.0e19)", "int(0.0/0.0)"] {
            assert!(eval(source).is_err(), "{source} produced a value");
        }
    }

    #[test]
    fn language_core_is_not_ours_to_close() {
        assert_eq!(eval("1 + 1"), Ok(2i64.into()));
        assert_eq!(eval("[1, 2, 3].all(x, x > 0)"), Ok(true.into()));
        assert_eq!(eval("has({'a': 1}.a)"), Ok(true.into()));
        assert_eq!(eval("[1, 2].map(x, x * 2)"), Ok(vec![2i64, 4].into()));
    }
}
