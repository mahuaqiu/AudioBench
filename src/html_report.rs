//! HTML 报告生成模块
//! 生成自包含的 HTML 报告页面，内嵌 Chart.js 图表

use crate::report::EvaluationReport;

/// 生成 HTML 报告
pub fn generate_html_report(report: &EvaluationReport) -> String {
    // 将所有数据序列化为 JSON
    let json_data = serde_json::to_string(report).unwrap_or_else(|_| "{}".to_string());

    // 准备各图表数据
    let seg_labels: Vec<String> = report.segments.iter()
        .enumerate()
        .map(|(i, _)| format!("第{}段", i + 1))
        .collect();
    let mos_values: Vec<f64> = report.segments.iter().map(|s| s.quality.moslqo).collect();
    let vnsim_values: Vec<f64> = report.segments.iter().map(|s| s.quality.vnsim).collect();

    // fVNSIM 数据
    let fvnsim_data: Vec<Vec<f64>> = report.segments.iter()
        .map(|seg| seg.quality.fvnsim.iter().take(32).cloned().collect())
        .collect();

    // 频段���量比
    let energy_data: Vec<Vec<f64>> = report.segments.iter()
        .map(|seg| seg.band_energy_ratios.iter().take(32).cloned().collect())
        .collect();

    // Patch 相似度
    let patch_data: Vec<Vec<f64>> = report.segments.iter()
        .map(|seg| seg.quality.patch_sims.iter().map(|p| p.similarity).collect())
        .collect();

    // 表格行
    let table_rows = generate_table_rows(report);

    // JSON 序列化各个数据数组
    let seg_labels_json = serde_json::to_string(&seg_labels).unwrap_or("[]".to_string());
    let mos_values_json = serde_json::to_string(&mos_values).unwrap_or("[]".to_string());
    let vnsim_values_json = serde_json::to_string(&vnsim_values).unwrap_or("[]".to_string());
    let fvnsim_json = serde_json::to_string(&fvnsim_data).unwrap_or("[]".to_string());
    let energy_json = serde_json::to_string(&energy_data).unwrap_or("[]".to_string());
    let patch_json = serde_json::to_string(&patch_data).unwrap_or("[]".to_string());

    // 卡顿统计
    let dropout_count: usize = report.segments.iter().map(|s| s.dropouts.count).sum();
    let dropout_dur: f64 = report.segments.iter().map(|s| s.dropouts.total_duration_ms).sum();

    // 模式名称
    let mode_name = if report.config.target_sample_rate == 16000 { "语音模式" } else { "音频模式" };

    // 把 JSON 数据转换为 JS 字符串字面量（单引号包裹，内部单引号转义）
    fn to_js_str(s: &str) -> String {
        format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>音频质量评估报告</title>
<script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.7/dist/chart.umd.min.js"></script>
<style>
  :root {{ --bg:#f5f7fa; --card:#fff; --border:#e2e8f0; --text:#1a202c; --text2:#4a5568; --text3:#718096; --accent:#3182ce; --green:#38a169; --yellow:#d69e2e; --red:#e53e3e; }}
  * {{ margin:0; padding:0; box-sizing:border-box; }}
  body {{ font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif; background:var(--bg); color:var(--text); line-height:1.6; padding:24px; }}
  .container {{ max-width:1100px; margin:0 auto; }}
  h1 {{ font-size:24px; font-weight:700; margin-bottom:4px; }}
  .subtitle {{ color:var(--text3); font-size:14px; margin-bottom:24px; }}
  .cards {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(220px,1fr)); gap:16px; margin-bottom:24px; }}
  .card {{ background:var(--card); border:1px solid var(--border); border-radius:8px; padding:20px; }}
  .card-label {{ font-size:13px; color:var(--text3); margin-bottom:4px; }}
  .card-value {{ font-size:32px; font-weight:700; }}
  .card-value.good {{ color:var(--green); }}
  .card-value.warn {{ color:var(--yellow); }}
  .card-value.bad {{ color:var(--red); }}
  .card-hint {{ font-size:12px; color:var(--text3); margin-top:6px; }}
  .section {{ background:var(--card); border:1px solid var(--border); border-radius:8px; padding:20px; margin-bottom:24px; }}
  .section-title {{ font-size:16px; font-weight:600; margin-bottom:16px; padding-bottom:8px; border-bottom:1px solid var(--border); }}
  .chart-full {{ position:relative; height:280px; }}
  @media(max-width:768px){{}}
  table{{width:100%;border-collapse:collapse;font-size:13px}}
  th,td{{text-align:left;padding:8px 12px;border-bottom:1px solid var(--border)}}
  th{{font-weight:600;color:var(--text2);background:#f7fafc}}
  tr:hover td{{background:#f7fafc}}
  .info-grid{{display:grid;grid-template-columns:1fr 1fr;gap:8px 24px;font-size:13px}}
  .info-grid .label{{color:var(--text3)}}.info-grid .value{{color:var(--text);font-weight:500}}
  .glossary{{font-size:13px}}
  .glossary dt{{font-weight:600;color:var(--text);margin-top:10px}}
  .glossary dd{{color:var(--text2);margin-left:0;margin-bottom:4px}}
  .tag{{display:inline-block;background:#ebf8ff;color:var(--accent);padding:2px 8px;border-radius:4px;font-size:12px;margin-right:6px}}
</style>
</head>
<body>
<div class="container">
<h1>音频质量评估报告</h1>
<p class="subtitle">生成时间：{timestamp}</p>

<div class="section">
<div class="section-title">基本信息</div>
<div class="info-grid">
<div><span class="label">参考音频：</span><span class="value">{ref_path}</span></div>
<div><span class="label">录制音频：</span><span class="value">{deg_path}</span></div>
<div><span class="label">参考时长：</span><span class="value">{ref_dur:.2}s</span></div>
<div><span class="label">录制时长：</span><span class="value">{deg_dur:.2}s</span></div>
<div><span class="label">采样率：</span><span class="value">{sample_rate}Hz（{mode}）</span></div>
<div><span class="label">对齐延迟：</span><span class="value">{delay_ms:.1}ms</span></div>
<div><span class="label">对齐置信度：</span><span class="value">{conf:.1}%</span></div>
<div><span class="label">分��数量：</span><span class="value">{seg_count}</span></div>
</div>
</div>

<div class="cards">
<div class="card">
<div class="card-label">MOS-LQO 均值</div>
<div class="card-value good">{mos_mean:.2}</div>
<div class="card-hint">ViSQOL 预测质量分（1-5），值越高越好<br>范围：{mos_min:.2}~{mos_max:.2}</div>
</div>
<div class="card">
<div class="card-label">VNSIM 均值</div>
<div class="card-value">{vnsim_mean:.4}</div>
<div class="card-hint">全局神经图相似度（0-1），1=完全相同</div>
</div>
<div class="card">
<div class="card-label">卡顿检测</div>
<div class="card-value">{dropout_count}次</div>
<div class="card-hint">信号中断/丢包事件<br>总时长：{dropout_dur:.0}ms</div>
</div>
</div>

<div class="section"><div class="section-title">MOS-LQO 分段趋势</div>
<div class="chart-full"><canvas id="chartMos"></canvas></div>
</div>

<div class="section"><div class="section-title">VNSIM 分段趋势</div>
<div class="chart-full"><canvas id="chartVnsim"></canvas></div>
</div>

<div class="section"><div class="section-title">fVNSIM 频段相似度（多段对比）</div>
<div class="chart-full"><canvas id="chartFvnsim"></canvas></div>
</div>

<div class="section"><div class="section-title">频段能量比 fvdegenergy（多段对比）</div>
<div class="chart-full"><canvas id="chartEnergy"></canvas></div>
</div>

<div class="section"><div class="section-title">Patch 时间片段相似度</div>
<div class="chart-full"><canvas id="chartPatch"></canvas></div>
</div>

<div class="section"><div class="section-title">各段详细评分</div>
<table><thead><tr><th>段</th><th>时间范围</th><th>MOS-LQO</th><th>VNSIM</th><th>低频相似度</th><th>高频相似度</th><th>能量比均值</th><th>卡顿</th></tr></thead>
<tbody>{table_rows}</tbody>
</table>
</div>

<div class="section"><div class="section-title">指标说明</div>
<dl class="glossary">
<dt><span class="tag">MOS-LQO</span>ViSQOL预测质量分</dt>
<dd>ViSQOL通过SVM模型将频域相似度映射为1-5的预测分。值域1-5，越高表示预测质量越好。建议用于同场景相对比较。</dd>
<dt><span class="tag">VNSIM</span>全局神经图相似度</dt>
<dd>基于Gammatone听觉滤波器组提取的频谱图，计算参考与录制之间的NSIM。值域0-1，1表示频谱完全一致。</dd>
<dt><span class="tag">fVNSIM</span>各频段相似度</dt>
<dd>VNSIM在每个Gammatone频带上的分解值。低频带(1-10)对应50-500Hz，中频带(11-20)对应500-2000Hz，高频带(21-32)对应2000Hz以上。</dd>
<dt><span class="tag">fvdegenergy</span>频段能量比</dt>
<dd>每个频带中录制信号相对于参考信号的能量变化比例。值>1表示能量增加(如添加噪声)，值<1表示能量减少(如高频衰减)。</dd>
<dt><span class="tag">Patch相似度</span>时间片段相似度</dt>
<dd>ViSQOL将音频按约0.6秒切分为多个Patch，分别计算每个Patch的NSIM。多段叠加显示便于定位问题时段。</dd>
<dt><span class="tag">卡顿检测</span>信号中断事件</dt>
<dd>检测录制音频中参考有声但录制无声的片段（丢包、缓冲区欠载等）。</dd>
</dl>
</div>

</div>

<script>
var REPORT = JSON.parse({report_json});
var segLabels = JSON.parse({seg_labels_json});
var mosValues = JSON.parse({mos_values_json});
var vnsimValues = JSON.parse({vnsim_values_json});
var fvnsimData = JSON.parse({fvnsim_json});
var energyData = JSON.parse({energy_json});
var patchData = JSON.parse({patch_json});

var segColors = ['#3182ce','#e53e3e','#38a169','#d69e2e','#805ad5','#dd6b20','#319795','#b83280'];

// MOS-LQO 分段趋势
new Chart(document.getElementById('chartMos'),{{
  type:'line',
  data:{{labels:segLabels,datasets:[{{label:'MOS-LQO',data:mosValues,borderColor:'#3182ce',backgroundColor:'rgba(49,130,206,0.1)',fill:true,tension:0.3,pointRadius:5}}]}},
  options:{{responsive:true,maintainAspectRatio:false,scales:{{y:{{min:0,max:5,title:{{display:true,text:'MOS-LQO'}}}}}},plugins:{{title:{{display:true,text:'MOS-LQO分段趋势'}}}}}}
}});

// VNSIM 分段趋势
new Chart(document.getElementById('chartVnsim'),{{
  type:'line',
  data:{{labels:segLabels,datasets:[{{label:'VNSIM',data:vnsimValues,borderColor:'#38a169',backgroundColor:'rgba(56,161,105,0.1)',fill:true,tension:0.3,pointRadius:5}}]}},
  options:{{responsive:true,maintainAspectRatio:false,scales:{{y:{{min:0,max:1,title:{{display:true,text:'相似度'}}}}}},plugins:{{title:{{display:true,text:'VNSIM分段趋势'}}}}}}
}});

// fVNSIM 频段相似度
var fvnsimDatasets = fvnsimData.map(function(d,i){{
  return {{label:'第'+(i+1)+'段',data:d,borderColor:segColors[i%segColors.length],fill:false,tension:0.3}};
}});
var bandLabels = Array.from({{length:32}},function(_,i){{return 'B'+(i+1);}});
new Chart(document.getElementById('chartFvnsim'),{{
  type:'line',
  data:{{labels:bandLabels,datasets:fvnsimDatasets}},
  options:{{responsive:true,maintainAspectRatio:false,scales:{{y:{{min:0,max:1,title:{{display:true,text:'相似度'}}}}}},plugins:{{title:{{display:true,text:'fVNSIM频段相似度（多段对比）'}}}}}}
}});

// 频段能量比
var energyDatasets = energyData.map(function(d,i){{
  return {{label:'第'+(i+1)+'段',data:d,borderColor:segColors[i%segColors.length],fill:false,tension:0.3}};
}});
new Chart(document.getElementById('chartEnergy'),{{
  type:'line',
  data:{{labels:bandLabels,datasets:energyDatasets}},
  options:{{responsive:true,maintainAspectRatio:false,scales:{{y:{{title:{{display:true,text:'能量比'}}}}}},plugins:{{title:{{display:true,text:'频段能量比（多段对比）'}}}}}}
}});

// Patch 时间片段相似度
if(patchData.length > 0 && patchData[0].length > 0){{
  var allPatchLabels = patchData[0].map(function(_,i){{return 'Patch'+(i+1);}});
  var patchDatasets = patchData.map(function(d,i){{
    return {{label:'第'+(i+1)+'段',data:d,borderColor:segColors[i%segColors.length],fill:false,tension:0.3}};
  }});
  new Chart(document.getElementById('chartPatch'),{{
    type:'line',
    data:{{labels:allPatchLabels,datasets:patchDatasets}},
    options:{{responsive:true,maintainAspectRatio:false,scales:{{y:{{min:0,max:1,title:{{display:true,text:'相似度'}}}}}},plugins:{{title:{{display:true,text:'Patch时间片段相似度'}}}}}}
  }});
}}
</script>
</body>
</html>"#,
        timestamp = format_timestamp(),
        ref_path = report.config.reference_path,
        deg_path = report.config.recorded_path,
        ref_dur = report.reference_duration_s,
        deg_dur = report.recorded_duration_s,
        sample_rate = report.config.target_sample_rate,
        mode = mode_name,
        delay_ms = report.alignment.delay_ms,
        conf = report.alignment.confidence * 100.0,
        seg_count = report.segments.len(),
        mos_mean = report.overall.moslqo_mean,
        mos_min = report.overall.moslqo_min,
        mos_max = report.overall.moslqo_max,
        vnsim_mean = report.overall.vnsim_mean,
        dropout_count = dropout_count,
        dropout_dur = dropout_dur,
        // JS 字符串字面量注入
        report_json = to_js_str(&json_data),
        seg_labels_json = to_js_str(&seg_labels_json),
        mos_values_json = to_js_str(&mos_values_json),
        vnsim_values_json = to_js_str(&vnsim_values_json),
        fvnsim_json = to_js_str(&fvnsim_json),
        energy_json = to_js_str(&energy_json),
        patch_json = to_js_str(&patch_json),
        table_rows = table_rows,
    )
}


/// 生成可读的时��戳
fn format_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let hour = (time_of_day / 3600) as u32;
    let minute = ((time_of_day % 3600) / 60) as u32;
    let second = (time_of_day % 60) as u32;
    let (year, month, day) = days_to_ymd(days_since_epoch);
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", year, month, day, hour, minute, second)
}

/// 天数转日期
fn days_to_ymd(mut days: u64) -> (u32, u32, u32) {
    let mut year = 1970u32;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days < days_in_year { break; }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap_year(year);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u32;
    for &md in &month_days {
        if days < md { break; }
        days -= md;
        month += 1;
    }
    (year, month, (days + 1) as u32)
}

fn is_leap_year(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn generate_table_rows(report: &EvaluationReport) -> String {
    report.segments.iter().enumerate().map(|(i, seg)| {
        let (low, high) = compute_bands(&seg.quality.fvnsim);
        let energy_mean = if seg.band_energy_ratios.is_empty() {
            0.0
        } else {
            seg.band_energy_ratios.iter().sum::<f64>() / seg.band_energy_ratios.len() as f64
        };
        let dropout = if seg.dropouts.count > 0 {
            format!("{}次/{:.0}ms", seg.dropouts.count, seg.dropouts.total_duration_ms)
        } else { "无".to_string() };
        format!("<tr><td>第{}段</td><td>{:.2}s-{:.2}s</td><td>{:.2}</td><td>{:.4}</td><td>{:.4}</td><td>{:.4}</td><td>{:.4}</td><td>{}</td></tr>",
            i+1, seg.start_time_s, seg.end_time_s, seg.quality.moslqo, seg.quality.vnsim, low, high, energy_mean, dropout)
    }).collect()
}

fn compute_bands(fvnsim: &[f64]) -> (f64, f64) {
    if fvnsim.is_empty() { return (0.0, 0.0); }
    let n = fvnsim.len();
    let low_n = (n/3).max(1);
    let high_start = n * 2 / 3;
    let low = fvnsim.iter().take(low_n).sum::<f64>() / low_n as f64;
    let high = if high_start < n { fvnsim[high_start..].iter().sum::<f64>() / (n - high_start) as f64 } else { *fvnsim.last().unwrap_or(&0.0) };
    (low, high)
}
