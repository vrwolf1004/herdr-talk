@echo off
rem Python TUI in a new window with the REAL saved settings (the normal user state folder).
chcp 65001 >nul
start "py-tui" cmd /k "chcp 65001 >nul && cd /d C:\work\app\herdr-talk\python && python src\talk.py run --name py-tui"
