//! ViSQOL 集成模块
//! 
//! 通过子进程调用官方 visqol 二进制进行音频质量评估
//! 使用 --results_csv 解析 MOS-LQO 等指标

use std::path::Path;
use std::process::Command;
use std::fs;

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

/// 评估音频质量
/// 
/// 使用 visqol 命令行工具进行评估
/// visqol_path: visqol 二进制路径
/// ref_path: 参考音频路径 (WAV, 16kHz speech模式 或 48kHz audio模式)
/// deg_path: 录制音频路径
/// use_speech_mode: 是否使用语音模式 (16kHz)
pub fn evaluate_with_visqol(
    visqol_path: &Path,
    ref_path: &Path,
    deg_path: &Path,
    use_speech_mode: bool,
) -> Result<VisqolResult, String> {
    // 创建临时CSV文件用于输出结果
    let temp_csv = std::env::temp_dir().join("visqol_result.csv");
    let temp_json = std::env::temp_dir().join("visqol_debug.json");
    
    // 删除可能存在的旧文件
    let _ = fs::remove_file(&temp_csv);
    let _ = fs::remove_file(&temp_json);
    
    // 构建 visqol 命令
    let mut cmd = Command::new(visqol_path);
    cmd.arg("--reference_file").arg(ref_path);
    cmd.arg("--degraded_file").arg(deg_path);
    cmd.arg("--results_csv").arg(&temp_csv);
    cmd.arg("--output_debug").arg(&temp_json);
    
    if use_speech_mode {
        cmd.arg("--use_speech_mode");
    }
    
    // 执行命令
    let output = cmd.output()
        .map_err(|e| format!("启动 visqol 失败: {}", e))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("visqol 执行失败: {}", stderr));
    }
    
    // 解析CSV输出
    parse_csv_results(&temp_csv)
}

/// 从CSV文件解析结果
fn parse_csv_results(csv_path: &Path) -> Result<VisqolResult, String> {
    let content = fs::read_to_string(csv_path)
        .map_err(|e| format!("读取结果文件失败: {}", e))?;
    
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() < 2 {
        return Err("CSV结果文件格式错误".to_string());
    }
    
    // 第二行是数据，第一行是表头
    let header: Vec<&str> = lines[0].split(',').collect();
    let values: Vec<&str> = lines[1].split(',').collect();
    
    if values.is_empty() {
        return Err("CSV结果为空".to_string());
    }
    
    // 格式: reference, degraded, moslqo, fvnsim0, fvnsim1, ..., fvnsim10_0, ..., fstdnsim0, ..., fvdegenergy0, ...
    let moslqo_idx = header.iter().position(|h| *h == "moslqo")
        .ok_or("找不到 moslqo 列")?;
    
    let moslqo: f64 = values[moslqo_idx].trim().parse()
        .map_err(|e| format!("解析 moslqo 失败: {}", e))?;
    
    // 收集所有 fvnsim 值
    let mut fvnsim = Vec::new();
    let mut fvnsim10 = Vec::new();
    let mut fstdnsim = Vec::new();
    let mut fvdegenergy = Vec::new();
    let center_freq_bands = Vec::new();
    
    for (i, h) in header.iter().enumerate() {
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
    
    // 尝试从JSON获取patch信息
    let patch_sims = parse_patch_from_json(&std::env::temp_dir().join("visqol_debug.json"));
    
    // 清理临时文件
    let _ = fs::remove_file(csv_path);
    let _ = fs::remove_file(std::env::temp_dir().join("visqol_debug.json"));
    
    Ok(VisqolResult {
        moslqo,
        vnsim,
        fvnsim,
        fvnsim10,
        fstdnsim,
        fvdegenergy,
        center_freq_bands,
        patch_sims,
    })
}

/// 从JSON调试文件解析patch信息
fn parse_patch_from_json(json_path: &Path) -> Vec<PatchSimilarityResult> {
    let content = match fs::read_to_string(json_path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    
    // 简单的JSON解析，提取patch_sims数组
    let patches = Vec::new();
    
    // 查找 patch_sims 部分
    if let Some(start) = content.find("\"patch_sims\"") {
        let _ = start;
    }
    
    patches
}

/// 获取visqol二进制路径
/// 
/// 优先级:
/// 1. 用户提供的路径
/// 2. 尝试从环境变量 VISQOL_PATH 获取
/// 3. 使用默认路径
pub fn get_visqol_path(user_path: Option<&Path>) -> Result<std::path::PathBuf, String> {
    if let Some(p) = user_path {
        if p.exists() {
            return Ok(p.to_path_buf());
        } else {
            return Err(format!("visqol 不存在: {:?}", p));
        }
    }
    
    // 尝试环境变量
    if let Ok(p) = std::env::var("VISQOL_PATH") {
        let path = Path::new(&p);
        if path.exists() {
            return Ok(path.to_path_buf());
        }
    }
    
    // 默认路径
    let default_path = Path::new("visqol");
    if default_path.exists() {
        return Ok(default_path.to_path_buf());
    }
    
    Err("找不到 visqol 二进制，请设置 VISQOL_PATH 环境变量或提供路径".to_string())
}
