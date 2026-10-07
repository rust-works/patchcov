//! The `[package.metadata.binstall]` table in `Cargo.toml` must keep describing the archives
//! that `.github/workflows/release-plz.yml` uploads. Nothing else fails when the two drift:
//! cargo-binstall quietly falls back to guessing, or to a source build, and the metadata can
//! only be corrected by the next release.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

fn read(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// The lines of one TOML table, up to the next table header.
fn table<'a>(manifest: &'a str, header: &str) -> Vec<&'a str> {
    manifest
        .lines()
        .skip_while(|line| line.trim() != header)
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .collect()
}

fn value<'a>(lines: &[&'a str], key: &str) -> Option<&'a str> {
    lines.iter().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"'))
    })
}

#[test]
fn the_binstall_layout_matches_the_release_archives() {
    let manifest = read("Cargo.toml");
    let workflow = read(".github/workflows/release-plz.yml");

    let binstall = table(&manifest, "[package.metadata.binstall]");
    assert_eq!(
        value(&binstall, "pkg-url"),
        Some("{ repo }/releases/download/v{ version }/{ name }-v{ version }-{ target }{ archive-suffix }"),
    );
    assert_eq!(
        value(&binstall, "bin-dir"),
        Some("{ name }-v{ version }-{ target }/{ bin }{ binary-ext }"),
    );
    assert_eq!(value(&binstall, "pkg-fmt"), Some("tgz"));

    // The workflow names the archive and its directory `patchcov-<tag>-<target>`, where the tag
    // is `v<version>`, and uploads it to that tag's release.
    assert!(workflow.contains("name=patchcov-${TAG}-${{ matrix.target }}"));
    assert!(workflow.contains(r#"$name = "patchcov-$env:TAG-${{ matrix.target }}""#));
    assert!(workflow.contains(r#"archive="patchcov-${TAG}-${{ matrix.target }}.${ARCHIVE_EXT}""#));
    assert!(workflow.contains("tar -czf \"$name.tar.gz\" \"$name\""));
}

#[test]
fn the_windows_override_matches_the_workflows_zip_target() {
    let manifest = read("Cargo.toml");
    let workflow = read(".github/workflows/release-plz.yml");

    let overrides = "[package.metadata.binstall.overrides.";
    let windows: Vec<&str> = manifest
        .lines()
        .filter_map(|line| line.trim().strip_prefix(overrides)?.strip_suffix(']'))
        .collect();
    assert_eq!(
        windows,
        ["x86_64-pc-windows-msvc"],
        "exactly the zip target is overridden"
    );

    let table = table(
        &manifest,
        "[package.metadata.binstall.overrides.x86_64-pc-windows-msvc]",
    );
    assert_eq!(value(&table, "pkg-fmt"), Some("zip"));
    assert!(workflow.contains("matrix.target == 'x86_64-pc-windows-msvc' && 'zip' || 'tar.gz'"));
}
