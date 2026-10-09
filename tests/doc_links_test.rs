//! Relative links and `#anchors` in `README.md`, `CONTRIBUTING.md` and `docs/` must resolve.
//! A renamed heading or a moved file otherwise breaks them silently. The check is offline: it
//! reads the markdown and the repository tree, and ignores external URLs, so it cannot flake
//! on the network. It runs wherever `cargo test --all-targets` does, which includes CI.
//!
//! Anchors follow GitHub's slug rules: the heading text is lowercased, punctuation other than
//! `-` and `_` is dropped, each space becomes `-`, and a repeated slug gets `-1`, `-2`, ...
//! CommonMark parsing handles headings, links and images while skipping code and HTML comments.
//! Raw HTML support covers double-quoted `src`/`href` attributes and `<a id>` / `<a name>`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
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

regex!(SLUG_PUNCTUATION, r"[^\p{L}\p{N}\p{M}_ -]");
regex!(EXPLICIT_ANCHOR, r#"<a\s[^>]*?(?:id|name)\s*=\s*"([^"]*)""#);
regex!(HTML_ATTRIBUTE, r#"\b(?:src|href)\s*=\s*"([^"]*)""#);
regex!(URL_SCHEME, r"^[A-Za-z][A-Za-z0-9+.-]*:");

/// GitHub's anchor for rendered heading text, before any duplicate suffix.
fn slug(heading: &str) -> String {
    SLUG_PUNCTUATION
        .replace_all(&heading.to_lowercase(), "")
        .replace(' ', "-")
}

/// Non-comment slices of a raw HTML event, with offsets within the event. Block HTML can be
/// split into several events, so carry comment state across them. Offsets preserve diagnostics.
fn html_prose<'a>(html: &'a str, in_comment: &mut bool) -> Vec<(usize, &'a str)> {
    let mut found = Vec::new();
    let mut offset = 0;
    while offset < html.len() {
        let remaining = &html[offset..];
        if *in_comment {
            let Some(end) = remaining.find("-->") else {
                break;
            };
            offset += end + 3;
            *in_comment = false;
        } else if let Some(start) = remaining.find("<!--") {
            found.push((offset, &remaining[..start]));
            offset += start + 4;
            *in_comment = true;
        } else {
            found.push((offset, remaining));
            break;
        }
    }
    found
}

/// Every anchor a markdown file defines: its headings, and explicit `<a id>` / `<a name>`.
fn anchors(text: &str) -> HashSet<String> {
    // A generated `notes-1` also blocks a heading that slugs to `notes-1`.
    let mut taken: HashMap<String, usize> = HashMap::new();
    let mut found = HashSet::new();
    let mut heading = None::<String>;
    let mut in_comment = false;
    for event in Parser::new(text) {
        match event {
            Event::Start(Tag::Heading { .. }) => heading = Some(String::new()),
            Event::Text(value) | Event::Code(value) => {
                if let Some(heading) = &mut heading {
                    heading.push_str(&value);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(heading) = &mut heading {
                    heading.push(' ');
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                let base = slug(&heading.take().unwrap());
                let mut result = base.clone();
                while taken.contains_key(&result) {
                    let count = taken.entry(base.clone()).or_insert(0);
                    *count += 1;
                    result = format!("{base}-{count}");
                }
                taken.insert(result.clone(), 0);
                found.insert(result);
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                for (_, prose) in html_prose(&html, &mut in_comment) {
                    for caps in EXPLICIT_ANCHOR.captures_iter(prose) {
                        found.insert(caps[1].to_string());
                    }
                }
            }
            _ => {}
        }
    }
    found
}

/// Every rendered link/image destination, with the 1-based source line of its usage.
/// Unused reference definitions are not links. Raw HTML src/href attributes are also checked.
fn destinations(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut in_comment = false;
    let line_at = |offset| text[..offset].bytes().filter(|&b| b == b'\n').count() + 1;
    for (event, range) in Parser::new(text).into_offset_iter() {
        match event {
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                found.push((line_at(range.start), dest_url.into_string()));
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                for (offset, prose) in html_prose(&html, &mut in_comment) {
                    for caps in HTML_ATTRIBUTE.captures_iter(prose) {
                        let start = range.start + offset + caps.get(0).unwrap().start();
                        found.push((line_at(start), caps[1].to_string()));
                    }
                }
            }
            _ => {}
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
            // Do not follow directory cycles or include symlinked files.
            if path.is_symlink() {
                continue;
            }
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
    assert_eq!(slug("patchcov.config.yaml"), "patchcovconfigyaml");
    assert_eq!(
        slug("--diff-allow-path-mismatch"),
        "--diff-allow-path-mismatch"
    );
    assert_eq!(
        slug("Why the total differs from llvm-cov's summary"),
        "why-the-total-differs-from-llvm-covs-summary"
    );
    assert_eq!(slug("project_delta"), "project_delta");
    assert_eq!(slug("Tests & fixtures"), "tests--fixtures");
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
                [used][ref]\n\n\
                [ref]: four.md\n\n\
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

#[test]
fn commonmark_headings_use_rendered_text_and_preserve_combining_marks() {
    let text = "Setext *title*\n==============\n\nSecond\n------\n\n\
                # A [link](x.md) and `code` with <em>HTML</em> &amp; an ![image](x.png)\n\
                # Closed ##\n\
                # Cafe\u{301} İ\n";
    assert_eq!(
        anchors(text),
        HashSet::from([
            "setext-title".into(),
            "second".into(),
            "a-link-and-code-with-html--an-image".into(),
            "closed".into(),
            "cafe\u{301}-i\u{307}".into(),
        ])
    );
}

#[test]
fn reference_links_and_parenthesized_destinations_have_usage_lines() {
    let text = "[full][label] ![image][label]\n\
                [label][] [label]\n\
                [nested](file(a(b)).md) [escaped](file\\(c\\).md)\n\n\
                [label]: <target (one).md> \"a title\"\n\
                [unused]: missing.md\n";
    assert_eq!(
        destinations(text),
        vec![
            (1, "target (one).md".into()),
            (1, "target (one).md".into()),
            (2, "target (one).md".into()),
            (2, "target (one).md".into()),
            (3, "file(a(b)).md".into()),
            (3, "file(c).md".into()),
        ]
    );
}

#[test]
fn multi_backtick_and_multiline_code_spans_and_indented_code_are_skipped() {
    let text = concat!(
        "Use `` `[example](missing.md)` `` and ``across\n",
        "[example](missing.md) ` lines``.\n\n",
        "    # Not a heading\n",
        "    [example](missing.md)\n\n",
        "[live](real.md)\n",
    );
    assert!(anchors(text).is_empty());
    assert_eq!(destinations(text), vec![(7, "real.md".into())]);
}

#[test]
fn html_comments_define_no_anchors_and_contain_no_links() {
    let text = "<!--\n\
                # Hidden\n\
                [missing](missing.md) <a id=\"hidden\" href=\"missing.md\">\n\
                -->\n\n\
                # Visible <!-- <a id=\"also-hidden\" href=\"missing.md\"> --> title\n\n\
                <!-- hidden --> <a id=\"live\" href=\"real.md\">live</a>\n\
                <img\n src=\"real.png\">\n";
    assert_eq!(
        anchors(text),
        HashSet::from(["visible--title".into(), "live".into()])
    );
    assert_eq!(
        destinations(text),
        vec![(8, "real.md".into()), (10, "real.png".into())]
    );
}

#[test]
fn commonmark_links_resolve_and_report_dead_reference_usages() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("target(one).md"), "Cafe\u{301}\n=====\n").unwrap();
    fs::write(
        root.join("README.md"),
        "[good][target]\n[bad][dead]\n\n[target]: target(one).md#cafe%CC%81\n[dead]: missing.md\n[unused]: also-missing.md\n",
    ).unwrap();
    assert_eq!(
        check(root, &[PathBuf::from("README.md")]),
        vec!["README.md:2: `missing.md` does not exist"]
    );
}

#[cfg(unix)]
#[test]
fn documentation_walk_skips_symlinks_including_cycles() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("docs/sub")).unwrap();
    fs::write(root.join("docs/sub/real.md"), "# Real\n").unwrap();
    symlink(root.join("docs"), root.join("docs/sub/cycle")).unwrap();
    symlink(root.join("docs/sub/real.md"), root.join("docs/link.md")).unwrap();
    assert_eq!(
        documentation(root),
        vec![
            PathBuf::from("README.md"),
            PathBuf::from("CONTRIBUTING.md"),
            PathBuf::from("docs/sub/real.md")
        ]
    );
}
