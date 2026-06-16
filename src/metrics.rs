//! 音频异常检测模块
//! 
//! 基于 ITU-T G.1021 / RTC 行业标准，将异常细分为三维：
//! - 时域中断 (Audio Dropout)：录制信号能量断崖式下跌
//! - 时轴漂移 (Time Warping)：多段对齐间距不一致
//! - 频谱损伤 (Spectral Artifacts)：频域结构被破坏但时域能量正常

/// 音频异常检测报告
#[derive(Debug, Clone, serde::Serialize)]
pub struct AudioAnomalyReport {
    /// 是否存在任何异常
    pub has_anomaly: bool,
    
    /// 时域中断（异常静音/丢包）事件列表
    pub dropouts: Vec<DropoutEvent>,
    
    /// 时域中断总时长 (ms)
    pub dropout_duration_ms: f64,
    
    /// 时轴漂移事件列表
    pub warpings: Vec<WarpingEvent>,
    
    /// 时轴漂移总时长 (ms)
    pub warping_duration_ms: f64,
    
    /// 频谱损伤严重程度 (0.0=无损伤, 1.0=极度损伤)
    pub spectral_artifacts_score: f64,
    
    /// 频谱损伤事件列表（低相似度时间段）
    pub spectral_artifacts: Vec<SpectralArtifactEvent>,
}

/// 时域中断事件（异常静音/丢包）
#[derive(Debug, Clone, serde::Serialize)]
pub struct DropoutEvent {
    /// 中断起始时间 (秒)
    pub start_time_s: f64,
    /// 中断结束时间 (秒)
    pub end_time_s: f64,
    /// 中断持续时长 (毫秒)
    pub duration_ms: f64,
    /// 参考信号在该时间段的 RMS 能量
    pub ref_rms: f64,
    /// 录制信号在该时间段的 RMS 能量
    pub deg_rms: f64,
    /// 能量衰减比例 (deg_rms / ref_rms)，0=完全中断
    pub attenuation_ratio: f64,
}

/// 时轴漂移事件（对齐间距不一致）
#[derive(Debug, Clone, serde::Serialize)]
pub struct WarpingEvent {
    /// 涉及的段索引（前一段）
    pub segment_before: usize,
    /// 涉及的段索引（后一段）
    pub segment_after: usize,
    /// 参考音频的段间间距 (ms)
    pub ref_gap_ms: f64,
    /// 录制音频的段间间距 (ms)
    pub deg_gap_ms: f64,
    /// 漂移时长 = deg_gap_ms - ref_gap_ms (ms)，正值表示拉伸
    pub drift_ms: f64,
    /// 漂移比例 = drift_ms / ref_gap_ms
    pub drift_ratio: f64,
}

/// 频谱损伤事件
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpectralArtifactEvent {
    /// 对应的段索引
    pub segment_index: usize,
    /// Patch 索引
    pub patch_index: usize,
    /// Patch 相似度 (0-1)
    pub patch_similarity: f64,
    /// 参考音频时间段 (起始秒)
    pub ref_start_s: f64,
    /// 参考音频时间段 (结束秒)
    pub ref_end_s: f64,
}

/// SNR（信噪比）计算结果（保留兼容）
#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)]
pub struct SnrResult {
    pub snr_db: f64,
    pub ref_energy: f64,
    pub noise_energy: f64,
}

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

// ============================================================
// 维度一：时域中断检测 (Audio Dropout / Interruption)
// ============================================================

/// 时域中断检测参数
pub struct DropoutDetectorConfig {
    /// 静音阈值（RMS 低于此值判定为静音）
    pub silence_threshold: f64,
    /// 最短中断持续时间 (ms)，低于此不判定为中断
    pub min_duration_ms: f64,
    /// 相对于参考信号的能量衰减比阈值
    pub attenuation_threshold: f64,
    /// 分帧大小（采样点数）
    pub frame_size: usize,
    /// 帧移（采样点数）
    pub hop_size: usize,
}

impl Default for DropoutDetectorConfig {
    fn default() -> Self {
        Self {
            silence_threshold: 0.005,
            min_duration_ms: 20.0,
            attenuation_threshold: 0.05,
            frame_size: 320,
            hop_size: 160,
        }
    }
}

/// 计算一段采样的 RMS 能量
fn compute_rms(samples: &[f64]) -> f64 {
    if samples.is_empty() { return 0.0; }
    (samples.iter().map(|x| x * x).sum::<f64>() / samples.len() as f64).sqrt()
}

/// 维度一：时域中断检测
pub fn detect_dropouts(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    config: &DropoutDetectorConfig,
) -> Vec<DropoutEvent> {
    let frame_size = config.frame_size.min(reference.len());
    let hop_size = config.hop_size.max(1);
    let min_samples = (sample_rate as f64 * config.min_duration_ms / 1000.0) as usize;
    
    let mut events = Vec::new();
    let mut in_dropout = false;
    let mut dropout_start_frame = 0usize;
    let mut dropout_ref_rms_acc = 0.0f64;
    let mut dropout_deg_rms_acc = 0.0f64;
    let mut dropout_frame_count = 0usize;
    
    let mut frame_idx = 0;
    while frame_idx + frame_size <= reference.len() && frame_idx + frame_size <= degraded.len() {
        let ref_frame = &reference[frame_idx..frame_idx + frame_size];
        let deg_frame = &degraded[frame_idx..frame_idx + frame_size];
        
        let ref_rms = compute_rms(ref_frame);
        let deg_rms = compute_rms(deg_frame);
        
        let ref_has_sound = ref_rms > config.silence_threshold * 2.0;
        let deg_silent = deg_rms < config.silence_threshold;
        let energy_attenuation = if ref_rms > 0.0 {
            deg_rms / ref_rms < config.attenuation_threshold
        } else {
            false
        };
        
        let is_dropout = ref_has_sound && (deg_silent || energy_attenuation);
        
        if is_dropout {
            if !in_dropout {
                in_dropout = true;
                dropout_start_frame = frame_idx;
                dropout_ref_rms_acc = 0.0;
                dropout_deg_rms_acc = 0.0;
                dropout_frame_count = 0;
            }
            dropout_ref_rms_acc += ref_rms;
            dropout_deg_rms_acc += deg_rms;
            dropout_frame_count += 1;
        } else {
            if in_dropout {
                let duration_samples = frame_idx - dropout_start_frame;
                if duration_samples >= min_samples {
                    let start_time = dropout_start_frame as f64 / sample_rate as f64;
                    let end_time = frame_idx as f64 / sample_rate as f64;
                    let avg_ref_rms = if dropout_frame_count > 0 { dropout_ref_rms_acc / dropout_frame_count as f64 } else { 0.0 };
                    let avg_deg_rms = if dropout_frame_count > 0 { dropout_deg_rms_acc / dropout_frame_count as f64 } else { 0.0 };
                    let attenuation = if avg_ref_rms > 0.0 { avg_deg_rms / avg_ref_rms } else { 0.0 };
                    events.push(DropoutEvent {
                        start_time_s: start_time,
                        end_time_s: end_time,
                        duration_ms: (end_time - start_time) * 1000.0,
                        ref_rms: avg_ref_rms,
                        deg_rms: avg_deg_rms,
                        attenuation_ratio: attenuation,
                    });
                }
                in_dropout = false;
            }
        }
        
        frame_idx += hop_size;
    }
    
    if in_dropout {
        let duration_samples = frame_idx.saturating_sub(dropout_start_frame);
        if duration_samples >= min_samples {
            let end_time = frame_idx as f64 / sample_rate as f64;
            let start_time = dropout_start_frame as f64 / sample_rate as f64;
            let avg_ref_rms = if dropout_frame_count > 0 { dropout_ref_rms_acc / dropout_frame_count as f64 } else { 0.0 };
            let avg_deg_rms = if dropout_frame_count > 0 { dropout_deg_rms_acc / dropout_frame_count as f64 } else { 0.0 };
            let attenuation = if avg_ref_rms > 0.0 { avg_deg_rms / avg_ref_rms } else { 0.0 };
            events.push(DropoutEvent {
                start_time_s: start_time,
                end_time_s: end_time,
                duration_ms: (end_time - start_time) * 1000.0,
                ref_rms: avg_ref_rms,
                deg_rms: avg_deg_rms,
                attenuation_ratio: attenuation,
            });
        }
    }
    
    events
}

// ============================================================
// 维度二：时轴漂移检测 (Time Warping)
// ============================================================

/// 维度二：时轴漂移检测
pub fn detect_warpings(
    alignment_offsets: &[f64],
    ref_duration: f64,
    warping_threshold: f64,
) -> Vec<WarpingEvent> {
    if alignment_offsets.len() < 2 {
        return vec![];
    }
    
    let mut events = Vec::new();
    
    for i in 0..alignment_offsets.len() - 1 {
        let deg_gap = alignment_offsets[i + 1] - alignment_offsets[i];
        let ref_gap = ref_duration;
        
        let drift = deg_gap - ref_gap;
        let drift_ratio = if ref_gap > 0.0 { drift / ref_gap } else { 0.0 };
        
        if drift_ratio.abs() > warping_threshold {
            events.push(WarpingEvent {
                segment_before: i,
                segment_after: i + 1,
                ref_gap_ms: ref_gap * 1000.0,
                deg_gap_ms: deg_gap * 1000.0,
                drift_ms: drift * 1000.0,
                drift_ratio,
            });
        }
    }
    
    events
}

// ============================================================
// 维度三：频谱损伤检测 (Spectral Artifacts)
// ============================================================

/// 维度三：频谱损伤检测
pub fn detect_spectral_artifacts(
    patch_sims: &[Vec<crate::visqol::PatchSimilarityResult>],
    artifact_threshold: f64,
) -> (f64, Vec<SpectralArtifactEvent>) {
    if patch_sims.is_empty() {
        return (0.0, vec![]);
    }
    
    let mut all_artifacts = Vec::new();
    let mut total_low_count = 0usize;
    let mut total_patch_count = 0usize;
    
    for (seg_idx, patches) in patch_sims.iter().enumerate() {
        for (patch_idx, patch) in patches.iter().enumerate() {
            total_patch_count += 1;
            if patch.similarity < artifact_threshold {
                total_low_count += 1;
                all_artifacts.push(SpectralArtifactEvent {
                    segment_index: seg_idx,
                    patch_index: patch_idx,
                    patch_similarity: patch.similarity,
                    ref_start_s: patch.ref_patch_start_time,
                    ref_end_s: patch.ref_patch_end_time,
                });
            }
        }
    }
    
    let score = if total_patch_count > 0 {
        total_low_count as f64 / total_patch_count as f64
    } else {
        0.0
    };
    
    (score, all_artifacts)
}

// ============================================================
// 综合异常检测入口
// ============================================================

/// 综合异常检测参数
pub struct AnomalyDetectConfig {
    pub dropout_config: DropoutDetectorConfig,
    pub warping_threshold: f64,
    pub artifact_threshold: f64,
}

impl Default for AnomalyDetectConfig {
    fn default() -> Self {
        Self {
            dropout_config: DropoutDetectorConfig::default(),
            warping_threshold: 0.1,
            artifact_threshold: 0.4,
        }
    }
}

/// 综合音频异常检测
pub fn detect_anomalies(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    alignment_offsets_s: &[f64],
    ref_duration: f64,
    patch_sims: &[Vec<crate::visqol::PatchSimilarityResult>],
    config: &AnomalyDetectConfig,
) -> AudioAnomalyReport {
    // 维度一：时域中断
    let dropouts = detect_dropouts(reference, degraded, sample_rate, &config.dropout_config);
    let dropout_duration_ms: f64 = dropouts.iter().map(|e| e.duration_ms).sum();
    
    // 维度二：时轴漂移
    let warpings = detect_warpings(alignment_offsets_s, ref_duration, config.warping_threshold);
    let warping_duration_ms: f64 = warpings.iter().map(|w| w.drift_ms.abs()).sum();
    
    // 维度三：频谱损伤
    let (spectral_artifacts_score, spectral_artifacts) = detect_spectral_artifacts(patch_sims, config.artifact_threshold);
    
    let has_anomaly = !dropouts.is_empty() || !warpings.is_empty() || spectral_artifacts_score > 0.1;
    
    AudioAnomalyReport {
        has_anomaly,
        dropouts,
        dropout_duration_ms,
        warpings,
        warping_duration_ms,
        spectral_artifacts_score,
        spectral_artifacts,
    }
}
