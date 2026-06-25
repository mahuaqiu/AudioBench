//! 资源解压模块
//!
//! 使用 zstd 解压编译时嵌入的压缩资源
//! 提供统一的解压接口给 dnsmos.rs 和 visqol.rs 使用

use std::fs;
use std::path::PathBuf;

/// 使用 zstd 解压数据
///
/// # Arguments
/// * `compressed_data` - 压缩后的字节数据
///
/// # Returns
/// 解压后的原始字节数据
pub fn decompress_zstd(compressed_data: &[u8]) -> Result<Vec<u8>, String> {
    let mut decompressor = zstd::stream::Decoder::new(compressed_data)
        .map_err(|e| format!("zstd 解压初始化失败: {}", e))?;

    let mut decompressed = Vec::new();
    use std::io::Read;
    decompressor.read_to_end(&mut decompressed)
        .map_err(|e| format!("zstd 解压失败: {}", e))?;

    Ok(decompressed)
}

/// 释放嵌入的压缩文件到临时目录
///
/// 如果文件已存在且哈希匹配，则跳过解压
///
/// # Arguments
/// * `compressed_data` - 编译时嵌入的压缩数据
/// * `original_size` - 原始文件大小（用于哈希验证）
/// * `filename` - 目标文件名
/// * `temp_dir` - 临时目录路径
/// * `hash` - 用于验证的文件哈希（通常是原始大小）
///
/// # Returns
/// 解压后文件的完整路径
pub fn extract_compressed_file(
    compressed_data: &[u8],
    original_size: usize,
    filename: &str,
    temp_dir: &PathBuf,
    hash: &str,
) -> Result<PathBuf, String> {
    let file_path = temp_dir.join(filename);
    let hash_path = temp_dir.join(format!("{}.hash", filename));

    // 检查是否需要解压：文件不存在或哈希不匹配
    let need_decompress = if file_path.exists() && hash_path.exists() {
        let existing_hash = fs::read_to_string(&hash_path).unwrap_or_default();
        existing_hash != hash
    } else {
        true
    };

    if need_decompress {
        // 确保目录存在
        if !temp_dir.exists() {
            fs::create_dir_all(temp_dir)
                .map_err(|e| format!("创建临时目录失败: {}", e))?;
        }

        // 解压数据
        let decompressed = decompress_zstd(compressed_data)?;

        // 验证解压后的大小
        if decompressed.len() != original_size {
            return Err(format!(
                "解压后大小不匹配: 期望 {} 字节, 实际 {} 字节",
                original_size,
                decompressed.len()
            ));
        }

        // 写入文件
        fs::write(&file_path, &decompressed)
            .map_err(|e| format!("写入文件失败: {}", e))?;

        // 写入哈希文件
        fs::write(&hash_path, hash)
            .map_err(|e| format!("写入哈希文件失败: {}", e))?;

        eprintln!("[+] 解压文件: {} ({} bytes)", filename, decompressed.len());
    }

    Ok(file_path)
}

/// 快速解压（不验证大小）
pub fn decompress_fast(compressed_data: &[u8]) -> Result<Vec<u8>, String> {
    let mut decompressor = zstd::stream::Decoder::new(compressed_data)
        .map_err(|e| format!("zstd 解压初始化失败: {}", e))?;

    let mut decompressed = Vec::new();
    use std::io::Read;
    decompressor.read_to_end(&mut decompressed)
        .map_err(|e| format!("zstd 解压失败: {}", e))?;

    Ok(decompressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zstd_roundtrip() {
        let original = b"Hello, AudioBench! This is a test message for zstd compression.";

        // 压缩
        let compressed = zstd::encode_all(original, 3).unwrap();

        // 解压
        let decompressed = decompress_zstd(&compressed).unwrap();

        assert_eq!(original.as_slice(), decompressed.as_slice());
    }
}