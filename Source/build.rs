fn main() {
    println!("cargo:rerun-if-changed=video.cpp");
    println!("cargo:rerun-if-changed=flow.manifest");
    println!("cargo:rerun-if-changed=flow.rc");
    println!("cargo:rerun-if-changed=flow.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    cc::Build::new()
        .cpp(true)
        .cpp_link_stdlib(None)
        .file("video.cpp")
        .flag_if_supported("-fno-exceptions")
        .flag_if_supported("-fno-rtti")
        .compile("flow_video");
    for library in ["ole32", "mfuuid", "uuid", "dwmapi"] {
        println!("cargo:rustc-link-lib={library}");
    }
    let dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let target = std::env::var("TARGET").unwrap();
    if target.contains("msvc") {
        let output = dir.join("flow.res");
        let rc = cc::windows_registry::find_tool(&target, "rc.exe")
            .map(|t| t.to_command())
            .unwrap_or_else(|| std::process::Command::new("rc.exe"));
        let mut rc = rc;
        assert!(rc
            .arg("/nologo")
            .arg(format!("/fo{}", output.display()))
            .arg("flow.rc")
            .status()
            .expect("Windows resource compiler")
            .success());
        println!("cargo:rustc-link-arg-bins={}", output.display());
    } else {
        let output = dir.join("flow-resource.o");
        let rc = std::env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".into());
        assert!(std::process::Command::new(rc)
            .args(["-i", "flow.rc", "-O", "coff", "-o"])
            .arg(&output)
            .status()
            .expect("windres")
            .success());
        println!("cargo:rustc-link-arg-bins={}", output.display());
    }
}
