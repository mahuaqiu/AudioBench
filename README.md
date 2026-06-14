# AudioBench - 音频质量评估工具

## 概述

集成了官方 ViSQOL 的音频质量评估命令行工具，单 EXE 运行。
通过对比参考音频和录制音频，输出 ViSQOL 兼容的音质指标。

## 功能特性

- **单 EXE 运行**: 编译时嵌入 visqol 二进制，运行时自动释放到临时目录
- **自动采样率适配**: 输入音频自动重采样到 ViSQOL 所需采样率（16kHz/48kHz）
- **信号对齐**: FFT 互相关 + 归一化相关系数，分段局部对齐
- **ViSQOL 兼容指标**: MOS-LQO、VNSIM、fVNSIM、频段分析
- **分段评估**: 录制长于参考时，每段独立对齐评分

## 前置要求

### 编译 visqol 二进制

在 Windows 上从源码编译 `visqol.exe`:

```bash
# 安装 Bazel (https://bazel.build/)
bazel build :visqol -c opt
# 输出: bazel-bin/visqol.exe
```

将编译好的 `visqol.exe` 复制到项目根目录的 `bin/` 文件夹下:

```
AudioBench/
├── bin/
│   └── visqol.exe    # 编译好的 ViSQOL 二进制
├── src/
├── Cargo.toml
└── README.md
```

### 输入要求

- **格式**: WAV (单声���或多声道，自动混音为单声道)
- **采样率**: 任意，工具会自动重采样到 ViSQOL 所需采样率
  - ≤ 16kHz: 使用语音模式 (16kHz)
  - > 16kHz: 使用音频模式 (48kHz)

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
| `-o, --output` | 输出 JSON 报告（可选） |

### 示例

```bash
# 基本用法
./audio_bench -r ref.wav -c rec.wav

# 输出 JSON 报告
./audio_bench -r ref.wav -c rec.wav -o report.json
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

首次构建时需要将 `visqol.exe` 放入 `bin/` 目录，否则编译会使用空二进制。

## 工作流程

1. 加载参考音频和录制音频，自动检测采样率
2. 自动选择 ViSQOL 模式（语音 16kHz / 音频 48kHz）并重采样
3. 使用 FFT 互相关进行多峰检测，定位参考音频在录制中的所有出现位置
4. 对每个位置，提取对应长度的音频段，调用 visqol 进行质量评估
5. 汇总各段结果，生成整体统计报告
