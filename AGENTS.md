# Repository Guidelines

## 项目简介

AudioBench 是音频质量评估命令行工具，纯 Rust 实现，单 EXE 运行，无需外部依赖。
核心功能：将参考音频与录制音频对齐、分段评估，输出 ViSQOL 兼容的音质指标。

## 项目结构

```
src/
  main.rs        # 入口：参数解析、分段评估主流程
  quality.rs     # 纯 Rust 音质评估（ViSQOL 兼容指标：MOS-LQO、VNSIM、fVNSIM 等）
  gammatone.rs   # Gammatone 滤波器组 + 预处理（SPL 归一化、dB 转换、噪声门限）
  spectrogram.rs # NSIM 类 SSIM 相似度计算（intensity × structure，与 ViSQOL 对齐）
  alignment.rs   # FFT 互相关信号对齐
  audio_io.rs    # WAV 解码、重采样、单声道化
  metrics.rs     # SNR、卡顿检测、幅值统计
  report.rs      # 分段报告 + 整体统计，JSON/控制台输出
```

## 构建 & 运行

```bash
cargo check              # 编译检查
cargo build --release    # 发布构建（~1.5MB，LTO + strip）
audio_bench --reference ref.wav --recorded rec.wav --speech
audio_bench -r ref.wav -c rec.wav -o report.json
```

关键参数：`--speech` 语音模式 (16kHz)；`--sample-rate` 自定义采样率；`--output` 输出 JSON。

## 核心算法

- **Gammatone 滤波器组**：模拟人耳听觉特性，替代 FFT 频谱分析
- **NSIM 相似度**：intensity × structure 公式，与 ViSQOL 的 neurogram similarity 对齐
- **帧参数**：80ms 帧长，75% 重叠，与 ViSQOL 一致
- **预处理**：SPL 归一化 → dB 转换 → 噪声门限
- **分段评估**：录制音频长于参考音频时，按参考长度循环分段，每段独立评分并汇总

输出指标：MOS-LQO（映射后 MOS）、VNSIM（全局 NSIM）、fVNSIM（各频段 NSIM）、fVDegEnergy（各频段降质能量比）、fstdnsim（NSIM 标准差）、SNR、卡顿检测。

## 编码规范

- 语言：代码注释和文档使用中文
- Rust edition 2021，`cargo fmt` 格式化
- 错误处理用 `Result<T, String>` 或 `Box<dyn Error>`
- 依赖尽量精简，保持单 EXE 无外部依赖的目标

## 测试

```bash
cargo test
```

目前以集成测试为主，使用实际 WAV 文件验证分段评估流程。

## 提交规范

- 中文描述，简明扼要
- 一次提交聚焦一个改动点
- 发布构建前务必 `cargo check` 和 `cargo build --release` 通过
