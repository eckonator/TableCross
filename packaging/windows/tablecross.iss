; Inno Setup script for the TableCross installer (64-bit Windows 10 and 11).
; Built by the release workflow: iscc /DVersion=x.y.z /DSource=<folder with the programs> tablecross.iss
; Installs the app, the command line tool and the service (sign-in screen, UAC, elevated programs),
; and opens UDP port 24800 in the Windows firewall for private networks. Starting with Windows is
; a setting in the app.

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef Source
  #define Source "..\..\target\release"
#endif

[Setup]
AppId={{6E0C5B3A-4D1F-4C8E-9B57-7A2D6C1E9F30}
AppName=TableCross
AppVersion={#Version}
AppPublisher=Markus Eckert
AppPublisherURL=https://github.com/eckonator/tablecross
DefaultDirName={autopf}\TableCross
DefaultGroupName=TableCross
DisableProgramGroupPage=yes
LicenseFile=..\..\LICENSE
OutputBaseFilename=TableCross-{#Version}-Windows-Setup
SetupIconFile=..\..\assets\tablecross.ico
UninstallDisplayIcon={app}\TableCross.exe
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"

[CustomMessages]
en.Service=Install the TableCross service (operate the sign-in screen, UAC prompts and programs started as administrator)
de.Service=TableCross-Dienst installieren (Anmeldebildschirm, UAC-Abfragen und als Administrator gestartete Programme bedienen)

[Tasks]
Name: "service"; Description: "{cm:Service}"

[Files]
Source: "{#Source}\tablecross.exe"; DestDir: "{app}"; DestName: "TableCross.exe"; Flags: ignoreversion
Source: "{#Source}\tablecross-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\tablecross-service.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\fonts\*-LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\THIRD-PARTY-LICENSES.yml"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{group}\TableCross"; Filename: "{app}\TableCross.exe"

[Run]
; The old service was removed before the files were copied (see [Code])
Filename: "{app}\tablecross-service.exe"; Parameters: "install"; Flags: runhidden; Tasks: service
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""TableCross"""; Flags: runhidden
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""TableCross"" dir=in action=allow protocol=UDP localport=24800 profile=private,domain program=""{app}\TableCross.exe"""; Flags: runhidden
Filename: "{app}\TableCross.exe"; Description: "{cm:LaunchProgram,TableCross}"; Flags: nowait postinstall skipifsilent
; Update from inside the app (silent): start the new version again, as the signed-in user
Filename: "{app}\TableCross.exe"; Flags: nowait runasoriginaluser; Check: WizardSilent

[UninstallRun]
Filename: "{app}\tablecross-service.exe"; Parameters: "uninstall"; Flags: runhidden; RunOnceId: "RemoveService"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""TableCross"""; Flags: runhidden; RunOnceId: "RemoveFirewallRule"
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM TableCross.exe"; Flags: runhidden; RunOnceId: "StopApp"

[Code]
// An older service (and its agent) still runs from the program folder: stop and remove it before
// the new files are copied, else the service program is in use and cannot be replaced.
procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
begin
  if (CurStep = ssInstall) and FileExists(ExpandConstant('{app}\tablecross-service.exe')) then
    Exec(ExpandConstant('{app}\tablecross-service.exe'), 'uninstall', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;
