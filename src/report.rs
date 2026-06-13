//! 报告生成模块
//! 汇总所有指标，生成 JSON 报告和控制台输出
//! 支持分段评估：每段独立评分 + 整体统计汇总

use serde::Serialize;
use crate::metrics::{DropoutResult, LevelResult, SnrResult};
use crate::quality::{DiagnosisResult, QualityResult};

/// 评估配置信息
#[derive(Debug, Serialize)]
pub struct ReportConfig {
    /// 参考音频路径
    pub reference_path: String,
    /// 录制音频路径
    pub recorded_path: String,
    /// 目标采样率
    pub target_sample_rate: u32,
}

/// 对齐信息
#[derive(Debug, Serialize)]
pub struct AlignmentInfo {
    /// 偏移采样点数
    pub offset_samples: usize,
    /// 延迟时间（毫秒）
    pub delay_ms: f64,
    /// 对齐置信度
    pub confidence: f64,
}

/// 单段评估结果
#[derive(Debug, Serialize)]
pub struct SegmentResult {
    /// 分段序号（从 0 开始）
    pub segment_index: usize,
    /// 分段在录制音频中的起始时间（秒）
    pub start_time_s: f64,
    /// 分段在录制音频中的结束时间（秒）
    pub end_time_s: f64,
    /// 质量评估结果（ViSQOL 兼容指标）
    pub quality: QualityResult,
    /// SNR 结果
    pub snr: SnrResult,
    /// 卡顿检测结果
    pub dropouts: DropoutResult,
    /// 参考音频幅值统计
    pub level_ref: LevelResult,
    /// 录制音频幅值统计
    pub level_deg: LevelResult,
    /// 诊断结果
    pub diagnosis: DiagnosisResult,
}

/// 整体统计汇总
#[derive(Debug, Serialize)]
pub struct OverallStats {
    /// 分段总数
    pub segment_count: usize,
    /// MOS-LQO 均值
    pub moslqo_mean: f64,
    /// MOS-LQO 最小值
    pub moslqo_min: f64,
    /// MOS-LQO 最大值
    pub moslqo_max: f64,
    /// MOS-LQO 标准差
    pub moslqo_stddev: f64,
    /// VNSIM 均值
    pub vnsim_mean: f64,
    /// SNR 均值 (dB)
    pub snr_mean_db: f64,
    /// 总卡顿次数
    pub total_dropout_count: usize,
    /// 总卡顿时长 (ms)
    pub total_dropout_duration_ms: f64,
    /// 背景噪声检出段数
    pub segments_with_noise: usize,
    /// 高频损失检出段数
    pub segments_with_hf_loss: usize,
    /// 间歇性杂音检出段数
    pub segments_with_artifacts: usize,
    /// 整体质量评级
    pub overall_rating: String,
}

/// 完整评估报告
#[derive(Debug, Serialize)]
pub struct EvaluationReport {
    /// 评估配置
    pub config: ReportConfig,
    /// 对齐信息
    pub alignment: AlignmentInfo,
    /// 参考音频时长（秒）
    pub reference_duration_s: f64,
    /// 录制音频时长（秒）
    pub recorded_duration_s: f64,
    /// 整体统计汇总
    pub overall: OverallStats,
    /// 各段详细结果
    pub segments: Vec<SegmentResult>,
}

/// 生成完整评估报告
pub fn generate_report(
    config: ReportConfig,
    alignment: AlignmentInfo,
    ref_duration: f64,
    rec_duration: f64,
    segments: Vec<SegmentResult>,
) -> EvaluationReport {
    let overall = compute_overall_stats(&segments);
    EvaluationReport {
        config,
        alignment,
        reference_duration_s: ref_duration,
        recorded_duration_s: rec_duration,
        overall,
        segments,
    }
}

/// 计算整体统计
fn compute_overall_stats(segments: &[SegmentResult]) -> OverallStats {
    let n = segments.len();
    if n == 0 {
        return OverallStats {
            segment_count: 0,
            moslqo_mean: 0.0, moslqo_min: 0.0, moslqo_max: 0.0, moslqo_stddev: 0.0,
            vnsim_mean: 0.0, snr_mean_db: 0.0,
            total_dropout_count: 0, total_dropout_duration_ms: 0.0,
            segments_with_noise: 0, segments_with_hf_loss: 0, segments_with_artifacts: 0,
            overall_rating: "无数据".to_string(),
        };
    }

    let mos_values: Vec<f64> = segments.iter().map(|s| s.quality.moslqo).collect();
    let mos_mean = mos_values.iter().sum::<f64>() / n as f64;
    let mos_min = mos_values.iter().cloned().fold(f64::INFINITY, f64::min);
    let mos_max = mos_values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let mos_var = mos_values.iter().map(|v| (v - mos_mean).powi(2)).sum::<f64>() / n as f64;
    let mos_stddev = mos_var.sqrt();

    let vnsim_mean = segments.iter().map(|s| s.quality.vnsim).sum::<f64>() / n as f64;
    let snr_mean = segments.iter().map(|s| s.snr.snr_db).sum::<f64>() / n as f64;

    let total_dropout_count: usize = segments.iter().map(|s| s.dropouts.count).sum();
    let total_dropout_duration_ms: f64 = segments.iter().map(|s| s.dropouts.total_duration_ms).sum();

    let segments_with_noise = segments.iter().filter(|s| s.diagnosis.background_noise_detected).count();
    let segments_with_hf_loss = segments.iter().filter(|s| s.diagnosis.high_freq_loss_detected).count();
    let segments_with_artifacts = segments.iter().filter(|s| s.diagnosis.intermittent_artifacts_detected).count();

    let overall_rating = match mos_mean {
        s if s >= 4.5 => "优秀".to_string(),
        s if s >= 4.0 => "良好".to_string(),
        s if s >= 3.5 => "一般".to_string(),
        s if s >= 3.0 => "较差".to_string(),
        s if s >= 2.0 => "差".to_string(),
        _ => "极差".to_string(),
    };

    OverallStats {
        segment_count: n,
        moslqo_mean: mos_mean,
        moslqo_min: mos_min,
        moslqo_max: mos_max,
        moslqo_stddev: mos_stddev,
        vnsim_mean,
        snr_mean_db: snr_mean,
        total_dropout_count,
        total_dropout_duration_ms,
        segments_with_noise,
        segments_with_hf_loss,
        segments_with_artifacts,
        overall_rating,
    }
}

/// 控制台输出报告
pub fn print_console_report(report: &EvaluationReport) {
    println!("\n{}", "=".repeat(60));
    println!("                    音频质量评估报告");
    println!("{}", "=".repeat(60));

    // 基本信息
    println!("\n【基本信息】");
    println!("  参考音频: {}", report.config.reference_path);
    println!("  录制音频: {}", report.config.recorded_path);
    println!("  参考时长: {:.2}s, 录制时长: {:.2}s", 
             report.reference_duration_s, report.recorded_duration_s);
    println!("  采样率: {} Hz, 模式: {}", 
             report.config.target_sample_rate,
             format!("{} Hz", report.config.target_sample_rate));

    // 对齐信息
    println!("\n【时间对齐】");
    println!("  传输延迟: {:.1} ms", report.alignment.delay_ms);
    println!("  对齐置信度: {:.2}%", report.alignment.confidence * 100.0);

    // 整体统计
    let o = &report.overall;
    println!("\n【整体统计】");
    println!("  分段数: {}", o.segment_count);
    println!("  MOS-LQO: 均值={:.2}, 最小={:.2}, 最大={:.2}, 标准差={:.2}",
             o.moslqo_mean, o.moslqo_min, o.moslqo_max, o.moslqo_stddev);
    println!("  VNSIM 均值: {:.4}", o.vnsim_mean);
    println!("  SNR 均值: {:.1} dB", o.snr_mean_db);
    println!("  整体评级: {}", o.overall_rating);

    // 问题统计
    if o.segments_with_noise > 0 || o.segments_with_hf_loss > 0 || o.segments_with_artifacts > 0 {
        println!("\n【问题统计】");
        if o.segments_with_noise > 0 {
            println!("  背景噪声检出: {}/{} 段", o.segments_with_noise, o.segment_count);
        }
        if o.segments_with_hf_loss > 0 {
            println!("  高频损失检出: {}/{} 段", o.segments_with_hf_loss, o.segment_count);
        }
        if o.segments_with_artifacts > 0 {
            println!("  间歇性杂音检出: {}/{} 段", o.segments_with_artifacts, o.segment_count);
        }
        if o.total_dropout_count > 0 {
            println!("  卡顿/丢包: {} 次, 总时长 {:.1} ms", 
                     o.total_dropout_count, o.total_dropout_duration_ms);
        }
    }

    // 各段详细结果
    println!("\n{}", "-".repeat(60));
    println!("                    各段详细评分");
    println!("{}", "-".repeat(60));

    for seg in &report.segments {
        println!("\n  第 {}/{} 段 ({:.2}s - {:.2}s)", 
                 seg.segment_index + 1, report.segments.len(),
                 seg.start_time_s, seg.end_time_s);
        println!("    MOS-LQO: {:.2}  VNSIM: {:.4}  SNR: {:.1} dB",
                 seg.quality.moslqo, seg.quality.vnsim, seg.snr.snr_db);
        println!("    评级: {}", seg.diagnosis.quality_rating);

        // 频段分析摘要
        if !seg.quality.fvnsim.is_empty() {
            let low_n = (seg.quality.fvnsim.len() / 3).max(1);
            let high_n = seg.quality.fvnsim.len().saturating_sub(seg.quality.fvnsim.len() * 2 / 3);
            let low_sim: f64 = seg.quality.fvnsim.iter().take(low_n).sum::<f64>() / low_n as f64;
            let high_sim = if high_n > 0 {
                seg.quality.fvnsim.iter().rev().take(high_n).sum::<f64>() / high_n as f64
            } else {
                seg.quality.vnsim
            };
            println!("    低频相似度: {:.4}  高频相似度: {:.4}", low_sim, high_sim);
        }

        // 诊断
        let mut issues = Vec::new();
        if seg.diagnosis.background_noise_detected { issues.push("背景噪声".to_string()); }
        if seg.diagnosis.high_freq_loss_detected { issues.push("高频损失".to_string()); }
        if seg.diagnosis.intermittent_artifacts_detected { issues.push("间歇性杂音".to_string()); }
        if seg.dropouts.count > 0 {
            let dropout_msg = format!("卡顿{}次({:.0}ms)", seg.dropouts.count, seg.dropouts.total_duration_ms);
            issues.push(dropout_msg);
        }
        if issues.is_empty() {
            println!("    诊断: 无明显异常");
        } else {
            println!("    诊断: {}", issues.join(", "));
        }

        // 最差 patch
        if let Some((worst_sim, start, end)) = seg.diagnosis.worst_patch {
            println!("    最差 patch: 相似度 {:.4}, 时间 {:.2}s - {:.2}s", worst_sim, start, end);
        }
    }

    println!("\n{}", "=".repeat(60));
}
