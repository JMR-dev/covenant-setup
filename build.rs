use embed_manifest::embed_manifest;
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    publish_csharp_ui();
    embed_manifest(embed_manifest::new_manifest("Comctl32"))
        .expect("unable to embed application manifest");
}

fn publish_csharp_ui() {
    println!("cargo:rerun-if-changed=ui/Covenant.Setup.Ui/Covenant.Setup.Ui.csproj");
    println!("cargo:rerun-if-changed=ui/Covenant.Setup.Ui/Program.cs");
    println!("cargo:rerun-if-changed=ui/Covenant.Setup.Ui/app.manifest");

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let project = manifest_dir.join("ui/Covenant.Setup.Ui/Covenant.Setup.Ui.csproj");
    let publish_dir = out_dir.join("csharp-ui");
    let dotnet_home = out_dir.join("dotnet-home");
    let status = Command::new("dotnet")
        .env("DOTNET_CLI_HOME", &dotnet_home)
        .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .arg("publish")
        .arg(&project)
        .arg("--nologo")
        .arg("--configuration")
        .arg("Release")
        .arg("--runtime")
        .arg("win-x64")
        .arg("--self-contained")
        .arg("true")
        .arg("-p:PublishSingleFile=true")
        .arg("-p:IncludeNativeLibrariesForSelfExtract=true")
        .arg("-p:PublishTrimmed=false")
        .arg("-p:DebugType=none")
        .arg("-p:DebugSymbols=false")
        .arg("-o")
        .arg(&publish_dir)
        .status()
        .expect("failed to launch dotnet publish for C# UI");
    if !status.success() {
        panic!("dotnet publish failed for C# UI with {status}");
    }

    let ui_exe = publish_dir.join("Covenant.Setup.Ui.exe");
    if !ui_exe.exists() {
        panic!("C# UI publish did not produce {}", ui_exe.display());
    }
    println!("cargo:rustc-env=COVENANT_SETUP_UI_EXE={}", ui_exe.display());
}
