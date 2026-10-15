//! The source files a crate compiles outside tests, found by following `mod`
//! declarations from its root rather than by listing a directory.

use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::Item;

use super::test_code::is_cfg_test;

/// `root` and every file a `mod foo;` reaches from it, except a module declared
/// `#[cfg(test)]` or whose file starts `#![cfg(test)]`, and everything below it.
pub fn shipped_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let submodule_dir = root.parent().unwrap().to_path_buf();
    walk_file(root, &submodule_dir, &mut found);
    found
}

fn walk_file(file: &Path, submodule_dir: &Path, found: &mut Vec<PathBuf>) {
    let src = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let parsed =
        syn::parse_file(&src).unwrap_or_else(|e| panic!("{} does not parse: {e}", file.display()));
    if is_test_only(&parsed.attrs) {
        return;
    }
    found.push(file.to_path_buf());
    walk_items(&parsed.items, submodule_dir, found);
}

fn walk_items(items: &[Item], submodule_dir: &Path, found: &mut Vec<PathBuf>) {
    for item in items {
        let Item::Mod(m) = item else { continue };
        if is_test_only(&m.attrs) {
            continue;
        }
        let name = m.ident.to_string();
        let child_dir = submodule_dir.join(&name);
        match &m.content {
            Some((_, items)) => walk_items(items, &child_dir, found),
            None => {
                let flat = submodule_dir.join(format!("{name}.rs"));
                let nested = child_dir.join("mod.rs");
                let file = if flat.exists() { flat } else { nested };
                walk_file(&file, &child_dir, found);
            }
        }
    }
}

fn is_test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| is_cfg_test(&a.meta.to_token_stream()))
}

#[cfg(test)]
mod tests {
    use super::shipped_files;

    fn shipped_in(name: &str, files: &[(&str, &str)]) -> Vec<String> {
        let dir = std::env::temp_dir().join(format!("palette-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, src) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, src).unwrap();
        }
        let found = shipped_files(&dir.join("main.rs"));
        std::fs::remove_dir_all(&dir).unwrap();
        found
            .iter()
            .map(|p| p.strip_prefix(&dir).unwrap().to_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn a_module_declared_cfg_test_is_not_followed_but_a_plain_one_is() {
        let found = shipped_in(
            "module-tree",
            &[
                (
                    "main.rs",
                    "mod bar;\n#[cfg(test)]\nmod foo;\nmod inline { mod deep; }\n",
                ),
                ("bar.rs", "mod child;"),
                ("bar/child.rs", ""),
                ("inline/deep/mod.rs", ""),
                ("foo.rs", "mod child;"),
                ("foo/child.rs", ""),
            ],
        );
        assert_eq!(
            found,
            ["main.rs", "bar.rs", "bar/child.rs", "inline/deep/mod.rs"]
        );
    }

    #[test]
    fn a_file_starting_with_an_inner_cfg_test_is_not_followed_but_other_inner_attributes_are() {
        let found = shipped_in(
            "inner-cfg-test",
            &[
                ("main.rs", "mod only_tests;\nmod lenient;\n"),
                ("only_tests.rs", "#![cfg(test)]\nmod child;"),
                ("only_tests/child.rs", "mod grandchild;"),
                ("only_tests/child/grandchild.rs", ""),
                ("lenient.rs", "#![allow(dead_code)]\nmod child;"),
                ("lenient/child.rs", ""),
            ],
        );
        assert_eq!(found, ["main.rs", "lenient.rs", "lenient/child.rs"]);
    }
}
