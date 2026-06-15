//! AudioBench - 音频质量评估工具
//!
//! 集成官方 ViSQOL 进行音频质量评估，单 EXE 运行。
//! 编译时嵌入 visqol 二进制，运行时自动释放到临时目录。
//! 使用方法:
//!   audio_bench --reference ref.wav --recorded rec.wav

mod alignment;
mod audio_io;
mod metrics;
mod visqol;
mod report;

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

    /// 强制使用语音模式 (16kHz)，默认使用音频模式 (48kHz)
    #[clap(long = "speech")]
    speech: bool,

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

    println!("[*] 加载参考音频: {:?}", args.reference);
    let ref_audio = audio_io::AudioData::from_wav(&args.reference)?;
    println!("      原始采样率: {}, 时长: {:.2}s", 
             ref_audio.sample_rate, ref_audio.duration_secs());
    
    println!("[*] 加载录制音频: {:?}", args.recorded);
    let rec_audio = audio_io::AudioData::from_wav(&args.recorded)?;
    println!("      原始采样率: {}, 时长: {:.2}s", 
             rec_audio.sample_rate, rec_audio.duration_secs());

    // 自动选择 ViSQOL 模式并重采样
    let target_rate = if args.speech { 16000 } else { 48000 };
    let mode = if args.speech { visqol::VisqolMode::Speech } else { visqol::VisqolMode::Audio };
    
    let mode_name = if args.speech { "语音模式" } else { "音频模式" };
    println!("[*] ViSQOL 模式: {}Hz ({})", target_rate, mode_name);
    // 重采样到 ViSQOL 所需采样率
    let ref_audio = ref_audio.resample(target_rate)?;
    let rec_audio = rec_audio.resample(target_rate)?;

    let ref_duration = ref_audio.duration_secs();
    let rec_duration = rec_audio.duration_secs();
    
    // 多峰检测：自动发现录制音频中参考音频的所有出现位置
    println!("[*] 执行多峰检测，定位参考音频的所有出现位置...");
    let alignment_peaks = alignment::find_all_alignments(
        &ref_audio.samples,
        &rec_audio.samples,
        ref_audio.sample_rate,
        0.3,  // 置信度阈值
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

    for (seg_idx, seg_align) in alignment_peaks.iter().enumerate() {
        let seg_start = seg_align.offset_samples.min(rec_audio.samples.len());
        let seg_end = (seg_start + ref_audio.samples.len()).min(rec_audio.samples.len());

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

        // ViSQOL 频段能量比（fvdegenergy）
        let band_energy_ratios = visqol_result.fvdegenergy.clone();


        println!("      MOS-LQO: {:.2}, VNSIM: {:.4}", 
                 visqol_result.moslqo, visqol_result.vnsim);

        // SNR
        let snr = metrics::compute_snr(&ref_audio.samples, &seg_degraded);

        // 卡顿检测
        let dropouts = metrics::detect_dropouts(
            &ref_audio.samples,
            &seg_degraded,
            ref_audio.sample_rate,
            0.005,
            20.0,
            0,
        );

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
            snr,
            dropouts,
            level_ref,
            level_deg,
            band_energy_ratios,

        });
    }
    
    // 全局对齐信息（使用第一段的对齐信息）
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
    
    Ok(())
}
