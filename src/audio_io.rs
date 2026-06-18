//! 音频加载与预处理模块
//! 负责 WAV 解码、重采样（带抗混叠低通滤波）、单声道化

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use rustfft::{num_complex::Complex, FftPlanner};
use std::path::Path;

/// FIR 低通抗混叠滤波器的阶数（奇数，越高越陡峭，计算量越大）
const FIR_FILTER_ORDER: usize = 127;

/// 设计加窗 sinc 低通滤波器（线性相位 FIR）
///
/// 截止频率 `cutoff_hz`，采样率 `sample_rate`，返回长度 = order 的对称 FIR 系数。
fn design_lowpass_fir(cutoff_hz: f64, sample_rate: f64, order: usize) -> Vec<f64> {
    let mut h = vec![0.0; order];
    let mid = (order as f64 - 1.0) / 2.0;
    // 归一化截止角频率：fc = cutoff / Nyquist
    let fc = cutoff_hz / (sample_rate / 2.0);
    for i in 0..order {
        let n = i as f64 - mid;
        let mut val;
        if n.abs() < 1e-12 {
            // sinc(0) = 1
            val = 2.0 * fc;
        } else {
            // sinc 函数：sin(2π fc n) / (π n)
            val = (2.0 * fc * std::f64::consts::PI * n).sin() / (std::f64::consts::PI * n);
            val *= 2.0 * fc;
        }
        // 汉明窗，压低旁瓣
        let window = 0.54 - 0.46
            * (2.0 * std::f64::consts::PI * i as f64 / (order - 1) as f64).cos();
        h[i] = val * window;
    }
    // 归一化到单位直流增益
    let sum: f64 = h.iter().sum();
    if sum.abs() > 1e-12 {
        for v in h.iter_mut() {
            *v /= sum;
        }
    }
    h
}

/// 对信号做 FFT 卷积（重叠保留风格：补零到 FFT 大小，频域相乘，反变换）
fn apply_fir_filter(samples: &[f64], fir: &[f64]) -> Vec<f64> {
    if samples.is_empty() {
        return vec![];
    }
    let n = samples.len();
    let m = fir.len();
    // FFT 长度取 2 的幂 ≥ n + m - 1
    let fft_len = (n + m - 1).next_power_of_two().max(64);
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(fft_len);
    let ifft = planner.plan_fft_inverse(fft_len);

    // 准备信号（补零）
    let mut sig_buf: Vec<Complex<f64>> = (0..fft_len)
        .map(|i| {
            if i < n {
                Complex::new(samples[i], 0.0)
            } else {
                Complex::new(0.0, 0.0)
            }
        })
        .collect();
    // 准备滤波器（补零）
    let mut fir_buf: Vec<Complex<f64>> = (0..fft_len)
        .map(|i| {
            if i < m {
                Complex::new(fir[i], 0.0)
            } else {
                Complex::new(0.0, 0.0)
            }
        })
        .collect();

    fft.process(&mut sig_buf);
    fft.process(&mut fir_buf);
    for i in 0..fft_len {
        sig_buf[i] *= fir_buf[i];
    }
    ifft.process(&mut sig_buf);

    // 取前 n+m-1 个点，并除以 fft_len（rustfft 的 inverse 不做归一化）
    let out_len = n + m - 1;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        out.push(sig_buf[i].re / fft_len as f64);
    }
    out
}

/// 带抗混叠低通滤波的线性插值重采样
///
/// 流程：
/// - 降采样（from > to）：先 FIR 低通（截止 = to/2 × 0.9）滤掉高于新奈奎斯特的成分，
///   再做线性抽取，避免高频折叠回基带。
/// - 升采样（from < to）：先线性插值上采样，再用截止 = from/2 × 0.9 的低通清理镜像。
/// - 同采样率：直接克隆。
///
/// 相比纯线性插值，可显著降低大幅降采样（如 48k→16k）时的混叠损伤，
/// 改善 ViSQOL NSIM 频谱相似度。
fn resample_with_antialiasing(samples: &[f64], from_rate: u32, to_rate: u32) -> Vec<f64> {
    if from_rate == to_rate {
        return samples.to_vec();
    }

    let from = from_rate as f64;
    let to = to_rate as f64;

    let mut working: Vec<f64> = samples.to_vec();
    let mut working_rate = from_rate;

    // 降采样路径：先抗混叠低通，再抽取
    if from > to {
        let cutoff = (to / 2.0) * 0.9;
        let fir = design_lowpass_fir(cutoff, from, FIR_FILTER_ORDER);
        let filtered = apply_fir_filter(samples, &fir);
        // 抽取前用滤波后信号，去掉前导暂态（半个滤波器长度）
        let half = FIR_FILTER_ORDER / 2;
        let trimmed = if filtered.len() > half {
            &filtered[half..]
        } else {
            &filtered[..]
        };
        working = linear_extract(trimmed, from_rate, to_rate);
        working_rate = to_rate;
    }

    // 升采样路径：线性插值后清理镜像
    if from < to {
        let upsampled = linear_extract(samples, from_rate, to_rate);
        let cutoff = (from / 2.0) * 0.9;
        let fir = design_lowpass_fir(cutoff, to, FIR_FILTER_ORDER);
        let filtered = apply_fir_filter(&upsampled, &fir);
        let half = FIR_FILTER_ORDER / 2;
        working = if filtered.len() > half {
            filtered[half..].to_vec()
        } else {
            filtered
        };
        working_rate = to_rate;
    }

    // working_rate 与 to_rate 在两条分支里都已对齐，直接返回
    let _ = working_rate;
    working
}

/// 线性插值抽取/插值（内部使用，不单独做抗混叠）
fn linear_extract(samples: &[f64], from_rate: u32, to_rate: u32) -> Vec<f64> {
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


/// 音频数据（单声道 f64）
pub struct AudioData {
    pub samples: Vec<f64>,
    pub sample_rate: u32,
}

impl AudioData {
    /// 从 WAV 文件加载音频，自动转为单声道 f64
    pub fn from_wav(path: &Path) -> Result<Self, String> {
        let mut reader = WavReader::open(path)
            .map_err(|e| format!("无法打开 WAV 文件 {:?}: {}", path, e))?;

        let spec = reader.spec();
        let channels = spec.channels as usize;
        let sample_rate = spec.sample_rate;

        let raw: Vec<f64> = match spec.sample_format {
            SampleFormat::Float => {
                reader.samples::<f32>()
                    .map(|s| s.map(|v| v as f64).map_err(|e| e.to_string()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("读取采样失败: {}", e))?
            }
            SampleFormat::Int => {
                let max_val = (1i64 << (spec.bits_per_sample - 1)) as f64;
                reader.samples::<i32>()
                    .map(|s| s.map(|v| v as f64 / max_val).map_err(|e| e.to_string()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("读取采样失败: {}", e))?
            }
        };

        // 多声道转单声道：取各声道平均值
        let mono = if channels > 1 {
            let frame_count = raw.len() / channels;
            let mut out = Vec::with_capacity(frame_count);
            for i in 0..frame_count {
                let sum: f64 = (0..channels).map(|ch| raw[i * channels + ch]).sum();
                out.push(sum / channels as f64);
            }
            out
        } else {
            raw
        };

        Ok(AudioData {
            samples: mono,
            sample_rate,
        })
    }

    /// 重采样到目标采样率（带 FIR 抗混叠低通滤波）
    #[allow(dead_code)]
    pub fn resample(&self, target_rate: u32) -> Result<Self, String> {
        if self.sample_rate == target_rate {
            return Ok(AudioData {
                samples: self.samples.clone(),
                sample_rate: target_rate,
            });
        }

        let output = resample_with_antialiasing(&self.samples, self.sample_rate, target_rate);

        Ok(AudioData {
            samples: output,
            sample_rate: target_rate,
        })
    }

    /// 音频时长（秒）
    pub fn duration_secs(&self) -> f64 {
        self.samples.len() as f64 / self.sample_rate as f64
    }
}

/// 写入单声道 16-bit PCM WAV 文件（供 visqol 调用）
/// ViSQOL 只支持 16-bit PCM
pub fn write_wav_mono(path: &std::path::Path, samples: &[f64], sample_rate: u32) -> Result<(), String> {
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    
    let mut writer = WavWriter::create(path, spec)
        .map_err(|e| format!("创建 WAV 文件失败: {}", e))?;
    
    let max_val = 32767.0;
    for &sample in samples {
        let s = (sample * max_val).clamp(-32768.0, 32767.0) as i32;
        writer.write_sample(s)
            .map_err(|e| format!("写入采样失败: {}", e))?;
    }
    
    writer.finalize()
        .map_err(|e| format!("关闭 WAV 文件失败: {}", e))?;
    
    Ok(())
}
