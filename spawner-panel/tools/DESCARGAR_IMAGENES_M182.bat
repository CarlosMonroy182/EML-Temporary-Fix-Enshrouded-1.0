@echo off
chcp 65001 >nul
title M182 - Descargar imagenes de la wiki
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0DESCARGAR_IMAGENES_M182.ps1"
echo.
pause
