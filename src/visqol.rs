//! ViSQOL 集成模块
//!
//! 通过 include_bytes! 在编译时嵌入 visqol 二进制文件和模型文件，
//! 运行时自动释放到临时目录并调用。
//! 不需要设置环境变量，单 EXE 即可运行。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;

// 编译时嵌入 visqol 二进制文件
// 用户需要将编译好的 visqol.exe 放到项目根目录的 bin/ 目录下
#[cfg(target_os = "windows")]
const VISQOL_BIN: &[u8] = include_bytes!("../bin/visqol.exe");

#[cfg(not(target_os = "windows"))]
const VISQOL_BIN: &[u8] = include_bytes!("../bin/visqol");

// 编译时嵌入 ViSQOL 模型文件
// 音频模式使用的 SVM 模型
const VISQOL_AUDIO_MODEL: &[u8] = include_bytes!("../bin/model/libsvm_nu_svr_model.txt");
// 语音模式使用的 TFLite 模型
const VISQOL_SPEECH_MODEL: &[u8] = include_bytes!("../bin/model/lattice_tcditugenmeetpackhref_ls2_nl60_lr12_bs2048_learn.005_ep2400_train1_7_raw.tflite");

/// 嵌入的 visqol 二进制的唯一标识（用于判断是否需要重新释放）
const VISQOL_BIN_HASH: &str = env!("VISQOL_BIN_HASH");
/// 嵌入的音频模型唯一标识
const VISQOL_AUDIO_MODEL_HASH: &str = env!("VISQOL_AUDIO_MODEL_HASH");
/// 嵌入的语音模型唯一标识
const VISQOL_SPEECH_MODEL_HASH: &str = env!("VISQOL_SPEECH_MODEL_HASH");

/// ViSQOL 调用结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct VisqolResult {
    pub moslqo: f64,
    pub vnsim: f64,
    pub fvnsim: Vec<f64>,
    pub fvnsim10: Vec<f64>,
    pub fstdnsim: Vec<f64>,
    pub fvdegenergy: Vec<f64>,
    pub center_freq_bands: Vec<f64>,
    pub patch_sims: Vec<PatchSimilarityResult>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PatchSimilarityResult {
    pub similarity: f64,
    pub ref_patch_start_time: f64,
    pub ref_patch_end_time: f64,
    pub deg_patch_start_time: f64,
    pub deg_patch_end_time: f64,
}

/// 释放嵌入文件到临时目录
///
/// 使用 hash 文件判断是否需要重新释放，避免每次运行都写磁盘
fn extract_file(data: &[u8], filename: &str, hash: &str, temp_dir: &Path) -> Result<PathBuf, String> {
    let file_path = temp_dir.join(filename);
    let hash_path = temp_dir.join(format!("{}.hash", filename));

    let need_extract = if file_path.exists() && hash_path.exists() {
        let existing_hash = fs::read_to_string(&hash_path).unwrap_or_default();
        existing_hash != hash
    } else {
        true
    };

    if need_extract {
        fs::write(&file_path, data)
            .map_err(|e| format!("释放文件 {} 失败: {}", filename, e))?;
        fs::write(&hash_path, hash)
            .map_err(|e| format!("写入 hash 文件失败: {}", e))?;
    }

    Ok(file_path)
}

/// 释放嵌入的 visqol 二进制到临时目录
fn extract_visqol() -> Result<PathBuf, String> {
    let temp_dir = std::env::temp_dir().join("audiobench");
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("创建临时目录失败: {}", e))?;

    #[cfg(target_os = "windows")]
    let exe_name = "visqol.exe";
    #[cfg(not(target_os = "windows"))]
    let exe_name = "visqol";

    let exe_path = extract_file(VISQOL_BIN, exe_name, VISQOL_BIN_HASH, &temp_dir)?;

    // Linux/macOS 需要设置可执行权限
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&exe_path)
            .map_err(|e| format!("获取文件权限失败: {}", e))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&exe_path, perms)
            .map_err(|e| format!("设置可执行权限失败: {}", e))?;
    }

    Ok(exe_path)
}

/// 释放 ViSQOL 模型文件到临时目录
fn extract_models() -> Result<(PathBuf, PathBuf), String> {
    let temp_dir = std::env::temp_dir().join("audiobench");
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("创建临时目录失败: {}", e))?;

    let audio_model_path = extract_file(
        VISQOL_AUDIO_MODEL,
        "libsvm_nu_svr_model.txt",
        VISQOL_AUDIO_MODEL_HASH,
        &temp_dir,
    )?;

    let speech_model_path = extract_file(
        VISQOL_SPEECH_MODEL,
        "lattice_tcditugenmeetpackhref_ls2_nl60_lr12_bs2048_learn.005_ep2400_train1_7_raw.tflite",
        VISQOL_SPEECH_MODEL_HASH,
        &temp_dir,
    )?;

    Ok((audio_model_path, speech_model_path))
}

/// ViSQOL 的运行模式
#[derive(Debug, Clone, Copy)]
pub enum VisqolMode {
    /// 语音模式：16kHz 采样率
    Speech,
    /// 音频模式：48kHz 采样率（默认）
    Audio,
}

/// 评估音频质量
///
/// 自动释放 visqol 二进制和模型文件，调用 visqol 进行评估
///
/// ref_path: 参考音频临时文件路径（已重采样到目标采样率）
/// deg_path: 录制音频临时文件路径（已重采样到目标采样率）
/// mode: ViSQOL 运行模式
pub fn evaluate_with_visqol(
    ref_path: &Path,
    deg_path: &Path,
    mode: VisqolMode,
) -> Result<VisqolResult, String> {
    // 释放 visqol 二进制
    let visqol_exe = extract_visqol()?;
    // 释放模型文件
    let (audio_model_path, speech_model_path) = extract_models()?;

    println!("[*] ViSQOL 二进制: {:?}", visqol_exe);
    println!("[*] ViSQOL 模式: {}", match mode {
        VisqolMode::Speech => "语音 (16kHz)",
        VisqolMode::Audio => "音频 (48kHz)",
    });

    // 根据模式选择模型文件
    let model_path = match mode {
        VisqolMode::Audio => &audio_model_path,
        VisqolMode::Speech => &speech_model_path,
    };
    println!("[*] ViSQOL 模型: {:?}", model_path);

    // 创建临时CSV/JSON文件用于输出结果
    let temp_dir = std::env::temp_dir().join("audiobench");
    let _ = fs::create_dir_all(&temp_dir);
    let temp_csv = temp_dir.join("visqol_result.csv");
    let temp_json = temp_dir.join("visqol_debug.json");

    // 删除可能存在的旧文件
    let _ = fs::remove_file(&temp_csv);
    let _ = fs::remove_file(&temp_json);

    // 构建 visqol 命令
    let mut cmd = Command::new(&visqol_exe);
    cmd.arg("--reference_file").arg(ref_path);
    cmd.arg("--degraded_file").arg(deg_path);
    cmd.arg("--results_csv").arg(&temp_csv);
    cmd.arg("--output_debug").arg(&temp_json);
    // 指定模型文件路径，避免 ViSQOL 在当前工作目录下查找
    cmd.arg("--similarity_to_quality_model").arg(model_path);

    if matches!(mode, VisqolMode::Speech) {
        cmd.arg("--use_speech_mode");
    }

    println!("[DEBUG] 执行: {:?}", cmd);

    // 执行命令
    let output = cmd.output()
        .map_err(|e| format!("启动 visqol 失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("visqol 执行失败:\n  stdout: {}\n  stderr: {}", stdout, stderr));
    }

    // 解析CSV输出
    let mut result = parse_csv_results(&temp_csv)?;

    // 尝试从JSON获取patch信息（可选，失败不影响主流程）
    result.patch_sims = parse_patch_from_json(&temp_json);

    // 清理临时文件
    let _ = fs::remove_file(&temp_csv);
    let _ = fs::remove_file(&temp_json);

    Ok(result)
}

/// 从CSV文件解析结果
fn parse_csv_results(csv_path: &Path) -> Result<VisqolResult, String> {
    let content = fs::read_to_string(csv_path)
        .map_err(|e| format!("读取结果文件失败: {}", e))?;

    let lines: Vec<&str> = content.lines().collect();
    if lines.len() < 2 {
        return Err(format!("CSV结果文件格式错误，内容:\n{}", content));
    }

    // 第二行是数据，第一行是表头
    let header: Vec<&str> = lines[0].split(',').collect();
    let values: Vec<&str> = lines[1].split(',').collect();

    if values.is_empty() {
        return Err("CSV结果为空".to_string());
    }

    // 格式: reference, degraded, moslqo, fvnsim0, fvnsim1, ..., fvnsim10_0, ..., fstdnsim0, ..., fvdegenergy0, ...
    let moslqo_idx = header.iter().position(|h| h.trim() == "moslqo")
        .ok_or_else(|| format!("找不到 moslqo 列，表头: {:?}", header))?;

    let moslqo: f64 = values[moslqo_idx].trim().parse()
        .map_err(|e| format!("解析 moslqo 失败: {} (值: {:?})", e, values.get(moslqo_idx)))?;

    // 收集所有频段指标
    let mut fvnsim = Vec::new();
    let mut fvnsim10 = Vec::new();
    let mut fstdnsim = Vec::new();
    let mut fvdegenergy = Vec::new();
    let center_freq_bands = Vec::new();

    for (i, h) in header.iter().enumerate() {
        let h = h.trim();
        if h.starts_with("fvnsim") && !h.contains("10") {
            if i < values.len() {
                if let Ok(v) = values[i].trim().parse::<f64>() {
                    fvnsim.push(v);
                }
            }
        } else if h.starts_with("fvnsim10") {
            if i < values.len() {
                if let Ok(v) = values[i].trim().parse::<f64>() {
                    fvnsim10.push(v);
                }
            }
        } else if h.starts_with("fstdnsim") {
            if i < values.len() {
                if let Ok(v) = values[i].trim().parse::<f64>() {
                    fstdnsim.push(v);
                }
            }
        } else if h.starts_with("fvdegenergy") {
            if i < values.len() {
                if let Ok(v) = values[i].trim().parse::<f64>() {
                    fvdegenergy.push(v);
                }
            }
        }
    }

    // VNSIM 是 fvnsim 的均值
    let vnsim = if !fvnsim.is_empty() {
        fvnsim.iter().sum::<f64>() / fvnsim.len() as f64
    } else {
        0.0
    };

    Ok(VisqolResult {
        moslqo,
        vnsim,
        fvnsim,
        fvnsim10,
        fstdnsim,
        fvdegenergy,
        center_freq_bands,
        patch_sims: vec![],
    })
}

/// 从JSON调试文件解析patch信息
fn parse_patch_from_json(json_path: &Path) -> Vec<PatchSimilarityResult> {
    let content = match fs::read_to_string(json_path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    // 打印JSON内容长度用于调试
    println!("[DEBUG] ViSQOL JSON 内容长度: {} bytes", content.len());
    
    // 解析JSON，尝试多个可能的字段名
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
        let field_names = ["patchSims", "patch_sims", "patch_similarities", "nsims", "similarities"];
        
        for field_name in field_names {
            if let Some(patches) = json.get(field_name).and_then(|p| p.as_array()) {
                println!("[DEBUG] 找到字段 '{}', patch数量: {}", field_name, patches.len());
                let mut results = Vec::new();
                for patch in patches {
                    if let Some(arr) = patch.as_array() {
                        if arr.len() >= 5 {
                            results.push(PatchSimilarityResult {
                                similarity: arr[0].as_f64().unwrap_or(0.0),
                                ref_patch_start_time: arr[1].as_f64().unwrap_or(0.0),
                                ref_patch_end_time: arr[2].as_f64().unwrap_or(0.0),
                                deg_patch_start_time: arr[3].as_f64().unwrap_or(0.0),
                                deg_patch_end_time: arr[4].as_f64().unwrap_or(0.0),
                            });
                        }
                    }
                }
                if !results.is_empty() {
                    return results;
                }
            }
        }
        
        // 打印所有顶层键用于调试
        if let Some(obj) = json.as_object() {
            println!("[DEBUG] JSON顶层键: {:?}", obj.keys().collect::<Vec<_>>());
        }
    }

    vec![]
}
