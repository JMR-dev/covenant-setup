# Future State

## WinUI Helper Extraction Hardening

The WinUI 3 UI helper is embedded as a folder bundle and extracted to a temp directory at runtime. The current implementation already extracts to a per-run temp path, rejects absolute or traversal paths inside the bundle, launches the extracted `Covenant.Setup.Ui.exe`, and cleans the temp folder after the helper exits.

Remaining production hardening:

- Create the temp extraction directory with cryptographic randomness and exclusive creation. The current path includes the process id and timestamp-like suffix, but it is not a strong random name and `create_dir_all` does not prove the directory did not already exist.
- Avoid trusting any pre-existing temp contents. This should fall out of exclusive directory creation, but the extraction code should treat an already-existing target directory or file as an error instead of overwriting.
- Sign the shipping installer and helper artifacts. The project currently builds and embeds the helper, but there is no signing step for the Rust installer, the WinUI helper, or bundled support binaries.
- Add integrity verification for the embedded UI bundle. A future build should include a hash or manifest for the bundled helper files and validate it before extraction and launch.

## Authoring UI Integration

The manifest authoring UI should remain a separate developer-facing WinUI 3 app from the runtime installer progress helper. The runtime helper is embedded into generated installers and launched over named-pipe IPC, while the authoring UI is a local tool for creating `install.toml` files.

Future integration should make the separate app feel first-class from the CLI without merging those responsibilities:

- Add a `covenant-setup author` command that locates and launches the authoring app when it is available.
- Keep the runtime progress helper embedded with installers, but do not embed the authoring UI in generated installer artifacts.
- Keep installer generation in the authoring UI gated on detection of a real `covenant-setup.exe`, and call the Rust CLI for packaging rather than duplicating packager behavior in C#.
- Add a Rust CLI validation command and have the authoring UI call it before packaging so the Rust manifest schema remains the source of truth.
