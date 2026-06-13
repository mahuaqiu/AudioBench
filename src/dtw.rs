//! DTW Patch 匹配模块
//!
//! 实现与 ViSQOL ComparisonPatchesSelector 等效的 DTW 搜索机制
//! 核心参考: comparison_patches_selector.cc



/// Patch 匹配结果
#[derive(Debug, Clone)]
pub struct PatchMatchResult {
    #[allow(dead_code)]
    pub similarity: f64,
    pub ref_start_time: f64,
    pub ref_end_time: f64,
    pub deg_start_time: f64,
    #[allow(dead_code)]
    pub deg_end_time: f64,
}

/// DTW 搜索：在搜索窗口内找最优的 ref-deg patch 匹配（完整DP版）
#[allow(dead_code)]
/// DTW 搜索：在搜索窗口内找最优的 ref-deg patch 匹配（完整DP版）
/// 
/// 算法：
/// 1. 对每个参考 patch，在 deg 信号的搜索窗口内遍历所有可能位置
/// 2. 使用 DP/累积相似度找最优路径
/// 3. Viterbi 回溯获取最终匹配
#[allow(dead_code)]
pub fn find_optimal_patch_matches(
    ref_spectro: &[Vec<f64>],
    deg_spectro: &[Vec<f64>],
    patch_size_bands: usize,
    patch_size_frames: usize,
    frame_duration: f64,
    search_window_radius: usize,
) -> Vec<PatchMatchResult> {
    let num_ref_frames = if !ref_spectro.is_empty() { ref_spectro[0].len() } else { 0 };
    let num_deg_frames = if !deg_spectro.is_empty() { deg_spectro[0].len() } else { 0 };
    
    if num_ref_frames == 0 || num_deg_frames == 0 || patch_size_frames == 0 {
        return vec![];
    }
    
    // 创建参考 patch 索引（每隔 patch_size_frames 一个）
    let patch_size = patch_size_frames;
    let first_patch_idx = patch_size / 2;
    let mut ref_patch_indices: Vec<usize> = vec![];
    let mut i = first_patch_idx;
    while i + patch_size <= num_ref_frames {
        ref_patch_indices.push(i);
        i += patch_size;
    }
    
    if ref_patch_indices.is_empty() {
        return vec![];
    }
    
    let search_window = search_window_radius * patch_size;
    let num_patches = ref_patch_indices.len();
    let num_deg_positions = num_deg_frames.saturating_sub(patch_size - 1);
    
    // DP 表：cumulative_similarity_dp[patch_index][deg_position]
    let mut dp: Vec<Vec<f64>> = vec![vec![f64::NEG_INFINITY; num_deg_positions]; num_patches];
    let mut backtrace: Vec<Vec<i32>> = vec![vec![-1; num_deg_positions]; num_patches];
    
    // 预计算所有 deg patches 的 NSIM（滑动窗口方式）
    // 为了效率，我们只计算需要的
    let _hop_bands = patch_size_bands;
    let _hop_frames = 1; // 逐帧滑动以支持 DTW
    
    // 对每个参考 patch，计算其在搜索窗口内的所有相似度
    for (patch_idx, &ref_frame_start) in ref_patch_indices.iter().enumerate() {
        // 搜索窗口范围
        let search_start = if ref_frame_start > search_window {
            ref_frame_start - search_window
        } else { 0 };
        let search_end = (ref_frame_start + search_window).min(num_deg_positions - 1);
        
        for deg_frame_start in search_start..=search_end {
            // 提取 ref patch
            let ref_patch = extract_patch(ref_spectro, 0, patch_size_bands, 
                ref_frame_start, patch_size_frames);
            
            // 提取 deg patch
            let deg_patch = extract_patch(deg_spectro, 0, patch_size_bands,
                deg_frame_start, patch_size_frames);
            
            // 计算 NSIM
            let nsim_result = crate::spectrogram::compute_patch_nsim(&ref_patch, &deg_patch);
            let sim = nsim_result.similarity;
            
            // DP 计算
            if patch_idx == 0 {
                // 第一个 patch，直接用相似度
                dp[patch_idx][deg_frame_start] = sim;
                backtrace[patch_idx][deg_frame_start] = -1; // 无回溯
            } else {
                // 找前一个 patch 的最优位置
                let prev_search_start = if ref_patch_indices[patch_idx - 1] > search_window {
                    ref_patch_indices[patch_idx - 1] - search_window
                } else { 0 };
                
                let mut best_prev = -1;
                let mut best_score = f64::NEG_INFINITY;
                
                for prev_pos in prev_search_start..deg_frame_start {
                    if prev_pos < num_deg_positions && dp[patch_idx - 1][prev_pos] > best_score {
                        best_score = dp[patch_idx - 1][prev_pos];
                        best_prev = prev_pos as i32;
                    }
                }
                
                if best_prev >= 0 {
                    dp[patch_idx][deg_frame_start] = sim + best_score;
                    backtrace[patch_idx][deg_frame_start] = best_prev;
                }
            }
        }
    }
    
    // 找最后一个 patch 的最优结束位置
    let last_patch_idx = num_patches - 1;
    let search_start = if ref_patch_indices[last_patch_idx] > search_window {
        ref_patch_indices[last_patch_idx] - search_window
    } else { 0 };
    let search_end = (ref_patch_indices[last_patch_idx] + search_window).min(num_deg_positions - 1);
    
    let mut best_end_pos = search_start;
    let mut best_end_score = f64::NEG_INFINITY;
    for pos in search_start..=search_end {
        if dp[last_patch_idx][pos] > best_end_score {
            best_end_score = dp[last_patch_idx][pos];
            best_end_pos = pos;
        }
    }
    
    // Viterbi 回溯
    let mut results = Vec::with_capacity(num_patches);
    let mut current_pos = best_end_pos;
    
    for patch_idx in (0..num_patches).rev() {
        let ref_frame_start = ref_patch_indices[patch_idx];
        let ref_start_time = ref_frame_start as f64 * frame_duration;
        let ref_end_time = (ref_frame_start + patch_size_frames) as f64 * frame_duration;
        let deg_start_time = current_pos as f64 * frame_duration;
        let deg_end_time = (current_pos + patch_size_frames) as f64 * frame_duration;
        
        let ref_patch = extract_patch(ref_spectro, 0, patch_size_bands,
            ref_frame_start, patch_size_frames);
        let deg_patch = extract_patch(deg_spectro, 0, patch_size_bands,
            current_pos, patch_size_frames);
        let nsim = crate::spectrogram::compute_patch_nsim(&ref_patch, &deg_patch);
        
        results.push(PatchMatchResult {
            similarity: nsim.similarity,
            ref_start_time: ref_start_time,
            ref_end_time: ref_end_time,
            deg_start_time,
            deg_end_time,
        });
        
        if patch_idx > 0 {
            current_pos = backtrace[patch_idx][current_pos] as usize;
        }
    }
    
    results.reverse();
    results
}

/// 提取 patch（从频谱图）
fn extract_patch(
    spectrogram: &[Vec<f64>],
    start_band: usize,
    num_bands: usize,
    start_frame: usize,
    num_frames: usize,
) -> Vec<Vec<f64>> {
    let mut patch = Vec::with_capacity(num_bands);
    
    for b in start_band..(start_band + num_bands).min(spectrogram.len()) {
        let mut band_data = Vec::with_capacity(num_frames);
        for f in start_frame..(start_frame + num_frames) {
            let v = if f < spectrogram[b].len() { spectrogram[b][f] } else { 0.0 };
            band_data.push(v);
        }
        patch.push(band_data);
    }
    
    patch
}

/// 简化版 DTW：只做滑动搜索（不使用完整 DP，适合快速评估）
/// 对每个参考 patch，在搜索窗口内找最优 deg 位置
pub fn simple_sliding_search(
    ref_spectro: &[Vec<f64>],
    deg_spectro: &[Vec<f64>],
    patch_size_bands: usize,
    patch_size_frames: usize,
    frame_duration: f64,
    search_window_radius: usize,
) -> Vec<PatchMatchResult> {
    let num_ref_frames = if !ref_spectro.is_empty() { ref_spectro[0].len() } else { 0 };
    let num_deg_frames = if !deg_spectro.is_empty() { deg_spectro[0].len() } else { 0 };
    
    if num_ref_frames == 0 || num_deg_frames == 0 || patch_size_frames == 0 {
        return vec![];
    }
    
    // 创建参考 patch 索引
    let patch_size = patch_size_frames;
    let first_patch_idx = patch_size / 2;
    let mut ref_patch_indices: Vec<usize> = vec![];
    let mut i = first_patch_idx;
    while i + patch_size <= num_ref_frames {
        ref_patch_indices.push(i);
        i += patch_size;
    }
    
    if ref_patch_indices.is_empty() {
        return vec![];
    }
    
    let search_window = search_window_radius * patch_size;
    let num_deg_positions = num_deg_frames.saturating_sub(patch_size - 1);
    
    let mut results = Vec::with_capacity(ref_patch_indices.len());
    
    for &ref_frame_start in &ref_patch_indices {
        // 搜索窗口范围
        let search_start = ref_frame_start.saturating_sub(search_window);
        let search_end = (ref_frame_start + search_window).min(num_deg_positions.saturating_sub(1));
        
        // 在搜索窗口内找最优位置
        let mut best_pos = ref_frame_start;
        let mut best_sim = f64::NEG_INFINITY;
        
        for deg_frame_start in search_start..=search_end {
            let ref_patch = extract_patch(ref_spectro, 0, patch_size_bands,
                ref_frame_start, patch_size_frames);
            let deg_patch = extract_patch(deg_spectro, 0, patch_size_bands,
                deg_frame_start, patch_size_frames);
            
            let nsim = crate::spectrogram::compute_patch_nsim(&ref_patch, &deg_patch);
            
            if nsim.similarity > best_sim {
                best_sim = nsim.similarity;
                best_pos = deg_frame_start;
            }
        }
        
        let ref_start_time = ref_frame_start as f64 * frame_duration;
        let ref_end_time = (ref_frame_start + patch_size_frames) as f64 * frame_duration;
        let deg_start_time = best_pos as f64 * frame_duration;
        let deg_end_time = (best_pos + patch_size_frames) as f64 * frame_duration;
        
        results.push(PatchMatchResult {
            similarity: best_sim,
            ref_start_time,
            ref_end_time,
            deg_start_time,
            deg_end_time,
        });
    }
    
    results
}
