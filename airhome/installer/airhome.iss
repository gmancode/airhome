; Inno Setup script for airhome. Build:  iscc /DAppVersion=0.9.7 installer\airhome.iss
; Per-user install (no administrator rights): %LOCALAPPDATA%\Programs\airhome

#ifndef ExeDir
  #define ExeDir "..\target\release"
#endif

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{B3A6D1B2-6F0E-4C55-9C1B-2D3B7C0A4E11}
AppName=airhome
AppVersion={#AppVersion}
AppPublisher=airhome
DefaultDirName={localappdata}\Programs\airhome
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=airhome-setup-{#AppVersion}
SetupIconFile=..\logos\airhome.ico
UninstallDisplayIcon={app}\airhome.exe
LicenseFile=..\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; refuse to update/remove while the program runs (it keeps a named mutex)
AppMutex=airhome-single-instance
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"

[CustomMessages]
english.Autostart=Start airhome with Windows
russian.Autostart=Запускать airhome вместе с Windows
spanish.Autostart=Iniciar airhome con Windows
german.Autostart=airhome mit Windows starten
french.Autostart=Lancer airhome avec Windows
brazilianportuguese.Autostart=Iniciar o airhome com o Windows
japanese.Autostart=Windows の起動時に airhome を起動する

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "autostart"; Description: "{cm:Autostart}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#ExeDir}\airhome.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\NOTICE.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; The Start-menu shortcut carries the AppUserModelID that gives notifications the name and icon "airhome".
Name: "{userprograms}\airhome"; Filename: "{app}\airhome.exe"; AppUserModelID: "airhome.app"
Name: "{userdesktop}\airhome"; Filename: "{app}\airhome.exe"; AppUserModelID: "airhome.app"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "airhome"; \
  ValueData: """{app}\airhome.exe"" --hidden"; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{app}\airhome.exe"; Description: "{cm:LaunchProgram,airhome}"; Flags: nowait postinstall skipifsilent
