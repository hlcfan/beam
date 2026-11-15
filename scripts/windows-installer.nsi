; Build from the repository root with VERSION, BINARY and OUTPUT defined.
Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

Name "Beam"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\Beam"
RequestExecutionLevel user
SetCompressor /SOLID lzma
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Beam"
!define MUI_ICON "..\assets\icon.ico"
!define MUI_UNICON "..\assets\icon.ico"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
    ${IfNot} ${RunningX64}
        MessageBox MB_OK|MB_ICONSTOP "Beam requires 64-bit Windows."
        Abort
    ${EndIf}
    SetShellVarContext current
    SetRegView 64
FunctionEnd

Section "Beam"
    SetOutPath "$INSTDIR"
    ; Abort rather than scheduling replacement if Beam is still running.
    SetOverwrite on
    ClearErrors
    File /nonfatal /oname=beam.exe "${BINARY}"
    ${If} ${Errors}
        MessageBox MB_OK|MB_ICONSTOP "Could not install Beam. Close Beam and try again."
        SetErrorLevel 1
        Abort
    ${EndIf}
    WriteUninstaller "$INSTDIR\Uninstall.exe"
    CreateShortcut "$SMPROGRAMS\Beam.lnk" "$INSTDIR\beam.exe"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "Beam"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "hlcfan"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\beam.exe"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
    WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
    SetShellVarContext current
    SetRegView 64
    ClearErrors
    Delete "$INSTDIR\beam.exe"
    ${If} ${Errors}
        MessageBox MB_OK|MB_ICONSTOP "Could not uninstall Beam. Close Beam and try again."
        SetErrorLevel 1
        Abort
    ${EndIf}
    Delete "$SMPROGRAMS\Beam.lnk"
    DeleteRegKey HKCU "${UNINSTALL_KEY}"
    Delete "$INSTDIR\Uninstall.exe"
    ; Remove only installed files; retain workspace data and any other files.
    RMDir "$INSTDIR"
SectionEnd
