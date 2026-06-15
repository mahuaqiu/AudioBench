# AudioBench - 音频质量评估工具

## 概述

集成了官方 ViSQOL 的音频质量评估命令行工具，单 EXE 运行。
通过对比参考音频和录制音频，输出 ViSQOL 兼容的音质指标。

## 功能特性

- **单 EXE 运行**: 编译时嵌入 visqol 二进制和模型文件，运行时自动释放到临时目录，无需环境变量
- **自动采样率适配**: 输入音频自动重采样到 ViSQOL 所需采样率
- **双模式支持**: 音频模式（默认，48kHz）和语音模式（--speech，16kHz），两种模式均自动重采样
- **信号对齐**: FFT 互相关 + 归一化相关系数，多峰检测分段对齐
- **ViSQOL 兼容指标**: MOS-LQO、VNSIM、fVNSIM、频段分析
- **分段评估**: 录制长于参考时，自动检测参考音频的多次出现，每段独立对齐评分

## 前置要求

### 编译 visqol 二进制

在 Windows 上从源码编译 `visqol.exe`:

```bash
# 安装 Bazel (https://bazel.build/)
bazel build :visqol -c opt
# 输出: bazel-bin/visqol.exe
```

### 准备文件

将编译好的 `visqol.exe` 和 ViSQOL 模型文件放入项目目录:

```
AudioBench/
├── bin/
│   ├── visqol.exe                                          # 编译好的 ViSQOL 二进制
│   └── model/
│       ├── libsvm_nu_svr_model.txt                         # 音频模式 SVM 模型
│       └── lattice_..._raw.tflite                          # 语音模式 TFLite 模型
├── src/
├── Cargo.toml
└── README.md
```

模型文件来自 ViSQOL 源码的 `model/` 目录：
- `libsvm_nu_svr_model.txt` — 音频模式使用
- `lattice_tcditugenmeetpackhref_ls2_nl60_lr12_bs2048_learn.005_ep2400_train1_7_raw.tflite` — 语音模式使用

编译时，这些文件会通过 `include_bytes!` 嵌入到 Rust 二进制中，最终用户只需一个 EXE 即可运行。

### 输入要求

- **格式**: WAV（单声道或多声道，自动混音为单声道）
- **采样率**: 任意，工具会自动重采样到 ViSQOL 所需采样率

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
| `--speech` | 使用语音模式（16kHz），默认为音频模式（48kHz） |
| `-o, --output` | 输出 JSON 报告（可选） |

### 模式说明

- **音频模式**（默认）: 适用于音乐、环境音等。输入音频自动重采样到 48kHz，使用 SVM 模型。
- **语音模式**（`--speech`）: 适用于语音通话质量评估。输入音频自动重采样到 16kHz，使用 TFLite 格点模型。

### 示例

```bash
# 音频模式（默认）
audio_bench -r ref.wav -c rec.wav

# 语音模式
audio_bench -r ref.wav -c rec.wav --speech

# 输出 JSON 报告
audio_bench -r ref.wav -c rec.wav -o report.json
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
# 输出: target/release/audio_bench.exe
```

**注意**: 首次构建前必须将 `visqol.exe` 放入 `bin/` 目录、模型文件放入 `bin/model/` 目录，否则编译会使用占位文件，运行时会报错。

## 工作流程

1. 加载参考音频和录制音频，自动混音为单声道
2. 根据模式（音频/语音）自动重采样到目标采样率
3. 使用 FFT 互相关进行多峰检测，定位参考音频在录制中的所有出现位置
4. 对每个位置，提取对应长度的音频段，写入临时 WAV 文件
5. 释放嵌入的 visqol 二进制和模型文件到临时目录
6. 通过 `--similarity_to_quality_model` 指定模型路径，调用 visqol 进行质量评估
7. 解析 ViSQOL 输出的 CSV/JSON 结果
8. 汇总各段结果，生成整体统计报告
