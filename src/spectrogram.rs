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

/// 将可能不等长（ragged）的 patch 规整为统一列数的矩形矩阵。
/// 取所有频段中最短的帧长作为统一列数，避免后续按 `m[0].len()` 遍历时越界。
fn normalize_patch(patch: &[Vec<f64>]) -> Vec<Vec<f64>> {
    if patch.is_empty() {
        return Vec::new();
    }
    let min_cols = patch.iter().map(|row| row.len()).min().unwrap_or(0);
    patch
        .iter()
        .map(|row| row[..min_cols].to_vec())
        .collect()
}

/// 应用 3x3 高斯卷积（valid 模式，行列都裁剪）。
/// 当行数或列数不足 3 时，退化为逐点拷贝（identity），
/// 以保证小 patch（如帧宽 < 3）也能安全计算逐点 SSIM。
fn conv2d_valid(input: &[Vec<f64>], window: &[[f64; 3]; 3]) -> Vec<Vec<f64>> {
    let rows = input.len();
    if rows == 0 {
        return Vec::new();
    }
    let cols = input[0].len();

    // 尺寸不足卷积核：退化为原值（逐点），保持矩形。
    if rows < 3 || cols < 3 {
        return input.iter().map(|row| row[..cols].to_vec()).collect();
    }

    let out_rows = rows - 2;
    let out_cols = cols - 2;
    let mut output = Vec::with_capacity(out_rows);
    for i in 0..out_rows {
        let mut row = Vec::with_capacity(out_cols);
        for j in 0..out_cols {
            let mut sum = 0.0;
            for (di, wrow) in window.iter().enumerate() {
                for (dj, &w) in wrow.iter().enumerate() {
                    sum += input[i + di][j + dj] * w;
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

/// 通用的窗口统计（方差/协方差）。
/// 当输入做了 3x3 valid 卷积（行列各减 2），逐窗对齐到原始矩阵的 [i, i+3) x [j, j+3)；
/// 当退化为逐点（identity）时，窗口为单点。所有索引均做越界保护。
fn windowed_stat<F>(
    spec: &[Vec<f64>],
    mean: &[Vec<f64>],
    mut accum: F,
) -> Vec<Vec<f64>>
where
    F: FnMut(usize, usize, usize, usize) -> f64,
{
    let mean_rows = mean.len();
    if mean_rows == 0 {
        return Vec::new();
    }
    let mean_cols = mean[0].len();
    let spec_rows = spec.len();
    let spec_cols = if spec_rows > 0 { spec[0].len() } else { 0 };

    // 判断是否为 valid 卷积输出（行列各减 2）。
    let is_valid = spec_rows >= 3 && spec_cols >= 3 && mean_rows == spec_rows - 2;
    let win = if is_valid { 3 } else { 1 };

    let mut result = Vec::with_capacity(mean_rows);
    for i in 0..mean_rows {
        let mut row = Vec::with_capacity(mean_cols);
        for j in 0..mean_cols {
            let mut sum = 0.0;
            let mut count = 0.0_f64;
            for di in 0..win {
                for dj in 0..win {
                    let ni = i + di;
                    let nj = j + dj;
                    if ni < spec_rows && nj < spec_cols {
                        sum += accum(ni, nj, i, j);
                        count += 1.0;
                    }
                }
            }
            row.push(sum / count.max(1.0));
        }
        result.push(row);
    }
    result
}

/// 计算局部方差
fn compute_local_variance(spectrogram: &[Vec<f64>], mean: &[Vec<f64>]) -> Vec<Vec<f64>> {
    windowed_stat(spectrogram, mean, |ni, nj, i, j| {
        let diff = spectrogram[ni][nj] - mean[i][j];
        diff * diff
    })
}

/// 计算局部协方差
fn compute_local_covariance(
    spec1: &[Vec<f64>],
    mean1: &[Vec<f64>],
    spec2: &[Vec<f64>],
    mean2: &[Vec<f64>],
) -> Vec<Vec<f64>> {
    windowed_stat(spec1, mean1, |ni, nj, i, j| {
        let diff1 = spec1[ni][nj] - mean1[i][j];
        let diff2 = spec2[ni][nj] - mean2[i][j];
        diff1 * diff2
    })
}

/// 计算单个 patch 的 NSIM 相似度
pub fn compute_patch_nsim(
    ref_patch_in: &[Vec<f64>],
    deg_patch_in: &[Vec<f64>],
) -> NsimPatchResult {
    // 先规整为统一列数的矩形矩阵，消除 ragged 行导致的越界。
    let ref_patch = normalize_patch(ref_patch_in);
    let deg_patch = normalize_patch(deg_patch_in);

    let num_bands = ref_patch.len().min(deg_patch.len());
    let num_frames = if num_bands > 0 {
        ref_patch[0].len().min(deg_patch[0].len())
    } else {
        0
    };

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

    let ref_mean = compute_local_mean(&ref_patch);
    let deg_mean = compute_local_mean(&deg_patch);

    let ref_var = compute_local_variance(&ref_patch, &ref_mean);
    let deg_var = compute_local_variance(&deg_patch, &deg_mean);

    let cov = compute_local_covariance(&ref_patch, &ref_mean, &deg_patch, &deg_mean);

    let mut intensity_per_band = vec![0.0; num_bands];
    let mut structure_per_band = vec![0.0; num_bands];
    let mut stddev_per_band = vec![0.0; num_bands];
    let mut energy_per_band = vec![0.0; num_bands];
    let mut valid_count_per_band = vec![0; num_bands];

    // 统一以各统计矩阵的最小尺寸为准遍历，杜绝越界。
    let effective_rows = ref_mean
        .len()
        .min(deg_mean.len())
        .min(ref_var.len())
        .min(deg_var.len())
        .min(cov.len())
        .min(num_bands);

    for i in 0..effective_rows {
        let effective_cols = ref_mean[i]
            .len()
            .min(deg_mean[i].len())
            .min(ref_var[i].len())
            .min(deg_var[i].len())
            .min(cov[i].len());

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
        } else {
            // 该频段没有有效窗口（patch 过小），按完全相似处理避免误拉低评分。
            intensity_per_band[i] = 1.0;
            structure_per_band[i] = 1.0;
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

/// 从完整频谱图中提取 patch（保证返回矩形矩阵：不足部分补零）
fn extract_patch(
    spectrogram: &[Vec<f64>],
    start_band: usize,
    num_bands: usize,
    start_frame: usize,
    num_frames: usize,
) -> Vec<Vec<f64>> {
    let mut patch = Vec::with_capacity(num_bands);

    for b in start_band..(start_band + num_bands) {
        let mut band_data = Vec::with_capacity(num_frames);
        for f in start_frame..(start_frame + num_frames) {
            let v = if b < spectrogram.len() && f < spectrogram[b].len() {
                spectrogram[b][f]
            } else {
                0.0
            };
            band_data.push(v);
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
    let num_deg_frames = if num_deg_bands > 0 { deg_spectro[0].len() } else { 0 };
    let num_frames = num_ref_frames.min(num_deg_frames);

    let mut results = Vec::new();

    if patch_size_bands == 0 || patch_size_frames == 0 || hop_bands == 0 || hop_frames == 0 {
        return results;
    }

    let max_bands = num_ref_bands.min(num_deg_bands);

    let mut band_pos = 0;
    while band_pos + patch_size_bands <= max_bands {
        let mut frame_pos = 0;
        while frame_pos + patch_size_frames <= num_frames {
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
            result.start_time_s = frame_pos as f64 * 0.04; // 近似
            result.end_time_s = (frame_pos + patch_size_frames) as f64 * 0.04;

            results.push(result);

            frame_pos += hop_frames;
        }
        band_pos += hop_bands;
    }

    results
}
