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

[Files]
Source: "{#BundleDir}\nvim-gpui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BundleDir}\gpvim.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BundleDir}\rime\*"; DestDir: "{app}\rime"; Flags: ignoreversion recursesubdirs createallsubdirs

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional icons:"

[Icons]
Name: "{autoprograms}\nvim-gpui"; Filename: "{app}\nvim-gpui.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\nvim-gpui"; Filename: "{app}\nvim-gpui.exe"; WorkingDir: "{app}"; Tasks: desktopicon
