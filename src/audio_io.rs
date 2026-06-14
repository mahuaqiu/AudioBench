//! 音频加载与预处理模块
//! 负责 WAV 解码、重采样（线性插值）、单声道化

use hound::{WavReader, SampleFormat};
use std::path::Path;

/// 简单的线性插值重采样
#[allow(dead_code)]
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
    #[allow(dead_code)]
    pub fn resample(&self, target_rate: u32) -> Result<Self, String> {
        if self.sample_rate == target_rate {
            return Ok(AudioData {
                samples: self.samples.clone(),
                sample_rate: target_rate,
            });
        }

        let output = linear_resample(&self.samples, self.sample_rate, target_rate);

        Ok(AudioData {
            samples: output,
            sample_rate: target_rate,
        })
    }

    /// 音频时长（秒）
    pub fn duration_secs(&self) -> f64 {
        self.samples.len() as f64 / self.sample_rate as f64
    }
}

/// 写入单声道 WAV 文件（供 visqol 调用）
pub fn write_wav_mono(path: &std::path::Path, samples: &[f64], sample_rate: u32) -> Result<(), String> {
    use hound::{WavWriter, WavSpec, SampleFormat};
    
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    
    let mut writer = WavWriter::create(path, spec)
        .map_err(|e| format!("创建 WAV 文件失败: {}", e))?;
    
    let max_val = 32767.0f64;
    for &sample in samples {
        let s = (sample * max_val).clamp(-32768.0, 32767.0) as i32;
        writer.write_sample(s)
            .map_err(|e| format!("写入采样失败: {}", e))?;
    }
    
    writer.finalize()
        .map_err(|e| format!("关闭 WAV 文件失败: {}", e))?;
    
    Ok(())
}
