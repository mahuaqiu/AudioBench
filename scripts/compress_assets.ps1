# AudioBench 资源压缩脚本
# 使用 zstd 压缩 bin/ 目录下的所有嵌入资源

$ErrorActionPreference = "Stop"

$binDir = "bin"
$compressedDir = "bin_compressed"
$manifestFile = "compressed_manifest.txt"

# 创建压缩输出目录
if (Test-Path $compressedDir) {
    Remove-Item $compressedDir -Recurse -Force
}
New-Item -ItemType Directory -Path $compressedDir | Out-Null

# 清空清单文件
"" | Out-File -FilePath $manifestFile -Encoding utf8

Write-Host "开始压缩资源文件..." -ForegroundColor Cyan
Write-Host "输出目录: $compressedDir" -ForegroundColor Gray
Write-Host ""

$totalOriginal = 0
$totalCompressed = 0

# 压缩函数
function Compress-FileWithZstd {
    param(
        [string]$InputPath,
        [string]$OutputPath,
        [int]$CompressionLevel = 3  # 平衡压缩速度和体积
    )

    $originalSize = (Get-Item $InputPath).Length

    # 使用 Python zstd 模块（如果可用）或调用 zstd 命令行
    $pythonCmd = @"
import zstandard as zstd
import os

input_path = r'$InputPath'
output_path = r'$OutputPath'

with open(input_path, 'rb') as f_in:
    data = f_in.read()

cctx = zstd.ZstdCompressor(level=$CompressionLevel)
compressed = cctx.compress(data)

with open(output_path, 'wb') as f_out:
    f_out.write(compressed)

print(len(compressed))
"@

    # 尝试使用 Python zstd
    $pythonCmd | python - > "$OutputPath.size" 2>$null
    $compressedSize = [int](Get-Content "$OutputPath.size" -ErrorAction SilentlyContinue)

    if (-not $compressedSize -or $compressedSize -eq 0) {
        # 回退：直接复制文件（不压缩）
        Copy-Item $InputPath $OutputPath
        $compressedSize = $originalSize
    }

    Remove-Item "$OutputPath.size" -ErrorAction SilentlyContinue

    return @{
        OriginalSize = $originalSize
        CompressedSize = $compressedSize
    }
}

# 获取所有需要压缩的文件
$files = @(
    "bin/onnxruntime.dll",
    "bin/onnxruntime_providers_shared.dll",
    "bin/visqol.exe",
    "bin/visqol",
    "bin/model/sig_bak_ovr.onnx",
    "bin/model/libsvm_nu_svr_model.txt",
    "bin/model/lattice_tcditugenmeetpackhref_ls2_nl60_lr12_bs2048_learn.005_ep2400_train1_7_raw.tflite"
)

foreach ($file in $files) {
    if (Test-Path $file) {
        $fileName = Split-Path $file -Leaf
        $compressedName = "$fileName.zst"
        $outputPath = Join-Path $compressedDir $compressedName

        Write-Host "压缩: $file" -ForegroundColor Yellow
        $result = Compress-FileWithZstd -InputPath $file -OutputPath $outputPath

        $ratio = [math]::Round(($result.CompressedSize / $result.OriginalSize) * 100, 1)
        $origMB = [math]::Round($result.OriginalSize / 1MB, 2)
        $compMB = [math]::Round($result.CompressedSize / 1MB, 2)

        Write-Host "  原始: $origMB MB -> 压缩: $compMB MB ($ratio%)" -ForegroundColor Gray

        # 写入清单文件: filename original_size compressed_size
        "$fileName|$($result.OriginalSize)|$($result.CompressedSize)" | Add-Content -Path $manifestFile

        $totalOriginal += $result.OriginalSize
        $totalCompressed += $result.CompressedSize
    } else {
        Write-Host "跳过 (不存在): $file" -ForegroundColor DarkGray
    }
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "压缩完成!" -ForegroundColor Green
Write-Host "  原始总大小: $([math]::Round($totalOriginal / 1MB, 2)) MB" -ForegroundColor White
Write-Host "  压缩总大小: $([math]::Round($totalCompressed / 1MB, 2)) MB" -ForegroundColor White
$totalRatio = [math]::Round(($totalCompressed / $totalOriginal) * 100, 1)
Write-Host "  压缩率: $totalRatio%" -ForegroundColor White
Write-Host "========================================" -ForegroundColor Cyan