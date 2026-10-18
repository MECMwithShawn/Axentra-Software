; build.ps1 supplies the version and compiled binary path.
#define AppName "Axentra USB"
#ifndef AppVersion
  #define AppVersion "0.1.1"
#endif
#ifndef AppBinary
  #define AppBinary "..\target\release\axentra-usb.exe"
#endif
[Setup]
AppId={{379E227C-1024-4D76-9A85-ED057406DF52}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Axentra
AppPublisherURL=https://github.com/MECMwithShawn/Axentra-Software
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} Setup
DefaultDirName={autopf}\Axentra\USB
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir=..\dist
OutputBaseFilename=AxentraUSB-{#AppVersion}-Setup
SetupIconFile=..\assets\axentra-usb.ico
UninstallDisplayIcon={app}\icons\usb-outline-v1.ico
UninstallDisplayName={#AppName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"

[Files]
Source: "{#AppBinary}"; DestDir: "{app}"; DestName: "AxentraUSB.exe"; Flags: ignoreversion
Source: "..\assets\axentra-usb.ico"; DestDir: "{app}\icons"; DestName: "usb-outline-v1.ico"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\VALIDATION.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\AxentraUSB.exe"; WorkingDir: "{app}"; IconFilename: "{app}\icons\usb-outline-v1.ico"; AppUserModelID: "Axentra.USB"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\AxentraUSB.exe"; WorkingDir: "{app}"; IconFilename: "{app}\icons\usb-outline-v1.ico"; AppUserModelID: "Axentra.USB"; Tasks: desktopicon

[Run]
Filename: "{app}\AxentraUSB.exe"; Description: "Open {#AppName}"; Flags: nowait postinstall skipifsilent
