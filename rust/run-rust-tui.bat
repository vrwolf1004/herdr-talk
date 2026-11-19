@echo off
rem Build, then run the Rust TUI in a new window with the REAL saved settings (the normal user state folder).
chcp 65001 >nul
call "%~dp0build-rust-tui.bat"
if errorlevel 1 exit /b 1
start "rust-tui" cmd /k "chcp 65001 >nul && cd /d C:\work\app\herdr-talk\rust && target\debug\herdr-talk.exe run"
