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

[Code]
var
  PortPage: TInputQueryWizardPage;

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

[Icons]
Name: "{group}\MediaBridge"; Filename: "{app}\mediabridge.exe"
Name: "{group}\Uninstall MediaBridge"; Filename: "{uninstallexe}"
Name: "{autodesktop}\MediaBridge"; Filename: "{app}\mediabridge.exe"; Tasks: desktopicon

[Run]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""MediaBridge Server"""; Flags: runhidden waituntilterminated
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""MediaBridge TCP 8765"""; Flags: runhidden waituntilterminated
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""MediaBridge Server"" dir=in action=allow protocol=TCP localport={code:GetPort} profile=private program=""{app}\mediabridge.exe"""; StatusMsg: "Adding the private-network firewall rule..."; Flags: runhidden waituntilterminated
Filename: "{sys}\icacls.exe"; Parameters: """{app}\mediabridge.json"" /grant ""{username}"":M"; StatusMsg: "Allowing the installing Windows account to save preferences..."; Flags: runhidden waituntilterminated
Filename: "{app}\mediabridge.exe"; Description: "Launch MediaBridge"; Flags: postinstall nowait skipifsilent

[UninstallRun]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""MediaBridge Server"""; Flags: runhidden waituntilterminated
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""MediaBridge TCP 8765"""; Flags: runhidden waituntilterminated
