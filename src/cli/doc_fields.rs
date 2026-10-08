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
pub fn json_keys(line: &str) -> Vec<String> {
    match serde_json::from_str::<Keys>(line) {
        Ok(Keys(keys)) => keys,
        Err(error) => panic!("not a JSON object ({error}): {line}"),
    }
}

/// The body of the section headed `heading` (any level), up to the next heading of
/// the same or a higher level.
pub fn section<'a>(markdown: &'a str, heading: &str) -> &'a str {
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
pub fn reference_section(heading: &str) -> &'static str {
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

/// The field names of the table in `section` that documents the line whose `kind`
/// is `kind`: the one with a `kind` row that says `"<kind>"`.
pub fn table_fields(section: &str, kind: &str) -> Vec<String> {
    let quoted = format!("\"{kind}\"");
    let found = tables(section).into_iter().find(|(_, rows)| {
        rows.iter()
            .any(|row| row.len() >= 3 && unticked(row[0]) == "kind" && row[2].contains(&quoted))
    });
    match found {
        Some((_, rows)) => rows.iter().map(|row| unticked(row[0]).to_owned()).collect(),
        None => panic!("no field table for kind `{kind}` in the section"),
    }
}

/// The field names in the first column of the first table of `section`.
pub fn first_table_fields(section: &str) -> Vec<String> {
    match tables(section).into_iter().next() {
        Some((_, rows)) => rows.iter().map(|row| unticked(row[0]).to_owned()).collect(),
        None => panic!("no table in the section"),
    }
}

/// The kinds the table headed `kind` in `section` lists, with the fields each
/// adds, read from the last cell: the first backticked name of each
/// `;`-separated clause.
pub fn kind_rows(section: &str) -> Vec<(String, Vec<String>)> {
    let Some((_, rows)) = tables(section)
        .into_iter()
        .find(|(header, _)| header.first().is_some_and(|h| unticked(h) == "kind"))
    else {
        panic!("no table headed `kind` in the section");
    };
    rows.iter()
        .map(|row| {
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

/// The keys of the single-line `json` example in `section` whose `kind` is `kind`,
/// if the section has one.
pub fn example_keys(section: &str, kind: &str) -> Option<Vec<String>> {
    let marker = format!("\"kind\":\"{kind}\"");
    let mut in_json = false;
    for line in section.lines() {
        if line.starts_with("```") {
            in_json = !in_json && line.trim_end() == "```json";
        } else if in_json && line.contains(&marker) {
            return Some(json_keys(line));
        }
    }
    None
}

#[cfg(test)]
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

| `kind` | When | Fields |
|--------|------|--------|
| `a` | now | `x`, the x; `y`, the y |
| `b` | later | `z` |

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
        assert_eq!(table_fields(inner, "thing"), ["level", "kind", "n"]);
        assert_eq!(first_table_fields(inner), ["level", "kind", "n"]);
        assert_eq!(
            example_keys(inner, "thing").unwrap(),
            ["level", "kind", "n"]
        );
        assert_eq!(example_keys(inner, "other"), None);
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
}
