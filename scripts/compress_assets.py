# AudioBench 资源压缩脚本
# 使用 Python zstandard 压缩 bin/ 目录下的所有嵌入资源

import os
import zstandard as zstd
import sys

def compress_file(input_path, output_path, level=3):
    """使用 zstd 压缩单个文件"""
    with open(input_path, 'rb') as f_in:
        data = f_in.read()

    original_size = len(data)

    # 压缩
    cctx = zstd.ZstdCompressor(level=level)
    compressed = cctx.compress(data)

    with open(output_path, 'wb') as f_out:
        f_out.write(compressed)

    compressed_size = len(compressed)
    return original_size, compressed_size

def main():
    bin_dir = "bin"
    compressed_dir = "bin_compressed"
    manifest_file = "compressed_manifest.txt"

    # 创建压缩输出目录
    if os.path.exists(compressed_dir):
        import shutil
        shutil.rmtree(compressed_dir)
    os.makedirs(compressed_dir)

    # 要压缩的文件列表
    files = [
        ("onnxruntime.dll", "bin/onnxruntime.dll"),
        ("onnxruntime_providers_shared.dll", "bin/onnxruntime_providers_shared.dll"),
        ("visqol.exe", "bin/visqol.exe"),
        ("visqol", "bin/visqol"),
        ("sig_bak_ovr.onnx", "bin/model/sig_bak_ovr.onnx"),
        ("libsvm_nu_svr_model.txt", "bin/model/libsvm_nu_svr_model.txt"),
        ("lattice_tcditugenmeetpackhref_ls2_nl60_lr12_bs2048_learn.005_ep2400_train1_7_raw.tflite", "bin/model/lattice_tcditugenmeetpackhref_ls2_nl60_lr12_bs2048_learn.005_ep2400_train1_7_raw.tflite"),
    ]

    total_original = 0
    total_compressed = 0

    print("开始压缩资源文件...")
    print(f"输出目录: {compressed_dir}")
    print()

    with open(manifest_file, 'w', encoding='utf-8') as mf:
        for name, path in files:
            if os.path.exists(path):
                output_path = os.path.join(compressed_dir, f"{name}.zst")

                print(f"压缩: {path}")
                orig_size, comp_size = compress_file(path, output_path, level=3)

                ratio = (comp_size / orig_size) * 100
                orig_mb = orig_size / (1024 * 1024)
                comp_mb = comp_size / (1024 * 1024)

                print(f"  原始: {orig_mb:.2f} MB -> 压缩: {comp_mb:.2f} MB ({ratio:.1f}%)")

                # 写入清单: filename|original_size|compressed_size
                mf.write(f"{name}|{orig_size}|{comp_size}\n")

                total_original += orig_size
                total_compressed += comp_size
            else:
                print(f"跳过 (不存在): {path}")

    print()
    print("=" * 40)
    print("压缩完成!")
    print(f"  原始总大小: {total_original / (1024*1024):.2f} MB")
    print(f"  压缩总大小: {total_compressed / (1024*1024):.2f} MB")
    print(f"  压缩率: {(total_compressed / total_original) * 100:.1f}%")
    print("=" * 40)

if __name__ == "__main__":
    main()