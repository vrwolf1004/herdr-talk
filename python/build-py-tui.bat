@echo off
rem Python TUI 을 단일 exe(dist\herdr-talk-py.exe)로 빌드한다. PyInstaller 필요: python -m pip install pyinstaller
chcp 65001 >nul
cd /d "%~dp0"
python -m compileall -q src || (pause & exit /b 1)
python -m PyInstaller --noconfirm --clean --onefile --name herdr-talk-py --paths src --add-data "%~dp0src\locales;locales" --distpath dist --workpath build --specpath build src\talk.py || (pause & exit /b 1)
echo.
echo BUILD OK: %~dp0dist\herdr-talk-py.exe
