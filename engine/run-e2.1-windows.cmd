@echo off
setlocal
cd /d "%~dp0"
py -3 tools\verify_e2_1_platform.py --output reports\e2.1\platform-windows-amd64.json
if errorlevel 1 (
  echo.
  echo E2.1 Windows validation failed.
  exit /b 1
)
echo.
echo E2.1 Windows candidate report generated:
echo reports\e2.1\platform-windows-amd64.json
endlocal
