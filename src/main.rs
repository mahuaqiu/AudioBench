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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // 参数校验
    if !args.reference.exists() {
        return Err(format!("参考音频文件不存在: {:?}", args.reference).into());
    }
    if !args.recorded.exists() {
        return Err(format!("录制音频文件不存在: {:?}", args.recorded).into());
    }

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

    let mut segment_results = Vec::with_capacity(num_segments);

    // 用于收集三维异常检测的数据
    let mut alignment_offsets: Vec<f64> = Vec::new();
    let mut all_patch_sims: Vec<Vec<visqol::PatchSimilarityResult>> = Vec::new();
    // 各段实际音频样本数（不含末尾补零），用于内容截断检测
    let mut seg_actual_samples: Vec<usize> = Vec::new();

    for (seg_idx, seg_align) in alignment_peaks.iter().enumerate() {
        let seg_start = seg_align.offset_samples.min(rec_audio.samples.len());
        let seg_end = (seg_start + ref_audio.samples.len()).min(rec_audio.samples.len());

        // 收集对齐偏移（秒）
        alignment_offsets.push(seg_start as f64 / ref_audio.sample_rate as f64);
        // 收集该段实际音频样本数（不含末尾补零），用于内容截断检测
        seg_actual_samples.push(seg_end - seg_start);

        // 调试：打印分段提取的详细信息
        let seg_samples = &rec_audio.samples[seg_start..seg_end];
        let seg_rms = (seg_samples.iter().map(|&x| x * x).sum::<f64>() / seg_samples.len() as f64).sqrt();
        let seg_peak = seg_samples.iter().map(|&x| x.abs()).fold(0.0f64, |a, b| a.max(b));
        let seg_max = seg_samples.iter().cloned().fold(f64::NEG_INFINITY, |a, b| a.max(b));
        let seg_min = seg_samples.iter().cloned().fold(f64::INFINITY, |a, b| a.min(b));
        
        println!("[DEBUG] 第{}段提取: 样本数={}, 偏移={}, RMS={:.6}, 峰值={:.6}, 最大={:.6}, 最小={:.6}", 
                 seg_idx+1, seg_samples.len(), seg_start, seg_rms, seg_peak, seg_max, seg_min);

        let mut seg_degraded = rec_audio.samples[seg_start..seg_end].to_vec();
        // 不足参考长度的末尾补零
        seg_degraded.resize(ref_audio.samples.len(), 0.0);

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
        let dropout_duration_ms: f64 = dropout_events.iter().map(|e| e.duration_ms).sum();
        let anomaly = metrics::AudioAnomalyReport {
            has_anomaly: !dropout_events.is_empty(),
            dropouts: dropout_events,
            dropout_duration_ms,
            warpings: vec![],
            warping_duration_ms: 0.0,
            spectral_artifacts_score: 0.0,
            spectral_artifacts: vec![],
            truncations: vec![],
            truncation_duration_ms: 0.0,
        };

        // 幅值统计
        let level_ref = metrics::compute_level_stats(&ref_audio.samples);
        let level_deg = metrics::compute_level_stats(&seg_degraded);

        // 清理临时文件
        let _ = fs::remove_file(&ref_temp);
        let _ = fs::remove_file(&deg_temp);

        segment_results.push(report::SegmentResult {
            segment_index: seg_idx,
            start_time_s: seg_start_time,
            end_time_s: seg_end_time,
            quality: visqol_result.into(),
            anomaly,
            level_ref,
            level_deg,
            band_energy_ratios,
        });
    }

    // 维度二：时轴漂移检测（在所有分段完成后）
    // 采用段间间距的中位数做基准（对循环停顿不固定鲁棒），段数 < 4 时自动跳过。
    let warping_events = metrics::detect_warpings(
        &alignment_offsets,
        metrics::WarpingThreshold::default(),
    );

    // 维度二补充：内容截断/裁剪检测
    // 直接比对各段实际长度 vs 参考长度，绕过中断/频谱检测对裁剪的盲区。
    let truncation_events = metrics::detect_truncation(
        &seg_actual_samples,
        ref_audio.samples.len(),
        ref_audio.sample_rate,
        metrics::TruncationThreshold::default(),
    );

    // 维度三：频谱损伤检测
    let artifact_threshold = 0.4; // 相似度低于 0.4 判定为损伤（与 AnomalyDetectConfig 一致）
    let (_spectral_score, spectral_events) = metrics::detect_spectral_artifacts(
        &all_patch_sims,
        artifact_threshold,
        true, // 排除每段首尾 patch（边界效应：补零/能量过渡天然偏低）
    );

    // 将时轴漂移和频谱损伤结果更新到每段报告中
    for (seg_idx, seg_result) in segment_results.iter_mut().enumerate() {
        // 合并时轴漂移事件到对应段。
        // 注意：每个 warping 事件只归 segment_before（前一段），避免与 segment_after 重复，
        // 从而避免 report.rs/html_report.rs 的跨段累加把 drift_ms 翻倍。
        let seg_warpings: Vec<metrics::WarpingEvent> = warping_events.iter()
            .filter(|w| w.segment_before == seg_idx)
            .cloned()
            .collect();
        let seg_warping_ms: f64 = seg_warpings.iter().map(|w| w.drift_ms.abs()).sum();

        // 合并频谱损伤事件到对应段
        let seg_artifacts: Vec<metrics::SpectralArtifactEvent> = spectral_events.iter()
            .filter(|a| a.segment_index == seg_idx)
            .cloned()
            .collect();

        // 合并内容截断事件到对应段
        let seg_truncations: Vec<metrics::TruncationEvent> = truncation_events.iter()
            .filter(|t| t.segment_index == seg_idx)
            .cloned()
            .collect();
        let seg_truncation_ms: f64 = seg_truncations.iter().map(|t| t.truncation_ms).sum();

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
            || !seg_truncations.is_empty()
            || seg_spectral_score > 0.25;

        seg_result.anomaly.warpings = seg_warpings;
        seg_result.anomaly.warping_duration_ms = seg_warping_ms;
        seg_result.anomaly.spectral_artifacts_score = seg_spectral_score;
        seg_result.anomaly.spectral_artifacts = seg_artifacts;
        seg_result.anomaly.truncations = seg_truncations;
        seg_result.anomaly.truncation_duration_ms = seg_truncation_ms;
        seg_result.anomaly.has_anomaly = has_anomaly;
    }
    
    // 全局对齐信息（使用第一段的对齐信息）
    let first_peak = alignment_peaks.first().cloned().unwrap_or(alignment_v2::AlignmentResult {
        offset_samples: 0,
        delay_ms: 0.0,
        confidence: 0.0,
    });
    
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
