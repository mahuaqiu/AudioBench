# AudioBench - 音频质量评估工具

## 概述

纯 Rust 实现的音频质量评估命令行工具，单 EXE 运行，无需外部依赖。
通过对比参考音频和录制音频，输出 ViSQOL 兼容的音质指标。

## 功能特性

- **信号对齐**: FFT 互相关 + 归一化相关系数，分段局部对齐
- **ViSQOL 兼容指标**: MOS-LQO、VNSIM、fVNSIM、频段分析
- **纯 Rust 实现**: 无外部依赖，约 1.5MB
- **分段评估**: 录制长于参考时，每段独立对齐评分
- **自动适配采样率**: 默认使用输入文件采样率

## 使用方法

### 命令行参数

```
audio_bench -r <参考音频> -c <录制音频> [选项]
```

### 参数说明

| 参数 | 说明 |
|------|------|
| `-r, --reference` | 参考音频文件（WAV） |
| `-c, --recorded` | 录制音频文件（WAV） |
| `-s, --sample-rate` | 目标采样率（默认使用输入文件采样率，>0 时强制重采样） |
| `-o, --output` | 输出 JSON 报告 |

### 示例

```bash
# 直接运行（自动使用输入文件的采样率）
./audio_bench -r ref.wav -c rec.wav

# 指定输出
./audio_bench -r ref.wav -c rec.wav -o report.json

# 强制重采样到指定采样率
./audio_bench -r ref.wav -c rec.wav -s 16000
```

## 输出指标

- **MOS-LQO**: 预测 Mean Opinion Score (1-5)
- **VNSIM**: 全局 NSIM 相似度
- **fVNSIM**: 各频段相似度
- **fVDegEnergy**: 各频段降质能量比
- **SNR**: 信噪比
- **卡顿检测**: 丢包/静音事件统计
- **诊断**: 背景噪声、高频损失、间歇杂音

## 构建

```bash
cargo build --release
# 输出: target/release/audio_bench
```

## 运行

```bash
./target/release/audio_bench -r ref.wav -c rec.wav
```

## 技术细节

- **Gammatone 滤波器组**: 模拟人耳听觉特性，ERB 刻度，最高频率 = sample_rate/2
- **NSIM 相似度**: intensity × structure，与 ViSQOL 对齐
- **帧参数**: 80ms 帧长，75% 重叠
- **对齐算法**: 归一化互相关 + 首次显著峰 + 分段局部对齐
