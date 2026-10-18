@echo off
rem Deploys the DonutFree portable build from the D: cargo target.
rem Usage: deploy-free.cmd [destination]   (default C:\Users\max\Desktop\Donut-Free)
setlocal
set "SRC=D:\ZDownloads\donut-free-target\release"
set "DST=%~1"
if "%DST%"=="" set "DST=C:\Users\max\Desktop\Donut-Free"

if not exist "%SRC%\donutbrowser.exe" (
  echo ERROR: %SRC%\donutbrowser.exe not found - build first.
  exit /b 1
)

mkdir "%DST%" 2>nul
rem Raw tauri output keeps the cargo package name; ship it as DonutFree.exe.
copy /y "%SRC%\donutbrowser.exe" "%DST%\DonutFree.exe" >nul || goto :fail
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
> "%DST%\version.txt" echo donut-free 0.34.0 built %date% %time%

echo DEPLOY_OK: %DST%
exit /b 0
:fail
echo DEPLOY_FAILED
exit /b 1
