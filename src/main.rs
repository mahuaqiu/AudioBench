//! AudioBench - 音频质量评估工具
//! 
//! 纯 Rust 实现，无需外部 ViSQOL 依赖，单 EXE 运行。
//! 使用 Gammatone 滤波器组 + NSIM 类 SSIM 算法，与 ViSQOL 指标准确对齐。
//! 
//! 使用方法:
//!   audio_bench --reference ref.wav --recorded rec.wav
//!
//! 分段评估:
//!   当录制音频比参考音频长时，自动按参考音频长度分段评估，
//!   每段输出独立评分，并汇总整体统计。

mod alignment;
mod audio_io;
mod metrics;
mod gammatone;
mod quality;
mod spectrogram;
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

    /// 录制���频文件路径（WAV 格式）
    #[clap(long = "recorded", short = 'c', required = true)]
    recorded: PathBuf,

    /// 目标采样率（语音模式用 16000，音频模式用 48000）
    #[clap(long = "sample-rate", short = 's', default_value = "48000")]
    sample_rate: u32,

    /// 使用语音模式（推荐语音音频用这个，16kHz）
    #[clap(long = "speech", conflicts_with = "sample_rate")]
    speech_mode: bool,

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
    
    // 确定采样率和工作模式
    let (target_sample_rate, use_speech_mode) = if args.speech_mode {
        (16000, true)
    } else {
        (args.sample_rate, false)
    };
    
    // 加载并预处理音频
    println!("[*] 加载参考音频: {:?}", args.reference);
    let mut ref_audio = audio_io::AudioData::from_wav(&args.reference)?;
    println!("      原始采样率: {}, 时长: {:.2}s", 
             ref_audio.sample_rate, ref_audio.duration_secs());
    
    println!("[*] 加载录制音频: {:?}", args.recorded);
    let mut rec_audio = audio_io::AudioData::from_wav(&args.recorded)?;
    println!("      原始采样率: {}, 时长: {:.2}s", 
             rec_audio.sample_rate, rec_audio.duration_secs());
    
    // 重采样
    ref_audio = ref_audio.resample(target_sample_rate)?;
    rec_audio = rec_audio.resample(target_sample_rate)?;
    println!("[*] 重采样到 {} Hz", target_sample_rate);
    
    // FFT 互相关对齐：找到参考音频在录制音频中的起始位置
    println!("[*] 执行信号对齐...");
    let align_result = alignment::find_alignment(
        &ref_audio.samples,
        &rec_audio.samples,
        target_sample_rate,
    );
    println!("      延迟: {:.1} ms, 置信度: {:.1}%", 
             align_result.delay_ms, align_result.confidence * 100.0);
    
    let ref_len = ref_audio.samples.len();
    let rec_len = rec_audio.samples.len();
    let ref_duration = ref_audio.duration_secs();
    let rec_duration = rec_audio.duration_secs();
    
    // 从对齐偏移开始，按参考音频长度分段评估录制音频
    let aligned_start = align_result.offset_samples.min(rec_len);
    let available_len = rec_len.saturating_sub(aligned_start);
    
    // 计算分段数量：以「完整覆盖一个参考长度」的整段为主，
    // 仅当末尾残段超过参考长度一半时才额外计入，避免出现几乎全是补零的尾段。
    let num_segments = if ref_len == 0 {
        1
    } else if available_len >= ref_len {
        let full = available_len / ref_len;
        let remainder = available_len % ref_len;
        if remainder * 2 >= ref_len { full + 1 } else { full }
    } else {
        1
    }
    .max(1);
    
    println!("[*] 参考音频时长: {:.2}s, 录制音频对齐后可用: {:.2}s", 
             ref_duration, available_len as f64 / target_sample_rate as f64);
    println!("[*] 分段数量: {}", num_segments);
    
    let mut segment_results = Vec::with_capacity(num_segments);
    
    for seg_idx in 0..num_segments {
        let seg_start = aligned_start + seg_idx * ref_len;
        let seg_end = (seg_start + ref_len).min(rec_len);
        
        // 录制音频分段
        let mut seg_degraded = rec_audio.samples[seg_start..seg_end].to_vec();
        // 不足参考长度的末尾补零
        seg_degraded.resize(ref_len, 0.0);
        
        let seg_ref_samples = ref_audio.samples.clone();
        
        let seg_start_time = seg_start as f64 / target_sample_rate as f64;
        let seg_end_time = seg_end as f64 / target_sample_rate as f64;
        
        println!("[*] 评估第 {}/{} 段 ({:.2}s - {:.2}s)...", 
                 seg_idx + 1, num_segments, seg_start_time, seg_end_time);
        
        // 质量评估（ViSQOL 兼容指标）
        let quality_result = quality::evaluate_quality(
            &seg_ref_samples,
            &seg_degraded,
            target_sample_rate,
            use_speech_mode,
        );
        
        // SNR
        let snr = metrics::compute_snr(&seg_ref_samples, &seg_degraded);
        
        // 卡顿检测
        let dropouts = metrics::detect_dropouts(
            &seg_ref_samples,
            &seg_degraded,
            target_sample_rate,
            0.005,
            20.0,
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
    
    // 生成报告
    let report = report::generate_report(
        report::ReportConfig {
            reference_path: args.reference.to_string_lossy().to_string(),
            recorded_path: args.recorded.to_string_lossy().to_string(),
            target_sample_rate,
            speech_mode: use_speech_mode,
        },
        report::AlignmentInfo {
            offset_samples: align_result.offset_samples,
            delay_ms: align_result.delay_ms,
            confidence: align_result.confidence,
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
