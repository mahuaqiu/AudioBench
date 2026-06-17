//! 构建脚本：在编译时处理 visqol 二进制和模型文件嵌入

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();

    // 根据目标平台选择二进制文件
    #[cfg(target_os = "windows")]
    let bin_file = "bin/visqol.exe";
    #[cfg(not(target_os = "windows"))]
    let bin_file = "bin/visqol";

    let bin_path = Path::new(&manifest_dir).join(bin_file);
    let bin_dir = bin_path.parent().unwrap();

    // 确保 bin 目录存在
    if !bin_dir.exists() {
        let _ = fs::create_dir_all(bin_dir);
    }

    // 如果 visqol 二进制不存在，创建占位文件
    if !bin_path.exists() {
        println!("cargo:warning=visqol 二进制文件不存在: {:?}，创建占位文件", bin_path);
        let placeholder = b"AUDIOBENCH_PLACEHOLDER_VISQOL_BINARY";
        fs::write(&bin_path, placeholder).expect("无法创建占位文件");
    }

    // 计算 visqol 二进制 hash
    let bin_data = fs::read(&bin_path).expect("无法读取 visqol 二进制文件");
    let bin_hash = format!("{:016x}", bin_data.len());
    println!("cargo:rustc-env=VISQOL_BIN_HASH={}", bin_hash);
    println!("cargo:warning=visqol 二进制: {} (size: {} bytes)", bin_file, bin_data.len());

    // 确保模型目录存在
    let model_dir = Path::new(&manifest_dir).join("bin/model");
    if !model_dir.exists() {
        let _ = fs::create_dir_all(&model_dir);
    }

    // 处理音频模式 SVM 模型
    let audio_model_path = model_dir.join("libsvm_nu_svr_model.txt");
    if !audio_model_path.exists() {
        println!("cargo:warning=ViSQOL 音频模型文件不存在: {:?}，创建占位文件", audio_model_path);
        fs::write(&audio_model_path, b"PLACEHOLDER_AUDIO_MODEL").expect("无法创建占位文件");
    }
    let audio_model_data = fs::read(&audio_model_path).expect("无法读取音频模型文件");
    let audio_model_hash = format!("{:016x}", audio_model_data.len());
    println!("cargo:rustc-env=VISQOL_AUDIO_MODEL_HASH={}", audio_model_hash);
    println!("cargo:warning=ViSQOL 音频模型: {} bytes", audio_model_data.len());

    // 处理语音模式 TFLite 模型
    let speech_model_path = model_dir.join("lattice_tcditugenmeetpackhref_ls2_nl60_lr12_bs2048_learn.005_ep2400_train1_7_raw.tflite");
    if !speech_model_path.exists() {
        println!("cargo:warning=ViSQOL 语音模型文件不存在: {:?}，创建占位文件", speech_model_path);
        fs::write(&speech_model_path, b"PLACEHOLDER_SPEECH_MODEL").expect("无法创建占位文件");
    }
    let speech_model_data = fs::read(&speech_model_path).expect("无法读取语音模型文件");
    let speech_model_hash = format!("{:016x}", speech_model_data.len());
    println!("cargo:rustc-env=VISQOL_SPEECH_MODEL_HASH={}", speech_model_hash);
    println!("cargo:warning=ViSQOL 语音模型: {} bytes", speech_model_data.len());

    // 处理 DNSMOS ONNX 模型
    let dnsmos_model_path = model_dir.join("sig_bak_ovr.onnx");
    if !dnsmos_model_path.exists() {
        println!("cargo:warning=DNSMOS 模型文件不存在: {:?}，创建占位文件", dnsmos_model_path);
        fs::write(&dnsmos_model_path, b"PLACEHOLDER_DNSMOS_MODEL").expect("无法创建占位文件");
    }
    let dnsmos_model_data = fs::read(&dnsmos_model_path).expect("无法读取 DNSMOS 模型文件");
    let dnsmos_model_hash = format!("{:016x}", dnsmos_model_data.len());
    println!("cargo:rustc-env=DNSMOS_MODEL_HASH={}", dnsmos_model_hash);
    println!("cargo:warning=DNSMOS 模型: {} bytes", dnsmos_model_data.len());

    // 处理 DNSMOS ONNX Runtime DLL（仅 Windows）
    #[cfg(target_os = "windows")]
    {
        let dll_path = Path::new(&manifest_dir).join("bin/onnxruntime.dll");
        if dll_path.exists() {
            let dll_data = fs::read(&dll_path).expect("无法读取 ONNX Runtime DLL");
            let dll_hash = format!("{:016x}", dll_data.len());
            println!("cargo:rustc-env=ONNXRUNTIME_DLL_HASH={}", dll_hash);
            println!("cargo:warning=ONNX Runtime DLL: {} bytes", dll_data.len());
        }
        
        let providers_dll_path = Path::new(&manifest_dir).join("bin/onnxruntime_providers_shared.dll");
        if providers_dll_path.exists() {
            let providers_data = fs::read(&providers_dll_path).expect("无法读取 ONNX Runtime providers DLL");
            let providers_hash = format!("{:016x}", providers_data.len());
            println!("cargo:rustc-env=ONNXRUNTIME_PROVIDERS_DLL_HASH={}", providers_hash);
            println!("cargo:warning=ONNX Runtime providers DLL: {} bytes", providers_data.len());
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // 非 Windows 平台不需要处理 DLL
    }
}
