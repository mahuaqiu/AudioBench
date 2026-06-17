# DNSMOS 集成实施计划

**关联设计文档**: `docs/superpowers/specs/2026-06-17-dnsmos-integration-design.md`

## 前置条件

- [x] 设计文档已通过 3 轮审查并获批准
- [ ] 下载 `sig_bak_ovr.onnx` 模型文件到 `bin/model/`（需手动操作）

## 实施步骤

### 步骤 1：下载 ONNX 模型文件

**文件**: `bin/model/sig_bak_ovr.onnx`

从微软 DNS-Challenge 仓库获取 `sig_bak_ovr.onnx`（~1.1MB），复制到 `bin/model/` 目录。

**验证**: 文件存在且大小约 1.1MB

---

### 步骤 2：添加 Cargo 依赖

**文件**: `Cargo.toml`

新增依赖：
```toml
ort = { version = "2", default-features = false, features = ["static"] }
ndarray = "0.16"
```

**注意**: `ort` 的 `static` feature 会静态链接 onnxruntime，确保单 EXE 运行。首次编译可能较慢（构建 onnxruntime 静态库）。

**验证**: `cargo check` 通过

---

### 步骤 3：修改 build.rs — 增加 DNSMOS 模型 hash

**文件**: `build.rs`

在现有模型处理逻辑之后，增加 `sig_bak_ovr.onnx` 的 hash 计算：
```rust
// 处理 DNSMOS 模型
let dnsmos_model_path = model_dir.join("sig_bak_ovr.onnx");
if !dnsmos_model_path.exists() {
    println!("cargo:warning=DNSMOS 模型文件不存在: {:?}，创建占位文件", dnsmos_model_path);
    fs::write(&dnsmos_model_path, b"PLACEHOLDER_DNSMOS_MODEL").expect("无法创建占位文件");
}
let dnsmos_model_data = fs::read(&dnsmos_model_path).expect("无法读取 DNSMOS 模型文件");
let dnsmos_model_hash = format!("{:016x}", dnsmos_model_data.len());
println!("cargo:rustc-env=DNSMOS_MODEL_HASH={}", dnsmos_model_hash);
println!("cargo:warning=DNSMOS 模型: {} bytes", dnsmos_model_data.len());
```

**验证**: `cargo build` 输出 DNSMOS 模型信息

---

### 步骤 4：创建 dnsmos.rs 核心模块

**文件**: `src/dnsmos.rs`（新建）

实现内容：
1. `include_bytes!` 嵌入 ONNX 模型
2. 多项式校准系数常量 + `polyfit()` 函数
3. `DnsMosResult` 结构体（sig, bak, ovrl）
4. `DnsMosEvaluator` 结构体及方法：
   - `new(model_bytes)` — 从字节创建 ONNX Session
   - `evaluate(&self, samples: &[f64], sample_rate: u32)` — 核心评估逻辑
5. 内部辅助函数：
   - `resample_to_16kHz()` — 重采样（复用 audio_io 的线性插值或内联实现）
   - 滑窗切分逻辑（长音频 ≥9.01s → 滑动窗口；短音频 <9.01s → 零填充）
   - 逐窗推理 + 取均值
   - 结果裁剪到 [1.0, 5.0]

关键实现细节：
- ONNX 输入: `ndarray::Array2<f32>` 形状 [1, 144160]
- ONNX 输出: 形状 [1, 3]，依次 [SIG_raw, BAK_raw, OVRL_raw]
- 滑窗参数: 窗口 144160 点 (9.01s)，步进 16000 点 (1s)
- 零填充: 不足 144160 点时末尾补 0.0

**验证**: 编写单元测试
- `polyfit()` 对照 Python 系数验证
- 滑窗切分边界条件

---

### 步骤 5：修改 report.rs — 增加 DNSMOS 字段

**文件**: `src/report.rs`

5a. `SegmentResult` 增加字段：
```rust
pub sig: Option<f64>,   // 人声信号分
pub bak: Option<f64>,   // 背景噪声分
pub ovrl: Option<f64>,  // 整体综合分
```

5b. `OverallStats` 增加字段：
```rust
pub sig_mean: Option<f64>,
pub bak_mean: Option<f64>,
pub ovrl_mean: Option<f64>,
```

5c. `compute_overall_stats()` 增加 DNSMOS 均值计算逻辑

5d. `print_console_report()` 增加 DNSMOS 输出行：
- 整体统计区域增加: `DNSMOS: SIG=4.35, BAK=4.80, OVRL=4.20`
- 各段详细评分增加: `DNSMOS: SIG=4.35  BAK=4.80  OVRL=4.20`

**验证**: JSON 序列化输出包含新字段；控制台输出包含 DNSMOS 行

---

### 步骤 6：修改 main.rs — 集成 DNSMOS 评估

**文件**: `src/main.rs`

6a. 增加 `mod dnsmos;` 声明

6b. 在 `main()` 函数开头，加载 DNSMOS 模型：
```rust
const DNSMOS_MODEL: &[u8] = include_bytes!("../bin/model/sig_bak_ovr.onnx");
let dnsmos_evaluator = dnsmos::DnsMosEvaluator::new(DNSMOS_MODEL)
    .expect("无法加载 DNSMOS 模型");
println!("[*] DNSMOS 模型加载成功");
```

6c. 在每段 ViSQOL 评估之后，调用 DNSMOS 评估：
```rust
// DNSMOS 评估（仅对录制音频，无参考）
let dnsmos_result = dnsmos_evaluator.evaluate(&seg_degraded, ref_audio.sample_rate);
let (sig, bak, ovrl) = match dnsmos_result {
    Ok(r) => (Some(r.sig), Some(r.bak), Some(r.ovrl)),
    Err(e) => {
        println!("[!] DNSMOS 评估失败: {}", e);
        (None, None, None)
    }
};
```

6d. 在 `segment_results.push()` 时填充 sig/bak/ovrl 字段

**验证**: 编译通过，运行时不崩溃

---

### 步骤 7：修改 html_report.rs — 增加 DNSMOS 图表和卡片

**文件**: `src/html_report.rs`

7a. 数值卡片区域增加 3 个 DNSMOS 卡片（SIG、BAK、OVRL），颜色阈值 < 3.0 为红色

7b. 图表布局调整：
- 原"MOS-LQO 分段趋势"改为与 "DNSMOS 趋势"并排（chart-row 两列布局）
- DNSMOS 折线图包含三条线：SIG(红)、BAK(绿)、OVRL(紫)
- Y 轴范围 1-5

7c. 各段详细评分表格增加 SIG、BAK、OVRL 列

7d. 指标说明区域增加 DNSMOS 三指标描述

7e. Chart.js 初始化代码增加 DNSMOS 图表渲染

**验证**: HTML 报告渲染正确，图表和卡片均显示

---

### 步骤 8：集成测试与验证

8a. 编译完整项目：`cargo build --release`

8b. 运行评估：`audio_bench --reference ref.wav --recorded rec.wav --html report.html`

8c. 检查项：
- [ ] 控制台输出包含 DNSMOS 行
- [ ] JSON 报告包含 sig/bak/ovrl 字段
- [ ] HTML 报告 DNSMOS 卡片正确显示
- [ ] HTML 报告 DNSMOS 折线图正确渲染
- [ ] SIG/BAK/OVRL 数值在 1.0-5.0 范围内
- [ ] 单段场景下 DNSMOS 卡片正常显示

---

## 风险与缓解

| 风险 | 缓解措施 |
|------|----------|
| `ort` static feature 编译失败 | 预先在开发环境试编译；备选方案用 `dynamic` + 打包 DLL |
| onnxruntime 静态链接增大二进制体积 | release profile 已启用 LTO + strip；接受体积增量 |
| DNSMOS 评分与 Python 结果偏差 | 归因于重采样差异；文档中说明已知限制 |
| 短音频零填充对 OVRL 的影响 | 设计文档已确认零填充策略，属于已知限制 |
