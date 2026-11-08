@echo off
setlocal
cd /d "%~dp0"

echo ==================================================
echo   MAIN BUILD   stable 1.98.x  /  Win10 - Win11
echo   For Win7 use: ..\frameanalyzer-win7\build_win7.bat
echo ==================================================
echo.

rem   build_main.bat             -> build exe (compile, verify, copy, remove target)
rem   build_main.bat --clean-only  -> remove target\ only (no compile)
if /i "%~1"=="--clean-only" (
    call :clean_only
    goto end
)

:build
echo [1/5] toolchain
cargo +stable --version
if errorlevel 1 goto fail

echo.
echo [2/5] building release
cargo +stable build --release
if not errorlevel 1 goto built_ok
echo   first attempt failed - retrying once (fresh target / AV file lock)
cargo +stable build --release
if errorlevel 1 goto fail
:built_ok
if not exist "target\release\frameanalyzer.exe" goto fail

echo.
echo [3/5] verifying build artifact (PE import table)
python verify_release.py target\release\frameanalyzer.exe --expect main
if errorlevel 1 goto fail

echo.
echo [4/5] copying exe into this folder
copy /y "target\release\frameanalyzer.exe" "frameanalyzer.exe" >nul
if errorlevel 1 goto fail
if not exist "frameanalyzer.exe" goto fail
fc /b "target\release\frameanalyzer.exe" "frameanalyzer.exe" >nul
if errorlevel 1 goto fail
python verify_release.py frameanalyzer.exe --expect main
if errorlevel 1 goto fail
echo     copied and verified: %~dp0frameanalyzer.exe

echo.
echo [5/5] removing target\ (build cache only; recreated on next build)
rmdir /s /q target 2>nul
if exist "target" (
    echo   WARNING: target\ not fully removed - a file may be locked.
    echo            The exe is ready. Delete target\ manually when possible.
) else (
    echo   target\ removed.
)

echo.
echo DONE: %~dp0frameanalyzer.exe
echo NOTE: this build CANNOT run on Win7 (0xc0000005).
echo NOTE: next build starts from scratch and takes 2-3 minutes.
echo.
if "%~1"=="" pause
goto end

:fail
echo.
echo BUILD FAILED - target\ kept so the previous exe is not lost.
if "%~1"=="" pause
exit /b 1

:clean_only
echo [CLEAN] Removing build leftovers (target\)...
if not exist "target" (
    echo   [OK] Nothing to clean - target\ does not exist.
    goto :eof
)
rmdir /s /q target 2>nul
if exist "target" (
    echo   [WARN] target\ not fully removed - a file may be locked.
) else (
    echo   [OK] target\ removed.
)
echo   [NOTE] exe and .config.json are kept untouched.
goto :eof

:end
endlocal
