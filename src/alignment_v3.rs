//! 批量自适应对齐模块 v3
//!
//! 使用 RMS 能量包络 + VAD 状态机 + 核心区互相关
//! 专门针对会议软件（Opus 等）的语音编解码器优化

/// 对齐结果（与 alignment_v2 兼容）
#[derive(Debug, Clone)]
pub struct AlignmentResult {
    /// 参考音频在录制音频中的起始偏移（采样点数）
    pub offset_samples: usize,
    /// 延迟时间（毫秒）
    pub delay_ms: f64,
    /// 归一化相关系数峰值（0~1，越高越可靠）
    pub confidence: f64,
}

/// 时间段结构
#[derive(Clone, Copy, Debug)]
pub struct TimeSegment {
    /// 起始采样点
    pub start_sample: usize,
    /// 结束采样点
    pub end_sample: usize,
}

/// VAD 状态
#[derive(PartialEq, Debug)]
enum VadState {
    /// 静音状态
    Silence,
    /// 有声状态
    Active,
}

// ============================================================================
// 1. 信号提纯：滑动 RMS 能量包络提取
// ============================================================================

/// 计算音频的滑动 RMS（均方根）能量包络
/// window_size 推荐设为 480 个采样点（在 48kHz 下对应 10 毫秒）
///
/// # 参数
/// - samples: 音频采样点
/// - window_size: 滑动窗口大小（采样点数）
///
/// # 返回
/// 每个窗口的 RMS 能量值
pub fn compute_rms_envelope(samples: &[f64], window_size: usize) -> Vec<f64> {
    if samples.is_empty() || window_size == 0 {
        return Vec::new();
    }
    samples
        .chunks(window_size)
        .map(|chunk| {
            let sum_sq: f64 = chunk.iter().map(|&x| x * x).sum();
            (sum_sq / chunk.len() as f64).sqrt()
        })
        .collect()
}

// ============================================================================
// 2. 区间检索：双向 VAD 状态机粗切
// ============================================================================

/// 基于能量包络在大长条录音中粗切出所有可能包含测试信号的区间
///
/// # 参数
/// - deg_samples: 录制音频采样点
/// - sample_rate: 采样率
///
/// # 返回
/// 所有检测到的潜在测试区间
pub fn segment_long_audio_by_envelope(deg_samples: &[f64], sample_rate: usize) -> Vec<TimeSegment> {
    let window_size = (0.010 * sample_rate as f64) as usize; // 10ms 帧
    let envelope = compute_rms_envelope(deg_samples, window_size);

    // 会议软件极限降噪后底噪极低，门限设为 0.005 极其安全
    let threshold = 0.005;
    // 状态保持窗口（防窒息/换气/卡顿导致的过度误切），设定为 600ms
    let hold_frames = (0.600 / 0.010) as usize;

    let mut segments = Vec::new();
    let mut state = VadState::Silence;
    let mut active_start_frame = 0;
    let mut silence_counter = 0;

    for (frame_idx, &energy) in envelope.iter().enumerate() {
        let is_active = energy > threshold;

        match state {
            VadState::Silence => {
                if is_active {
                    state = VadState::Active;
                    active_start_frame = frame_idx;
                    silence_counter = 0;
                }
            }
            VadState::Active => {
                if !is_active {
                    silence_counter += 1;
                    if silence_counter >= hold_frames {
                        let start_sample = active_start_frame * window_size;
                        // 往前后各冗余多切 300ms 缓冲区，交由下一层核心互相关裁决
                        let safety_margin = (0.3 * sample_rate as f64) as usize;

                        segments.push(TimeSegment {
                            start_sample: start_sample.saturating_sub(safety_margin),
                            end_sample: (frame_idx * window_size) + safety_margin,
                        });
                        state = VadState::Silence;
                    }
                } else {
                    silence_counter = 0;
                }
            }
        }
    }

    // 如果最后仍在 Active 状态，追加最后一段
    if state == VadState::Active {
        let start_sample = active_start_frame * window_size;
        let safety_margin = (0.3 * sample_rate as f64) as usize;
        let end_sample = (envelope.len() * window_size) + safety_margin;

        segments.push(TimeSegment {
            start_sample: start_sample.saturating_sub(safety_margin),
            end_sample: end_sample.min(deg_samples.len()),
        });
    }

    segments
}

// ============================================================================
// 3. 精准对齐：自适应核心区包络互相关
// ============================================================================

/// 扫描音频，定位真正有声的核心声学区间边界
///
/// # 参数
/// - samples: 音频采样点
/// - threshold: 能量阈值，低于此值认为静音
///
/// # 返回
/// (有声起点, 有声终点) 采样索引
pub fn locate_acoustic_core(samples: &[f64], threshold: f64) -> (usize, usize) {
    let start_voice = samples.iter().position(|&x| x.abs() > threshold).unwrap_or(0);
    let end_voice = samples.iter().rposition(|&x| x.abs() > threshold).unwrap_or(samples.len());
    (start_voice, end_voice)
}

/// 利用标准音频的纯声音核心包络，在长录音区间内精细对齐
///
/// # 参数
/// - pure_voice_ref: 参考音频的纯有声核心段
/// - deg_long_audio: 录制音频（全长）
/// - segment: VAD 状态机粗切的区间
/// - sample_rate: 采样率
///
/// # 返回
/// 有声核心在录制音频中的起始采样点（相对 deg_long_audio）
pub fn align_core_by_envelope(
    pure_voice_ref: &[f64],
    deg_long_audio: &[f64],
    segment: &TimeSegment,
    sample_rate: usize
) -> Option<usize> {
    let window_size = (0.010 * sample_rate as f64) as usize; // 10ms 帧

    // 计算参考音频核心段的包络
    let ref_env = compute_rms_envelope(pure_voice_ref, window_size);

    if ref_env.is_empty() {
        return None;
    }

    // 扩大搜索区间：在 segment 基础上往外扩展 1 秒
    // 防止 segment 刚好覆盖整个音频时没有搜索空间
    let margin = sample_rate;
    let search_start = segment.start_sample.saturating_sub(margin);
    let search_end = (segment.end_sample + margin).min(deg_long_audio.len());

    if search_end <= search_start {
        return None;
    }

    let search_src = &deg_long_audio[search_start..search_end];
    let deg_env = compute_rms_envelope(search_src, window_size);

    if deg_env.len() < ref_env.len() {
        return None;
    }

    let search_range_frames = deg_env.len() - ref_env.len();
    let mut max_correlation = f64::NEG_INFINITY;
    let mut best_frame_offset = 0;

    // 在包络域执行点积滑动互相关
    for offset in 0..search_range_frames {
        let mut current_sum = 0.0;
        for i in 0..ref_env.len() {
            current_sum += ref_env[i] * deg_env[offset + i];
        }
        if current_sum > max_correlation {
            max_correlation = current_sum;
            best_frame_offset = offset;
        }
    }

    // 转换回采样点坐标（相对于搜索区间起点）
    let local_sample_offset = best_frame_offset * window_size;
    // 再加上搜索区间在全长中的偏移
    Some(search_start + local_sample_offset)
}

// ============================================================================
// 4. 置信度计算：归一化互相关
// ============================================================================

/// 计算两个音频片段的包络归一化互相关作为置信度
fn compute_envelope_correlation(
    ref_audio: &[f64],
    deg_audio: &[f64],
    sample_rate: usize,
) -> f64 {
    let window_size = (0.010 * sample_rate as f64) as usize;
    let ref_env = compute_rms_envelope(ref_audio, window_size);
    let deg_env = compute_rms_envelope(deg_audio, window_size);

    if ref_env.is_empty() || deg_env.is_empty() {
        return 0.0;
    }

    // 取等长部分
    let min_len = ref_env.len().min(deg_env.len());
    if min_len == 0 {
        return 0.0;
    }

    let ref_slice = &ref_env[..min_len];
    let deg_slice = &deg_env[..min_len];

    // 计算均值
    let ref_mean = ref_slice.iter().sum::<f64>() / min_len as f64;
    let deg_mean = deg_slice.iter().sum::<f64>() / min_len as f64;

    // 计算标准差
    let ref_var: f64 = ref_slice.iter().map(|x| (x - ref_mean).powi(2)).sum::<f64>() / min_len as f64;
    let deg_var: f64 = deg_slice.iter().map(|x| (x - deg_mean).powi(2)).sum::<f64>() / min_len as f64;

    let ref_std = ref_var.sqrt();
    let deg_std = deg_var.sqrt();

    if ref_std < 1e-10 || deg_std < 1e-10 {
        return 0.0;
    }

    // 计算相关系数
    let mut correlation = 0.0;
    for i in 0..min_len {
        correlation += (ref_slice[i] - ref_mean) * (deg_slice[i] - deg_mean);
    }
    correlation /= min_len as f64 * ref_std * deg_std;

    correlation.clamp(0.0, 1.0)
}

// ============================================================================
// 5. 主入口：批量自适应对齐
// ============================================================================

/// 使用 RMS 包络 + VAD 状态机 + 核心区互相关进行多峰检测
///
/// # 参数
/// - reference: 参考音频采样点
/// - degraded: 录制音频采样点
/// - sample_rate: 采样率
/// - confidence_threshold: 置信度阈值
///
/// # 返回
/// 所有检测到的参考音频出现位置
pub fn find_all_alignments_v3(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    confidence_threshold: f64,
) -> Vec<AlignmentResult> {
    let sample_rate = sample_rate as usize;

    // 1. VAD 状态机全局粗切潜在录制循环
    let segments = segment_long_audio_by_envelope(degraded, sample_rate);
    println!("🔔 VAD 状态机共捕获到 {} 个潜在受测音频区间", segments.len());

    // 调试：打印每个区间
    for (i, seg) in segments.iter().enumerate() {
        println!("      区间{}: start={}, end={}, 长度={}",
                 i + 1, seg.start_sample, seg.end_sample,
                 seg.end_sample - seg.start_sample);
    }

    if segments.is_empty() {
        println!("[!] 未检测到任何潜在测试区间");
        return vec![];
    }

    // 2. 提纯标准音频：揪出中间纯有声核心区
    let (v_start, v_end) = locate_acoustic_core(reference, 0.005);
    let pure_voice_ref = &reference[v_start..v_end];

    println!("🔔 参考音频核心区: 样本 {}-{} (时长 {:.2}s), 长度={}",
             v_start, v_end, (v_end - v_start) as f64 / sample_rate as f64,
             pure_voice_ref.len());

    if pure_voice_ref.is_empty() {
        println!("[!] 参考音频提取核心区失败");
        return vec![];
    }

    let mut results = Vec::new();

    // 3. 对每个潜在区间进行核心区互相关对齐
    for (seg_idx, segment) in segments.iter().enumerate() {
        println!("[*] 处理区间{}: start={}, end={}", seg_idx + 1, segment.start_sample, segment.end_sample);

        // 用纯有声区包络去录音中对表
        match align_core_by_envelope(pure_voice_ref, degraded, segment, sample_rate) {
            Some(deg_voice_start) => {
                println!("      核心区对齐: deg_voice_start={}", deg_voice_start);

                // 4. 【自适应单块反推核心】：从有声起点往前倒退前静音长度
                // 得到整段的绝对对齐切片起点
                println!("      检查: deg_voice_start={}, v_start={}", deg_voice_start, v_start);

                if deg_voice_start >= v_start {
                    let absolute_start = deg_voice_start - v_start;
                    let absolute_end = (absolute_start + reference.len()).min(degraded.len());

                    println!("      绝对坐标: start={}, end={}, 裁剪后={}, deg_len={}, ref_len={}",
                             absolute_start, absolute_start + reference.len(),
                             absolute_end, degraded.len(), reference.len());

                    if absolute_end <= degraded.len() {
                        println!("      ✅ 条件通过，进入置信度计算...");
                        // 5. 计算置信度（归一化互相关）
                        let confidence = compute_envelope_correlation(
                            pure_voice_ref,
                            &degraded[deg_voice_start..deg_voice_start + pure_voice_ref.len().min(degraded.len() - deg_voice_start)],
                            sample_rate,
                        );

                        // 额外验证：整段的对齐质量
                        let full_confidence = compute_envelope_correlation(
                            reference,
                            &degraded[absolute_start..absolute_end.min(degraded.len())],
                            sample_rate,
                        );

                        println!("      置信度: 核心区={:.3}, 整段={:.3}", confidence, full_confidence);

                        // 取两者的较小值作为最终置信度（更严格）
                        let final_confidence = confidence.min(full_confidence);

                        if final_confidence >= confidence_threshold {
                            results.push(AlignmentResult {
                                offset_samples: absolute_start,
                                delay_ms: absolute_start as f64 / sample_rate as f64 * 1000.0,
                                confidence: final_confidence,
                            });

                            println!("      ✅ 添加: 偏移 {:.3}s, 置信度 {:.1}%",
                                     results.last().unwrap().delay_ms / 1000.0,
                                     final_confidence * 100.0);
                        } else {
                            println!("      ❌ 置信度 {:.3} < {:.3}", final_confidence, confidence_threshold);
                        }
                    } else {
                        println!("      ❌ absolute_end {} > degraded.len() {}", absolute_end, degraded.len());
                    }
                } else {
                    println!("      ❌ deg_voice_start {} < v_start {}", deg_voice_start, v_start);
                }
            }
            None => {
                println!("      ❌ align_core_by_envelope 返回 None");
            }
        }
    }

    // 去重 + 排序（按置信度降序）
    results.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));

    // 最小峰间距：参考长度的 80%
    let min_gap = (reference.len() as f64 * 0.8) as usize;
    let mut deduped = Vec::new();
    for r in results {
        let too_close = deduped.iter().any(|existing: &AlignmentResult| {
            (r.offset_samples as isize - existing.offset_samples as isize).unsigned_abs() < min_gap
        });
        if !too_close {
            deduped.push(r);
        }
    }

    // 按位置排序
    deduped.sort_by_key(|r| r.offset_samples);

    // 限制最大数量
    if deduped.len() > 50 {
        deduped.truncate(50);
    }

    println!("🔔 有效对齐位置: {} 个", deduped.len());

    deduped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rms_envelope() {
        // 生成 1kHz 正弦波
        let sample_rate = 48000;
        let duration = 1.0; // 1秒
        let freq = 1000.0;
        let samples: Vec<f64> = (0..(sample_rate as f64 * duration) as usize)
            .map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / sample_rate as f64).sin())
            .collect();

        let envelope = compute_rms_envelope(&samples, 480); // 10ms 窗口

        // 检查包络长度
        assert!(!envelope.is_empty());
        // RMS 值应该在 0.707 左右（正弦波的 RMS = amplitude / sqrt(2)，amplitude=1）
        let avg_rms = envelope.iter().sum::<f64>() / envelope.len() as f64;
        assert!((avg_rms - 0.707).abs() < 0.01);
    }

    #[test]
    fn test_locate_acoustic_core() {
        // 创建带静音���音频：1000 样本静音 + 10000 样本正弦 + 1000 样本静音
        let mut samples = vec![0.0; 1000];
        samples.extend((0..10000).map(|i| (i as f64 * 0.001).sin()));
        samples.extend(vec![0.0; 1000]);

        let (start, end) = locate_acoustic_core(&samples, 0.001);

        // 核心区应该从 1000 开始，到 11000 结束
        assert!(start >= 990 && start <= 1010);
        assert!(end >= 10990 && end <= 11010);
    }

    #[test]
    fn test_segment_audio() {
        // 创建测试音频：静音 1s + 有声 3s + 静音 1s + 有声 2s + 静音 1s
        let sample_rate = 48000;
        let samples: Vec<f64> = vec![0.0; sample_rate] // 1s 静音
            .into_iter()
            .chain((0..sample_rate * 3).map(|i| (i as f64 * 0.01).sin())) // 3s 有声
            .chain(vec![0.0; sample_rate]) // 1s 静音
            .chain((0..sample_rate * 2).map(|i| (i as f64 * 0.01).sin())) // 2s 有声
            .chain(vec![0.0; sample_rate]) // 1s 静音
            .collect();

        let segments = segment_long_audio_by_envelope(&samples, sample_rate);

        // 应该检测到至少 2 个有声音频
        assert!(segments.len() >= 2);
    }
}