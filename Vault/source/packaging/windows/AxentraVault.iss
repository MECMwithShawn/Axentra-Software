; Axentra Vault installer (Inno Setup 6). Installs under Program Files\Axentra; administrator rights required.
; Built by scripts\package.ps1, which passes /DAppVersion and /DSourceExe.
#define AppName "Axentra Vault"
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\..\target\release\axentra-vault.exe"
#endif
#define AppExe "axentra-vault.exe"

[Setup]
AppId={{C36D03EC-E3A1-41C6-B147-019EC2B9B245}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Axentra
AppPublisherURL=https://github.com/MECMwithShawn/Axentra-Software
AppSupportURL=https://github.com/MECMwithShawn/Axentra-Software/issues
UsePreviousAppDir=no
DefaultDirName={autopf}\Axentra\Vault
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=admin
OutputDir=..\..\dist
OutputBaseFilename=AxentraVault-{#AppVersion}-Setup
SetupIconFile=..\..\app\assets\app\vault.ico
UninstallDisplayIcon={app}\icons\vault-circuit-v2.ico
UninstallDisplayName={#AppName}
LicenseFile=..\..\LICENSE
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
CloseApplications=yes
RestartApplications=no
VersionInfoVersion={#AppVersion}
VersionInfoCompany=Axentra
VersionInfoDescription={#AppName} installer

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion
Source: "..\..\app\assets\app\vault.ico"; DestDir: "{app}\icons"; DestName: "vault-circuit-v2.ico"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; IconFilename: "{app}\icons\vault-circuit-v2.ico"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; IconFilename: "{app}\icons\vault-circuit-v2.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Open {#AppName} now"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: dirifempty; Name: "{app}"

[Messages]
FinishedLabel=Setup has installed [name] on your computer.%n%nYour vault files, preferences (%APPDATA%\Axentra Vault) and backups (%LOCALAPPDATA%\Axentra Vault\Backups) are never touched by installing, upgrading or uninstalling.
UninstalledAll=%1 was removed from your computer.%n%nYour vault files, preferences and backups (%LOCALAPPDATA%\Axentra Vault\Backups) were kept. Delete that folder yourself if you no longer want the backups.
