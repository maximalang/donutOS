@echo off
rem Deploys the DonutOS portable build from the D: cargo target.
rem Usage: deploy-free.cmd [destination]   (default C:\Users\max\Desktop\DonutOS)
setlocal
set "SRC=D:\ZDownloads\donut-free-target\release"
set "DST=%~1"
if "%DST%"=="" set "DST=C:\Users\max\Desktop\DonutOS"

if not exist "%SRC%\donutbrowser.exe" (
  echo ERROR: %SRC%\donutbrowser.exe not found - build first.
  exit /b 1
)

mkdir "%DST%" 2>nul
rem Raw tauri output keeps the cargo package name; ship it as DonutOS.exe.
copy /y "%SRC%\donutbrowser.exe" "%DST%\DonutOS.exe" >nul || goto :fail
for %%F in (donut-proxy.exe xray.exe donut-xray.exe) do (
  if exist "%SRC%\%%F" copy /y "%SRC%\%%F" "%DST%\" >nul
)
if exist "%SRC%\resources" (
  if not exist "%DST%\resources" mkdir "%DST%\resources"
  xcopy /e /i /y "%SRC%\resources" "%DST%\resources" >nul
)

rem Portable marker: keeps all data next to the exe.
type nul > "%DST%\.portable"

rem Pre-seed settings: local API ON, port pinned to 10109 (prod v0.29.6 owns 10108).
if not exist "%DST%\data\settings" mkdir "%DST%\data\settings"
if not exist "%DST%\data\settings\app_settings.json" (
  > "%DST%\data\settings\app_settings.json" echo {"api_enabled": true, "api_port": 10109}
)

rem Stable automation token (survives data resets; read via DONUT_AUTOMATION_TOKEN at start).
if not exist "%DST%\config" mkdir "%DST%\config"
if not exist "%DST%\config\automation-token.txt" (
  powershell -NoProfile -Command "([guid]::NewGuid().ToString('N')+[guid]::NewGuid().ToString('N'))" > "%DST%\config\automation-token.txt"
)

copy /y "%~dp0donut-freectl.cmd" "%DST%\" >nul
copy /y "%~dp0donut-freectl.ps1" "%DST%\" >nul
rem Watchdog ships with the deploy so its two operating modes (on-demand /
rem -AlwaysOn) reach the instance instead of drifting from a hand-copied file.
copy /y "%~dp0donut-watchdog.ps1" "%DST%\" >nul
rem Retire stale exe artifacts from older deploys: raw cargo-name binaries and
rem .bak leftovers no current deploy step owns (observed in the wild:
rem donutbrowser.stale-20241019.exe.bak). The shipped set is DonutOS.exe
rem (freshly copied above) + donut-proxy.exe + xray.exe + donut-xray.exe.
for %%F in ("%DST%\donutbrowser*.exe" "%DST%\*.exe.bak") do (
  if exist "%%~F" (
    del /q "%%~F"
    echo Retired stale artifact: %%~nxF
  )
)
for %%F in ("%DST%\*.exe") do (
  if /i not "%%~nxF"=="DonutOS.exe" if /i not "%%~nxF"=="donut-proxy.exe" if /i not "%%~nxF"=="xray.exe" if /i not "%%~nxF"=="donut-xray.exe" (
    del /q "%%~F"
    echo Retired stale artifact: %%~nxF
  )
)

> "%DST%\version.txt" echo donut-free 0.34.0 built %date% %time%

echo DEPLOY_OK: %DST%
exit /b 0
:fail
echo DEPLOY_FAILED
exit /b 1
