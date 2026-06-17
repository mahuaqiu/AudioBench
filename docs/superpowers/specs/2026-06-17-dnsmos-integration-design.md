# DNSMOS 指标集成设计文档

**日期**: 2026-06-17  
**项目**: AudioBench  
**主题**: 集成微软 DNSMOS 指标

## 1. 需求概述

在 AudioBench 中增加微软 DNSMOS 指标，对输入音频进行质量评估。

### 1.1 核心需求

- 新增 3 个 DNSMOS 指标：**SIG**（人声信号分）、**BAK**（背景噪声分）、**OVRL**（整体综合分）
- 分数范围：1.0 到 5.0，符合 ITU-T P.835 标准
- **DNSMOS 是无参考（no-reference）指标，仅对录制音频评分**，不需要参考音频
- HTML 报告中：
  - 保留原 MOS-LQO 折线图
  - 新增 SIG/BAK/OVRL 三线折线图，与 MOS-LQO 图并排显示
  - 数值卡片区域增加 SIG、BAK、OVRL 三个卡片
  - 指标说明中增加 DNSMOS 三个指标的详细描述
- 控制台报告和 JSON 报告都输出 SIG、BAK、OVRL

### 1.2 技术约束

- AudioBench 是 Rust 项目，需纯 Rust 实现（不使用 Python 依赖）
- 统一使用 WB（Wideband，16kHz）模型：48kHz 音频重采样到 16kHz 后评分
- ONNX 模型编译时嵌入二进制（通过 build.rs）
- 使用 `ort` crate 加载 ONNX 模型进行推理

## 2. 架构设计

### 2.1 文件变更

| 操作 | 文件路径 | 说明 |
|------|----------|------|
| 新增 | `src/dnsmos.rs` | DNSMOS 核心计算模块，使用 `include_bytes!` 嵌入 ONNX 模型 |
| 修改 | `Cargo.toml` | 增加 `ort`（static feature）和 `ndarray` 依赖 |
| 修改 | `build.rs` | 增加 `sig_bak_ovr.onnx` 模型文件的 hash 计算（供构建缓存使用） |
| 修改 | `main.rs` | 增加 DNSMOS 评估调用 |
| 修改 | `src/report.rs` | `SegmentResult` 增加 `sig/bak/ovrl: Option<f64>` 字段 |
| 修改 | `src/html_report.rs` | 增加 DNSMOS 折线图和指标卡片 |

### 2.2 模型文件

```
bin/
└── model/
    └── sig_bak_ovr.onnx   # DNSMOS 主模型（~1.1MB），从源码仓库复制
```

**注意**：不嵌入 P.808 模型（model_v8.onnx），仅使用主模型输出 SIG/BAK/OVRL。

## 3. DNSMOS 核心计算（dnsmos.rs）

### 3.1 推理链

```
输入音频（任意采样率）
    │
    ▼
重采样到 16kHz（复用现有 audio_io::resample）
    │
    ▼
滑窗切分：窗口 9.01s（144160 采样点），步进 1s（16000 采样点）
    ├─ 音频 ≥ 9.01s：正常滑窗
    └─ 音频 < 9.01s：指数翻倍拼接（`while len(audio) < len_samples: audio = np.append(audio, audio)`），直到 >= 9.01s
        - **警告**：对极短音频（< 3s）评估结果可能无意义，建议在报告中添加警告标记
    │
    ▼
逐窗 ONNX 推理：
    输入 input_1: [1, 144160] (float32)
    输出 1 个形状为 [1, 3] 的张量（名称为 "Identity:0"），依次为 [SIG_raw, BAK_raw, OVRL_raw]
    │
    ▼
多项式校准（对每个 raw 值）：
    SIG  = -0.08397278 * SIG_raw² + 1.22083953 * SIG_raw + 0.0052439
    BAK  = -0.13166888 * BAK_raw² + 1.60915514 * BAK_raw - 0.39604546
    OVRL = -0.06766283 * OVRL_raw² + 1.11546468 * OVRL_raw + 0.04602535
    │
    ▼
所有窗取均值 → 返回 DnsMosResult { sig, bak, ovrl }
```

### 3.2 多项式校准系数（来自 Python 源码，非个性化版本）

```rust
/// DNSMOS 多项式校准系数（非个性化模型）
const POLY_SIG: (f64, f64, f64) = (-0.08397278, 1.22083953, 0.0052439);
const POLY_BAK: (f64, f64, f64) = (-0.13166888, 1.60915514, -0.39604546);
const POLY_OVRL: (f64, f64, f64) = (-0.06766283, 1.11546468, 0.04602535);

/// 多项式计算: a*x² + b*x + c
fn polyfit(x: f64, (a, b, c): (f64, f64, f64)) -> f64 {
    a * x * x + b * x + c
}
```

### 3.3 公开 API

```rust
use ndarray::Array;

const POLY_SIG: (f64, f64, f64) = (-0.08397278, 1.22083953, 0.0052439);
const POLY_BAK: (f64, f64, f64) = (-0.13166888, 1.60915514, -0.39604546);
const POLY_OVRL: (f64, f64, f64) = (-0.06766283, 1.11546468, 0.04602535);

/// DNSMOS 多项式计算: a*x² + b*x + c
fn polyfit(x: f64, (a, b, c): (f64, f64, f64)) -> f64 {
    a * x * x + b * x + c
}

/// DNSMOS 评估结果
pub struct DnsMosResult {
    pub sig: f64,   // 人声信号分 (1.0-5.0)
    pub bak: f64,   // 背景噪声分 (1.0-5.0)
    pub ovrl: f64,  // 整体综合分 (1.0-5.0)
}

/// DNSMOS 评估器
pub struct DnsMosEvaluator {
    session: ort::Session,  // ONNX 推理会话
}

impl DnsMosEvaluator {
    /// 从嵌入的模型字节创建评估器
    pub fn new(model_bytes: &[u8]) -> Result<Self> {
        let session = ort::Session::builder()?
            .commit_from_memory(model_bytes)?;
        Ok(Self { session })
    }

    /// 对一段音频进行评分（支持任意采样率，内部自动重采样到 16kHz）
    pub fn evaluate(&self, samples: &[f64], sample_rate: u32) -> Result<DnsMosResult> {
        // 1. 重采样到 16kHz（内部处理）
        let samples_16k = resample_to_16kHz(samples, sample_rate)?;

        // 2. 滑窗切分 + 逐窗推理 + 多项式校准
        // ... 详细实现见 3.1 节推理链
        // 返回前裁剪到 [1.0, 5.0] 范围

        Ok(DnsMosResult { sig, bak, ovrl })
    }
}
```

**注意**：`ort` crate 使用 `Session::builder()?.commit_from_memory()` 模式加载模型。

### 3.4 依赖配置（Cargo.toml）

```toml
[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
hound = "3.5"
rustfft = "6.4.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# 新增依赖
ort = { version = "2", default-features = false, features = ["static"] }  # 静态链接 onnxruntime
ndarray = "0.16" # ort 的数组输入格式
```

**重要**：使用 `ort` 的 `static` feature 进行静态链接，确保单 EXE 运行，无需目标机器安装 onnxruntime 动态库。

## 4. 数据流集成

### 4.1 main.rs 修改

程序启动时一次性加载模型（嵌入的 ONNX 模型字节）：

```rust
// 启动时
const DNSMOS_MODEL: &[u8] = include_bytes!("../bin/model/sig_bak_ovr.onnx");
let dnsmos_evaluator = DnsMosEvaluator::new(DNSMOS_MODEL)
    .expect("无法加载 DNSMOS 模型");
```

每个对齐段的评估流程：

```
现有流程：
  1. 提取段录制数据 → 2. 创建临时 WAV → 3. 调用 ViSQOL（需要参考+录制） → 4. 解析结果

新增步骤（DNSMOS 仅对录制音频评分）：
  1. 提取段录制数据 → 2. 创建临时 WAV → 3. 调用 ViSQOL → 4. 解析结果
                                                            → 5. 调用 DNSMOS（仅录制音频） → 6. 收集 sig/bak/ovrl
```

- DNSMOS 不需要临时 WAV 文件，直接接收采样数据（比 ViSQOL 更简洁）
- 每段评估后调用 `dnsmos_evaluator.evaluate(&segment_samples_16khz)`

### 4.2 report.rs 修改

`SegmentResult` 结构体扩展：

```rust
pub struct SegmentResult {
    pub segment_index: usize,
    pub start_time_s: f64,
    pub end_time_s: f64,
    pub quality: QualityResult,  // 现有 ViSQOL 结果
    pub anomaly: AudioAnomalyReport,
    pub level_ref: LevelResult,
    pub level_deg: LevelResult,
    pub band_energy_ratios: Vec<f64>,

    // 新增 DNSMOS 字段（使用 Option 便于表示评估失败）
    pub sig: Option<f64>,   // 人声信号分，None 表示评估失败
    pub bak: Option<f64>,   // 背景噪声分，None 表示评估失败
    pub ovrl: Option<f64>,  // 整体综合分，None 表示评估失败
}
```

`OverallStats` 扩展：

```rust
pub struct OverallStats {
    pub segment_count: usize,
    pub moslqo_mean: f64,
    pub moslqo_min: f64,
    pub moslqo_max: f64,
    pub moslqo_stddev: f64,
    pub vnsim_mean: f64,

    // 新增 DNSMOS 统计（仅计算成功评估的段）
    pub sig_mean: Option<f64>,
    pub bak_mean: Option<f64>,
    pub ovrl_mean: Option<f64>,
}
```

### 4.3 控制台输出示例

```
【整体统计】
  分段数: 3
  MOS-LQO: 均值=4.12, 最小=3.85, 最大=4.35, 标准差=0.21
  VNSIM 均值: 0.8923
  DNSMOS: SIG=4.35, BAK=4.80, OVRL=4.20

各段详细评分
  第 1/3 段 (0.00s - 10.00s)
    MOS-LQO: 4.12  VNSIM: 0.8923
    DNSMOS: SIG=4.35  BAK=4.80  OVRL=4.20
```

### 4.4 JSON 输出示例

```json
{
  "config": { ... },
  "alignment": { ... },
  "overall": {
    "segment_count": 3,
    "moslqo_mean": 4.12,
    "moslqo_min": 3.85,
    "moslqo_max": 4.35,
    "moslqo_stddev": 0.21,
    "vnsim_mean": 0.8923,
    "sig_mean": 4.35,
    "bak_mean": 4.80,
    "ovrl_mean": 4.20
  },
  "segments": [
    {
      "segment_index": 0,
      "start_time_s": 0.0,
      "end_time_s": 10.0,
      "quality": { ... },
      "anomaly": { ... },
      "sig": 4.35,
      "bak": 4.80,
      "ovrl": 4.20
    }
  ]
}
```

## 5. HTML 报告设计

### 5.1 图表布局

质量评估趋势区域改为两列并排：

```
┌──────────────────────────────────────────────────────┐
│                  质量评估趋势                          │
├────────────────────────┬─────────────────────────────┤
│   MOS-LQO 趋势         │   DNSMOS 趋势               │
│   ── MOS-LQO           │   ── SIG（人声信号）         │
│   Y: 1-5               │   ── BAK（背景噪声）         │
│                        │   ── OVRL（整体综合）        │
│                        │   Y: 1-5                     │
└────────────────────────┴─────────────────────────────┘
```

### 5.2 数值卡片

```
┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐
│  MOS    │ │ VNSIM   │ │   SIG   │ │   BAK   │ │  OVRL   │ │  SNR    │
│  4.12   │ │ 0.8923  │ │  4.35   │ │  4.80   │ │  4.20   │ │  28dB   │
└─────────┘ └─────────┘ └─────────┘ └─────────┘ └─────────┘ └─────────┘
```

**颜色阈值**：
- MOS < 3.0：红色（质量差）
- SIG/BAK/OVRL < 3.0：红色（质量差）
- 与现有 MOS 卡片保持一致

**响应式布局**：6 卡片使用 `grid-template-columns: repeat(auto-fit, minmax(220px, 1fr))`，在宽屏显示为 2 行 x 3 列，在中等宽度显示为 3 行 x 2 列，布局合理。

### 5.3 折线图配置（Chart.js）

- **左侧图（MOS-LQO）**：保持现有配置不变
- **右侧图（DNSMOS）**：
  - 三条线用不同颜色区分：
    - SIG（人声信号）：`#e53e3e`（红色）
    - BAK（背景噪声）：`#38a169`（绿色）
    - OVRL（整体综合）：`#805ad5`（紫色）
  - Y 轴范围：`min: 1, max: 5`
  - X 轴标签复用 `segLabels`（段号）
- **单段隐藏逻辑**：与 MOS-LQO 保持一致，当只有一段时不显示趋势图，只显示数值卡片

### 5.4 指标说明扩展

在现有指标说明中增加 DNSMOS 三个指标的描述：

```html
<dt><span class="tag">SIG</span>人声信号分</dt>
<dd>DNSMOS 的人声信号质量评分，符合 ITU-T P.835 标准。评估人声是否清晰、自然。如果降噪算法用力过猛导致发言人声音变小或变哑，这个分数就会很低。值域 1.0-5.0，分数越高越好。</dd>

<dt><span class="tag">BAK</span>背景噪声分</dt>
<dd>DNSMOS 的背景噪声质量评分，符合 ITU-T P.835 标准。评估背景杂音的消除程度。如果会议室里键盘敲击声、空调风噪被去得很干净，这个分数就会很高。值域 1.0-5.0，分数越高越好。</dd>

<dt><span class="tag">OVRL</span>整体综合分</dt>
<dd>DNSMOS 的整体质量评分，符合 ITU-T P.835 标准。结合人声和噪声后的整体听感主观评分。值域 1.0-5.0，分数越高越好。</dd>
```

### 5.5 表格扩展

各段详细评分表格增加 SIG、BAK、OVRL 列：

```html
<th>段</th><th>时间范围</th><th>MOS-LQO</th><th>VNSIM</th><th>SIG</th><th>BAK</th><th>OVRL</th>...
```

## 6. 错误处理

- **模型加载失败**：返回 `Err` 并打印错误信息，程序退出
- **音频预处理失败**（如重采样）：返回 `Err`，该段标记为评估失败
- **ONNX 推理失败**：返回 `Err`，该段标记为评估失败
- **模型输出异常**（如 NaN 或超出合理范围）：裁剪到 [1.0, 5.0] 范围

## 7. 测试计划

1. **单元测试**：
   - 多项式校准函数测试（对照 Python 源码输出）
   - 滑窗切分边界条件测试

2. **集成测试**：
   - 对已知质量的音频样例进行评估，验证 SIG/BAK/OVRL 数值合理
   - 对比 Rust 实现与 Python 源码的输出差异

3. **报告测试**：
   - 验证 HTML 报告中 DNSMOS 图表正确渲染
   - 验证 JSON 输出包含新增字段

## 8. 待定事项

- [ ] 下载 `sig_bak_ovr.onnx` 模型文件到 `bin/model/` 目录，并确认微软模型许可证允许嵌入分发
- [ ] 确认 `ort` static feature 编译通过（可能需要额外编译参数）
- [ ] 测试在目标平台上的二进制体积增量（静态链接 onnxruntime）
- [ ] 对极短音频（< 3s）的 DNSMOS 评估添加警告提示

## 9. 已知限制

- **重采样方法**：使用现有线性插值重采样，与 Python 源码的 `librosa.resample`（高质量重采样）存在差异，可能对评分产生可测量偏差。建议在测试计划中增加对比验证。
- **音频模式行为**：当用户使用 `--audio` 标志（48kHz 音频模式）时，DNSMOS 仍将对 16kHz 降采样后的音频进行评分。对于音乐等非语音内容，DNSMOS 评分可能无意义。报告中可考虑显示提示信息。
- **短音频质量**：��� < 3s 的极短音频，指数翻倍拼接会导致大量重复，DNSMOS 评分可能不可靠。