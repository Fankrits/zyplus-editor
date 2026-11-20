; Inno Setup script for the Zyplus Windows installer.
;
;   iscc /DAppVersion=0.5.0 /DSourceExe=C:\path\zyplus-gpui.exe /DOutDir=C:\out ^
;        /DOutName=Zyplus_0.5.0_windows-x64-setup packaging\windows\zyplus.iss
;
; Per-user install by default (no admin prompt); the dialog lets the user choose
; all-users instead.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\..\target\release\zyplus-gpui.exe"
#endif
#ifndef OutDir
  #define OutDir "..\..\target\installer"
#endif
#ifndef OutName
  #define OutName "Zyplus_setup"
#endif

[Setup]
AppId={{F4C857C7-833E-425A-A0AE-3F6D340CD8DE}
AppName=Zyplus
AppVersion={#AppVersion}
AppPublisher=Fankrits
AppPublisherURL=https://github.com/Fankrits/zyplus-editor
DefaultDirName={autopf}\Zyplus
DefaultGroupName=Zyplus
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=..\..\assets\app-icons\icon.ico
UninstallDisplayIcon={app}\zyplus.ico
OutputDir={#OutDir}
OutputBaseFilename={#OutName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "zyplus-gpui.exe"; Flags: ignoreversion
Source: "..\..\assets\app-icons\icon.ico"; DestDir: "{app}"; DestName: "zyplus.ico"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Zyplus"; Filename: "{app}\zyplus-gpui.exe"; IconFilename: "{app}\zyplus.ico"
Name: "{autodesktop}\Zyplus"; Filename: "{app}\zyplus-gpui.exe"; IconFilename: "{app}\zyplus.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\zyplus-gpui.exe"; Description: "Launch Zyplus"; Flags: nowait postinstall skipifsilent
