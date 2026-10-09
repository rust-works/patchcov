//! Relative links and `#anchors` in `README.md`, `CONTRIBUTING.md` and `docs/` must resolve.
//! A renamed heading or a moved file otherwise breaks them silently. The check is offline: it
//! reads the markdown and the repository tree, and ignores external URLs, so it cannot flake
//! on the network. It runs wherever `cargo test --all-targets` does, which includes CI.
//!
//! Anchors follow GitHub's slug rules: the heading text is lowercased, punctuation other than
//! `-` and `_` is dropped, each space becomes `-`, and a repeated slug gets `-1`, `-2`, ...
//! The parsing is deliberately small, not a CommonMark parser: it covers the constructs these
//! files use (ATX headings, inline and reference-style links, `src`/`href` attributes, `<a id>`)
//! and skips fenced code blocks and inline code.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The lines of a markdown file that are prose: fenced code blocks are dropped. Each line is
/// paired with its 1-based number.
fn prose_lines(text: &str) -> Vec<(usize, &str)> {
    let mut fence: Option<char> = None;
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let opener = ["```", "~~~"]
            .into_iter()
            .find(|marker| trimmed.starts_with(marker))
            .and_then(|marker| marker.chars().next());
        match (fence, opener) {
            (None, Some(c)) => fence = Some(c),
            (Some(open), Some(c)) if open == c => fence = None,
            (None, None) => lines.push((index + 1, line)),
            _ => {}
        }
    }
    lines
}

/// GitHub's anchor for a heading's text, before any `-1` suffix for a duplicate.
fn slug(heading: &str) -> String {
    let links = Regex::new(r"\[([^\]]*)\]\([^)]*\)").unwrap();
    let tags = Regex::new(r"<[^>]*>").unwrap();
    let closing = Regex::new(r"\s+#+\s*$").unwrap();
    let text = links.replace_all(heading, "$1");
    let text = tags.replace_all(&text, "");
    let text = closing.replace(text.trim(), "");
    text.chars()
        .filter(|c| !matches!(c, '`' | '*'))
        .flat_map(char::to_lowercase)
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

/// Every anchor a markdown file defines: its headings, and explicit `<a id>` / `<a name>`.
fn anchors(text: &str) -> HashSet<String> {
    let heading = Regex::new(r"^ {0,3}#{1,6}\s+(.*)$").unwrap();
    let explicit = Regex::new(r#"<a\s[^>]*?(?:id|name)\s*=\s*"([^"]*)""#).unwrap();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut found = HashSet::new();
    for (_, line) in prose_lines(text) {
        if let Some(caps) = heading.captures(line) {
            let base = slug(&caps[1]);
            let count = seen.entry(base.clone()).or_insert(0);
            found.insert(if *count == 0 {
                base
            } else {
                format!("{base}-{count}")
            });
            *count += 1;
        }
        for caps in explicit.captures_iter(line) {
            found.insert(caps[1].to_string());
        }
    }
    found
}

/// Every link destination in a markdown file, with its line number. Covers `[text](dest)`,
/// `![alt](dest)`, `[label]: dest` and `src="dest"` / `href="dest"`.
fn destinations(text: &str) -> Vec<(usize, String)> {
    let inline_code = Regex::new(r"`[^`]*`").unwrap();
    let inline = Regex::new(r"\]\(\s*(<[^>]*>|[^)\s]*)").unwrap();
    let reference = Regex::new(r"^ {0,3}\[[^\]]+\]:\s*(<[^>]*>|\S+)").unwrap();
    let attribute = Regex::new(r#"\b(?:src|href)\s*=\s*"([^"]*)""#).unwrap();
    let mut found = Vec::new();
    for (number, line) in prose_lines(text) {
        let line = inline_code.replace_all(line, "");
        let mut push = |dest: &str| {
            let dest = dest.trim_start_matches('<').trim_end_matches('>');
            found.push((number, dest.to_string()));
        };
        for caps in inline.captures_iter(&line) {
            push(&caps[1]);
        }
        if let Some(caps) = reference.captures(&line) {
            push(&caps[1]);
        }
        for caps in attribute.captures_iter(&line) {
            push(&caps[1]);
        }
    }
    found
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether the destination leaves the repository: `https://...`, `mailto:...`, `//host/...`.
fn is_external(dest: &str) -> bool {
    let scheme = Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:").unwrap();
    dest.starts_with("//") || scheme.is_match(dest)
}

/// The problems in `files` (paths relative to `root`), one line each as `file:line: message`.
fn check(root: &Path, files: &[PathBuf]) -> Vec<String> {
    let mut anchor_cache: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    let mut problems = Vec::new();
    for file in files {
        let text = fs::read_to_string(root.join(file)).unwrap();
        for (line, dest) in destinations(&text) {
            if dest.is_empty() || is_external(&dest) {
                continue;
            }
            let (path, fragment) = match dest.split_once('#') {
                Some((path, fragment)) => (path, Some(percent_decode(fragment))),
                None => (dest.as_str(), None),
            };
            let path = percent_decode(path);
            let target = if path.is_empty() {
                root.join(file)
            } else if let Some(from_root) = path.strip_prefix('/') {
                root.join(from_root)
            } else {
                root.join(file).parent().unwrap().join(&path)
            };
            let here = format!("{}:{line}", file.display());
            if !target.exists() {
                problems.push(format!("{here}: `{dest}` does not exist"));
                continue;
            }
            // Fragments are only checked in markdown. Elsewhere they are viewer features, such
            // as `src/lib.rs#L10`.
            let Some(fragment) = fragment.filter(|f| !f.is_empty()) else {
                continue;
            };
            if target.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let defined = anchor_cache
                .entry(target.clone())
                .or_insert_with(|| anchors(&fs::read_to_string(&target).unwrap()));
            if !defined.contains(&fragment) {
                problems.push(format!("{here}: `{dest}` has no heading `#{fragment}`"));
            }
        }
    }
    problems
}

/// `README.md`, `CONTRIBUTING.md` and every markdown file under `docs/`, relative to `root`.
fn documentation(root: &Path) -> Vec<PathBuf> {
    fn walk(root: &Path, dir: &Path, into: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = fs::read_dir(root.join(dir))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if path.is_dir() {
                walk(root, &relative, into);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                into.push(relative);
            }
        }
    }
    let mut files = vec![PathBuf::from("README.md"), PathBuf::from("CONTRIBUTING.md")];
    walk(root, Path::new("docs"), &mut files);
    files
}

#[test]
fn relative_links_and_anchors_in_the_documentation_resolve() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let files = documentation(&root);
    assert!(files.len() > 2, "found no documentation under docs/");
    let problems = check(&root, &files);
    assert!(
        problems.is_empty(),
        "dead links in the documentation:\n{}",
        problems.join("\n")
    );
}

#[test]
fn slugs_follow_githubs_rules() {
    assert_eq!(slug("Exit codes"), "exit-codes");
    assert_eq!(slug("`patchcov.config.yaml`"), "patchcovconfigyaml");
    assert_eq!(
        slug("`--diff-allow-path-mismatch`"),
        "--diff-allow-path-mismatch"
    );
    assert_eq!(
        slug("Why the total differs from llvm-cov's summary"),
        "why-the-total-differs-from-llvm-covs-summary"
    );
    assert_eq!(slug("project_delta"), "project_delta");
    assert_eq!(slug("Tests & fixtures"), "tests--fixtures");
    assert_eq!(slug("A [link](x.md) in a heading"), "a-link-in-a-heading");
    assert_eq!(slug("Closed ##"), "closed");
    assert_eq!(slug("Über Café"), "über-café");
}

#[test]
fn duplicate_headings_get_numeric_suffixes() {
    let found = anchors("# Notes\n\n## Notes\n\n### Notes\n");
    assert!(["notes", "notes-1", "notes-2"]
        .iter()
        .all(|a| found.contains(*a)));
    assert_eq!(found.len(), 3);
}

#[test]
fn code_blocks_define_no_anchors_and_contain_no_links() {
    let text = "# Real\n\n```\n# Not a heading\n[x](missing.md)\n```\n\nUse `[y](missing.md)`.\n";
    assert_eq!(anchors(text), HashSet::from(["real".to_string()]));
    assert!(destinations(text).is_empty());
}

#[test]
fn every_kind_of_destination_is_found() {
    let text = "[a](one.md) ![b](two.svg \"title\") [c](<three four.md>)\n\
                [ref]: four.md\n\
                <img src=\"five.png\"> <a href=\"six.md#x\">six</a>\n";
    let found: Vec<_> = destinations(text).into_iter().map(|(_, d)| d).collect();
    assert_eq!(
        found,
        [
            "one.md",
            "two.svg",
            "three four.md",
            "four.md",
            "five.png",
            "six.md#x"
        ]
    );
}

#[test]
fn dead_links_and_anchors_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("docs")).unwrap();
    fs::write(
        root.join("README.md"),
        "# Title\n\n\
         [good](docs/a.md#section-one) [dir](docs/) [self](#title) [root](/docs/a.md)\n\
         [web](https://example.com/x#y) [mail](mailto:a@b.c) [line](docs/code.rs#L3)\n\
         [missing file](docs/nope.md)\n\
         [missing anchor](docs/a.md#section-two)\n\
         [missing self anchor](#nowhere)\n\
         [encoded](docs/a.md#section%2Done)\n\
         <a id=\"custom\"></a> [custom](#custom)\n",
    )
    .unwrap();
    fs::write(root.join("docs/code.rs"), "fn main() {}\n").unwrap();
    fs::write(
        root.join("docs/a.md"),
        "## Section one\n\n[up](../README.md#title) [bad](../README.md#Title)\n",
    )
    .unwrap();

    let files = [PathBuf::from("README.md"), PathBuf::from("docs/a.md")];
    let problems = check(root, &files);
    assert_eq!(problems.len(), 4, "{problems:#?}");
    assert!(problems[0].contains("README.md:5") && problems[0].contains("does not exist"));
    assert!(
        problems[1].contains("README.md:6") && problems[1].contains("no heading `#section-two`")
    );
    assert!(problems[2].contains("README.md:7") && problems[2].contains("no heading `#nowhere`"));
    assert!(problems[3].contains("a.md:3") && problems[3].contains("no heading `#Title`"));
}
