//! 音频加载与预处理模块
//! 负责 WAV 解码、重采样（线性插值）、单声道化
//! 
//! 注意：为了简化依赖，高质量重采样需要用户自行处理（如使用 ffmpeg）
//! 本模块提供简单的线性插值重采样，仅适用于采样率差异不大的情况

use hound::{WavReader, WavWriter, SampleFormat};
use std::path::Path;

/// 简单的线性插值重采样
fn linear_resample(samples: &[f64], from_rate: u32, to_rate: u32) -> Vec<f64> {
    if from_rate == to_rate {
        return samples.to_vec();
    }
    
    let ratio = to_rate as f64 / from_rate as f64;
    let new_len = ((samples.len() as f64) * ratio) as usize;
    let mut output = Vec::with_capacity(new_len);
    
    for i in 0..new_len {
        let src_pos = i as f64 / ratio;
        let idx = src_pos as usize;
        let frac = src_pos.fract();
        
        if idx + 1 < samples.len() {
            let val = samples[idx] * (1.0 - frac) + samples[idx + 1] * frac;
            output.push(val);
        } else if idx < samples.len() {
            output.push(samples[idx]);
        } else {
            output.push(0.0);
        }
    }
    
    output
}

/// 音频数据（单声道 f64）
pub struct AudioData {
    pub samples: Vec<f64>,
    pub sample_rate: u32,
}

impl AudioData {
    /// 从 WAV 文件加载音频，自动转为单声道 f64
    pub fn from_wav(path: &Path) -> Result<Self, String> {
        let mut reader = WavReader::open(path)
            .map_err(|e| format!("无法打开 WAV 文件 {:?}: {}", path, e))?;

        let spec = reader.spec();
        let channels = spec.channels as usize;
        let sample_rate = spec.sample_rate;

        let raw: Vec<f64> = match spec.sample_format {
            SampleFormat::Float => {
                reader.samples::<f32>()
                    .map(|s| s.map(|v| v as f64).map_err(|e| e.to_string()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("读取采样失败: {}", e))?
            }
            SampleFormat::Int => {
                let max_val = (1i64 << (spec.bits_per_sample - 1)) as f64;
                reader.samples::<i32>()
                    .map(|s| s.map(|v| v as f64 / max_val).map_err(|e| e.to_string()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("读取采样失败: {}", e))?
            }
        };

        // 多声道转单声道：取各声道平均值
        let mono = if channels > 1 {
            let frame_count = raw.len() / channels;
            let mut out = Vec::with_capacity(frame_count);
            for i in 0..frame_count {
                let sum: f64 = (0..channels).map(|ch| raw[i * channels + ch]).sum();
                out.push(sum / channels as f64);
            }
            out
        } else {
            raw
        };

        Ok(AudioData {
            samples: mono,
            sample_rate,
        })
    }

    /// 重采样到目标采样率
    pub fn resample(&self, target_rate: u32) -> Result<Self, String> {
        if self.sample_rate == target_rate {
            return Ok(AudioData {
                samples: self.samples.clone(),
                sample_rate: target_rate,
            });
        }

        // 使用简单的线性插值重采样
        // 注意：对高品质音频，建议使用 ffmpeg 预先重采样
        let output = linear_resample(&self.samples, self.sample_rate, target_rate);

        Ok(AudioData {
            samples: output,
            sample_rate: target_rate,
        })
    }

    /// 保存为 WAV 文件（16bit PCM）
    pub fn save_wav(&self, path: &Path) -> Result<(), String> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: self.sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec)
            .map_err(|e| format!("无法创建 WAV 文件 {:?}: {}", path, e))?;

        for &s in &self.samples {
            let val = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
            writer.write_sample(val)
                .map_err(|e| format!("写入采样失败: {}", e))?;
        }

        writer.finalize()
            .map_err(|e| format!("写入 WAV 失败: {}", e))?;
        Ok(())
    }

    /// 音频时长（秒）
    pub fn duration_secs(&self) -> f64 {
        self.samples.len() as f64 / self.sample_rate as f64
    }
}
