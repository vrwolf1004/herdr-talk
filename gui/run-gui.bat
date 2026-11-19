@echo off
rem Build the GUI (herdr-talk-gui.exe) into gui\target\debug, then start it. Stops with an error code when the build fails.
chcp 65001 >nul
cd /d %~dp0
cargo build
if errorlevel 1 (
  echo.
  echo BUILD FAILED
  pause
  exit /b 1
)
echo.
echo BUILD OK: target\debug\herdr-talk-gui.exe
start "herdr-talk-gui" target\debug\herdr-talk-gui.exe
