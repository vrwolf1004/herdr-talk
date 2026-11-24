@echo off
rem Python TUI: exe 빌드 후 새 창에서 실행. 저장된 실제 설정 폴더를 쓴다.
chcp 65001 >nul
set "ROOT=%~dp0.."
rem 실행 중인 herdr-talk 프로세스가 있으면 빌드를 막는다 (exe 가 잠겨 링크가 실패한다)
set "RUNNING="
for %%P in (herdr-talk.exe herdr-talk-gui.exe herdr-talk-py.exe) do (
    tasklist /FI "IMAGENAME eq %%P" 2>nul | find /I "%%P" >nul && set "RUNNING=%%P"
)
if defined RUNNING (
    echo [ERROR] 실행 중인 %RUNNING% 가 있습니다. 창을 닫은 뒤 다시 실행하세요.
    pause
    exit /b 1
)
call "%ROOT%\python\build-py-tui.bat"
if errorlevel 1 exit /b 1
start "py-tui" cmd /k "chcp 65001 >nul && cd /d "%ROOT%\python" && dist\herdr-talk-py.exe run"