@echo off
rem Expo mobile run. No argument: show the menu. Argument: web, android or expo.
chcp 65001 >nul
set "ROOT=%~dp0.."
set "MODE=%~1"
if defined MODE goto check

echo ==============================================================
echo  1) web      - PC browser
echo  2) android  - emulator or USB phone
echo  3) expo     - QR code, scan with Expo Go
echo  q) quit
echo ==============================================================
set "CHOICE="
set /p "CHOICE=Select [1-3, q=quit]: "
if /i "%CHOICE%"=="q" exit /b 0
if "%CHOICE%"=="1" set "MODE=web"
if "%CHOICE%"=="2" set "MODE=android"
if "%CHOICE%"=="3" set "MODE=expo"

:check
if /i "%MODE%"=="web" goto run
if /i "%MODE%"=="android" goto run
if /i "%MODE%"=="expo" goto run
echo [ERROR] Unknown choice: %CHOICE%%MODE%
pause
exit /b 1

:run
pushd "%ROOT%\mobile"
if not exist node_modules (
    echo [BUILD] npm install
    call npm install || (popd & pause & exit /b 1)
)
echo [RUN] Expo %MODE%
if /i "%MODE%"=="expo" (call npx expo start) else (call npx expo start --%MODE%)
popd
if errorlevel 1 pause