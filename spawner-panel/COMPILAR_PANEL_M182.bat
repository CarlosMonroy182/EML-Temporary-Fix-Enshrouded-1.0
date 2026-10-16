@echo off
chcp 65001 >nul
title M182 - Compilar panel Spawner
echo.
echo ===== M182: compilando el panel Spawner (la primera vez tarda varios minutos) =====
echo.
cd /d "C:\Users\morado\source\M182_Spawner_Panel"
if errorlevel 1 (
  echo No encuentro la carpeta C:\Users\morado\source\M182_Spawner_Panel
  pause
  exit /b 1
)
cargo build --release
if errorlevel 1 (
  echo.
  echo *** La compilacion fallo. Copia el texto rojo de arriba y mandamelo. ***
  pause
  exit /b 1
)
set "DEST=%USERPROFILE%\Downloads\M182_Panel"
if not exist "%DEST%" mkdir "%DEST%"
copy /Y "C:\Users\morado\source\M182_Spawner_Panel\target\release\M182_Spawner.exe" "%DEST%\M182_Spawner.exe"
if errorlevel 1 (
  echo.
  echo *** No pude copiar M182_Spawner.exe a Descargas. Mandame una captura de este mensaje. ***
  pause
  exit /b 1
)
echo.
echo ===== LISTO =====
echo El panel quedo en: %DEST%\M182_Spawner.exe
echo.
start "" explorer "%DEST%"
pause
