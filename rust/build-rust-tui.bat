@echo off
rem Build the Rust TUI (herdr-talk.exe) into rust\target\debug. Stops with an error code when the build fails.
chcp 65001 >nul
cd /d C:\work\app\herdr-talk\rust
cargo build -p herdr-talk
if errorlevel 1 (
  echo.
  echo BUILD FAILED
  pause
  exit /b 1
)
echo.
echo BUILD OK: target\debug\herdr-talk.exe
