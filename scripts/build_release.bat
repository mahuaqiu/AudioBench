@echo off
chcp 65001 >nul
setlocal enabledelayedexpansion

echo ========================================
echo AudioBench 打包脚本
echo ========================================
echo.

REM 检查 Python 是否可用
python --version >nul 2>&1
if errorlevel 1 (
    echo [错误] 未找到 Python，请先安装 Python
    exit /b 1
)

REM 检查 zstandard 库
python -c "import zstandard" 2>nul
if errorlevel 1 (
    echo [信息] 正在安装 zstandard 库...
    pip install zstandard
    if errorlevel 1 (
        echo [错误] 安装 zstandard 失败
        exit /b 1
    )
)

REM 压缩资源文件
echo [*] 压缩资源文件...
python scripts/compress_assets.py
if errorlevel 1 (
    echo [错误] 资源压缩失败
    exit /b 1
)
echo.

REM 编译 release 版本
echo [*] 编译 release 版本...
cargo build --release
if errorlevel 1 (
    echo [错误] 编译失败
    exit /b 1
)
echo.

REM 获取 EXE 大小
for %%A in (target\release\audio_bench.exe) do set EXE_SIZE=%%~zA
set /a EXE_SIZE_MB=!EXE_SIZE! / 1048576

echo ========================================
echo 打包完成！
echo   EXE 路径: target\release\audio_bench.exe
echo   EXE 大小: !EXE_SIZE_MB! MB
echo ========================================

endlocal