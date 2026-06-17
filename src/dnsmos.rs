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
//!
//! 部署方案 - 仅 Windows：
//! - ONNX Runtime DLL 编译时嵌入 EXE，运行时释放到临时目录，单 EXE 部署

use std::error::Error;

// ============================================================================
// Windows 完整实现
// ============================================================================

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::error::Error;
    use std::fs;
    use std::path::PathBuf;

    /// ONNX Runtime DLL（编译时嵌入，运行时释放到临时目录）
    pub const ONNXRUNTIME_DLL: &[u8] = include_bytes!("../bin/onnxruntime.dll");
    pub const ONNXRUNTIME_PROVIDERS_DLL: &[u8] = include_bytes!("../bin/onnxruntime_providers_shared.dll");

    /// 获取临时目录路径
    pub fn get_dll_dir() -> PathBuf {
        std::env::temp_dir().join("audiobench_dnsmos")
    }

    /// 初始化 DLL（首次调用时释放到临时目录）
    pub fn init_dlls() {
        use std::sync::Once;
        static INIT: Once = Once::new();

        INIT.call_once(|| {
            let dll_dir = get_dll_dir();
            if !dll_dir.exists() {
                let _ = fs::create_dir_all(&dll_dir);
            }

            let dll_path = dll_dir.join("onnxruntime.dll");
            if !dll_path.exists() {
                let _ = fs::write(&dll_path, ONNXRUNTIME_DLL);
            }

            let providers_path = dll_dir.join("onnxruntime_providers_shared.dll");
            if !providers_path.exists() {
                let _ = fs::write(&providers_path, ONNXRUNTIME_PROVIDERS_DLL);
            }

            // 设置 DLL 搜索路径
            {
                use std::os::windows::ffi::OsStrExt;
                let path_str = dll_dir.to_string_lossy().to_owned();
                let path_wide: Vec<u16> = std::ffi::OsStr::new(&*path_str)
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                unsafe {
                    windows::Win32::System::LibraryLoader::SetDllDirectoryW(path_wide.as_ptr());
                }
            }

            eprintln!("[+] DNSMOS DLL 已释放到: {:?}", dll_dir);
        });
    }
}

// ============================================================================
// 多项式校准系数（来自 Microsoft DNSMOS 官方 Python 源码）
// ============================================================================

const POLY_SIG: (f64, f64, f64) = (-0.08397278, 1.22083953, 0.0052439);
const POLY_BAK: (f64, f64, f64) = (-0.13166888, 1.60915514, -0.39604546);
const POLY_OVRL: (f64, f64, f64) = (-0.06766283, 1.11546468, 0.04602535);

const MODEL_SAMPLE_RATE: u32 = 16000;
const MODEL_INPUT_LENGTH: usize = 144160;
const WINDOW_STEP: usize = 16000;

// ============================================================================
// 辅助函数
// ============================================================================

#[inline]
fn polyfit(x: f64, (a, b, c): (f64, f64, f64)) -> f64 {
    a * x * x + b * x + c
}

#[inline]
fn clamp(value: f64, min_val: f64, max_val: f64) -> f64 {
    if value.is_nan() || value.is_infinite() {
        return 3.0;
    }
    value.max(min_val).min(max_val)
}

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

fn resample_to_16kHz(samples: &[f64], sample_rate: u32) -> Vec<f64> {
    resample(samples, sample_rate, MODEL_SAMPLE_RATE)
}

fn segment_audio(samples: &[f64]) -> Vec<Vec<f64>> {
    if samples.len() < MODEL_INPUT_LENGTH {
        let mut padded = vec![0.0; MODEL_INPUT_LENGTH];
        padded[..samples.len()].copy_from_slice(samples);
        return vec![padded];
    }

    let mut windows = Vec::new();
    let mut start = 0;
    while start + MODEL_INPUT_LENGTH <= samples.len() {
        windows.push(samples[start..start + MODEL_INPUT_LENGTH].to_vec());
        start += WINDOW_STEP;
    }

    if start < samples.len() {
        let mut padded = vec![0.0; MODEL_INPUT_LENGTH];
        let remaining = samples.len() - start;
        padded[..remaining].copy_from_slice(&samples[start..]);
        windows.push(padded);
    }

    windows
}

// ============================================================================
// DNSMOS 结果
// ============================================================================

#[derive(Debug, Clone)]
pub struct DnsMosResult {
    pub sig: f64,
    pub bak: f64,
    pub ovrl: f64,
}

impl DnsMosResult {
    fn from_raw(sig_raw: f64, bak_raw: f64, ovrl_raw: f64) -> Self {
        let sig = clamp(polyfit(sig_raw, POLY_SIG), 1.0, 5.0);
        let bak = clamp(polyfit(bak_raw, POLY_BAK), 1.0, 5.0);
        let ovrl = clamp(polyfit(ovrl_raw, POLY_OVRL), 1.0, 5.0);
        Self { sig, bak, ovrl }
    }
}

// ============================================================================
// DNSMOS 评估器 - Windows 实现
// ============================================================================

#[cfg(target_os = "windows")]
pub struct DnsMosEvaluator {
    session: ort::session::Session,
}

/// DNSMOS 模型字节
const DNSMOS_MODEL: &[u8] = include_bytes!("../bin/model/sig_bak_ovr.onnx");
const DNSMOS_MODEL_HASH: &str = env!("DNSMOS_MODEL_HASH");

#[cfg(target_os = "windows")]
impl DnsMosEvaluator {
    pub fn new(_model_bytes: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        windows_impl::init_dlls();
        let session = ort::session::Session::builder()?
            .commit_from_memory(DNSMOS_MODEL)?;
        Ok(Self { session })
    }

    pub fn evaluate(&mut self, samples: &[f64], sample_rate: u32) -> Result<DnsMosResult, Box<dyn Error + Send + Sync>> {
        let samples_16k = resample_to_16kHz(samples, sample_rate);
        let windows = segment_audio(&samples_16k);

        let mut sig_sum = 0.0;
        let mut bak_sum = 0.0;
        let mut ovrl_sum = 0.0;

        for window in &windows {
            let input_data: Vec<f32> = window.iter().map(|&x| x as f32).collect();
            let input_tensor = ort::value::Tensor::from_array((
                vec![1i64, MODEL_INPUT_LENGTH as i64],
                input_data.into_boxed_slice()
            ))?;

            let outputs = self.session.run(ort::inputs![
                "input_1" => input_tensor
            ])?;

            let output_view = outputs[0].try_extract_array::<f32>()?;
            if output_view.len() != 3 {
                return Err(format!("DNSMOS 模型输出维度错误: {}", output_view.len()).into());
            }

            let sig_raw = output_view[[0, 0]] as f64;
            let bak_raw = output_view[[0, 1]] as f64;
            let ovrl_raw = output_view[[0, 2]] as f64;

            let result = DnsMosResult::from_raw(sig_raw, bak_raw, ovrl_raw);
            sig_sum += result.sig;
            bak_sum += result.bak;
            ovrl_sum += result.ovrl;
        }

        let n = windows.len() as f64;
        Ok(DnsMosResult {
            sig: sig_sum / n,
            bak: bak_sum / n,
            ovrl: ovrl_sum / n,
        })
    }
}

// ============================================================================
// Mac/Linux 空实现（仅满足类型定义）
// ============================================================================

#[cfg(not(target_os = "windows"))]
pub struct DnsMosEvaluator;

#[cfg(not(target_os = "windows"))]
impl DnsMosEvaluator {
    pub fn new(_model_bytes: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        Err("DNSMOS 功能仅在 Windows 上可用".into())
    }

    pub fn evaluate(&mut self, _samples: &[f64], _sample_rate: u32) -> Result<DnsMosResult, Box<dyn Error + Send + Sync>> {
        Err("DNSMOS 功能仅在 Windows 上可用".into())
    }
}

// ============================================================================
// 公开 API
// ============================================================================

pub fn evaluate_audio(samples: &[f64], sample_rate: u32) -> Result<DnsMosResult, Box<dyn Error + Send + Sync>> {
    let mut evaluator = DnsMosEvaluator::new(DNSMOS_MODEL)?;
    evaluator.evaluate(samples, sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_polyfit() {
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
        assert!((clamp(f64::NAN, 1.0, 5.0) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_resample() {
        let samples = vec![0.0, 1.0, 2.0, 3.0, 4.0];
        let resampled = resample(&samples, 1000, 2000);
        assert_eq!(resampled.len(), 10);
    }

    #[test]
    fn test_segment_long_audio() {
        let samples: Vec<f64> = (0..320000).map(|i| (i as f64 * 0.01).sin()).collect();
        let windows = segment_audio(&samples);
        assert!(windows.len() >= 10);
        for w in &windows {
            assert_eq!(w.len(), MODEL_INPUT_LENGTH);
        }
    }

    #[test]
    fn test_segment_short_audio() {
        let samples: Vec<f64> = (0..48000).map(|i| (i as f64 * 0.01).sin()).collect();
        let windows = segment_audio(&samples);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].len(), MODEL_INPUT_LENGTH);
    }
}
