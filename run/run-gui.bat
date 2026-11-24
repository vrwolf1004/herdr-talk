@echo off
rem GUI: talk-core 와 herdr-talk(CLI), GUI 를 release 로 빌드하고 실행. 코드가 안 바뀌었으면 재컴파일할지 묻는다.
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
set "EXE=%ROOT%\gui\target\release\herdr-talk-gui.exe"
set "SRC=%ROOT%\gui\src,%ROOT%\rust\crates\talk-core\src"
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
pushd "%ROOT%\rust"
cargo clean --release -p talk-core -p herdr-talk >nul 2>&1
popd
pushd "%ROOT%\gui"
cargo clean --release -p herdr-talk-gui >nul 2>&1
popd
:build
echo [1/2] CLI 빌드: talk-core + herdr-talk (release)
pushd "%ROOT%\rust"
cargo build --release -p herdr-talk || (popd & pause & exit /b 1)
popd
echo [2/2] GUI 빌드 (release)
pushd "%ROOT%\gui"
cargo build --release || (popd & pause & exit /b 1)
popd
echo [RUN] GUI 실행
start "herdr-talk-gui" "%ROOT%\gui\target\release\herdr-talk-gui.exe"