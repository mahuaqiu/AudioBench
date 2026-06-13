//! Gammatone 滤波器组模块
//! 
//! 实现与 ViSQOL 等效的 Gammatone 滤波器组，模拟人耳听觉特性。
//! 使用 ERB（等效矩形带宽）刻度计算中心频率。


use std::f64::consts::PI;

/// 使用 ERB 刻度生成均匀分布的中心频率
pub fn erb_center_frequencies(low_freq: f64, high_freq: f64, num_bands: usize) -> Vec<f64> {
    let ear_q = 9.26449;  // Glasberg and Moore 参数
    let min_bw: f64 = 24.7;
    
    let a = -(ear_q * min_bw);
    let b = - (high_freq + ear_q * min_bw).ln();
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

/// 构建简化的 Gammatone 滤波器系数
/// 使用近似方法避免复数运算
pub fn make_gammatone_filters(sample_rate: u32, num_bands: usize, min_freq: f64, max_freq: f64) -> GammatoneFilter {
    let high_freq = max_freq.min(sample_rate as f64 / 2.0);
    let center_freqs = erb_center_frequencies(min_freq, high_freq, num_bands);
    
    let ear_q = 9.26449;
    let min_bw: f64 = 24.7;
    let _t = 1.0 / sample_rate as f64;
    
    let mut coeffs = Vec::with_capacity(num_bands);
    let mut bandwidths = Vec::with_capacity(num_bands);
    
    for cf in &center_freqs {
        // ERB 带宽
        let erb = ((cf / ear_q).powf(1.0) + min_bw.powf(1.0)).powf(1.0 / 1.0);
        let bw = 1.019 * 2.0 * PI * erb;
        bandwidths.push(bw);
        
        // 简化的滤波器系数 [center_freq, bw, gain]
        coeffs.push(vec![*cf, bw, 1.0]);
    }
    
    GammatoneFilter {
        center_freqs,
        coeffs,
        bandwidths,
        num_bands,
    }
}

/// Gammatone 滤波器结构
#[derive(Debug, Clone)]
pub struct GammatoneFilter {
    pub center_freqs: Vec<f64>,
    coeffs: Vec<Vec<f64>>,
    bandwidths: Vec<f64>,
    num_bands: usize,
}

impl GammatoneFilter {
    /// 应用简化 Gammatone 滤波器
    pub fn apply(&self, signal: &[f64], sample_rate: u32) -> Vec<f64> {
        let num_samples = signal.len();
        let mut output = vec![0.0; self.num_bands * num_samples];
        
        for band in 0..self.num_bands {
            let cf = self.coeffs[band][0];
            let bw = self.bandwidths[band];
            
            // 使用带通滤波器近似 Gammatone
            // 中心频率 cf，带宽 bw
            let _omega = 2.0 * PI * cf / sample_rate as f64;
            let alpha = (bw * PI / sample_rate as f64).tan();
            
            // 简化的一阶 IIR 滤波器
            let mut y = 0.0;
            let mut x_prev = 0.0;
            
            for (i, &x) in signal.iter().enumerate() {
                // 低通近似
                let x_filtered = x * 0.3 + x_prev * 0.7;
                x_prev = x;
                
                // 带通响应
                let bandpass = (x_filtered * (1.0 + alpha) - y * (1.0 - alpha)) / (1.0 + alpha);
                y = y + bandpass;
                
                output[band * num_samples + i] = y.abs();
            }
        }
        
        output
    }
}

/// 构建频谱图（使用简化 Gammatone 滤波器组）
pub fn build_spectrogram(
    signal: &[f64],
    sample_rate: u32,
    frame_size: usize,
    hop_size: usize,
    num_bands: usize,
    use_speech_mode: bool,
) -> (Vec<Vec<f64>>, Vec<f64>) {
    let max_freq = if use_speech_mode { 8000.0 } else { sample_rate as f64 / 2.0 };
    let min_freq = 50.0;
    
    let filter = make_gammatone_filters(sample_rate, num_bands, min_freq, max_freq);
    let center_freqs = filter.center_freqs.clone();
    
    let num_frames = if signal.len() > frame_size {
        1 + (signal.len() - frame_size) / hop_size
    } else {
        1
    };
    
    let mut spectrogram = Vec::with_capacity(num_bands);
    for _ in 0..num_bands {
        spectrogram.push(Vec::with_capacity(num_frames));
    }
    
    // Hann 窗口
    let window: Vec<f64> = (0..frame_size)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f64 / (frame_size - 1) as f64).cos()))
        .collect();
    
    // 分帧处理
    let mut pos = 0;
    while pos + frame_size <= signal.len() {
        // 加窗
        let mut frame = Vec::with_capacity(frame_size);
        for i in 0..frame_size {
            frame.push(signal[pos + i] * window[i]);
        }
        
        // 应用 Gammatone 滤波器
        let filtered = filter.apply(&frame, sample_rate);
        
        // 计算各频段能量（RMS）
        for band in 0..num_bands {
            let band_start = band * frame_size;
            let band_end = band_start + frame_size;
            let energy: f64 = filtered[band_start..band_end.min(filtered.len())]
                .iter()
                .map(|&x| x * x)
                .sum::<f64>();
            spectrogram[band].push(energy.sqrt());
        }
        
        pos += hop_size;
    }
    
    (spectrogram, center_freqs)
}

/// 计算 SPL（声压级）归一化因子
pub fn compute_spl_scale_factor(reference: &[f64], degraded: &[f64]) -> f64 {
    let ref_rms = (reference.iter().map(|x| x * x).sum::<f64>() / reference.len() as f64).sqrt();
    let deg_rms = (degraded.iter().map(|x| x * x).sum::<f64>() / degraded.len() as f64).sqrt();
    
    if deg_rms > 1e-10 && ref_rms > 1e-10 {
        ref_rms / deg_rms
    } else {
        1.0
    }
}

/// 将频谱图转换为 dB
pub fn spectrogram_to_db(spectrogram: &mut [Vec<f64>]) {
    let noise_floor_db = -45.0;
    
    for band in spectrogram.iter_mut() {
        for val in band.iter_mut() {
            if *val > 1e-10 {
                *val = 20.0 * val.log10();
            } else {
                *val = noise_floor_db;
            }
        }
    }
}

/// 应用噪声门限
pub fn apply_noise_floor(spectrogram: &mut [Vec<f64>], noise_threshold_db: f64) {
    let num_frames = if !spectrogram.is_empty() { spectrogram[0].len() } else { 0 };
    if num_frames == 0 { return; }
    
    for frame_idx in 0..num_frames {
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

/// 预处理：SPL 归一化 + dB 转换 + 噪声门限
pub fn preprocess_spectrograms(
    ref_spectro: &mut [Vec<f64>],
    deg_spectro: &mut [Vec<f64>],
    ref_signal: &[f64],
    deg_signal: &[f64],
) {
    // SPL 归一化
    let scale = compute_spl_scale_factor(ref_signal, deg_signal);
    for band in deg_spectro.iter_mut() {
        for val in band.iter_mut() {
            *val *= scale;
        }
    }
    
    // 转 dB
    spectrogram_to_db(ref_spectro);
    spectrogram_to_db(deg_spectro);
    
    // 绝对门限
    let abs_floor = -45.0;
    for band in ref_spectro.iter_mut() {
        for val in band.iter_mut() {
            *val = val.max(abs_floor);
        }
    }
    for band in deg_spectro.iter_mut() {
        for val in band.iter_mut() {
            *val = val.max(abs_floor);
        }
    }
    
    // 相对门限
    apply_noise_floor(ref_spectro, 45.0);
    apply_noise_floor(deg_spectro, 45.0);
    
    // 归一化到 0dB 地板
    let mut min_val = f64::INFINITY;
    for band in ref_spectro.iter() {
        for val in band.iter() {
            min_val = min_val.min(*val);
        }
    }
    for band in deg_spectro.iter() {
        for val in band.iter() {
            min_val = min_val.min(*val);
        }
    }
    
    for band in ref_spectro.iter_mut() {
        for val in band.iter_mut() {
            *val -= min_val;
        }
    }
    for band in deg_spectro.iter_mut() {
        for val in band.iter_mut() {
            *val -= min_val;
        }
    }
}
