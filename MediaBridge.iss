#define AppName "MediaBridge"
#define AppPublisher "MediaBridge"
#define AppExeName "mediabridge.exe"

[Setup]
AppId={{D3290F3F-60CD-4D91-9473-B23D40E9F2A1}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\MediaBridge
DefaultGroupName=MediaBridge
UninstallDisplayIcon={app}\mediabridge.exe
OutputDir=dist
OutputBaseFilename=MediaBridge-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=admin
ArchitecturesInstallIn64BitMode=x64
DisableProgramGroupPage=yes
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked
Name: "startwithwindows"; Description: "Start MediaBridge automatically when I sign in to Windows"; GroupDescription: "Startup options:"

[Code]
var
  PortPage: TInputQueryWizardPage;
  RemoveUserData: Boolean;

procedure InitializeWizard;
begin
  PortPage := CreateInputQueryPage(wpSelectTasks,
    'Server port', 'Choose the TCP port for MediaBridge',
    'Enter a port available on this computer. The firewall rule will use this port too.');
  PortPage.Add('TCP port:', False);
  PortPage.Values[0] := '8765';
end;

function NextButtonClick(CurPageID: Integer): Boolean;
var
  PortNumber: Integer;
  PortText: String;
  I: Integer;
  Digit: Integer;
begin
  Result := True;
  if CurPageID = PortPage.ID then
  begin
    PortNumber := 0;
    PortText := Trim(PortPage.Values[0]);
    Result := Length(PortText) > 0;
    for I := 1 to Length(PortText) do
    begin
      Digit := Ord(PortText[I]);
      if (Digit < Ord('0')) or (Digit > Ord('9')) then
      begin
        Result := False;
        Break;
      end;
      PortNumber := PortNumber * 10 + Digit - Ord('0');
      if PortNumber > 65535 then
      begin
        Result := False;
        Break;
      end;
    end;
    Result := Result and (PortNumber >= 1);
    if not Result then
      MsgBox('Enter a TCP port number from 1 through 65535.', mbError, MB_OK);
  end;
end;

function GetPort(Param: String): String;
begin
  Result := Trim(PortPage.Values[0]);
end;

function InitializeUninstall: Boolean;
begin
  RemoveUserData := False;
  if not UninstallSilent then
    RemoveUserData := MsgBox('Also remove MediaBridge settings and logs stored for this Windows account?',
      mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
  Result := True;
end;

function ShouldRemoveUserData: Boolean;
begin
  Result := RemoveUserData;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    { This file is generated after installation and therefore is not owned by [Files]. }
    DeleteFile(ExpandConstant('{app}\mediabridge.json'));
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ConfigPath: String;
  ConfigText: String;
begin
  if CurStep = ssPostInstall then
  begin
    ConfigPath := ExpandConstant('{app}\mediabridge.json');
    if not FileExists(ConfigPath) then
    begin
      ConfigText := '{' + #13#10 +
        '  "bind": "0.0.0.0:' + Trim(PortPage.Values[0]) + '",' + #13#10 +
        '  "remote_origin_servers": [],' + #13#10 +
        '  "default_screen": null,' + #13#10 +
        '  "default_quality": null,' + #13#10 +
        '  "subtitles": false,' + #13#10 +
        '  "preferred_language": null' + #13#10 + '}' + #13#10;
      if not SaveStringToFile(ConfigPath, ConfigText, False) then
        RaiseException('Could not create the MediaBridge configuration file.');
    end;
  end;
end;

[Files]
Source: "dist\package\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Dirs]
Name: "{app}\librewolf-profile"; Permissions: users-modify

[InstallDelete]
Type: files; Name: "{app}\moonlight\portable.dat"
Type: files; Name: "{userstartup}\MediaBridge.lnk"
Type: files; Name: "{autoprograms}\MediaBridge\MediaBridge.lnk"
Type: files; Name: "{autoprograms}\MediaBridge\Uninstall MediaBridge.lnk"
Type: dirifempty; Name: "{autoprograms}\MediaBridge"

[Icons]
Name: "{group}\MediaBridge"; Filename: "{app}\mediabridge.exe"
Name: "{group}\Uninstall MediaBridge"; Filename: "{uninstallexe}"
Name: "{userstartup}\MediaBridge"; Filename: "{app}\mediabridge.exe"; Tasks: startwithwindows
Name: "{autodesktop}\MediaBridge"; Filename: "{app}\mediabridge.exe"; Tasks: desktopicon

[UninstallDelete]
Type: filesandordirs; Name: "{localappdata}\MediaBridge"; Check: ShouldRemoveUserData
Type: filesandordirs; Name: "{app}\librewolf-profile"
Type: filesandordirs; Name: "{app}\mediabridge.json"

[Run]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""MediaBridge Server"""; Flags: runhidden waituntilterminated
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""MediaBridge TCP 8765"""; Flags: runhidden waituntilterminated
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""MediaBridge Server"" dir=in action=allow protocol=TCP localport={code:GetPort} profile=private program=""{app}\mediabridge.exe"""; StatusMsg: "Adding the private-network firewall rule..."; Flags: runhidden waituntilterminated
Filename: "{sys}\icacls.exe"; Parameters: """{app}\mediabridge.json"" /grant ""{username}"":M"; StatusMsg: "Allowing the installing Windows account to save preferences..."; Flags: runhidden waituntilterminated
Filename: "{app}\mediabridge-dashboard.bat"; Parameters: "{code:GetPort}"; Description: "Launch MediaBridge and sign in to YouTube and Twitch"; Flags: postinstall nowait skipifsilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM mediabridge.exe"; Flags: runhidden waituntilterminated
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""MediaBridge Server"""; Flags: runhidden waituntilterminated
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""MediaBridge TCP 8765"""; Flags: runhidden waituntilterminated
