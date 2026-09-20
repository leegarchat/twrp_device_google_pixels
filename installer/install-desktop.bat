@echo off
rem Thin launcher for `bootsmasher install` (the whole installer lives in
rem the binary: menus, fetch/rebuild/report/flash, export.txt paths).
rem Binaries live in bin\windows\ as install[-small]-windows-<arch>;
rem the full build wins, small is the fallback. Then legacy spots.
rem Every argument is forwarded unchanged.
rem On interactive runs the script prints the menu keys up front,
rem pauses for Enter at the end, and after a successful run offers
rem to reboot the device to recovery via fastboot (default: No).
rem Every exit path goes through :finish, which waits for user
rem confirmation (except --force) so a double-clicked window never
rem vanishes before the result is read.
rem NOTE: paths ($ROOT, $BIN, ...) are ALWAYS referenced as !VAR!
rem (never %VAR%), because %VAR% eats `!` when delayed expansion is
rem on — and user paths do contain `!` (e.g. ...!App\...). ROOT and
rem BAT are captured before delayed expansion is even enabled.
rem NOTE: file parsing uses find and for/f (not findstr): findstr is
rem unreliable on UTF-8 codepages (chcp 65001 is set below for the
rem menus) while find does a plain byte search.
set "ROOT=%~dp0"
set "ROOT=%ROOT:~0,-1%"
set "BAT=%~f0"
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

rem Which export.txt this run uses (caller's --export wins, else the
rem one next to this script).
set "EXPORT_FILE=!ROOT!\export.txt"
set "PREV="
if not "%*"=="" (
  for %%a in (%*) do (
    if "!PREV!"=="--export" (
      set "EXPORT_FILE=%%a"
      set "PREV="
    ) else if "%%a"=="--export" (
      set "PREV=--export"
    ) else (
      set "EX=%%a"
      if "!EX:~0,9!"=="--export=" set "EXPORT_FILE=!EX:~9!"
    )
  )
)
rem Carriage return for trimming CRLF-edited values (copy /Z trick).
set "CR="
for /f %%C in ('copy /z "!BAT!" nul') do set "CR=%%C"
rem A relative --export means the caller's working directory (same as
rem the binary treats it); anchor it so all later paths resolve there.
set "EF=!EXPORT_FILE!"
if not "!EF:~1,1!"==":" if not "!EF:~0,1!"=="\" if not "!EF:~0,2!"=="\\" (
  set "EXPORT_FILE=!CD!\!EF!"
)
call :getcfg BACKUP_DIR backup BACKUP_DIR
call :abspath BACKUP_DIR

rem Menu runs are interactive; usage/help/--force/--file are not.
set "INTERACTIVE=1"
echo %* | findstr /i /c:"--force" /c:"-h" /c:"--help" /c:"--file" >nul 2>&1 && set "INTERACTIVE=0"
if "!INTERACTIVE!"=="1" (
  echo Arrow-key menus: Up/Down to move, Enter to choose, q/Esc to exit.
  echo.
)

rem Pin export.txt so the launcher works from any working directory.
echo %* | findstr /c:"--export" >nul 2>&1
if errorlevel 1 (
  set PIN=--export "!EXPORT_FILE!"
) else (
  set "PIN="
)

rem UTF-8 console so the installer menus render correctly.
chcp 65001 >nul
"!BIN!" install !PIN! %*
set "RC=%ERRORLEVEL%"

rem After a successful run (exit 0, not --force/--file/help), always
rem offer the reboot: the user has just watched the result scroll by,
rem so no log forensics can wrongly swallow the question. The serial
rem is read best-effort for a targeted reboot; without it the reboot
rem still goes out broadcast (works with a single device attached).
set "ASK=0"
if "!RC!"=="0" (
  echo %* | findstr /i /c:"--force" /c:"--file" /c:"-h" /c:"--help" >nul 2>&1 || set "ASK=1"
)
if "!ASK!"=="1" call :maybe_reboot
goto finish

:maybe_reboot
set "SERIAL="
set "AFTER="
for /f "delims=" %%d in ('dir "!BACKUP_DIR!" /b /ad /o-d 2^>nul') do (
  if not defined AFTER set "AFTER=%%d"
)
if defined AFTER (
  for /f "delims=" %%l in ('find "device=" "!BACKUP_DIR!\!AFTER!\install.log" 2^>nul') do (
    set "L=%%l"
    if "!L:~0,7!"=="device=" set "SERIAL=!L:~7!"
  )
)
if defined SERIAL (
  set /p "ANS=Reboot !SERIAL! to recovery now? [y/N] "
) else (
  set /p "ANS=Reboot the device to recovery now? [y/N] "
)
if /i "%ANS:~0,1%"=="y" goto do_reboot
echo   (leaving the device in the bootloader)
exit /b 0
:do_reboot
call :getcfg PLATFORM_TOOLS_WINDOWS platform-tools-windows PT
call :abspath PT
call :getcfg FASTBOOT_BIN fastboot FBIN
set "FB=!PT!\!FBIN!"
if not exist "!FB!" if exist "!FB!.exe" set "FB=!FB!.exe"
if not exist "!FB!" (
  rem Fallback: standard layout next to the installer binary itself
  rem (BIN lives in bin\windows\, so one level up + platform-tools).
  for %%F in ("!BIN!\..\platform-tools\fastboot.exe") do set "FB=%%~fF"
)
if not exist "!FB!" (
  echo   fastboot not found ^(!FB!^): reboot manually ^(bootloader menu -^> Recovery mode^)
  exit /b 0
)
if defined SERIAL (
  echo   rebooting !SERIAL! to recovery...
  "!FB!" -s "!SERIAL!" reboot recovery
) else (
  echo   rebooting to recovery...
  "!FB!" reboot recovery
)
if errorlevel 1 (
  echo   reboot failed: select Recovery mode in the bootloader menu manually
) else (
  echo   reboot command sent
)
exit /b 0

:getcfg
rem %1=key %2=default %3=outvar: one KEY=VALUE out of export.txt.
rem Read with for/f (no findstr: unreliable on UTF-8 codepages).
set "%~3=%~2"
for /f "usebackq tokens=1* delims==" %%k in ("!EXPORT_FILE!") do (
  if "%%k"=="%~1" set "%~3=%%l"
)
call :trim %~3
exit /b 0

:trim
rem Trim leading/trailing spaces and CR (CRLF-safe for Notepad edits).
set "V=!%~1!"
for /f "tokens=* delims= " %%t in ("!V!") do set "V=%%t"
:trimloop
if "!V!"=="" goto trimdone
if "!V:~-1!"==" " (
  set "V=!V:~0,-1!"
  goto trimloop
)
if defined CR if "!V:~-1!"=="!CR!" (
  set "V=!V:~0,-1!"
  goto trimloop
)
:trimdone
set "%~1=!V!"
exit /b 0

:abspath
rem Make %1 absolute against the export file's directory if relative.
set "P=!%~1!"
set "ABS=0"
if "!P:~1,1!"==":" set "ABS=1"
if "!P:~0,2!"=="\\" set "ABS=1"
if "!ABS!"=="0" (
  for %%e in ("!EXPORT_FILE!") do set "EB=%%~dpe"
  set "EB=!EB:~0,-1!"
  set "%~1=!EB!\!P!"
)
exit /b 0

:finish
rem Single exit gate: wait for user confirmation (except --force)
rem so the window never closes before the result is read, then exit
rem with the installer's code. %RC% stays percent-form on purpose:
rem it expands before endlocal runs (RC is always numeric anyway).
if not defined RC set "RC=1"
set "PAUSEME=1"
echo %* | findstr /i /c:"--force" /c:"-h" /c:"--help" >nul 2>&1 && set "PAUSEME=0"
if "!PAUSEME!"=="1" (
  echo.
  set /p "DUMMY=Press Enter to exit... "
)
endlocal & exit /b %RC%
