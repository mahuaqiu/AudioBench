//! 时轴漂移检测模块（重构）
//!
//! 使用滑窗 FFT 互相关提取 offset 序列，再通过形态分析（突变/斜坡）分类为 4 种子类型：
//! - Cut（裁剪）：内容缺失，前移
//! - Insertion（插入）：内容重复/新增，后移
//! - Stretch（拉伸）：匀速变慢，后移
//! - Compress（压缩）：匀速变快，前移

use crate::metrics::{WarpingConfig, WarpingEvent, WarpingType};

/// 计算一段采样的 RMS 能量
fn compute_rms(samples: &[f64]) -> f64 {
    if samples.is_empty() { return 0.0; }
    (samples.iter().map(|x| x * x).sum::<f64>() / samples.len() as f64).sqrt()
}

/// 局部归一化互相关：在 degraded 的 [center-radius, center+radius] 范围内
/// 找与 reference 最匹配的偏移位置，返回相对偏移（采样点数）。
fn local_xcorr_offset(
    reference: &[f64],
    degraded: &[f64],
    center: usize,
    search_radius: usize,
) -> Option<f64> {
    let ref_len = reference.len();
    if ref_len == 0 || degraded.len() < ref_len {
        return None;
    }

    let ref_energy: f64 = reference.iter().map(|x| x * x).sum();
    if ref_energy < 1e-12 {
        return None;
    }

    let lo = center.saturating_sub(search_radius);
    let hi = (center + search_radius).min(degraded.len().saturating_sub(ref_len));
    if hi <= lo {
        return None;
    }

    let mut best_offset: isize = 0;
    let mut best_conf: f64 = -1.0;

    for k in lo..=hi {
        let seg = &degraded[k..k + ref_len];
        let seg_energy: f64 = seg.iter().map(|x| x * x).sum();
        let denom = (ref_energy * seg_energy).sqrt();
        if denom < 1e-12 {
            continue;
        }
        let sum: f64 = reference.iter().zip(seg.iter()).map(|(r, &d)| r * d).sum();
        let conf = (sum / denom).abs();
        if conf > best_conf {
            best_conf = conf;
            best_offset = k as isize - center as isize;
        }
    }

    if best_conf < 0.0 {
        None
    } else {
        // 返回偏移量（ms）
        Some(best_offset as f64)
    }
}

/// 阶段一：计算 offset 序列
///
/// 对整段音频做滑窗互相关，返回每个窗口的局部偏移量（ms）。
/// 静音窗标记为 None，不参与后续形态分析。
///
/// # Arguments
/// * `reference` - 参考音频样本
/// * `degraded` - 录制音频样本
/// * `sample_rate` - 采样率
/// * `config` - 漂移检测配置
///
/// # Returns
/// Vec<Option<f64>> - offset 序列，None 表示该窗为静音
pub fn compute_offset_series(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    config: &WarpingConfig,
) -> Vec<Option<f64>> {
    let window_samples = (sample_rate as f64 * config.window_ms / 1000.0) as usize;
    let hop_samples = (sample_rate as f64 * config.hop_ms / 1000.0) as usize;
    let search_radius_samples = (sample_rate as f64 * config.search_radius_ms / 1000.0) as usize;

    if reference.len() < window_samples || degraded.len() < window_samples {
        return vec![];
    }

    let mut offsets = Vec::new();
    let num_windows = (reference.len().saturating_sub(window_samples)) / hop_samples + 1;

    for i in 0..num_windows {
        let start = i * hop_samples;
        if start + window_samples > reference.len() {
            break;
        }

        let ref_window = &reference[start..start + window_samples];

        // 静音窗检测
        let rms = compute_rms(ref_window);
        if rms < config.silence_threshold {
            offsets.push(None);
            continue;
        }

        // 局部互相关找偏移（相对于窗口中心）
        let center = start + window_samples / 2;
        let deg_center = center.min(degraded.len().saturating_sub(window_samples));

        if let Some(offset_samples) = local_xcorr_offset(
            ref_window,
            degraded,
            deg_center,
            search_radius_samples,
        ) {
            // 转换为 ms
            let offset_ms = offset_samples / sample_rate as f64 * 1000.0;
            offsets.push(Some(offset_ms));
        } else {
            offsets.push(None);
        }
    }

    offsets
}

/// ���有效 offset 点做线性回归，返回 (斜率, 截距, R²)
fn linear_regression(offsets: &[f64]) -> (f64, f64, f64) {
    let n = offsets.len() as f64;
    if n < 2.0 {
        return (0.0, 0.0, 0.0);
    }

    let sum_x: f64 = (0..offsets.len()).map(|i| i as f64).sum();
    let sum_y: f64 = offsets.iter().sum();
    let sum_xx: f64 = (0..offsets.len()).map(|i| (i as f64).powi(2)).sum();
    let sum_xy: f64 = offsets.iter().enumerate().map(|(i, &y)| i as f64 * y).sum();

    let denom = n * sum_xx - sum_x * sum_x;
    if denom.abs() < 1e-12 {
        return (0.0, 0.0, 0.0);
    }

    let slope = (n * sum_xy - sum_x * sum_y) / denom;
    let intercept = (sum_y - slope * sum_x) / n;

    // 计算 R²
    let mean_y = sum_y / n;
    let ss_tot: f64 = offsets.iter().map(|&y| (y - mean_y).powi(2)).sum();
    let ss_res: f64 = offsets.iter().enumerate()
        .map(|(i, &y)| (y - (slope * i as f64 + intercept)).powi(2))
        .sum();
    let r_squared = if ss_tot > 1e-12 {
        1.0 - ss_res / ss_tot
    } else {
        0.0
    };

    (slope, intercept, r_squared)
}

/// 阶段二：从 offset 序列检测漂移事件
///
/// 对 offset 序列进行形态分析，检测突变（裁剪/插入）和斜坡（拉伸/压缩）。
///
/// # Arguments
/// * `offsets` - offset 序列（None 表示静音窗）
/// * `sample_rate` - 采样率
/// * `hop_ms` - 步进长度（ms），用于计算时间
/// * `segment_index` - 段索引
/// * `config` - 漂移检测配置
///
/// # Returns
/// Vec<WarpingEvent> - 漂移事件列表
pub fn detect_warpings_from_offsets(
    offsets: &[Option<f64>],
    _sample_rate: u32,
    hop_ms: f64,
    segment_index: usize,
    config: &WarpingConfig,
) -> Vec<WarpingEvent> {
    if offsets.is_empty() {
        return vec![];
    }

    // 收集有效 offset（用于分析）
    let valid_offsets: Vec<(usize, f64)> = offsets.iter()
        .enumerate()
        .filter_map(|(i, o)| o.map(|v| (i, v)))
        .collect();

    if valid_offsets.len() < 3 {
        return vec![];
    }

    // 去基线：减去首个有效点的值，归一化为「相对首个窗的偏移变化」
    let baseline = valid_offsets[0].1;
    let relative_offsets: Vec<f64> = valid_offsets.iter()
        .map(|(_, v)| v - baseline)
        .collect();

    // ===== 步骤 1：突变检测（裁剪 / 插入） =====
    let mut jump_events: Vec<(usize, f64)> = Vec::new(); // (窗口索引, 突变幅度)

    for i in 1..relative_offsets.len() {
        let delta = relative_offsets[i] - relative_offsets[i - 1];
        if delta.abs() > config.jump_threshold_ms {
            // 找到突变点，计算前后均值确定幅度
            let before = if i >= 2 {
                (relative_offsets[i - 2] + relative_offsets[i - 1]) / 2.0
            } else {
                relative_offsets[i - 1]
            };
            let after = if i + 1 < relative_offsets.len() {
                (relative_offsets[i] + relative_offsets[i + 1]) / 2.0
            } else {
                relative_offsets[i]
            };
            let jump_magnitude = after - before;
            jump_events.push((i, jump_magnitude));
        }
    }

    // ===== 步骤 2：斜坡检测（拉伸 / 压缩） =====
    let (slope, _intercept, r_squared) = linear_regression(&relative_offsets);

    let is_slope_significant = slope.abs() > config.slope_threshold && r_squared > config.min_r_squared;

    // ===== 步骤 3：方向与子类型判定 =====
    let mut events = Vec::new();

    // 优先检测突变（裁剪/插入更明确）
    if !jump_events.is_empty() {
        // 取最大幅度的突变
        let (jump_idx, jump_magnitude) = jump_events.iter()
            .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
            .unwrap();

        // 计算时间范围
        let start_time_s = (*jump_idx as f64 * hop_ms) / 1000.0;
        let end_time_s = ((*jump_idx + 1) as f64 * hop_ms) / 1000.0;

        // 方向判定：正=后移(插入)，负=前移(裁剪)
        let drift_type = if *jump_magnitude > 0.0 {
            WarpingType::Insertion
        } else {
            WarpingType::Cut
        };

        let drift_ms = jump_magnitude.abs();
        if drift_ms >= config.min_drift_ms {
            events.push(WarpingEvent {
                segment_index,
                start_time_s,
                end_time_s,
                drift_ms,
                drift_type,
            });
        }
    } else if is_slope_significant {
        // 斜坡检测：计算总漂移幅度
        let total_drift_ms = slope * relative_offsets.len() as f64;

        // 方向判定：正=后移(拉伸)，负=前移(压缩)
        let drift_type = if total_drift_ms > 0.0 {
            WarpingType::Stretch
        } else {
            WarpingType::Compress
        };

        let drift_ms = total_drift_ms.abs();
        if drift_ms >= config.min_drift_ms {
            let start_time_s = 0.0;
            let end_time_s = (relative_offsets.len() as f64 * hop_ms) / 1000.0;

            events.push(WarpingEvent {
                segment_index,
                start_time_s,
                end_time_s,
                drift_ms,
                drift_type,
            });
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试：未错位信号，offset 应接近 0
    #[test]
    fn test_offset_series_flat() {
        let sample_rate = 16000u32;
        let duration_ms = 1000u32;
        let samples = (sample_rate as f64 * duration_ms as f64 / 1000.0) as usize;

        // 参考信号：正弦波
        let reference: Vec<f64> = (0..samples)
            .map(|i| (2.0 * std::f64::consts::PI * 440.0 * i as f64 / sample_rate as f64).sin())
            .collect();

        // 录制信号：完全相同
        let degraded = reference.clone();

        let config = WarpingConfig::default();
        let offsets = compute_offset_series(&reference, &degraded, sample_rate, &config);

        // 有效 offset 应该接近 0
        let valid: Vec<f64> = offsets.iter().filter_map(|o| *o).collect();
        if !valid.is_empty() {
            let avg = valid.iter().sum::<f64>() / valid.len() as f64;
            assert!(avg.abs() < 20.0, "未错位信号的 offset 均值应接近 0, 实际: {}", avg);
        }
    }

    /// 测试：含静音窗，验证 None 标记
    #[test]
    fn test_silence_window_skipped() {
        let sample_rate = 16000u32;
        let duration_ms = 500u32;
        let samples = (sample_rate as f64 * duration_ms as f64 / 1000.0) as usize;

        let mut reference = vec![0.0f64; samples];
        // 前半段有声音
        for i in 0..samples/2 {
            reference[i] = (2.0 * std::f64::consts::PI * 440.0 * i as f64 / sample_rate as f64).sin();
        }
        // 后半段静音

        let degraded = reference.clone();

        let config = WarpingConfig::default();
        let offsets = compute_offset_series(&reference, &degraded, sample_rate, &config);

        // 应该有静音窗被标记为 None
        let valid_count = offsets.iter().filter(|o| o.is_some()).count();
        // 至少有部分窗口应���是有效的
        assert!(valid_count < offsets.len(), "静音窗应被标记为 None");
    }

    /// 测试：大幅漂移 < min_drift_ms 应被忽略
    #[test]
    fn test_below_threshold_ignored() {
        let sample_rate = 16000u32;
        let duration_ms = 1000u32;
        let samples = (sample_rate as f64 * duration_ms as f64 / 1000.0) as usize;

        // 参考信号
        let reference: Vec<f64> = (0..samples)
            .map(|i| (2.0 * std::f64::consts::PI * 440.0 * i as f64 / sample_rate as f64).sin())
            .collect();

        // 录制信号：轻微偏移 30ms（小于默认 60ms 阈值）
        let shift_samples = (sample_rate as f64 * 0.030) as usize; // 30ms
        let degraded: Vec<f64> = reference.iter()
            .skip(shift_samples)
            .chain(std::iter::repeat(&0.0))
            .take(samples)
            .cloned()
            .collect();

        let config = WarpingConfig::default();
        let offsets = compute_offset_series(&reference, &degraded, sample_rate, &config);
        let events = detect_warpings_from_offsets(&offsets, sample_rate, config.hop_ms, 0, &config);

        // 应该没有检测到漂移（30ms < 60ms 阈值）
        assert!(events.is_empty(), "小幅漂移应被忽略");
    }
}