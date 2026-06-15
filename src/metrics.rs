//! 独立指标计算模块
//! SNR、卡顿检测、幅值分析等纯 Rust 实现的指标

/// SNR（信噪比）计算结果
#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)]
pub struct SnrResult {
    /// 信噪比（dB）
    pub snr_db: f64,
    /// 参考信号能量
    pub ref_energy: f64,
    /// 噪声能量
    pub noise_energy: f64,
}

/// 计算信噪比
/// SNR = 10 * log10(Σref² / Σnoise²)
#[allow(dead_code)]
pub fn compute_snr(reference: &[f64], degraded: &[f64]) -> SnrResult {
    assert_eq!(reference.len(), degraded.len(), "参考和录制音频长度必须一致");
    
    let ref_energy: f64 = reference.iter().map(|x| x * x).sum();
    let noise: Vec<f64> = reference.iter()
        .zip(degraded.iter())
        .map(|(r, d)| d - r)
        .collect();
    let noise_energy: f64 = noise.iter().map(|x| x * x).sum();
    
    let snr_db = if noise_energy > 0.0 && ref_energy > 0.0 {
        10.0 * (ref_energy / noise_energy).log10()
    } else if ref_energy > 0.0 {
        99.0
    } else {
        0.0
    };
    
    SnrResult { snr_db, ref_energy, noise_energy }
}

/// 卡顿/丢包检测结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct DropoutResult {
    pub events: Vec<DropoutEvent>,
    pub count: usize,
    pub total_duration_ms: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DropoutEvent {
    pub start_time_s: f64,
    pub end_time_s: f64,
    pub duration_ms: f64,
}

/// 检测卡顿/丢包
pub fn detect_dropouts(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    silence_threshold: f64,
    min_duration_ms: f64,
    padded_len: usize,
) -> DropoutResult {
    // 有效检测范围：排除补零尾段
    let valid_len = reference.len().saturating_sub(padded_len);
    let min_samples = (sample_rate as f64 * min_duration_ms / 1000.0) as usize;
    let mut events = Vec::new();
    let mut in_dropout = false;
    let mut dropout_start = 0usize;
    
    for (i, (r, d)) in reference.iter().zip(degraded.iter()).enumerate() {
        // 跳过补零尾段，不检测卡顿
        if i >= valid_len {
            break;
        }
        let degraded_silent = d.abs() < silence_threshold;
        let ref_has_sound = r.abs() > silence_threshold * 2.0;
        
        if degraded_silent && ref_has_sound {
            if !in_dropout {
                in_dropout = true;
                dropout_start = i;
            }
        } else {
            if in_dropout {
                let duration = i - dropout_start;
                if duration >= min_samples {
                    let start_time = dropout_start as f64 / sample_rate as f64;
                    let end_time = i as f64 / sample_rate as f64;
                    events.push(DropoutEvent {
                        start_time_s: start_time,
                        end_time_s: end_time,
                        duration_ms: (end_time - start_time) * 1000.0,
                    });
                }
                in_dropout = false;
            }
        }
    }
    
    // 处理末尾可能未关闭的卡顿事件（仅在有效范围内）
    if in_dropout && dropout_start < valid_len {
        let duration = valid_len - dropout_start;
        if duration >= min_samples {
            let start_time = dropout_start as f64 / sample_rate as f64;
            let end_time = valid_len as f64 / sample_rate as f64;
            events.push(DropoutEvent {
                start_time_s: start_time,
                end_time_s: end_time,
                duration_ms: (end_time - start_time) * 1000.0,
            });
        }
    }
    
    let count = events.len();
    let total_duration_ms: f64 = events.iter().map(|e| e.duration_ms).sum();
    
    DropoutResult { events, count, total_duration_ms }
}

/// 幅值统计结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct LevelResult {
    pub rms: f64,
    pub peak: f64,
    pub rms_dbfs: f64,
    pub peak_dbfs: f64,
    pub clipped_samples: usize,
}

/// 计算音频幅值统计
pub fn compute_level_stats(samples: &[f64]) -> LevelResult {
    let rms = (samples.iter().map(|x| x * x).sum::<f64>() / samples.len() as f64).sqrt();
    let peak = samples.iter().map(|x| x.abs()).fold(0.0f64, f64::max);
    let rms_dbfs = if rms > 0.0 { 20.0 * (rms).log10() } else { -99.0 };
    let peak_dbfs = if peak > 0.0 { 20.0 * (peak).log10() } else { -99.0 };
    let clipped_samples = samples.iter().filter(|&&x| x.abs() >= 0.99).count();
    
    LevelResult { rms, peak, rms_dbfs, peak_dbfs, clipped_samples }
}
