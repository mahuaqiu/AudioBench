# ViSQOL Windows 构建脚本
# 需要先安装 Bazel: https://bazel.build/install/windows

$ErrorActionPreference = "Stop"

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "ViSQOL Windows 构建脚本" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# 检查 Bazel 是否安装
Write-Host "[1/4] 检查 Bazel..." -ForegroundColor Yellow
try {
    $bazelVersion = bazel --version
    Write-Host "  ✓ Bazel 已安装: $bazelVersion" -ForegroundColor Green
} catch {
    Write-Host "  ✗ Bazel 未安装，请先安装: https://bazel.build/install/windows" -ForegroundColor Red
    exit 1
}

# 检查 TensorFlow 依赖（Windows）
Write-Host "[2/4] 检查 TensorFlow 依赖..." -ForegroundColor Yellow
Write-Host "  请参考: https://www.tensorflow.org/install/source_windows" -ForegroundColor Cyan
Write-Host "  按回车继续..." -ForegroundColor Yellow
Read-Host

# 构建 ViSQOL
Write-Host "[3/4] 构建 ViSQOL..." -ForegroundColor Yellow
$projectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = Split-Path -Parent $projectRoot

Set-Location $projectRoot

Write-Host "  执行: bazel build :visqol -c opt" -ForegroundColor Cyan
bazel build :visqol -c opt

if ($LASTEXITCODE -ne 0) {
    Write-Host "  ✗ 构建失败" -ForegroundColor Red
    exit 1
}

# 输出结果
Write-Host "[4/4] 构建完成!" -ForegroundColor Green
Write-Host ""
Write-Host "ViSQOL 可执行文件位置:" -ForegroundColor Cyan
Write-Host "  $projectRoot\bazel-bin\visqol.exe" -ForegroundColor White
Write-Host ""
Write-Host "建议将此文件复制到 audio_bench 项目目录，例如:" -ForegroundColor Cyan
Write-Host "  mkdir visqol" -ForegroundColor White
Write-Host "  copy bazel-bin\visqol.exe visqol\" -ForegroundColor White
