// Generate Rust bindings for the frozen android.hardware.gnss AIDL API, version 2 (Android 13).
// Every interface is stamped with version 2 and the frozen hash, as AOSP's build does for a frozen
// aidl_interface version.
use std::{
    fs,
    path::{Path, PathBuf},
};

fn aidl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display()));
    let mut paths: Vec<PathBuf> = entries
        .map(|e| {
            e.unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display()))
                .path()
        })
        .collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            aidl_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "aidl") {
            out.push(p);
        }
    }
}

fn main() {
    let root = PathBuf::from("aidl");
    let hash = fs::read_to_string(root.join(".hash"))
        .unwrap_or_else(|e| panic!("cannot read aidl/.hash: {e}"))
        .trim()
        .to_owned();
    let mut files = Vec::new();
    aidl_files(&root.join("android/hardware/gnss"), &mut files);
    let mut b = rsbinder_aidl::Builder::new().include_dir(&root);
    for f in &files {
        b = b.source(f).version(2).hash(hash.clone());
    }
    b.output(PathBuf::from("gnss_aidl.rs"))
        .generate()
        .unwrap_or_else(|e| panic!("AIDL code generation failed: {e}"));
    println!("cargo:rerun-if-changed=aidl");
}
