use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/strata-hub.ico");
    let rc = env::split_paths(&env::var_os("PATH").expect("PATH is required"))
        .map(|path| path.join("rc.exe"))
        .find(|path| path.is_file())
        .or_else(|| {
            let sdk = PathBuf::from(env::var_os("ProgramFiles(x86)")?).join("Windows Kits/10/bin");
            let mut candidates: Vec<_> = fs::read_dir(sdk)
                .ok()?
                .filter_map(|row| row.ok())
                .map(|row| row.path().join("x64/rc.exe"))
                .filter(|path| path.is_file())
                .collect();
            candidates.sort();
            candidates.pop()
        })
        .expect("Windows SDK resource compiler rc.exe is required");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is required"));
    let icon =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("assets/strata-hub.ico");
    let source = out.join("app.rc");
    let resource = out.join("app.res");
    fs::write(
        &source,
        format!("1 ICON \"{}\"\n", icon.to_string_lossy().replace('\\', "/")),
    )
    .unwrap();
    let status = Command::new(rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource)
        .arg(source)
        .status()
        .expect("Cannot run the Windows resource compiler");
    assert!(status.success(), "Cannot compile the Strata Hub icon");
    println!(
        "cargo:rustc-link-arg-bin=strata-dashboard={}",
        resource.display()
    );
}
