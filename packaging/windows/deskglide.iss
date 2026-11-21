#ifndef MyAppVersion
  #define MyAppVersion "0.1.2"
#endif

[Setup]
AppId={{8D6D8F14-7D1E-4F26-9C45-B3D6027A4E10}
AppName=DeskGlide
AppVersion={#MyAppVersion}
AppPublisher=DeskGlide Contributors
DefaultDirName={autopf}\DeskGlide
DefaultGroupName=DeskGlide
AllowNoIcons=yes
LicenseFile=..\..\LICENSE-MIT
OutputDir=..\..\dist
OutputBaseFilename=DeskGlide-Windows-Setup-x64
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "autostart"; Description: "Start DeskGlide automatically when signing in to Windows"; GroupDescription: "Startup:"
Name: "firewall"; Description: "Allow DeskGlide through Windows Defender Firewall (UDP port 24910)"; GroupDescription: "Network:"; Flags: checkedonce

[Files]
Source: "..\..\target\release\deskglide-app.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\deskglide.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\DeskGlide"; Filename: "{app}\deskglide-app.exe"
Name: "{group}\Uninstall DeskGlide"; Filename: "{uninstallexe}"
Name: "{autodesktop}\DeskGlide"; Filename: "{app}\deskglide-app.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "DeskGlide"; ValueData: """{app}\deskglide-app.exe"""; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "netsh"; Parameters: "advfirewall firewall delete rule name=""DeskGlide (QUIC/UDP)"""; Flags: runhidden waituntilterminated; Tasks: firewall
Filename: "netsh"; Parameters: "advfirewall firewall add rule name=""DeskGlide (QUIC/UDP)"" dir=in action=allow protocol=UDP localport=24910"; Flags: runhidden waituntilterminated; Tasks: firewall
Filename: "{app}\deskglide-app.exe"; Description: "{cm:LaunchProgram,DeskGlide}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "netsh"; Parameters: "advfirewall firewall delete rule name=""DeskGlide (QUIC/UDP)"""; Flags: runhidden waituntilterminated; RunOnceId: "RemoveFirewallRule"
