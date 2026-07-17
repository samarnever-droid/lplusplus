@echo off
setlocal

set "SCRIPT_DIR=%~dp0"
set "RELEASE_BIN=%SCRIPT_DIR%target\release\lpp.exe"

if exist "%RELEASE_BIN%" (
    "%RELEASE_BIN%" %*
    exit /b %ERRORLEVEL%
)

cargo run --manifest-path "%SCRIPT_DIR%Cargo.toml" --release -- %*
exit /b %ERRORLEVEL%
