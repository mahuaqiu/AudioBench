//! 构建脚本：在编译时处理 visqol 二进制嵌入
//!
//! 如果 bin/visqol 不存在，创建一个小的占位文件

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // 根据目标平台选择二进制文件
    #[cfg(target_os = "windows")]
    let bin_file = "bin/visqol.exe";
    #[cfg(not(target_os = "windows"))]
    let bin_file = "bin/visqol";

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let bin_path = Path::new(&manifest_dir).join(bin_file);
    let bin_dir = bin_path.parent().unwrap();

    // 确保 bin 目录存在
    if !bin_dir.exists() {
        let _ = fs::create_dir_all(bin_dir);
    }

    // 如果文件不存在，创建一个小的占位文件
    if !bin_path.exists() {
        println!("cargo:warning=visqol 二进制文件不存在: {:?}，创建占位文件", bin_path);
        // 创建一个小的占位文件（至少有一些内容）
        let placeholder = b"AUDIOBENCH_PLACEHOLDER_VISQOL_BINARY";
        fs::write(&bin_path, placeholder).expect("无法创建占位文件");
    }

    // 读取二进制文件内容
    let data = fs::read(&bin_path).expect("无法读取 visqol 二进制文件");

    // 计算 hash（使用简单的哈希）
    let hash = format!("{:016x}", data.len());
    println!("cargo:rustc-env=VISQOL_BIN_HASH={}", hash);
    println!("cargo:warning=visqol 二进制: {} (size: {} bytes)", bin_file, data.len());
}
