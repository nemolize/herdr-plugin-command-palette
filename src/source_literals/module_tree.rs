//! The source files a crate compiles outside tests, found by following `mod`
//! declarations from its root rather than by listing a directory.

use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::Item;

use super::test_code::is_cfg_test;

/// `root` and every file a `mod foo;` reaches from it, except a module declared
/// `#[cfg(test)]` and everything below it.
pub fn shipped_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let dir = root.parent().unwrap().to_path_buf();
    walk_file(root, &dir, &mut found);
    found
}

fn walk_file(file: &Path, dir: &Path, found: &mut Vec<PathBuf>) {
    let src = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let parsed =
        syn::parse_file(&src).unwrap_or_else(|e| panic!("{} does not parse: {e}", file.display()));
    found.push(file.to_path_buf());
    walk_items(&parsed.items, dir, found);
}

fn walk_items(items: &[Item], dir: &Path, found: &mut Vec<PathBuf>) {
    for item in items {
        let Item::Mod(m) = item else { continue };
        if m.attrs
            .iter()
            .any(|a| is_cfg_test(&a.meta.to_token_stream()))
        {
            continue;
        }
        let name = m.ident.to_string();
        let child_dir = dir.join(&name);
        match &m.content {
            Some((_, items)) => walk_items(items, &child_dir, found),
            None => {
                let flat = dir.join(format!("{name}.rs"));
                let nested = child_dir.join("mod.rs");
                let file = if flat.exists() { flat } else { nested };
                walk_file(&file, &child_dir, found);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::shipped_files;

    #[test]
    fn a_module_declared_cfg_test_is_not_followed_but_a_plain_one_is() {
        let dir = std::env::temp_dir().join(format!("palette-module-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = [
            (
                "main.rs",
                "mod bar;\nmod inline { mod deep; }\n#[cfg(test)]\nmod foo;\n",
            ),
            ("bar.rs", "mod child;"),
            ("bar/child.rs", ""),
            ("inline/deep/mod.rs", ""),
            ("foo.rs", "mod child;"),
            ("foo/child.rs", ""),
        ];
        for (path, src) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, src).unwrap();
        }
        let found = shipped_files(&dir.join("main.rs"));
        std::fs::remove_dir_all(&dir).unwrap();
        let expected =
            ["main.rs", "bar.rs", "bar/child.rs", "inline/deep/mod.rs"].map(|p| dir.join(p));
        assert_eq!(found, expected);
    }
}
