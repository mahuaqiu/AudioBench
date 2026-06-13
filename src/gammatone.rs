//! Gammatone 滤波器组模块
//! 
//! 实现与 ViSQOL 等效的四阶 Gammatone 滤波器组，模拟人耳听觉特性。
//! 使用 ERB（等效矩形带宽）刻度计算中心频率。

use std::f64::consts::PI;

/// 使用 ERB 刻度生成均匀分布的中心频率
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

/// 二阶 IIR 滤波器系数
#[derive(Clone, Debug)]
struct BiquadCoeffs {
    a1: f64,
    a2: f64,
    b0: f64,
    b1: f64,
    b2: f64,
}

/// 单个 Gammatone 频段滤波器（四阶 IIR）
#[derive(Clone)]
struct GammatoneBandFilter {
    stage1: BiquadCoeffs,
    stage2: BiquadCoeffs,
    stage3: BiquadCoeffs,
    stage4: BiquadCoeffs,
}

impl GammatoneBandFilter {
    fn apply(&self, signal: &[f64]) -> Vec<f64> {
        let stage1_out = self.biquad_filter(signal, &self.stage1);
        let stage2_out = self.biquad_filter(&stage1_out, &self.stage2);
        let stage3_out = self.biquad_filter(&stage2_out, &self.stage3);
        self.biquad_filter(&stage3_out, &self.stage4)
    }
    
    fn biquad_filter(&self, input: &[f64], coeffs: &BiquadCoeffs) -> Vec<f64> {
        let n = input.len();
        let mut output = vec![0.0; n];
        let mut s0 = 0.0_f64;
        let mut s1 = 0.0_f64;
        
        for (i, &x) in input.iter().enumerate() {
            let y = coeffs.b0 * x + s0;
            s0 = coeffs.b1 * x + s1 - coeffs.a1 * y;
            s1 = coeffs.b2 * x - coeffs.a2 * y;
            output[i] = y;
        }
        output
    }
}

/// Gammatone 滤波器结构
#[derive(Clone)]
pub struct GammatoneFilter {
    pub center_freqs: Vec<f64>,
    bands: Vec<GammatoneBandFilter>,
    num_bands: usize,
}

/// 计算 Gammatone 滤波器系数（与 ViSQOL equivalent_rectangular_bandwidth.cc 等效）
pub fn make_gammatone_filters(sample_rate: u32, num_bands: usize, min_freq: f64, max_freq: f64) -> GammatoneFilter {
    let sample_rate = sample_rate as f64;
    let high_freq = max_freq.min(sample_rate / 2.0);
    let center_freqs = erb_center_frequencies(min_freq, high_freq, num_bands);
    
    let ear_q = 9.26449;
    let min_bw: f64 = 24.7;
    let t = 1.0 / sample_rate;
    
    let mut bands = Vec::with_capacity(num_bands);
    
    for &cf in &center_freqs {
        let erb = ((cf / ear_q).powi(1) + min_bw.powi(1)).powf(1.0);
        let b_val = 1.019 * 2.0 * PI * erb;
        
        let exp_bt = (-b_val * t).exp();
        let cos_2pi_cf_t = (2.0 * PI * cf * t).cos();
        let sin_2pi_cf_t = (2.0 * PI * cf * t).sin();
        let b1 = -2.0 * cos_2pi_cf_t / exp_bt;
        let b2 = (-2.0 * b_val * t).exp();
        
        let a_coef = cos_2pi_cf_t * 2.0 * t;
        let b0 = sin_2pi_cf_t * t;
        
        let p1 = 2.0_f64.powf(3.0 / 2.0);
        let s1 = (3.0 - p1).sqrt();
        let s2 = (3.0 + p1).sqrt();
        
        let x01 = -2.0 * t;
        let x02 = 2.0 * (-b_val * t).exp() * t;
        
        let x12 = cos_2pi_cf_t - s1 * sin_2pi_cf_t;
        let x1 = x01 + x02 * x12;
        let x22 = cos_2pi_cf_t + s1 * sin_2pi_cf_t;
        let x2 = x01 + x02 * x22;
        let x32 = cos_2pi_cf_t - s2 * sin_2pi_cf_t;
        let x3 = x01 + x02 * x32;
        let x42 = cos_2pi_cf_t + s2 * sin_2pi_cf_t;
        let x4 = x01 + x02 * x42;
        
        let x5 = -2.0 / (2.0 * b_val * t).exp() - 2.0 + 2.0 * (1.0 + 1.0_f64.exp()) / (b_val * t).exp();
        let y = x5.powi(4);
        let gain = ((x1 * x2 * x3 * x4) / y).abs();
        
        let a11 = -(a_coef / exp_bt + b0 * 2.0 * s1 / exp_bt) / 2.0;
        let a12 = -(a_coef / exp_bt - b0 * 2.0 * s1 / exp_bt) / 2.0;
        let a13 = -(a_coef / exp_bt + b0 * 2.0 * s2 / exp_bt) / 2.0;
        let a14 = -(a_coef / exp_bt - b0 * 2.0 * s2 / exp_bt) / 2.0;
        
        let b0_s1 = b0 * 2.0 * s1;
        let b0_s2 = b0 * 2.0 * s2;
        
        bands.push(GammatoneBandFilter {
            stage1: BiquadCoeffs { a1: a11 / gain, a2: 0.0, b0: b0_s1, b1, b2 },
            stage2: BiquadCoeffs { a1: a12, a2: 0.0, b0: b0_s1, b1, b2 },
            stage3: BiquadCoeffs { a1: a13, a2: 0.0, b0: b0_s2, b1, b2 },
            stage4: BiquadCoeffs { a1: a14, a2: 0.0, b0: b0_s2, b1, b2 },
        });
    }
    
    GammatoneFilter { center_freqs, bands, num_bands }
}

impl GammatoneFilter {
    pub fn apply(&self, signal: &[f64]) -> Vec<f64> {
        let num_samples = signal.len();
        let mut output = vec![0.0; self.num_bands * num_samples];
        
        for (band_idx, band_filter) in self.bands.iter().enumerate() {
            let filtered = band_filter.apply(signal);
            for (i, &v) in filtered.iter().enumerate() {
                output[band_idx * num_samples + i] = v;
            }
        }
        output
    }
}

/// 构建频谱图
pub fn build_spectrogram(
    signal: &[f64],
    sample_rate: u32,
    frame_size: usize,
    hop_size: usize,
    num_bands: usize,
) -> (Vec<Vec<f64>>, Vec<f64>) {
    let max_freq = sample_rate as f64 / 2.0;
    let min_freq = 50.0;
    
    let filter = make_gammatone_filters(sample_rate, num_bands, min_freq, max_freq);
    let center_freqs = filter.center_freqs.clone();
    
    let num_frames = if signal.len() > frame_size {
        1 + (signal.len() - frame_size) / hop_size
    } else { 1 };
    
    let mut spectrogram = Vec::with_capacity(num_bands);
    for _ in 0..num_bands {
        spectrogram.push(Vec::with_capacity(num_frames));
    }
    
    // Hann 窗口
    let window: Vec<f64> = (0..frame_size)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f64 / (frame_size - 1) as f64).cos()))
        .collect();
    
    let mut pos = 0;
    while pos + frame_size <= signal.len() {
        let mut frame = Vec::with_capacity(frame_size);
        for i in 0..frame_size {
            frame.push(signal[pos + i] * window[i]);
        }
        
        let filtered = filter.apply(&frame);
        
        for band in 0..num_bands {
            let band_start = band * frame_size;
            let band_end = (band_start + frame_size).min(filtered.len());
            let energy: f64 = filtered[band_start..band_end].iter().map(|&x| x * x).sum::<f64>();
            let rms = (energy / frame_size as f64).sqrt();
            spectrogram[band].push(rms);
        }
        
        pos += hop_size;
    }
    
    (spectrogram, center_freqs)
}

/// 与 ViSQOL MiscAudio::ScaleToMatchSoundPressureLevel 一致
/// 在时域对 degraded 信号做 SPL 归一化
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
                *val = 10.0 * val.log10();  // 与 ViSQOL 一致：10*log10
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

/// 预处理（与 ViSQOL PrepareSpectrogramsForComparison 一致）
pub fn preprocess_spectrograms(
    ref_spectro: &mut [Vec<f64>],
    deg_spectro: &mut [Vec<f64>],
    _ref_signal: &[f64],
    _deg_signal: &[f64],
) {
    // 注意：SPL 归一化现在在外部时域做了，这里只做频域预处理
    
    // 转 dB（与 ViSQOL 一致）
    spectrogram_to_db(ref_spectro);
    spectrogram_to_db(deg_spectro);
    
    // 绝对门限 -45dB（与 ViSQOL 一致）
    let abs_floor = -45.0;
    for band in ref_spectro.iter_mut() {
        for val in band.iter_mut() { *val = val.max(abs_floor); }
    }
    for band in deg_spectro.iter_mut() {
        for val in band.iter_mut() { *val = val.max(abs_floor); }
    }
    
    // 相对门限（与 ViSQOL RaiseFloorPerFrame 一致）
    apply_noise_floor(ref_spectro, 30.0);
    apply_noise_floor(deg_spectro, 30.0);
    
    // 归一化到 0dB 地板（与 ViSQOL SubtractFloor 一致）
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

/// 在时域对 degraded 信号做 SPL 归一化（与 ViSQOL ScaleToMatchSoundPressureLevel 一致）
pub fn scale_to_match_spl(reference: &[f64], degraded: &mut [f64]) {
    let scale = compute_spl_scale_factor(reference, degraded);
    for val in degraded.iter_mut() {
        *val *= scale;
    }
}
