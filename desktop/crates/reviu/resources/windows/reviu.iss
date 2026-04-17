[Setup]
AppId={{7B6F2C53-7AC7-4F7D-BC8D-4C1E33ED6CE3}
AppName={#AppName}
AppVerName={#AppName} {#Version}
AppPublisher=Joris Gallot
AppPublisherURL=https://reviu.dev/
AppSupportURL=https://reviu.dev/
AppUpdatesURL=https://reviu.dev/
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableReadyPage=yes
AllowNoIcons=yes
OutputDir={#OutputDir}
OutputBaseFilename={#AppSetupName}
Compression=lzma
SolidCompression=yes
SetupIconFile={#IconPath}
UninstallDisplayIcon={app}\{#AppExeName}.exe
ChangesAssociations=true
MinVersion=10.0.16299
SourceDir={#SourceDir}
AppVersion={#Version}
VersionInfoVersion={#VersionInfoVersion}
WizardStyle=modern
CloseApplications=force
DefaultDirName={autopf}\{#AppName}
PrivilegesRequired=lowest
ArchitecturesAllowed={#ArchitecturesAllowed}
ArchitecturesInstallIn64BitMode={#ArchitecturesInstallIn64BitMode}

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#BinaryPath}"; DestDir: "{app}"; DestName: "{#AppExeName}.exe"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}.exe"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}.exe"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall; Check: WizardNotSilent

[Registry]
Root: HKCU; Subkey: "Software\Classes\reviu"; ValueType: "string"; ValueData: "URL:Reviu Protocol"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\reviu"; ValueType: "string"; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\reviu\DefaultIcon"; ValueType: "string"; ValueData: "{app}\{#AppExeName}.exe,1"
Root: HKCU; Subkey: "Software\Classes\reviu\shell\open\command"; ValueType: "string"; ValueData: """{app}\{#AppExeName}.exe"" ""%1"""

[Code]
function WizardNotSilent(): Boolean;
begin
  Result := not WizardSilent();
end;
