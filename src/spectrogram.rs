//! 频谱图与 NSIM 相似度计算模块
//! 
//! 实现 ViSQOL 的类 SSIM 相似度算法（NSIM）

use serde::Serialize;

/// 单个 patch 的 NSIM 相似度结果
#[derive(Debug, Clone, Serialize)]
pub struct NsimPatchResult {
    /// 整体 NSIM 相似度
    pub similarity: f64,
    /// 各频段的 intensity 相似度
    pub intensity: Vec<f64>,
    /// 各频段的 structure 相似度
    pub structure: Vec<f64>,
    /// 各频段的降质能量
    pub degraded_energy: Vec<f64>,
    /// 各频段内帧间标准差
    pub freq_band_stddevs: Vec<f64>,
    /// Patch 起始时间（秒）
    pub start_time_s: f64,
    /// Patch 结束时间（秒）
    pub end_time_s: f64,
}

/// 2D 高斯窗口（3x3）
const GAUSSIAN_WINDOW: [[f64; 3]; 3] = [
    [0.0113033910173052, 0.0838251475442633, 0.0113033910173052],
    [0.0838251475442633, 0.619485845753726, 0.0838251475442633],
    [0.0113033910173052, 0.0838251475442633, 0.0113033910173052],
];

/// SSIM/NSIM 常量
const C1: f64 = 0.0001;
const C3: f64 = 0.000045;

/// 应用 2D 卷积（有效模式）
fn conv2d_valid(input: &[Vec<f64>], window: &[[f64; 3]; 3]) -> Vec<Vec<f64>> {
    let rows = input.len();
    if rows < 3 { return input.to_vec(); }
    let cols = input[0].len();
    if cols < 3 { return input.to_vec(); }
    
    let mut output = Vec::with_capacity(rows);
    for i in 0..rows {
        let mut row = Vec::with_capacity(cols.saturating_sub(2));
        for j in 0..cols.saturating_sub(2) {
            let mut sum = 0.0;
            for di in 0..3 {
                for dj in 0..3 {
                    let ni = i + di;
                    let nj = j + dj;
                    if ni < rows && nj < cols {
                        sum += input[ni][nj] * window[di][dj];
                    }
                }
            }
            row.push(sum);
        }
        output.push(row);
    }
    output
}

/// 计算局部均值
fn compute_local_mean(spectrogram: &[Vec<f64>]) -> Vec<Vec<f64>> {
    conv2d_valid(spectrogram, &GAUSSIAN_WINDOW)
}

/// 计算局部方差
fn compute_local_variance(spectrogram: &[Vec<f64>], mean: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let rows = spectrogram.len();
    if rows == 0 { return vec![]; }
    let cols = spectrogram[0].len();
    
    let mut variance = Vec::with_capacity(rows);
    for i in 0..rows {
        let mut row = Vec::with_capacity(cols.saturating_sub(2));
        for j in 0..cols.saturating_sub(2) {
            let mut sum_sq = 0.0;
            let mut count: f64 = 0.0;
            for di in 0..3 {
                for dj in 0..3 {
                    let ni = i + di;
                    let nj = j + dj;
                    if ni < rows && nj < cols {
                        let diff = spectrogram[ni][nj] - mean[i][j];
                        sum_sq += diff * diff;
                        count += 1.0;
                    }
                }
            }
            row.push(sum_sq / count.max(1.0));
        }
        variance.push(row);
    }
    variance
}

/// 计算局部协方差
fn compute_local_covariance(
    spec1: &[Vec<f64>], 
    mean1: &[Vec<f64>], 
    spec2: &[Vec<f64>], 
    mean2: &[Vec<f64>]
) -> Vec<Vec<f64>> {
    let rows = spec1.len();
    if rows == 0 { return vec![]; }
    let cols = spec1[0].len();
    
    let mut covariance = Vec::with_capacity(rows);
    for i in 0..rows {
        let mut row = Vec::with_capacity(cols.saturating_sub(2));
        for j in 0..cols.saturating_sub(2) {
            let mut sum_prod = 0.0;
            let mut count: f64 = 0.0;
            for di in 0..3 {
                for dj in 0..3 {
                    let ni = i + di;
                    let nj = j + dj;
                    if ni < rows && nj < cols {
                        let diff1 = spec1[ni][nj] - mean1[i][j];
                        let diff2 = spec2[ni][nj] - mean2[i][j];
                        sum_prod += diff1 * diff2;
                        count += 1.0;
                    }
                }
            }
            row.push(sum_prod / count.max(1.0));
        }
        covariance.push(row);
    }
    covariance
}

/// 计算单个 patch 的 NSIM 相似度
pub fn compute_patch_nsim(
    ref_patch: &[Vec<f64>],
    deg_patch: &[Vec<f64>],
) -> NsimPatchResult {
    let num_bands = ref_patch.len();
    let num_frames = if num_bands > 0 { ref_patch[0].len() } else { 0 };
    
    if num_bands == 0 || num_frames == 0 {
        return NsimPatchResult {
            similarity: 0.0,
            intensity: vec![],
            structure: vec![],
            degraded_energy: vec![],
            freq_band_stddevs: vec![],
            start_time_s: 0.0,
            end_time_s: 0.0,
        };
    }
    
    let ref_mean = compute_local_mean(ref_patch);
    let deg_mean = compute_local_mean(deg_patch);
    
    let ref_var = compute_local_variance(ref_patch, &ref_mean);
    let deg_var = compute_local_variance(deg_patch, &deg_mean);
    
    let cov = compute_local_covariance(ref_patch, &ref_mean, deg_patch, &deg_mean);
    
    let mut intensity_per_band = vec![0.0; num_bands];
    let mut structure_per_band = vec![0.0; num_bands];
    let mut stddev_per_band = vec![0.0; num_bands];
    let mut energy_per_band = vec![0.0; num_bands];
    let mut valid_count_per_band = vec![0; num_bands];
    
    let effective_rows = ref_mean.len();
    let effective_cols = if effective_rows > 0 { ref_mean[0].len() } else { 0 };
    
    for i in 0..effective_rows.min(num_bands) {
        for j in 0..effective_cols {
            let mu_r = ref_mean[i][j];
            let mu_d = deg_mean[i][j];
            let sigma_r_sq = ref_var[i][j];
            let sigma_d_sq = deg_var[i][j];
            let sigma_r_d = cov[i][j];
            
            let intensity_num = 2.0 * mu_r * mu_d + C1;
            let intensity_denom = mu_r * mu_r + mu_d * mu_d + C1;
            let intensity = if intensity_denom > 0.0 {
                intensity_num / intensity_denom
            } else {
                1.0
            };
            
            let structure_num = sigma_r_d + C3;
            let structure_denom = (sigma_r_sq * sigma_d_sq).sqrt() + C3;
            let structure = if structure_denom > 0.0 {
                structure_num / structure_denom
            } else {
                1.0
            };
            
            intensity_per_band[i] += intensity;
            structure_per_band[i] += structure;
            stddev_per_band[i] += (sigma_r_sq + sigma_d_sq).sqrt() / 2.0;
            valid_count_per_band[i] += 1;
        }
    }
    
    for i in 0..num_bands {
        let count = valid_count_per_band[i] as f64;
        if count > 0.0 {
            intensity_per_band[i] /= count;
            structure_per_band[i] /= count;
            stddev_per_band[i] /= count;
        }
    }
    
    // 降质能量
    for i in 0..num_bands {
        let ref_energy: f64 = ref_patch[i].iter().map(|&x| x * x).sum();
        let deg_energy: f64 = deg_patch[i].iter().map(|&x| x * x).sum();
        let ratio = if ref_energy > 1e-10 {
            (deg_energy / ref_energy).min(10.0)
        } else {
            1.0
        };
        energy_per_band[i] = ratio;
    }
    
    let mut total_sim = 0.0;
    for i in 0..num_bands {
        total_sim += intensity_per_band[i] * structure_per_band[i];
    }
    let similarity = if num_bands > 0 {
        total_sim / num_bands as f64
    } else {
        0.0
    };
    
    NsimPatchResult {
        similarity,
        intensity: intensity_per_band,
        structure: structure_per_band,
        degraded_energy: energy_per_band,
        freq_band_stddevs: stddev_per_band,
        start_time_s: 0.0,
        end_time_s: 0.0,
    }
}

/// 从完整频谱图中提取 patch
fn extract_patch(
    spectrogram: &[Vec<f64>],
    start_band: usize,
    num_bands: usize,
    start_frame: usize,
    num_frames: usize,
) -> Vec<Vec<f64>> {
    let mut patch = Vec::with_capacity(num_bands);
    
    for b in start_band..(start_band + num_bands).min(spectrogram.len()) {
        let mut band_data = Vec::with_capacity(num_frames);
        for f in start_frame..(start_frame + num_frames).min(spectrogram[b].len()) {
            band_data.push(spectrogram[b][f]);
        }
        patch.push(band_data);
    }
    
    patch
}

/// 评估多个 patch 的 NSIM 相似度
pub fn evaluate_patch_similarities(
    ref_spectro: &[Vec<f64>],
    deg_spectro: &[Vec<f64>],
    patch_size_bands: usize,
    patch_size_frames: usize,
    hop_bands: usize,
    hop_frames: usize,
) -> Vec<NsimPatchResult> {
    let num_ref_bands = ref_spectro.len();
    let num_deg_bands = deg_spectro.len();
    let num_ref_frames = if num_ref_bands > 0 { ref_spectro[0].len() } else { 0 };
    
    let mut results = Vec::new();
    
    let mut band_pos = 0;
    while band_pos + patch_size_bands <= num_ref_bands.min(num_deg_bands) {
        let mut frame_pos = 0;
        while frame_pos + patch_size_frames <= num_ref_frames {
            let ref_patch = extract_patch(
                ref_spectro,
                band_pos,
                patch_size_bands,
                frame_pos,
                patch_size_frames,
            );
            let deg_patch = extract_patch(
                deg_spectro,
                band_pos,
                patch_size_bands,
                frame_pos,
                patch_size_frames,
            );
            
            let mut result = compute_patch_nsim(&ref_patch, &deg_patch);
            result.start_time_s = frame_pos as f64 * 0.04;  // 近似
            result.end_time_s = (frame_pos + patch_size_frames) as f64 * 0.04;
            
            results.push(result);
            
            frame_pos += hop_frames;
        }
        band_pos += hop_bands;
    }
    
    results
}
