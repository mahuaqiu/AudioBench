//! 构建脚本：处理压缩资源文件的嵌入
//!
//! 计算压缩文件的哈希（基于原始大小），用于运行时验证

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();

    // 压缩文件目录
    let compressed_dir = Path::new(&manifest_dir).join("bin_compressed");

    // 确保目录存在
    if !compressed_dir.exists() {
        println!("cargo:warning=压缩资源目录不存在: {:?}，请先运行 compress_assets.py", compressed_dir);
    }

    // 处理压缩后的 visqol 二进制
    #[cfg(target_os = "windows")]
    {
        process_compressed_file(&compressed_dir, "visqol.exe.zst", "VISQOL_BIN_HASH");
    }
    #[cfg(not(target_os = "windows"))]
    {
        process_compressed_file(&compressed_dir, "visqol.zst", "VISQOL_BIN_HASH");
    }

    // 处理压缩后的 ViSQOL 模型文件
    process_compressed_file(&compressed_dir, "libsvm_nu_svr_model.txt.zst", "VISQOL_AUDIO_MODEL_HASH");
    process_compressed_file(&compressed_dir, "lattice_tcditugenmeetpackhref_ls2_nl60_lr12_bs2048_learn.005_ep2400_train1_7_raw.tflite.zst", "VISQOL_SPEECH_MODEL_HASH");

    // 处理压缩后的 DNSMOS 模型
    process_compressed_file(&compressed_dir, "sig_bak_ovr.onnx.zst", "DNSMOS_MODEL_HASH");

    // 处理压缩后的 ONNX Runtime DLL（仅 Windows）
    #[cfg(target_os = "windows")]
    {
        process_compressed_file(&compressed_dir, "onnxruntime.dll.zst", "ONNXRUNTIME_DLL_HASH");
        process_compressed_file(&compressed_dir, "onnxruntime_providers_shared.dll.zst", "ONNXRUNTIME_PROVIDERS_DLL_HASH");
    }

    println!("cargo:warning=资源哈希生成完成");
}

/// 处理压缩文件，读取原始大小并生成哈希
fn process_compressed_file(compressed_dir: &Path, filename: &str, hash_env: &str) {
    let compressed_path = compressed_dir.join(filename);

    if compressed_path.exists() {
        // 读取压缩文件
        let compressed_data = fs::read(&compressed_path)
            .expect(&format!("无法读取压缩文件: {:?}", compressed_path));

        // 读取原始大小（从压缩清单文件）
        let original_size = get_original_size(filename);

        // 使用原始大小作为哈希（简单有效）
        let hash = format!("{:016x}", original_size);
        println!("cargo:rustc-env={}={}", hash_env, hash);
        println!("cargo:warning={}: {} bytes (compressed: {} bytes)", filename, original_size, compressed_data.len());
    } else {
        println!("cargo:warning=压缩文件不存在: {:?}，将跳过", compressed_path);
    }
}

/// 从清单文件获取原始文件大小
fn get_original_size(filename: &str) -> usize {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let manifest_path = Path::new(&manifest_dir).join("compressed_manifest.txt");

    if manifest_path.exists() {
        if let Ok(content) = fs::read_to_string(&manifest_path) {
            // 格式: filename|original_size|compressed_size
            for line in content.lines() {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 2 {
                    let name = parts[0];
                    if name == filename.replace(".zst", "") {
                        if let Ok(size) = parts[1].parse::<usize>() {
                            return size;
                        }
                    }
                }
            }
        }
    }

    // 默认值（如果清单文件不存在）
    0
}