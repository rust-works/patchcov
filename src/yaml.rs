//! YAML emission with multi-line strings as readable `|` block scalars.
//!
//! `serde_yaml` quotes and escapes a multi-line string. Routing through the
//! `yaml-rust` emitter instead, with multi-line strings enabled, keeps text such
//! as a markdown comment legible in `--output yaml`.

use anyhow::{Context, Result};
use serde::Serialize;
use yaml_rust_davvid::{Yaml, YamlEmitter};

/// Serializes `data` to a YAML document.
pub fn to_yaml<T: Serialize>(data: &T) -> Result<String> {
    let value = serde_yaml::to_value(data).context("could not serialize to a YAML value")?;
    let value = convert(&value);

    let mut output = String::new();
    let mut emitter = YamlEmitter::new(&mut output);
    emitter.multiline_strings(true);
    emitter.dump(&value).context("could not emit YAML")?;
    Ok(output)
}

/// Converts a `serde_yaml` value to the emitter's representation.
fn convert(value: &serde_yaml::Value) -> Yaml {
    use serde_yaml::Value;
    match value {
        Value::Null => Yaml::Null,
        Value::Bool(b) => Yaml::Boolean(*b),
        Value::Number(n) => n.as_i64().map_or_else(
            || {
                n.as_f64().map_or_else(
                    || Yaml::String(n.to_string()),
                    |f| Yaml::Real(f.to_string()),
                )
            },
            Yaml::Integer,
        ),
        Value::String(s) => Yaml::String(s.clone()),
        Value::Sequence(seq) => Yaml::Array(seq.iter().map(convert).collect()),
        Value::Mapping(map) => {
            let mut hash = yaml_rust_davvid::yaml::Hash::new();
            for (k, v) in map {
                hash.insert(convert(k), convert(v));
            }
            Yaml::Hash(hash)
        }
        Value::Tagged(tagged) => convert(&tagged.value),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct Doc {
        count: u32,
        ratio: f64,
        text: String,
    }

    #[test]
    fn multiline_strings_become_block_scalars() {
        let out = to_yaml(&Doc {
            count: 3,
            ratio: 0.5,
            text: "first\nsecond".to_string(),
        })
        .unwrap();
        assert!(out.contains("count: 3"), "{out}");
        assert!(out.contains("ratio: 0.5"), "{out}");
        assert!(out.contains("text: |"), "{out}");
        assert!(out.contains("first") && out.contains("second"), "{out}");
    }
}
