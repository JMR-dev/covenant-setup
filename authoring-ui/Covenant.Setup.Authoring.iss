; Inno Setup script for the Covenant Setup Authoring UI.
;
; Build prerequisites from the repository root:
;   dotnet publish .\authoring-ui\Covenant.Setup.Authoring.csproj -c Release -r win-x64 --self-contained true -o .\dist\authoring-ui
;
; Compile this installer from the repository root:
;   iscc .\authoring-ui\Covenant.Setup.Authoring.iss

#define MyAppName "Covenant Setup Authoring"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "covenant-setup"
#define MyAppExeName "Covenant.Setup.Authoring.exe"
#define MyPublishDir "..\dist\authoring-ui"

[Setup]
AppId={{0E0A77B1-2045-49D9-A722-2C9B328BE900}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={localappdata}\Programs\Covenant Setup Authoring
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
MinVersion=10.0.19041
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist\inno
OutputBaseFilename=covenant-setup-authoring-installer
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupLogging=yes
UninstallDisplayIcon={app}\{#MyAppExeName}
VersionInfoVersion={#MyAppVersion}.0
VersionInfoDescription=Covenant Setup Authoring installer

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "{#MyPublishDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "README.md"; DestDir: "{app}\docs"; DestName: "authoring-ui-readme.md"; Flags: ignoreversion

[Icons]
Name: "{group}\Covenant Setup Authoring"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"
Name: "{group}\Authoring UI README"; Filename: "{app}\docs\authoring-ui-readme.md"
Name: "{group}\Uninstall Covenant Setup Authoring"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Covenant Setup Authoring"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch Covenant Setup Authoring"; Flags: nowait postinstall skipifsilent unchecked
