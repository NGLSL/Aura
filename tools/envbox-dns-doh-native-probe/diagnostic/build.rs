use std::{env, fs, path::PathBuf};

fn main() {
    let source = PathBuf::from("../../../crates/envbox-dns-doh/src/trust.rs");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-env-changed=DOH_DIAGNOSTIC_INCLUDE_AUTHROOT");
    let contents = fs::read_to_string(&source).expect("production trust source must be readable");
    // `trust.rs` begins with a crate-level `//!` comment. `include!` inside
    // this diagnostic module cannot preserve that inner attribute, so strip
    // exactly that documentation line while keeping every Rust item intact.
    let mut contents = contents
        .strip_prefix(
            "//! Offline local trust snapshot and explicit distrust; no chain/URL APIs.\n",
        )
        .expect("trust source header changed unexpectedly")
        .to_owned();
    const ROOT_LABELS: &str = "for label in [\"ROOT\", \"CA\", \"Disallowed\"]";
    const ROOT_BRANCH: &str = "} else if label == \"ROOT\" {";
    let root_labels = contents.matches(ROOT_LABELS).count();
    let root_branch = contents.matches(ROOT_BRANCH).count();
    assert_eq!(
        root_labels, 1,
        "production trust source changed: expected exactly one ROOT/CA store loop, found {root_labels}"
    );
    assert_eq!(
        root_branch, 1,
        "production trust source changed: expected exactly one ROOT branch, found {root_branch}"
    );

    // Research-only switch: test whether the already-materialized AuthRoot
    // physical store supplies the public anchor under the same eligibility,
    // deny-list and CRL policy. It never changes the product crate.
    let include_authroot = env::var_os("DOH_DIAGNOSTIC_INCLUDE_AUTHROOT")
        .is_some_and(|value| value == "1");
    let variant = if include_authroot { "authroot" } else { "native" };
    if include_authroot {
        assert_eq!(
            contents.matches(ROOT_LABELS).count(),
            1,
            "AuthRoot diagnostic label replacement source count changed"
        );
        let replaced_labels = contents.replace(
            ROOT_LABELS,
            "for label in [\"ROOT\", \"AuthRoot\", \"CA\", \"Disallowed\"]",
        );
        assert_ne!(
            replaced_labels, contents,
            "AuthRoot diagnostic replacement did not change the trust source"
        );
        assert_eq!(
            replaced_labels.matches(ROOT_LABELS).count(),
            0,
            "AuthRoot diagnostic label replacement left the original loop in place"
        );
        assert_eq!(
            replaced_labels
                .matches("for label in [\"ROOT\", \"AuthRoot\", \"CA\", \"Disallowed\"]")
                .count(),
            1,
            "AuthRoot diagnostic label replacement did not produce exactly one variant loop"
        );
        contents = replaced_labels;

        assert_eq!(
            contents.matches(ROOT_BRANCH).count(),
            1,
            "AuthRoot diagnostic branch replacement source count changed"
        );
        let replaced_branch = contents.replace(
            ROOT_BRANCH,
            "} else if label == \"ROOT\" || label == \"AuthRoot\" {",
        );
        assert_ne!(
            replaced_branch, contents,
            "AuthRoot diagnostic branch replacement did not change the trust source"
        );
        assert_eq!(
            replaced_branch.matches(ROOT_BRANCH).count(),
            0,
            "AuthRoot diagnostic branch replacement left the original branch in place"
        );
        assert_eq!(
            replaced_branch
                .matches("} else if label == \"ROOT\" || label == \"AuthRoot\" {")
                .count(),
            1,
            "AuthRoot diagnostic branch replacement did not produce exactly one variant branch"
        );
        contents = replaced_branch;
    }
    let marker = format!(
        "// DOH_DIAGNOSTIC_VARIANT={variant}\n// DOH_DIAGNOSTIC_AUTHROOT_VARIANT={}\n",
        if include_authroot { 1 } else { 0 }
    );
    contents = marker + &contents;
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    fs::write(output.join("trust.rs"), &contents).expect("write generated trust source");
    println!("cargo:rustc-env=DOH_DIAGNOSTIC_VARIANT={variant}");
}
