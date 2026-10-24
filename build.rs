// Generate Rust bindings for the frozen android.hardware.gnss AIDL API,
// version 2 (Android 13). Every interface is stamped with version 2 and the
// frozen hash, as AOSP's build does for a frozen aidl_interface version.
use std::path::{Path, PathBuf};

fn aidl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            aidl_files(&p, out);
        } else if p.extension().map_or(false, |e| e == "aidl") {
            out.push(p);
        }
    }
}

fn main() {
    let root = PathBuf::from("aidl");
    let hash = std::fs::read_to_string(root.join(".hash"))
        .unwrap()
        .trim()
        .to_string();
    let mut files = Vec::new();
    aidl_files(&root.join("android/hardware/gnss"), &mut files);
    let mut b = rsbinder_aidl::Builder::new().include_dir(&root);
    for f in &files {
        b = b.source(f).version(2).hash(hash.clone());
    }
    b.output(PathBuf::from("gnss_aidl.rs")).generate().unwrap();
    println!("cargo:rerun-if-changed=aidl");
}
