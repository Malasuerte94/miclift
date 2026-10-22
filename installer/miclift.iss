#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef AppRoot
  #define AppRoot ".."
#endif
[Setup]
AppId={{6D5B96D2-A5C7-4E9F-A30B-1BDB8AE772CB}
AppName=MicLift
AppVersion={#AppVersion}
AppPublisher=MicLift contributors
DefaultDirName={localappdata}\Programs\MicLift
DefaultGroupName=MicLift
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19045
OutputDir={#AppRoot}\dist
OutputBaseFilename=MicLift-{#AppVersion}-windows-x64-setup
SetupIconFile={#AppRoot}\assets\miclift.ico
UninstallDisplayIcon={app}\miclift.exe
LicenseFile={#AppRoot}\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
AppMutex=Local\MicLift.Instance.v1
UninstallDisplayName=MicLift

[Files]
Source: "{#AppRoot}\target\release\miclift.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#AppRoot}\target\release\miclift-diagnostics.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#AppRoot}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#AppRoot}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#AppRoot}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#AppRoot}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Icons]
Name: "{group}\MicLift"; Filename: "{app}\miclift.exe"
Name: "{autodesktop}\MicLift"; Filename: "{app}\miclift.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "MicLift"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\miclift.exe"; Description: "Launch MicLift"; Flags: nowait postinstall skipifsilent runasoriginaluser

; User profiles and third-party drivers are deliberately retained on uninstall.
