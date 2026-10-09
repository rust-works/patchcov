//! Relative links and `#anchors` in `README.md`, `CONTRIBUTING.md` and `docs/` must resolve.
//! A renamed heading or a moved file otherwise breaks them silently. The check is offline: it
//! reads the markdown and the repository tree, and ignores external URLs, so it cannot flake
//! on the network. It runs wherever `cargo test --all-targets` does, which includes CI.
//!
//! Anchors follow GitHub's slug rules: the heading text is lowercased, punctuation other than
//! `-` and `_` is dropped, each space becomes `-`, and a repeated slug gets `-1`, `-2`, ...
//! The parsing is deliberately small, not a CommonMark parser: it covers the constructs these
//! files use (ATX headings, inline and reference-style links, `src`/`href` attributes, `<a id>`)
//! and skips fenced code blocks and inline code. Not supported: setext headings (`Title` over
//! `=====`), `[text][label]` usage (the definitions are checked), multi-backtick code spans,
//! parentheses inside a destination, and links in HTML comments.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

macro_rules! regex {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pattern).unwrap());
    };
}

regex!(LINK_TEXT, r"\[([^\]]*)\]\([^)]*\)");
regex!(HTML_TAG, r"<[^>]*>");
regex!(CLOSING_HASHES, r"\s+#+\s*$");
regex!(HEADING, r"^ {0,3}#{1,6}\s+(.*)$");
regex!(EXPLICIT_ANCHOR, r#"<a\s[^>]*?(?:id|name)\s*=\s*"([^"]*)""#);
regex!(INLINE_CODE, r"`[^`]*`");
regex!(INLINE_LINK, r"\]\(\s*(<[^>]*>|[^)\s]*)");
// A footnote definition (`[^1]: text`) is not a link definition.
regex!(LINK_DEFINITION, r"^ {0,3}\[[^\]^][^\]]*\]:\s*(<[^>]*>|\S+)");
regex!(HTML_ATTRIBUTE, r#"\b(?:src|href)\s*=\s*"([^"]*)""#);
regex!(URL_SCHEME, r"^[A-Za-z][A-Za-z0-9+.-]*:");

/// The lines of a markdown file that are prose: fenced code blocks are dropped. Each line is
/// paired with its 1-based number.
fn prose_lines(text: &str) -> Vec<(usize, &str)> {
    // The open fence: its character and length. A closing fence repeats the character at least
    // as many times and carries no info string.
    let mut fence: Option<(char, usize)> = None;
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let marker = trimmed
            .chars()
            .next()
            .filter(|c| matches!(c, '`' | '~'))
            .map(|c| (c, trimmed.chars().take_while(|&x| x == c).count()))
            .filter(|&(_, length)| length >= 3);
        match (fence, marker) {
            (None, Some(open)) => fence = Some(open),
            (None, None) => lines.push((index + 1, line)),
            (Some((c, length)), Some((found, count)))
                if found == c && count >= length && trimmed.trim_end().chars().all(|x| x == c) =>
            {
                fence = None;
            }
            (Some(_), _) => {}
        }
    }
    lines
}

/// GitHub's anchor for a heading's text, before any `-1` suffix for a duplicate.
fn slug(heading: &str) -> String {
    let text = LINK_TEXT.replace_all(heading, "$1");
    let text = HTML_TAG.replace_all(&text, "");
    let text = CLOSING_HASHES.replace(text.trim(), "");
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
    // As GitHub numbers duplicates: while the slug is taken, count up from the heading's own
    // slug. A generated `notes-1` therefore also blocks a heading that slugs to `notes-1`.
    let mut taken: HashMap<String, usize> = HashMap::new();
    let mut found = HashSet::new();
    for (_, line) in prose_lines(text) {
        if let Some(caps) = HEADING.captures(line) {
            let base = slug(&caps[1]);
            let mut result = base.clone();
            while taken.contains_key(&result) {
                let count = taken.entry(base.clone()).or_insert(0);
                *count += 1;
                result = format!("{base}-{count}");
            }
            taken.insert(result.clone(), 0);
            found.insert(result);
        }
        for caps in EXPLICIT_ANCHOR.captures_iter(line) {
            found.insert(caps[1].to_string());
        }
    }
    found
}

/// Every link destination in a markdown file, with its line number. Covers `[text](dest)`,
/// `![alt](dest)`, `[label]: dest` and `src="dest"` / `href="dest"`.
fn destinations(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (number, line) in prose_lines(text) {
        let line = INLINE_CODE.replace_all(line, "");
        let mut push = |dest: &str| {
            let dest = dest.trim_start_matches('<').trim_end_matches('>');
            found.push((number, dest.to_string()));
        };
        for caps in INLINE_LINK.captures_iter(&line) {
            push(&caps[1]);
        }
        if let Some(caps) = LINK_DEFINITION.captures(&line) {
            push(&caps[1]);
        }
        for caps in HTML_ATTRIBUTE.captures_iter(&line) {
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
    dest.starts_with("//") || URL_SCHEME.is_match(dest)
}

/// Whether `target` exists with exactly this spelling. macOS and Windows look names up
/// case-insensitively, so `Path::exists` alone would accept `docs/Usage.md` there while GitHub
/// does not.
fn exists_exactly(root: &Path, target: &Path) -> bool {
    if !target.exists() {
        return false;
    }
    let mut normal = PathBuf::new();
    for component in target.components() {
        match component {
            Component::ParentDir => {
                normal.pop();
            }
            Component::CurDir => {}
            other => normal.push(other),
        }
    }
    let Ok(below_root) = normal.strip_prefix(root) else {
        return true;
    };
    let mut dir = root.to_path_buf();
    for name in below_root.components() {
        let Ok(entries) = fs::read_dir(&dir) else {
            return false;
        };
        if !entries
            .filter_map(Result::ok)
            .any(|entry| entry.file_name() == name.as_os_str())
        {
            return false;
        }
        dir.push(name);
    }
    true
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
            // A query string (`?plain=1`) is a viewer option, not part of the file name.
            let path = percent_decode(path.split_once('?').map_or(path, |(path, _)| path));
            let target = if path.is_empty() {
                root.join(file)
            } else if let Some(from_root) = path.strip_prefix('/') {
                root.join(from_root)
            } else {
                root.join(file).parent().unwrap().join(&path)
            };
            let here = format!("{}:{line}", file.display());
            if !exists_exactly(root, &target) {
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

    // A generated suffix can collide with a literal heading: GitHub gives the third `notes-1-1`.
    let found = anchors("# Notes\n\n## Notes\n\n## Notes 1\n");
    assert_eq!(
        found,
        HashSet::from(["notes".into(), "notes-1".into(), "notes-1-1".into()])
    );
}

#[test]
fn code_blocks_define_no_anchors_and_contain_no_links() {
    let text = "# Real\n\n```\n# Not a heading\n[x](missing.md)\n```\n\nUse `[y](missing.md)`.\n";
    assert_eq!(anchors(text), HashSet::from(["real".to_string()]));
    assert!(destinations(text).is_empty());

    // A longer fence is not closed by a shorter one or by one with an info string.
    let nested = "````md\n```\n```rust\n[x](missing.md)\n```\n````\n[y](after.md)\n";
    let found: Vec<_> = destinations(nested).into_iter().map(|(_, d)| d).collect();
    assert_eq!(found, ["after.md"]);
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
         <a id=\"custom\"></a> [custom](#custom)\n\
         [query](docs/a.md?plain=1#section-one) [footnote]\n\n\
         [^1]: Not a link.\n\
         [wrong case](docs/A.md)\n",
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
    assert_eq!(problems.len(), 5, "{problems:#?}");
    assert!(problems[0].contains("README.md:5") && problems[0].contains("does not exist"));
    assert!(
        problems[1].contains("README.md:6") && problems[1].contains("no heading `#section-two`")
    );
    assert!(problems[2].contains("README.md:7") && problems[2].contains("no heading `#nowhere`"));
    assert!(
        problems[3].contains("README.md:13") && problems[3].contains("`docs/A.md` does not exist")
    );
    assert!(problems[4].contains("a.md:3") && problems[4].contains("no heading `#Title`"));
}
