//! 频域特征匹配对齐模块
//!
//! 使用音频指纹（峰值频率）进行鲁棒的对齐检测。
//! 步骤：
//! 1. 分帧 + 汉宁窗
//! 2. FFT 获取频谱，提取幅度
//! 3. 每帧提取能量最大的前 K 个频率点作为指纹
//! 4. 滑动匹配短音频特征序列在长音频中的位置
//! 5. 找到连续高相似度区域

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
const HOP_SIZE: usize = 512;  // 50% 重叠
const TOP_K_PEAKS: usize = 5;  // 每帧取��� 5 个峰值作为指纹

/// 创建汉宁窗
fn hanning_window(size: usize) -> Vec<f64> {
    (0..size)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / (size - 1) as f64).cos()))
        .collect()
}

/// 对音频提取帧特征序列
pub fn extract_features(samples: &[f64]) -> Vec<Vec<usize>> {
    let n = samples.len();
    let window = hanning_window(FRAME_SIZE);
    
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FRAME_SIZE);
    
    let mut features = Vec::new();
    let mut pos = 0;
    
    while pos + FRAME_SIZE <= n {
        // 加窗
        let mut frame: Vec<Complex<f64>> = samples[pos..pos + FRAME_SIZE]
            .iter()
            .zip(window.iter())
            .map(|(&s, &w)| Complex::new(s * w, 0.0))
            .collect();
        
        // FFT
        fft.process(&mut frame);
        
        // 提取幅度谱（前一半是有效的，共振频率）
        let half_len = FRAME_SIZE / 2;
        let mut magnitudes: Vec<(usize, f64)> = (0..half_len)
            .map(|i| (i, frame[i].norm()))
            .collect();
        
        // 取前 TOP_K_PEAKS 个峰值
        magnitudes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let top_peaks: Vec<usize> = magnitudes.into_iter().take(TOP_K_PEAKS).map(|(i, _)| i).collect();
        features.push(top_peaks);
        
        pos += HOP_SIZE;
    }
    
    features
}

/// 计算两个帧特征的相似度（基于峰值频率的重叠程度）
fn feature_similarity(f1: &[usize], f2: &[usize]) -> f64 {
    if f1.is_empty() || f2.is_empty() {
        return 0.0;
    }
    
    // 计算峰值索引的重叠率
    let mut matches = 0;
    for &idx1 in f1 {
        for &idx2 in f2 {
            // 允许一定的频率容差（±2 个 bin）
            if idx1.abs_diff(idx2) <= 2 {
                matches += 1;
                break;
            }
        }
    }
    
    // Jaccard 相似度
    let union = f1.len() + f2.len() - matches;
    if union == 0 {
        return 0.0;
    }
    matches as f64 / union as f64
}

/// 在长音频特征序列中滑动匹配短音频特征序列
fn sliding_match(short_features: &[Vec<usize>], long_features: &[Vec<usize>]) -> Vec<(usize, f64)> {
    if short_features.is_empty() || long_features.is_empty() {
        return vec![];
    }
    
    let short_len = short_features.len();
    let long_len = long_features.len();
    
    // 滑动窗口，计算每帧的相似度
    let mut similarities = Vec::new();
    
    for start in 0..long_len.saturating_sub(short_len) {
        // 计算这个窗口内所有帧的相似度
        let mut total_sim = 0.0;
        for (i, short_f) in short_features.iter().enumerate() {
            total_sim += feature_similarity(short_f, &long_features[start + i]);
        }
        let avg_sim = total_sim / short_len as f64;
        similarities.push((start, avg_sim));
    }
    
    similarities
}

/// 在相似度序列中寻找峰值（局部极大值）
fn find_peaks(similarities: &[(usize, f64)], min_gap_frames: usize) -> Vec<(usize, f64)> {
    if similarities.is_empty() {
        return vec![];
    }
    
    let mut peaks = Vec::new();
    
    for (i, &(pos, sim)) in similarities.iter().enumerate() {
        // 判断是否为局部极大值
        let lo = i.saturating_sub(2);
        let hi = (i + 2).min(similarities.len());
        let is_local_max = similarities[lo..hi].iter().all(|(_, s)| *s <= sim);
        
        if is_local_max && sim > 0.1 {
            peaks.push((pos, sim));
        }
    }
    
    // 按相似度降序排序
    peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    
    // 去重：保留高置信度的峰值
    let mut filtered = Vec::new();
    for (pos, sim) in peaks {
        let too_close = filtered.iter().any(|(existing_pos, _)| {
            (pos as isize - *existing_pos as isize).abs() < min_gap_frames as isize
        });
        if !too_close {
            filtered.push((pos, sim));
        }
    }
    
    // 按位置排序
    filtered.sort_by_key(|(pos, _)| *pos);
    filtered
}

/// 使用频域特征匹配进行多峰检测
pub fn find_all_alignments_v2(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    confidence_threshold: f64,
) -> Vec<AlignmentResult> {
    // 提取特征
    let ref_features = extract_features(reference);
    let deg_features = extract_features(degraded);
    
    if ref_features.is_empty() || deg_features.is_empty() {
        return vec![AlignmentResult {
            offset_samples: 0,
            delay_ms: 0.0,
            confidence: 0.0,
        }];
    }
    
    // 滑动匹配
    let similarities = sliding_match(&ref_features, &deg_features);
    
    // 找峰值
    let min_gap_frames = ref_features.len() / 2;
    let peaks = find_peaks(&similarities, min_gap_frames);
    
    let mut results = Vec::new();
    for (frame_start, confidence) in peaks {
        if confidence >= confidence_threshold {
            // 将帧位置转换为采样点位置
            let offset_samples = frame_start * HOP_SIZE;
            
            results.push(AlignmentResult {
                offset_samples,
                delay_ms: offset_samples as f64 / sample_rate as f64 * 1000.0,
                confidence,
            });
        }
    }
    
    // 兜底：如果没有找到足够的峰，返回全局最佳匹配
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

/// 组合对齐：先用频域特征匹配找候选位置，再用 FFT 互相关精细化
pub fn find_all_alignments_hybrid(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    confidence_threshold: f64,
) -> Vec<AlignmentResult> {
    // 第一步：使用频域特征匹配找候选位置
    let candidates = find_all_alignments_v2(reference, degraded, sample_rate, confidence_threshold * 0.5);
    
    if candidates.is_empty() {
        return vec![AlignmentResult {
            offset_samples: 0,
            delay_ms: 0.0,
            confidence: 0.0,
        }];
    }
    
    // 第二步：对每个候选位置用 FFT 互相关精细化（局部搜索）
    let mut refined = Vec::new();
    let ref_len = reference.len();
    let search_window = ref_len / 4;
    
    for candidate in candidates {
        let search_start = candidate.offset_samples.saturating_sub(search_window);
        let search_end = (candidate.offset_samples + search_window).min(degraded.len().saturating_sub(ref_len));
        
        if search_end > search_start {
            let window_ref = &reference[..ref_len.min(reference.len())];
            let window_deg = &degraded[search_start..search_end.min(degraded.len())];
            
            let local_corr = local_xcorr(window_ref, window_deg);
            if let Some((best_offset, confidence)) = local_corr {
                refined.push(AlignmentResult {
                    offset_samples: search_start + best_offset,
                    delay_ms: (search_start + best_offset) as f64 / sample_rate as f64 * 1000.0,
                    confidence,
                });
            }
        }
    }
    
    // 按置信度排序
    refined.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    
    // 取前 N 个结果
    let mut final_results = Vec::new();
    let min_gap = ref_len / 2;
    for r in refined {
        let too_close = final_results.iter().any(|existing: &AlignmentResult| {
            (r.offset_samples as isize - existing.offset_samples as isize).abs() < (min_gap as isize)
        });
        if !too_close {
            final_results.push(r);
        }
        if final_results.len() >= 10 {
            break;
        }
    }
    
    // 按位置排序
    final_results.sort_by_key(|r| r.offset_samples);
    final_results
}

/// 局部互相关精细化
fn local_xcorr(reference: &[f64], degraded: &[f64]) -> Option<(usize, f64)> {
    let ref_len = reference.len();
    let deg_len = degraded.len();
    
    if ref_len > deg_len {
        return None;
    }
    
    let ref_energy: f64 = reference.iter().map(|x| x * x).sum();
    
    let mut best_offset = 0;
    let mut best_conf = 0.0;
    
    for offset in 0..deg_len.saturating_sub(ref_len) {
        let sum: f64 = reference
            .iter()
            .zip(degraded[offset..].iter())
            .map(|(r, &d)| r * d)
            .sum();
        
        let deg_energy: f64 = degraded[offset..offset + ref_len].iter().map(|x| x * x).sum();
        let denom = (ref_energy * deg_energy).sqrt();
        
        if denom > 1e-10 {
            let conf = (sum / denom).abs();
            if conf > best_conf {
                best_conf = conf;
                best_offset = offset;
            }
        }
    }
    
    if best_conf > 0.1 {
        Some((best_offset, best_conf))
    } else {
        None
    }
}
