Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "nsDialogs.nsh"
!include "x64.nsh"
!include "WinVer.nsh"
!include "TextFunc.nsh"
!include "FileFunc.nsh"
!ifndef VERSION
!define VERSION "0.2.0"
!endif
!ifndef PAYLOAD
!error "Pass /DPAYLOAD=<staged files> to makensis"
!endif
!ifndef OUTPUT
!define OUTPUT "OmniSync-Setup-${VERSION}-x64.exe"
!endif
Name "OmniSync"
OutFile "${OUTPUT}"
InstallDir "$PROGRAMFILES64\OmniSync"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
ManifestDPIAware true
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "OmniSync"
VIAddVersionKey "FileDescription" "OmniSync Windows Setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "See README for licensing"
!define MUI_ICON "${PAYLOAD}\icons\app.ico"
!define MUI_UNICON "${PAYLOAD}\icons\app.ico"
!define MUI_ABORTWARNING
!define MUI_BGCOLOR "FFFFFF"
!define MUI_TEXTCOLOR "17233B"
!define MUI_WELCOMEPAGE_TITLE "Welcome to OmniSync"
!define MUI_WELCOMEPAGE_TEXT "Your files. Your destination.$\r$\n$\r$\nInstall the OmniSync Windows app, tray controls and optional Explorer integration.$\r$\n$\r$\nYour folders stay where they are. Choose a local, synced or network destination after installation. Existing folder selections and exclusions are kept.$\r$\n$\r$\nThis development build is unsigned."
!define MUI_FINISHPAGE_TEXT "OmniSync is installed.$\r$\n$\r$\nOpen it from the Start menu. Choose a backup destination. Your folder selections and exclusions are preserved; earlier Google Drive backups are untouched.$\r$\n$\r$\nExplorer may need a sign-out or refresh before new badges appear. Setup has not restarted Explorer.$\r$\n$\r$\nOverlay recovery receipts are retained in ProgramData\OmniSync\OverlayBackups."
!insertmacro MUI_PAGE_WELCOME
Page custom IntegrationPage IntegrationLeave
Page custom TakeoverPage TakeoverLeave
Page custom ReviewPage
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Var OverlayControl
Var ContextControl
Var OverlayChoice
Var ContextChoice
Var SelectionControl
Var Selection
Var InventoryText
Var Receipt

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "OmniSync requires Windows 11 x64."
    Abort
  ${EndIf}
  SetRegView 64
  ReadRegStr $0 HKLM "SOFTWARE\Microsoft\Windows NT\CurrentVersion" "CurrentBuildNumber"
  ${If} $0 < 22000
    MessageBox MB_ICONSTOP "OmniSync requires Windows 11 or later."
    Abort
  ${EndIf}
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File "${PAYLOAD}\omnisync-setup.exe"
  ${GetParameters} $3
  ClearErrors
  ${GetOptions} $3 "/WAITFORAPP" $4
  ${IfNot} ${Errors}
    ; The app launches this only after an explicit Install choice. Never kill it.
    StrCpy $5 0
    wait_for_app:
      nsExec::ExecToStack '"$PLUGINSDIR\omnisync-setup.exe" probe-running'
      Pop $0
      Pop $1
      ${If} $0 == 0
        Goto app_finished
      ${EndIf}
      IntOp $5 $5 + 1
      ${If} $5 >= 300
        MessageBox MB_ICONEXCLAMATION "OmniSync is still finishing a backup. Wait for it to close, then run setup again."
        Abort
      ${EndIf}
      Sleep 200
      Goto wait_for_app
    app_finished:
  ${EndIf}
  nsExec::ExecToStack '"$PLUGINSDIR\omnisync-setup.exe" probe-running'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION "Close OmniSync from its tray menu, then run setup again. Setup will not close it for you."
    Abort
  ${EndIf}
  StrCpy $OverlayChoice ${BST_CHECKED}
  StrCpy $ContextChoice ${BST_CHECKED}
  StrCpy $Selection ""
  nsExec::ExecToStack '"$PLUGINSDIR\omnisync-setup.exe" inventory "$PLUGINSDIR\overlays"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION "Overlay inventory could not be read. Setup cannot safely review slot conflicts.$\r$\n$1"
    Abort
  ${EndIf}
FunctionEnd

Function IntegrationPage
  !insertmacro MUI_HEADER_TEXT "Windows integration" "Choose how OmniSync appears in Explorer."
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateLabel} 0 0 100% 30u "OmniSync installs to $PROGRAMFILES64\OmniSync. Backup runs in your Windows account when the app or tray is running."
  Pop $0
  ${NSD_CreateCheckbox} 0 45u 100% 18u "Register five Explorer status overlays"
  Pop $OverlayControl
  ${NSD_SetState} $OverlayControl $OverlayChoice
  ${NSD_CreateLabel} 14u 65u 95% 32u "Backed up, syncing, pending, excluded and issue. Windows shares a limited overlay pool across installed apps."
  Pop $0
  ${NSD_CreateCheckbox} 0 105u 100% 18u "Add right-click backup, exclude and status commands"
  Pop $ContextControl
  ${NSD_SetState} $ContextControl $ContextChoice
  ${NSD_CreateLabel} 14u 125u 95% 35u "Commands appear in Show more options on Windows 11. The signed modern-menu package can be added by a release publisher."
  Pop $0
  ${NSD_CreateLabel} 0 178u 100% 40u "No competing sync app is stopped or uninstalled. The next screen offers explicit, recoverable overlay slot takeover."
  Pop $0
  nsDialogs::Show
FunctionEnd
Function IntegrationLeave
  ${NSD_GetState} $OverlayControl $OverlayChoice
  ${NSD_GetState} $ContextControl $ContextChoice
FunctionEnd

Function TakeoverPage
  !insertmacro MUI_HEADER_TEXT "Review overlay slots" "Optional: disable only the provider registrations you select."
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateLabel} 0 0 100% 32u "Leave the selection empty to keep every provider. Protected entries cannot be taken over. Existing sync applications keep working."
  Pop $0
  nsDialogs::CreateControl EDIT ${WS_VISIBLE}|${WS_CHILD}|${WS_TABSTOP}|${WS_VSCROLL}|${ES_MULTILINE}|${ES_AUTOVSCROLL}|${ES_READONLY} ${WS_EX_CLIENTEDGE} 0 35u 100% 120u ""
  Pop $InventoryText
  SendMessage $InventoryText 0x00C5 0 0
  FileOpen $0 "$PLUGINSDIR\overlays\inventory.txt" r
  FileSeek $0 2 SET
  loop_read:
    ClearErrors
    FileReadUTF16LE $0 $1
    IfErrors done_read
    SendMessage $InventoryText 0x00C2 0 "STR:$1"
    Goto loop_read
  done_read:
  FileClose $0
  SendMessage $InventoryText 0x00B1 0 0
  SendMessage $InventoryText 0x0115 6 0
  ${NSD_CreateLabel} 0 164u 100% 16u "Registration numbers to disable (comma-separated, for example 2, 4):"
  Pop $0
  ${NSD_CreateText} 0 182u 100% 18u "$Selection"
  Pop $SelectionControl
  ${NSD_CreateLabel} 0 205u 100% 22u "Values and security descriptors are backed up before removal. Newer provider keys are preserved during undo."
  Pop $0
  nsDialogs::Show
FunctionEnd
Function TakeoverLeave
  ${NSD_GetText} $SelectionControl $Selection
  ${If} $Selection != ""
    ${If} $OverlayChoice != ${BST_CHECKED}
      MessageBox MB_ICONEXCLAMATION "Enable OmniSync overlays before selecting a slot takeover, or clear the selection."
      Abort
    ${EndIf}
    MessageBox MB_YESNO|MB_DEFBUTTON2|MB_ICONQUESTION "At installation, disable only registration numbers: $Selection?$\r$\n$\r$\nOther providers may lose Explorer badges. Their applications keep syncing. Recovery receipts will be retained." IDYES confirmed
    Abort
    confirmed:
  ${EndIf}
FunctionEnd

Function ReviewPage
  !insertmacro MUI_HEADER_TEXT "Ready to install" "Review your choices before setup changes this PC."
  nsDialogs::Create 1018
  Pop $0
  StrCpy $0 "No"
  ${If} $OverlayChoice == ${BST_CHECKED}
    StrCpy $0 "Yes"
  ${EndIf}
  StrCpy $1 "No"
  ${If} $ContextChoice == ${BST_CHECKED}
    StrCpy $1 "Yes"
  ${EndIf}
  StrCpy $2 "$Selection"
  ${If} $2 == ""
    StrCpy $2 "None"
  ${EndIf}
  ${NSD_CreateLabel} 0 0 100% 150u "Application: OmniSync ${VERSION} (x64)$\r$\n$\r$\nLocation: $PROGRAMFILES64\OmniSync$\r$\n$\r$\nExplorer overlays: $0$\r$\nRight-click commands: $1$\r$\nCompeting registration numbers to disable: $2$\r$\n$\r$\nExplorer refresh: deferred$\r$\nBackup destination and source selection: after installation"
  Pop $0
  ${NSD_CreateLabel} 0 165u 100% 55u "Existing backups, local source files and other sync applications are retained. Microsoft Edge WebView2 Runtime is required; Windows 11 normally includes it. See the README if the app cannot start."
  Pop $0
  nsDialogs::Show
FunctionEnd

Section "OmniSync" SEC_APP
  SetRegView 64
  SetShellVarContext all
  StrCpy $Receipt ""
  ${If} $Selection != ""
    nsExec::ExecToStack '"$PLUGINSDIR\omnisync-setup.exe" takeover "$PLUGINSDIR\overlays\inventory.json" "$Selection"'
    Pop $0
    Pop $Receipt
    ${TrimNewLines} $Receipt $Receipt
    ${If} $0 != 0
      MessageBox MB_ICONSTOP "Overlay takeover stopped. Setup did not continue.$\r$\n$Receipt"
      Abort
    ${EndIf}
  ${EndIf}
  SetOutPath "$INSTDIR\${VERSION}"
  File "${PAYLOAD}\omnisync.exe"
  File "${PAYLOAD}\omnisync_shell.dll"
  File "${PAYLOAD}\omnisync-setup.exe"
  File "${PAYLOAD}\README.md"
  SetOutPath "$INSTDIR\${VERSION}\docs"
  File "${PAYLOAD}\docs\guide.html"
  SetOutPath "$INSTDIR\${VERSION}\icons"
  File "${PAYLOAD}\icons\*.ico"
  StrCpy $0 "register"
  ${If} $OverlayChoice == ${BST_CHECKED}
    StrCpy $0 "$0 --overlays"
  ${EndIf}
  ${If} $ContextChoice == ${BST_CHECKED}
    StrCpy $0 "$0 --context"
  ${EndIf}
  nsExec::ExecToStack '"$INSTDIR\${VERSION}\omnisync-setup.exe" $0'
  Pop $0
  Pop $1
  ${If} $0 != 0
    nsExec::ExecToLog '"$INSTDIR\${VERSION}\omnisync-setup.exe" unregister'
    ${If} $Receipt != ""
      ; The receipt remains available even if an interrupted installer cannot finish rollback.
      DetailPrint "Overlay recovery receipt: $Receipt"
    ${EndIf}
    MessageBox MB_ICONSTOP "Explorer registration failed.$\r$\n$1$\r$\nReview recovery receipts in ProgramData\OmniSync\OverlayBackups."
    Abort
  ${EndIf}
  CreateDirectory "$SMPROGRAMS\OmniSync"
  CreateShortcut "$SMPROGRAMS\OmniSync\OmniSync.lnk" "$INSTDIR\${VERSION}\omnisync.exe" "" "$INSTDIR\${VERSION}\icons\app.ico"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\OmniSync" "DisplayName" "OmniSync"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\OmniSync" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\OmniSync" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\OmniSync" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\OmniSync" "DisplayIcon" "$INSTDIR\${VERSION}\icons\app.ico"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\OmniSync" "URLInfoAbout" "https://github.com/william-pyle/OmniSync"
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\OmniSync" "NoModify" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\OmniSync" "NoRepair" 1
SectionEnd

Function un.onInit
  nsExec::ExecToStack '"$INSTDIR\${VERSION}\omnisync-setup.exe" probe-running'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION "Close OmniSync from its tray menu before uninstalling."
    Abort
  ${EndIf}
FunctionEnd
Function .onInstFailed
  ${If} $Receipt != ""
    nsExec::ExecToStack '"$PLUGINSDIR\omnisync-setup.exe" restore "$Receipt"'
    Pop $0
    Pop $1
    ${If} $0 != 0
      MessageBox MB_ICONEXCLAMATION "Automatic overlay rollback needs review. The protected receipt remains at:$\r$\n$Receipt$\r$\n$1"
    ${EndIf}
  ${EndIf}
FunctionEnd
Section "Uninstall"
  SetRegView 64
  SetShellVarContext all
  nsExec::ExecToStack '"$INSTDIR\${VERSION}\omnisync-setup.exe" unregister'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP "Integration cleanup failed. Files were retained for recovery.$\r$\n$1"
    Abort
  ${EndIf}
  Delete "$SMPROGRAMS\OmniSync\OmniSync.lnk"
  RMDir "$SMPROGRAMS\OmniSync"
  Delete /REBOOTOK "$INSTDIR\${VERSION}\omnisync.exe"
  Delete /REBOOTOK "$INSTDIR\${VERSION}\omnisync_shell.dll"
  Delete /REBOOTOK "$INSTDIR\${VERSION}\omnisync-setup.exe"
  Delete "$INSTDIR\${VERSION}\README.md"
  Delete "$INSTDIR\${VERSION}\docs\guide.html"
  RMDir "$INSTDIR\${VERSION}\docs"
  Delete "$INSTDIR\${VERSION}\icons\*.ico"
  RMDir "$INSTDIR\${VERSION}\icons"
  RMDir /REBOOTOK "$INSTDIR\${VERSION}"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\OmniSync"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir /REBOOTOK "$INSTDIR"
  MessageBox MB_OK "OmniSync was removed. Source files, Google Drive backups, local settings and overlay recovery receipts were retained. A loaded shell DLL may be removed at the next restart; setup will not restart Windows."
SectionEnd
