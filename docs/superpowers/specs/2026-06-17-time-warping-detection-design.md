# 时轴漂移检测重构设计

**日期**：2026-06-17
**状态**：待评审
**关联**：AudioBench 异常检测模块

## 1. 背景与动机

### 1.1 现状问题

AudioBench 当前有 4 个异常检测维度。通过 4 组对照测试（A 基线 / B 中间裁剪 / C 插静音 / D 插入重复谱）发现：

1. **维度 2「时轴漂移」当前事实上是死的**：基于「段总时长 vs 参考时长」的检测，因 `seg_degraded` 被 resize 补齐到 `ref_len` 再由 `find_actual_audio_end` 切除补零，导致 `seg_dur ≡ ref_dur`，所有测试场景漂移恒为 0ms。
2. **维度 3「内容截断」基准口径错位**：`detect_truncation` 用「参考全长（含尾静音）」对比「录制去尾静音长度」，两侧口径不一致。
3. **B（中间裁剪）和 D（插入重复内容）四个维度全部漏报**：总长未变 → 漂移/截断失效；无静音 → 中断不触发；ViSQOL patch 相似度被 DTW 吸收 → 频谱不报（D 场景 patch 最低相似度 0.928）。

### 1.2 关键技术发现

通过审阅 ViSQOL 源码（`/Users/ma/Downloads/visqol-master/src/comparison_patches_selector.cc`）确认：

- ViSQOL **内部对每个 patch 做 DTW 时间规整**（`FindMostOptimalDegPatch`，第 39-115 行）
- `degPatchStartTime` 是 DTW 找到的**最优对齐位置**，不是录制信号的真实物理时间
- DTW 会主动吸收时间轴错位（这是 ViSQOL 给出公平质量分的设计目的），导致 D 场景 patch 相似度高达 0.928

**结论**：依赖 ViSQOL patch 时间戳检测时间轴错位不可行，必须自建独立于 ViSQOL 的检测。

### 1.3 目标

用一个机制重构维度 2「时轴漂移」，统一检测 4 种时间轴异常场景，子类型用标签区分。

## 2. 覆盖场景

| 子类型 | 成因 | offset 序列形态 | 方向 | 测试场景 |
|---|---|---|---|---|
| 裁剪 Cut | 编辑删除 | 突变阶梯 | 前移（offset 减小） | B |
| 插入 Insertion | 编辑插入 | 突变阶梯 | 后移（offset 增大） | D |
| 拉伸 Stretch | WSOLA / 抖动缓冲等包 | 线性斜坡 | 后移（offset 渐增） | 真实拉伸 |
| 压缩 Compress | 播放加速 / 丢包压缩 | 线性斜坡 | 前移（offset 渐减） | 真实压缩 |

不误报：A（基线）、C（插静音，由中断检测负责）。

## 3. 核心算法

### 3.1 阶段一：offset 序列提取

对每段，在整段 `seg_degraded[0..ref_len]`（与现有中断/频谱检测范围一致）上滑窗算局部偏移：

**参数**：
- 窗长 `window_ms = 200ms`（3200 采样点 @16kHz，含完整音节，互相关峰尖锐）
- 步进 `hop_ms = 100ms`（50% 重叠，时间分辨率 100ms）
- 搜索半径 `search_radius_ms = ±300ms`（覆盖一次裁剪/插入位移量）

**单窗处理**：
1. 取参考窗 `ref[t..t+window]`
2. 在录制端 `[t-R, t+R]` 范围内，做 FFT 归一化互相关
3. 互相关峰值位置 = 该窗 `offset[i]`（单位 ms，正值=录制滞后/后移）

**静音窗处理**：
- 每窗先算 RMS，低于 `silence_threshold`（复用 `DropoutDetectorConfig.silence_threshold = 0.005`）的窗标记无效
- 无效窗不参与 offset 计算和形态分析，序列里标记为 `None`

**输出**：`Vec<Option<f64>>`，长度 = (段长 - 窗长) / 步进 + 1，5s 段约 48 个点。

**基础设施复用**：直接复用 `alignment.rs::raw_cross_correlation`（FFT 互相关）和 `prefix_square_sums`（O(1) 窗能量查询）。

### 3.2 阶段二：形态分析与分类

对 offset 序列去基线（减去首个有效点的值，归一化为「相对首个窗的偏移变化」）后：

**步骤 1 - 突变检测（裁剪 / 插入）**：
- 计算相邻有效点差 `Δoffset[i]`
- 若 `|Δoffset[i]| > jump_threshold_ms`（默认 80ms，约 1 个步进），标记为突变点
- 突变幅度 = 突变点前后 offset 均值之差
- 后续 offset 整体偏移 = 事件幅度

**步骤 2 - 斜坡检测（拉伸 / 压缩）**：
- 对全部有效 offset 点做线性回归（最小二乘）
- 若 `|斜率| > slope_threshold`（默认每秒漂移 > 30ms，即斜率 > 0.3）**且** `R² > 0.7`（线性度好），判为渐变
- 渐变幅度 = 斜率 × 有效时长

**步骤 3 - 方向与子类型判定**：
- 突变：幅度 > 0 → Insertion（后移）；< 0 → Cut（前移）
- 斜坡：幅度 > 0 → Stretch（后移）；< 0 → Compress（前移）

**步骤 4 - 幅度门槛**：
- 最终事件 `drift_ms` 绝对值需 ≥ `min_drift_ms`（默认 60ms，与 `min_duration_ms` 对齐），否则不报

**步骤 5 - 突变与斜坡的优先级**：
- 同一段若同时满足突变和斜坡判据，优先报突变（裁剪/插入是更明确的异常）
- 实测中两者极少同时满足强判据

### 3.3 与中断检测的关系

C 场景（插静音）由中断检测负责。漂移检测遇静音窗跳过，静音段不产生误判 offset，避免重复报警。

## 4. 数据结构

### 4.1 新增/重构（`metrics.rs`）

```rust
/// 时轴漂移子类型
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq)]
pub enum WarpingType {
    /// 裁剪：内容缺失，前移
    Cut,
    /// 插入：内容重复/新增，后移
    Insertion,
    /// 拉伸：匀速变慢，后移
    Stretch,
    /// 压缩：匀速变快，前移
    Compress,
}

/// 时轴漂移事件（重构）
#[derive(Debug, Clone, serde::Serialize)]
pub struct WarpingEvent {
    /// 涉及的段索引
    pub segment_index: usize,
    /// 漂移起始时间（秒，基于参考时间轴）
    pub start_time_s: f64,
    /// 漂移结束时间（秒，基于参考时间轴）
    pub end_time_s: f64,
    /// 总漂移幅度（ms，带符号：+后移，-前移）
    pub drift_ms: f64,
    /// 子类型标签
    pub drift_type: WarpingType,
}
```

### 4.2 删除

- `metrics.rs::detect_warpings`（基于总长度的旧实现）
- `metrics.rs::WarpingThreshold`（旧的双重阈值，被 `WarpingConfig` 取代）

### 4.3 保留不变

- `AudioAnomalyReport` 字段名 `warpings: Vec<WarpingEvent>`、`warping_duration_ms: f64` 不变
- `warping_duration_ms` 计算改为各事件 `drift_ms.abs()` 求和

## 5. 模块组织

新建 `src/time_warping.rs`，独立于 `metrics.rs`：

```rust
/// 漂移检测配置
pub struct WarpingConfig {
    pub window_ms: f64,           // 默认 200
    pub hop_ms: f64,              // 默认 100
    pub search_radius_ms: f64,    // 默认 300
    pub silence_threshold: f64,   // 默认 0.005（复用中断检测）
    pub jump_threshold_ms: f64,   // 默认 80
    pub slope_threshold: f64,     // 默认 0.3（ms 漂移/秒）
    pub min_drift_ms: f64,        // 默认 60
    pub min_r_squared: f64,       // 默认 0.7
}

impl WarpingConfig {
    pub fn for_sample_rate(sr: u32) -> Self { /* ... */ }
}

/// 阶段一：计算 offset 序列（None 表示静音窗）
pub fn compute_offset_series(
    reference: &[f64],
    degraded: &[f64],
    sample_rate: u32,
    config: &WarpingConfig,
) -> Vec<Option<f64>>

/// 阶段二：从 offset 序列检测漂移事件
pub fn detect_warpings_from_offsets(
    offsets: &[Option<f64>],
    sample_rate: u32,
    hop_ms: f64,
    segment_index: usize,
    config: &WarpingConfig,
) -> Vec<WarpingEvent>
```

**职责边界**：
- `time_warping.rs` 只做漂移检测，不依赖 `metrics.rs`（避免循环）
- `WarpingEvent` / `WarpingType` 定义在 `metrics.rs`（因为 `AudioAnomalyReport` 要引用），`time_warping.rs` 通过 `crate::metrics::` 引用

`main.rs` 改动：
- `mod time_warping;`
- 删除旧的 `metrics::detect_warpings` 调用，改为对每段调用 `time_warping::compute_offset_series` + `detect_warpings_from_offsets`
- 合并结果到 `seg_result.anomaly.warpings`

## 6. 报告呈现

### 6.1 Console 报告（`report.rs`）

异常检测段从：
```
时轴漂移: 180ms (1次)
```
改为：
```
时轴漂移: 180ms (裁剪, 1次)
```

### 6.2 HTML 报告（`html_report.rs`）

表格「异常」列从：
```
漂移180ms
```
改为：
```
漂移180ms(裁剪) / 漂移250ms(插入) / 漂移300ms(拉伸) / 漂移200ms(压缩)
```

HTML 卡片「时轴漂移」总时长数值不变（所有事件 `drift_ms.abs()` 求和）。

指标说明区「时轴漂移」条目补充子类型说明：
> 检测录制时间轴相对参考的局部错位。子类型：裁剪（内容缺失，前移）、插入（内容重复/新增，后移）、拉伸（匀速变慢）、压缩（匀速变快）。

## 7. 诊断日志

复用现有 `[DIAG]` 机制，在 `time_warping.rs` 关键节点加日志：
- offset 序列提取后：打印每段的有效点数、offset 范围（min/max）
- 形态分析后：打印突变点位置/幅度、线性回归斜率/R²、最终事件

便于后续调参与阈值验证。

## 8. 已知边界与权衡

### 8.1 D 场景（插入重复内容）的固有局限

若插入的内容与原内容**频谱完全一致**（如复制粘贴同一段），插入点处 offset 会有一次突变，随后保持新基线——**理论上能被突变检测抓到**。但实测中可能出现：
- 插入内容与前后衔接平滑，offset 仅轻微抖动而非突变
- 此情况需依赖实测调参，若仍漏检，后续可考虑加入「patch 时间戳与自算 offset 的交叉验证」作为补充

### 8.2 静音窗处理

整段范围内静音窗（如 C 场景的插入静音）互相关峰不稳，会标记为 `None` 跳过。这保证 C 场景不误报漂移，由中断检测负责。

### 8.3 搜索半径与计算成本

搜索半径 ±300ms @16kHz = 9600 采样点，FFT 点数取窗长+搜索范围 = ~12800，向上取 2^14 = 16384。每窗 FFT 约 50μs，5s 段 48 窗约 2.4ms，可忽略。

### 8.4 维度 3「内容截断」基准口径 bug

本次重构**仅修维度 2**。维度 3 的口径错位 bug（参考全长 vs 录制去尾长度）属于独立问题，不在本次范围。但需注意：本次重构后，尾部内容缺失场景（如果存在）将由维度 3（修正口径后）或维度 2（裁剪子类型）覆盖，两者不冲突。

## 9. 测试策略

### 9.1 单元测试（`time_warping.rs`）

由于无真实音频文件，构造合成信号测试：
- `test_offset_series_flat`：未错位信号，offset 恒 0
- `test_offset_series_cut`：合成中间裁剪信号，验证 offset 突变
- `test_offset_series_stretch`：合成线性拉伸信号，验证斜坡检测
- `test_silence_window_skipped`：含静音窗，验证 `None` 标记
- `test_below_threshold_ignored`：小幅漂移 < 60ms 不报

### 9.2 集成验证

用户在 Windows 环境用 A/B/C/D 四组音频验证：
- A：无漂移（不误报）
- B：报 Cut，幅度接近实际裁剪量
- C：不报漂移（由中断负责）
- D：报 Insertion（或验证 8.1 的局限边界）

## 10. 不做的事（YAGNI）

- 不引入 DTW 库（自建滑窗互相关足够）
- 不改 ViSQOL 调用（保持单 EXE 无外部依赖）
- 不重构中断/频谱/截断检测（仅维度 2）
- 不做多段间漂移关联（每段独立检测）
