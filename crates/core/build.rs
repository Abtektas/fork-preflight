// SPDX-License-Identifier: BSD-3-Clause
//! Embeds every data/releases/*.json file, so the binary needs no data on disk.

use std::{env, fs, path::PathBuf};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../..")
        .canonicalize()
        .unwrap();
    let data = root.join("data/releases");
    println!("cargo:rerun-if-changed={}", data.display());
    let mut files: Vec<PathBuf> = fs::read_dir(&data)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    let mut out = String::from("pub(crate) const RELEASE_FILES: &[(&str, &str)] = &[\n");
    for path in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        let name = path.file_stem().unwrap().to_str().unwrap();
        out.push_str(&format!(
            "    ({name:?}, include_str!({:?})),\n",
            path.display().to_string()
        ));
    }
    out.push_str("];\n");
    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("releases.rs");
    fs::write(dest, out).unwrap();
}
