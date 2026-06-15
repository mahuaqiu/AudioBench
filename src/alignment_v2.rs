//! 频域特征匹配对齐模块
//!
//! 使用 MFCC 特征进行鲁棒的对齐检测。
//! 步骤：
//! 1. 分帧 + 汉宁窗 + FFT
//! 2. 梅尔滤波器组提取梅尔频谱
//! 3. DCT 变换得到 MFCC 特征
//! 4. 滑动余弦相似度匹配
//! 5. 峰值检测定位参考音频出现位置
//! 6. FFT 互相关精细化

use rustfft::{num_complex::Complex, FftPlanner};

/// 对齐结果（与 alignment.rs 保持一致）
#[derive(Debug, Clone)]
pub struct AlignmentResult {
    /// 参考音频在录制音频中的起始偏移（采样点数）
    pub offset_samples: usize,
    /// 延迟时间（毫秒）
    pub delay_ms: f64,
    /// 归一化相关系数峰值（0~1，越高越可靠）
    pub confidence: f64,
}

/// 分帧参数
const FRAME_SIZE: usize = 1024;
const HOP_SIZE: usize = 512;
/// MFCC 系数数量
const NUM_MFCC: usize = 13;
/// 梅尔滤波器组数量
const NUM_MEL_BANDS: usize = 26;

/// 创建汉宁窗
fn hanning_window(size: usize) -> Vec<f64> {
    (0..size)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / (size - 1) as f64).cos()))
        .collect()
}

/// Hz → Mel 刻度转换
fn hz_to_mel(hz: f64) -> f64 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

/// Mel → Hz 刻度转换
fn mel_to_hz(mel: f64) -> f64 {
    700.0 * (10.0f64.powf(mel / 2595.0) - 1.0)
}

/// 生成梅尔滤波器组
fn create_mel_filterbank(sample_rate: u32) -> Vec<Vec<f64>> {
    let fft_len = FRAME_SIZE / 2 + 1;
    let high_freq = sample_rate as f64 / 2.0;
    let low_mel = hz_to_mel(0.0);
    let high_mel = hz_to_mel(high_freq);

    let mel_points: Vec<f64> = (0..=NUM_MEL_BANDS + 1)
        .map(|i| low_mel + (high_mel - low_mel) * i as f64 / (NUM_MEL_BANDS + 1) as f64)
        .collect();
    let hz_points: Vec<f64> = mel_points.iter().map(|m| mel_to_hz(*m)).collect();
    let bin_points: Vec<f64> = hz_points
        .iter()
        .map(|hz| (FRAME_SIZE as f64 + 1.0) * hz / sample_rate as f64)
        .collect();

    let mut filterbank = vec![vec![0.0f64; fft_len]; NUM_MEL_BANDS];
    for m in 0..NUM_MEL_BANDS {
        let f_left = bin_points[m];
        let f_center = bin_points[m + 1];
        let f_right = bin_points[m + 2];

        for k in 0..fft_len {
            let kf = k as f64;
            if kf >= f_left && kf <= f_center && f_center > f_left {
                filterbank[m][k] = (kf - f_left) / (f_center - f_left);
            } else if kf >= f_center && kf <= f_right && f_right > f_center {
                filterbank[m][k] = (f_right - kf) / (f_right - f_center);
            }
        }
    }
    filterbank
}

/// DCT-II 变换
fn dct_ii(input: &[f64], num_coeffs: usize) -> Vec<f64> {
    let n = input.len();
    let mut output = Vec::with_capacity(num_coeffs);
    for k in 0..num_coeffs {
        let mut sum = 0.0;
        for (i, &val) in input.iter().enumerate() {
            sum += val * (std::f64::consts::PI * (i as f64 + 0.5) * k as f64 / n as f64).cos();
        }
        output.push(sum);
    }
    output
}

/// 对音频提取 MFCC 特征序列
pub fn extract_mfcc_features(samples: &[f64], sample_rate: u32) -> Vec<Vec<f64>> {
    let n = samples.len();
    let window = hanning_window(FRAME_SIZE);
    let filterbank = create_mel_filterbank(sample_rate);

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FRAME_SIZE);

    let mut features = Vec::new();
    let mut pos = 0;

    while pos + FRAME_SIZE <= n {
        let mut frame: Vec<Complex<f64>> = samples[pos..pos + FRAME_SIZE]
            .iter()
            .zip(window.iter())
            .map(|(&s, &w)| Complex::new(s * w, 0.0))
            .collect();

        fft.process(&mut frame);

        let half_len = FRAME_SIZE / 2 + 1;
        let power_spectrum: Vec<f64> = frame[..half_len]
            .iter()
            .map(|c| (c.re * c.re + c.im * c.im) / FRAME_SIZE as f64)
            .collect();

        let mel_energies: Vec<f64> = filterbank
            .iter()
            .map(|band| {
                let energy: f64 = band.iter().zip(power_spectrum.iter()).map(|(w, p)| w * p).sum();
                (energy + 1e-10).ln()
            })
            .collect();

        let mut mfcc = dct_ii(&mel_energies, NUM_MFCC);
        let log_energy = (samples[pos..pos + FRAME_SIZE].iter().map(|x| x * x).sum::<f64>() + 1e-10).ln();
        mfcc[0] = log_energy;

        features.push(mfcc);
        pos += HOP_SIZE;
    }

    features
}

/// 计算余弦相似度
fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    let denom = norm_a * norm_b;
    if denom < 1e-12 {
        return 0.0;
    }
    (dot / denom).clamp(-1.0, 1.0)
}

/// 滑动 MFCC 匹配
fn sliding_mfcc_match(
    short_mfcc: &[Vec<f64>],
    long_mfcc: &[Vec<f64>],
) -> Vec<(usize, f64)> {
    if short_mfcc.is_empty() || long_mfcc.is_empty() {
        return vec![];
    }

    let short_len = short_mfcc.len();
    let long_len = long_mfcc.len();

    let short_norms: Vec<f64> = short_mfcc
        .iter()
        .map(|f| f.iter().map(|x| x * x).sum::<f64>().sqrt())
        .collect();

    let mut similarities = Vec::with_capacity(long_len.saturating_sub(short_len) + 1);

    for start in 0..=long_len.saturating_sub(short_len) {
        let mut total_sim = 0.0;
        let mut valid_frames = 0;
        for (i, short_f) in short_mfcc.iter().enumerate() {
            let long_f = &long_mfcc[start + i];
            let sim = cosine_similarity(short_f, long_f);
            if short_norms[i] > 0.1 && long_f.iter().map(|x| x * x).sum::<f64>().sqrt() > 0.1 {
                total_sim += sim;
                valid_frames += 1;
            }
        }
        if valid_frames > 0 {
            similarities.push((start, total_sim / valid_frames as f64));
        } else {
            similarities.push((start, 0.0));
        }
    }

    similarities
}

/// 寻找峰值
fn find_peaks(similarities: &[(usize, f64)], min_gap_frames: usize, threshold: f64) -> Vec<(usize, f64)> {
    if similarities.is_empty() {
        return vec![];
    }

    let mut peaks = Vec::new();
    let half_win = 5.min(similarities.len() / 4).max(1);

    for (i, &(pos, sim)) in similarities.iter().enumerate() {
        if sim < threshold {
            continue;
        }
        let lo = i.saturating_sub(half_win);
        let hi = (i + half_win + 1).min(similarities.len());
        let is_local_max = similarities[lo..hi].iter().all(|&(_, s)| s <= sim);

        if is_local_max {
            peaks.push((pos, sim));
        }
    }

    peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut filtered = Vec::new();
    for (pos, sim) in peaks {
        let too_close = filtered.iter().any(|(existing_pos, _)| {
            ((pos as isize - *existing_pos as isize).unsigned_abs() as usize) < min_gap_frames
        });
        if !too_close {
            filtered.push((pos, sim));
        }
    }

    filtered.sort_by_key(|(pos, _)| *pos);
    filtered
}

/// 使用 MFCC 特征匹配进行多峰检测
pub fn find_all_alignments_v2(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    confidence_threshold: f64,
) -> Vec<AlignmentResult> {
    let ref_mfcc = extract_mfcc_features(reference, sample_rate);
    let deg_mfcc = extract_mfcc_features(degraded, sample_rate);

    if ref_mfcc.is_empty() || deg_mfcc.is_empty() {
        return vec![AlignmentResult {
            offset_samples: 0,
            delay_ms: 0.0,
            confidence: 0.0,
        }];
    }

    let similarities = sliding_mfcc_match(&ref_mfcc, &deg_mfcc);

    let min_gap_frames = ref_mfcc.len() / 2;
    let peaks = find_peaks(&similarities, min_gap_frames, confidence_threshold);

    let mut results = Vec::new();
    for (frame_start, confidence) in peaks {
        let offset_samples = frame_start * HOP_SIZE;
        results.push(AlignmentResult {
            offset_samples,
            delay_ms: offset_samples as f64 / sample_rate as f64 * 1000.0,
            confidence,
        });
    }

    if results.is_empty() {
        if let Some(&(pos, sim)) = similarities.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)) {
            results.push(AlignmentResult {
                offset_samples: pos * HOP_SIZE,
                delay_ms: pos as f64 * HOP_SIZE as f64 / sample_rate as f64 * 1000.0,
                confidence: sim,
            });
        }
    }

    results
}

/// 组合对齐：MFCC + FFT 互相关精细化
pub fn find_all_alignments_hybrid(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    confidence_threshold: f64,
) -> Vec<AlignmentResult> {
    let candidates = find_all_alignments_v2(reference, degraded, sample_rate, confidence_threshold * 0.5);

    if candidates.is_empty() {
        let fallbacks = crate::alignment::find_all_alignments(reference, degraded, sample_rate, confidence_threshold);
        return fallbacks.into_iter().map(|r| AlignmentResult {
            offset_samples: r.offset_samples,
            delay_ms: r.delay_ms,
            confidence: r.confidence,
        }).collect();
    }

    let ref_len = reference.len();
    let search_window = sample_rate as usize;

    let mut refined = Vec::new();

    for candidate in candidates {
        let search_start = candidate.offset_samples.saturating_sub(search_window);
        let search_end = (candidate.offset_samples + search_window)
            .min(degraded.len().saturating_sub(ref_len));

        if search_end <= search_start || ref_len > degraded.len() {
            refined.push(candidate);
            continue;
        }

        let local_result = local_fft_xcorr(
            reference,
            &degraded[search_start..],
            sample_rate,
            search_end - search_start,
        );

        match local_result {
            Some((best_local_offset, confidence)) => {
                refined.push(AlignmentResult {
                    offset_samples: search_start + best_local_offset,
                    delay_ms: (search_start + best_local_offset) as f64 / sample_rate as f64 * 1000.0,
                    confidence,
                });
            }
            None => {
                refined.push(candidate);
            }
        }
    }

    refined.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));

    let min_gap = ref_len / 2;
    let mut final_results = Vec::new();
    for r in refined {
        let too_close = final_results.iter().any(|existing: &AlignmentResult| {
            ((r.offset_samples as isize - existing.offset_samples as isize).unsigned_abs() as usize) < min_gap
        });
        if !too_close {
            final_results.push(r);
        }
        if final_results.len() >= 10 {
            break;
        }
    }

    final_results.sort_by_key(|r| r.offset_samples);

    if final_results.is_empty() {
        let fallbacks = crate::alignment::find_all_alignments(reference, degraded, sample_rate, confidence_threshold);
        return fallbacks.into_iter().map(|r| AlignmentResult {
            offset_samples: r.offset_samples,
            delay_ms: r.delay_ms,
            confidence: r.confidence,
        }).collect();
    }

    final_results
}

/// 局部 FFT 互相关精细化
fn local_fft_xcorr(
    reference: &[f64],
    degraded_local: &[f64],
    _sample_rate: u32,
    search_len: usize,
) -> Option<(usize, f64)> {
    let ref_len = reference.len();
    let deg_len = degraded_local.len().min(search_len + ref_len);

    if ref_len > deg_len {
        return None;
    }

    let max_search = deg_len.saturating_sub(ref_len);
    if max_search == 0 {
        return None;
    }

    let n = deg_len.next_power_of_two();

    let mut ref_fft: Vec<Complex<f64>> = reference
        .iter()
        .map(|&x| Complex::new(x, 0.0))
        .chain(std::iter::repeat(Complex::new(0.0, 0.0)))
        .take(n)
        .collect();

    let mut deg_fft: Vec<Complex<f64>> = degraded_local
        .iter()
        .map(|&x| Complex::new(x, 0.0))
        .chain(std::iter::repeat(Complex::new(0.0, 0.0)))
        .take(n)
        .collect();

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(n);
    fft.process(&mut ref_fft);
    fft.process(&mut deg_fft);

    let mut product: Vec<Complex<f64>> = ref_fft
        .iter()
        .zip(deg_fft.iter())
        .map(|(r, d)| r.conj() * d)
        .collect();

    let ifft = planner.plan_fft_inverse(n);
    ifft.process(&mut product);

    let scale = 1.0 / n as f64;

    let prefix_sq: Vec<f64> = {
        let mut p = vec![0.0];
        let mut acc = 0.0;
        for &x in degraded_local {
            acc += x * x;
            p.push(acc);
        }
        p
    };

    let ref_energy: f64 = reference.iter().map(|x| x * x).sum();

    let mut best_offset = 0;
    let mut best_conf = 0.0f64;

    for k in 0..=max_search {
        let xcorr_val = (product[k].re * scale).abs();

        let seg_energy = if k + ref_len <= prefix_sq.len() {
            prefix_sq[k + ref_len] - prefix_sq[k]
        } else {
            0.0
        };

        let denom = (ref_energy * seg_energy).sqrt();
        if denom > 1e-12 {
            let conf = (xcorr_val / denom).min(1.0);
            if conf > best_conf {
                best_conf = conf;
                best_offset = k;
            }
        }
    }

    if best_conf > 0.05 {
        Some((best_offset, best_conf))
    } else {
        None
    }
}
