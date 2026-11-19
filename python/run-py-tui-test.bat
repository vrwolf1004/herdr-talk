@echo off
rem Python TUI in a new window with a TEST state folder (does not touch the real saved settings).
chcp 65001 >nul
set "HERDR_PLUGIN_STATE_DIR=C:\Users\PC\AppData\Local\Temp\claude\2c78217723a84c60\9039351c-ea57-4c9d-a4ff-8d552df51778\scratchpad\tui-py-state"
start "py-tui" cmd /k "chcp 65001 >nul && cd /d C:\work\app\herdr-talk\python && python src\talk.py run"
