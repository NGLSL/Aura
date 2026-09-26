; Aura / EnvBox — Kite-style per-user Windows installer (Inno Setup 6)
; Build:  .\scripts\make-installer.ps1
; Or:     iscc installer\Aura.iss

#define MyAppName "Aura"
#define MyAppVersion "0.3.0"
#define MyAppPublisher "EnvBox"
#define MyAppExeName "envbox-app.exe"
#define StageDir "..\dist\stage"

[Setup]
AppId={{8F3C2A91-5B6E-4D2A-9C7E-AURA03ENVB0}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL=https://github.com/example/envbox
DefaultDirName={localappdata}\Programs\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
LicenseFile=
OutputDir=..\dist
OutputBaseFilename=Aura-{#MyAppVersion}-x64-setup
SetupIconFile=..\icons\icon.ico
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#MyAppExeName}
UninstallDisplayName={#MyAppName}
CloseApplications=yes
RestartApplications=no
; Silent:  Aura-0.3.0-x64-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesesimplified"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "associatefiles"; Description: "Associate *.envbox.toml with import (optional)"; GroupDescription: "Other:"; Flags: unchecked

[Files]
; Runtime + tools (staged by scripts/make-installer.ps1)
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; IconFilename: "{app}\icon.ico"
Name: "{group}\EnvBox CLI (envbox)"; Filename: "{app}\envbox.exe"; WorkingDir: "{app}"
Name: "{group}\Environment Probe"; Filename: "{app}\envbox-probe.exe"; WorkingDir: "{app}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; IconFilename: "{app}\icon.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Config / audit live in %LocalAppData%\EnvBox — only remove app payload here.
Type: filesandordirs; Name: "{app}\logs"
