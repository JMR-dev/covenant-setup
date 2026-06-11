use embed_manifest::embed_manifest;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

// Must match the bundle format read by extract_ui_bundle in src/ui.rs.
const UI_BUNDLE_MAGIC: &[u8] = b"COVENANT_SETUP_UI_BUNDLE_V1\n";

fn main() {
    println!("cargo:rustc-check-cfg=cfg(covenant_setup_embedded_ui)");
    publish_csharp_ui();
    embed_manifest(embed_manifest::new_manifest("Comctl32"))
        .expect("unable to embed application manifest");
}

fn publish_csharp_ui() {
    println!("cargo:rerun-if-changed=ui/Covenant.Setup.Ui/Covenant.Setup.Ui.csproj");
    println!("cargo:rerun-if-changed=ui/Covenant.Setup.Ui/Program.cs");
    println!("cargo:rerun-if-changed=ui/Covenant.Setup.Ui/InstallerUiWindow.cs");
    println!("cargo:rerun-if-changed=ui/Covenant.Setup.Ui/UiProtocol.cs");
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
        .arg("-p:WindowsPackageType=None")
        .arg("-p:WindowsAppSDKSelfContained=true")
        .arg("-p:PublishSingleFile=false")
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
    let ui_bundle = out_dir.join("csharp-ui.bundle");
    if let Err(err) = write_ui_bundle(&publish_dir, &ui_bundle) {
        println!(
            "cargo:warning=C# UI helper was not bundled because bundle creation failed: {err}"
        );
        return;
    }
    println!("cargo:rustc-cfg=covenant_setup_embedded_ui");
    println!(
        "cargo:rustc-env=COVENANT_SETUP_UI_BUNDLE={}",
        ui_bundle.display()
    );
}

fn write_ui_bundle(publish_dir: &Path, bundle_path: &Path) -> io::Result<()> {
    let mut files = Vec::new();
    collect_files(publish_dir, publish_dir, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut bundle = fs::File::create(bundle_path)?;
    bundle.write_all(UI_BUNDLE_MAGIC)?;
    for (relative_path, path) in files {
        let relative_path = relative_path.to_string_lossy().replace('\\', "/");
        let relative_path_bytes = relative_path.as_bytes();
        let data = fs::read(path)?;
        bundle.write_all(&(relative_path_bytes.len() as u32).to_le_bytes())?;
        bundle.write_all(&(data.len() as u64).to_le_bytes())?;
        bundle.write_all(relative_path_bytes)?;
        bundle.write_all(&data)?;
    }
    bundle.write_all(&0u32.to_le_bytes())?;
    bundle.write_all(&0u64.to_le_bytes())?;
    Ok(())
}

fn collect_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
) -> io::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files)?;
        } else if path.is_file() {
            let relative_path = path
                .strip_prefix(root)
                .map_err(io::Error::other)?
                .to_path_buf();
            files.push((relative_path, path));
        }
    }
    Ok(())
}
