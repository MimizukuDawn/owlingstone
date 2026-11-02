@echo off
rem Met a jour Owlingstone : teste le code du projet, reconstruit Owlingstone.exe, puis le relance.
rem Double-clic, ou bouton "Update" de l'appli. Mes donnees (%APPDATA%\Owlingstone, ex-BGTracker) ne sont jamais touchees.
rem Si un test ou la construction echoue, l'ancienne version reste en place et continue de marcher.
cd /d "%~dp0"
title Owlingstone update
echo === Owlingstone update ===
if not exist .venv\Scripts\python.exe py -3.12 -m venv .venv
.venv\Scripts\python -m pip install --quiet --disable-pip-version-check -r requirements.txt || goto failed
echo Running tests...
.venv\Scripts\python -m pytest -q -m "not slow" || goto failed
echo Building the new version...
if exist build\new rmdir /s /q build\new
.venv\Scripts\pyinstaller --noconfirm --clean --windowed --log-level WARN --name Owlingstone ^
    --distpath build\new --workpath build\work --specpath build ^
    --icon "%~dp0bgtracker\icon.ico" ^
    --add-data "%~dp0bgtracker\icon.ico;bgtracker" ^
    --add-data "%~dp0data\*.yaml;data" ^
    --add-data "%~dp0CHANGELOG.md;." ^
    Owlingstone.pyw || goto failed
echo Replacing the old version...
taskkill /im Owlingstone.exe /f >nul 2>&1
rem Ancien nom de l'appli (BG Tracker) : on ferme et on range l'ancienne version.
taskkill /im BGTracker.exe /f >nul 2>&1
ping -n 3 127.0.0.1 >nul
if exist dist\Owlingstone rmdir /s /q dist\Owlingstone
if exist dist\Owlingstone goto locked
if exist dist\BGTracker rmdir /s /q dist\BGTracker
if exist "%USERPROFILE%\Desktop\BG Tracker.lnk" del "%USERPROFILE%\Desktop\BG Tracker.lnk"
if not exist dist mkdir dist
move build\new\Owlingstone dist\Owlingstone >nul || goto failed
rem Raccourci sur le Bureau
powershell -NoProfile -Command "$s=(New-Object -ComObject WScript.Shell).CreateShortcut([Environment]::GetFolderPath('Desktop')+'\Owlingstone.lnk'); $s.TargetPath='%~dp0dist\Owlingstone\Owlingstone.exe'; $s.WorkingDirectory='%~dp0dist\Owlingstone'; $s.Save()"
echo.
echo OK : Owlingstone is up to date. Starting it...
start "" "%~dp0dist\Owlingstone\Owlingstone.exe"
ping -n 5 127.0.0.1 >nul
exit /b 0

:locked
echo.
echo dist\Owlingstone is in use (is Owlingstone still open?). Close it and run update.bat again.
pause
exit /b 1

:failed
echo.
echo UPDATE FAILED - nothing was changed, the old version still works. See the messages above.
pause
exit /b 1
