//! Evaluates extension constructors on edge-case strings in Rust Cedar.
//! `cargo run --example ext_probe -- 'datetime("0000-01-01")' ...`, or no
//! arguments for the built-in list.

use std::str::FromStr;

use cedar_policy::{Context, Entities, EntityUid, Expression, Request, eval_expression};

const BUILT_IN: &[&str] = &[
    r#"datetime("0000-01-01").toTime()"#,
    r#"datetime("0000-01-01").durationSince(datetime("1970-01-01"))"#,
    r#"datetime("2023-02-29")"#,
    r#"datetime("2024-02-29")"#,
    r#"datetime("2024-04-31")"#,
    r#"datetime("2024-01-01T24:00:00Z")"#,
    r#"datetime("2024-01-01garbage")"#,
    r#"datetime("2024-01-01T10:00:00Z12")"#,
    r#"datetime("9999-12-31T23:59:59.999-2359").durationSince(datetime("1970-01-01"))"#,
    r#"datetime("0000-01-01T00:00:00+2359").durationSince(datetime("1970-01-01"))"#,
    r#"decimal(".5")"#,
    r#"decimal("+1.0")"#,
    r#"decimal("1.")"#,
    r#"decimal("-0.0001")"#,
    r#"decimal("-922337203685477.5808")"#,
    r#"duration("-9223372036854775808ms")"#,
    r#"duration("+1d")"#,
    r#"ip("::1:2:3:4:5:6:7")"#,
    r#"ip("1:2:3:4:5:6:7::")"#,
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let inputs: Vec<&str> =
        if args.is_empty() { BUILT_IN.to_vec() } else { args.iter().map(String::as_str).collect() };
    let uid = EntityUid::from_str(r#"A::"a""#).unwrap();
    let request = Request::new(uid.clone(), uid.clone(), uid, Context::empty(), None).unwrap();
    let entities = Entities::empty();
    for input in inputs {
        let result = match Expression::from_str(input) {
            Ok(expr) => match eval_expression(&request, &entities, &expr) {
                Ok(value) => format!("{value}"),
                Err(e) => format!("error: {e}"),
            },
            Err(e) => format!("parse error: {e}"),
        };
        println!("{input}\n  => {result}");
    }
}
