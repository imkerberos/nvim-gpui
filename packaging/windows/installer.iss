; The application and bundled Rime runtime are x86_64 Windows binaries.
; x64compatible also matches Windows 11 on Arm, where x64 applications run
; through Microsoft's compatibility layer.
#define RepoRoot AddBackslash(SourcePath) + "..\.."

#ifndef AppVersion
#define AppVersion "0.0.0-dev"
#endif

#ifndef BundleDir
#define BundleDir AddBackslash(RepoRoot) + ".cache\windows\nvim-gpui"
#endif

#ifndef OutputDir
#define OutputDir AddBackslash(RepoRoot) + "dist\windows"
#endif

[Setup]
AppId={{7C8B1D6A-8E2E-4D1A-9E2C-5A7E6D7C0F31}
AppName=nvim-gpui
AppVersion={#AppVersion}
AppVerName=nvim-gpui {#AppVersion}
AppPublisher=imkerberos
AppPublisherURL=https://github.com/imkerberos/nvim-gpui
AppSupportURL=https://github.com/imkerberos/nvim-gpui/issues
DefaultDirName={localappdata}\Programs\nvim-gpui
DefaultGroupName=nvim-gpui
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile={#RepoRoot}\assets\icons\neovim-gpui.ico
UninstallDisplayIcon={app}\nvim-gpui.exe
OutputDir={#OutputDir}
OutputBaseFilename=nvim-gpui-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
Uninstallable=yes
ChangesAssociations=yes
ChangesEnvironment=yes

[Files]
Source: "{#BundleDir}\nvim-gpui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BundleDir}\gpvim.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BundleDir}\rime\*"; DestDir: "{app}\rime"; Flags: ignoreversion recursesubdirs createallsubdirs

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional icons:"
Name: "addtopath"; Description: "Add nvim-gpui and gpvim to the user PATH"; GroupDescription: "Additional options:"

[Icons]
Name: "{autoprograms}\nvim-gpui"; Filename: "{app}\nvim-gpui.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\nvim-gpui"; Filename: "{app}\nvim-gpui.exe"; WorkingDir: "{app}"; Tasks: desktopicon

; Make nvim-gpui available in Explorer's Open With menu for every file
; without replacing the user's existing default file associations.
[Registry]
Root: HKCU; Subkey: "Software\Classes\*\OpenWithProgids"; ValueType: string; ValueName: "nvim-gpui"; ValueData: ""; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\nvim-gpui"; ValueType: string; ValueName: ""; ValueData: "nvim-gpui file"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\nvim-gpui\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\nvim-gpui.exe"" ""%1"""; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\nvim-gpui"; Flags: uninsdeletekeyifempty

[Code]
const
  UserPathStateKey = 'Software\nvim-gpui';
  UserPathStateValue = 'AddedUserPathEntry';

function PathContainsEntry(const PathValue, Entry: string): Boolean;
var
  Remaining, Item: string;
  Separator: Integer;
begin
  Remaining := PathValue;
  while True do begin
    Separator := Pos(';', Remaining);
    if Separator > 0 then begin
      Item := Trim(Copy(Remaining, 1, Separator - 1));
      Delete(Remaining, 1, Separator);
    end else begin
      Item := Trim(Remaining);
      Remaining := '';
    end;

    if CompareText(Item, Entry) = 0 then begin
      Result := True;
      Exit;
    end;
    if Remaining = '' then
      Break;
  end;
  Result := False;
end;

function PathWithoutEntry(const PathValue, Entry: string): string;
var
  Remaining, Item, KeptPath: string;
  Separator: Integer;
begin
  Remaining := PathValue;
  KeptPath := '';
  while True do begin
    Separator := Pos(';', Remaining);
    if Separator > 0 then begin
      Item := Trim(Copy(Remaining, 1, Separator - 1));
      Delete(Remaining, 1, Separator);
    end else begin
      Item := Trim(Remaining);
      Remaining := '';
    end;

    if (Item <> '') and (CompareText(Item, Entry) <> 0) then begin
      if KeptPath <> '' then
        KeptPath := KeptPath + ';';
      KeptPath := KeptPath + Item;
    end;
    if Remaining = '' then
      Break;
  end;
  Result := KeptPath;
end;

procedure AddApplicationToUserPath;
var
  UserPath, AppPath: string;
begin
  AppPath := ExpandConstant('{app}');
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', UserPath) then
    UserPath := '';

  if PathContainsEntry(UserPath, AppPath) then
    Exit;
  if UserPath <> '' then
    UserPath := UserPath + ';';
  UserPath := UserPath + AppPath;

  if RegWriteExpandStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', UserPath) then
    RegWriteStringValue(HKEY_CURRENT_USER, UserPathStateKey, UserPathStateValue, AppPath)
  else
    MsgBox('Could not add nvim-gpui to the user PATH.', mbError, MB_OK);
end;

procedure RemoveApplicationFromUserPath;
var
  UserPath, AppPath, UpdatedPath: string;
begin
  if not RegQueryStringValue(
    HKEY_CURRENT_USER, UserPathStateKey, UserPathStateValue, AppPath
  ) then
    Exit;
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', UserPath) then
    UserPath := '';

  UpdatedPath := PathWithoutEntry(UserPath, AppPath);
  if UpdatedPath <> UserPath then begin
    if UpdatedPath = '' then
      RegDeleteValue(HKEY_CURRENT_USER, 'Environment', 'Path')
    else
      RegWriteExpandStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', UpdatedPath);
  end;
  RegDeleteValue(HKEY_CURRENT_USER, UserPathStateKey, UserPathStateValue);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
    AddApplicationToUserPath;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RemoveApplicationFromUserPath;
end;
