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
    // fVNSIM 数据（取全部，不限制32个）
    let fvnsim_data: Vec<Vec<f64>> = report.segments.iter()
        .map(|seg| seg.quality.fvnsim.iter().cloned().collect())
        .collect();

    // 频段能量比（取全部，不限制32个）
    let energy_data: Vec<Vec<f64>> = report.segments.iter()
        .map(|seg| seg.band_energy_ratios.iter().cloned().collect())
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
    // centerFreqBands - 各频带中心频率（用于 tooltip 显示）
    let center_freq_bands: Vec<f64> = report.segments.first()
        .map(|s| s.quality.center_freq_bands.clone())
        .unwrap_or_default();
    let center_freq_json = serde_json::to_string(&center_freq_bands).unwrap_or("[]".to_string());
    // 波形数据 JSON
    let waveform_ref_json = serde_json::to_string(&report.waveform_ref).unwrap_or_else(|_| "{}".to_string());
    let waveform_deg_json = serde_json::to_string(&report.waveform_deg).unwrap_or_else(|_| "{}".to_string());


   // 异常检测统计
   let total_dropout: f64 = report.segments.iter().map(|s| s.anomaly.dropout_duration_ms.abs()).sum();
   let total_warping: f64 = report.segments.iter().map(|s| s.anomaly.warping_duration_ms.abs()).sum();
    let avg_spectral: f64 = if report.segments.is_empty() { 0.0 } else { report.segments.iter().map(|s| s.anomaly.spectral_artifacts_score).sum::<f64>() / report.segments.len() as f64 };

    // 时轴漂移子类型统计
    let mut warping_cut = 0usize;
    let mut warping_insertion = 0usize;
    let mut warping_stretch = 0usize;
    let mut warping_compress = 0usize;
    for seg in &report.segments {
        for w in &seg.anomaly.warpings {
            match w.drift_type {
                crate::metrics::WarpingType::Cut => warping_cut += 1,
                crate::metrics::WarpingType::Insertion => warping_insertion += 1,
                crate::metrics::WarpingType::Stretch => warping_stretch += 1,
                crate::metrics::WarpingType::Compress => warping_compress += 1,
            }
        }
    }
    let warping_types_str = {
        let mut parts = vec![];
        if warping_cut > 0 { parts.push(format!("裁剪{}次", warping_cut)); }
        if warping_insertion > 0 { parts.push(format!("插入{}次", warping_insertion)); }
        if warping_stretch > 0 { parts.push(format!("拉伸{}次", warping_stretch)); }
        if warping_compress > 0 { parts.push(format!("压缩{}次", warping_compress)); }
        if parts.is_empty() { "无".to_string() } else { parts.join(", ") }
    };

    // MOS 分是否低于 3 分
    let mos_is_low = report.overall.moslqo_mean < 3.0;
    let mos_class = if mos_is_low { "bad" } else { "good" };

    // 各异常类型独立判断颜色
    let dropout_class = if total_dropout > 0.0 { "bad" } else { "" };
    let warping_class = if total_warping > 0.0 { "bad" } else { "" };
    let spectral_class = if avg_spectral > 0.25 { "bad" } else { "" };

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
  .chart-row {{ display:grid; grid-template-columns:1fr 1fr; gap:20px; margin-bottom:24px; }}
  @media(max-width:900px){{ .chart-row {{ grid-template-columns:1fr; }} }}
  .chart-row {{ display:grid; grid-template-columns:1fr 1fr; gap:20px; margin-bottom:24px; }}
  @media(max-width:900px){{ .chart-row {{ grid-template-columns:1fr; }} }}
  @media(max-width:768px){{}}
  table{{width:100%;border-collapse:collapse;font-size:13px}}
  th,td{{text-align:left;padding:8px 12px;border-bottom:1px solid var(--border)}}
  th{{font-weight:600;color:var(--text2);background:#f7fafc}}
  tr:hover td{{background:#f7fafc}}
  .pagination {{ margin-top:16px; text-align:center }}
  .pagination button {{ margin:0 4px; padding:6px 12px; border:1px solid var(--border); background:var(--card); cursor:pointer; border-radius:4px }}
  .pagination button:hover {{ background:#e2e8f0 }}
  .pagination button.active {{ background:var(--accent); color:#fff; border-color:var(--accent) }}
  .pagination button:disabled {{ opacity:0.5; cursor:not-allowed }}
  .info-grid{{display:grid;grid-template-columns:1fr 1fr;gap:8px 24px;font-size:13px}}
  .info-grid .label{{color:var(--text3)}}.info-grid .value{{color:var(--text);font-weight:500}}
  .glossary{{font-size:13px}}
  .glossary dt{{font-weight:600;color:var(--text);margin-top:10px}}
  .glossary dd{{color:var(--text2);margin-left:0;margin-bottom:4px}}
  .tag{{display:inline-block;background:#ebf8ff;color:var(--accent);padding:2px 8px;border-radius:4px;font-size:12px;margin-right:6px}}
  .waveform-section {{ background:var(--card); border:1px solid var(--border); border-radius:8px; padding:20px; margin-bottom:24px; }}
  .waveform-title {{ font-size:16px; font-weight:600; margin-bottom:12px; padding-bottom:8px; border-bottom:1px solid var(--border); }}
  .waveform-label {{ font-size:13px; color:var(--text2); margin-bottom:4px; font-weight:500; }}
  .waveform-container {{ position:relative; width:100%; overflow-x:auto; overflow-y:hidden; cursor:grab; border:1px solid var(--border); border-radius:4px; background:#ffffff; margin-bottom:16px; }}
  .waveform-container:active {{ cursor:grabbing; }}
  .waveform-container canvas {{ display:block; }}
  .waveform-time-axis {{ position:relative; height:20px; background:#f7fafc; border:1px solid var(--border); border-radius:4px; margin-bottom:8px; }}
  .waveform-scrollbar {{ position:relative; height:10px; background:#e2e8f0; border-radius:5px; margin-top:4px; }}
  .waveform-scrollbar-thumb {{ position:absolute; height:100%; background:var(--accent); border-radius:5px; min-width:30px; cursor:pointer; opacity:0.7; }}
  .waveform-scrollbar-thumb:hover {{ opacity:1; }}

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
<div><span class="label">对齐置信度：</span><span class="value">{conf:.1}%</span></div>
<div><span class="label">分段数量：</span><span class="value">{seg_count}</span></div>
</div>
</div>

<div class="cards">
<div class="card">
<div class="card-label">MOS-LQO 均值</div>
<div class="card-value {mos_class}">{mos_mean:.2}</div>
<div class="card-hint">ViSQOL 预测质量分（1-5），值越高越好<br>范围：{mos_min:.2}~{mos_max:.2}</div>
</div>
<div class="card">
<div class="card-label">VNSIM 均值</div>
<div class="card-value">{vnsim_mean:.4}</div>
<div class="card-hint">全局神经图相似度（0-1），1=完全相同</div>
</div>
<div class="card">
<div class="card-label">时域中断</div>
<div class="card-value {dropout_class}">{dropout_dur:.0}ms</div>
<div class="card-hint">网络丢包/静音（能量断崖下跌）</div>
</div>
<div class="card">
<div class="card-label">时轴漂移</div>
<div class="card-value {warping_class}">{warping_dur:.0}ms</div>
<div class="card-hint">{warping_types_str}</div>
</div>
<div class="card">
<div class="card-label">频谱损伤</div>
<div class="card-value {spectral_class}">{spectral_score_pct}%</div>
<div class="card-hint">低相似度片段比例</div>
</div>
</div>

<div class="section"><div class="section-title">各段详细评分</div>
<table id="segmentsTable"><thead><tr><th>段</th><th>时间范围</th><th>MOS-LQO</th><th>VNSIM</th><th>低频相似度</th><th>高频相似度</th><th>能量比均值</th><th>异常</th></tr></thead>
<tbody>{table_rows}</tbody>
</table>
<div class="pagination" id="tablePagination"></div>
</div>


<div class="waveform-section" id="sectionWaveform">
<div class="waveform-title">波形对比</div>
<div class="waveform-label">参考音频</div>
<div class="waveform-container" id="waveformRefContainer">
<canvas id="waveformRef" height="120"></canvas>
</div>
<div class="waveform-label">录制音频</div>
<div class="waveform-container" id="waveformDegContainer">
<canvas id="waveformDeg" height="120"></canvas>
</div>
<div class="waveform-time-axis" id="waveformTimeAxis"></div>
</div>

<div class="section" id="sectionMos"><div class="section-title">MOS-LQO 分段趋势</div>
<div class="chart-full"><canvas id="chartMos"></canvas></div>
</div>

<div class="chart-row">
<div class="section" style="margin-bottom:0" id="sectionVnsim"><div class="section-title">VNSIM 分段趋势</div>
<div class="chart-full"><canvas id="chartVnsim"></canvas></div>
</div>
<div class="section" style="margin-bottom:0" id="sectionPatch"><div class="section-title">Patch 时间片段相似度</div>
<div class="chart-full"><canvas id="chartPatch"></canvas></div>
</div>
</div>

<div class="chart-row">
<div class="section" style="margin-bottom:0"><div class="section-title">fVNSIM 频段相似度（多段对比）</div>
<div class="chart-full"><canvas id="chartFvnsim"></canvas></div>
</div>
<div class="section" style="margin-bottom:0"><div class="section-title">频段能量比（多段对比）</div>
<div class="chart-full"><canvas id="chartEnergy"></canvas></div>
</div>
</div>

<div class="section"><div class="section-title">指标说明</div>
<dl class="glossary">
<dt><span class="tag">MOS-LQO</span>ViSQOL预测质量分</dt>
<dd>ViSQOL通过SVM模型将频域相似度映射为1-5的预测分。值域1-5，越高表示预测质量越好。建议用于同场景相对比较。</dd>
<dt><span class="tag">VNSIM</span>全局神经图相似度</dt>
<dd>基于Gammatone听觉滤波器组提取的频谱图，计算参考与录制之间的NSIM。值域0-1，1表示频谱完全一致。</dd>
<dt><span class="tag">fVNSIM</span>各频段相似度</dt>
<dd>每个Gammatone频带上的参考与录制频谱相似度，值域0-1：0=频谱完全不匹配，1=该频带频谱完全一致。低频带(1-10)对应50-500Hz，中频带(11-20)对应500-2000Hz，高频带(21-32)对应2000Hz以上。某频带值低说明该频率范围存在明显降质。</dd>
<dt><span class="tag">fvdegenergy</span>频段能量比</dt>
<dd>每个频带中录制信号相对于参考信号的能量变化比例。值>1表示能量增加(如添加噪声)，值<1表示能量减少(如高频衰减)。</dd>
<dt><span class="tag">Patch相似度</span>时间片段相似度</dt>
<dd>ViSQOL将音频按约0.6秒切分为多个Patch，分别计算每个Patch的NSIM。多段叠加显示便于定位问题时段。</dd>
<dt><span class="tag">时域中断</span>异常静音/丢包</dt>
<dd>检测网络丢包或长时间静音导致的能量断崖下跌。</dd>
<dt><span class="tag">时轴漂移</span>抖动拉伸/压缩</dt>
<dd>检测同一段音频内容在录制端的时长偏差，反映网络抖动导致的音频拉长/压缩。</dd>
<dt><span class="tag">频谱损伤</span>机械音</dt>
<dd>检测时域能量正常但频域结构被破坏的片段（PLC 伪造音、编解码杂音等）。</dd>
</dl>
</div>

</div>

<script>
var REPORT = JSON.parse({report_json});

var waveformRef = JSON.parse({waveform_ref_json});
var waveformDeg = JSON.parse({waveform_deg_json});

// 波形渲染器
(function() {{
  var WAVEFORM_HEIGHT = 120;
  var PIXELS_PER_SECOND = 100;
  var SCROLL_SYNC_GROUP = [];

  function renderWaveform(canvasId, containerId, data, color) {{
    if (!data || !data.pixel_count || data.pixel_count === 0) return;
    
    var canvas = document.getElementById(canvasId);
    var container = document.getElementById(containerId);
    if (!canvas || !container) return;
    
    var ctx = canvas.getContext('2d');
    var pixelCount = data.pixel_count;
    var dpr = window.devicePixelRatio || 1;
    
    // 画布宽度 = 像素点数，高度固定
    var canvasWidth = pixelCount;
    canvas.width = canvasWidth * dpr;
    canvas.height = WAVEFORM_HEIGHT * dpr;
    canvas.style.width = canvasWidth + 'px';
    canvas.style.height = WAVEFORM_HEIGHT + 'px';
    ctx.scale(dpr, dpr);
    
    var centerY = WAVEFORM_HEIGHT / 2;
    var scale = centerY * 0.9; // 留一点边距
    
    // 背景渐变
    ctx.fillStyle = '#ffffff';
    ctx.fillRect(0, 0, canvasWidth, WAVEFORM_HEIGHT);
    
    // 中心线
    ctx.strokeStyle = 'rgba(0,0,0,0.1)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, centerY);
    ctx.lineTo(canvasWidth, centerY);
    ctx.stroke();
    
    // 绘制波形（min/max 填充）
    ctx.fillStyle = color;
    for (var i = 0; i < pixelCount; i++) {{
      var minVal = data.min_values[i];
      var maxVal = data.max_values[i];
      var y1 = centerY - maxVal * scale;
      var y2 = centerY - minVal * scale;
      // 确保至少 1px 高度（静音区域）
      if (Math.abs(y2 - y1) < 0.5) {{
        y1 = centerY - 0.5;
        y2 = centerY + 0.5;
      }}
      ctx.fillRect(i, y1, 1, y2 - y1);
    }}
    
    // 时间刻度（每秒一条竖线）
    var samplesPerPixel = data.samples_per_pixel;
    var duration = data.duration_s;
    ctx.strokeStyle = 'rgba(0,0,0,0.15)';
    ctx.fillStyle = 'rgba(0,0,0,0.4)';
    ctx.font = '10px sans-serif';
    ctx.textAlign = 'center';
    for (var t = 1; t < Math.ceil(duration); t++) {{
      var x = Math.round(t * PIXELS_PER_SECOND);
      if (x >= canvasWidth) break;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, WAVEFORM_HEIGHT);
      ctx.stroke();
      ctx.fillText(t + 's', x, 10);
    }}
    
    // 拖拽滚动
    SCROLL_SYNC_GROUP.push(containerId);
    setupDragScroll(container, canvasId);
  }}
  
  function setupDragScroll(container, canvasId) {{
    var isDragging = false;
    var startX = 0;
    var scrollLeft = 0;
    
    container.addEventListener('mousedown', function(e) {{
      isDragging = true;
      startX = e.pageX - container.offsetLeft;
      scrollLeft = container.scrollLeft;
      container.style.cursor = 'grabbing';
      e.preventDefault();
    }});
    
    container.addEventListener('mousemove', function(e) {{
      if (!isDragging) return;
      var x = e.pageX - container.offsetLeft;
      var walk = (x - startX) * 1.5;
      var newScroll = scrollLeft - walk;
      container.scrollLeft = newScroll;
      // 同步其他波形容器
      SCROLL_SYNC_GROUP.forEach(function(id) {{
        if (id !== container.id) {{
          document.getElementById(id).scrollLeft = newScroll;
        }}
      }});
    }});
    
    container.addEventListener('mouseup', function() {{
      isDragging = false;
      container.style.cursor = 'grab';
    }});
    
    container.addEventListener('mouseleave', function() {{
      isDragging = false;
      container.style.cursor = 'grab';
    }});
    
    // 滚轮水平滚动（支持鼠标滚轮）
    container.addEventListener('wheel', function(e) {{
      if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {{
        e.preventDefault();
        var newScroll = container.scrollLeft + e.deltaY;
        container.scrollLeft = newScroll;
        SCROLL_SYNC_GROUP.forEach(function(id) {{
          if (id !== container.id) {{
            document.getElementById(id).scrollLeft = newScroll;
          }}
        }});
      }}
    }}, {{ passive: false }});
    
    // 触摸拖拽支持
    var touchStartX = 0;
    var touchScrollLeft = 0;
    container.addEventListener('touchstart', function(e) {{
      touchStartX = e.touches[0].pageX;
      touchScrollLeft = container.scrollLeft;
    }}, {{ passive: true }});
    
    container.addEventListener('touchmove', function(e) {{
      var x = e.touches[0].pageX;
      var walk = (touchStartX - x) * 1.5;
      var newScroll = touchScrollLeft + walk;
      container.scrollLeft = newScroll;
      SCROLL_SYNC_GROUP.forEach(function(id) {{
        if (id !== container.id) {{
          document.getElementById(id).scrollLeft = newScroll;
        }}
      }});
    }}, {{ passive: true }});
  }}
  
  // 渲染两个波形
  renderWaveform('waveformRef', 'waveformRefContainer', waveformRef, '#3182ce');
  renderWaveform('waveformDeg', 'waveformDegContainer', waveformDeg, '#e53e3e');
}})();


var segLabels = JSON.parse({seg_labels_json});
var mosValues = JSON.parse({mos_values_json});
var vnsimValues = JSON.parse({vnsim_values_json});
var fvnsimData = JSON.parse({fvnsim_json});
var energyData = JSON.parse({energy_json});
var patchData = JSON.parse({patch_json});
var centerFreqBands = JSON.parse({center_freq_json});

var segColors = ['#3182ce','#e53e3e','#38a169','#d69e2e','#805ad5','#dd6b20','#319795','#b83280'];

// MOS-LQO 分段趋势（单段不显示）
if(segLabels.length > 1){{
  new Chart(document.getElementById('chartMos'),{{
    type:'line',
    data:{{labels:segLabels,datasets:[{{label:'MOS-LQO',data:mosValues,borderColor:'#3182ce',backgroundColor:'rgba(49,130,206,0.1)',fill:true,tension:0.3,pointRadius:5}}]}},
    options:{{responsive:true,maintainAspectRatio:false,scales:{{y:{{min:0,max:5,title:{{display:true,text:'MOS-LQO'}}}}}},plugins:{{title:{{display:true,text:'MOS-LQO分段趋势'}},legend:{{labels:{{usePointStyle:true,pointStyle:'circle',boxWidth:8}}}}}}}}
  }});
}} else {{
  document.getElementById('sectionMos').style.display = 'none';
}}

// VNSIM 分段趋势（单段不显示）
if(segLabels.length > 1){{
  new Chart(document.getElementById('chartVnsim'),{{
    type:'line',
    data:{{labels:segLabels,datasets:[{{label:'VNSIM',data:vnsimValues,borderColor:'#38a169',backgroundColor:'rgba(56,161,105,0.1)',fill:true,tension:0.3,pointRadius:5}}]}},
    options:{{responsive:true,maintainAspectRatio:false,scales:{{y:{{min:0,max:1,title:{{display:true,text:'相似度'}}}}}},plugins:{{title:{{display:true,text:'VNSIM分段趋势'}},legend:{{labels:{{usePointStyle:true,pointStyle:'circle',boxWidth:8}}}}}}}}
  }});
}} else {{
  document.getElementById('sectionVnsim').style.display = 'none';
}}

// fVNSIM 频段相似度 - 动态生成bandLabels基于实际数据长度
var bandLabels = fvnsimData.length > 0 && fvnsimData[0].length > 0 
  ? Array.from({{length:fvnsimData[0].length}},function(_,i){{return 'B'+(i+1);}})
  : Array.from({{length:32}},function(_,i){{return 'B'+(i+1);}});
// 生成频带tooltip标签（带中心频率）
function bandTooltipLabel(label, dataIndex) {{
  if (centerFreqBands.length > dataIndex) {{
    var freq = centerFreqBands[dataIndex];
    return label + ' (' + (freq >= 1000 ? (freq/1000).toFixed(1)+'kHz' : freq.toFixed(0)+'Hz') + ')';
  }}
  return label;
}}
var fvnsimDatasets = fvnsimData.map(function(d,i){{
  return {{label:'第'+(i+1)+'段',data:d,borderColor:segColors[i%segColors.length],pointStyle:'circle',pointRadius:3,fill:false,tension:0.3}};
}});
// fVNSIM 频段相似度（多段对比，单段隐藏）
if(fvnsimData.length > 1){{
  new Chart(document.getElementById('chartFvnsim'),{{
    type:'line',
    data:{{labels:bandLabels,datasets:fvnsimDatasets}},
    options:{{responsive:true,maintainAspectRatio:false,scales:{{y:{{min:0,max:1,title:{{display:true,text:'相似度'}}}}}},plugins:{{title:{{display:true,text:'fVNSIM频段相似度（多段对比）'}},legend:multiSegLegend(fvnsimData.length),tooltip:{{callbacks:{{title:function(items){{return bandTooltipLabel(items[0].label,items[0].dataIndex);}}}}}}}}}}
  }});
}} else {{
  document.getElementById('chartFvnsim').parentElement.style.display = 'none';
}}

// 频段能量比 - 使用与fVNSIM相同的动态bandLabels
var energyDatasets = energyData.map(function(d,i){{
  return {{label:'第'+(i+1)+'段',data:d,borderColor:segColors[i%segColors.length],pointStyle:'circle',pointRadius:3,fill:false,tension:0.3}};
}});
var energyBandLabels = energyData.length > 0 && energyData[0].length > 0
  ? Array.from({{length:energyData[0].length}},function(_,i){{return 'B'+(i+1);}})
  : bandLabels;
// 频段能量比（多段对比，单段隐藏）
if(energyData.length > 1){{
  new Chart(document.getElementById('chartEnergy'),{{
    type:'line',
    data:{{labels:energyBandLabels,datasets:energyDatasets}},
    options:{{responsive:true,maintainAspectRatio:false,scales:{{y:{{title:{{display:true,text:'能量比'}}}}}},plugins:{{title:{{display:true,text:'频段能量比（多段对比）'}},legend:multiSegLegend(energyData.length),tooltip:{{callbacks:{{title:function(items){{return bandTooltipLabel(items[0].label,items[0].dataIndex);}}}}}}}}}}
  }});
}} else {{
  document.getElementById('chartEnergy').parentElement.style.display = 'none';
}}

// Patch 时间片段相似度
if(patchData.length > 0 && patchData[0].length > 0){{
  var allPatchLabels = patchData[0].map(function(_,i){{return 'Patch'+(i+1);}});
  var patchDatasets = patchData.map(function(d,i){{
    return {{label:'第'+(i+1)+'段',data:d,borderColor:segColors[i%segColors.length],pointStyle:'circle',pointRadius:3,fill:false,tension:0.3}};
  }});
  new Chart(document.getElementById('chartPatch'),{{
    type:'line',
    data:{{labels:allPatchLabels,datasets:patchDatasets}},
    options:{{responsive:true,maintainAspectRatio:false,scales:{{y:{{min:0,max:1,title:{{display:true,text:'相似度'}}}}}},plugins:{{title:{{display:true,text:'Patch时间片段相似度'}},legend:multiSegLegend(patchData.length)}}}}
  }});
}} else {{
  document.getElementById('sectionPatch').style.display = 'none';
}}

// 多段图例配置：段数<=8正常显示，>8自动折叠
function multiSegLegend(segCount) {{
  if (segCount <= 8) {{
    return {{labels:{{usePointStyle:true,pointStyle:'circle',boxWidth:8,padding:12}}}};
  }}
  return {{
    position:'bottom',
    labels:{{
      usePointStyle:true,
      pointStyle:'circle',
      boxWidth:8,
      padding:8,
      font:{{size:11}},
      generateLabels:function(chart){{
        var data = chart.data;
        if(!data.datasets.length) return [];
        var shown = [];
        data.datasets.forEach(function(ds,i){{
          shown.push({{
            text:ds.label,
            fillStyle:ds.borderColor,
            strokeStyle:ds.borderColor,
            lineWidth:2,
            pointStyle:'circle',
            hidden:!chart.isDatasetVisible(i),
            datasetIndex:i
          }});
        }});
        // 段数过多时只显示前4个+省略+最后1个
        if(shown.length > 12){{
          var compact = shown.slice(0,4);
          compact.push({{text:'...共'+shown.length+'段',fillStyle:'transparent',strokeStyle:'transparent',lineWidth:0,pointStyle:'circle',hidden:true,datasetIndex:-1}});
          compact.push(shown[shown.length-1]);
          return compact;
        }}
        return shown;
      }}
    }},
    onClick:function(e,item,legend){{
      var index = item.datasetIndex;
      if(index<0) return;
      var ci = legend.chart;
      if(ci.isDatasetVisible(index)){{
        ci.hide(index);
      }} else {{
        ci.show(index);
      }}
    }}
  }};
}}

// X轴优化：当标签过多时自动跳显
(function() {{
  var charts = ['chartMos', 'chartVnsim', 'chartFvnsim', 'chartEnergy', 'chartPatch'];
  charts.forEach(function(id) {{
    var canvas = document.getElementById(id);
    if (!canvas) return;
    var chart = Chart.getChart(canvas);
    if (!chart || !chart.options.scales || !chart.options.scales.x) return;
    var labels = chart.data.labels;
    if (labels && labels.length > 20) {{
      var step = Math.ceil(labels.length / 20);
      chart.options.scales.x.ticks = {{
        autoSkip: true,
        maxRotation: 0,
        callback: function(val, index) {{
          return index % step === 0 ? labels[index] : '';
        }}
      }};
      chart.update('none');
    }}
  }});
}})();

// 表格分页
(function() {{
  var table = document.getElementById('segmentsTable');
  if (!table) return;
  var tbody = table.querySelector('tbody');
  var rows = Array.from(tbody.querySelectorAll('tr'));
  if (rows.length <= 20) return;
  
  var pageSize = 20;
  var currentPage = 1;
  var totalPages = Math.ceil(rows.length / pageSize);
  
  function showPage(page) {{
    currentPage = page;
    rows.forEach(function(row, index) {{
      var start = (page - 1) * pageSize;
      var end = start + pageSize;
      row.style.display = (index >= start && index < end) ? '' : 'none';
    }});
    updateButtons();
  }}
  
  function updateButtons() {{
    var pagination = document.getElementById('tablePagination');
    pagination.innerHTML = '';
    
    var prevBtn = document.createElement('button');
    prevBtn.textContent = '上一页';
    prevBtn.disabled = currentPage === 1;
    prevBtn.onclick = function() {{ showPage(currentPage - 1); }};
    pagination.appendChild(prevBtn);
    
    for (var i = 1; i <= totalPages; i++) {{
      if (i === 1 || i === totalPages || (i >= currentPage - 1 && i <= currentPage + 1)) {{
        var btn = document.createElement('button');
        btn.textContent = i;
        btn.className = (i === currentPage) ? 'active' : '';
        btn.onclick = function() {{ showPage(parseInt(this.textContent)); }};
        pagination.appendChild(btn);
      }} else if (i === currentPage - 2 || i === currentPage + 2) {{
        var span = document.createElement('span');
        span.textContent = '...';
        span.style.padding = '0 4px';
        pagination.appendChild(span);
      }}
    }}
    
    var nextBtn = document.createElement('button');
    nextBtn.textContent = '下一页';
    nextBtn.disabled = currentPage === totalPages;
    nextBtn.onclick = function() {{ showPage(currentPage + 1); }};
    pagination.appendChild(nextBtn);
    
    var info = document.createElement('span');
    info.textContent = ' 共 ' + rows.length + ' 条，' + totalPages + ' 页';
    info.style.marginLeft = '12px';
    info.style.color = 'var(--text3)';
    pagination.appendChild(info);
  }}
  
  showPage(1);
}})();
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
        conf = report.alignment.confidence * 100.0,
        seg_count = report.segments.len(),
        mos_mean = report.overall.moslqo_mean,
        mos_min = report.overall.moslqo_min,
        mos_max = report.overall.moslqo_max,
        mos_class = mos_class,
        dropout_class = dropout_class,
        warping_class = warping_class,
        spectral_class = spectral_class,
        vnsim_mean = report.overall.vnsim_mean,
        dropout_dur = total_dropout,
        warping_dur = total_warping,
        warping_types_str = warping_types_str,
        spectral_score_pct = avg_spectral * 100.0,
        // JS 字符串字面量注入
        report_json = to_js_str(&json_data),
        seg_labels_json = to_js_str(&seg_labels_json),
        mos_values_json = to_js_str(&mos_values_json),
        vnsim_values_json = to_js_str(&vnsim_values_json),
        fvnsim_json = to_js_str(&fvnsim_json),
        energy_json = to_js_str(&energy_json),
        patch_json = to_js_str(&patch_json),
        center_freq_json = to_js_str(&center_freq_json),
        waveform_ref_json = to_js_str(&waveform_ref_json),
        waveform_deg_json = to_js_str(&waveform_deg_json),
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
        // 异常检测：时域中断 + 时轴漂移 + 频谱损伤
        let dropout_ms = seg.anomaly.dropout_duration_ms;
        let warping_ms = seg.anomaly.warping_duration_ms;
        let spectral = seg.anomaly.spectral_artifacts_score;
        let anomaly_str = if seg.anomaly.has_anomaly {
            let mut parts = vec![];
            if dropout_ms > 0.0 {
                parts.push(format!("中断{:.0}ms", dropout_ms.abs()));
            }
            if !seg.anomaly.warpings.is_empty() {
                // 显示漂移子类型：漂移Xms(类型1/类型2)
                let ms = warping_ms.abs();
                let types: Vec<String> = seg.anomaly.warpings.iter()
                    .map(|w| w.drift_type.chinese().to_string())
                    .collect();
                let type_str = types.join("/");
                parts.push(format!("漂移{:.0}ms({})", ms, type_str));
            }
            if spectral > 0.25 {
                parts.push(format!("损伤{:.0}%", spectral * 100.0));
            }
            parts.join(", ")
        } else {
            "无".to_string()
        };

        // 颜色类：MOS < 3 用红色，异常用红色
        let mos_color = if seg.quality.moslqo < 3.0 { "color:#e53e3e;font-weight:bold;" } else { "" };
        let anomaly_color = if seg.anomaly.has_anomaly { "color:#e53e3e;font-weight:bold;" } else { "" };

        format!("<tr><td>第{}段</td><td>{:.2}s-{:.2}s</td><td style=\"{}\">{:.2}</td><td>{:.4}</td><td>{:.4}</td><td>{:.4}</td><td>{:.4}</td><td style=\"{}\">{}</td></tr>",
            i+1, seg.start_time_s, seg.end_time_s, mos_color, seg.quality.moslqo, seg.quality.vnsim, low, high, energy_mean, anomaly_color, anomaly_str)
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
