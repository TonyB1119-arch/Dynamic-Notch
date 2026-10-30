; Dynamic Notch: the smallest installer that does the job.
;
; Build it with Inno Setup 6 (https://jrsoftware.org/isinfo.php):
;   ISCC.exe installer\DynamicNotch.iss
; or pass the version and the exe, which is what the release workflow does:
;   ISCC.exe /DAppVersion=3.0.0 /DExePath=..\target\release\dynamic-notch.exe installer\DynamicNotch.iss
;
; What it does: copies the exe into your user folder (no administrator prompt), adds a Start
; menu shortcut and an uninstaller. Nothing else: no service, no scheduled task, no updater.
; "Start with Windows" stays a setting inside the app. The same installer is what
; Settings -> General -> Updates runs when you press Install.

#ifndef AppVersion
  #define AppVersion "3.0.0"
#endif
#ifndef ExePath
  #define ExePath "..\target\release\dynamic-notch.exe"
#endif

[Setup]
AppId={{6E0B5B1E-3C57-4F5A-9A44-0D1E2C7A9B11}
AppName=Dynamic Notch
AppVersion={#AppVersion}
AppPublisher=Dynamic Notch
DefaultDirName={localappdata}\Programs\DynamicNotch
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=DynamicNotch-Setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\dynamic-notch.exe
; closes a running copy before replacing the exe, and starts it again afterwards (silent updates)
CloseApplications=yes
RestartApplications=yes

[Files]
Source: "{#ExePath}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Dynamic Notch"; Filename: "{app}\dynamic-notch.exe"

[Run]
; normal install: offer to start it
Filename: "{app}\dynamic-notch.exe"; Description: "Start Dynamic Notch"; Flags: nowait postinstall skipifsilent
; silent install (an update from Settings): start it again without asking
Filename: "{app}\dynamic-notch.exe"; Flags: nowait; Check: WizardSilent

[UninstallRun]
; ask a running copy to quit so its files can be removed
Filename: "{app}\dynamic-notch.exe"; Parameters: "quit"; Flags: runhidden skipifdoesntexist; RunOnceId: "QuitNotch"

[Registry]
; remove the "Start with Windows" entry if the app added one
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "DynamicNotch"; Flags: dontcreatekey uninsdeletevalue
