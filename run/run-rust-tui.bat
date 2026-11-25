@echo off
rem Rust TUI: talk-core 와 herdr-talk 빌드 후 새 창에서 실행. 코드가 안 바뀌었으면 재컴파일할지 묻는다.
rem 번호를 주면 (예: run-rust-tui.bat 1) 그 번호의 테스트 폴더를 쓰고 LAN(BASE STATION)으로 바로 시작한다.
chcp 65001 >nul
set "ROOT=%~dp0.."
if "%~1"=="" goto nostate
set "HERDR_PLUGIN_STATE_DIR=%LOCALAPPDATA%\herdr-talk-test%~1"
if not exist "%HERDR_PLUGIN_STATE_DIR%\" mkdir "%HERDR_PLUGIN_STATE_DIR%"
if not exist "%HERDR_PLUGIN_STATE_DIR%\relay.json" > "%HERDR_PLUGIN_STATE_DIR%\relay.json" echo {"mode":"local"}
echo [테스트 %~1] 설정 폴더: %HERDR_PLUGIN_STATE_DIR%
:nostate
rem 실행 중인 herdr-talk.exe 는 다시 링크할 때만 문제다 (exe 가 잠겨 링크가 실패한다). 빌드 직전에 검사한다.
set "RUNNING="
for %%P in (herdr-talk.exe) do (
    tasklist /FI "IMAGENAME eq %%P" 2>nul | find /I "%%P" >nul && set "RUNNING=%%P"
)
set "EXE=%ROOT%\rust\target\release\herdr-talk.exe"
set "SRC=%ROOT%\rust\crates\talk-core\src,%ROOT%\rust\crates\herdr-talk\src"
for /f "delims=" %%S in ('powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0check-changed.ps1" -Exe "%EXE%" -Src "%SRC%"') do set "STATE=%%S"
if "%STATE%"=="SAME" goto ask
goto build
:ask
echo 코드 변경이 없습니다.
echo [1] 재컴파일   [2] 그대로 빌드
set "ANS="
set /p "ANS=선택 [1-2]: "
if "%ANS%"=="1" goto clean
goto build
:clean
if defined RUNNING (
    echo [ERROR] 실행 중인 %RUNNING% 가 있어 재컴파일할 수 없습니다. 창을 닫은 뒤 다시 실행하세요.
    pause
    exit /b 1
)
pushd "%ROOT%\rust"
cargo clean --release -p talk-core -p herdr-talk >nul 2>&1
popd
:build
if defined RUNNING if "%STATE%"=="CHANGED" (
    echo [ERROR] 코드가 바뀌어 다시 빌드해야 하는데 실행 중인 %RUNNING% 가 있습니다. 창을 닫은 뒤 다시 실행하세요.
    pause
    exit /b 1
)
echo [1/2] Rust TUI 빌드: talk-core + herdr-talk (release)
pushd "%ROOT%\rust"
cargo build --release -p herdr-talk || (popd & pause & exit /b 1)
popd
echo [2/2] TUI 실행 창 열기
start "rust-tui" cmd /k "chcp 65001 >nul && cd /d "%ROOT%\rust" && target\release\herdr-talk.exe run"
