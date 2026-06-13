//! 信号对齐模块
//! 使用 FFT 互相关 + 归一化相关系数，在录制音频中定位参考音频的精确起始位置。
//! 支持全局对齐（首次出现位置）与分段局部对齐（预期位置邻域内重对齐）。

use rustfft::{num_complex::Complex, FftPlanner};

/// 对齐结果
pub struct AlignmentResult {
    /// 参考音频在录制音频中的起始偏移（采样点数）
    pub offset_samples: usize,
    /// 延迟时间（毫秒）
    pub delay_ms: f64,
    /// 归一化相关系数峰值（0~1，越高越可靠）
    pub confidence: f64,
}

/// 显著峰判定阈值：低于该值认为没有可靠匹配，回退到全局最大峰。
const PEAK_CONFIDENCE_THRESHOLD: f64 = 0.5;

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

/// 查询 degraded[start..start+len] 的能量（平方和）。
fn window_energy(prefix: &[f64], start: usize, len: usize) -> f64 {
    let end = (start + len).min(prefix.len() - 1);
    if end <= start {
        return 0.0;
    }
    prefix[end] - prefix[start]
}

/// 在指定搜索区间 [search_start, search_end] 内，
/// 计算归一化相关系数序列，并返回 (首个显著峰偏移, 峰值)。
///
/// 归一化：corr[k] / √(ref_energy × seg_energy(k))，落在 0~1。
/// 选峰策略：优先返回首个 >= 阈值的局部峰（首次出现位置），
/// 否则返回区间内全局最大峰。
fn pick_peak(
    xcorr: &[f64],
    ref_energy: f64,
    deg_prefix: &[f64],
    ref_len: usize,
    search_start: usize,
    search_end: usize,
) -> (usize, f64) {
    let search_end = search_end.min(xcorr.len().saturating_sub(1));
    if search_start > search_end || ref_energy <= 0.0 {
        return (search_start, 0.0);
    }

    let normalized = |k: usize| -> f64 {
        let seg_energy = window_energy(deg_prefix, k, ref_len);
        let denom = (ref_energy * seg_energy).sqrt();
        if denom > 1e-12 {
            (xcorr[k].abs() / denom).min(1.0)
        } else {
            0.0
        }
    };

    // 全局最大峰（兜底）
    let mut best_idx = search_start;
    let mut best_val = normalized(search_start);
    // 首个显著局部峰
    let mut first_significant: Option<(usize, f64)> = None;

    for k in search_start..=search_end {
        let v = normalized(k);
        if v > best_val {
            best_val = v;
            best_idx = k;
        }
        if first_significant.is_none() && v >= PEAK_CONFIDENCE_THRESHOLD {
            // 简单局部峰确认：比相邻点不低
            let prev_ok = k == search_start || v >= normalized(k - 1);
            let next_ok = k == search_end || v >= normalized(k + 1);
            if prev_ok && next_ok {
                first_significant = Some((k, v));
            }
        }
    }

    first_significant.unwrap_or((best_idx, best_val))
}

/// 全局对齐：在整个录制中定位参考首次出现的位置。
pub fn find_alignment(reference: &[f64], degraded: &[f64], sample_rate: u32) -> AlignmentResult {
    let ref_len = reference.len();
    let deg_len = degraded.len();

    let xcorr = raw_cross_correlation(reference, degraded);
    let deg_prefix = prefix_square_sums(degraded);
    let ref_energy: f64 = reference.iter().map(|x| x * x).sum();

    let max_search = deg_len.saturating_sub(ref_len);
    let (peak_idx, peak_val) =
        pick_peak(&xcorr, ref_energy, &deg_prefix, ref_len, 0, max_search);

    AlignmentResult {
        offset_samples: peak_idx,
        delay_ms: peak_idx as f64 / sample_rate as f64 * 1000.0,
        confidence: peak_val,
    }
}

/// 分段局部对齐：在「预期起点 ± search_radius」邻域内重新对齐当前段。
///
/// 用于分段评估时每段独立对齐，避免 `aligned_start + i*ref_len` 盲推
/// 因循环周期偏差 / 时钟漂移累积而错位。
pub fn find_local_alignment(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    expected_start: usize,
    search_radius: usize,
) -> AlignmentResult {
    let ref_len = reference.len();
    let deg_len = degraded.len();

    let xcorr = raw_cross_correlation(reference, degraded);
    let deg_prefix = prefix_square_sums(degraded);
    let ref_energy: f64 = reference.iter().map(|x| x * x).sum();

    let search_start = expected_start.saturating_sub(search_radius);
    let search_end = (expected_start + search_radius).min(deg_len.saturating_sub(ref_len));

    let (peak_idx, peak_val) = pick_peak(
        &xcorr,
        ref_energy,
        &deg_prefix,
        ref_len,
        search_start,
        search_end,
    );

    AlignmentResult {
        offset_samples: peak_idx,
        delay_ms: peak_idx as f64 / sample_rate as f64 * 1000.0,
        confidence: peak_val,
    }
}
