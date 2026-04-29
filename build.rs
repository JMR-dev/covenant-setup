use embed_manifest::embed_manifest;
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(covenant_setup_embedded_ui)");
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
        .status();
    let status = match status {
        Ok(status) => status,
        Err(err) => {
            println!(
                "cargo:warning=C# UI helper was not bundled because dotnet publish could not start: {err}"
            );
            return;
        }
    };
    if !status.success() {
        println!(
            "cargo:warning=C# UI helper was not bundled because dotnet publish failed with {status}"
        );
        return;
    }

    let ui_exe = publish_dir.join("Covenant.Setup.Ui.exe");
    if !ui_exe.exists() {
        println!(
            "cargo:warning=C# UI helper was not bundled because dotnet publish did not produce {}",
            ui_exe.display()
        );
        return;
    }
    println!("cargo:rustc-cfg=covenant_setup_embedded_ui");
    println!("cargo:rustc-env=COVENANT_SETUP_UI_EXE={}", ui_exe.display());
}
