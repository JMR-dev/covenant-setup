# HOW TO TEST INSTALLER UI (Mocked Mode)

This document describes how to use the WinUI 3 Installer UI Mocking Framework to test the installer UI both **visually** (by running a simulated scenario end-to-end) and **automatically** (via xUnit tests).

---

## 1. Visual/Interactive Testing (Manual)

The WinUI 3 installer UI (`ui/Covenant.Setup.Ui`) can be launched in a mocked mode directly from the command-line by passing the `--mock <scenario>` flag.

In this mode, a background mock engine client spins up inside the application, connects to its own named pipe loop, and feeds it scripted installation events. You can watch the full installation/uninstallation flow, click dialog options, and inspect visual styling without running the real Rust backend.

### How to Run

1. **Build the UI project as self-contained win-x64**:
   ```powershell
   dotnet build ui/Covenant.Setup.Ui/Covenant.Setup.Ui.csproj -r win-x64
   ```

2. **Execute with a mock scenario**:
   Navigate to the output directory and run:
   ```powershell
   .\ui\Covenant.Setup.Ui\bin\x64\Debug\net10.0-windows10.0.19041.0\win-x64\Covenant.Setup.Ui.exe --mock <scenario-name-or-file-path>
   ```

   *Examples:*
   ```powershell
   # Run the happy path installation scenario
   .\ui\Covenant.Setup.Ui\bin\x64\Debug\net10.0-windows10.0.19041.0\win-x64\Covenant.Setup.Ui.exe --mock install-happy

   # Run a slow-progress visual test with verbose log output
   .\ui\Covenant.Setup.Ui\bin\x64\Debug\net10.0-windows10.0.19041.0\win-x64\Covenant.Setup.Ui.exe --mock install-slow
   ```

---

## 2. Available Canned Scenarios

The framework includes 6 pre-configured scenarios located in [ui/Covenant.Setup.Ui/Scenarios/](file:///C:/Users/jasonross/workspace/covenant-setup/ui/Covenant.Setup.Ui/Scenarios/):

| Scenario Name | Description |
| --- | --- |
| `install-happy` | Standard 8-step happy path (directories, copying files, registry, shortcuts, scripts, uninstaller, ARP) ending in success. |
| `install-prompt` | Prompts with an OK/Cancel dialog first, then triggers the happy path installation upon acceptance. |
| `install-fail-errata` | Walks through steps 1-3, fails, and displays the "Save error data" button containing a complete `covenant_setup_errata_v1` payload. |
| `uninstall-happy` | Triggers a reverse LIFO-flavored uninstallation sequence and completes successfully. |
| `uninstall-reboot-prompt` | Triggers uninstallation and prompts the user with a Yes/No restart dialog at the end. |
| `install-slow` | Runs 12 progress steps, outputs verbose log lines, and introduces 500-1500ms delays to test layout transitions. |

---

## 3. Automated Testing (xUnit)

Automated integration tests run the real named pipe IPC loop and feed simulated scenarios to the message handler, asserting that the view receives correct callbacks in the expected order.

To execute all tests (including the new mock engine and scenario parser tests):
```powershell
dotnet test ui/Covenant.Setup.Ui.Tests/Covenant.Setup.Ui.Tests.csproj
```

---

## 4. Scenario File Format (`.jsonl`)

Scenarios are stored as newline-delimited JSON (`.jsonl`) files. Empty lines and comment lines starting with `#` are automatically skipped.

Each active line must follow one of these three structures:

1. **Send verbatim to UI pipe**:
   If the JSON contains a `"type"` property, the line is written directly to the pipe.
   ```json
   {"type":"init","title":"Installing Application","message":"Preparing...","total_steps":10}
   ```

2. **Introduce delay**:
   Use `"wait_ms"` to delay execution of subsequent steps (delays are skipped in automated tests by setting `SkipDelays = true`).
   ```json
   {"wait_ms": 500}
   ```

3. **Await prompt response**:
   Use `"await_response"` to block playback until the UI sends back a response to a prompt.
   ```json
   {"await_response": {"id": "prompt-id", "expect": "ok"}}
   ```

---

## 5. Scripting Scenarios Programmatically

You can programmatically construct a `Scenario` object using the C# fluent API in tests:

```csharp
using Covenant.Setup.Ui.Mocking;

var scenario = new ScenarioBuilder("custom-scenario")
    .Init("Installing App", "Starting...")
    .Progress(1, 5, "Creating folders")
    .Delay(200)
    .Prompt("p1", "Confirm", "Proceed?", "ok_cancel", "information")
    .AwaitPromptResponse("p1", "ok")
    .Finish("Success!")
    .Close()
    .Build();
```
