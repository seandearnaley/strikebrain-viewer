; The Windows installer, for Inno Setup 6: https://jrsoftware.org/ishelp/
; `just windows-packages` compiles it, and defines Version, Stage (the folder
; of files to install) and, when the build is signed, Sign.

[Setup]
; Names the installation across versions, so a later one upgrades it.
AppId=ai.brainstrike.viewer
AppName=Strikebrain Viewer
AppVersion={#Version}
AppPublisher=Sean Dearnaley
AppPublisherURL=https://github.com/seandearnaley/strikebrain-viewer
; For the person who runs it, in their own Programs folder: no administrator
; prompt.
PrivilegesRequired=lowest
DefaultDirName={autopf}\Strikebrain Viewer
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=icon.ico
UninstallDisplayIcon={app}\strikebrain-viewer.exe
WizardStyle=modern
OutputBaseFilename=Strikebrain-Viewer-{#Version}-windows-x64-setup
#ifdef Sign
; Signs the installer, the uninstaller and the copies of itself Setup runs.
SignTool=sign
#endif

[Files]
; signonce: with a sign tool, the executable is signed where it lies before
; it is packed; without one the flag does nothing.
Source: "{#Stage}\strikebrain-viewer.exe"; DestDir: "{app}"; Flags: ignoreversion signonce
Source: "{#Stage}\LICENSE-*"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Strikebrain Viewer"; Filename: "{app}\strikebrain-viewer.exe"

[Run]
Filename: "{app}\strikebrain-viewer.exe"; Description: "{cm:LaunchProgram,Strikebrain Viewer}"; Flags: nowait postinstall skipifsilent
