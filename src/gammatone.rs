//! 频谱图构建模块
//! 
//! 使用 FFT 频谱分析替代有问题的 Gammatone 滤波器
//! 与 ViSQOL 的核心思路一致：时频分析 + 比较

use std::f64::consts::PI;

/// 使用 ERB 刻度生成均匀分布的中心频率（保留兼容）
pub fn erb_center_frequencies(low_freq: f64, high_freq: f64, num_bands: usize) -> Vec<f64> {
    let ear_q = 9.26449;
    let min_bw: f64 = 24.7;
    
    let a = -(ear_q * min_bw);
    let b = -(high_freq + ear_q * min_bw).ln();
    let c = (low_freq + ear_q * min_bw).ln();
    let d = high_freq + ear_q * min_bw;
    let e = (b + c) / num_bands as f64;
    
    let mut cfs = Vec::with_capacity(num_bands);
    for i in 0..num_bands {
        let f = ((i + 1) as f64) * e;
        let freq = f.exp() * d;
        cfs.push(a + freq);
    }
    cfs
}

/// 构建频谱图（使用 FFT 频谱分析）
pub fn build_spectrogram(
    signal: &[f64],
    sample_rate: u32,
    frame_size: usize,
    hop_size: usize,
    num_bands: usize,
) -> (Vec<Vec<f64>>, Vec<f64>) {
    let sample_rate = sample_rate as f64;
    let num_frames = if signal.len() > frame_size {
        1 + (signal.len() - frame_size) / hop_size
    } else { 1 };
    
    // 计算ERB中心频率
    let center_freqs = erb_center_frequencies(50.0, sample_rate / 2.0, num_bands);
    
    // 预计算FFT所需的正弦余弦表（简化版：使用直接FFT）
    let mut spectrogram = Vec::with_capacity(num_bands);
    for _ in 0..num_bands {
        spectrogram.push(Vec::with_capacity(num_frames));
    }
    
    // Hann 窗口
    let window: Vec<f64> = (0..frame_size)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f64 / (frame_size - 1) as f64).cos()))
        .collect();
    
    // 对每一帧进行FFT分析
    let mut pos = 0;
    while pos + frame_size <= signal.len() {
        // 加窗
        let mut frame = Vec::with_capacity(frame_size);
        for i in 0..frame_size {
            frame.push(signal[pos + i] * window[i]);
        }
        
        // 简单FFT计算频谱幅度
        let fft_result = simple_fft(&frame);
        
        // 将FFT结果映射到ERB频带
        for band in 0..num_bands {
            let cf = center_freqs[band];
            // 将中心频率转换为FFT bins
            let freq_resolution = sample_rate / frame_size as f64;
            let bin_center = (cf / freq_resolution).round() as usize;
            let bin_width = ((cf * 0.5) / freq_resolution).round() as usize;
            
            // 计算该频带的平均能量
            let mut sum = 0.0;
            let mut count = 0;
            for b in (bin_center.saturating_sub(bin_width))..(bin_center + bin_width).min(fft_result.len()) {
                sum += fft_result[b];
                count += 1;
            }
            let energy = if count > 0 { sum / count as f64 } else { 0.0 };
            spectrogram[band].push(energy);
        }
        
        pos += hop_size;
    }
    
    // 如果信号太短，至少返回一个frame
    if spectrogram[0].is_empty() {
        for band in 0..num_bands {
            spectrogram[band].push(0.0);
        }
    }
    
    (spectrogram, center_freqs)
}

/// 简化的离散傅里叶变换（DFT）
/// 返回每个频率bin的幅度
fn simple_fft(signal: &[f64]) -> Vec<f64> {
    let n = signal.len();
    let mut magnitudes = Vec::with_capacity(n / 2);
    
    // 只需要计算到Nyquist频率
    for k in 0..(n / 2) {
        let mut real = 0.0;
        let mut imag = 0.0;
        
        for (t, &sample) in signal.iter().enumerate() {
            let angle = -2.0 * PI * (k * t) as f64 / n as f64;
            real += sample * angle.cos();
            imag += sample * angle.sin();
        }
        
        // 计算幅度
        let magnitude = (real * real + imag * imag).sqrt();
        magnitudes.push(magnitude);
    }
    
    magnitudes
}

/// 与 ViSQOL MiscAudio::ScaleToMatchSoundPressureLevel 一致
pub fn compute_spl_scale_factor(reference: &[f64], degraded: &[f64]) -> f64 {
    let ref_spl = compute_spl_db(reference);
    let deg_spl = compute_spl_db(degraded);
    
    if deg_spl > -100.0 && ref_spl > -100.0 {
        10.0_f64.powf((ref_spl - deg_spl) / 20.0)
    } else {
        1.0
    }
}

/// 计算信号 SPL（dB）
fn compute_spl_db(signal: &[f64]) -> f64 {
    let sum: f64 = signal.iter().map(|&x| x * x).sum();
    let rms = (sum / signal.len().max(1) as f64).sqrt();
    if rms > 1e-10 {
        20.0 * (rms / 0.00002).log10()
    } else {
        -100.0
    }
}

/// 将频谱图转换为 dB（与 ViSQOL Spectrogram::ConvertToDb 一致）
pub fn spectrogram_to_db(spectrogram: &mut [Vec<f64>]) {
    for band in spectrogram.iter_mut() {
        for val in band.iter_mut() {
            if *val > 1e-10 {
                *val = 10.0 * val.log10();
            } else {
                *val = -100.0;
            }
        }
    }
}

/// 应用噪声门限（与 ViSQOL Spectrogram::RaiseFloorPerFrame 一致）
pub fn apply_noise_floor(spectrogram: &mut [Vec<f64>], noise_threshold_db: f64) {
    let min_cols = spectrogram.iter().map(|b| b.len()).min().unwrap_or(0);
    if min_cols == 0 { return; }
    
    for frame_idx in 0..min_cols {
        let mut frame_max = f64::NEG_INFINITY;
        for band in spectrogram.iter() {
            if frame_idx < band.len() {
                frame_max = frame_max.max(band[frame_idx]);
            }
        }
        
        let frame_floor = frame_max - noise_threshold_db;
        
        for band in spectrogram.iter_mut() {
            if frame_idx < band.len() {
                band[frame_idx] = band[frame_idx].max(frame_floor);
            }
        }
    }
}

/// 绝对门限（dB）
const ABSOLUTE_NOISE_FLOOR_DB: f64 = -80.0;

/// 预处理（与 ViSQOL PrepareSpectrogramsForComparison 一致）
pub fn preprocess_spectrograms(
    ref_spectro: &mut [Vec<f64>],
    deg_spectro: &mut [Vec<f64>],
    _ref_signal: &[f64],
    _deg_signal: &[f64],
) {
    // 转 dB
    spectrogram_to_db(ref_spectro);
    spectrogram_to_db(deg_spectro);
    
    // 绝对门限 -80dB
    for band in ref_spectro.iter_mut() {
        for val in band.iter_mut() { *val = val.max(ABSOLUTE_NOISE_FLOOR_DB); }
    }
    for band in deg_spectro.iter_mut() {
        for val in band.iter_mut() { *val = val.max(ABSOLUTE_NOISE_FLOOR_DB); }
    }
    
    // 相对门限
    apply_noise_floor(ref_spectro, 30.0);
    apply_noise_floor(deg_spectro, 30.0);
    
    // 归一化到 0dB 地板
    let mut min_val = f64::INFINITY;
    for band in ref_spectro.iter() {
        for val in band.iter() { min_val = min_val.min(*val); }
    }
    for band in deg_spectro.iter() {
        for val in band.iter() { min_val = min_val.min(*val); }
    }
    
    for band in ref_spectro.iter_mut() {
        for val in band.iter_mut() { *val -= min_val; }
    }
    for band in deg_spectro.iter_mut() {
        for val in band.iter_mut() { *val -= min_val; }
    }
}

/// 在时域对 degraded 信号做 SPL 归一化
pub fn scale_to_match_spl(reference: &[f64], degraded: &mut [f64]) {
    let scale = compute_spl_scale_factor(reference, degraded);
    for val in degraded.iter_mut() {
        *val *= scale;
    }
}
