@echo off
setlocal enabledelayedexpansion

echo ============================================
echo   Nuitka Build Script
echo ============================================
echo.

set /p PYFILE=Enter py file name (e.g. 9.py): 

set PYFILE=%PYFILE:"=%
for /f "tokens=* delims= " %%a in ("%PYFILE%") do set PYFILE=%%a

if "%PYFILE%"=="" (
    echo [ERROR] No file name entered.
    pause
    exit /b 1
)

if not exist "%PYFILE%" (
    echo [ERROR] File not found: %PYFILE%
    pause
    exit /b 1
)

for %%F in ("%PYFILE%") do set BASENAME=%%~nF

echo.
echo [INFO] Source : %PYFILE%
echo [INFO] Output : %BASENAME%.exe
echo.

where nuitka >nul 2>nul
if errorlevel 1 (
    echo [ERROR] nuitka not found. Run: pip install nuitka
    pause
    exit /b 1
)

echo [1/4] Cleaning old build files...
if exist "%BASENAME%.build" rmdir /s /q "%BASENAME%.build"
if exist "%BASENAME%.dist" rmdir /s /q "%BASENAME%.dist"
if exist "%BASENAME%.onefile-build" rmdir /s /q "%BASENAME%.onefile-build"
if exist "%BASENAME%.exe.dist" rmdir /s /q "%BASENAME%.exe.dist"
if exist "%BASENAME%.exe.onefile-build" rmdir /s /q "%BASENAME%.exe.onefile-build"

echo [2/4] Building with Nuitka...
python -m nuitka --onefile --windows-console-mode=disable --enable-plugin=tk-inter --windows-icon-from-ico=app.ico --include-data-file=app.ico=app.ico --zig "%PYFILE%"

if errorlevel 1 (
    echo.
    echo [ERROR] Build failed.
    pause
    exit /b 1
)

echo.
echo [3/4] Copying exe to current folder...
if exist "%BASENAME%.exe" (
    copy /y "%BASENAME%.exe" "%~dp0%BASENAME%.exe" >nul
    echo [OK] Copied: %BASENAME%.exe
) else (
    echo [WARN] %BASENAME%.exe not found
)

echo.
echo [3.5/4] Cleaning intermediate files...
if exist "%BASENAME%.build" rmdir /s /q "%BASENAME%.build"
if exist "%BASENAME%.dist" rmdir /s /q "%BASENAME%.dist"
if exist "%BASENAME%.onefile-build" rmdir /s /q "%BASENAME%.onefile-build"
if exist "%BASENAME%.exe.dist" rmdir /s /q "%BASENAME%.exe.dist"
if exist "%BASENAME%.exe.onefile-build" rmdir /s /q "%BASENAME%.exe.onefile-build"

echo.
echo [4/4] Done!
echo Output: %~dp0%BASENAME%.exe
echo.

pause
endlocal