@echo off
setlocal
rem Run the repo combat-sim binary. The path is resolved from this wrapper,
rem so the caller's working directory does not matter.
"%~dp0..\..\tools\bin\combat-sim.exe" %*
exit /b %ERRORLEVEL%
