//! 信号对齐模块
//! 使用 FFT 互相关算法，在录制音频中定位参考音频的精确起始位置

use rustfft::{FftPlanner, num_complex::Complex};

/// 对齐结果
pub struct AlignmentResult {
    /// 参考音频在录制音频中的起始偏移（采样点数）
    pub offset_samples: usize,
    /// 延迟时间（毫秒）
    pub delay_ms: f64,
    /// 互相关峰值（归一化后 0~1，越高表示匹配越可靠）
    pub confidence: f64,
}

/// 通过 FFT 互相关找到参考音频在录制音频中的最佳对齐位置
/// 
/// 原理：将参考信号补零到与录制信号等长，分别做 FFT，
/// 在频域做点乘（参考取共轭），再 IFFT 回时域，
/// 峰值位置即传输延迟。
pub fn find_alignment(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
) -> AlignmentResult {
    let ref_len = reference.len();
    let deg_len = degraded.len();
    let n = deg_len.next_power_of_two();

    // 构造复数数组，参考信号补零到 n
    let mut ref_fft: Vec<Complex<f64>> = reference.iter()
        .map(|&x| Complex::new(x, 0.0))
        .chain(std::iter::repeat(Complex::new(0.0, 0.0)))
        .take(n)
        .collect();
    
    let mut deg_fft: Vec<Complex<f64>> = degraded.iter()
        .map(|&x| Complex::new(x, 0.0))
        .chain(std::iter::repeat(Complex::new(0.0, 0.0)))
        .take(n)
        .collect();

    // FFT
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(n);
    fft.process(&mut ref_fft);
    fft.process(&mut deg_fft);

    // 频域点乘：参考取共轭 × 录制
    let mut product: Vec<Complex<f64>> = ref_fft.iter()
        .zip(deg_fft.iter())
        .map(|(r, d)| r.conj() * d)
        .collect();

    // IFFT
    let ifft = planner.plan_fft_inverse(n);
    ifft.process(&mut product);

    // 归一化
    let scale = 1.0 / n as f64;
    let xcorr: Vec<f64> = product.iter().map(|c| (c.re * scale).abs()).collect();

    // 计算参考和录制信号的能量，用于归一化置信度
    let ref_energy: f64 = reference.iter().map(|x| x * x).sum::<f64>().sqrt();
    
    // 在有效搜索范围内寻找峰值
    // 搜索范围：0 到 degraded_len - ref_len（参考信号不能超出录制信号尾部）
    let max_search = deg_len.saturating_sub(ref_len);
    let (peak_idx, peak_val) = xcorr[..=max_search]
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, &v)| (i, v))
        .unwrap_or((0, 0.0));

    // 归一化置信度
    let confidence = if ref_energy > 0.0 {
        (peak_val / ref_energy).min(1.0)
    } else {
        0.0
    };

    AlignmentResult {
        offset_samples: peak_idx,
        delay_ms: peak_idx as f64 / sample_rate as f64 * 1000.0,
        confidence,
    }
}

/// 从录制音频中截取与参考音频等长的片段（基于对齐偏移）
pub fn extract_aligned_segment(
    degraded: &[f64],
    offset: usize,
    ref_len: usize,
) -> Vec<f64> {
    let end = (offset + ref_len).min(degraded.len());
    let mut segment = degraded[offset..end].to_vec();
    // 如果录制音频不够长，补零
    segment.resize(ref_len, 0.0);
    segment
}
