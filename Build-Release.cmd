@echo off
setlocal
cd /d "%~dp0"

powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-release.ps1"
set "build_exit_code=%ERRORLEVEL%"

echo.
if not "%build_exit_code%"=="0" (
    echo Release build failed. Review the message above.
) else (
    echo Release build completed successfully.
)
pause
exit /b %build_exit_code%
