use gtk4::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvEntry {
    pub key: String,
    pub value: String,
    pub enabled: bool,
    pub comment_above: Option<String>,
}

fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

fn parse_env_value(raw_val: &str) -> String {
    let mut val = raw_val.trim();
    if !val.starts_with('"') && !val.starts_with('\'') {
        if let Some((v, _)) = val.split_once('#') {
            val = v.trim();
        }
    }
    val.trim_matches(|c| c == '\'' || c == '"').to_string()
}

fn parse_disabled_env_line(trimmed: &str) -> Option<EnvEntry> {
    let after_hash = trimmed.strip_prefix('#')?;
    if after_hash.starts_with('#') {
        return None;
    }
    let candidate = after_hash.trim_start();
    let var_line = candidate.strip_prefix("export ").unwrap_or(candidate).trim_start();
    let (raw_key, raw_val) = var_line.split_once('=')?;
    if !is_valid_env_key(raw_key) || ManagedFeatures::is_managed_key(raw_key) {
        return None;
    }
    Some(EnvEntry {
        key: raw_key.to_string(),
        value: parse_env_value(raw_val),
        enabled: false,
        comment_above: None,
    })
}

pub fn is_key_overridden_in_entries(entries: &[EnvEntry], global_key: &str) -> bool {
    let target = global_key.trim();
    if target.is_empty() {
        return false;
    }
    entries.iter().any(|e| e.enabled && e.key.trim() == target)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeatureState {
    Inherit,
    Enabled(String),
    Disabled,
}

impl FeatureState {
    pub fn is_enabled(&self) -> bool {
        matches!(self, FeatureState::Enabled(_))
    }

    pub fn extra_args(&self) -> &str {
        match self {
            FeatureState::Enabled(s) => s.as_str(),
            _ => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Unknown,
}

impl GpuVendor {
    pub fn name(&self) -> &'static str {
        match self {
            GpuVendor::Nvidia => "NVIDIA",
            GpuVendor::Amd => "AMD",
            GpuVendor::Intel => "Intel",
            GpuVendor::Unknown => "Unknown",
        }
    }

    fn pci_vendor_id(self) -> Option<&'static str> {
        match self {
            GpuVendor::Nvidia => Some("0x10de"),
            GpuVendor::Amd => Some("0x1002"),
            GpuVendor::Intel => Some("0x8086"),
            GpuVendor::Unknown => None,
        }
    }
}

pub fn detect_gpu_vendor() -> GpuVendor {
    if let Ok(entries) = fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if name.starts_with("card") && name[4..].chars().all(|c| c.is_ascii_digit()) {
                let vendor_file = entry.path().join("device/vendor");
                if let Ok(content) = fs::read_to_string(vendor_file) {
                    let v = content.trim().to_lowercase();
                    if v == "0x10de" {
                        return GpuVendor::Nvidia;
                    } else if v == "0x1002" {
                        return GpuVendor::Amd;
                    } else if v == "0x8086" {
                        return GpuVendor::Intel;
                    }
                }
            }
        }
    }

    if is_command_available("nvidia-smi") {
        return GpuVendor::Nvidia;
    }

    GpuVendor::Unknown
}

fn normalize_hex_id(raw: &str) -> String {
    let trimmed = raw.trim().to_lowercase();
    if trimmed.starts_with("0x") {
        trimmed
    } else {
        format!("0x{trimmed}")
    }
}

fn find_drm_card_device(vendor: GpuVendor) -> Option<PathBuf> {
    let target_vendor = vendor.pci_vendor_id();
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if !name.starts_with("card") || !name[4..].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let dev_dir = entry.path().join("device");
        if let Some(tv) = target_vendor {
            let Ok(v) = std::fs::read_to_string(dev_dir.join("vendor")) else {
                continue;
            };
            if normalize_hex_id(&v) != tv {
                continue;
            }
        }
        return Some(dev_dir);
    }
    None
}

pub fn get_pci_device_id(vendor: GpuVendor) -> Option<String> {
    if let Some(dev_dir) = find_drm_card_device(vendor) {
        if let (Ok(v), Ok(d)) = (
            std::fs::read_to_string(dev_dir.join("vendor")),
            std::fs::read_to_string(dev_dir.join("device")),
        ) {
            return Some(format!("{}:{}", normalize_hex_id(&v), normalize_hex_id(&d)));
        }
    }

    if vendor == GpuVendor::Nvidia {
        if let Ok(output) = std::process::Command::new("nvidia-smi")
            .args(["--query-gpu=pci.device_id", "--format=csv,noheader"])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(first_line) = text.lines().next() {
                    let id = first_line.trim().to_lowercase();
                    if id.starts_with("0x") && id.len() == 10 {
                        return Some(format!("0x{}:0x{}", &id[6..10], &id[2..6]));
                    }
                }
            }
        }
    }

    None
}

pub fn detect_gpu_vram_mb(vendor: GpuVendor) -> Option<u64> {
    if vendor == GpuVendor::Nvidia {
        if let Ok(output) = std::process::Command::new("nvidia-smi")
            .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(first_line) = text.lines().next() {
                    if let Ok(mb) = first_line.trim().parse::<u64>() {
                        if mb > 0 {
                            return Some(mb);
                        }
                    }
                }
            }
        }
    }

    if let Some(dev_dir) = find_drm_card_device(vendor) {
        if let Ok(bytes_str) = std::fs::read_to_string(dev_dir.join("mem_info_vram_total")) {
            if let Ok(bytes) = bytes_str.trim().parse::<u64>() {
                let mb = bytes / (1024 * 1024);
                if mb > 0 {
                    return Some(mb);
                }
            }
        }
    }

    None
}

pub fn compute_vram_envvars_with(
    mb_str: &str,
    total_vram_mb: Option<u64>,
    pci_device: Option<&str>,
) -> Vec<(&'static str, String)> {
    let digits: String = mb_str.chars().filter(|c| c.is_ascii_digit()).collect();
    let reserve_mb = digits.parse::<u64>().unwrap_or(0);
    if reserve_mb == 0 {
        return Vec::new();
    }

    let Some(total_mb) = total_vram_mb else {
        return Vec::new();
    };
    if reserve_mb >= total_mb {
        return Vec::new();
    }

    let Some(dev_str) = pci_device.filter(|s| !s.trim().is_empty()) else {
        return Vec::new();
    };

    let limit_mb = total_mb - reserve_mb;
    vec![
        ("VK_LOADER_LAYERS_ENABLE", "VK_LAYER_MESA_vram_report_limit".to_string()),
        ("VK_VRAM_REPORT_LIMIT_HEAP_SIZE", limit_mb.to_string()),
        ("VK_VRAM_REPORT_LIMIT_DEVICE_ID", dev_str.to_string()),
    ]
}

pub fn compute_vram_envvars(mb_str: &str, vendor: GpuVendor) -> Vec<(&'static str, String)> {
    static CACHED_GPU_INFO: std::sync::OnceLock<(Option<u64>, Option<String>)> = std::sync::OnceLock::new();
    let (total_vram_mb, pci_device) =
        CACHED_GPU_INFO.get_or_init(|| (detect_gpu_vram_mb(vendor), get_pci_device_id(vendor)));
    compute_vram_envvars_with(mb_str, *total_vram_mb, pci_device.as_deref())
}

pub fn compute_shader_cache_envvars(mb_str: &str, vendor: GpuVendor) -> Vec<(&'static str, String)> {
    let digits: String = mb_str.chars().filter(|c| c.is_ascii_digit()).collect();
    let mb = digits.parse::<u64>().unwrap_or(0);
    if mb == 0 {
        return match vendor {
            GpuVendor::Nvidia => {
                vec![("__GL_SHADER_DISK_CACHE", "0".to_string())]
            }
            GpuVendor::Amd | GpuVendor::Intel => {
                vec![("MESA_SHADER_CACHE_DISABLE", "true".to_string())]
            }
            GpuVendor::Unknown => {
                vec![
                    ("__GL_SHADER_DISK_CACHE", "0".to_string()),
                    ("MESA_SHADER_CACHE_DISABLE", "true".to_string()),
                ]
            }
        };
    }

    let bytes = mb.saturating_mul(1024 * 1024);
    match vendor {
        GpuVendor::Nvidia => {
            vec![
                ("__GL_SHADER_DISK_CACHE", "1".to_string()),
                ("__GL_SHADER_DISK_CACHE_SKIP_CLEANUP", "1".to_string()),
                ("__GL_SHADER_DISK_CACHE_SIZE", bytes.to_string()),
            ]
        }
        GpuVendor::Amd | GpuVendor::Intel => {
            vec![("MESA_SHADER_CACHE_MAX_SIZE", format!("{mb}M"))]
        }
        GpuVendor::Unknown => {
            vec![
                ("__GL_SHADER_DISK_CACHE_SIZE", bytes.to_string()),
                ("MESA_SHADER_CACHE_MAX_SIZE", format!("{mb}M")),
            ]
        }
    }
}

pub fn compute_effective_envvars(app_id: &str) -> Vec<(String, String)> {
    compute_effective_envvars_from_dir(&get_config_dir(), app_id)
}

pub fn compute_effective_envvars_from_dir(config_dir: &Path, app_id: &str) -> Vec<(String, String)> {
    let trimmed = app_id.trim();
    let global_cfg = GameConfig::load_from_file(&config_dir.join("global.env")).ok();
    let game_cfg = if !trimmed.is_empty() && trimmed != "global" {
        GameConfig::load_from_file(&config_dir.join(format!("{}.env", trimmed))).ok()
    } else {
        None
    };

    let mut env_map = std::collections::BTreeMap::<String, String>::new();

    // 1. Custom envvars from global.env
    if let Some(cfg) = &global_cfg {
        for entry in &cfg.entries {
            if !entry.enabled {
                continue;
            }
            let k = entry.key.trim();
            if !k.is_empty() && !ManagedFeatures::is_managed_key(k) && !k.starts_with("NEUTRON_") {
                env_map.insert(k.to_string(), entry.value.clone());
            }
        }
    }

    // 2. Custom envvars from game config (overriding global)
    if let Some(cfg) = &game_cfg {
        for entry in &cfg.entries {
            if !entry.enabled {
                continue;
            }
            let k = entry.key.trim();
            if !k.is_empty() && !ManagedFeatures::is_managed_key(k) && !k.starts_with("NEUTRON_") {
                env_map.insert(k.to_string(), entry.value.clone());
            }
        }
    }

    // 3. Managed features
    let global_mf = global_cfg
        .as_ref()
        .map(|c| ManagedFeatures::extract(&c.entries, true))
        .unwrap_or_else(|| ManagedFeatures::extract(&[], true));
    let game_mf = game_cfg.as_ref().map(|c| ManagedFeatures::extract(&c.entries, false));

    // MangoHud
    let mangohud = game_mf
        .as_ref()
        .map(|g| &g.mangohud)
        .filter(|m| !matches!(m, FeatureState::Inherit))
        .unwrap_or(&global_mf.mangohud);
    match mangohud {
        FeatureState::Enabled(cfg) => {
            env_map.insert("MANGOHUD".to_string(), "1".to_string());
            if !cfg.trim().is_empty() {
                env_map.insert("MANGOHUD_CONFIG".to_string(), cfg.trim().to_string());
            }
        }
        FeatureState::Disabled => {
            env_map.insert("MANGOHUD".to_string(), "0".to_string());
        }
        FeatureState::Inherit => {}
    }

    // GameScope
    let gamescope = game_mf
        .as_ref()
        .map(|g| &g.gamescope)
        .filter(|m| !matches!(m, FeatureState::Inherit))
        .unwrap_or(&global_mf.gamescope);
    if gamescope.is_enabled() {
        env_map.insert("ENABLE_GAMESCOPE_WSI".to_string(), "1".to_string());
    }

    // Disable SMT
    let disable_smt = game_mf
        .as_ref()
        .map(|g| &g.disable_smt)
        .filter(|m| !matches!(m, FeatureState::Inherit))
        .unwrap_or(&global_mf.disable_smt);
    if disable_smt.is_enabled() {
        let physical_cpus = get_physical_cpu_ids();
        if !physical_cpus.is_empty() {
            let count = physical_cpus.len();
            let cpus_str = physical_cpus
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(",");
            env_map.insert("WINE_CPU_TOPOLOGY".to_string(), format!("{}:{}", count, cpus_str));
        }
    }

    // Reserve VRAM Headroom
    let vram = game_mf
        .as_ref()
        .map(|g| &g.reserve_vram)
        .filter(|m| !matches!(m, FeatureState::Inherit))
        .unwrap_or(&global_mf.reserve_vram);
    if let FeatureState::Enabled(mb_str) = vram {
        let vendor = detect_gpu_vendor();
        for (k, v) in compute_vram_envvars(mb_str, vendor) {
            env_map.insert(k.to_string(), v);
        }
    }

    // Shader Cache Size
    let shader = game_mf
        .as_ref()
        .map(|g| &g.shader_cache_size)
        .filter(|m| !matches!(m, FeatureState::Inherit))
        .unwrap_or(&global_mf.shader_cache_size);
    let vendor = detect_gpu_vendor();
    match shader {
        FeatureState::Enabled(mb_str) => {
            for (k, v) in compute_shader_cache_envvars(mb_str, vendor) {
                env_map.insert(k.to_string(), v);
            }
        }
        FeatureState::Disabled => {
            for (k, v) in compute_shader_cache_envvars("0", vendor) {
                env_map.insert(k.to_string(), v);
            }
        }
        FeatureState::Inherit => {}
    }

    // DLSS overrides
    let dlss = game_mf
        .as_ref()
        .and_then(|g| g.dlss.as_ref())
        .or(global_mf.dlss.as_ref());
    if let Some(dlss) = dlss {
        if dlss.upgrade {
            env_map.insert("PROTON_DLSS_UPGRADE".to_string(), "1".to_string());
        }
        if dlss.indicator {
            env_map.insert(
                "DXVK_NVAPI_SET_NGX_DEBUG_OPTIONS".to_string(),
                "DLSSIndicator=1024,DLSSGIndicator=2".to_string(),
            );
        }
        if !dlss.super_resolution.is_empty() {
            env_map.insert("DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE".to_string(), "on".to_string());
            env_map.insert(
                "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION".to_string(),
                dlss.super_resolution.clone(),
            );
        }
        if !dlss.ray_reconstruction.is_empty() {
            env_map.insert("DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE".to_string(), "on".to_string());
            env_map.insert(
                "DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE_RENDER_PRESET_SELECTION".to_string(),
                dlss.ray_reconstruction.clone(),
            );
        }
        if !dlss.frame_generation.is_empty() {
            env_map.insert("DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE".to_string(), "on".to_string());
            env_map.insert(
                "DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE_RENDER_PRESET_SELECTION".to_string(),
                dlss.frame_generation.clone(),
            );
        }
    }

    env_map.into_iter().collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DlssSettings {
    pub upgrade: bool,
    pub indicator: bool,
    pub super_resolution: String,   // e.g. "render_preset_k" or ""
    pub ray_reconstruction: String, // e.g. "render_preset_e" or ""
    pub frame_generation: String,   // e.g. "render_preset_latest" or ""
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DlssPresetOption {
    pub key: &'static str,
    pub title: &'static str,
    pub note: &'static str,
}

pub const DLSS_SR_PRESETS: &[DlssPresetOption] = &[
    DlssPresetOption {
        key: "",
        title: "Default",
        note: "Game default selection",
    },
    DlssPresetOption {
        key: "render_preset_latest",
        title: "Preset Latest",
        note: "Recommended auto-selection",
    },
    DlssPresetOption {
        key: "render_preset_e",
        title: "Preset E",
        note: "CNN model",
    },
    DlssPresetOption {
        key: "render_preset_f",
        title: "Preset F",
        note: "DLAA / Ultra Quality modes",
    },
    DlssPresetOption {
        key: "render_preset_j",
        title: "Preset J",
        note: "CNN refined",
    },
    DlssPresetOption {
        key: "render_preset_k",
        title: "Preset K",
        note: "Transformer 1st gen",
    },
    DlssPresetOption {
        key: "render_preset_m",
        title: "Preset M",
        note: "Transformer 2nd gen",
    },
];

pub const DLSS_RR_PRESETS: &[DlssPresetOption] = &[
    DlssPresetOption {
        key: "",
        title: "Default",
        note: "Game default selection",
    },
    DlssPresetOption {
        key: "render_preset_latest",
        title: "Preset Latest",
        note: "Recommended auto-selection",
    },
    DlssPresetOption {
        key: "render_preset_d",
        title: "Preset D",
        note: "Transformer 1st gen (DLSS 3.5 baseline)",
    },
    DlssPresetOption {
        key: "render_preset_e",
        title: "Preset E",
        note: "Transformer refined (improved stability)",
    },
    DlssPresetOption {
        key: "render_preset_f",
        title: "Preset F",
        note: "Transformer 2nd gen (DLSS 4.5, minimal ghosting)",
    },
];

pub const DLSS_FG_PRESETS: &[DlssPresetOption] = &[
    DlssPresetOption {
        key: "",
        title: "Default",
        note: "Game default selection",
    },
    DlssPresetOption {
        key: "render_preset_latest",
        title: "Preset Latest",
        note: "Recommended auto-selection",
    },
    DlssPresetOption {
        key: "render_preset_a",
        title: "Preset A",
        note: "Standard frame generation",
    },
    DlssPresetOption {
        key: "render_preset_b",
        title: "Preset B",
        note: "Alternative frame generation",
    },
    DlssPresetOption {
        key: "render_preset_c",
        title: "Preset C",
        note: "Refined frame generation",
    },
    DlssPresetOption {
        key: "render_preset_d",
        title: "Preset D",
        note: "Transformer frame generation",
    },
];

pub fn get_nvidia_gpu_model() -> Option<String> {
    // 1. Check /proc/driver/nvidia/gpus/*/information
    if let Ok(entries) = fs::read_dir("/proc/driver/nvidia/gpus") {
        for entry in entries.flatten() {
            let info = entry.path().join("information");
            if let Ok(content) = fs::read_to_string(info) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("Model:") {
                        let model = trimmed.strip_prefix("Model:").unwrap_or("").trim();
                        if !model.is_empty() {
                            return Some(model.to_string());
                        }
                    }
                }
            }
        }
    }

    // 2. Query nvidia-smi if available
    if let Ok(output) = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .output()
    {
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout);
            if let Some(first_line) = name.lines().next() {
                let trimmed = first_line.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    None
}

pub fn is_dlss_capable_model(model: &str) -> bool {
    let lower = model.to_lowercase();
    lower.contains("rtx")
        || lower.contains("titan v")
        || lower.contains("titan rtx")
        || lower.contains("ada generation")
        || lower.contains("blackwell")
}

pub fn supports_dlss() -> bool {
    let vendor = detect_gpu_vendor();
    if vendor != GpuVendor::Nvidia {
        return false;
    }

    if let Some(model) = get_nvidia_gpu_model() {
        return is_dlss_capable_model(&model);
    }

    // Fallback: If Nvidia vendor was detected but model couldn't be parsed, default to true
    true
}

pub fn supports_dlss_frame_generation() -> bool {
    if let Some(model) = get_nvidia_gpu_model() {
        return is_frame_generation_capable_model(&model);
    }
    false
}

pub fn is_frame_generation_capable_model(model: &str) -> bool {
    let lower = model.to_lowercase();
    lower.contains("rtx 40")
        || lower.contains("rtx 50")
        || lower.contains("rtx 60")
        || lower.contains("ada generation")
        || lower.contains("blackwell")
}

pub fn parse_cpu_list_string(content: &str) -> Vec<usize> {
    let trimmed = content.trim();
    let list_part = match trimmed.split_once(':') {
        Some((prefix, rest)) if prefix.trim().is_empty() || prefix.trim().chars().all(|c| c.is_ascii_digit()) => {
            rest.trim()
        }
        _ => trimmed,
    };

    let mut cpus = Vec::new();
    for part in list_part.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((start_str, end_str)) = part.split_once('-') {
            if let (Ok(start), Ok(end)) = (start_str.trim().parse::<usize>(), end_str.trim().parse::<usize>()) {
                for id in start..=end {
                    cpus.push(id);
                }
            }
        } else if let Ok(id) = part.parse::<usize>() {
            cpus.push(id);
        }
    }
    cpus.sort_unstable();
    cpus.dedup();
    cpus
}

pub fn get_physical_cpu_ids() -> Vec<usize> {
    let mut primary_threads = std::collections::BTreeSet::new();
    let cpu_base = Path::new("/sys/devices/system/cpu");
    if let Ok(entries) = fs::read_dir(cpu_base) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("cpu") && name_str[3..].chars().all(|c| c.is_ascii_digit()) {
                let topo_dir = entry.path().join("topology");
                let siblings_file = topo_dir.join("thread_siblings_list");
                let core_cpus_file = topo_dir.join("core_cpus_list");
                let file_to_read = if siblings_file.is_file() {
                    Some(siblings_file)
                } else if core_cpus_file.is_file() {
                    Some(core_cpus_file)
                } else {
                    None
                };
                if let Some(path) = file_to_read {
                    if let Ok(content) = fs::read_to_string(path) {
                        let cpus = parse_cpu_list_string(&content);
                        if let Some(&first) = cpus.first() {
                            primary_threads.insert(first);
                        }
                    }
                }
            }
        }
    }
    primary_threads.into_iter().collect()
}

pub fn get_total_cpu_count() -> usize {
    let cpu_base = Path::new("/sys/devices/system/cpu");
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(cpu_base) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("cpu") && name_str[3..].chars().all(|c| c.is_ascii_digit()) {
                count += 1;
            }
        }
    }
    if count > 0 {
        count
    } else {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
    }
}

pub fn get_physical_cpu_count() -> usize {
    let ids = get_physical_cpu_ids();
    if !ids.is_empty() {
        ids.len()
    } else {
        get_total_cpu_count()
    }
}

pub fn is_smt_supported_and_active() -> bool {
    if let Ok(control) = fs::read_to_string("/sys/devices/system/cpu/smt/control") {
        let trimmed = control.trim();
        if trimmed == "off" || trimmed == "notsupported" || trimmed == "notimplemented" {
            return false;
        }
    }
    let total = get_total_cpu_count();
    let phys = get_physical_cpu_ids().len();
    phys > 0 && total > phys
}

pub fn format_cpu_ranges(cpus: &[usize]) -> String {
    if cpus.is_empty() {
        return String::new();
    }
    let mut sorted = cpus.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut ranges = Vec::new();
    let mut start = sorted[0];
    let mut prev = start;

    for &c in &sorted[1..] {
        if c == prev + 1 {
            prev = c;
        } else {
            if start == prev {
                ranges.push(start.to_string());
            } else if prev == start + 1 {
                ranges.push(format!("{},{}", start, prev));
            } else {
                ranges.push(format!("{}-{}", start, prev));
            }
            start = c;
            prev = c;
        }
    }

    if start == prev {
        ranges.push(start.to_string());
    } else if prev == start + 1 {
        ranges.push(format!("{},{}", start, prev));
    } else {
        ranges.push(format!("{}-{}", start, prev));
    }

    ranges.join(",")
}

pub fn get_physical_cpu_cores_arg() -> String {
    let ids = get_physical_cpu_ids();
    format_cpu_ranges(&ids)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedFeatures {
    pub proton_version: Option<String>,
    pub gamemode: FeatureState,
    pub gamescope: FeatureState,
    pub mangohud: FeatureState,
    pub disable_smt: FeatureState,
    pub reserve_vram: FeatureState,
    pub shader_cache_size: FeatureState,
    pub launcher_timeout: Option<String>,
    pub dlss: Option<DlssSettings>,
}

impl ManagedFeatures {
    pub fn extract(entries: &[EnvEntry], is_global: bool) -> Self {
        let get = |key: &str| -> Option<String> {
            entries
                .iter()
                .rev()
                .find(|e| e.enabled && e.key == key)
                .map(|e| e.value.clone())
        };

        let proton_version = get("NEUTRON_USE_PROTON").or_else(|| get("PROTON_VERSION")).or_else(|| {
            if is_global {
                Some("proton-cachyos-slr".to_string())
            } else {
                None
            }
        });

        let gamemode = if let Some(v) = get("NEUTRON_USE_GAMEMODE")
            .or_else(|| get("NEUTRON_GAMEMODE"))
            .or_else(|| get("GAMEMODERUN"))
        {
            if v == "0" {
                FeatureState::Disabled
            } else if v == "1" || v.is_empty() {
                FeatureState::Enabled("".to_string())
            } else {
                FeatureState::Enabled(v)
            }
        } else if is_global {
            FeatureState::Enabled("".to_string())
        } else {
            FeatureState::Inherit
        };

        let gamescope = if let Some(v) = get("NEUTRON_USE_GAMESCOPE").or_else(|| get("NEUTRON_GAMESCOPE")) {
            if v == "0" {
                FeatureState::Disabled
            } else if v == "1" || v.is_empty() {
                FeatureState::Enabled("".to_string())
            } else {
                FeatureState::Enabled(v)
            }
        } else if is_global {
            FeatureState::Disabled
        } else {
            FeatureState::Inherit
        };

        let mangohud = if let Some(v) = get("MANGOHUD") {
            if v == "0" {
                FeatureState::Disabled
            } else {
                let cfg = get("MANGOHUD_CONFIG").unwrap_or_default();
                FeatureState::Enabled(cfg)
            }
        } else if is_global {
            FeatureState::Disabled
        } else {
            FeatureState::Inherit
        };

        let disable_smt = if let Some(v) = get("NEUTRON_DISABLE_SMT") {
            if v == "0" {
                FeatureState::Disabled
            } else {
                FeatureState::Enabled(String::new())
            }
        } else if let Some(v) = get("NEUTRON_SMT") {
            if v == "0" {
                FeatureState::Enabled(String::new())
            } else {
                FeatureState::Disabled
            }
        } else if let Some(v) = get("WINE_CPU_TOPOLOGY") {
            let trimmed = v.trim();
            if trimmed.is_empty() || trimmed == "0" {
                FeatureState::Disabled
            } else {
                FeatureState::Enabled(String::new())
            }
        } else if is_global {
            FeatureState::Disabled
        } else {
            FeatureState::Inherit
        };

        let reserve_vram = if let Some(v) = get("NEUTRON_RESERVE_VRAM") {
            if v == "0" {
                FeatureState::Disabled
            } else {
                FeatureState::Enabled(v)
            }
        } else if is_global {
            FeatureState::Disabled
        } else {
            FeatureState::Inherit
        };

        let shader_cache_size = if let Some(v) = get("NEUTRON_SHADER_CACHE_SIZE") {
            if v == "0" {
                FeatureState::Disabled
            } else {
                FeatureState::Enabled(v)
            }
        } else if let Some(gl_size) = get("__GL_SHADER_DISK_CACHE_SIZE") {
            if get("__GL_SHADER_DISK_CACHE").as_deref() == Some("0") {
                FeatureState::Disabled
            } else if let Ok(bytes) = gl_size.parse::<u64>() {
                let mib = bytes / (1024 * 1024);
                FeatureState::Enabled(mib.to_string())
            } else {
                FeatureState::Enabled("10240".to_string())
            }
        } else if let Some(mesa_size) = get("MESA_SHADER_CACHE_MAX_SIZE") {
            if get("MESA_SHADER_CACHE_DISABLE").as_deref() == Some("true") {
                FeatureState::Disabled
            } else if let Some(stripped) = mesa_size.strip_suffix(['g', 'G']) {
                if let Ok(gib) = stripped.parse::<u64>() {
                    FeatureState::Enabled((gib * 1024).to_string())
                } else {
                    FeatureState::Enabled("10240".to_string())
                }
            } else if let Some(stripped) = mesa_size.strip_suffix(['m', 'M']) {
                FeatureState::Enabled(stripped.to_string())
            } else if let Ok(val) = mesa_size.parse::<u64>() {
                FeatureState::Enabled((val / (1024 * 1024)).to_string())
            } else {
                FeatureState::Enabled("10240".to_string())
            }
        } else if is_global {
            FeatureState::Enabled("10240".to_string())
        } else {
            FeatureState::Inherit
        };

        let launcher_timeout = if is_global {
            get("NEUTRON_LAUNCHER_TIMEOUT").or_else(|| Some("2".to_string()))
        } else {
            None
        };

        let has_any_dlss_key = entries.iter().any(|e| {
            e.enabled
                && matches!(
                    e.key.as_str(),
                    "PROTON_DLSS_UPGRADE"
                        | "PROTON_ENABLE_NGX_UPDATER"
                        | "DXVK_NVAPI_SET_NGX_DEBUG_OPTIONS"
                        | "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE"
                        | "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION"
                        | "DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE"
                        | "DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE_RENDER_PRESET_SELECTION"
                        | "DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE"
                        | "DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE_RENDER_PRESET_SELECTION"
                )
        });

        let dlss = if is_global || has_any_dlss_key {
            let upgrade = get("PROTON_DLSS_UPGRADE").as_deref() == Some("1");
            let indicator = get("DXVK_NVAPI_SET_NGX_DEBUG_OPTIONS")
                .map(|s| s.contains("DLSSIndicator=1024"))
                .unwrap_or(false);
            let super_resolution =
                get("DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION").unwrap_or_default();
            let ray_reconstruction =
                get("DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE_RENDER_PRESET_SELECTION").unwrap_or_default();
            let frame_generation =
                get("DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE_RENDER_PRESET_SELECTION").unwrap_or_default();
            Some(DlssSettings {
                upgrade,
                indicator,
                super_resolution,
                ray_reconstruction,
                frame_generation,
            })
        } else {
            None
        };

        ManagedFeatures {
            proton_version,
            gamemode,
            gamescope,
            mangohud,
            disable_smt,
            reserve_vram,
            shader_cache_size,
            launcher_timeout,
            dlss,
        }
    }

    pub fn is_managed_key(key: &str) -> bool {
        matches!(
            key,
            "NEUTRON_USE_PROTON"
                | "PROTON_VERSION"
                | "NEUTRON_USE_GAMEMODE"
                | "NEUTRON_GAMEMODE"
                | "GAMEMODERUN"
                | "NEUTRON_USE_GAMESCOPE"
                | "NEUTRON_GAMESCOPE"
                | "MANGOHUD"
                | "MANGOHUD_CONFIG"
                | "NEUTRON_DISABLE_SMT"
                | "NEUTRON_SMT"
                | "WINE_CPU_TOPOLOGY"
                | "NEUTRON_RESERVE_VRAM"
                | "NEUTRON_SHADER_CACHE_SIZE"
                | "NEUTRON_LAUNCHER_TIMEOUT"
                | "__GL_SHADER_DISK_CACHE"
                | "__GL_SHADER_DISK_CACHE_SKIP_CLEANUP"
                | "__GL_SHADER_DISK_CACHE_SIZE"
                | "MESA_SHADER_CACHE_MAX_SIZE"
                | "MESA_SHADER_CACHE_DISABLE"
                | "PROTON_DLSS_UPGRADE"
                | "PROTON_DLSS_INDICATOR"
                | "PROTON_ENABLE_NGX_UPDATER"
                | "DXVK_NVAPI_SET_NGX_DEBUG_OPTIONS"
                | "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE"
                | "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION"
                | "DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE"
                | "DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE_RENDER_PRESET_SELECTION"
                | "DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE"
                | "DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE_RENDER_PRESET_SELECTION"
        ) || key.starts_with("NEUTRON_")
    }

    pub fn sync_from_entries(&mut self, entries: &[EnvEntry]) {
        for entry in entries {
            if !entry.enabled {
                continue;
            }
            let key = entry.key.trim();
            let val = entry.value.trim();
            if key.is_empty() {
                continue;
            }

            match key {
                "PROTON_DLSS_UPGRADE" => {
                    let dlss = self.dlss.get_or_insert_with(DlssSettings::default);
                    dlss.upgrade = val == "1";
                }
                "DXVK_NVAPI_SET_NGX_DEBUG_OPTIONS" => {
                    let dlss = self.dlss.get_or_insert_with(DlssSettings::default);
                    dlss.indicator = val.contains("DLSSIndicator=1024");
                }
                "PROTON_DLSS_INDICATOR" => {
                    let dlss = self.dlss.get_or_insert_with(DlssSettings::default);
                    dlss.indicator = val == "1";
                }
                "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION" => {
                    let dlss = self.dlss.get_or_insert_with(DlssSettings::default);
                    dlss.super_resolution = val.to_string();
                }
                "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE" => {
                    if val == "off" || val == "0" {
                        if let Some(dlss) = self.dlss.as_mut() {
                            dlss.super_resolution = String::new();
                        }
                    }
                }
                "DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE_RENDER_PRESET_SELECTION" => {
                    let dlss = self.dlss.get_or_insert_with(DlssSettings::default);
                    dlss.ray_reconstruction = val.to_string();
                }
                "DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE" => {
                    if val == "off" || val == "0" {
                        if let Some(dlss) = self.dlss.as_mut() {
                            dlss.ray_reconstruction = String::new();
                        }
                    }
                }
                "DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE_RENDER_PRESET_SELECTION" => {
                    let dlss = self.dlss.get_or_insert_with(DlssSettings::default);
                    dlss.frame_generation = val.to_string();
                }
                "DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE" => {
                    if val == "off" || val == "0" {
                        if let Some(dlss) = self.dlss.as_mut() {
                            dlss.frame_generation = String::new();
                        }
                    }
                }
                "MANGOHUD" => {
                    let extra = self.mangohud.extra_args().to_string();
                    self.mangohud = if val == "1" {
                        FeatureState::Enabled(extra)
                    } else {
                        FeatureState::Disabled
                    };
                }
                "MANGOHUD_CONFIG" => {
                    self.mangohud = FeatureState::Enabled(val.to_string());
                }
                "GAMEMODERUN" | "NEUTRON_USE_GAMEMODE" => {
                    let extra = self.gamemode.extra_args().to_string();
                    self.gamemode = if val == "1" {
                        FeatureState::Enabled(extra)
                    } else {
                        FeatureState::Disabled
                    };
                }
                "NEUTRON_GAMEMODE" => {
                    self.gamemode = FeatureState::Enabled(val.to_string());
                }
                "NEUTRON_USE_GAMESCOPE" => {
                    let extra = self.gamescope.extra_args().to_string();
                    self.gamescope = if val == "1" {
                        FeatureState::Enabled(extra)
                    } else {
                        FeatureState::Disabled
                    };
                }
                "NEUTRON_GAMESCOPE" => {
                    self.gamescope = FeatureState::Enabled(val.to_string());
                }
                "NEUTRON_USE_PROTON" | "PROTON_VERSION" => {
                    self.proton_version = Some(val.to_string());
                }
                "NEUTRON_RESERVE_VRAM" => {
                    let digits: String = val.chars().filter(|c| c.is_ascii_digit()).collect();
                    self.reserve_vram = if digits.is_empty() || digits == "0" {
                        FeatureState::Disabled
                    } else {
                        FeatureState::Enabled(digits)
                    };
                }
                "NEUTRON_SHADER_CACHE_SIZE" => {
                    let digits: String = val.chars().filter(|c| c.is_ascii_digit()).collect();
                    self.shader_cache_size = if digits.is_empty() || digits == "0" {
                        FeatureState::Disabled
                    } else {
                        FeatureState::Enabled(digits)
                    };
                }
                "NEUTRON_LAUNCHER_TIMEOUT" => {
                    self.launcher_timeout = Some(val.to_string());
                }
                "NEUTRON_DISABLE_SMT" => {
                    self.disable_smt = if val == "0" {
                        FeatureState::Disabled
                    } else {
                        FeatureState::Enabled(String::new())
                    };
                }
                "NEUTRON_SMT" => {
                    self.disable_smt = if val == "0" {
                        FeatureState::Enabled(String::new())
                    } else {
                        FeatureState::Disabled
                    };
                }
                "WINE_CPU_TOPOLOGY" => {
                    let trimmed = val.trim();
                    self.disable_smt = if trimmed.is_empty() || trimmed == "0" {
                        FeatureState::Disabled
                    } else {
                        FeatureState::Enabled(String::new())
                    };
                }
                _ => {}
            }
        }
    }

    pub fn apply_to_entries(&self, entries: &mut Vec<EnvEntry>, is_global: bool) {
        let remove_keys = |entries: &mut Vec<EnvEntry>, keys: &[&str]| {
            entries.retain(|e| !keys.contains(&e.key.as_str()));
        };

        let upsert = |entries: &mut Vec<EnvEntry>, key: &str, value: &str| {
            if let Some(existing) = entries.iter_mut().find(|e| e.key == key) {
                existing.value = value.to_string();
                existing.enabled = true;
            } else {
                entries.push(EnvEntry {
                    key: key.to_string(),
                    value: value.to_string(),
                    enabled: true,
                    comment_above: None,
                });
            }
        };

        // 0. Launcher timeout (global only)
        if is_global {
            remove_keys(entries, &["NEUTRON_LAUNCHER_TIMEOUT"]);
            let val = self.launcher_timeout.as_deref().unwrap_or("2").trim();
            let val = if val.is_empty() { "2" } else { val };
            upsert(entries, "NEUTRON_LAUNCHER_TIMEOUT", val);
        }

        // 1. Proton version
        remove_keys(entries, &["NEUTRON_USE_PROTON", "PROTON_VERSION"]);
        if is_global {
            let ver = self.proton_version.as_deref().unwrap_or("proton-cachyos-slr");
            upsert(entries, "NEUTRON_USE_PROTON", ver);
        } else if let Some(ver) = &self.proton_version {
            upsert(entries, "NEUTRON_USE_PROTON", ver);
        }

        // 1. Gamemode
        remove_keys(entries, &["NEUTRON_USE_GAMEMODE", "NEUTRON_GAMEMODE", "GAMEMODERUN"]);
        match &self.gamemode {
            FeatureState::Inherit => {}
            FeatureState::Disabled => {
                upsert(entries, "NEUTRON_USE_GAMEMODE", "0");
            }
            FeatureState::Enabled(args) => {
                let val = if args.trim().is_empty() { "1" } else { args.trim() };
                upsert(entries, "NEUTRON_USE_GAMEMODE", val);
            }
        }

        // 2. Gamescope
        remove_keys(entries, &["NEUTRON_USE_GAMESCOPE", "NEUTRON_GAMESCOPE"]);
        match &self.gamescope {
            FeatureState::Inherit => {}
            FeatureState::Disabled => {
                upsert(entries, "NEUTRON_USE_GAMESCOPE", "0");
            }
            FeatureState::Enabled(args) => {
                let val = if args.trim().is_empty() { "1" } else { args.trim() };
                upsert(entries, "NEUTRON_USE_GAMESCOPE", val);
            }
        }

        // 3. MangoHud
        remove_keys(entries, &["MANGOHUD", "MANGOHUD_CONFIG"]);
        match &self.mangohud {
            FeatureState::Inherit => {}
            FeatureState::Disabled => {
                upsert(entries, "MANGOHUD", "0");
            }
            FeatureState::Enabled(cfg) => {
                upsert(entries, "MANGOHUD", "1");
                if !cfg.trim().is_empty() {
                    upsert(entries, "MANGOHUD_CONFIG", cfg.trim());
                }
            }
        }

        // Disable SMT
        remove_keys(entries, &["NEUTRON_DISABLE_SMT", "NEUTRON_SMT", "WINE_CPU_TOPOLOGY"]);
        match &self.disable_smt {
            FeatureState::Inherit => {}
            FeatureState::Disabled => {
                upsert(entries, "NEUTRON_DISABLE_SMT", "0");
            }
            FeatureState::Enabled(_) => {
                upsert(entries, "NEUTRON_DISABLE_SMT", "1");
            }
        }

        // 4. Reserve VRAM
        remove_keys(entries, &["NEUTRON_RESERVE_VRAM"]);
        match &self.reserve_vram {
            FeatureState::Inherit => {}
            FeatureState::Disabled => {
                if is_global {
                    upsert(entries, "NEUTRON_RESERVE_VRAM", "0");
                }
            }
            FeatureState::Enabled(mb) => {
                let val = if mb.trim().is_empty() { "256" } else { mb.trim() };
                upsert(entries, "NEUTRON_RESERVE_VRAM", val);
            }
        }

        // 5. Shader cache size
        remove_keys(
            entries,
            &[
                "NEUTRON_SHADER_CACHE_SIZE",
                "__GL_SHADER_DISK_CACHE",
                "__GL_SHADER_DISK_CACHE_SKIP_CLEANUP",
                "__GL_SHADER_DISK_CACHE_SIZE",
                "MESA_SHADER_CACHE_MAX_SIZE",
                "MESA_SHADER_CACHE_DISABLE",
            ],
        );
        match &self.shader_cache_size {
            FeatureState::Inherit => {}
            FeatureState::Disabled => {
                if is_global {
                    upsert(entries, "NEUTRON_SHADER_CACHE_SIZE", "0");
                }
            }
            FeatureState::Enabled(mb) => {
                let val = if mb.trim().is_empty() { "10240" } else { mb.trim() };
                upsert(entries, "NEUTRON_SHADER_CACHE_SIZE", val);
            }
        }

        // 6. DLSS
        remove_keys(
            entries,
            &[
                "PROTON_DLSS_UPGRADE",
                "PROTON_ENABLE_NGX_UPDATER",
                "DXVK_NVAPI_SET_NGX_DEBUG_OPTIONS",
                "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE",
                "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION",
                "DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE",
                "DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE_RENDER_PRESET_SELECTION",
                "DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE",
                "DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE_RENDER_PRESET_SELECTION",
            ],
        );
        if let Some(dlss) = &self.dlss {
            if dlss.upgrade {
                upsert(entries, "PROTON_DLSS_UPGRADE", "1");
            }
            if dlss.indicator {
                upsert(
                    entries,
                    "DXVK_NVAPI_SET_NGX_DEBUG_OPTIONS",
                    "DLSSIndicator=1024,DLSSGIndicator=2",
                );
            }
            if !dlss.super_resolution.is_empty() {
                upsert(entries, "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE", "on");
                upsert(
                    entries,
                    "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION",
                    &dlss.super_resolution,
                );
            }
            if !dlss.ray_reconstruction.is_empty() {
                upsert(entries, "DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE", "on");
                upsert(
                    entries,
                    "DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE_RENDER_PRESET_SELECTION",
                    &dlss.ray_reconstruction,
                );
            }
            if !dlss.frame_generation.is_empty() {
                upsert(entries, "DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE", "on");
                upsert(
                    entries,
                    "DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE_RENDER_PRESET_SELECTION",
                    &dlss.frame_generation,
                );
            }
        }
    }
}

pub fn compute_default_gamescope_args() -> String {
    let mut args = Vec::new();

    let mut mode: Option<(i32, i32, Option<i32>)> = None;

    if let Some(display) = gtk4::gdk::Display::default() {
        let monitors = display.monitors();
        if let Some(first_monitor) = monitors
            .item(0)
            .and_then(|obj| obj.downcast::<gtk4::gdk::Monitor>().ok())
        {
            let geom = first_monitor.geometry();
            let rate = first_monitor.refresh_rate();
            let hz = if rate > 0 {
                Some(((rate as f64) / 1000.0).round() as i32)
            } else {
                None
            };
            mode = Some((geom.width(), geom.height(), hz));
        }
    }

    if mode.is_none() {
        mode = detect_display_mode_fallback();
    }

    if let Some((w, h, r)) = mode {
        args.push(format!("-W {}", w));
        args.push(format!("-H {}", h));
        args.push(format!("-w {}", w));
        args.push(format!("-h {}", h));
        if let Some(rate) = r {
            if rate > 0 {
                args.push(format!("-r {}", rate));
            }
        }
    }

    args.push("-f".to_string());

    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        args.push("--backend wayland".to_string());
    }

    if std::env::var("DXVK_HDR").map(|v| v == "1").unwrap_or(false) {
        args.push("--hdr-enabled".to_string());
    }

    args.push("--adaptive-sync".to_string());

    args.join(" ")
}

fn detect_display_mode_fallback() -> Option<(i32, i32, Option<i32>)> {
    if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let path = entry.path();
            let status = path.join("status");
            let modes = path.join("modes");
            if status.is_file() && modes.is_file() {
                if let Ok(status_str) = std::fs::read_to_string(&status) {
                    if status_str.trim() == "connected" {
                        if let Ok(modes_str) = std::fs::read_to_string(&modes) {
                            if let Some(first_mode) = modes_str.lines().next() {
                                let parts: Vec<&str> = first_mode.trim().split('x').collect();
                                if parts.len() == 2 {
                                    if let (Ok(w), Ok(h)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
                                        return Some((w, h, None));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct GameConfig {
    pub id: String, // "global" or Steam App ID
    pub title: String,
    pub path: PathBuf,
    pub icon_path: Option<PathBuf>,
    pub entries: Vec<EnvEntry>,
}

impl GameConfig {
    pub fn load_from_file(path: &Path) -> Result<Self, std::io::Error> {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let id = file_name.strip_suffix(".env").unwrap_or(file_name).to_string();

        let content = fs::read_to_string(path)?;
        let mut title = if id == "global" {
            "Global".to_string()
        } else {
            format!("Game {}", id)
        };

        let mut entries = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('#') {
                let comment_text = trimmed.trim_start_matches('#').trim();
                // 1. Standard format: "# <Game Title> overrides ======"
                let clean = comment_text.trim_end_matches('=').trim();
                if let Some(t) = clean.strip_suffix(" overrides") {
                    if id != "global" && title == format!("Game {}", id) {
                        let t_trim = t.trim();
                        if !t_trim.is_empty() {
                            title = t_trim.to_string();
                        }
                    }
                    continue;
                }
                // 2. Legacy format: "# Game-specific overrides for <Game Title> (App ID: ...)"
                if comment_text.starts_with("Game-specific overrides for ") {
                    if let Some(rest) = comment_text.strip_prefix("Game-specific overrides for ") {
                        if let Some((name, _)) = rest.split_once(" (App ID:") {
                            title = name.trim().to_string();
                        }
                    }
                    continue;
                }
                // 3. Disabled custom envvar: "#<key>=<value>"
                if let Some(disabled_entry) = parse_disabled_env_line(trimmed) {
                    entries.push(disabled_entry);
                }
                continue;
            }

            let var_line = trimmed.strip_prefix("export ").unwrap_or(trimmed).trim();
            if let Some((key, val)) = var_line.split_once('=') {
                let key = key.trim().to_string();
                if key.is_empty() {
                    continue;
                }
                let value = parse_env_value(val);

                entries.push(EnvEntry {
                    key,
                    value,
                    enabled: true,
                    comment_above: None,
                });
            }
        }

        if id != "global" && title == format!("Game {}", id) {
            if let Some(steam_name) = find_steam_game_name(&id) {
                title = steam_name;
            }
        }

        let icon_path = if id != "global" {
            find_steam_game_icon(&id)
        } else {
            None
        };

        Ok(GameConfig {
            id,
            title,
            path: path.to_path_buf(),
            icon_path,
            entries,
        })
    }

    pub fn format_content(&self) -> String {
        let mut out = String::new();

        // 1. Comment with game name, padded with '=' up to column 60
        let top_prefix = if self.id == "global" {
            "# Global configuration".to_string()
        } else {
            format!("# {} overrides", self.title)
        };
        let top_len = top_prefix.chars().count();
        if top_len + 1 < 60 {
            out.push_str(&format!("{} {}\n", top_prefix, "=".repeat(60 - top_len - 1)));
        } else {
            out.push_str(&format!("{} ======\n", top_prefix));
        }

        let get_val = |keys: &[&str]| -> Option<String> {
            self.entries
                .iter()
                .rev()
                .find(|e| e.enabled && keys.contains(&e.key.as_str()))
                .map(|e| e.value.clone())
        };

        let format_section_header = |name: &str| -> String {
            let prefix = format!("# {}", name);
            let cur_len = prefix.chars().count();
            if cur_len + 1 < 40 {
                format!("{} {}\n", prefix, "-".repeat(40 - cur_len - 1))
            } else {
                format!("{} ---\n", prefix)
            }
        };

        let format_env_line = |key: &str, val: &str| -> String {
            if val.is_empty() {
                format!("{}=\"\"\n", key)
            } else if val.contains(' ') || val.contains('\t') || val.contains('=') {
                if !val.contains('"') {
                    format!("{}=\"{}\"\n", key, val)
                } else if !val.contains('\'') {
                    format!("{}='{}'\n", key, val)
                } else {
                    format!("{}=\"{}\"\n", key, val.replace('"', "\\\""))
                }
            } else {
                format!("{}={}\n", key, val)
            }
        };

        let mut sections: Vec<(String, Vec<String>)> = Vec::new();

        // Section 1: Neutron configs (Timeout, Proton, GameMode, MangoHud, GameScope)
        let mut neutron_lines = Vec::new();
        if let Some(v) = get_val(&["NEUTRON_LAUNCHER_TIMEOUT"]) {
            neutron_lines.push(format_env_line("NEUTRON_LAUNCHER_TIMEOUT", &v));
        }
        if let Some(v) = get_val(&["NEUTRON_USE_PROTON", "PROTON_VERSION"]) {
            neutron_lines.push(format_env_line("NEUTRON_USE_PROTON", &v));
        }
        if let Some(v) = get_val(&["NEUTRON_USE_GAMEMODE", "NEUTRON_GAMEMODE", "GAMEMODERUN"]) {
            neutron_lines.push(format_env_line("NEUTRON_USE_GAMEMODE", &v));
        }
        if let Some(v) = get_val(&["MANGOHUD"]) {
            neutron_lines.push(format_env_line("MANGOHUD", &v));
            if let Some(cfg) = get_val(&["MANGOHUD_CONFIG"]) {
                if !cfg.trim().is_empty() {
                    neutron_lines.push(format_env_line("MANGOHUD_CONFIG", &cfg));
                }
            }
        } else if let Some(cfg) = get_val(&["MANGOHUD_CONFIG"]) {
            if !cfg.trim().is_empty() {
                neutron_lines.push(format_env_line("MANGOHUD_CONFIG", &cfg));
            }
        }
        if let Some(v) = get_val(&["NEUTRON_USE_GAMESCOPE", "NEUTRON_GAMESCOPE"]) {
            neutron_lines.push(format_env_line("NEUTRON_USE_GAMESCOPE", &v));
        }
        if let Some(v) = get_val(&["NEUTRON_DISABLE_SMT", "NEUTRON_SMT"]) {
            neutron_lines.push(format_env_line("NEUTRON_DISABLE_SMT", &v));
        }
        if !neutron_lines.is_empty() {
            sections.push(("Neutron configs".to_string(), neutron_lines));
        }

        // Section 2: Managed config: Reserve VRAM headroom
        if let Some(v) = get_val(&["NEUTRON_RESERVE_VRAM"]) {
            sections.push((
                "Reserve VRAM headroom".to_string(),
                vec![format_env_line("NEUTRON_RESERVE_VRAM", &v)],
            ));
        }

        // Section 3: Managed config: Shader cache size
        if let Some(v) = get_val(&[
            "NEUTRON_SHADER_CACHE_SIZE",
            "__GL_SHADER_DISK_CACHE_SIZE",
            "MESA_SHADER_CACHE_MAX_SIZE",
        ]) {
            sections.push((
                "Shader cache size".to_string(),
                vec![format_env_line("NEUTRON_SHADER_CACHE_SIZE", &v)],
            ));
        }

        // Section: Managed config: DLSS overrides
        let mut dlss_lines = Vec::new();
        if let Some(v) = get_val(&["PROTON_DLSS_UPGRADE"]) {
            if v == "1" {
                dlss_lines.push(format_env_line("PROTON_DLSS_UPGRADE", "1"));
            }
        }
        if let Some(v) = get_val(&["DXVK_NVAPI_SET_NGX_DEBUG_OPTIONS"]) {
            if !v.trim().is_empty() {
                dlss_lines.push(format_env_line("DXVK_NVAPI_SET_NGX_DEBUG_OPTIONS", &v));
            }
        }
        if let Some(v) = get_val(&["DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION"]) {
            if !v.trim().is_empty() {
                dlss_lines.push(format_env_line("DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE", "on"));
                dlss_lines.push(format_env_line(
                    "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION",
                    &v,
                ));
            }
        }
        if let Some(v) = get_val(&["DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE_RENDER_PRESET_SELECTION"]) {
            if !v.trim().is_empty() {
                dlss_lines.push(format_env_line("DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE", "on"));
                dlss_lines.push(format_env_line(
                    "DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE_RENDER_PRESET_SELECTION",
                    &v,
                ));
            }
        }
        if let Some(v) = get_val(&["DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE_RENDER_PRESET_SELECTION"]) {
            if !v.trim().is_empty() {
                dlss_lines.push(format_env_line("DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE", "on"));
                dlss_lines.push(format_env_line(
                    "DXVK_NVAPI_DRS_NGX_DLSS_FG_OVERRIDE_RENDER_PRESET_SELECTION",
                    &v,
                ));
            }
        }
        if !dlss_lines.is_empty() {
            sections.push(("DLSS overrides".to_string(), dlss_lines));
        }

        // Section 4: Custom envvars
        let mut custom_lines = Vec::new();
        for entry in &self.entries {
            let key = entry.key.trim();
            if key.is_empty() || ManagedFeatures::is_managed_key(key) {
                continue;
            }
            let line = format_env_line(key, &entry.value);
            if entry.enabled {
                custom_lines.push(line);
            } else {
                custom_lines.push(format!("#{}", line));
            }
        }
        if !custom_lines.is_empty() {
            sections.push(("Custom envvars".to_string(), custom_lines));
        }

        for (section_name, lines) in sections {
            out.push('\n');
            out.push_str(&format_section_header(&section_name));
            for line in lines {
                out.push_str(&line);
            }
        }

        out
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let out = self.format_content();
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, out)
    }
}

pub fn get_config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("neutron");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("neutron");
    }
    PathBuf::from(".config/neutron")
}

pub fn load_all_configs() -> Vec<GameConfig> {
    let config_dir = get_config_dir();
    let mut configs = Vec::new();

    let global_path = config_dir.join("global.env");
    if global_path.is_file() {
        if let Ok(cfg) = GameConfig::load_from_file(&global_path) {
            configs.push(cfg);
        }
    } else {
        configs.push(GameConfig {
            id: "global".to_string(),
            title: "Global".to_string(),
            path: global_path,
            icon_path: None,
            entries: Vec::new(),
        });
    }

    if let Ok(read_dir) = fs::read_dir(&config_dir) {
        let mut game_paths = Vec::new();
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("env") {
                let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or_default();
                if file_name != "global.env" {
                    game_paths.push(path);
                }
            }
        }
        game_paths.sort();
        for path in game_paths {
            if let Ok(cfg) = GameConfig::load_from_file(&path) {
                configs.push(cfg);
            }
        }
    }

    configs
}

pub fn find_steam_game_icon(app_id: &str) -> Option<PathBuf> {
    let trimmed = app_id.trim();
    if trimmed.is_empty() || trimmed == "global" {
        return None;
    }

    let home = std::env::var("HOME").ok()?;
    let mut roots = vec![
        PathBuf::from(&home).join(".local/share/Steam"),
        PathBuf::from(&home).join(".steam/steam"),
        PathBuf::from(&home).join(".steam/root"),
    ];

    if let Ok(client_path) = std::env::var("STEAM_COMPAT_CLIENT_INSTALL_PATH") {
        if !client_path.is_empty() {
            roots.insert(0, PathBuf::from(client_path));
        }
    }
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        if !xdg_data.is_empty() {
            roots.push(PathBuf::from(xdg_data).join("Steam"));
        }
    }

    for root in roots {
        if !root.is_dir() {
            continue;
        }

        let cache_dir = root.join("appcache/librarycache").join(app_id);
        if cache_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&cache_dir) {
                let mut candidates = Vec::new();
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or_default()
                        .to_lowercase();
                    if ext != "jpg" && ext != "jpeg" && ext != "png" {
                        continue;
                    }

                    if name.starts_with("header")
                        || name.starts_with("library_hero")
                        || name.starts_with("library_600x900")
                        || name.starts_with("library_header")
                    {
                        continue;
                    }

                    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
                    let is_hex_hash = stem.len() >= 32 && stem.chars().all(|c| c.is_ascii_hexdigit());
                    let has_icon_keyword = stem.to_lowercase().contains("icon") || stem.to_lowercase().contains("logo");

                    if is_hex_hash || has_icon_keyword {
                        candidates.insert(0, path);
                    } else {
                        candidates.push(path);
                    }
                }

                if let Some(best) = candidates.into_iter().next() {
                    return Some(best);
                }
            }
        }
    }

    None
}

pub fn find_steam_game_header(app_id: &str) -> Option<PathBuf> {
    let trimmed = app_id.trim();
    if trimmed.is_empty() || trimmed == "global" {
        return None;
    }

    let home = std::env::var("HOME").ok()?;
    let mut roots = vec![
        PathBuf::from(&home).join(".local/share/Steam"),
        PathBuf::from(&home).join(".steam/steam"),
        PathBuf::from(&home).join(".steam/root"),
    ];

    if let Ok(client_path) = std::env::var("STEAM_COMPAT_CLIENT_INSTALL_PATH") {
        if !client_path.is_empty() {
            roots.insert(0, PathBuf::from(client_path));
        }
    }
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        if !xdg_data.is_empty() {
            roots.push(PathBuf::from(xdg_data).join("Steam"));
        }
    }

    let target_names = ["library_header.jpg", "library_header.png", "header.jpg", "header.png"];

    for root in roots {
        if !root.is_dir() {
            continue;
        }

        let cache_dir = root.join("appcache/librarycache").join(app_id);
        if !cache_dir.is_dir() {
            continue;
        }

        for target in &target_names {
            let direct = cache_dir.join(target);
            if direct.is_file() {
                return Some(direct);
            }

            if let Ok(entries) = fs::read_dir(&cache_dir) {
                for entry in entries.flatten() {
                    let sub = entry.path();
                    if sub.is_dir() {
                        let cand = sub.join(target);
                        if cand.is_file() {
                            return Some(cand);
                        }
                    }
                }
            }
        }
    }

    None
}

pub fn get_launcher_timeout() -> Duration {
    let config_dir = get_config_dir();
    let global_path = config_dir.join("global.env");
    if let Ok(content) = fs::read_to_string(&global_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == "NEUTRON_LAUNCHER_TIMEOUT" {
                    let val = v.trim().trim_matches(|c| c == '\'' || c == '"');
                    if let Ok(secs) = val.parse::<u64>() {
                        return Duration::from_secs(secs);
                    }
                    if let Ok(secs_f) = val.parse::<f64>() {
                        return Duration::from_secs_f64(secs_f);
                    }
                }
            }
        }
    }
    Duration::from_secs(2)
}

pub fn discover_proton_versions() -> Vec<String> {
    let mut versions = std::collections::BTreeSet::new();

    let home = std::env::var("HOME").ok().unwrap_or_default();
    let mut roots = vec![
        PathBuf::from(&home).join(".local/share/Steam"),
        PathBuf::from(&home).join(".steam/steam"),
        PathBuf::from(&home).join(".steam/root"),
        PathBuf::from("/usr/share/steam"),
        PathBuf::from("/usr/local/share/steam"),
    ];

    if let Ok(client_path) = std::env::var("STEAM_COMPAT_CLIENT_INSTALL_PATH") {
        if !client_path.is_empty() {
            roots.insert(0, PathBuf::from(client_path));
        }
    }
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        if !xdg_data.is_empty() {
            roots.push(PathBuf::from(xdg_data).join("Steam"));
        }
    }

    for root in roots {
        // 1. compatibilitytools.d
        let comp_dir = root.join("compatibilitytools.d");
        if comp_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&comp_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() && path.join("proton").is_file() {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            versions.insert(name.to_string());
                        }
                    }
                }
            }
        }

        // 2. steamapps/common/
        let common_dir = root.join("steamapps/common");
        if common_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&common_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() && path.join("proton").is_file() {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if name.to_lowercase().contains("proton") {
                                versions.insert(name.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    if versions.is_empty() {
        versions.insert("proton-cachyos-slr".to_string());
        versions.insert("Proton - Experimental".to_string());
    }

    let mut list: Vec<String> = versions.into_iter().collect();
    list.sort_by(|a, b| {
        let score = |s: &str| {
            if s == "proton-cachyos-slr" {
                0
            } else if s == "proton-cachyos-native" {
                1
            } else if s == "Proton - Experimental" {
                2
            } else if s.starts_with("GE-Proton") {
                3
            } else {
                4
            }
        };
        score(a).cmp(&score(b)).then_with(|| a.cmp(b))
    });

    list
}

pub fn find_steam_game_name(app_id: &str) -> Option<String> {
    let trimmed = app_id.trim();
    if trimmed.is_empty() || trimmed == "global" {
        return None;
    }
    let home = std::env::var("HOME").ok()?;
    let roots = [
        PathBuf::from(&home).join(".local/share/Steam"),
        PathBuf::from(&home).join(".steam/steam"),
        PathBuf::from(&home).join(".steam/root"),
    ];

    let manifest_name = format!("appmanifest_{}.acf", app_id);

    for root in roots {
        let candidate = root.join("steamapps").join(&manifest_name);
        if candidate.is_file() {
            if let Ok(content) = fs::read_to_string(&candidate) {
                if let Some(name) = parse_vdf_key(&content, "name") {
                    return Some(name);
                }
            }
        }

        let lib_vdf = root.join("steamapps/libraryfolders.vdf");
        if lib_vdf.is_file() {
            if let Ok(content) = fs::read_to_string(&lib_vdf) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("\"path\"") {
                        if let Some((_, path_val)) = trimmed.split_once("\"path\"") {
                            let p = path_val.trim().trim_matches('"');
                            let lib_manifest = PathBuf::from(p).join("steamapps").join(&manifest_name);
                            if lib_manifest.is_file() {
                                if let Ok(mc) = fs::read_to_string(&lib_manifest) {
                                    if let Some(name) = parse_vdf_key(&mc, "name") {
                                        return Some(name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

pub fn parse_vdf_key(vdf: &str, key: &str) -> Option<String> {
    let search = format!("\"{}\"", key);
    for line in vdf.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&search) {
            let rest = trimmed.strip_prefix(&search)?.trim();
            let val = rest.trim_matches('"');
            return Some(val.to_string());
        }
    }
    None
}

pub fn is_command_available(cmd: &str) -> bool {
    if let Ok(path) = std::env::var("PATH") {
        for p in path.split(':') {
            let full = Path::new(p).join(cmd);
            if full.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = full.metadata() {
                        if meta.permissions().mode() & 0o111 != 0 {
                            return true;
                        }
                    }
                }
                #[cfg(not(unix))]
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_vdf_key() {
        let vdf = r#"
"AppState"
{
    "appid" "3669870"
    "name" "CONTROL Resonant"
}
"#;
        assert_eq!(parse_vdf_key(vdf, "name"), Some("CONTROL Resonant".to_string()));
        assert_eq!(parse_vdf_key(vdf, "appid"), Some("3669870".to_string()));
        assert_eq!(parse_vdf_key(vdf, "missing"), None);
    }

    #[test]
    fn test_managed_features_extract_and_apply() {
        let mut entries = vec![
            EnvEntry {
                key: "NEUTRON_USE_PROTON".into(),
                value: "proton-cachyos-slr".into(),
                enabled: true,
                comment_above: None,
            },
            EnvEntry {
                key: "MANGOHUD".into(),
                value: "1".into(),
                enabled: true,
                comment_above: None,
            },
            EnvEntry {
                key: "NEUTRON_RESERVE_VRAM".into(),
                value: "512".into(),
                enabled: true,
                comment_above: None,
            },
            EnvEntry {
                key: "NEUTRON_SHADER_CACHE_SIZE".into(),
                value: "10240".into(),
                enabled: true,
                comment_above: None,
            },
            EnvEntry {
                key: "NEUTRON_LAUNCHER_TIMEOUT".into(),
                value: "3".into(),
                enabled: true,
                comment_above: None,
            },
            EnvEntry {
                key: "DXVK_HDR".into(),
                value: "1".into(),
                enabled: true,
                comment_above: None,
            },
        ];

        let mut mf = ManagedFeatures::extract(&entries, true);
        assert_eq!(mf.proton_version, Some("proton-cachyos-slr".into()));
        assert_eq!(mf.mangohud, FeatureState::Enabled("".into()));
        assert_eq!(mf.reserve_vram, FeatureState::Enabled("512".into()));
        assert_eq!(mf.shader_cache_size, FeatureState::Enabled("10240".into()));
        assert_eq!(mf.launcher_timeout, Some("3".into()));
        assert_eq!(mf.gamescope, FeatureState::Disabled);
        assert_eq!(mf.disable_smt, FeatureState::Disabled);

        mf.proton_version = Some("Proton - Experimental".into());
        mf.gamescope = FeatureState::Enabled("-F fsr".into());
        mf.mangohud = FeatureState::Disabled;
        mf.disable_smt = FeatureState::Enabled("".into());
        mf.shader_cache_size = FeatureState::Disabled;
        mf.launcher_timeout = Some("5".into());
        mf.apply_to_entries(&mut entries, true);

        let re_extracted = ManagedFeatures::extract(&entries, true);
        assert_eq!(re_extracted.proton_version, Some("Proton - Experimental".into()));
        assert_eq!(re_extracted.gamescope, FeatureState::Enabled("-F fsr".into()));
        assert_eq!(re_extracted.mangohud, FeatureState::Disabled);
        assert_eq!(re_extracted.disable_smt, FeatureState::Enabled("".into()));
        assert_eq!(re_extracted.reserve_vram, FeatureState::Enabled("512".into()));
        assert_eq!(re_extracted.shader_cache_size, FeatureState::Disabled);
        assert_eq!(re_extracted.launcher_timeout, Some("5".into()));

        assert!(entries.iter().any(|e| e.key == "NEUTRON_DISABLE_SMT" && e.value == "1"));
        assert!(entries.iter().any(|e| e.key == "DXVK_HDR" && e.value == "1"));
    }

    #[test]
    fn test_is_command_available() {
        assert!(is_command_available("sh"));
        assert!(!is_command_available("definitely_nonexistent_binary_xyz_123"));
    }

    #[test]
    fn test_compute_default_gamescope_args() {
        let args = compute_default_gamescope_args();
        assert!(args.contains("-f"));
        assert!(args.contains("--adaptive-sync"));
    }

    #[test]
    fn test_compute_effective_envvars() {
        let temp_dir = std::env::temp_dir().join(format!("neutron_test_effective_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        std::fs::write(
            temp_dir.join("global.env"),
            "NEUTRON_USE_GAMEMODE=1\nPROTON_ENABLE_WAYLAND=1\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("3669870.env"),
            "NEUTRON_USE_PROTON=Proton - Experimental\nDXVK_HDR=1\nVKD3D_CONFIG=descriptor_heap\n",
        )
        .unwrap();

        let envs = compute_effective_envvars_from_dir(&temp_dir, "3669870");
        // Should contain custom overrides from global.env and 3669870.env
        assert!(envs.iter().any(|(k, v)| k == "PROTON_ENABLE_WAYLAND" && v == "1"));
        assert!(envs.iter().any(|(k, v)| k == "DXVK_HDR" && v == "1"));
        assert!(envs.iter().any(|(k, v)| k == "VKD3D_CONFIG" && v == "descriptor_heap"));
        // Should NOT contain NEUTRON_* internal keys
        assert!(!envs.iter().any(|(k, _)| k.starts_with("NEUTRON_")));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_config_format_content_global() {
        let cfg = GameConfig {
            id: "global".to_string(),
            title: "Global".to_string(),
            path: PathBuf::from("/dummy/global.env"),
            icon_path: None,
            entries: vec![
                EnvEntry {
                    key: "NEUTRON_LAUNCHER_TIMEOUT".into(),
                    value: "2".into(),
                    enabled: true,
                    comment_above: None,
                },
                EnvEntry {
                    key: "NEUTRON_USE_PROTON".into(),
                    value: "proton-cachyos-slr".into(),
                    enabled: true,
                    comment_above: None,
                },
                EnvEntry {
                    key: "NEUTRON_USE_GAMEMODE".into(),
                    value: "1".into(),
                    enabled: true,
                    comment_above: None,
                },
                EnvEntry {
                    key: "MANGOHUD".into(),
                    value: "0".into(),
                    enabled: true,
                    comment_above: None,
                },
                EnvEntry {
                    key: "NEUTRON_USE_GAMESCOPE".into(),
                    value: "0".into(),
                    enabled: true,
                    comment_above: None,
                },
                EnvEntry {
                    key: "NEUTRON_RESERVE_VRAM".into(),
                    value: "256".into(),
                    enabled: true,
                    comment_above: None,
                },
                EnvEntry {
                    key: "NEUTRON_SHADER_CACHE_SIZE".into(),
                    value: "10240".into(),
                    enabled: true,
                    comment_above: None,
                },
            ],
        };

        let formatted = cfg.format_content();
        let lines: Vec<&str> = formatted.lines().collect();

        // 1. Top comment padded to column 60
        assert_eq!(lines[0].len(), 60);
        assert!(lines[0].starts_with("# Global configuration ==="));
        assert!(lines[0].ends_with('='));

        // 2. Empty line between sections
        assert_eq!(lines[1], "");

        // 3. Neutron configs section header padded to column 40
        assert_eq!(lines[2].len(), 40);
        assert!(lines[2].starts_with("# Neutron configs ---"));
        assert!(lines[2].ends_with('-'));

        // Check ordering of neutron configs
        assert_eq!(lines[3], "NEUTRON_LAUNCHER_TIMEOUT=2");
        assert_eq!(lines[4], "NEUTRON_USE_PROTON=proton-cachyos-slr");
        assert_eq!(lines[5], "NEUTRON_USE_GAMEMODE=1");
        assert_eq!(lines[6], "MANGOHUD=0");
        assert_eq!(lines[7], "NEUTRON_USE_GAMESCOPE=0");

        // 4. Reserve VRAM headroom section
        assert_eq!(lines[8], "");
        assert_eq!(lines[9].len(), 40);
        assert!(lines[9].starts_with("# Reserve VRAM headroom ---"));
        assert_eq!(lines[10], "NEUTRON_RESERVE_VRAM=256");

        // 5. Shader cache size section
        assert_eq!(lines[11], "");
        assert_eq!(lines[12].len(), 40);
        assert!(lines[12].starts_with("# Shader cache size ---"));
        assert_eq!(lines[13], "NEUTRON_SHADER_CACHE_SIZE=10240");
    }

    #[test]
    fn test_config_format_content_game() {
        let cfg = GameConfig {
            id: "3669870".to_string(),
            title: "CONTROL Resonant".to_string(),
            path: PathBuf::from("/dummy/3669870.env"),
            icon_path: None,
            entries: vec![
                EnvEntry {
                    key: "MANGOHUD".into(),
                    value: "1".into(),
                    enabled: true,
                    comment_above: None,
                },
                EnvEntry {
                    key: "MANGOHUD_CONFIG".into(),
                    value: "cpu_temp,gpu_temp".into(),
                    enabled: true,
                    comment_above: None,
                },
                EnvEntry {
                    key: "DXVK_HDR".into(),
                    value: "1".into(),
                    enabled: true,
                    comment_above: None,
                },
                EnvEntry {
                    key: "VKD3D_CONFIG".into(),
                    value: "descriptor_heap".into(),
                    enabled: true,
                    comment_above: None,
                },
            ],
        };

        let formatted = cfg.format_content();
        let lines: Vec<&str> = formatted.lines().collect();

        // 1. Top comment padded to column 60
        assert_eq!(lines[0].len(), 60);
        assert!(lines[0].starts_with("# CONTROL Resonant overrides ==="));
        assert!(lines[0].ends_with('='));

        // 2. Empty line
        assert_eq!(lines[1], "");

        // 3. Neutron configs section (contains mangohud)
        assert_eq!(lines[2].len(), 40);
        assert!(lines[2].starts_with("# Neutron configs ---"));
        assert_eq!(lines[3], "MANGOHUD=1");
        assert_eq!(lines[4], "MANGOHUD_CONFIG=cpu_temp,gpu_temp");

        // 4. Custom envvars section
        assert_eq!(lines[5], "");
        assert_eq!(lines[6].len(), 40);
        assert!(lines[6].starts_with("# Custom envvars ---"));
        assert_eq!(lines[7], "DXVK_HDR=1");
        assert_eq!(lines[8], "VKD3D_CONFIG=descriptor_heap");
    }

    #[test]
    fn test_no_duplicate_comments_and_clean_reload() {
        let dirty_content = r#"
# Game-specific overrides for ARC Raiders (App ID: 1808500)

# Game-specific overrides for ARC Raiders (App ID: 1808500)
# Random custom comment
PROTON_DLSS_UPGRADE=1
DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION=render_preset_k
"#;

        let temp_dir = std::env::temp_dir().join(format!("neutron_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let env_file = temp_dir.join("1808500.env");
        std::fs::write(&env_file, dirty_content).unwrap();

        // Load dirty file
        let cfg = GameConfig::load_from_file(&env_file).unwrap();
        assert_eq!(cfg.title, "ARC Raiders");
        assert_eq!(cfg.entries.len(), 2);

        // Format content
        let formatted = cfg.format_content();
        let occurrences = formatted.matches("ARC Raiders overrides").count();
        assert_eq!(occurrences, 1);
        assert!(!formatted.contains("Game-specific overrides for"));
        assert!(!formatted.contains("Random custom comment"));

        // Save back
        cfg.save().unwrap();
        let saved_content = std::fs::read_to_string(&env_file).unwrap();
        assert_eq!(saved_content, formatted);

        // Load again and save again (Idempotence)
        let reloaded_cfg = GameConfig::load_from_file(&env_file).unwrap();
        assert_eq!(reloaded_cfg.title, "ARC Raiders");
        assert_eq!(reloaded_cfg.entries.len(), 3);
        let re_formatted = reloaded_cfg.format_content();
        assert_eq!(re_formatted, formatted);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_dlss_overrides_extract_and_format() {
        let entries = vec![
            EnvEntry {
                key: "PROTON_DLSS_UPGRADE".into(),
                value: "1".into(),
                enabled: true,
                comment_above: None,
            },
            EnvEntry {
                key: "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION".into(),
                value: "render_preset_k".into(),
                enabled: true,
                comment_above: None,
            },
            EnvEntry {
                key: "DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE_RENDER_PRESET_SELECTION".into(),
                value: "render_preset_e".into(),
                enabled: true,
                comment_above: None,
            },
            EnvEntry {
                key: "DXVK_NVAPI_SET_NGX_DEBUG_OPTIONS".into(),
                value: "DLSSIndicator=1024,DLSSGIndicator=2".into(),
                enabled: true,
                comment_above: None,
            },
        ];

        let mf = ManagedFeatures::extract(&entries, false);
        let dlss = mf.dlss.expect("dlss should be Some");
        assert!(dlss.upgrade);
        assert!(dlss.indicator);
        assert_eq!(dlss.super_resolution, "render_preset_k");
        assert_eq!(dlss.ray_reconstruction, "render_preset_e");
        assert_eq!(dlss.frame_generation, "");

        let cfg = GameConfig {
            id: "3669870".to_string(),
            title: "CONTROL Resonant".to_string(),
            path: PathBuf::from("/dummy/3669870.env"),
            icon_path: None,
            entries,
        };

        let formatted = cfg.format_content();
        assert!(formatted.contains("# DLSS overrides -----------------------"));
        assert!(formatted.contains("PROTON_DLSS_UPGRADE=1"));
        assert!(formatted.contains("DXVK_NVAPI_SET_NGX_DEBUG_OPTIONS=\"DLSSIndicator=1024,DLSSGIndicator=2\""));
        assert!(formatted.contains("DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION=render_preset_k"));
        assert!(formatted.contains("DXVK_NVAPI_DRS_NGX_DLSS_RR_OVERRIDE_RENDER_PRESET_SELECTION=render_preset_e"));
    }

    #[test]
    fn test_dlss_model_compatibility() {
        assert!(is_dlss_capable_model("NVIDIA GeForce RTX 3080"));
        assert!(is_dlss_capable_model("NVIDIA GeForce RTX 2080 Ti"));
        assert!(is_dlss_capable_model("NVIDIA GeForce RTX 4090"));
        assert!(is_dlss_capable_model("Quadro RTX 6000"));
        assert!(is_dlss_capable_model("NVIDIA RTX A5000"));
        assert!(is_dlss_capable_model("NVIDIA TITAN RTX"));
        assert!(is_dlss_capable_model("NVIDIA TITAN V"));

        assert!(!is_dlss_capable_model("NVIDIA GeForce GTX 1080 Ti"));
        assert!(!is_dlss_capable_model("NVIDIA GeForce GTX 1660 Super"));
        assert!(!is_dlss_capable_model("NVIDIA GeForce GTX 980"));
        assert!(!is_dlss_capable_model("NVIDIA GeForce GT 1030"));
        assert!(!is_dlss_capable_model("AMD Radeon RX 7900 XTX"));

        assert!(is_frame_generation_capable_model("NVIDIA GeForce RTX 4090"));
        assert!(is_frame_generation_capable_model("NVIDIA GeForce RTX 5080"));
        assert!(is_frame_generation_capable_model("NVIDIA RTX 4000 Ada Generation"));
        assert!(!is_frame_generation_capable_model("NVIDIA GeForce RTX 3080"));
        assert!(!is_frame_generation_capable_model("NVIDIA GeForce RTX 2080"));
    }

    #[test]
    fn test_sync_from_custom_entries_and_reformat() {
        let mut mf = ManagedFeatures::extract(&[], false);
        let mut entries = vec![
            EnvEntry {
                key: "CUSTOM_VAR".into(),
                value: "hello".into(),
                enabled: true,
                comment_above: None,
            },
            EnvEntry {
                key: "MANGOHUD".into(),
                value: "1".into(),
                enabled: true,
                comment_above: None,
            },
            EnvEntry {
                key: "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION".into(),
                value: "render_preset_custom_99".into(),
                enabled: true,
                comment_above: None,
            },
        ];

        mf.sync_from_entries(&entries);
        assert!(mf.mangohud.is_enabled());
        let dlss = mf.dlss.as_ref().expect("dlss should be set");
        assert_eq!(dlss.super_resolution, "render_preset_custom_99");

        mf.apply_to_entries(&mut entries, false);

        let cfg = GameConfig {
            id: "12345".to_string(),
            title: "Test Game".to_string(),
            path: PathBuf::from("/dummy/12345.env"),
            icon_path: None,
            entries,
        };

        let formatted = cfg.format_content();
        assert!(formatted.contains("# DLSS overrides -----------------------"));
        assert!(
            formatted.contains("DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION=render_preset_custom_99")
        );
        assert!(formatted.contains("# Custom envvars -----------------------"));
        assert!(formatted.contains("CUSTOM_VAR=hello"));
        assert!(!formatted.contains("# Custom envvars -----------------------\nMANGOHUD=1"));
    }

    #[test]
    fn test_cpu_topology_and_disable_smt() {
        // CPU list parsing
        assert_eq!(parse_cpu_list_string("0,8"), vec![0, 8]);
        assert_eq!(parse_cpu_list_string("0-3,8-11"), vec![0, 1, 2, 3, 8, 9, 10, 11]);
        assert_eq!(parse_cpu_list_string("0-7"), vec![0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(parse_cpu_list_string("8,0,8,1"), vec![0, 1, 8]);
        assert_eq!(parse_cpu_list_string(""), Vec::<usize>::new());

        // CPU range formatting
        assert_eq!(format_cpu_ranges(&[0, 1, 2, 3, 4, 5, 6, 7]), "0-7");
        assert_eq!(format_cpu_ranges(&[0, 2, 4, 6]), "0,2,4,6");
        assert_eq!(format_cpu_ranges(&[0, 1, 4, 5]), "0,1,4,5");
        assert_eq!(format_cpu_ranges(&[0, 1, 2, 5, 6, 7, 9]), "0-2,5-7,9");
        assert_eq!(format_cpu_ranges(&[5]), "5");
        assert_eq!(format_cpu_ranges(&[]), "");

        // SMT Feature extraction variations
        let entries_enabled = vec![EnvEntry {
            key: "NEUTRON_DISABLE_SMT".into(),
            value: "1".into(),
            enabled: true,
            comment_above: None,
        }];
        let mf = ManagedFeatures::extract(&entries_enabled, false);
        assert_eq!(mf.disable_smt, FeatureState::Enabled("".into()));

        let entries_custom = vec![EnvEntry {
            key: "NEUTRON_DISABLE_SMT".into(),
            value: "0-7".into(),
            enabled: true,
            comment_above: None,
        }];
        let mf_custom = ManagedFeatures::extract(&entries_custom, false);
        assert_eq!(mf_custom.disable_smt, FeatureState::Enabled("".into()));

        let entries_disabled = vec![EnvEntry {
            key: "NEUTRON_DISABLE_SMT".into(),
            value: "0".into(),
            enabled: true,
            comment_above: None,
        }];
        let mf_disabled = ManagedFeatures::extract(&entries_disabled, false);
        assert_eq!(mf_disabled.disable_smt, FeatureState::Disabled);

        let entries_smt_zero = vec![EnvEntry {
            key: "NEUTRON_SMT".into(),
            value: "0".into(),
            enabled: true,
            comment_above: None,
        }];
        let mf_smt_zero = ManagedFeatures::extract(&entries_smt_zero, false);
        assert_eq!(mf_smt_zero.disable_smt, FeatureState::Enabled("".into()));

        let entries_smt_one = vec![EnvEntry {
            key: "NEUTRON_SMT".into(),
            value: "1".into(),
            enabled: true,
            comment_above: None,
        }];
        let mf_smt_one = ManagedFeatures::extract(&entries_smt_one, false);
        assert_eq!(mf_smt_one.disable_smt, FeatureState::Disabled);

        let entries_topology = vec![EnvEntry {
            key: "WINE_CPU_TOPOLOGY".into(),
            value: "8:0,1,2,3,4,5,6,7".into(),
            enabled: true,
            comment_above: None,
        }];
        let mf_topo = ManagedFeatures::extract(&entries_topology, false);
        assert_eq!(mf_topo.disable_smt, FeatureState::Enabled("".into()));

        let entries_topology_zero = vec![EnvEntry {
            key: "WINE_CPU_TOPOLOGY".into(),
            value: "0".into(),
            enabled: true,
            comment_above: None,
        }];
        let mf_topo_zero = ManagedFeatures::extract(&entries_topology_zero, false);
        assert_eq!(mf_topo_zero.disable_smt, FeatureState::Disabled);

        // SMT sync from custom entries
        let mut mf_sync = ManagedFeatures::extract(&[], false);
        let custom_entries = vec![EnvEntry {
            key: "NEUTRON_DISABLE_SMT".into(),
            value: "0-7".into(),
            enabled: true,
            comment_above: None,
        }];
        mf_sync.sync_from_entries(&custom_entries);
        assert_eq!(mf_sync.disable_smt, FeatureState::Enabled("".into()));

        let custom_topo_entries = vec![EnvEntry {
            key: "WINE_CPU_TOPOLOGY".into(),
            value: "8:0,1,2,3,4,5,6,7".into(),
            enabled: true,
            comment_above: None,
        }];
        let mut mf_sync_topo = ManagedFeatures::extract(&[], false);
        mf_sync_topo.sync_from_entries(&custom_topo_entries);
        assert_eq!(mf_sync_topo.disable_smt, FeatureState::Enabled("".into()));

        let custom_topo_zero = vec![EnvEntry {
            key: "WINE_CPU_TOPOLOGY".into(),
            value: "0".into(),
            enabled: true,
            comment_above: None,
        }];
        mf_sync_topo.sync_from_entries(&custom_topo_zero);
        assert_eq!(mf_sync_topo.disable_smt, FeatureState::Disabled);
    }

    #[test]
    fn test_disabled_env_entries_roundtrip() {
        let content = r#"
# Global configuration =====================================
# Random comment
# note = something
##DOUBLE=1
#MANGOHUD=1

# Custom envvars -----------------------
ENABLED_VAR=1
#FOO=bar
#QUOTED="hello world"
#EMPTY=""
# SPACED=42
"#;

        let temp_dir = std::env::temp_dir().join(format!("neutron_disabled_rt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let env_file = temp_dir.join("999999.env");
        std::fs::write(&env_file, content).unwrap();

        let cfg = GameConfig::load_from_file(&env_file).unwrap();
        assert_eq!(
            cfg.entries,
            vec![
                EnvEntry {
                    key: "ENABLED_VAR".into(),
                    value: "1".into(),
                    enabled: true,
                    comment_above: None,
                },
                EnvEntry {
                    key: "FOO".into(),
                    value: "bar".into(),
                    enabled: false,
                    comment_above: None,
                },
                EnvEntry {
                    key: "QUOTED".into(),
                    value: "hello world".into(),
                    enabled: false,
                    comment_above: None,
                },
                EnvEntry {
                    key: "EMPTY".into(),
                    value: "".into(),
                    enabled: false,
                    comment_above: None,
                },
                EnvEntry {
                    key: "SPACED".into(),
                    value: "42".into(),
                    enabled: false,
                    comment_above: None,
                },
            ]
        );

        let formatted = cfg.format_content();
        assert!(formatted.contains("# Custom envvars -----------------------\nENABLED_VAR=1\n#FOO=bar\n#QUOTED=\"hello world\"\n#EMPTY=\"\"\n#SPACED=42\n"));
        assert!(!formatted.contains("Random comment"));
        assert!(!formatted.contains("DOUBLE"));

        cfg.save().unwrap();
        let reloaded = GameConfig::load_from_file(&env_file).unwrap();
        assert_eq!(reloaded.entries, cfg.entries);
        assert_eq!(reloaded.format_content(), formatted);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_compute_effective_envvars_with_disabled_entries() {
        let temp_dir = std::env::temp_dir().join(format!("neutron_disabled_eff_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        std::fs::write(
            temp_dir.join("global.env"),
            "GLOBAL_ACTIVE=yes\n#GLOBAL_DISABLED=no\nSHARED_VAR=from_global\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("123456.env"),
            "OVERRIDE_ACTIVE=1\n#GAME_DISABLED=99\n#SHARED_VAR=disabled_override\n",
        )
        .unwrap();
        std::fs::write(temp_dir.join("123457.env"), "SHARED_VAR=from_game\n").unwrap();

        let envs_disabled_override = compute_effective_envvars_from_dir(&temp_dir, "123456");
        assert!(envs_disabled_override.contains(&("GLOBAL_ACTIVE".into(), "yes".into())));
        assert!(!envs_disabled_override.iter().any(|(k, _)| k == "GLOBAL_DISABLED"));
        assert!(envs_disabled_override.contains(&("OVERRIDE_ACTIVE".into(), "1".into())));
        assert!(!envs_disabled_override.iter().any(|(k, _)| k == "GAME_DISABLED"));
        assert!(envs_disabled_override.contains(&("SHARED_VAR".into(), "from_global".into())));

        let envs_active_override = compute_effective_envvars_from_dir(&temp_dir, "123457");
        assert!(envs_active_override.contains(&("SHARED_VAR".into(), "from_game".into())));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_is_key_overridden_in_entries() {
        let disabled_only = vec![EnvEntry {
            key: "DXVK_HDR".into(),
            value: "1".into(),
            enabled: false,
            comment_above: None,
        }];
        assert!(!is_key_overridden_in_entries(&disabled_only, "DXVK_HDR"));

        let enabled_with_spaces = vec![EnvEntry {
            key: " DXVK_HDR ".into(),
            value: "0".into(),
            enabled: true,
            comment_above: None,
        }];
        assert!(is_key_overridden_in_entries(&enabled_with_spaces, "DXVK_HDR"));

        let disabled_and_enabled = vec![
            EnvEntry {
                key: "DXVK_HDR".into(),
                value: "0".into(),
                enabled: false,
                comment_above: None,
            },
            EnvEntry {
                key: "DXVK_HDR".into(),
                value: "1".into(),
                enabled: true,
                comment_above: None,
            },
        ];
        assert!(is_key_overridden_in_entries(&disabled_and_enabled, "DXVK_HDR"));
        assert!(!is_key_overridden_in_entries(
            &disabled_and_enabled,
            "PROTON_ENABLE_WAYLAND"
        ));
        assert!(!is_key_overridden_in_entries(&disabled_and_enabled, "   "));
    }

    #[test]
    fn test_compute_vram_envvars_with() {
        assert_eq!(
            compute_vram_envvars_with("2048", Some(10240), Some("0x10de:0x2206")),
            vec![
                ("VK_LOADER_LAYERS_ENABLE", "VK_LAYER_MESA_vram_report_limit".to_string()),
                ("VK_VRAM_REPORT_LIMIT_HEAP_SIZE", "8192".to_string()),
                ("VK_VRAM_REPORT_LIMIT_DEVICE_ID", "0x10de:0x2206".to_string()),
            ]
        );
        assert!(compute_vram_envvars_with("0", Some(10240), Some("0x10de:0x2206")).is_empty());
        assert!(compute_vram_envvars_with("10240", Some(10240), Some("0x10de:0x2206")).is_empty());
        assert!(compute_vram_envvars_with("12000", Some(10240), Some("0x10de:0x2206")).is_empty());
        assert!(compute_vram_envvars_with("2048", None, Some("0x10de:0x2206")).is_empty());
        assert!(compute_vram_envvars_with("2048", Some(10240), None).is_empty());
    }
}
