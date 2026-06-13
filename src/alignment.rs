//! 信号对齐模块
//! 使用 FFT 互相关 + 归一化相关系数，在录制音频中定位参考音频的精确起始位置。
//! 支持多峰检测（自动发现所有出现位置），用于分段评估场景。

use rustfft::{num_complex::Complex, FftPlanner};

/// 对齐结果
#[derive(Debug, Clone)]
pub struct AlignmentResult {
    /// 参考音频在录制音频中的起始偏移（采样点数）
    pub offset_samples: usize,
    /// 延迟时间（毫秒）
    pub delay_ms: f64,
    /// 归一化相关系数峰值（0~1，越高越可靠）
    pub confidence: f64,
}

/// 通过 FFT 互相关计算原始互相关序列（未归一化）。
/// 返回 xcorr[k] 表示参考相对录制偏移 k 个采样点时的相关值。
fn raw_cross_correlation(reference: &[f64], degraded: &[f64]) -> Vec<f64> {
    let deg_len = degraded.len();
    let n = deg_len.next_power_of_two();

    let mut ref_fft: Vec<Complex<f64>> = reference
        .iter()
        .map(|&x| Complex::new(x, 0.0))
        .chain(std::iter::repeat(Complex::new(0.0, 0.0)))
        .take(n)
        .collect();

    let mut deg_fft: Vec<Complex<f64>> = degraded
        .iter()
        .map(|&x| Complex::new(x, 0.0))
        .chain(std::iter::repeat(Complex::new(0.0, 0.0)))
        .take(n)
        .collect();

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(n);
    fft.process(&mut ref_fft);
    fft.process(&mut deg_fft);

    // 频域点乘：参考取共轭 × 录制
    let mut product: Vec<Complex<f64>> = ref_fft
        .iter()
        .zip(deg_fft.iter())
        .map(|(r, d)| r.conj() * d)
        .collect();

    let ifft = planner.plan_fft_inverse(n);
    ifft.process(&mut product);

    let scale = 1.0 / n as f64;
    product.iter().map(|c| c.re * scale).collect()
}

/// 计算录制信号的「前缀平方和」，用于 O(1) 查询任意窗口能量。
fn prefix_square_sums(signal: &[f64]) -> Vec<f64> {
    let mut prefix = Vec::with_capacity(signal.len() + 1);
    prefix.push(0.0);
    let mut acc = 0.0;
    for &x in signal {
        acc += x * x;
        prefix.push(acc);
    }
    prefix
}

/// 查询 degraded[start..start+len] 的��量（平方和）。
fn window_energy(prefix: &[f64], start: usize, len: usize) -> f64 {
    let end = (start + len).min(prefix.len() - 1);
    if end <= start {
        return 0.0;
    }
    prefix[end] - prefix[start]
}

/// 多峰检测：在录制音频中自动发现参考音频的所有出现位置。
///
/// 算法流程：
/// 1. 计算全程归一化互相关
/// 2. 在互相关序列上搜索所有局部极大值
/// 3. 过滤：峰值 >= 阈值 且 与前一个峰间距 >= min_gap_samples
/// 4. 按时间排序返回
///
/// 返回值保证非空：至少包含全局最高峰（即使低于阈值）。
pub fn find_all_alignments(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    confidence_threshold: f64,
) -> Vec<AlignmentResult> {
    let ref_len = reference.len();
    let deg_len = degraded.len();

    if ref_len == 0 || deg_len < ref_len {
        return vec![AlignmentResult {
            offset_samples: 0,
            delay_ms: 0.0,
            confidence: 0.0,
        }];
    }

    let xcorr = raw_cross_correlation(reference, degraded);
    let deg_prefix = prefix_square_sums(degraded);
    let ref_energy: f64 = reference.iter().map(|x| x * x).sum();

    let max_search = deg_len.saturating_sub(ref_len);

    // 预计算归一化互相关序列（只在搜索范围内）
    let normalized_xcorr: Vec<f64> = (0..=max_search)
        .map(|k| {
            let seg_energy = window_energy(&deg_prefix, k, ref_len);
            let denom = (ref_energy * seg_energy).sqrt();
            if denom > 1e-12 {
                (xcorr[k].abs() / denom).min(1.0)
            } else {
                0.0
            }
        })
        .collect();

    // 最小峰间距：参考长度的 80%（避免同一次出现的旁瓣被重复检测）
    let min_gap_samples = (ref_len as f64 * 0.8) as usize;
    // 峰检测半窗口：用于判断局部极大值
    let peak_half_window = ((ref_len as f64 * 0.05) as usize).max(1);

    // 搜索所有局部极大值
    let mut candidates: Vec<(usize, f64)> = Vec::new();

    for k in 0..normalized_xcorr.len() {
        let v = normalized_xcorr[k];
        if v < confidence_threshold {
            continue;
        }
        // 局部极大值判定：在 [k-half, k+half] 范围内 v 最大
        let lo = k.saturating_sub(peak_half_window);
        let hi = (k + peak_half_window).min(normalized_xcorr.len() - 1);
        let is_local_max = (lo..=hi).all(|j| normalized_xcorr[j] <= v);
        if !is_local_max {
            continue;
        }
        // 精确化：在局部极大值邻域内找到真正的最大点
        let refine_lo = k.saturating_sub(peak_half_window);
        let refine_hi = (k + peak_half_window).min(normalized_xcorr.len() - 1);
        let best_k = (refine_lo..=refine_hi)
            .max_by(|&a, &b| {
                normalized_xcorr[a]
                    .partial_cmp(&normalized_xcorr[b])
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(k);

        candidates.push((best_k, normalized_xcorr[best_k]));
    }

    // 按置信度降序排序，按 min_gap_samples 去重：优先保留置信度更高的峰
    candidates.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    let mut filtered: Vec<(usize, f64)> = Vec::new();
    for (pos, conf) in &candidates {
        let too_close = filtered.iter().any(|(existing_pos, _)| {
            (*pos as isize - *existing_pos as isize).unsigned_abs() < min_gap_samples
        });
        if !too_close {
            filtered.push((*pos, *conf));
        }
    }

    // 按时间排序
    filtered.sort_by_key(|(pos, _)| *pos);

    // 兜底：如果没找到任何满足阈值的峰，返回全���最高峰
    if filtered.is_empty() {
        let best_k = (0..normalized_xcorr.len())
            .max_by(|&a, &b| {
                normalized_xcorr[a]
                    .partial_cmp(&normalized_xcorr[b])
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(0);
        filtered.push((best_k, normalized_xcorr[best_k]));
    }

    filtered
        .into_iter()
        .map(|(offset, confidence)| AlignmentResult {
            offset_samples: offset,
            delay_ms: offset as f64 / sample_rate as f64 * 1000.0,
            confidence,
        })
        .collect()
}
