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
ChangesAssociations=yes

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "zyplus-gpui.exe"; Flags: ignoreversion
Source: "..\..\assets\app-icons\icon.ico"; DestDir: "{app}"; DestName: "zyplus.ico"; Flags: ignoreversion

[Registry]
; File types. "Open with" lists Zyplus for each extension; the user (not the
; installer) picks the default in Settings > Default apps, which Windows 10/11
; protects. HKA is HKCU for a per-user install and HKLM for an all-users one.
Root: HKA; Subkey: "Software\Classes\Zyplus.File"; ValueType: string; ValueName: ""; ValueData: "Zyplus document"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Zyplus.File\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\zyplus.ico"
Root: HKA; Subkey: "Software\Classes\Zyplus.File\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\zyplus-gpui.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueType: string; ValueName: "Zyplus.File"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.markdown\OpenWithProgids"; ValueType: string; ValueName: "Zyplus.File"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.mkd\OpenWithProgids"; ValueType: string; ValueName: "Zyplus.File"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.mdown\OpenWithProgids"; ValueType: string; ValueName: "Zyplus.File"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.txt\OpenWithProgids"; ValueType: string; ValueName: "Zyplus.File"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.csv\OpenWithProgids"; ValueType: string; ValueName: "Zyplus.File"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.tsv\OpenWithProgids"; ValueType: string; ValueName: "Zyplus.File"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.json\OpenWithProgids"; ValueType: string; ValueName: "Zyplus.File"; ValueData: ""; Flags: uninsdeletevalue
; Makes Zyplus appear by name in Settings > Default apps.
Root: HKA; Subkey: "Software\Zyplus\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "Zyplus"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Zyplus\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "A markdown editor that gets out of the way"
Root: HKA; Subkey: "Software\Zyplus\Capabilities\FileAssociations"; ValueType: string; ValueName: ".md"; ValueData: "Zyplus.File"
Root: HKA; Subkey: "Software\Zyplus\Capabilities\FileAssociations"; ValueType: string; ValueName: ".markdown"; ValueData: "Zyplus.File"
Root: HKA; Subkey: "Software\Zyplus\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mkd"; ValueData: "Zyplus.File"
Root: HKA; Subkey: "Software\Zyplus\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mdown"; ValueData: "Zyplus.File"
Root: HKA; Subkey: "Software\Zyplus\Capabilities\FileAssociations"; ValueType: string; ValueName: ".txt"; ValueData: "Zyplus.File"
Root: HKA; Subkey: "Software\Zyplus\Capabilities\FileAssociations"; ValueType: string; ValueName: ".csv"; ValueData: "Zyplus.File"
Root: HKA; Subkey: "Software\Zyplus\Capabilities\FileAssociations"; ValueType: string; ValueName: ".tsv"; ValueData: "Zyplus.File"
Root: HKA; Subkey: "Software\Zyplus\Capabilities\FileAssociations"; ValueType: string; ValueName: ".json"; ValueData: "Zyplus.File"
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "Zyplus"; ValueData: "Software\Zyplus\Capabilities"; Flags: uninsdeletevalue

[Icons]
Name: "{autoprograms}\Zyplus"; Filename: "{app}\zyplus-gpui.exe"; IconFilename: "{app}\zyplus.ico"
Name: "{autodesktop}\Zyplus"; Filename: "{app}\zyplus-gpui.exe"; IconFilename: "{app}\zyplus.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\zyplus-gpui.exe"; Description: "Launch Zyplus"; Flags: nowait postinstall skipifsilent
