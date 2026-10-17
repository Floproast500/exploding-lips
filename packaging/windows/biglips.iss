; Big Lips installer for Windows (Inno Setup 6). Like OldUnreal's installers, it installs the
; source port and then fetches the game itself: the last step runs `biglips.exe --install`,
; which downloads the full version of Exploding Lips from the Internet Archive
; (https://archive.org/details/exploding-lips-full, public domain mark), checks it and unpacks it
; into %APPDATA%\exploding-lips\game, where Big Lips finds it. No game files ship in here.
;
; Build (after `cargo build --release -p lips-desktop -p packbuilder`):
;   iscc /DVersion=1.0.0 packaging\windows\biglips.iss
; The installer lands in target\installer\.

#ifndef Version
  #define Version "1.0.0"
#endif

[Setup]
AppId={{6B2C0E1A-5D3E-4C1F-9A57-B16E0C1A5D01}
AppName=Big Lips
AppVersion={#Version}
AppPublisher=Big Lips contributors
AppSupportURL=https://github.com/Floproast500/exploding-lips
DefaultDirName={autopf}\Big Lips
DefaultGroupName=Big Lips
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\..\target\installer
OutputBaseFilename=BigLips-{#Version}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible

[Files]
Source: "..\..\target\release\biglips.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\packbuilder.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme
; The route guide's routes, one per level (beside biglips.exe, where it looks for them).
Source: "..\..\routes\*.route"; DestDir: "{app}\routes"; Flags: ignoreversion

[Icons]
Name: "{group}\Big Lips"; Filename: "{app}\biglips.exe"; WorkingDir: "{app}"
Name: "{group}\Download Exploding Lips again"; Filename: "{app}\biglips.exe"; Parameters: "--install"; WorkingDir: "{app}"
Name: "{group}\Uninstall Big Lips"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Big Lips"; Filename: "{app}\biglips.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Run]
; Fetch the game (about 43 MB) in a console window with a progress line; skipped when it is
; already there.
Filename: "{app}\biglips.exe"; Parameters: "--install"; WorkingDir: "{app}"; StatusMsg: "Downloading Exploding Lips from the Internet Archive..."; Flags: waituntilterminated
Filename: "{app}\biglips.exe"; WorkingDir: "{app}"; Description: "Play Big Lips now"; Flags: postinstall nowait skipifsilent
