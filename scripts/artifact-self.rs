use circular_core::{Value, compatibility};
use serde_json::{Value as Json, json};

fn json_value(value: &Value) -> Json {
    match value {
        Value::String(value) => json!(value),
        Value::UInt(value) => json!(value),
        Value::Object(fields) => fields
            .iter()
            .map(|(key, value)| (key.to_string(), json_value(value)))
            .collect(),
        _ => panic!("unsupported compatibility declaration JSON shape"),
    }
}

fn main() {
    let build = circular_transport::build_identity("circular");
    let mut description = json_value(&compatibility::current().to_value());
    description.as_object_mut().unwrap().insert(
        "identity".into(),
        json!({
            "version": build.version,
            "git_sha": build.git_sha,
            "git_dirty": build.git_dirty,
            "commit_date": build.commit_date,
            "target": build.target,
            "architecture": std::env::consts::ARCH,
            "platform": std::env::consts::OS,
        }),
    );
    println!("{}", serde_json::to_string_pretty(&description).unwrap());
}
