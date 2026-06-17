//! DNSMOS 指标计算模块
//!
//! 使用 ONNX 模型计算 DNSMOS 分数（SIG/BAK/OVRL）
//! 遵循 ITU-T P.835 标准，分数范围 1.0-5.0
//!
//! 技术细节：
//! - 输入：任意采样率的音频，内部重采样到 16kHz
//! - 模型输入：144160 个采样点（9.01s @ 16kHz）
//! - 长音频（≥9.01s）：滑动窗口（窗口 9.01s，步进 1s）
//! - 短音频（<9.01s）：末尾零填充至 144160 点
//! - 输出：多项式校准后的 SIG/BAK/OVRL 分数

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use ndarray::Array2;

/// DNSMOS ONNX 模型（编译时嵌入）
const DNSMOS_MODEL: &[u8] = include_bytes!("../bin/model/sig_bak_ovr.onnx");

/// ONNX Runtime DLL（编译时嵌入，运行时释放到临时目录）
const ONNXRUNTIME_DLL: &[u8] = include_bytes!("../bin/onnxruntime.dll");
const ONNXRUNTIME_PROVIDERS_DLL: &[u8] = include_bytes!("../bin/onnxruntime_providers_shared.dll");

/// 模型 hash（用于缓存验证）
const DNSMOS_MODEL_HASH: &str = env!("DNSMOS_MODEL_HASH");

// ============================================================================
// 多项式校准系数（来自 Microsoft DNSMOS 官方 Python 源码）
// 公式: result = a * x² + b * x + c
// ============================================================================

/// SIG（人声信号分）多项式系数
const POLY_SIG: (f64, f64, f64) = (-0.08397278, 1.22083953, 0.0052439);
/// BAK（背景噪声分）多项式系数
const POLY_BAK: (f64, f64, f64) = (-0.13166888, 1.60915514, -0.39604546);
/// OVRL（整体综合分）多项式系数
const POLY_OVRL: (f64, f64, f64) = (-0.06766283, 1.11546468, 0.04602535);

/// DNSMOS 模型参数
const MODEL_SAMPLE_RATE: u32 = 16000;
const MODEL_INPUT_LENGTH: usize = 144160; // 9.01s @ 16kHz
const WINDOW_STEP: usize = 16000; // 1s 步进

/// 获取临时目录路径（包含 DLL）
fn get_dll_dir() -> PathBuf {
    let temp_dir = std::env::temp_dir().join("audiobench_dnsmos");
    if !temp_dir.exists() {
        fs::create_dir_all(&temp_dir).expect("无法创建 DNSMOS 临时目录");
    }
    temp_dir
}

/// 初始化 DLL（首次调用时释放到临时目录��
static INIT_DLL: std::sync::Once = std::sync::Once::new();

fn init_dlls() {
    INIT_DLL.call_once(|| {
        let dll_dir = get_dll_dir();
        
        // 释放主 DLL
        let dll_path = dll_dir.join("onnxruntime.dll");
        if !dll_path.exists() {
            fs::write(&dll_path, ONNXRUNTIME_DLL).expect("无法释放 onnxruntime.dll");
        }
        
        // 释放 providers DLL
        let providers_path = dll_dir.join("onnxruntime_providers_shared.dll");
        if !providers_path.exists() {
            fs::write(&providers_path, ONNXRUNTIME_PROVIDERS_DLL).expect("无法释放 onnxruntime_providers_shared.dll");
        }
        
        // 将临时目录添加到 DLL 搜索路径（Windows）
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::ffi::OsStrExt;
            let path_str = dll_dir.to_string_lossy().to_owned();
            let path_wide: Vec<u16> = std::ffi::OsStr::new(&*path_str)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            unsafe {
                libc::SetDllDirectoryW(path_wide.as_ptr());
            }
        }
        
        println!("[+] DNSMOS DLL 已释放到: {:?}", dll_dir);
    });
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 多项式校准: a*x² + b*x + c
#[inline]
fn polyfit(x: f64, (a, b, c): (f64, f64, f64)) -> f64 {
    a * x * x + b * x + c
}

/// 将值裁剪到 [min, max] 范围，处理 NaN 和无穷值
#[inline]
fn clamp(value: f64, min_val: f64, max_val: f64) -> f64 {
    // 处理 NaN 和无穷值，返回默认值 3.0（中间分）
    if value.is_nan() || value.is_infinite() {
        return 3.0;
    }
    value.max(min_val).min(max_val)
}

/// 简单的线性插值重采样
fn resample(samples: &[f64], from_rate: u32, to_rate: u32) -> Vec<f64> {
    if from_rate == to_rate {
        return samples.to_vec();
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let new_len = ((samples.len() as f64) * ratio) as usize;
    let mut output = Vec::with_capacity(new_len);

    for i in 0..new_len {
        let src_pos = i as f64 / ratio;
        let idx = src_pos as usize;
        let frac = src_pos.fract();

        if idx + 1 < samples.len() {
            let val = samples[idx] * (1.0 - frac) + samples[idx + 1] * frac;
            output.push(val);
        } else if idx < samples.len() {
            output.push(samples[idx]);
        } else {
            output.push(0.0);
        }
    }

    output
}

/// 重采样到 16kHz
fn resample_to_16kHz(samples: &[f64], sample_rate: u32) -> Vec<f64> {
    resample(samples, sample_rate, MODEL_SAMPLE_RATE)
}

/// 滑窗切分音频为标准窗口
/// - 长音频（≥144160）：滑动窗口，步进 16000
/// - 短音频（<144160）：末尾零填充
fn segment_audio(samples_16k: &[f64]) -> Vec<Vec<f64>> {
    let len = samples_16k.len();

    if len >= MODEL_INPUT_LENGTH {
        // 长音频：滑动窗口
        let mut windows = Vec::new();
        let mut pos = 0;
        while pos + MODEL_INPUT_LENGTH <= len {
            windows.push(samples_16k[pos..pos + MODEL_INPUT_LENGTH].to_vec());
            pos += WINDOW_STEP;
        }
        // 处理最后一个不完整窗口（如果有的话，实际取最后 144160 点）
        if pos < len {
            let remaining = &samples_16k[len - MODEL_INPUT_LENGTH..len];
            windows.push(remaining.to_vec());
        }
        windows
    } else {
        // 短音频：零填充到固定长度
        let mut padded = samples_16k.to_vec();
        padded.resize(MODEL_INPUT_LENGTH, 0.0);
        vec![padded]
    }
}

// ============================================================================
// 公开 API
// ============================================================================

/// DNSMOS 评估��果
#[derive(Debug, Clone, serde::Serialize)]
pub struct DnsMosResult {
    /// 人声信号分 (1.0-5.0)
    pub sig: f64,
    /// 背景噪声分 (1.0-5.0)
    pub bak: f64,
    /// 整体综合分 (1.0-5.0)
    pub ovrl: f64,
}

impl DnsMosResult {
    /// 从原始输出创建结果，并进行多项式校准
    fn from_raw(sig_raw: f64, bak_raw: f64, ovrl_raw: f64) -> Self {
        let sig = clamp(polyfit(sig_raw, POLY_SIG), 1.0, 5.0);
        let bak = clamp(polyfit(bak_raw, POLY_BAK), 1.0, 5.0);
        let ovrl = clamp(polyfit(ovrl_raw, POLY_OVRL), 1.0, 5.0);
        Self { sig, bak, ovrl }
    }
}

/// DNSMOS 评估器
pub struct DnsMosEvaluator {
    session: ort::Session,
}

impl DnsMosEvaluator {
    /// 从嵌入的模型字节创建评估器
    pub fn new(model_bytes: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        // 初始化 DLL（释放到临时目录并添加搜索路径）
        init_dlls();
        
        let session = ort::Session::builder()?
            .commit_from_memory(model_bytes)?;
        Ok(Self { session })
    }

    /// 对一段音频进行评分（支持任意采样率，内部自动重采样到 16kHz）
    pub fn evaluate(&self, samples: &[f64], sample_rate: u32) -> Result<DnsMosResult, Box<dyn Error + Send + Sync>> {
        // 1. 重采样到 16kHz
        let samples_16k = resample_to_16kHz(samples, sample_rate);

        // 2. 滑窗切分
        let windows = segment_audio(&samples_16k);

        // 3. 逐窗推理 + 多项式校准
        let mut sig_sum = 0.0;
        let mut bak_sum = 0.0;
        let mut ovrl_sum = 0.0;

        for window in &windows {
            // 转换为 float32 数组并创建 ndarray 输入
            let input_data: Vec<f32> = window.iter().map(|&x| x as f32).collect();
            let input_array = Array2::from_shape_vec((1, MODEL_INPUT_LENGTH), input_data)
                .map_err(|e| format!("创建输入数组失败: {}", e))?;

            // 运行推理 - ort 2.0 语法：先转换为 Value，再用 => 连接
            let input_value = ort::value::Value::from_array(&input_array)
                .map_err(|e| format!("创建输入 Value 失败: {}", e))?;
            let outputs = self.session.run(ort::inputs![
                "input_1" => input_value
            ])?;

            // 解析输出 [1, 3] -> [SIG_raw, BAK_raw, OVRL_raw]
            let output_tensor = outputs["Identity:0"].as_tensor()?;
            let output_values = output_tensor.view();

            if output_values.len() != 3 {
                return Err(format!("DNSMOS 模型输出维度错误: 期望 3, 实际 {}", output_values.len()).into());
            }

            let sig_raw = output_values[[0, 0]] as f64;
            let bak_raw = output_values[[0, 1]] as f64;
            let ovrl_raw = output_values[[0, 2]] as f64;

            let result = DnsMosResult::from_raw(sig_raw, bak_raw, ovrl_raw);
            sig_sum += result.sig;
            bak_sum += result.bak;
            ovrl_sum += result.ovrl;
        }

        // 4. 计算均值
        let n = windows.len() as f64;
        Ok(DnsMosResult {
            sig: sig_sum / n,
            bak: bak_sum / n,
            ovrl: ovrl_sum / n,
        })
    }
}

/// 便捷函数：直接评估音频文件
pub fn evaluate_audio(samples: &[f64], sample_rate: u32) -> Result<DnsMosResult, Box<dyn Error + Send + Sync>> {
    let evaluator = DnsMosEvaluator::new(DNSMOS_MODEL)?;
    evaluator.evaluate(samples, sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_polyfit() {
        // 测试多项式计算
        let x = 3.0;
        let sig = polyfit(x, POLY_SIG);
        let expected = -0.08397278 * 9.0 + 1.22083953 * 3.0 + 0.0052439;
        assert!((sig - expected).abs() < 1e-6);
    }

    #[test]
    fn test_clamp() {
        assert!((clamp(0.5, 1.0, 5.0) - 1.0).abs() < 1e-6);
        assert!((clamp(6.0, 1.0, 5.0) - 5.0).abs() < 1e-6);
        assert!((clamp(3.0, 1.0, 5.0) - 3.0).abs() < 1e-6);
        // NaN 处理
        assert!((clamp(f64::NAN, 1.0, 5.0) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_resample() {
        let samples = vec![0.0, 1.0, 2.0, 3.0, 4.0];
        let resampled = resample(&samples, 1000, 2000); // 2x 上采样
        assert_eq!(resampled.len(), 10);
    }

    #[test]
    fn test_segment_long_audio() {
        // 创建 20s @ 16kHz 的测试音频
        let samples: Vec<f64> = (0..320000).map(|i| (i as f64 * 0.01).sin()).collect();
        let windows = segment_audio(&samples);
        // 20s = 320000 采样，窗口 144160，步进 16000
        // 窗口数 = (320000 - 144160) / 16000 + 1 = 11
        assert!(windows.len() >= 10);
        for w in &windows {
            assert_eq!(w.len(), MODEL_INPUT_LENGTH);
        }
    }

    #[test]
    fn test_segment_short_audio() {
        // 创建 3s @ 16kHz 的测试音频（短于 9.01s）
        let samples: Vec<f64> = (0..48000).map(|i| (i as f64 * 0.01).sin()).collect();
        let windows = segment_audio(&samples);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].len(), MODEL_INPUT_LENGTH);
        // 验证末尾填充的是零
        assert!(windows[0][48000..].iter().all(|&x| x == 0.0));
    }
}
