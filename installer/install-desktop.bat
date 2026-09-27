@echo off
rem Thin launcher for `bootsmasher install` (the whole installer lives in
rem the binary: menus, fetch/rebuild/report/flash, export.txt paths).
rem Binaries live in bin\windows\ as install[-small]-windows-<arch>;
rem the full build wins, small is the fallback. Then legacy spots.
rem Every argument is forwarded unchanged, except a drag-and-dropped
rem file: when the FIRST argument is an existing file (not a flag) it
rem is consumed as the recovery cpio payload and forwarded as
rem --recovery-img (overrides export.txt RECOVERY_IMG for this run,
rem no editing needed). Just drop the .lz4 onto this .bat.
rem No payload at all (nothing dropped, export.txt entry missing) ->
rem a system file picker (PowerShell OpenFileDialog) asks for it.
rem On interactive runs the script prints the menu keys up front and
rem pauses for Enter at the end. (The reboot-to-recovery question lives
rem inside the binary now, so the launcher never asks twice.)
rem Every exit path goes through :finish, which waits for user
rem confirmation (except --force) so a double-clicked window never
rem vanishes before the result is read.
rem NOTE: paths ($ROOT, $BIN, ...) are ALWAYS referenced as !VAR!
rem (never %VAR%), because %VAR% eats `!` when delayed expansion is
rem on — and user paths do contain `!` (e.g. ...!App\...). ROOT is
rem captured before delayed expansion is even enabled.
set "ROOT=%~dp0"
set "ROOT=%ROOT:~0,-1%"
setlocal EnableDelayedExpansion

if /i "%PROCESSOR_ARCHITECTURE%"=="ARM64" (
  set "ARCH=arm64"
) else (
  set "ARCH=x86_64"
)

set "BIN="
for %%n in (install-windows-!ARCH!.exe install-small-windows-!ARCH!.exe) do (
  if not defined BIN (
    if exist "!ROOT!\bin\windows\%%n" set "BIN=!ROOT!\bin\windows\%%n"
  )
)
if not defined BIN (
  for %%n in (bootsmasher.exe bootsmasher-x86_64.exe) do (
    if not defined BIN (
      if exist "!ROOT!\%%n" set "BIN=!ROOT!\%%n"
    )
  )
)
if not defined BIN (
  for %%n in (bootsmasher.exe bootsmasher-x86_64.exe) do (
    if not defined BIN (
      if exist "!ROOT!\..\target\release\%%n" set "BIN=!ROOT!\..\target\release\%%n"
    )
  )
)
if not defined BIN (
  for /f "delims=" %%p in ('where bootsmasher.exe 2^>nul') do (
    if not defined BIN set "BIN=%%p"
  )
)
if not defined BIN (
  echo no installer binary ^(expected bin\windows\install[-small]-windows-^<arch^>.exe next to install-desktop.bat^) 1>&2
  echo -- diagnostic: ARCH=!ARCH! PROCESSOR_ARCHITECTURE=%PROCESSOR_ARCHITECTURE% 1>&2
  echo -- diagnostic: ROOT=!ROOT! 1>&2
  for %%n in (install-windows-x86_64.exe install-small-windows-x86_64.exe install-windows-arm64.exe install-small-windows-arm64.exe) do (
    if exist "!ROOT!\bin\windows\%%n" (echo -- diagnostic: FOUND bin\windows\%%n 1>&2) else (echo -- diagnostic: MISSING bin\windows\%%n 1>&2)
  )
  if exist "!ROOT!\bin\windows" (echo -- diagnostic: bin\windows dir exists 1>&2) else (echo -- diagnostic: bin\windows dir MISSING 1>&2)
  if exist "!ROOT!\bin" (echo -- diagnostic: bin dir exists 1>&2) else (echo -- diagnostic: bin dir MISSING 1>&2)
  set "RC=1"
  goto finish
)

rem Drag-and-drop payload: %1 as an existing file (not a flag) is
rem consumed as the recovery cpio and forwarded as --recovery-img
rem (overrides export.txt RECOVERY_IMG for this run, no editing).
rem Only %1 qualifies; flags and the rest pass through untouched.
rem %* is NOT shift-aware, so the remaining args are rebuilt into
rem ARGS (every token re-quoted) and used below instead of %*.
set "DROP="
set "ARGS="
set "A1=%~1"
if defined A1 if not "!A1:~0,1!"=="-" if exist "%~1" (
  set "DROP=%~f1"
  shift
)
:collect_args
if "%~1"=="" goto collected_args
set "ARGS=!ARGS! \"%~1\""
shift
goto collect_args
:collected_args
if defined DROP echo Payload ^(dropped file^): !DROP!

rem Which export.txt this run uses (caller's --export wins, else the
rem one next to this script).
set "EXPORT_FILE=!ROOT!\export.txt"
set "PREV="
if defined ARGS (
  for %%a in (!ARGS!) do (
    if "!PREV!"=="--export" (
      set "EXPORT_FILE=%%~a"
      set "PREV="
    ) else if "%%~a"=="--export" (
      set "PREV=--export"
    ) else (
      set "EX=%%~a"
      if "!EX:~0,9!"=="--export=" set "EXPORT_FILE=!EX:~9!"
    )
  )
)
rem A relative --export means the caller's working directory (same as
rem the binary treats it); anchor it so all later paths resolve there.
set "EF=!EXPORT_FILE!"
if not "!EF:~1,1!"==":" if not "!EF:~0,1!"=="\" if not "!EF:~0,2!"=="\\" (
  set "EXPORT_FILE=!CD!\!EF!"
)

rem System file picker: no drop, no explicit --recovery-img, not a
rem scripted/non-payload flow (--force/--file/help: the caller knows
rem what it is doing), and the export.txt payload is missing -> ask
rem Windows for the file instead of failing (covers a stale export.txt
rem and bare double-clicks). A cancelled dialog proceeds and the
rem binary reports its usual clear error.
set "NEED_PICK=1"
if defined DROP set "NEED_PICK=0"
echo !ARGS! | findstr /c:"--recovery-img" >nul 2>&1
if not errorlevel 1 set "NEED_PICK=0"
echo !ARGS! | findstr /i /c:"--force" /c:"-h" /c:"--help" /c:"--file" >nul 2>&1
if not errorlevel 1 set "NEED_PICK=0"
if "!NEED_PICK!"=="1" (
  for %%d in ("!EXPORT_FILE!") do set "EXDIR=%%~dpd"
  set "EXPAY="
  for /f "tokens=1* delims==" %%k in ('findstr /r /c:"^RECOVERY_IMG=" /c:"^ *RECOVERY_IMG=" "!EXPORT_FILE!" 2^>nul') do set "EXPAY=%%l"
  if defined EXPAY (
    if "!EXPAY:~0,1!"==""" set "EXPAY=!EXPAY:~1,-1!"
    if "!EXPAY:~0,1!"=="'" set "EXPAY=!EXPAY:~1,-1!"
  )
  set "PAY=!EXPAY!"
  if defined EXPAY (
    if not "!EXPAY:~1,1!"==":" if not "!EXPAY:~0,2!"=="\\" set "PAY=!EXDIR!!EXPAY!"
  )
  set "PICK_NEEDED=1"
  if defined PAY if exist "!PAY!" set "PICK_NEEDED=0"
  if "!PICK_NEEDED!"=="1" (
    set "PICKFILE=!TEMP!\ofpick_!RANDOM!.txt"
    powershell -NoProfile -STA -Command "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.OpenFileDialog; $d.Title = 'OrangeFox recovery payload'; $d.Filter = 'Recovery payload (*.lz4;*.cpio;*.img)|*.lz4;*.cpio;*.img|All files (*.*)|*.*'; $d.InitialDirectory = '!EXDIR!'; if ($d.ShowDialog() -eq 'OK') { $d.FileName }" > "!PICKFILE!" 2>nul
    set "PICK="
    if exist "!PICKFILE!" set /p "PICK=" < "!PICKFILE!"
    del "!PICKFILE!" 2>nul
    if defined PICK (
      echo Payload (file picker): !PICK!
      set "DROP=!PICK!"
    )
  )
)

rem Menu runs are interactive; usage/help/--force/--file are not.
rem (Matched against the rebuilt arg list so a dropped filename can
rem never trip the heuristics.)
set "INTERACTIVE=1"
echo !ARGS! | findstr /i /c:"--force" /c:"-h" /c:"--help" /c:"--file" >nul 2>&1 && set "INTERACTIVE=0"
if "!INTERACTIVE!"=="1" (
  echo Arrow-key menus: Up/Down to move, Enter to choose, q/Esc to exit.
  echo.
)

rem Pin export.txt so the launcher works from any working directory.
echo !ARGS! | findstr /c:"--export" >nul 2>&1
if errorlevel 1 (
  set PIN=--export "!EXPORT_FILE!"
) else (
  set "PIN="
)

rem Dropped payload rides as --recovery-img (quoted once here; the
rem binary strips it). Empty when nothing was dropped.
set "RIMG="
set "RIMGV="
if defined DROP (
  set "RIMG=--recovery-img"
  set "RIMGV="!DROP!""
)

rem UTF-8 console so the installer menus render correctly.
chcp 65001 >nul
"!BIN!" install !PIN! !RIMG! !RIMGV! !ARGS!
set "RC=%ERRORLEVEL%"
goto finish

:finish
rem Single exit gate: wait for user confirmation (except --force)
rem so the window never closes before the result is read, then exit
rem with the installer's code. %RC% stays percent-form on purpose:
rem it expands before endlocal runs (RC is always numeric anyway).
if not defined RC set "RC=1"
set "PAUSEME=1"
echo !ARGS! | findstr /i /c:"--force" /c:"-h" /c:"--help" >nul 2>&1 && set "PAUSEME=0"
if "!PAUSEME!"=="1" (
  echo.
  set /p "DUMMY=Press Enter to exit... "
)
endlocal & exit /b %RC%
