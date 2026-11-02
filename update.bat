@echo off
rem Met a jour Owlingstone : teste le code du projet, reconstruit Owlingstone.exe, puis le relance.
rem Double-clic (fenetre), ou bouton "Update" de l'appli (argument quiet : sans fenetre, sortie dans %APPDATA%\Owlingstone\update.log, pas de pause). Mes donnees (%APPDATA%\Owlingstone, ex-BGTracker) ne sont jamais touchees.
rem Si un test ou la construction echoue, l'ancienne version reste en place et continue de marcher.
cd /d "%~dp0"
set QUIET=
set KILLED=
if /i "%~1"=="quiet" set QUIET=1
if not defined QUIET title Owlingstone update
echo === Owlingstone update ===
if not exist .venv\Scripts\python.exe py -3.12 -m venv .venv
rem Paquets : seulement quand requirements.txt a change (gagne du temps a chaque mise a jour).
fc /b requirements.txt .venv\requirements.done >nul 2>&1 || (.venv\Scripts\python -m pip install --quiet --disable-pip-version-check -r requirements.txt && copy /y requirements.txt .venv\requirements.done >nul) || goto failed
echo Running tests...
.venv\Scripts\python -m pytest -q -m "not slow" || goto failed
echo Building the new version...
if exist build\new rmdir /s /q build\new
rem Owlingstone.spec : la meme recette que les Releases, sans les parties de Qt inutiles.
rem Pas de --clean : PyInstaller reutilise ce qui n'a pas change (build\work), bien plus rapide.
.venv\Scripts\pyinstaller --noconfirm --log-level WARN --distpath build\new --workpath build\work Owlingstone.spec || goto failed
echo Replacing the old version...
taskkill /im Owlingstone.exe /f >nul 2>&1
set KILLED=1
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
rem Sans fenetre : on relance l'appli si elle a ete fermee, et elle dira que la mise a jour a echoue.
if defined QUIET if defined KILLED echo failed> "%APPDATA%\Owlingstone\update_failed.txt"
if defined QUIET if defined KILLED if exist "%~dp0dist\Owlingstone\Owlingstone.exe" start "" "%~dp0dist\Owlingstone\Owlingstone.exe"
if not defined QUIET pause
exit /b 1

:failed
echo.
echo UPDATE FAILED - nothing was changed, the old version still works. See the messages above.
rem Sans fenetre : on relance l'appli si elle a ete fermee, et elle dira que la mise a jour a echoue.
if defined QUIET if defined KILLED echo failed> "%APPDATA%\Owlingstone\update_failed.txt"
if defined QUIET if defined KILLED if exist "%~dp0dist\Owlingstone\Owlingstone.exe" start "" "%~dp0dist\Owlingstone\Owlingstone.exe"
if not defined QUIET pause
exit /b 1
