//! Test helpers that read the JSON field tables out of `docs/reference.md`, so a
//! test can check a serialized line against what the docs say it contains.
//!
//! The docs are hand-written, so a field renamed or reordered in code, or in the
//! docs alone, would otherwise go unnoticed. These helpers read the markdown by
//! its shape (headings, pipe tables, fenced `json` blocks) and panic with the
//! section and kind they were looking for when that shape is not found.

use std::fmt;

use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};
use serde::Deserialize;

/// The reference the tables are read from.
const REFERENCE: &str = include_str!("../../docs/reference.md");

/// The keys of a JSON object, in the order they were written.
///
/// `serde_json::Value` sorts keys, so it cannot tell the order a line has them in.
struct Keys(Vec<String>);

impl<'de> Deserialize<'de> for Keys {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeysVisitor;

        impl<'de> Visitor<'de> for KeysVisitor {
            type Value = Keys;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Keys, A::Error> {
                let mut keys = Vec::new();
                while let Some((key, IgnoredAny)) = map.next_entry::<String, IgnoredAny>()? {
                    keys.push(key);
                }
                Ok(Keys(keys))
            }
        }

        deserializer.deserialize_map(KeysVisitor)
    }
}

/// The keys of the JSON object `line`, in order.
pub(super) fn json_keys(line: &str) -> Vec<String> {
    match serde_json::from_str::<Keys>(line) {
        Ok(Keys(keys)) => keys,
        Err(error) => panic!("not a JSON object ({error}): {line}"),
    }
}

/// The body of the section headed `heading` (any level), up to the next heading of
/// the same or a higher level.
pub(super) fn section<'a>(markdown: &'a str, heading: &str) -> &'a str {
    let mut level = 0;
    let mut start = None;
    let mut in_fence = false;
    let mut offset = 0;
    for line in markdown.split_inclusive('\n') {
        if line.starts_with("```") {
            in_fence = !in_fence;
        }
        let hashes = line.bytes().take_while(|&b| b == b'#').count();
        if !in_fence && hashes > 0 && line[hashes..].starts_with(' ') {
            match start {
                None if line[hashes..].trim() == heading => {
                    level = hashes;
                    start = Some(offset + line.len());
                }
                Some(begin) if hashes <= level => return &markdown[begin..offset],
                _ => {}
            }
        }
        offset += line.len();
    }
    match start {
        Some(begin) => &markdown[begin..],
        None => panic!("no section headed `{heading}`"),
    }
}

/// A section of `docs/reference.md`.
pub(super) fn reference_section(heading: &str) -> &'static str {
    section(REFERENCE, heading)
}

/// The cells of a table row, trimmed. A `|` inside backticks is not supported.
fn cells(row: &str) -> Vec<&str> {
    row.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}

/// A pipe table: its header cells and its data rows (the `---` rule is left out).
type Table<'a> = (Vec<&'a str>, Vec<Vec<&'a str>>);

/// Every pipe table in `section`.
fn tables(section: &str) -> Vec<Table<'_>> {
    let mut tables = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in section.lines().chain(std::iter::once("")) {
        if line.trim_start().starts_with('|') {
            current.push(line);
        } else if !current.is_empty() {
            let header = cells(current[0]);
            let rows = std::mem::take(&mut current)
                .into_iter()
                .skip(2)
                .map(cells)
                .collect();
            tables.push((header, rows));
        }
    }
    tables
}

/// `cell` without the backticks around it.
fn unticked(cell: &str) -> &str {
    cell.trim_matches('`')
}

/// The first backticked token in `text`, without its backticks.
fn first_code(text: &str) -> Option<&str> {
    let rest = &text[text.find('`')? + 1..];
    Some(&rest[..rest.find('`')?])
}

/// A row of a field table: the field, its `Type` cell and its `Meaning` cell.
#[derive(Clone)]
pub(super) struct Field {
    pub(super) name: String,
    pub(super) ty: String,
    pub(super) meaning: String,
}

impl Field {
    fn from_row(row: &[&str]) -> Self {
        assert!(
            row.len() >= 3,
            "a field table row without name, type and meaning: {row:?}"
        );
        Self {
            name: unticked(row[0]).to_owned(),
            // `number or `null``: the backticks are markup, not part of the type.
            ty: row[1].replace('`', ""),
            meaning: row[2].to_owned(),
        }
    }
}

/// The rows of the table in `section` that documents the line whose `kind` is
/// `kind`: the one with a `kind` row that says `"<kind>"`.
pub(super) fn field_rows(section: &str, kind: &str) -> Vec<Field> {
    let quoted = format!("\"{kind}\"");
    let found = tables(section).into_iter().find(|(_, rows)| {
        rows.iter()
            .any(|row| row.len() >= 3 && unticked(row[0]) == "kind" && row[2].contains(&quoted))
    });
    match found {
        Some((_, rows)) => rows.iter().map(|row| Field::from_row(row)).collect(),
        None => panic!("no field table for kind `{kind}` in the section"),
    }
}

/// The rows of the first table of `section`.
pub(super) fn first_table_rows(section: &str) -> Vec<Field> {
    match tables(section).into_iter().next() {
        Some((_, rows)) => rows.iter().map(|row| Field::from_row(row)).collect(),
        None => panic!("no table in the section"),
    }
}

/// The field names of the table in `section` that documents the line whose `kind`
/// is `kind`.
pub(super) fn table_fields(section: &str, kind: &str) -> Vec<String> {
    field_rows(section, kind)
        .into_iter()
        .map(|field| field.name)
        .collect()
}

/// The field names in the first column of the first table of `section`, whatever
/// its other columns are.
pub(super) fn first_table_fields(section: &str) -> Vec<String> {
    match tables(section).into_iter().next() {
        Some((_, rows)) => rows.iter().map(|row| unticked(row[0]).to_owned()).collect(),
        None => panic!("no table in the section"),
    }
}

/// Whether `value` is what `phrase`, a cell of the `Type` column, says.
fn conforms(phrase: &str, value: &serde_json::Value) -> bool {
    use serde_json::Value;
    match phrase {
        "string" => value.is_string(),
        "number" | "numbers" => value.is_number(),
        "number or null" => value.is_number() || value.is_null(),
        "array of strings" => value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string)),
        "array of objects" => value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_object)),
        other => panic!("a type phrase this helper does not know: `{other}`"),
    }
}

/// The value a `Meaning` cell fixes with ``Always `"x"` `` (or ``Always `3` ``), if it
/// does: the backticked JSON that follows `Always`.
fn constant(meaning: &str) -> Option<serde_json::Value> {
    let code = first_code(meaning.strip_prefix("Always ")?)?;
    Some(
        serde_json::from_str(code)
            .unwrap_or_else(|error| panic!("`Always {code}` is not JSON ({error})")),
    )
}

/// Panics unless `value` is of the type `phrase` names; `what` says whose it is.
pub(super) fn check_type(phrase: &str, value: &serde_json::Value, what: &str) {
    assert!(
        conforms(phrase, value),
        "{what}: the docs say `{phrase}`, the value is {value}"
    );
}

/// Panics unless every documented field of `rows` is in the object `value`, of the
/// documented type and, where the docs say `Always "x"`, equal to it.
pub(super) fn check_fields(rows: &[Field], value: &serde_json::Value, what: &str) {
    for field in rows {
        let name = &field.name;
        let Some(actual) = value.get(name) else {
            panic!("{what}: the docs list `{name}`, the object has no such field: {value}");
        };
        check_type(&field.ty, actual, &format!("{what}: `{name}`"));
        if let Some(fixed) = constant(&field.meaning) {
            assert_eq!(*actual, fixed, "{what}: `{name}` is always `{fixed}`");
        }
    }
}

/// Panics unless `ordinals`, one per sample, cover `0..variants`: the check that a
/// test has a sample for every variant of an enum.
///
/// The test pairs it with a `match` on the enum with no `_` arm that numbers each
/// variant, so a new variant does not compile until it is numbered here.
pub(super) fn assert_every_variant(
    ordinals: impl IntoIterator<Item = usize>,
    variants: usize,
    what: &str,
) {
    let seen: std::collections::BTreeSet<usize> = ordinals.into_iter().collect();
    let all: std::collections::BTreeSet<usize> = (0..variants).collect();
    assert_eq!(seen, all, "{what}: the samples miss a variant");
}

/// The (`kind`, `code`) pairs of the table in `section` whose header is `kind` and
/// `code`.
pub(super) fn kind_codes(section: &str) -> Vec<(String, u8)> {
    let Some((_, rows)) = tables(section).into_iter().find(|(header, _)| {
        header.len() == 2 && unticked(header[0]) == "kind" && unticked(header[1]) == "code"
    }) else {
        panic!("no table headed `kind` and `code` in the section");
    };
    rows.iter()
        .map(|row| {
            assert!(
                row.len() == 2,
                "a `kind` and `code` row without 2 cells: {row:?}"
            );
            let code = unticked(row[1]);
            let code = code
                .parse()
                .unwrap_or_else(|_| panic!("`{code}` is not an exit code"));
            (unticked(row[0]).to_owned(), code)
        })
        .collect()
}

/// The gates the table headed `gate` in `section` lists, with the (field, type)
/// pairs each has: ``` `a`, `b` (numbers) ``` gives `a` and `b` the type `numbers`.
/// Backticks inside a type, as in ``number or `null` ``, are dropped.
pub(super) fn gate_rows(section: &str) -> Vec<(String, Vec<(String, String)>)> {
    let Some((_, rows)) = tables(section)
        .into_iter()
        .find(|(header, _)| header.first().is_some_and(|h| unticked(h) == "gate"))
    else {
        panic!("no table headed `gate` in the section");
    };
    rows.iter()
        .map(|row| {
            assert!(
                row.len() >= 2,
                "a `gate` row without a Fields cell: {row:?}"
            );
            let mut fields = Vec::new();
            // Each group is names, then `(type`: the text is cut at every `)`.
            for group in row[1].split(')') {
                let Some((names, ty)) = group.split_once('(') else {
                    continue;
                };
                let mut rest = names;
                while let Some(name) = first_code(rest) {
                    fields.push((name.to_owned(), ty.replace('`', "").trim().to_owned()));
                    let after = rest.find('`').unwrap_or(0) + name.len() + 2;
                    rest = &rest[after..];
                }
            }
            assert!(!fields.is_empty(), "no typed field in `{}`", row[1]);
            (unticked(row[0]).to_owned(), fields)
        })
        .collect()
}

/// The kinds the table headed `kind` in `section` lists, with the fields each
/// adds, read from the last cell: the first backticked name of each
/// `;`-separated clause.
pub(super) fn kind_rows(section: &str) -> Vec<(String, Vec<String>)> {
    let Some((_, rows)) = tables(section)
        .into_iter()
        .find(|(header, _)| header.first().is_some_and(|h| unticked(h) == "kind"))
    else {
        panic!("no table headed `kind` in the section");
    };
    rows.iter()
        .map(|row| {
            assert!(
                row.len() == 3,
                "a `kind` table row without 3 cells: {row:?}"
            );
            let fields = row[2]
                .split(';')
                .map(|clause| {
                    first_code(clause)
                        .unwrap_or_else(|| panic!("no field name in `{clause}`"))
                        .to_owned()
                })
                .collect();
            (unticked(row[0]).to_owned(), fields)
        })
        .collect()
}

/// The single-line `json` example in `section` whose `kind` is `kind`, if the
/// section has one. A line that is not a JSON object is not an example.
pub(super) fn example_line<'a>(section: &'a str, kind: &str) -> Option<&'a str> {
    let mut in_json = false;
    for line in section.lines() {
        if line.starts_with("```") {
            in_json = !in_json && line.trim_end() == "```json";
        } else if in_json && line.starts_with('{') {
            let value: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("a `json` example is not JSON ({error}): {line}"));
            if value["kind"] == kind {
                return Some(line);
            }
        }
    }
    None
}

/// The keys of the single-line `json` example for `kind`, in written order.
pub(super) fn example_keys(section: &str, kind: &str) -> Option<Vec<String>> {
    example_line(section, kind).map(json_keys)
}

#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# Top

## One

text

### Inner

| Field | Type | Meaning |
|-------|------|---------|
| `level` | string | Always `\"info\"` |
| `kind` | string | Always `\"thing\"` |
| `n` | number | A count |
| `m` | number or null | A maybe count |
| `names` | array of strings | Some names |

| `gate` | Fields | Meaning |
|--------|--------|---------|
| `g1` | `p`, `q` (numbers) | Both |
| `g2` | `p` (number), `q` (number or `null`) | One each |
| `g3` | `files` (array of strings) | Some |

| `kind` | When | Fields |
|--------|------|--------|
| `a` | now | `x`, the x; `y`, the y |
| `b` | later | `z` |

| `kind` | `code` |
|--------|-------:|
| `a` | `3` |
| `b` | `4` |

```json
{\"level\":\"info\",\"kind\":\"thing\",\"n\":1}
```

```
# not a heading
```

## Two

other
";

    #[test]
    fn json_keys_keep_the_written_order() {
        assert_eq!(json_keys(r#"{"b":1,"a":{"z":0},"c":[1]}"#), ["b", "a", "c"]);
    }

    #[test]
    fn a_section_runs_to_the_next_heading_of_its_level_or_higher() {
        let inner = section(SAMPLE, "Inner");
        assert!(inner.contains("`level`") && inner.contains("# not a heading"));
        assert!(!inner.contains("other"));
        assert!(section(SAMPLE, "One").contains("text"));
        assert_eq!(section(SAMPLE, "Two").trim(), "other");
    }

    #[test]
    #[should_panic(expected = "no section headed `Missing`")]
    fn a_missing_section_is_a_failure() {
        section(SAMPLE, "Missing");
    }

    #[test]
    fn the_table_for_a_kind_is_found_by_its_kind_row() {
        let inner = section(SAMPLE, "Inner");
        let fields = ["level", "kind", "n", "m", "names"];
        assert_eq!(table_fields(inner, "thing"), fields);
        assert_eq!(first_table_fields(inner), fields);
        assert_eq!(
            example_keys(inner, "thing").unwrap(),
            ["level", "kind", "n"]
        );
        assert_eq!(example_keys(inner, "other"), None);
        assert_eq!(
            example_line(inner, "thing"),
            Some(r#"{"level":"info","kind":"thing","n":1}"#)
        );
        assert_eq!(example_line(inner, "other"), None);
    }

    #[test]
    fn kind_rows_take_the_first_name_of_each_clause() {
        let inner = section(SAMPLE, "Inner");
        assert_eq!(
            kind_rows(inner),
            [
                ("a".to_owned(), vec!["x".to_owned(), "y".to_owned()]),
                ("b".to_owned(), vec!["z".to_owned()]),
            ]
        );
    }

    #[test]
    #[should_panic(expected = "no field table for kind `nothing`")]
    fn an_undocumented_kind_is_a_failure() {
        table_fields(section(SAMPLE, "Inner"), "nothing");
    }

    fn thing(extra: &str) -> serde_json::Value {
        let line =
            format!(r#"{{"level":"info","kind":"thing","n":1,"m":null,"names":["x"]{extra}}}"#);
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn a_value_that_fits_the_table_passes() {
        let rows = field_rows(section(SAMPLE, "Inner"), "thing");
        check_fields(&rows, &thing(""), "thing");
        let types: Vec<_> = rows.iter().map(|f| f.ty.as_str()).collect();
        assert_eq!(
            types,
            [
                "string",
                "string",
                "number",
                "number or null",
                "array of strings"
            ]
        );
    }

    #[test]
    #[should_panic(expected = "thing: `n`: the docs say `number`, the value is \"1\"")]
    fn a_value_of_another_type_is_a_failure() {
        let rows = field_rows(section(SAMPLE, "Inner"), "thing");
        let mut value = thing("");
        value["n"] = "1".into();
        check_fields(&rows, &value, "thing");
    }

    #[test]
    #[should_panic(expected = "`level` is always `\"info\"`")]
    fn a_value_other_than_the_constant_is_a_failure() {
        let rows = field_rows(section(SAMPLE, "Inner"), "thing");
        let mut value = thing("");
        value["level"] = "warning".into();
        check_fields(&rows, &value, "thing");
    }

    #[test]
    #[should_panic(expected = "the object has no such field")]
    fn a_documented_field_that_is_missing_is_a_failure() {
        let rows = field_rows(section(SAMPLE, "Inner"), "thing");
        let mut value = thing("");
        value.as_object_mut().unwrap().remove("m");
        check_fields(&rows, &value, "thing");
    }

    #[test]
    fn each_type_phrase_accepts_its_own_values_only() {
        use serde_json::json;
        let cases = [
            ("string", json!("s"), json!(1)),
            ("number", json!(1.5), json!(null)),
            ("numbers", json!(2), json!("2")),
            ("number or null", json!(null), json!("x")),
            ("array of strings", json!(["a"]), json!(["a", 1])),
            ("array of objects", json!([{}]), json!([1])),
        ];
        for (phrase, good, bad) in cases {
            assert!(conforms(phrase, &good), "{phrase}: {good}");
            assert!(!conforms(phrase, &bad), "{phrase}: {bad}");
        }
    }

    #[test]
    #[should_panic(expected = "a type phrase this helper does not know: `integer`")]
    fn an_unknown_type_phrase_is_a_failure() {
        conforms("integer", &serde_json::json!(1));
    }

    #[test]
    fn always_followed_by_a_json_value_is_a_constant() {
        use serde_json::json;
        assert_eq!(
            constant("Always `\"info\"`; more text"),
            Some(json!("info"))
        );
        assert_eq!(constant("Always `3`"), Some(json!(3)));
        assert_eq!(constant("Always `true`"), Some(json!(true)));
        assert_eq!(constant("Always"), None);
        assert_eq!(constant("A count"), None);
    }

    #[test]
    #[should_panic(expected = "`Always info` is not JSON")]
    fn a_constant_that_is_not_json_is_a_failure() {
        constant("Always `info`");
    }

    #[test]
    #[should_panic(expected = "`n` is always `3`")]
    fn a_number_other_than_the_constant_is_a_failure() {
        let rows = vec![Field {
            name: "n".to_owned(),
            ty: "number".to_owned(),
            meaning: "Always `3`".to_owned(),
        }];
        check_fields(&rows, &serde_json::json!({"n": 4}), "thing");
    }

    #[test]
    fn the_kind_and_code_table_is_read_by_its_header() {
        let inner = section(SAMPLE, "Inner");
        assert_eq!(
            kind_codes(inner),
            [("a".to_owned(), 3), ("b".to_owned(), 4)]
        );
    }

    #[test]
    fn the_gate_table_gives_each_field_its_type() {
        let pair = |name: &str, ty: &str| (name.to_owned(), ty.to_owned());
        assert_eq!(
            gate_rows(section(SAMPLE, "Inner")),
            [
                (
                    "g1".to_owned(),
                    vec![pair("p", "numbers"), pair("q", "numbers")]
                ),
                (
                    "g2".to_owned(),
                    vec![pair("p", "number"), pair("q", "number or null")]
                ),
                ("g3".to_owned(), vec![pair("files", "array of strings")]),
            ]
        );
    }

    #[test]
    fn every_variant_needs_a_sample() {
        assert_every_variant([0, 2, 1, 1], 3, "thing");
    }

    #[test]
    #[should_panic(expected = "thing: the samples miss a variant")]
    fn a_variant_without_a_sample_is_a_failure() {
        assert_every_variant([0, 2], 3, "thing");
    }
}
