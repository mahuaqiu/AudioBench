# AudioBench - 音频质量评估工具

## 概述

集成了官方 ViSQOL 和 DNSMOS 的音频质量评估命令行工具，单 EXE 运行。
提供两种评估方式：提供参考音频时进行有参考 ViSQOL 对比；不提供参考音频时直接对录制音频进行 DNSMOS 无参考评估。

## 功能特性

- **单 EXE 运行**: 编译时嵌入 visqol 二进制和模型文件，运行时自动释放到临时目录，无需环境变量
- **自动采样率适配**: 输入音频自动重采样到 ViSQOL 所需采样率
- **双模式支持**: 有参考评估支持音频模式（--audio，48kHz）和语音模式（默认，16kHz），两种模式均自动重采样
- **信号对齐**: FFT 互相关 + 归一化相关系数，多峰检测分段对齐
- **ViSQOL 兼容指标**: MOS-LQO、VNSIM、fVNSIM、频段分析
- **分段评估**: 录制长于参考时，自动检测参考音频的多次出现，每段独立对齐评分
- **DNSMOS 无参考评估**: 不需要参考音频，直接输出 SIG、BAK、OVRL 三项分数

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
# 有参考评估
audio_bench -r <参考音频> -c <录制音频> [选项]

# 无参考 DNSMOS 评估
audio_bench -c <录制音频> [选项]
```

### 参数说明

| 参数 | 说明 |
|------|------|
| `-r, --reference` | 参考音频文件（WAV，可选；省略时执行 DNSMOS 无参考评估） |
| `-c, --recorded` | 录制音频文件（WAV） |
| `--audio` | 有参考评估时使用音频模式（48kHz），默认使用语音模式（16kHz） |
| `-o, --output` | 输出 JSON 报告（可选） |
| `--html` | 输出 HTML 可视化报告（可选，需指定文件路径） |

### 模式说明

- **语音模式**（默认）: 有参考评估适用于语音通话质量评估。输入音频自动重采样到 16kHz，使用 TFLite 格点模型。
- **音频模式**（`--audio`）: 有参考评估适用于音乐、环境音等。输入音频自动重采样到 48kHz，使用 SVM 模型。
- **无参考模式**（省略 `--reference`）: 直接对录制音频执行 DNSMOS，固定使用 16kHz 模型；`--audio` 在此模式下不生效。

### 示例

```bash
# 音频模式
audio_bench -r ref.wav -c rec.wav --audio

# 语音模式
audio_bench -r ref.wav -c rec.wav

# 无参考 DNSMOS 评估
audio_bench -c rec.wav

# 无参考 DNSMOS 评估并输出 JSON
audio_bench -c rec.wav -o dnsmos.json

# 无参考 DNSMOS 评估并输出 HTML
audio_bench -c rec.wav --html dnsmos.html

# 输出 JSON 报告
audio_bench -r ref.wav -c rec.wav -o report.json

# 输出 HTML 可视化报告
audio_bench -r ref.wav -c rec.wav --html report.html

# 同时输出 JSON 和 HTML 报告
audio_bench -r ref.wav -c rec.wav -o report.json --html report.html

# 语音模式 + HTML 报告
audio_bench -r ref.wav -c rec.wav --html report.html
```

## 输出指标

有参考模式输出：

- **MOS-LQO**: 预测 Mean Opinion Score (1-5)
- **VNSIM**: 全局 NSIM 相似度
- **fVNSIM**: 各频段相似度
- **fVDegEnergy**: 各频段降质能量比
- **异常检测**: 时域中断（丢包/静音）、时轴漂移（音频拉长/压缩）、频谱损伤（PLC 伪造音/杂音）

无参考模式只输出 DNSMOS：

- **DNSMOS-SIG**: 人声信号质量分，越高越好
- **DNSMOS-BAK**: 背景噪声质量分，越高越好
- **DNSMOS-OVRL**: 整体音质分，越高越好

无参考 JSON 使用独立的简化结构，仅包含 `mode`、录制文件信息和 `sig`/`bak`/`ovrl`，不会输出空的 MOS-LQO、VNSIM 或参考音频字段。

## 构建

```bash
cargo build --release
# 输出: target/release/audio_bench.exe
```

**注意**: 首次构建前必须将 `visqol.exe` 放入 `bin/` 目录、模型文件放入 `bin/model/` 目录，否则编译会使用占位文件，运行时会报错。

## 工作流程

有参考模式：

1. 加载参考音频和录制音频，自动混音为单声道
2. 根据模式（音频/语音）自动重采样到目标采样率
3. 使用 FFT 互相关进行多峰检测，定位参考音频在录制中的所有出现位置
4. 对每个位置，提取对应长度的音频段，写入临时 WAV 文件
5. 释放嵌入的 visqol 二进制和模型文件到临时目录
6. 通过 `--similarity_to_quality_model` 指定模型路径，调用 visqol 进行质量评估
7. 解析 ViSQOL 输出的 CSV/JSON 结果
8. 汇总各段结果，并附加 DNSMOS 无参考分数

无参考模式：

1. 只加载录制音频，不要求 `--reference`
2. 将录制音频送入 DNSMOS，内部重采样到 16kHz 并按模型窗口评估
3. 输出 DNSMOS 的 SIG、BAK、OVRL 结果，不执行对齐、ViSQOL 或参考异常检测
