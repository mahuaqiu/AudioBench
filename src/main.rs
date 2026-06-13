//! AudioBench - 音频质量评估工具
//! 
//! 纯 Rust 实现，无需外部 ViSQOL 依赖，单 EXE 运行。
//! 使用 Gammatone 滤波器组 + NSIM 类 SSIM 算法，与 ViSQOL 指标准确对齐。
//! 
//! 使用方法:
//!   audio_bench --reference ref.wav --recorded rec.wav
//!
//! 分段评估:
//!   当录制音频中参考音频不规则出现多次时，使用多峰检测自动发现所有出现位置，
//!   每段独立评分，并汇总整体统计。

mod alignment;
mod audio_io;
mod metrics;
mod gammatone;
mod quality;
mod spectrogram;
mod dtw;
mod report;

use clap::Parser;
use std::fs;
use std::path::PathBuf;

/// 命令行参数
#[derive(Parser, Debug)]
#[clap(name = "audio_bench", version = "0.1.0", about = "音频质量评估工具")]
struct Args {
    /// 参考音频文件路径（WAV 格式）
    #[clap(long = "reference", short = 'r', required = true)]
    reference: PathBuf,

    /// 录制音频文件路径（WAV 格式）
    #[clap(long = "recorded", short = 'c', required = true)]
    recorded: PathBuf,

    /// 目标采样率（默认使用输入文件采样率）
    #[clap(long = "sample-rate", short = 's', default_value = "0")]
    sample_rate: u32,

    /// 输出 JSON 报告文件路径（可选）
    #[clap(long = "output", short = 'o')]
    output: Option<PathBuf>,
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
    
    // 加载音频（保留原始采样率）
    println!("[*] 加载参考音频: {:?}", args.reference);
    let mut ref_audio = audio_io::AudioData::from_wav(&args.reference)?;
    println!("      原始采样率: {}, 时长: {:.2}s", 
             ref_audio.sample_rate, ref_audio.duration_secs());
    
    println!("[*] 加载录制音频: {:?}", args.recorded);
    let mut rec_audio = audio_io::AudioData::from_wav(&args.recorded)?;
    println!("      原始采样率: {}, 时长: {:.2}s", 
             rec_audio.sample_rate, rec_audio.duration_secs());
    
    // 确定目标采样率
    let target_sample_rate = if args.sample_rate > 0 {
        args.sample_rate
    } else {
        ref_audio.sample_rate
    };

    // 采样率不同时发出警告
    if ref_audio.sample_rate != rec_audio.sample_rate {
        println!("[!] 警告: 参考采样率 {} Hz，录制采样率 {} Hz，将统一到 {} Hz",
                 ref_audio.sample_rate, rec_audio.sample_rate, target_sample_rate);
    }
    
    // 仅当目标采样率与输入不同时才重采样
    let needs_resample = ref_audio.sample_rate != target_sample_rate 
        || rec_audio.sample_rate != target_sample_rate;
    if needs_resample {
        ref_audio = ref_audio.resample(target_sample_rate)?;
        rec_audio = rec_audio.resample(target_sample_rate)?;
        println!("[*] 重采样到 {} Hz", target_sample_rate);
    } else {
        println!("[*] 使用原始采样率 {} Hz", target_sample_rate);
    }
    
    let ref_len = ref_audio.samples.len();
    let rec_len = rec_audio.samples.len();
    let ref_duration = ref_audio.duration_secs();
    let rec_duration = rec_audio.duration_secs();
    
    // 多峰检测：自动发现录制音频中参考音频的所有出现位置
    println!("[*] 执行多峰检测，定位参考音频的所有出现位置...");
    let alignment_peaks = alignment::find_all_alignments(
        &ref_audio.samples,
        &rec_audio.samples,
        target_sample_rate,
        0.3,  // 置信度阈值：低于 0.3 的峰被过滤
    );
    
    let num_segments = alignment_peaks.len();
    println!("[*] 检测到 {} 个参考音频出现位置", num_segments);
    for (i, peak) in alignment_peaks.iter().enumerate() {
        println!("      第 {} 处: 偏移 {:.2}s, 置信度 {:.1}%", 
                 i + 1, peak.delay_ms / 1000.0, peak.confidence * 100.0);
    }

    println!(
        "[*] 参考音频时长: {:.2}s, 录制音频时长: {:.2}s",
        ref_duration, rec_duration
    );
    println!("[*] 分段数量: {}", num_segments);

    let mut segment_results = Vec::with_capacity(num_segments);

    for (seg_idx, seg_align) in alignment_peaks.iter().enumerate() {
        let seg_start = seg_align.offset_samples.min(rec_len);
        let seg_end = (seg_start + ref_len).min(rec_len);

        // 记录该段实际补零长度，供卡顿检测排除补零尾段使用
        let real_len = seg_end.saturating_sub(seg_start);
        let padded_len = ref_len.saturating_sub(real_len);

        let mut seg_degraded = rec_audio.samples[seg_start..seg_end].to_vec();
        // 不足参考长度的末尾补零
        seg_degraded.resize(ref_len, 0.0);

        let seg_ref_samples = ref_audio.samples.clone();

        let seg_start_time = seg_start as f64 / target_sample_rate as f64;
        let seg_end_time = seg_end as f64 / target_sample_rate as f64;

        println!(
            "[*] 评估第 {}/{} 段 ({:.2}s - {:.2}s, 置信度 {:.1}%)...",
            seg_idx + 1,
            num_segments,
            seg_start_time,
            seg_end_time,
            seg_align.confidence * 100.0
        );

        // 质量评估（ViSQOL 兼容指标）
        let quality_result = quality::evaluate_quality(
            &seg_ref_samples,
            &seg_degraded,
            target_sample_rate,
        );

        // SNR
        let snr = metrics::compute_snr(&seg_ref_samples, &seg_degraded);

        // 卡顿检测（排除补零尾段）
        let dropouts = metrics::detect_dropouts(
            &seg_ref_samples,
            &seg_degraded,
            target_sample_rate,
            0.005,
            20.0,
            padded_len,
        );

        // 幅值统计
        let level_ref = metrics::compute_level_stats(&seg_ref_samples);
        let level_deg = metrics::compute_level_stats(&seg_degraded);

        // 诊断
        let diagnosis = quality::diagnose(&quality_result);

        segment_results.push(report::SegmentResult {
            segment_index: seg_idx,
            start_time_s: seg_start_time,
            end_time_s: seg_end_time,
            quality: quality_result,
            snr,
            dropouts,
            level_ref,
            level_deg,
            diagnosis,
        });
    }
    
    // 全局对齐信息（首次出现的峰值）
    let first_peak = alignment_peaks.first().cloned().unwrap_or(alignment::AlignmentResult {
        offset_samples: 0,
        delay_ms: 0.0,
        confidence: 0.0,
    });
    
    // 生成报告
    let report = report::generate_report(
        report::ReportConfig {
            reference_path: args.reference.to_string_lossy().to_string(),
            recorded_path: args.recorded.to_string_lossy().to_string(),
            target_sample_rate,
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
    
    Ok(())
}
