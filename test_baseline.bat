@echo off
setlocal
cd /d "%~dp0"

echo ============================================
echo   Rust Test Baseline  (one-click)
echo ============================================
echo.
echo Runs: cargo +stable test --lib  (MAIN 1.98)
echo Pass any verify_tests.py args, e.g.:
echo   test_baseline.bat --all
echo   test_baseline.bat --keep-target
echo   test_baseline.bat --expect-passed 53
echo.
echo NOTE: Do NOT run this at the same time as build_main.bat
echo       or build_win7.bat (two full builds will hang).
echo.

python verify_tests.py %*
if errorlevel 1 goto fail

echo.
echo BASELINE PASS - all tests green.
echo.
pause
endlocal
exit /b 0

:fail
echo.
echo BASELINE FAILED - target kept for diagnosis.
echo.
pause
endlocal
exit /b 1
