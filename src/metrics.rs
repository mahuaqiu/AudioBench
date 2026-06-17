//! 音频异常检测模块
//! 
//! 异常检测：时域中断、时轴漂移、频谱损伤
//! - 时域中断 (Audio Dropout)：录制信号能量断崖式下跌
//! - 时轴漂移 (Time Warping)：多段对齐间距不一致
//! - 频谱损伤 (Spectral Artifacts)：频域结构被破坏但时域能量正常

/// 音频异常检测报告
#[derive(Debug, Clone, serde::Serialize)]
pub struct AudioAnomalyReport {
    /// 是否存在任何异常
    pub has_anomaly: bool,

    /// 时域中断（异常静音/丢包）事件列表
    pub dropouts: Vec<DropoutEvent>,

    /// 时域中断总时长 (ms)
    pub dropout_duration_ms: f64,

    /// 时轴漂移事件列表
    pub warpings: Vec<WarpingEvent>,

    /// 时轴漂移总时长 (ms)
    pub warping_duration_ms: f64,

    /// 频谱损伤严重程度 (0.0=无损伤, 1.0=极度损伤)
    pub spectral_artifacts_score: f64,

    /// 麻烦损伤事件列表（低相似度时间段）
    pub spectral_artifacts: Vec<SpectralArtifactEvent>,

    /// 内容截断/裁剪事件列表（段实际长度明显短于参考）
    pub truncations: Vec<TruncationEvent>,

    /// 内容截断总时长 (ms)
    pub truncation_duration_ms: f64,
}

/// 时域中断事件（异常静音/丢包）
#[derive(Debug, Clone, serde::Serialize)]
pub struct DropoutEvent {
    /// 中断起始时间 (秒)
    pub start_time_s: f64,
    /// 中断结束时间 (秒)
    pub end_time_s: f64,
    /// 中断持续时长 (毫秒)
    pub duration_ms: f64,
    /// 参考信号在该时间段的 RMS 能量
    pub ref_rms: f64,
    /// 录制信号在该时间段的 RMS 能量
    pub deg_rms: f64,
    /// 能量衰减比例 (deg_rms / ref_rms)，0=完全中断
    pub attenuation_ratio: f64,
}

/// 内容截断/裁剪事件（段实际长度明显短于参考）
#[derive(Debug, Clone, serde::Serialize)]
pub struct TruncationEvent {
    /// 涉及的段索引
    pub segment_index: usize,
    /// 参考音频该段时长 (ms)
    pub ref_duration_ms: f64,
    /// 录制音频该段实际时长 (ms)
    pub deg_duration_ms: f64,
    /// 截断时长 = ref - deg (ms)，正值表示缺失
    pub truncation_ms: f64,
}

/// 频谱损伤事件
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpectralArtifactEvent {
    /// 对应的段索引
    pub segment_index: usize,
    /// Patch 索引
    pub patch_index: usize,
    /// Patch 相似度 (0-1)
    pub patch_similarity: f64,
    /// 参考音频时间段 (起始秒)
    pub ref_start_s: f64,
    /// 参考音频时间段 (结束秒)
    pub ref_end_s: f64,
}

/// SNR（信噪比）计算结果（保留兼容）
#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)]
pub struct SnrResult {
    pub snr_db: f64,
    pub ref_energy: f64,
    pub noise_energy: f64,
}

#[allow(dead_code)]
pub fn compute_snr(reference: &[f64], degraded: &[f64]) -> SnrResult {
    assert_eq!(reference.len(), degraded.len(), "参考和录制音频长度必须一致");
    let ref_energy: f64 = reference.iter().map(|x| x * x).sum();
    let noise: Vec<f64> = reference.iter()
        .zip(degraded.iter())
        .map(|(r, d)| d - r)
        .collect();
    let noise_energy: f64 = noise.iter().map(|x| x * x).sum();
    let snr_db = if noise_energy > 0.0 && ref_energy > 0.0 {
        10.0 * (ref_energy / noise_energy).log10()
    } else if ref_energy > 0.0 {
        99.0
    } else {
        0.0
    };
    SnrResult { snr_db, ref_energy, noise_energy }
}

/// 幅值统计结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct LevelResult {
    pub rms: f64,
    pub peak: f64,
    pub rms_dbfs: f64,
    pub peak_dbfs: f64,
    pub clipped_samples: usize,
}

/// 计算音频幅值统计
pub fn compute_level_stats(samples: &[f64]) -> LevelResult {
    let rms = (samples.iter().map(|x| x * x).sum::<f64>() / samples.len() as f64).sqrt();
    let peak = samples.iter().map(|x| x.abs()).fold(0.0f64, f64::max);
    let rms_dbfs = if rms > 0.0 { 20.0 * (rms).log10() } else { -99.0 };
    let peak_dbfs = if peak > 0.0 { 20.0 * (peak).log10() } else { -99.0 };
    let clipped_samples = samples.iter().filter(|&&x| x.abs() >= 0.99).count();
    LevelResult { rms, peak, rms_dbfs, peak_dbfs, clipped_samples }
}

// ============================================================
// 维度一：时域中断检测 (Audio Dropout / Interruption)
// ============================================================

/// 时域中断检测参数
pub struct DropoutDetectorConfig {
    /// 静音阈值（RMS 低于此值判定为静音）
    pub silence_threshold: f64,
    /// 最短中断持续时间 (ms)，低于此不判定为中断。
    /// 语义：参考信号在该时间段有声，但录制信号静音超过此时长才算中断。
    pub min_duration_ms: f64,
    /// 相对于参考信号的能量衰减比阈值（deg_rms / ref_rms < 此值判定为衰减）
    pub attenuation_threshold: f64,
    /// 分帧大小（采样点数）
    pub frame_size: usize,
    /// 帧移（采样点数）
    pub hop_size: usize,
    /// 帧级对齐搜索窗口（采样点数）。
    /// 每帧 RMS 比较前，先在录制端 ±此窗口范围内做局部互相关，
    /// 让参考帧对齐到录制帧，消除裁剪/边界错位导致的误判。
    pub frame_align_search_samples: usize,
}

impl Default for DropoutDetectorConfig {
    fn default() -> Self {
        Self {
            silence_threshold: 0.005,
            min_duration_ms: 60.0,
            attenuation_threshold: 0.05,
            frame_size: 320,
            hop_size: 160,
            frame_align_search_samples: 3200, // 200ms @ 16kHz，按采样率重算
        }
    }
}

/// 根据采样率创建默认配置（自动适配不同采样率）
impl DropoutDetectorConfig {
    pub fn for_sample_rate(sample_rate: u32) -> Self {
        let frame_samples = (sample_rate as f64 * 0.020) as usize; // 20ms 帧长
        // min_duration_ms：语音模式(16kHz) 60ms，音频模式(48kHz) 120ms
        let min_duration_ms = if sample_rate <= 16000 { 60.0 } else { 120.0 };
        // 帧级对齐搜索窗口 ±200ms
        let frame_align_search_samples = (sample_rate as f64 * 0.200) as usize;
        Self {
            silence_threshold: 0.005,
            min_duration_ms,
            attenuation_threshold: 0.05,
            frame_size: frame_samples,
            hop_size: frame_samples / 2,
            frame_align_search_samples,
        }
    }
}

/// 计算一段采样的 RMS 能量
fn compute_rms(samples: &[f64]) -> f64 {
    if samples.is_empty() { return 0.0; }
    (samples.iter().map(|x| x * x).sum::<f64>() / samples.len() as f64).sqrt()
}

/// 在录制信号指定位置附近的小窗口内，做局部归一化互相关，
/// 找到参考帧在录制端的最佳对齐偏移（采样点数）。
///
/// 返回相对于 `deg_center` 的相对偏移（可为负，调用方自行 clamp）。
/// 这样可以让参考帧与录制帧在 RMS 比较前先对齐到帧级，
/// 消除裁剪/边界错位导致的"参考有声、录制恰好落在过渡区"误判。
fn local_frame_align_offset(
    ref_frame: &[f64],
    degraded: &[f64],
    deg_center: usize,
    search_radius: usize,
) -> isize {
    let frame_len = ref_frame.len();
    if frame_len == 0 || degraded.len() < frame_len {
        return 0;
    }
    let ref_energy: f64 = ref_frame.iter().map(|x| x * x).sum();

    let lo = deg_center.saturating_sub(search_radius);
    let hi = (deg_center + search_radius).min(degraded.len().saturating_sub(frame_len));
    if hi <= lo || ref_energy < 1e-12 {
        return 0;
    }

    let mut best_offset: isize = 0;
    let mut best_conf: f64 = 0.0;
    for k in lo..=hi {
        let seg = &degraded[k..k + frame_len];
        let seg_energy: f64 = seg.iter().map(|x| x * x).sum();
        let denom = (ref_energy * seg_energy).sqrt();
        if denom < 1e-12 {
            continue;
        }
        let sum: f64 = ref_frame.iter().zip(seg.iter()).map(|(r, &d)| r * d).sum();
        let conf = (sum / denom).abs().min(1.0);
        if conf > best_conf {
            best_conf = conf;
            best_offset = k as isize - deg_center as isize;
        }
    }
    best_offset
}

/// 维度一：时域中断检测
/// 
/// # Arguments
/// *  - 参考音频样本
/// *  - 录制音频（待检测段）样本
/// *  - 采样率
/// *  - 检测配置
/// *  - 有效长度（排除补零尾段），0 表示使用整个数组
pub fn detect_dropouts(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    config: &DropoutDetectorConfig,
    valid_len: usize,
) -> Vec<DropoutEvent> {
    let frame_size = config.frame_size.min(reference.len());
    let hop_size = config.hop_size.max(1);
    let min_samples = (sample_rate as f64 * config.min_duration_ms / 1000.0) as usize;
    
    let mut events = Vec::new();
    let mut in_dropout = false;
    let mut dropout_start_frame = 0usize;
    let mut dropout_ref_rms_acc = 0.0f64;
    let mut dropout_deg_rms_acc = 0.0f64;
    let mut dropout_frame_count = 0usize;
    
    // 有效检测范围：排除补零尾段
    let max_valid_idx = if valid_len > 0 { valid_len } else { reference.len().min(degraded.len()) };
    
    let mut frame_idx = 0;
    while frame_idx + frame_size <= max_valid_idx {
        let ref_frame = &reference[frame_idx..frame_idx + frame_size];

        // 帧级对齐：在录制端 ±frame_align_search_samples/4 范围内找最佳偏移，
        // 让参考帧对齐到录制帧，消除裁剪/边界错位导致的误判。
        // 搜索半径取配置窗口的 1/4（即 ±50ms @ 默认 200ms 配置），避免对齐到无关内容。
        let align_radius = (config.frame_align_search_samples / 4).max(1);
        let deg_center = frame_idx.min(degraded.len().saturating_sub(frame_size));
        let rel_offset = local_frame_align_offset(ref_frame, degraded, deg_center, align_radius);
        let aligned_deg_start = (deg_center as isize + rel_offset) as usize;
        // 取对齐后的录制帧（保证不越界）
        let deg_frame = if aligned_deg_start + frame_size <= degraded.len() {
            &degraded[aligned_deg_start..aligned_deg_start + frame_size]
        } else {
            &degraded[frame_idx..frame_idx + frame_size]
        };

        let ref_rms = compute_rms(ref_frame);
        let deg_rms = compute_rms(deg_frame);

        // 语义：必须参考信号在该帧有声，且录制信号静音/能量严重衰减才算中断。
        // 参考本身静音（如正常停顿）不报中断。
        let ref_has_sound = ref_rms > config.silence_threshold * 2.0;
        let deg_silent = deg_rms < config.silence_threshold;
        let energy_attenuation = if ref_rms > 0.0 {
            deg_rms / ref_rms < config.attenuation_threshold
        } else {
            false
        };

        let is_dropout = ref_has_sound && (deg_silent || energy_attenuation);
        
        if is_dropout {
            if !in_dropout {
                in_dropout = true;
                dropout_start_frame = frame_idx;
                dropout_ref_rms_acc = 0.0;
                dropout_deg_rms_acc = 0.0;
                dropout_frame_count = 0;
            }
            dropout_ref_rms_acc += ref_rms;
            dropout_deg_rms_acc += deg_rms;
            dropout_frame_count += 1;
        } else {
            if in_dropout {
                let duration_samples = frame_idx - dropout_start_frame;
                if duration_samples >= min_samples {
                    let start_time = dropout_start_frame as f64 / sample_rate as f64;
                    let end_time = frame_idx as f64 / sample_rate as f64;
                    let avg_ref_rms = if dropout_frame_count > 0 { dropout_ref_rms_acc / dropout_frame_count as f64 } else { 0.0 };
                    let avg_deg_rms = if dropout_frame_count > 0 { dropout_deg_rms_acc / dropout_frame_count as f64 } else { 0.0 };
                    let attenuation = if avg_ref_rms > 0.0 { avg_deg_rms / avg_ref_rms } else { 0.0 };
                    events.push(DropoutEvent {
                        start_time_s: start_time,
                        end_time_s: end_time,
                        duration_ms: (end_time - start_time) * 1000.0,
                        ref_rms: avg_ref_rms,
                        deg_rms: avg_deg_rms,
                        attenuation_ratio: attenuation,
                    });
                }
                in_dropout = false;
            }
        }
        
        frame_idx += hop_size;
    }
    
    if in_dropout {
        let duration_samples = frame_idx.saturating_sub(dropout_start_frame);
        if duration_samples >= min_samples {
            let end_time = frame_idx as f64 / sample_rate as f64;
            let start_time = dropout_start_frame as f64 / sample_rate as f64;
            let avg_ref_rms = if dropout_frame_count > 0 { dropout_ref_rms_acc / dropout_frame_count as f64 } else { 0.0 };
            let avg_deg_rms = if dropout_frame_count > 0 { dropout_deg_rms_acc / dropout_frame_count as f64 } else { 0.0 };
            let attenuation = if avg_ref_rms > 0.0 { avg_deg_rms / avg_ref_rms } else { 0.0 };
            events.push(DropoutEvent {
                start_time_s: start_time,
                end_time_s: end_time,
                duration_ms: (end_time - start_time) * 1000.0,
                ref_rms: avg_ref_rms,
                deg_rms: avg_deg_rms,
                attenuation_ratio: attenuation,
            });
        }
    }
    
    events
}

// ============================================================
// 维度二：时轴漂移检测 (Time Warping)
// ============================================================

/// 时轴漂移子类型
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq)]
pub enum WarpingType {
    /// 裁剪：内容缺失，前移
    Cut,
    /// 插入：内容重复/新增，后移
    Insertion,
    /// 拉伸：匀速变慢，后移
    Stretch,
    /// 压缩：匀速变快，前移
    Compress,
}

impl WarpingType {
    /// 获取子类型的中文描述
    pub fn chinese(&self) -> &'static str {
        match self {
            WarpingType::Cut => "裁剪",
            WarpingType::Insertion => "插入",
            WarpingType::Stretch => "拉伸",
            WarpingType::Compress => "压缩",
        }
    }
}

/// 漂移检测配置
#[derive(Debug, Clone, Copy)]
pub struct WarpingConfig {
    /// 滑窗长度 (ms)
    pub window_ms: f64,
    /// 步进长度 (ms)
    pub hop_ms: f64,
    /// 搜索半径 (ms)
    pub search_radius_ms: f64,
    /// 静音阈值（RMS 低于此值判定为静音）
    pub silence_threshold: f64,
    /// 突变阈值 (ms)：相邻 offset 差超过此值判定为突变
    pub jump_threshold_ms: f64,
    /// 斜坡阈值：每秒漂移超过此值判定为渐变
    pub slope_threshold: f64,
    /// 最小漂移幅度 (ms)：drift_ms 绝对值需 >= 此值才报异常
    pub min_drift_ms: f64,
    /// 线性回归 R² 门槛：斜坡检测需要 R² >= 此值
    pub min_r_squared: f64,
}

impl Default for WarpingConfig {
    fn default() -> Self {
        Self {
            window_ms: 200.0,
            hop_ms: 100.0,
            search_radius_ms: 300.0,
            silence_threshold: 0.005, // 复用中断检测的静音阈值
            jump_threshold_ms: 80.0,
            slope_threshold: 0.3, // 30ms/s
            min_drift_ms: 60.0,
            min_r_squared: 0.7,
        }
    }
}

impl WarpingConfig {
    /// 根据采样率创建默认配置
    pub fn for_sample_rate(_sr: u32) -> Self {
        // 参数已针对 16kHz 优化，音频模式可后续调参
        Self::default()
    }
}

/// 时轴漂移事件（重构后）
#[derive(Debug, Clone, serde::Serialize)]
pub struct WarpingEvent {
    /// 涉及的段索引
    pub segment_index: usize,
    /// 漂移起始时间（秒，基于参考时间轴）
    pub start_time_s: f64,
    /// 漂移结束时间（秒，基于参考时间轴）
    pub end_time_s: f64,
    /// 总漂移幅度（ms，带符号：+后移，-前移）
    pub drift_ms: f64,
    /// 子类型标签
    pub drift_type: WarpingType,
}
// ============================================================
// 维度二补充：内容截断/裁剪检测 (Truncation)
// ============================================================

/// 内容截断检测参数
#[derive(Debug, Clone, Copy)]
pub struct TruncationThreshold {
    /// 最短截断时长 (ms)：段实际长度比参考短超过此值才报截断
    pub min_truncation_ms: f64,
}

impl Default for TruncationThreshold {
    fn default() -> Self {
        Self {
            min_truncation_ms: 60.0, // 与中断检测 min_duration_ms 对齐
        }
    }
}

/// 维度二补充：内容截断/裁剪检测
///
/// 直接比对每段**实际音频长度**（不含末尾补零）与参考长度。
/// 专门捕获"少量时域裁剪"这类三个维度都漏掉的异常：
/// - 中断检测：因 valid_len 把补零区切掉，裁剪造成的"参考有声、录制静音（补零）"
///   不进入检测循环（见 detect_dropouts 的 valid_len 逻辑）。
/// - 频谱检测：裁剪通常只影响末尾 1 个 patch，被 exclude_edge_patches 排除。
/// - 漂移检测：只看段间间距，对段内裁剪无感知。
///
/// 这里绕开上述过滤，直接看段长度差异，是最可靠的裁剪信号。
///
/// # Arguments
/// * `seg_actual_samples` - 各段实际音频样本数（不含补零），已按段顺序
/// * `ref_samples` - 参考音频总样本数
/// * `sample_rate` - 采样率
/// * `threshold` - 阈值
pub fn detect_truncation(
    seg_actual_samples: &[usize],
    ref_samples: usize,
    sample_rate: u32,
    threshold: TruncationThreshold,
) -> Vec<TruncationEvent> {
    if ref_samples == 0 || sample_rate == 0 {
        return vec![];
    }
    let ref_ms = ref_samples as f64 / sample_rate as f64 * 1000.0;
    // [DIAG] 截断检测内部基准（核对与 main.rs DIAG 是否一致）
    eprintln!("[DIAG] detect_truncation 内部: ref_samples={} → ref_ms={:.1}ms, 输入段数={}, 阈值 trunc>={}ms",
              ref_samples, ref_ms, seg_actual_samples.len(), threshold.min_truncation_ms);

    let mut events = Vec::new();
    for (seg_idx, &actual) in seg_actual_samples.iter().enumerate() {
        if actual >= ref_samples {
            continue; // 段不短于参考，无截断
        }
        let deg_ms = actual as f64 / sample_rate as f64 * 1000.0;
        let truncation_ms = ref_ms - deg_ms;
        if truncation_ms >= threshold.min_truncation_ms {
            events.push(TruncationEvent {
                segment_index: seg_idx,
                ref_duration_ms: ref_ms,
                deg_duration_ms: deg_ms,
                truncation_ms,
            });
        }
    }
    events
}

// ============================================================
// 维度三：频谱损伤检测 (Spectral Artifacts)
// ============================================================

/// 维度三：频谱损伤检测
///
/// # Arguments
/// * `patch_sims` - 各段的 patch 相似度列表
/// * `artifact_threshold` - 相似度低于此值判为损伤 patch
/// * `exclude_edge_patches` - 是否排除每段的首尾 patch（边界效应：补零/能量过渡）
pub fn detect_spectral_artifacts(
    patch_sims: &[Vec<crate::visqol::PatchSimilarityResult>],
    artifact_threshold: f64,
    exclude_edge_patches: bool,
) -> (f64, Vec<SpectralArtifactEvent>) {
    if patch_sims.is_empty() {
        return (0.0, vec![]);
    }

    let mut all_artifacts = Vec::new();
    let mut total_low_count = 0usize;
    let mut total_patch_count = 0usize;

    for (seg_idx, patches) in patch_sims.iter().enumerate() {
        if patches.is_empty() {
            continue;
        }
        // 排除首尾 patch（边界效应）：只有 patch 数 >= 4 才有意义排除
        let (start, end) = if exclude_edge_patches && patches.len() >= 4 {
            (1usize, patches.len() - 1)
        } else {
            (0usize, patches.len())
        };

        for patch_idx in start..end {
            let patch = &patches[patch_idx];
            total_patch_count += 1;
            if patch.similarity < artifact_threshold {
                total_low_count += 1;
                all_artifacts.push(SpectralArtifactEvent {
                    segment_index: seg_idx,
                    patch_index: patch_idx,
                    patch_similarity: patch.similarity,
                    ref_start_s: patch.ref_patch_start_time,
                    ref_end_s: patch.ref_patch_end_time,
                });
            }
        }
    }

    let score = if total_patch_count > 0 {
        total_low_count as f64 / total_patch_count as f64
    } else {
        0.0
    };

    (score, all_artifacts)
}

// ============================================================
// 综合异常检测入口
// ============================================================

/// 综合异常检测参数
#[allow(dead_code)]
pub struct AnomalyDetectConfig {
    pub dropout_config: DropoutDetectorConfig,
    pub warping_config: WarpingConfig,
    pub artifact_threshold: f64,
    pub truncation_threshold: TruncationThreshold,
}

impl Default for AnomalyDetectConfig {
    fn default() -> Self {
        Self {
            dropout_config: DropoutDetectorConfig::default(),
            warping_config: WarpingConfig::default(),
            artifact_threshold: 0.4,
            truncation_threshold: TruncationThreshold::default(),
        }
    }
}

/// 综合音频异常检测
#[allow(dead_code)]
pub fn detect_anomalies(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    _alignment_offsets_s: &[f64],
    _ref_duration: f64,
    patch_sims: &[Vec<crate::visqol::PatchSimilarityResult>],
    config: &AnomalyDetectConfig,
) -> AudioAnomalyReport {
    // ���度一：时域中断
    let dropouts = detect_dropouts(reference, degraded, sample_rate, &config.dropout_config, 0);
    let dropout_duration_ms: f64 = dropouts.iter().map(|e| e.duration_ms).sum();

    // 维度二：时轴漂移（现在由 main.rs 中的 time_warping 模块独立检测）
    // 这里留空，由调用方单独填充
    let warpings: Vec<WarpingEvent> = vec![];
    let warping_duration_ms: f64 = 0.0;

    // 维度三：频谱损伤
    let (spectral_artifacts_score, spectral_artifacts) =
        detect_spectral_artifacts(patch_sims, config.artifact_threshold, true);

    // 维度二补充：内容截断（这里无法获得各段实际长度，留空，由调用方单独填充）
    let truncations: Vec<TruncationEvent> = vec![];
    let truncation_duration_ms: f64 = 0.0;

    let has_anomaly = !dropouts.is_empty()
        || !warpings.is_empty()
        || !truncations.is_empty()
        || spectral_artifacts_score > 0.25;

    AudioAnomalyReport {
        has_anomaly,
        dropouts,
        dropout_duration_ms,
        warpings,
        warping_duration_ms,
        spectral_artifacts_score,
        spectral_artifacts,
        truncations,
        truncation_duration_ms,
    }
}
