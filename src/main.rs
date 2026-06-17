//! AudioBench - 音频质量评估工具
//!
//! 集成官方 ViSQOL 进行音频质量评估，单 EXE 运行。
//! 编译时嵌入 visqol 二进制，运行时自动释放到临时目录。
//! 使用方法:
//!   audio_bench --reference ref.wav --recorded rec.wav

mod alignment;
mod alignment_v2;
mod audio_io;
mod metrics;
mod visqol;
mod report;
mod html_report;
mod time_warping;
mod dnsmos;

use clap::Parser;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(name = "audio_bench", version = "0.1.0", about = "音频质量评估工具")]
struct Args {
    /// 参考音频文件路径（WAV 格式）
    #[clap(long = "reference", short = 'r', required = true)]
    reference: PathBuf,

    /// 录制音频文件路径（WAV 格式）
    #[clap(long = "recorded", short = 'c', required = true)]
    recorded: PathBuf,

    /// 使用音频模式 (48kHz)，默认使用语音模式 (16kHz)
    #[clap(long = "audio")]
    audio: bool,

    /// 输出 JSON 报告文件路径（可选）
    #[clap(long = "output", short = 'o')]
    output: Option<PathBuf>,

    #[clap(long = "html")]
    html: Option<PathBuf>,
}

// 查找实际音频末尾：找到最后一个能量明显非零的采样点
// 返回实际有信号的样本数（不含末尾静音/补零）
fn find_actual_audio_end(samples: &[f64], sample_rate: u32) -> usize {
    if samples.is_empty() {
        return 0;
    }
    
    // 能量阈值：RMS 低于此值认为已结束
    let energy_threshold = 0.001;
    // 末尾静音判断：连续这么多采样点能量都低于阈值则认为音频结束
    let silence_window = (sample_rate as f64 * 0.020) as usize; // 20ms
    
    let mut silent_count = 0;
    let window_size = silence_window.max(1);
    
    // 从末尾向前遍历
    for i in (0..samples.len()).rev() {
        // 计算当前窗口的 RMS
        let window_start = i.saturating_sub(window_size - 1);
        let window = &samples[window_start..=i];
        if window.is_empty() { break; }
        
        let rms = (window.iter().map(|&x| x * x).sum::<f64>() / window.len() as f64).sqrt();
        
        if rms < energy_threshold {
            silent_count += 1;
            if silent_count >= silence_window {
                // 找到静音窗口，返回非静音部分的结尾
                let actual_end = i + silent_count;
                return actual_end.min(samples.len());
            }
        } else {
            silent_count = 0;
        }
    }
    
    // 如果没有找到静音结尾，返回整个样本长度
    samples.len()
}



/// 对音频采样数据进行降采样，生成波形绘制数据
/// 每个像素点对应一组采样的最小值和最大值（类似 Audacity 的 min/max 波形）
fn downsample_waveform(samples: &[f64], sample_rate: u32, target_pixels: usize) -> report::WaveformData {
    if samples.is_empty() || target_pixels == 0 {
        return report::WaveformData {
            samples_per_pixel: 1,
            pixel_count: 0,
            duration_s: 0.0,
            min_values: vec![],
            max_values: vec![],
        };
    }
    
    let duration_s = samples.len() as f64 / sample_rate as f64;
    // 每像素对应的采样数，向上取整确保覆盖所有数据
    let samples_per_pixel = (samples.len() + target_pixels - 1) / target_pixels;
    let pixel_count = (samples.len() + samples_per_pixel - 1) / samples_per_pixel;
    
    let mut min_values = Vec::with_capacity(pixel_count);
    let mut max_values = Vec::with_capacity(pixel_count);
    
    for i in 0..pixel_count {
        let start = i * samples_per_pixel;
        let end = ((i + 1) * samples_per_pixel).min(samples.len());
        let chunk = &samples[start..end];
        let min_val = chunk.iter().cloned().fold(f64::INFINITY, f64::min) as f32;
        let max_val = chunk.iter().cloned().fold(f64::NEG_INFINITY, f64::max) as f32;
        min_values.push(min_val);
        max_values.push(max_val);
    }
    
    report::WaveformData {
        samples_per_pixel,
        pixel_count,
        duration_s,
        min_values,
        max_values,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // 参数校验
    if !args.reference.exists() {
        return Err(format!("参考音频文件不存在: {:?}", args.reference).into());
    }
    if !args.recorded.exists() {
        return Err(format!("录制音频文件不存在: {:?}", args.recorded).into());
    }

    // 加载 DNSMOS 模型（仅在需要时加载，可优化为延迟加载）
    println!("[*] 加载 DNSMOS 模型...");
    const DNSMOS_MODEL: &[u8] = include_bytes!("../bin/model/sig_bak_ovr.onnx");
    let mut dnsmos_evaluator = dnsmos::DnsMosEvaluator::new(DNSMOS_MODEL)
        .map_err(|e| format!("DNSMOS 模型加载失败: {}", e))?;
    println!("      DNSMOS 模型加载成功");

    println!("[*] 加载参考音频: {:?}", args.reference);
    let ref_audio = audio_io::AudioData::from_wav(&args.reference)?;
    println!("      原始采样率: {}, 时长: {:.2}s", 
             ref_audio.sample_rate, ref_audio.duration_secs());
    
    println!("[*] 加载录制音频: {:?}", args.recorded);
    let rec_audio = audio_io::AudioData::from_wav(&args.recorded)?;
    println!("      原始采样率: {}, 时长: {:.2}s", 
             rec_audio.sample_rate, rec_audio.duration_secs());

    // 自动选择 ViSQOL 模式并重采样
    let target_rate = if args.audio { 48000 } else { 16000 };
    let mode = if args.audio { visqol::VisqolMode::Audio } else { visqol::VisqolMode::Speech };
    
    let mode_name = if args.audio { "音频模式" } else { "语音模式" };
    println!("[*] ViSQOL 模式: {}Hz ({})", target_rate, mode_name);
    // 重采样到 ViSQOL 所需采样率
    let ref_audio = ref_audio.resample(target_rate)?;
    let rec_audio = rec_audio.resample(target_rate)?;

    let ref_duration = ref_audio.duration_secs();
    let rec_duration = rec_audio.duration_secs();
    
    // 多峰检测：使用频域特征匹配 + FFT 互相关精细化
    println!("[*] 执行多峰检测（频域特征匹配 + 精细对齐）...");
    let alignment_peaks = alignment_v2::find_all_alignments_hybrid(
        &ref_audio.samples,
        &rec_audio.samples,
        ref_audio.sample_rate,
        0.4,  // 置信度阈值：单次出现不该有第二个 >0.4 的峰；循环播放的相邻出现相关性也应 >0.4
    );
    
    let num_segments = alignment_peaks.len();
    println!("[*] 检测到 {} 个参考音频出现位置", num_segments);
    for (i, peak) in alignment_peaks.iter().enumerate() {
        println!("      第 {} 处: 偏移 {:.2}s, 置信度 {:.1}%", 
                 i + 1, peak.delay_ms / 1000.0, peak.confidence * 100.0);
    }
    if num_segments == 0 {
        return Err("未检测到参考音频在对齐结果中".into());
    }

    println!(
        "[*] 参考音频时长: {:.2}s, 录制音频时长: {:.2}s",
        ref_duration, rec_duration
    );
    println!("[*] 分段数量: {}", num_segments);

    // [DIAG] 全局长度诊断：参考/录制的全长 vs 去尾静音有效长度
    // 用于排查截断/漂移基准口径错位（根因 A）
    {
        let ref_full = ref_audio.samples.len();
        let ref_eff = find_actual_audio_end(&ref_audio.samples, ref_audio.sample_rate);
        let rec_full = rec_audio.samples.len();
        let rec_eff = find_actual_audio_end(&rec_audio.samples, ref_audio.sample_rate);
        let sr = ref_audio.sample_rate as f64;
        println!("[DIAG] 全局长度: 参考 全长={}/ {:.3}s, 去尾有效={}/ {:.3}s (尾部静音 {:.0}ms)",
                 ref_full, ref_full as f64 / sr, ref_eff, ref_eff as f64 / sr,
                 (ref_full - ref_eff) as f64 / sr * 1000.0);
        println!("[DIAG] 全局长度: 录制 全长={}/ {:.3}s, 去尾有效={}/ {:.3}s (尾部静音 {:.0}ms)",
                 rec_full, rec_full as f64 / sr, rec_eff, rec_eff as f64 / sr,
                 (rec_full - rec_eff) as f64 / sr * 1000.0);
        println!("[DIAG] 全局长度差: 录制-参考 = {:+.0}ms (全长口径), {:+.0}ms (去尾有效口径)",
                 (rec_full as f64 - ref_full as f64) / sr * 1000.0,
                 (rec_eff as f64 - ref_eff as f64) / sr * 1000.0);
    }

    let mut segment_results = Vec::with_capacity(num_segments);

    // 用于收集异常检测的数据
    let mut alignment_offsets: Vec<f64> = Vec::new();
    let mut all_patch_sims: Vec<Vec<visqol::PatchSimilarityResult>> = Vec::new();
    // 各段实际音频样本数（不含末尾补零），用于内容截断检测
    let mut seg_actual_samples: Vec<usize> = Vec::new();
    // 各段实际时长（秒），用于漂移检测
    let mut seg_durations_s: Vec<f64> = Vec::new();

    for (seg_idx, seg_align) in alignment_peaks.iter().enumerate() {
        let seg_start = seg_align.offset_samples.min(rec_audio.samples.len());
        let seg_end = (seg_start + ref_audio.samples.len()).min(rec_audio.samples.len());

       // 收集对齐偏移（秒）
       alignment_offsets.push(seg_start as f64 / ref_audio.sample_rate as f64);
       // 先提取段数据并补零到参考长度
       let mut seg_degraded = rec_audio.samples[seg_start..seg_end].to_vec();
       let _seg_raw_samples = seg_degraded.len(); // 补零前的实际样本数
       seg_degraded.resize(ref_audio.samples.len(), 0.0);

       // 检测补零后的实际音频末尾（用于截断和漂移检测）
       // 补零区在 seg_degraded 末尾，find_actual_audio_end 可以精确定位补零起点
       let seg_actual_end = find_actual_audio_end(&seg_degraded, ref_audio.sample_rate);
       seg_actual_samples.push(seg_actual_end);
       // 实际音频时长（秒），用于漂移检测
       let seg_actual_dur_s = seg_actual_end as f64 / ref_audio.sample_rate as f64;
       seg_durations_s.push(seg_actual_dur_s);

       // [DIAG] 每段提取诊断：边界 + 有效长度
       // - seg_raw_samples: resize 补零前原始段长（可能 < ref_len，说明录制在段尾被截断）
       // - seg_actual_end: resize 后再 find_actual_audio_end，定位「补零区起点」
       {
           let sr = ref_audio.sample_rate as f64;
           let ref_len = ref_audio.samples.len();
           let raw = _seg_raw_samples;
           println!("[DIAG] 第{}段 边界: seg_start={}, seg_end={}, 段原始长度={} (参考长度={}, 录制末尾剩余可取={})",
                    seg_idx + 1, seg_start, seg_end, raw, ref_len,
                    rec_audio.samples.len().saturating_sub(seg_start));
           println!("[DIAG] 第{}段 长度: 段原始={:.3}s, 补零后有效={:.3}s (尾部 {:.0}ms 被判为静音/补零), 参考={:.3}s",
                    seg_idx + 1, raw as f64 / sr, seg_actual_end as f64 / sr,
                    (ref_len - seg_actual_end) as f64 / sr * 1000.0, ref_len as f64 / sr);
           println!("[DIAG] 第{}段 口径差: 段原始-参考={:+.0}ms, 段有效-参考={:+.0}ms (截断/漂移实际将用后者)",
                    seg_idx + 1,
                    (raw as f64 - ref_len as f64) / sr * 1000.0,
                    (seg_actual_end as f64 - ref_len as f64) / sr * 1000.0);
       }

        // 调试：打印分段提取的详细信息
        let seg_samples = &rec_audio.samples[seg_start..seg_end];
        let seg_rms = (seg_samples.iter().map(|&x| x * x).sum::<f64>() / seg_samples.len() as f64).sqrt();
        let seg_peak = seg_samples.iter().map(|&x| x.abs()).fold(0.0f64, |a, b| a.max(b));
        let seg_max = seg_samples.iter().cloned().fold(f64::NEG_INFINITY, |a, b| a.max(b));
        let seg_min = seg_samples.iter().cloned().fold(f64::INFINITY, |a, b| a.min(b));
        
       println!("[DEBUG] 第{}段提取: 样本数={}, 偏移={}, RMS={:.6}, 峰值={:.6}, 最大={:.6}, 最小={:.6}", 
                seg_idx+1, seg_samples.len(), seg_start, seg_rms, seg_peak, seg_max, seg_min);

       let seg_start_time = seg_start as f64 / ref_audio.sample_rate as f64;
       let seg_end_time = seg_end as f64 / ref_audio.sample_rate as f64;

        println!(
            "[*] 评估第 {}/{} 段 ({:.2}s - {:.2}s, 置信度 {:.1}%)...",
            seg_idx + 1,
            num_segments,
            seg_start_time,
            seg_end_time,
            seg_align.confidence * 100.0
        );

        // 创建临时文件用于 visqol 对比
        let temp_dir = std::env::temp_dir().join("audiobench");
        fs::create_dir_all(&temp_dir)?;
        let ref_temp = temp_dir.join("ref.wav");
        let deg_temp = temp_dir.join("deg.wav");

        // 写入临时 WAV 文件
        audio_io::write_wav_mono(&ref_temp, &ref_audio.samples, ref_audio.sample_rate)?;
        audio_io::write_wav_mono(&deg_temp, &seg_degraded, ref_audio.sample_rate)?;

        // 调用 visqol 进行评估
        let visqol_result = visqol::evaluate_with_visqol(&ref_temp, &deg_temp, mode)?;

        // 收集该段的 patch 相似度数据
        all_patch_sims.push(visqol_result.patch_sims.clone());

        // ViSQOL 频段能量比（fvdegenergy）
        let band_energy_ratios = visqol_result.fvdegenergy.clone();

        println!("      MOS-LQO: {:.2}, VNSIM: {:.4}", 
                 visqol_result.moslqo, visqol_result.vnsim);

        // 时域中断检测（维度一），使用 seg_degraded 的实际长度排除补零尾段
        let seg_actual_len = seg_end - seg_start; // 实际音频长度（不含补零）
        let dropout_events = metrics::detect_dropouts(
            &ref_audio.samples,
            &seg_degraded,
            ref_audio.sample_rate,
            &metrics::DropoutDetectorConfig::for_sample_rate(ref_audio.sample_rate),
            seg_actual_len, // 有效长度，排除补零尾段
        );

        // [DIAG] 中断检测诊断：检测器输入范围 + 每个事件详情
        {
            let sr = ref_audio.sample_rate as f64;
            let cfg = metrics::DropoutDetectorConfig::for_sample_rate(ref_audio.sample_rate);
            println!("[DIAG] 第{}段 中断检测: 参考长={}, seg_degraded长={}, valid_len={} ({:.3}s), 最短中断阈值={:.0}ms, 静音阈值={}, 衰减阈值={}",
                     seg_idx + 1, ref_audio.samples.len(), seg_degraded.len(),
                     seg_actual_len, seg_actual_len as f64 / sr,
                     cfg.min_duration_ms, cfg.silence_threshold, cfg.attenuation_threshold);
            println!("[DIAG] 第{}段 中断事件数={}", seg_idx + 1, dropout_events.len());
            for (i, ev) in dropout_events.iter().enumerate() {
                println!("[DIAG]    中断#{}: {:.3}-{:.3}s 持续{:.0}ms, ref_rms={:.5}, deg_rms={:.5}, 衰减比={:.4}",
                         i + 1, ev.start_time_s, ev.end_time_s, ev.duration_ms,
                         ev.ref_rms, ev.deg_rms, ev.attenuation_ratio);
            }
        }

        let dropout_duration_ms: f64 = dropout_events.iter().map(|e| e.duration_ms).sum();
        let anomaly = metrics::AudioAnomalyReport {
            has_anomaly: !dropout_events.is_empty(),
            dropouts: dropout_events,
            dropout_duration_ms,
            warpings: vec![],
            warping_duration_ms: 0.0,
            spectral_artifacts_score: 0.0,
            spectral_artifacts: vec![],
            // 内容截断已移除
        };

        // 幅值统计
        let level_ref = metrics::compute_level_stats(&ref_audio.samples);
        let level_deg = metrics::compute_level_stats(&seg_degraded);

        // 清理临时文件
        let _ = fs::remove_file(&ref_temp);
        let _ = fs::remove_file(&deg_temp);

        // DNSMOS 评估（仅对录制音频，无参考）
        let (sig, bak, ovrl) = match dnsmos_evaluator.evaluate(&seg_degraded, ref_audio.sample_rate) {
            Ok(r) => (Some(r.sig), Some(r.bak), Some(r.ovrl)),
            Err(e) => {
                println!("[!] 第{}段 DNSMOS 评估失败: {}", seg_idx + 1, e);
                (None, None, None)
            }
        };

        segment_results.push(report::SegmentResult {
            segment_index: seg_idx,
            start_time_s: seg_start_time,
            end_time_s: seg_end_time,
            quality: visqol_result.into(),
            anomaly,
            level_ref,
            level_deg,
            band_energy_ratios,
            sig,
            bak,
            ovrl,
        });
    }

    // 维度二：时轴漂移检测（在所有分段完成后）
    // 使用新的滑窗互相关 + 形态分析检测
    let warping_config = metrics::WarpingConfig::for_sample_rate(ref_audio.sample_rate);

    // [DIAG] 漂移检测配置
    eprintln!("[DIAG] === 漂移检测配置 === window={}ms, hop={}ms, search_radius={}ms, silence={}, jump={}ms, slope={}, min_drift={}ms, min_r2={}",
              warping_config.window_ms, warping_config.hop_ms, warping_config.search_radius_ms,
              warping_config.silence_threshold, warping_config.jump_threshold_ms,
              warping_config.slope_threshold, warping_config.min_drift_ms, warping_config.min_r_squared);

    let mut all_warping_events: Vec<metrics::WarpingEvent> = Vec::new();

    // 对每段进行漂移检测
    for (seg_idx, seg_align) in alignment_peaks.iter().enumerate() {
        let seg_start = seg_align.offset_samples.min(rec_audio.samples.len());
        let seg_end = (seg_start + ref_audio.samples.len()).min(rec_audio.samples.len());

        // 提取段数据并补零到参考长度
        let mut seg_degraded = rec_audio.samples[seg_start..seg_end].to_vec();
        seg_degraded.resize(ref_audio.samples.len(), 0.0);

        // 阶段一：计算 offset 序列
        let offsets = time_warping::compute_offset_series(
            &ref_audio.samples,
            &seg_degraded,
            ref_audio.sample_rate,
            &warping_config,
        );

        // 阶段二：从 offset 序列检测漂移事件
        let seg_events = time_warping::detect_warpings_from_offsets(
            &offsets,
            ref_audio.sample_rate,
            warping_config.hop_ms,
            seg_idx,
            &warping_config,
        );

        all_warping_events.extend(seg_events);
    }

    // 维度三：频谱损伤检测
    let artifact_threshold = 0.4; // 相似度低于 0.4 判定为损伤（与 AnomalyDetectConfig 一致）
    let (_spectral_score, spectral_events) = metrics::detect_spectral_artifacts(
        &all_patch_sims,
        artifact_threshold,
        true, // 排除每段首尾 patch（边界效应：补零/能量过渡天然偏低）
    );

    // [DIAG] 全局频谱检测诊断：每段 patch 数量 + 低相似度分布
    {
        println!("[DIAG] === 频谱检测汇总 === 阈值 patch_sim<{}, 段内低相似度比例>{}% 才报异常",
                 artifact_threshold, 0.25 * 100.0);
        for (i, patches) in all_patch_sims.iter().enumerate() {
            if patches.is_empty() {
                println!("[DIAG]    第{}段: 无 patch", i + 1);
                continue;
            }
            let (start, end) = if patches.len() >= 4 { (1, patches.len() - 1) } else { (0, patches.len()) };
            let valid_count = end - start;
            let valid_sims: Vec<f64> = patches[start..end].iter().map(|p| p.similarity).collect();
            let low_count = valid_sims.iter().filter(|&&s| s < artifact_threshold).count();
            let min_sim = valid_sims.iter().cloned().fold(f64::INFINITY, f64::min);
            let score = if valid_count > 0 { low_count as f64 / valid_count as f64 } else { 0.0 };
            println!("[DIAG]    第{}段: patch总数={}, 有效patch(排除首尾)={}, 低相似度(<{})={}/{}) 比例={:.1}% 最低={:.3}",
                     i + 1, patches.len(), valid_count, artifact_threshold, low_count, valid_count,
                     score * 100.0, min_sim);
            if low_count > 0 {
                let detail: Vec<String> = patches[start..end].iter().enumerate()
                    .filter(|(_, p)| p.similarity < artifact_threshold)
                    .map(|(k, p)| format!("patch{}={:.3}", start + k, p.similarity))
                    .collect();
                println!("[DIAG]       低相似度 patch: {}", detail.join(", "));
            }
        }
    }

    // 将时轴漂移和频谱损伤结果更新到每段报告中
    for (seg_idx, seg_result) in segment_results.iter_mut().enumerate() {
        // 合并时轴漂移事件到对应段
        let seg_warpings: Vec<metrics::WarpingEvent> = all_warping_events.iter()
            .filter(|w| w.segment_index == seg_idx)
            .cloned()
            .collect();
        let seg_warping_ms: f64 = seg_warpings.iter().map(|w| w.drift_ms.abs()).sum();

        // 合并频谱损伤事件到对应段
        let seg_artifacts: Vec<metrics::SpectralArtifactEvent> = spectral_events.iter()
            .filter(|a| a.segment_index == seg_idx)
            .cloned()
            .collect();

        // 计算该段的频谱损伤比例（分母排除首尾 patch，与 detect_spectral_artifacts 一致）
        let seg_total_patch = all_patch_sims.get(seg_idx).map(|p| p.len()).unwrap_or(0);
        let seg_valid_patch = if seg_total_patch >= 4 { seg_total_patch - 2 } else { seg_total_patch };
        let seg_low_count = seg_artifacts.len();
        let seg_spectral_score = if seg_valid_patch > 0 {
            seg_low_count as f64 / seg_valid_patch as f64
        } else {
            0.0
        };

        // 更新异常检测报告
        // has_anomaly 的频谱门槛从 0.1 提到 0.25，避免少量低相似度 patch 就误标异常
        let has_anomaly = seg_result.anomaly.has_anomaly
            || !seg_warpings.is_empty()
            || seg_spectral_score > 0.25;

        seg_result.anomaly.warpings = seg_warpings;
        seg_result.anomaly.warping_duration_ms = seg_warping_ms;
        seg_result.anomaly.spectral_artifacts_score = seg_spectral_score;
        seg_result.anomaly.spectral_artifacts = seg_artifacts;
        // 内容截断已移除
        seg_result.anomaly.has_anomaly = has_anomaly;
    }
    
    // 全局对齐信息（使用第一段的对齐信息）
    let first_peak = alignment_peaks.first().cloned().unwrap_or(alignment_v2::AlignmentResult {
        offset_samples: 0,
        delay_ms: 0.0,
        confidence: 0.0,
    });
    

    // 生成波形数据（用于 HTML 报告中的波形图）
    // 目标：每秒约 200 个像素点，足够展示细节
    let waveform_ref = downsample_waveform(&ref_audio.samples, ref_audio.sample_rate, (ref_duration * 200.0) as usize);
    let waveform_deg = downsample_waveform(&rec_audio.samples, ref_audio.sample_rate, (rec_duration * 200.0) as usize);
    
    // 生成报告
    let report = report::generate_report(
        report::ReportConfig {
            reference_path: args.reference.to_string_lossy().to_string(),
            recorded_path: args.recorded.to_string_lossy().to_string(),
            target_sample_rate: ref_audio.sample_rate,
        },
        report::AlignmentInfo {
            offset_samples: first_peak.offset_samples,
            delay_ms: first_peak.delay_ms,
            confidence: first_peak.confidence,
        },
        ref_duration,
        rec_duration,
        segment_results,
        waveform_ref,
        waveform_deg,
    );
    
    // 输出控制台报告
    report::print_console_report(&report);
    
    // 输出 JSON 报告
    if let Some(output_path) = args.output {
        let json = serde_json::to_string_pretty(&report)
            .map_err(|e| format!("JSON 序列化失败: {}", e))?;
        fs::write(&output_path, json)?;
        println!("\n[+] JSON 报告已保存: {:?}", output_path);
    }

    // 输出 HTML 报告
    if let Some(html_path) = args.html {
        let html = html_report::generate_html_report(&report);
        fs::write(&html_path, html)?;
        println!("\n[+] HTML 报告已保存: {:?}", html_path);
    }
    
    Ok(())
}
