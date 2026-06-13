//! 频谱图与 NSIM 相似度计算模块
//! 
//! 实现与 ViSQOL 等效的 NSIM 相似度算法
//! 核心参考: neurogram_similiarity_index_measure.cc

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct NsimPatchResult {
    /// 该 patch 的整体 NSIM 分数（各频段 sim_map 行均值的均值）
    pub similarity: f64,
    /// 各频段的 sim_map 行均值（intensity * structure 后按时间帧取平均）
    pub freq_band_means: Vec<f64>,
    /// 各频段的 sim_map 行标准差
    pub freq_band_stddevs: Vec<f64>,
    /// 各频段的降质信号行均值（deg patch 的行均值）
    pub freq_band_deg_energy: Vec<f64>,
    pub start_time_s: f64,
    pub end_time_s: f64,
}

/// 2D 高斯窗口（与 ViSQOL neurogram_similiarity_index_measure.cc 一致）
const GAUSSIAN_WINDOW: [[f64; 3]; 3] = [
    [0.0113033910173052, 0.0838251475442633, 0.0113033910173052],
    [0.0838251475442633, 0.619485845753726, 0.0838251475442633],
    [0.0113033910173052, 0.0838251475442633, 0.0113033910173052],
];

/// SSIM/NSIM 常量（与 ViSQOL 一致）
/// C1 = (k1 * L)^2, C3 = (k2 * L)^2 / 2, 其中 L=intensity_range=1.0, k1=0.01, k2=0.03
const C1: f64 = 0.0001;      // (0.01 * 1.0)^2
const C3: f64 = 0.00045;     // (0.03 * 1.0)^2 / 2

/// 2D 卷积（与 ViSQOL Convolution2D::Valid2DConvWithBoundary 一致）
fn conv2d_with_boundary(input: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let rows = input.len();
    if rows == 0 { return Vec::new(); }
    let cols = input[0].len();
    
    if rows < 2 || cols < 2 {
        return input.to_vec();
    }
    
    // 添加边界填充（复制边缘，与 ViSQOL AddMatrixBoundary 一致）
    let mut padded = vec![vec![0.0; cols + 2]; rows + 2];
    for i in 0..rows {
        for j in 0..cols {
            padded[i + 1][j + 1] = input[i][j];
        }
        padded[i + 1][0] = input[i][0];
        padded[i + 1][cols + 1] = input[i][cols - 1];
    }
    for j in 0..cols + 2 {
        padded[0][j] = padded[1][j];
        padded[rows + 1][j] = padded[rows][j];
    }
    
    let out_rows = rows;
    let out_cols = cols;
    let mut output = Vec::with_capacity(out_rows);
    
    for i in 0..out_rows {
        let mut row = Vec::with_capacity(out_cols);
        for j in 0..out_cols {
            let mut sum = 0.0;
            for fc in 0..3 {
                for fr in 0..3 {
                    sum += padded[i + fr][j + fc] * GAUSSIAN_WINDOW[fr][fc];
                }
            }
            row.push(sum);
        }
        output.push(row);
    }
    output
}

/// 逐点乘法
fn pointwise_product(a: &[Vec<f64>], b: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let rows = a.len().min(b.len());
    let mut result = Vec::with_capacity(rows);
    for i in 0..rows {
        let cols = a[i].len().min(b[i].len());
        let mut row = Vec::with_capacity(cols);
        for j in 0..cols {
            row.push(a[i][j] * b[i][j]);
        }
        result.push(row);
    }
    result
}

/// 逐点除法
fn pointwise_divide(a: &[Vec<f64>], b: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let rows = a.len().min(b.len());
    let mut result = Vec::with_capacity(rows);
    for i in 0..rows {
        let cols = a[i].len().min(b[i].len());
        let mut row = Vec::with_capacity(cols);
        for j in 0..cols {
            let denom = b[i][j];
            row.push(if denom.abs() > 1e-15 { a[i][j] / denom } else { 1.0 });
        }
        result.push(row);
    }
    result
}

/// 标量乘法
fn scalar_multiply(mat: &[Vec<f64>], scalar: f64) -> Vec<Vec<f64>> {
    mat.iter().map(|row| row.iter().map(|&v| v * scalar).collect()).collect()
}

/// 逐点加标量
fn pointwise_add_scalar(mat: &[Vec<f64>], scalar: f64) -> Vec<Vec<f64>> {
    mat.iter().map(|row| row.iter().map(|&v| v + scalar).collect()).collect()
}

/// 逐点加法再加标量: a + b + scalar
fn pointwise_add(a: &[Vec<f64>], b: &[Vec<f64>], scalar: f64) -> Vec<Vec<f64>> {
    let rows = a.len().min(b.len());
    let mut result = Vec::with_capacity(rows);
    for i in 0..rows {
        let cols = a[i].len().min(b[i].len());
        let mut row = Vec::with_capacity(cols);
        for j in 0..cols {
            row.push(a[i][j] + b[i][j] + scalar);
        }
        result.push(row);
    }
    result
}

/// 矩阵减法（a - b）
fn matrix_subtract(a: &[Vec<f64>], b: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let rows = a.len().min(b.len());
    let mut result = Vec::with_capacity(rows);
    for i in 0..rows {
        let cols = a[i].len().min(b[i].len());
        let mut row = Vec::with_capacity(cols);
        for j in 0..cols {
            row.push(a[i][j] - b[i][j]);
        }
        result.push(row);
    }
    result
}

/// 计算行均值（与 ViSQOL AMatrix::Mean(kDimension::ROW) 一致）
fn row_means(mat: &[Vec<f64>]) -> Vec<f64> {
    mat.iter().map(|row| {
        if row.is_empty() { 0.0 } else { row.iter().sum::<f64>() / row.len() as f64 }
    }).collect()
}

/// 计算行标准差（与 ViSQOL AMatrix::StdDev(kDimension::ROW) 一致）
fn row_stddevs(mat: &[Vec<f64>]) -> Vec<f64> {
    mat.iter().map(|row| {
        if row.is_empty() { return 0.0; }
        let mean = row.iter().sum::<f64>() / row.len() as f64;
        let variance = row.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / row.len() as f64;
        variance.sqrt().max(0.0)
    }).collect()
}

/// 计算单个 patch 的 NSIM 相似度（与 ViSQOL NeurogramSimiliarityIndexMeasure 一致）
pub fn compute_patch_nsim(
    ref_patch: &[Vec<f64>],
    deg_patch: &[Vec<f64>],
) -> NsimPatchResult {
    let num_bands = ref_patch.len().min(deg_patch.len());
    if num_bands == 0 {
        return NsimPatchResult {
            similarity: 0.0, freq_band_means: vec![],
            freq_band_stddevs: vec![], freq_band_deg_energy: vec![],
            start_time_s: 0.0, end_time_s: 0.0,
        };
    }
    
    // 统一列数
    let min_cols = ref_patch[0].len().min(deg_patch[0].len());
    let ref_p: Vec<Vec<f64>> = ref_patch.iter().take(num_bands)
        .map(|row| row[..min_cols].to_vec()).collect();
    let deg_p: Vec<Vec<f64>> = deg_patch.iter().take(num_bands)
        .map(|row| row[..min_cols].to_vec()).collect();
    
    if min_cols == 0 {
        return NsimPatchResult {
            similarity: 0.0, freq_band_means: vec![],
            freq_band_stddevs: vec![], freq_band_deg_energy: vec![],
            start_time_s: 0.0, end_time_s: 0.0,
        };
    }
    
    // 与 ViSQOL NeurogramSimiliarityIndexMeasure::MeasurePatchSimilarity 一致
    // mu_r = conv2d(ref_patch, window)
    let mu_r = conv2d_with_boundary(&ref_p);
    let mu_d = conv2d_with_boundary(&deg_p);
    
    // ref_mu_sq = mu_r .* mu_r
    let ref_mu_sq = pointwise_product(&mu_r, &mu_r);
    let deg_mu_sq = pointwise_product(&mu_d, &mu_d);
    let mu_r_mu_d = pointwise_product(&mu_r, &mu_d);
    
    // ref_neuro_sq = ref_patch .* ref_patch
    let ref_neuro_sq = pointwise_product(&ref_p, &ref_p);
    let deg_neuro_sq = pointwise_product(&deg_p, &deg_p);
    
    // sigma_r_sq = conv2d(ref_neuro_sq, window) - ref_mu_sq
    let conv2_ref_neuro_sq = conv2d_with_boundary(&ref_neuro_sq);
    let sigma_r_sq = matrix_subtract(&conv2_ref_neuro_sq, &ref_mu_sq);
    
    let conv2_deg_neuro_sq = conv2d_with_boundary(&deg_neuro_sq);
    let sigma_d_sq = matrix_subtract(&conv2_deg_neuro_sq, &deg_mu_sq);
    
    // sigma_r_d = conv2d(ref_patch .* deg_patch, window) - mu_r_mu_d
    let ref_neuro_deg = pointwise_product(&ref_p, &deg_p);
    let conv2_ref_neuro_deg = conv2d_with_boundary(&ref_neuro_deg);
    let sigma_r_d = matrix_subtract(&conv2_ref_neuro_deg, &mu_r_mu_d);
    
    // intensity = (2*mu_r_mu_d + C1) / (ref_mu_sq + deg_mu_sq + C1)
    let intensity_numer = scalar_multiply(&mu_r_mu_d, 2.0);
    let intensity_numer = pointwise_add_scalar(&intensity_numer, C1);
    let intensity_denom = pointwise_add(&ref_mu_sq, &deg_mu_sq, C1);
    let intensity = pointwise_divide(&intensity_numer, &intensity_denom);
    
    // structure = (sigma_r_d + C3) / (sqrt(sigma_r_sq .* sigma_d_sq) + C3)
    let structure_numer = pointwise_add_scalar(&sigma_r_d, C3);
    let sigma_product = pointwise_product(&sigma_r_sq, &sigma_d_sq);
    let structure_denom: Vec<Vec<f64>> = sigma_product.iter().map(|row| {
        row.iter().map(|&d| {
            if d < 0.0 { C3 } else { d.sqrt() + C3 }
        }).collect()
    }).collect();
    let structure = pointwise_divide(&structure_numer, &structure_denom);
    
    // sim_map = intensity .* structure
    let sim_map = pointwise_product(&intensity, &structure);
    
    // 与 ViSQOL 一致：freq_band_deg_energy = deg_patch.Mean(kDimension::ROW)
    let freq_band_deg_energy = row_means(&deg_p);
    
    // freq_band_means = sim_map.Mean(kDimension::ROW)
    let freq_band_means = row_means(&sim_map);
    
    // freq_band_stddevs = sim_map.StdDev(kDimension::ROW)
    let freq_band_stddevs = row_stddevs(&sim_map);
    
    // NSIM = freq_band_means 的均值
    let similarity = if !freq_band_means.is_empty() {
        freq_band_means.iter().sum::<f64>() / freq_band_means.len() as f64
    } else { 0.0 };
    
    NsimPatchResult {
        similarity,
        freq_band_means,
        freq_band_stddevs,
        freq_band_deg_energy,
        start_time_s: 0.0,
        end_time_s: 0.0,
    }
}

/// 提取 patch
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
        for f in start_frame..(start_frame + num_frames) {
            let v = if f < spectrogram[b].len() { spectrogram[b][f] } else { 0.0 };
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
                ref_spectro, band_pos, patch_size_bands, frame_pos, patch_size_frames,
            );
            let deg_patch = extract_patch(
                deg_spectro, band_pos, patch_size_bands, frame_pos, patch_size_frames,
            );
            
            let mut result = compute_patch_nsim(&ref_patch, &deg_patch);
            result.start_time_s = frame_pos as f64 * 0.04;
            result.end_time_s = (frame_pos + patch_size_frames) as f64 * 0.04;
            
            results.push(result);
            frame_pos += hop_frames;
        }
        band_pos += hop_bands;
    }
    
    results
}
