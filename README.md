# AudioBench - 会议音频质量评估工具

## 概述

这是一个用于评估会议音频质量的命令行工具。它通过对比参考音频（原始音频）和录制音频（会议软件录制），分析音频质量并生成评估报告。

## 功能特性

- **信号对齐**: 使用 FFT 互相关算法自动对齐参考音频和录制音频
- **ViSQOL 评分**: 集成 Google ViSQOL 音频质量评估算法，输出 MOS-LQO 分数
- **详细指标**:
  - MOS-LQO 分数（1-5 分）
  - VNSIM（神经元网络相似度）
  - 频段相似度分析（低频/高频）
  - SNR（信噪比）
  - 卡顿/丢包检测
  - 幅值统计（RMS、峰值、削波检测）
  - 问题诊断（背景噪声、高频损失、间歇性杂音）

## 使用方法

### 命令行参数

```
audio_bench --reference <参考音频> --recorded <录制音频> --visqol <ViSQOL目录> [选项]
```

#### 必需参数

- `-r, --reference <文件>` - 参考音频文件（WAV 格式）
- `-c, --recorded <文件>` - 录制音频文件（WAV 格式）
- `-v, --visqol <目录>` - ViSQOL 可执行文件所在目录

#### 可选参数

- `-s, --sample-rate <采样率>` - 目标采样率（默认 48000，语音模式用 16000）
- `--speech` - 使用语音模式（16kHz，推荐会议音频使用）
- `-o, --output <文件>` - 输出 JSON 报告文件路径
- `--keep-temp` - 保留临时文件（用于调试）

### 示例

```bash
# 使用语音模式（推荐会议音频）
./audio_bench --reference ref.wav --recorded rec.wav --visqol ./visqol-bin --speech

# 使用音频模式
./audio_bench --reference ref.wav --recorded rec.wav --visqol ./visqol-bin -o report.json
```

## 输出示例

```
============================================================
                    音频质量评估报告
============================================================

【总体质量】 良好
  - MOS-LQO 分数: 4.12/5.0
  - VNSIM 相似度: 0.9234

【时间对齐】
  - 传输延迟: 125.3 ms
  - 对齐置信度: 98.5%

【信噪比】
  - SNR: 32.5 dB

【卡顿/丢包检测】
  - 卡顿次数: 0
  - 总卡顿时长: 0.0 ms

【问题诊断】
  ✓ 未检测到明显异���
```

## 构建步骤

### 1. 构建 AudioBench

```bash
cargo build --release
# 输出: target/release/audio_bench
```

### 2. 构建 ViSQOL（Windows）

需要先编译 Bazel 项目：

1. 安装 Bazel: https://bazel.build/install/windows
2. 进入 visqol 源码目录
3. 执行: `bazel build :visqol -c opt`
4. 将 `bazel-bin\visqol.exe` 复制到项目目录

## 技术方案

- **主控程序**: Rust 编译，约 1.4MB
- **音频评估**: ViSQOL 子进程调用（约 10MB）
- **总体积**: ~15MB
