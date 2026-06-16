//! 报告生成模块
//! 汇总所有指标，生成 JSON 报告和控制台输出
//! 支持分段评估：每段独立评分 + 整体统计汇总

use serde::Serialize;
use crate::metrics::{AudioAnomalyReport, LevelResult};
use crate::visqol::VisqolResult as QualityResult;


/// 评估配置信息
#[derive(Debug, Serialize)]
pub struct ReportConfig {
    pub reference_path: String,
    pub recorded_path: String,
    pub target_sample_rate: u32,
}

/// 对齐信息
#[derive(Debug, Serialize)]
pub struct AlignmentInfo {
    pub offset_samples: usize,
    pub delay_ms: f64,
    pub confidence: f64,
}

/// 单段评估结果
#[derive(Debug, Serialize)]
pub struct SegmentResult {
    pub segment_index: usize,
    pub start_time_s: f64,
    pub end_time_s: f64,
    pub quality: QualityResult,
    /// 音频异常检测报告（时域中断、时轴漂移、频谱损伤）
    pub anomaly: AudioAnomalyReport,
    pub level_ref: LevelResult,
    pub level_deg: LevelResult,
    pub band_energy_ratios: Vec<f64>,
}

/// 整体统计汇总
#[derive(Debug, Serialize)]
pub struct OverallStats {
    pub segment_count: usize,
    pub moslqo_mean: f64,
    pub moslqo_min: f64,
    pub moslqo_max: f64,
    pub moslqo_stddev: f64,
    pub vnsim_mean: f64,
}

/// 完整评估报告
#[derive(Debug, Serialize)]
pub struct EvaluationReport {
    pub config: ReportConfig,
    pub alignment: AlignmentInfo,
    pub reference_duration_s: f64,
    pub recorded_duration_s: f64,
    pub overall: OverallStats,
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
            vnsim_mean: 0.0,
        };
    }

    let mos_values: Vec<f64> = segments.iter().map(|s| s.quality.moslqo).collect();
    let mos_mean = mos_values.iter().sum::<f64>() / n as f64;
    let mos_min = mos_values.iter().cloned().fold(f64::INFINITY, f64::min);
    let mos_max = mos_values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let mos_var = mos_values.iter().map(|v| (v - mos_mean).powi(2)).sum::<f64>() / n as f64;
    let mos_stddev = mos_var.sqrt();

    let vnsim_mean = segments.iter().map(|s| s.quality.vnsim).sum::<f64>() / n as f64;

    OverallStats {
        segment_count: n,
        moslqo_mean: mos_mean,
        moslqo_min: mos_min,
        moslqo_max: mos_max,
        moslqo_stddev: mos_stddev,
        vnsim_mean,
    }
}

/// 控制台输出报告
pub fn print_console_report(report: &EvaluationReport) {
    println!("\n{}", "=".repeat(60));
    println!("                    音频质量评估报告");
    println!("{}", "=".repeat(60));

    println!("\n【基本信息】");
    println!("  参考音频: {}", report.config.reference_path);
    println!("  录制音频: {}", report.config.recorded_path);
    println!("  参考时长: {:.2}s, 录制时长: {:.2}s", 
             report.reference_duration_s, report.recorded_duration_s);
    println!("  采样率: {} Hz, 模式: {}", 
             report.config.target_sample_rate,
             format!("{} Hz", report.config.target_sample_rate));

    println!("\n【时间对齐】");
    println!("  传输延迟: {:.1} ms", report.alignment.delay_ms);
    println!("  对齐置信度: {:.2}%", report.alignment.confidence * 100.0);

    let o = &report.overall;
    println!("\n【整体统计】");
    println!("  分段数: {}", o.segment_count);
    println!("  MOS-LQO: 均值={:.2}, 最小={:.2}, 最大={:.2}, 标准差={:.2}",
             o.moslqo_mean, o.moslqo_min, o.moslqo_max, o.moslqo_stddev);
    println!("  VNSIM 均值: {:.4}", o.vnsim_mean);

    // 异常统计汇总
    let n = report.segments.len();
    let total_dropout: f64 = report.segments.iter().map(|s| s.anomaly.dropout_duration_ms.abs()).sum();
    let total_warping: f64 = report.segments.iter().map(|s| s.anomaly.warping_duration_ms.abs()).sum();
    let total_truncation: f64 = report.segments.iter().map(|s| s.anomaly.truncation_duration_ms.abs()).sum();
    let avg_spectral: f64 = if n == 0 { 0.0 } else {
        report.segments.iter().map(|s| s.anomaly.spectral_artifacts_score).sum::<f64>() / n as f64
    };
    println!("  异常检测: 时域中断={:.0}ms, 时轴漂移={:.0}ms, 内容截断={:.0}ms, 频谱损伤={:.2}",
             total_dropout, total_warping, total_truncation, avg_spectral);

    println!("\n{}", "-".repeat(60));
    println!("                    各段详细评分");
    println!("{}", "-".repeat(60));

    for seg in &report.segments {
        println!("\n  第 {}/{} 段 ({:.2}s - {:.2}s)", 
                 seg.segment_index + 1, report.segments.len(),
                 seg.start_time_s, seg.end_time_s);
        println!("    MOS-LQO: {:.2}  VNSIM: {:.4}",
                 seg.quality.moslqo, seg.quality.vnsim);

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

        let energy_mean = if seg.band_energy_ratios.is_empty() {
            0.0
        } else {
            seg.band_energy_ratios.iter().sum::<f64>() / seg.band_energy_ratios.len() as f64
        };
        println!("    能量比均值: {:.4}", energy_mean);

        // 异常检测结果
        if seg.anomaly.has_anomaly {
            if seg.anomaly.dropout_duration_ms > 0.0 {
                println!("    时域中断: {:.0}ms ({}次)", 
                         seg.anomaly.dropout_duration_ms, seg.anomaly.dropouts.len());
            }
            if !seg.anomaly.warpings.is_empty() {
                // 显示漂移时长，绝对值避免 -0ms
                let warping_ms = seg.anomaly.warping_duration_ms.abs();
                println!("    时轴漂移: {:.0}ms ({}次)", 
                         warping_ms, seg.anomaly.warpings.len());
            }
            if seg.anomaly.truncation_duration_ms > 0.0 {
                println!("    内容截断: {:.0}ms ({}次)", 
                         seg.anomaly.truncation_duration_ms, seg.anomaly.truncations.len());
            }
            if seg.anomaly.spectral_artifacts_score > 0.25 {
                println!("    频谱损伤: {:.1}%", 
                         seg.anomaly.spectral_artifacts_score * 100.0);
            }
        } else {
            println!("    异常检测: 无");
        }

        println!("    参考幅值: RMS={:.4}, 峰值={:.4}", seg.level_ref.rms, seg.level_ref.peak);
        println!("    录制幅值: RMS={:.4}, 峰值={:.4}", seg.level_deg.rms, seg.level_deg.peak);
    }

    println!("\n{}", "=".repeat(60));
}
