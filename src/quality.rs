//! 音频质量评估模块（与 ViSQOL 等效）

use serde::Serialize;

pub use crate::gammatone::{build_spectrogram, preprocess_spectrograms, scale_to_match_spl};
pub use crate::spectrogram::{evaluate_patch_similarities, NsimPatchResult};

#[derive(Debug, Clone, Serialize)]
pub struct QualityResult {
    pub moslqo: f64,
    pub vnsim: f64,
    pub fvnsim: Vec<f64>,
    pub fvnsim10: Vec<f64>,
    pub fstdnsim: Vec<f64>,
    pub fvdegenergy: Vec<f64>,
    pub center_freq_bands: Vec<f64>,
    pub patch_sims: Vec<PatchSimilarityResult>,
    pub alignment_lag_s: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PatchSimilarityResult {
    pub similarity: f64,
    pub freq_band_means: Vec<f64>,
    pub ref_patch_start_time: f64,
    pub ref_patch_end_time: f64,
    pub deg_patch_start_time: f64,
    pub deg_patch_end_time: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosisResult {
    pub quality_rating: String,
    pub mos_score: f64,
    pub background_noise_detected: bool,
    pub high_freq_loss_detected: bool,
    pub intermittent_artifacts_detected: bool,
    pub low_freq_similarity: f64,
    pub high_freq_similarity: f64,
    pub worst_patch: Option<(f64, f64, f64)>,
    pub freq_stability: f64,
}

const FRAME_DURATION_MS: f64 = 80.0;
const FRAME_OVERLAP_RATIO: f64 = 0.75;
const NUM_BANDS: usize = 24;

pub fn evaluate_quality(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
) -> QualityResult {
    let frame_size = (sample_rate as f64 * FRAME_DURATION_MS / 1000.0) as usize;
    let hop_size = (frame_size as f64 * FRAME_OVERLAP_RATIO) as usize;
    
    // 与 ViSQOL 一致：在时域对 degraded 信号做 SPL 归一化
    let mut deg_normalized: Vec<f64> = degraded.to_vec();
    scale_to_match_spl(reference, &mut deg_normalized);
    
    let (mut ref_spectro, center_freqs) = build_spectrogram(
        reference, sample_rate, frame_size, hop_size, NUM_BANDS, 
    );
    
    let (mut deg_spectro, _) = build_spectrogram(
        &deg_normalized, sample_rate, frame_size, hop_size, NUM_BANDS, 
    );
    
    // 检查数据有效性
    if ref_spectro.is_empty() || deg_spectro.is_empty() || 
       ref_spectro[0].len() < 3 || deg_spectro[0].len() < 3 {
        return QualityResult {
            moslqo: 1.0, vnsim: 0.0,
            fvnsim: vec![0.0; NUM_BANDS],
            fvnsim10: vec![0.0; NUM_BANDS],
            fstdnsim: vec![0.0; NUM_BANDS],
            fvdegenergy: vec![1.0; NUM_BANDS],
            center_freq_bands: center_freqs,
            patch_sims: vec![],
            alignment_lag_s: 0.0,
        };
    }
    
    // 预处理：dB 转换 + 噪声门限 + 归一化
    preprocess_spectrograms(&mut ref_spectro, &mut deg_spectro, reference, degraded);
    
    // patch 参数（与 ViSQOL 一致）
    let patch_size_bands = NUM_BANDS;
    let max_frames = ref_spectro[0].len().min(deg_spectro[0].len());
    let patch_size_frames = if max_frames > 5 { 8 } else { max_frames.saturating_sub(2) }.max(3);
    let hop_bands = NUM_BANDS;
    let hop_frames = 1;
    
    let nsim_results = evaluate_patch_similarities(
        &ref_spectro, &deg_spectro,
        patch_size_bands, patch_size_frames, hop_bands, hop_frames,
    );
    
    // 计算各频段统计量（与 ViSQOL visqol.cc 一致）
    let num_bands = NUM_BANDS;
    let fvnsim = compute_band_means(&nsim_results, num_bands);
    let fvnsim10 = compute_band_quantile(&nsim_results, num_bands, 0.10);
    let fstdnsim = compute_band_pooled_stddevs(&nsim_results, num_bands);
    let fvdegenergy = compute_band_degraded_energy(&nsim_results, num_bands);
    
    // 全局 NSIM（所有频段的均值）
    let vnsim = if !nsim_results.is_empty() {
        nsim_results.iter().map(|r| r.similarity).sum::<f64>() / nsim_results.len() as f64
    } else { 0.0 };
    
    // MOS 预测（使用 ViSQOL 风格的 SVR 近似）
    let moslqo = predict_mos(&fvnsim, &fvnsim10, &fstdnsim, &fvdegenergy, vnsim);
    
    // 构建 patch 结果
    let patch_sims: Vec<PatchSimilarityResult> = nsim_results.iter().map(|r| {
        PatchSimilarityResult {
            similarity: r.similarity,
            freq_band_means: r.freq_band_means.clone(),
            ref_patch_start_time: r.start_time_s,
            ref_patch_end_time: r.end_time_s,
            deg_patch_start_time: r.start_time_s,
            deg_patch_end_time: r.end_time_s,
        }
    }).collect();
    
    QualityResult {
        moslqo, vnsim, fvnsim, fvnsim10, fstdnsim, fvdegenergy,
        center_freq_bands: center_freqs, patch_sims, alignment_lag_s: 0.0,
    }
}

/// 计算各频段的平均 NSIM（与 ViSQOL CalcPerPatchMeanFreqBandMeans 一致）
fn compute_band_means(patches: &[NsimPatchResult], num_bands: usize) -> Vec<f64> {
    if patches.is_empty() { return vec![0.0; num_bands]; }
    
    let mut sums = vec![0.0; num_bands];
    for patch in patches {
        for (i, &mean) in patch.freq_band_means.iter().enumerate() {
            if i < num_bands {
                sums[i] += mean;
            }
        }
    }
    for s in &mut sums { *s /= patches.len() as f64; }
    sums
}

/// 计算各频段的 10 百分位 NSIM（与 ViSQOL CalcPerPatchFreqBandQuantile 一致）
fn compute_band_quantile(patches: &[NsimPatchResult], num_bands: usize, quantile: f64) -> Vec<f64> {
    if patches.is_empty() { return vec![0.0; num_bands]; }
    
    let mut result = Vec::with_capacity(num_bands);
    for band_idx in 0..num_bands {
        let mut band_values: Vec<f64> = patches.iter()
            .filter_map(|p| {
                if band_idx < p.freq_band_means.len() {
                    Some(p.freq_band_means[band_idx])
                } else { None }
            })
            .collect();
        
        if band_values.is_empty() {
            result.push(0.0);
            continue;
        }
        
        band_values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = (((band_values.len() as f64) * quantile).ceil() as usize)
            .min(band_values.len().saturating_sub(1));
        result.push(band_values[idx]);
    }
    result
}

/// 计算各频段的标准差（与 ViSQOL CalcPerPatchMeanFreqBandStdDevs 一致）
fn compute_band_pooled_stddevs(patches: &[NsimPatchResult], num_bands: usize) -> Vec<f64> {
    if patches.is_empty() { return vec![0.0; num_bands]; }
    
    // 计算全局均值
    let mut global_means = vec![0.0; num_bands];
    for patch in patches {
        for (i, &mean) in patch.freq_band_means.iter().enumerate() {
            if i < num_bands {
                global_means[i] += mean;
            }
        }
    }
    for m in &mut global_means { *m /= patches.len() as f64; }
    
    // 计算 pooled 方差（与 ViSQOL 一致）
    let mut frame_counts = vec![0; num_bands];
    let mut contribution = vec![0.0; num_bands];
    
    for patch in patches {
        let secs_in_patch = patch.end_time_s - patch.start_time_s;
        let frame_count = ((secs_in_patch / 0.04).ceil() as usize).max(1);
        
        for (i, &mean) in patch.freq_band_means.iter().enumerate() {
            if i < num_bands && i < patch.freq_band_stddevs.len() {
                let stddev = patch.freq_band_stddevs[i];
                contribution[i] += (frame_count as f64 - 1.0) * stddev * stddev;
                contribution[i] += frame_count as f64 * mean * mean;
                frame_counts[i] += frame_count;
            }
        }
    }
    
    let mut result = Vec::with_capacity(num_bands);
    for i in 0..num_bands {
        if frame_counts[i] > 1 {
            let fc = frame_counts[i] as f64;
            let variance = (contribution[i] - fc * global_means[i] * global_means[i]) / (fc - 1.0);
            result.push(if variance > 0.0 { variance.sqrt() } else { 0.0 });
        } else {
            result.push(0.0);
        }
    }
    result
}

/// 计算各频段的降质能量（与 ViSQOL CalcPerPatchMeanFreqBandDegradedEnergy 一致）
fn compute_band_degraded_energy(patches: &[NsimPatchResult], num_bands: usize) -> Vec<f64> {
    if patches.is_empty() { return vec![1.0; num_bands]; }
    
    let mut energy_sums = vec![0.0; num_bands];
    for patch in patches {
        for (i, &energy) in patch.freq_band_deg_energy.iter().enumerate() {
            if i < num_bands { energy_sums[i] += energy; }
        }
    }
    for e in &mut energy_sums { *e /= patches.len() as f64; }
    energy_sums
}

/// MOS 预测（与 ViSQOL svr_similarity_to_quality_mapper.cc 和 AlterForSimilarityExtremes 一致）
fn predict_mos(fvnsim: &[f64], fvnsim10: &[f64], fstdnsim: &[f64], fvdegenergy: &[f64], vnsim: f64) -> f64 {
    // 基础分数：使用 ViSQOL 风格的非线性映射
    // 近似 SVR 模型：MOS = 1 + 4 / (1 + exp(-k * (nsim - offset))))
    // 使用简化的多项式映射
    let base_mos = 1.0 + vnsim * 3.5 + vnsim.powi(2) * 1.5;
    
    // 低质量惩罚（与 ViSQOL AlterForSimilarityExtremes 一致）
    let mut penalty = 0.0;
    if !fvnsim.is_empty() && !fvnsim10.is_empty() {
        for (mean, p10) in fvnsim.iter().zip(fvnsim10.iter()) {
            if *p10 < *mean * 0.7 {
                penalty += (*mean - *p10) * 0.5;
            }
        }
    }
    
    // 不稳定性惩罚
    for std in fstdnsim.iter().take(5) {
        penalty += std * 0.3;
    }
    
    // 能量偏差惩罚
    for energy in fvdegenergy.iter().take(5) {
        let deviation = (energy - 1.0).abs();
        if deviation > 0.5 {
            penalty += deviation * 0.2;
        }
    }
    
    let mut mos = base_mos - penalty;
    
    // 与 ViSQOL AlterForSimilarityExtremes 一致
    if vnsim < 0.15 {
        mos = 1.0;
    }
    
    mos.clamp(1.0, 5.0)
}

pub fn diagnose(result: &QualityResult) -> DiagnosisResult {
    let quality_rating = match result.moslqo {
        s if s >= 4.5 => "优秀", s if s >= 4.0 => "良好", s if s >= 3.5 => "一般",
        s if s >= 3.0 => "较差", s if s >= 2.0 => "差", _ => "极差",
    }.to_string();
    
    let low_count = (result.fvnsim.len() / 3).max(1);
    let high_count = result.fvnsim.len().saturating_sub(result.fvnsim.len() * 2 / 3);
    let low_freq_similarity = result.fvnsim.iter().take(low_count).sum::<f64>() / low_count as f64;
    let high_freq_similarity = if high_count > 0 {
        result.fvnsim.iter().rev().take(high_count).sum::<f64>() / high_count as f64
    } else { result.vnsim };
    
    let background_noise_detected = result.vnsim < 0.85 && low_freq_similarity < 0.80;
    let high_freq_loss_detected = high_freq_similarity < low_freq_similarity * 0.8 && high_freq_similarity < 0.75;
    let worst_patch = result.patch_sims.iter()
        .min_by(|a, b| a.similarity.partial_cmp(&b.similarity).unwrap_or(std::cmp::Ordering::Equal))
        .map(|p| (p.similarity, p.ref_patch_start_time, p.ref_patch_end_time));
    let avg_sim = if !result.patch_sims.is_empty() {
        result.patch_sims.iter().map(|p| p.similarity).sum::<f64>() / result.patch_sims.len() as f64
    } else { result.vnsim };
    let intermittent_artifacts_detected = worst_patch.map(|(sim, _, _)| sim < avg_sim * 0.7).unwrap_or(false);
    let freq_stability = if !result.fstdnsim.is_empty() {
        result.fstdnsim.iter().sum::<f64>() / result.fstdnsim.len() as f64
    } else { 0.0 };
    
    DiagnosisResult {
        quality_rating, mos_score: result.moslqo,
        background_noise_detected, high_freq_loss_detected, intermittent_artifacts_detected,
        low_freq_similarity, high_freq_similarity, worst_patch, freq_stability,
    }
}
