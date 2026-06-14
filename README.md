# AudioBench - 音频质量评估工具

## 概述

集成了官方 ViSQOL 的音频质量评估命令行工具，单 EXE 运行。
通过对比参考音频和录制音频，输出 ViSQOL 兼容的音质指标。

## 功能特性

- **信号对齐**: FFT 互相关 + 归一化相关系数，分段局部对齐
- **ViSQOL 兼容指标**: MOS-LQO、VNSIM、fVNSIM、频段分析
- **分段评估**: 录制长于参考时，每段独立对齐评分
- **自动适配采样率**: 默认使用输入文件采样率

## 前置要求

### 安装 ViSQOL

1. **Windows**: 从源码编译 visqol.exe
   ```bash
   # 安装 Bazel (https://bazel.build/)
   bazel build :visqol -c opt
   # 输出: bazel-bin/visqol.exe
   ```

2. **设置环境变量**
   ```cmd
   set VISQOL_PATH=C:\path\to\visqol.exe
   ```
   或者使用 `--visqol-path` 参数指定路径。

### 输入要求

- **语音模式**: 16kHz 采���率，使用 `--use_speech_mode`
- **音频模式**: 48kHz 采样率
- **格式**: WAV (单声道或多声道，自动混音为单声道)

## 使用方法

### 命令行参数

```
audio_bench -r <参考���频> -c <录制音频> [选项]
```

### 参数说明

| 参数 | 说明 |
|------|------|
| `-r, --reference` | 参考音频文件（WAV） |
| `-c, --recorded` | 录制音频文件（WAV） |
| `--visqol-path` | visqol 二进制路径（可选，默认从环境变量读取） |
| `-o, --output` | 输出 JSON 报告 |

### 示例

```bash
# 使用环境变量
export VISQOL_PATH=/path/to/visqol
./audio_bench -r ref.wav -c rec.wav

# 指定 visqol 路径
./audio_bench -r ref.wav -c rec.wav --visqol-path /usr/local/bin/visqol

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

## 运行

```bash
# 确保 VISQOL_PATH 环境变量已设置
./target/release/audio_bench -r ref.wav -c rec.wav
```

## 工作流程

1. 加载参考音频和录制音频
2. 使用 FFT 互相关进行多峰检测，定位参考音频在录制中的所有出现位置
3. 对每个位置，提取对应长度的音频段
4. 调用 visqol 命令行工具进行质量评估
5. 汇总各段结果，生成整体统计报告
