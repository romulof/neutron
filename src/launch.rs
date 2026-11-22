use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

static ORIGINAL_RENDERER_ENV: OnceLock<(Option<OsString>, Option<OsString>)> = OnceLock::new();

/// Forces GTK4 to render with Cairo in software so Steam's preloaded overlay
/// hooks (OpenGL/Vulkan) do not paint Steam's performance monitor onto
/// Neutron's pre-launch windows. Must be called before GTK initializes.
pub fn enable_software_renderer_for_ui() {
    ORIGINAL_RENDERER_ENV.get_or_init(|| (env::var_os("GSK_RENDERER"), env::var_os("GDK_DISABLE")));
    unsafe {
        env::set_var("GSK_RENDERER", "cairo");
        env::set_var("GDK_DISABLE", "gl:vulkan");
    }
}

fn apply_restored_renderer_env(
    cmd: &mut Command,
    original: &(Option<OsString>, Option<OsString>),
    effective: &[(String, String)],
) {
    let vars = [("GSK_RENDERER", &original.0), ("GDK_DISABLE", &original.1)];
    for (key, orig_val) in vars {
        if effective.iter().any(|(k, _)| k == key) {
            continue;
        }
        match orig_val {
            Some(val) => {
                cmd.env(key, val);
            }
            None => {
                cmd.env_remove(key);
            }
        }
    }
}

const APPIMAGE_RESTORE_VARS: &[&str] = &[
    "PATH",
    "LD_LIBRARY_PATH",
    "XDG_DATA_DIRS",
    "GDK_BACKEND",
    "GDK_PIXBUF_MODULE_FILE",
    "GDK_PIXBUF_MODULEDIR",
    "GTK_PATH",
    "GTK_EXE_PREFIX",
    "GTK_DATA_PREFIX",
    "GTK_IM_MODULE_FILE",
    "GTK_THEME",
    "GSETTINGS_SCHEMA_DIR",
    "GI_TYPELIB_PATH",
    "GIO_EXTRA_MODULES",
];

fn restore_appimage_env_for_child(cmd: &mut Command, effective: &[(String, String)]) {
    if env::var_os("NEUTRON_APPIMAGE_ENV").is_none() {
        return;
    }

    cmd.env_remove("NEUTRON_APPIMAGE_ENV");
    for var in APPIMAGE_RESTORE_VARS {
        let orig_key = format!("NEUTRON_ORIG_{}", var);
        let unset_key = format!("NEUTRON_UNSET_{}", var);
        cmd.env_remove(&orig_key);
        cmd.env_remove(&unset_key);

        if effective.iter().any(|(k, _)| k == var) {
            continue;
        }

        if let Some(orig_val) = env::var_os(&orig_key) {
            cmd.env(var, orig_val);
        } else if env::var_os(&unset_key).is_some() {
            cmd.env_remove(var);
        }
    }
}

fn restore_renderer_env_for_child(cmd: &mut Command, effective: &[(String, String)]) {
    if let Some(original) = ORIGINAL_RENDERER_ENV.get() {
        apply_restored_renderer_env(cmd, original, effective);
    }
    restore_appimage_env_for_child(cmd, effective);
}

use clap::{Parser, Subcommand};

use crate::config::{
    FeatureState, GameConfig, ManagedFeatures, compute_effective_envvars, get_config_dir, is_command_available,
    parse_vdf_key,
};

pub const PROTON_VERBS: &[&str] = &[
    "waitforexitandrun",
    "run",
    "runinprefix",
    "destroyprefix",
    "getcompatpath",
    "getnativepath",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMode {
    /// Non-game Proton setup/query verb (getcompatpath, runinprefix, destroyprefix, etc.).
    /// Runs silently without GTK, immediately execs Proton.
    CompatVerb { verb: String, args: Vec<String> },
    /// Silent game/installer execution (verb == "run" or install script like iscriptevaluator.exe).
    /// Runs silently without GTK, immediately execs Proton with game env.
    SilentGame {
        verb: String,
        app_id: String,
        args: Vec<String>,
    },
    /// Interactive game launch via Steam compatibility tool (waitforexitandrun).
    /// Shows pre-launch countdown overlay once before launching the game.
    CompatGame {
        verb: String,
        app_id: String,
        args: Vec<String>,
    },
    /// Interactive game launch via Steam Launch Options (neutron launch %command%).
    /// Shows pre-launch countdown overlay with environment overrides for arbitrary game commands.
    CommandWrap { app_id: String, command: Vec<String> },
    /// Explicit preview/test mode from CLI (--test [app_id]).
    Preview { app_id: String },
    /// Standalone Settings Manager GUI (neutron with no args).
    Manager,
}

pub fn is_helper_binary(args: &[String]) -> bool {
    for arg in args {
        let lower = arg.to_lowercase();
        if lower.contains("iscriptevaluator")
            || lower.contains("legacycompat")
            || lower.contains("dxsetup")
            || lower.contains("vcredist")
            || lower.contains("installscript")
        {
            return true;
        }
    }
    false
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
#[command(
    name = "neutron",
    about = "Steam Proton compatibility tool and environment manager UI",
    version,
    disable_version_flag = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Test and preview the pre-launch countdown overlay for a Steam App ID
    #[arg(short = 't', long = "test", num_args = 0..=1, value_name = "APP_ID")]
    pub test: Option<Option<String>>,

    /// Print version
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    pub version: (),

    #[command(subcommand)]
    pub command: Option<SteamCommand>,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum SteamCommand {
    /// Run prerequisite install scripts silently without GUI (used by Steam)
    #[command(
        name = "run",
        about = "Run prerequisite install scripts silently without GUI (used by Steam)",
        after_help = "\
\x1b[1m\x1b[4mEnvironment Variables:\x1b[0m
  STEAM_COMPAT_APP_ID               Steam App ID for the game
  STEAM_COMPAT_DATA_PATH            Path to Proton prefix directory (.../compatdata/<appid>)
  STEAM_COMPAT_CLIENT_INSTALL_PATH  Path to Steam client installation
  STEAM_COMPAT_TOOL_PATHS           Colon-separated search paths for Proton tools"
    )]
    Run {
        /// Script evaluator or helper binary and its arguments (e.g. iscriptevaluator.exe)
        #[arg(
            trailing_var_arg = true,
            allow_hyphen_values = true,
            required = true,
            value_name = "COMMAND"
        )]
        command: Vec<String>,
    },
    /// Launch the game executable with the pre-launch countdown overlay (used by Steam)
    #[command(
        name = "waitforexitandrun",
        about = "Launch game executable with pre-launch countdown overlay (used by Steam)",
        after_help = "\
\x1b[1m\x1b[4mEnvironment Variables:\x1b[0m
  STEAM_COMPAT_APP_ID            Steam App ID for the game
  STEAM_COMPAT_DATA_PATH         Path to Proton prefix directory (.../compatdata/<appid>)
  STEAM_COMPAT_INSTALL_PATH      Path to game installation folder (.../steamapps/common/<game>)"
    )]
    Waitforexitandrun {
        /// Game executable to launch and its command-line arguments
        #[arg(
            trailing_var_arg = true,
            allow_hyphen_values = true,
            required = true,
            value_name = "COMMAND"
        )]
        command: Vec<String>,
    },
    /// Launch game via Steam Launch Options (%command%) with environment overrides
    #[command(
        name = "launch",
        about = "Launch game via Steam Launch Options (%command%) with environment overrides",
        after_help = "\
\x1b[1m\x1b[4mEnvironment Variables:\x1b[0m
  SteamAppId, SteamGameId        Steam App ID / Game ID (set by Steam for %command%)
  STEAM_COMPAT_APP_ID            Steam App ID for Proton games (if running via Proton)
  STEAM_COMPAT_DATA_PATH         Path to Proton prefix directory (.../compatdata/<appid>)
  STEAM_COMPAT_INSTALL_PATH      Path to game installation folder (.../steamapps/common/<game>)"
    )]
    Launch {
        /// Game command or executable to launch (e.g. %command%) and its arguments
        #[arg(
            trailing_var_arg = true,
            allow_hyphen_values = true,
            required = true,
            value_name = "COMMAND"
        )]
        command: Vec<String>,
    },
    /// Other Proton compatibility verbs or direct game commands
    #[command(external_subcommand)]
    External(Vec<String>),
}

pub fn detect_mode_from_cli(cli: &Cli) -> LaunchMode {
    if let Some(ref opt_app_id) = cli.test {
        let app_id = match opt_app_id {
            Some(id) if !id.trim().is_empty() => id.trim().to_string(),
            _ => {
                let resolved = resolve_app_id(&[]);
                if !resolved.is_empty() {
                    resolved
                } else {
                    "3669870".to_string()
                }
            }
        };
        return LaunchMode::Preview { app_id };
    }

    if let Some(ref cmd) = cli.command {
        match cmd {
            SteamCommand::Run { command } => {
                let verb = "run".to_string();
                let full_args = command.clone();
                let app_id = resolve_app_id(&full_args);
                return LaunchMode::SilentGame {
                    verb,
                    app_id,
                    args: full_args,
                };
            }
            SteamCommand::Waitforexitandrun { command } => {
                let verb = "waitforexitandrun".to_string();
                let full_args = command.clone();
                let app_id = resolve_app_id(&full_args);

                if is_helper_binary(&full_args) {
                    return LaunchMode::SilentGame {
                        verb,
                        app_id,
                        args: full_args,
                    };
                }

                return LaunchMode::CompatGame {
                    verb,
                    app_id,
                    args: full_args,
                };
            }
            SteamCommand::Launch { command } => {
                let full_args = command.clone();
                let app_id = resolve_app_id(&full_args);
                return LaunchMode::CommandWrap {
                    app_id,
                    command: full_args,
                };
            }
            SteamCommand::External(args) => {
                if !args.is_empty() {
                    let first_arg = &args[0];
                    if PROTON_VERBS.contains(&first_arg.as_str()) {
                        let verb = first_arg.clone();
                        let game_args = args[1..].to_vec();
                        return LaunchMode::CompatVerb { verb, args: game_args };
                    }
                }
            }
        }
    }

    // No command arguments provided:
    // If running in a Steam environment with App ID, show countdown preview for that game.
    // Otherwise open the Standalone Settings Manager.
    let resolved_id = resolve_app_id(&[]);
    if !resolved_id.is_empty() {
        LaunchMode::Preview { app_id: resolved_id }
    } else {
        LaunchMode::Manager
    }
}

pub fn detect_mode(args: &[String]) -> LaunchMode {
    match Cli::try_parse_from(args) {
        Ok(cli) => detect_mode_from_cli(&cli),
        Err(err) => err.exit(),
    }
}

pub fn resolve_app_id(game_args: &[String]) -> String {
    // 1. STEAM_COMPAT_APP_ID (set by Steam for compatibility tools)
    if let Ok(id) = env::var("STEAM_COMPAT_APP_ID") {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    // 2. SteamAppId / SteamGameId (set by Steam for %command% Launch Options)
    if let Ok(id) = env::var("SteamAppId") {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(id) = env::var("SteamGameId") {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    // 3. STEAM_COMPAT_DATA_PATH (e.g. .../compatdata/3669870)
    if let Ok(data_path) = env::var("STEAM_COMPAT_DATA_PATH") {
        if let Some(folder_name) = Path::new(&data_path).file_name().and_then(|n| n.to_str()) {
            let trimmed = folder_name.trim();
            if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
                return trimmed.to_string();
            }
        }
    }

    // 4. STEAM_COMPAT_INSTALL_PATH (e.g. .../common/CONTROL Resonant)
    if let Ok(install_path) = env::var("STEAM_COMPAT_INSTALL_PATH") {
        if let Some(deduced) = deduce_app_id_from_path(Path::new(&install_path)) {
            return deduced;
        }
    }

    // 5. Scan game_args for executable or folder path
    for arg in game_args {
        if arg.contains("steamapps") || arg.ends_with(".exe") || arg.starts_with('/') {
            if let Some(deduced) = deduce_app_id_from_path(Path::new(arg)) {
                return deduced;
            }
        }
    }

    String::new()
}

pub fn deduce_app_id_from_path(path: &Path) -> Option<String> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut current = canonical.as_path();
    while let Some(parent) = current.parent() {
        if let Some(name) = parent.file_name().and_then(|n| n.to_str()) {
            if name == "common" {
                if let Some(game_dir) = current.file_name().and_then(|n| n.to_str()) {
                    if let Some(steamapps) = parent.parent() {
                        if let Ok(entries) = fs::read_dir(steamapps) {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if let Some(fname) = p.file_name().and_then(|n| n.to_str()) {
                                    if fname.starts_with("appmanifest_") && fname.ends_with(".acf") {
                                        if let Ok(content) = fs::read_to_string(&p) {
                                            if let Some(idir) = parse_vdf_key(&content, "installdir") {
                                                if idir.eq_ignore_ascii_case(game_dir) {
                                                    let appid = fname
                                                        .trim_start_matches("appmanifest_")
                                                        .trim_end_matches(".acf");
                                                    return Some(appid.to_string());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        current = parent;
    }
    None
}

pub fn split_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '\\' if !in_single => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

pub fn get_steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let home = env::var("HOME").ok().unwrap_or_default();

    if let Ok(client_path) = env::var("STEAM_COMPAT_CLIENT_INSTALL_PATH") {
        if !client_path.is_empty() {
            let p = PathBuf::from(client_path);
            if p.is_dir() {
                roots.push(p);
            }
        }
    }

    if let Ok(xdg_data) = env::var("XDG_DATA_HOME") {
        if !xdg_data.is_empty() {
            let p = PathBuf::from(xdg_data).join("Steam");
            if p.is_dir() && !roots.contains(&p) {
                roots.push(p);
            }
        }
    }

    let default_candidates = [
        PathBuf::from(&home).join(".local/share/Steam"),
        PathBuf::from(&home).join(".steam/steam"),
        PathBuf::from(&home).join(".steam/root"),
        PathBuf::from("/usr/share/steam"),
        PathBuf::from("/usr/local/share/steam"),
    ];
    for p in default_candidates {
        if p.is_dir() && !roots.contains(&p) {
            roots.push(p);
        }
    }
    roots
}

pub fn get_compat_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for root in get_steam_roots() {
        let cd = root.join("compatibilitytools.d");
        if cd.is_dir() && !dirs.contains(&cd) {
            dirs.push(cd);
        }
    }
    for sys in [
        PathBuf::from("/usr/share/steam/compatibilitytools.d"),
        PathBuf::from("/usr/local/share/steam/compatibilitytools.d"),
    ] {
        if sys.is_dir() && !dirs.contains(&sys) {
            dirs.push(sys);
        }
    }
    dirs
}

pub fn get_steamapps_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(compat_install) = env::var("STEAM_COMPAT_INSTALL_PATH") {
        if let Some(parent2) = Path::new(&compat_install).parent().and_then(|p| p.parent()) {
            if parent2.is_dir() {
                dirs.push(parent2.to_path_buf());
            }
        }
    }

    for root in get_steam_roots() {
        let sa = root.join("steamapps");
        if sa.is_dir() && !dirs.contains(&sa) {
            dirs.push(sa.clone());
        }
        let vdf = sa.join("libraryfolders.vdf");
        if vdf.is_file() {
            if let Ok(content) = fs::read_to_string(&vdf) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("\"path\"") {
                        if let Some((_, p_val)) = trimmed.split_once("\"path\"") {
                            let p = p_val.trim().trim_matches('"');
                            let lib_sa = PathBuf::from(p).join("steamapps");
                            if lib_sa.is_dir() && !dirs.contains(&lib_sa) {
                                dirs.push(lib_sa);
                            }
                        }
                    }
                }
            }
        }
    }
    dirs
}

pub fn get_app_install_dir(app_id: &str) -> Option<PathBuf> {
    let manifest_name = format!("appmanifest_{}.acf", app_id);
    for sa in get_steamapps_dirs() {
        let manifest_path = sa.join(&manifest_name);
        if manifest_path.is_file() {
            if let Ok(content) = fs::read_to_string(&manifest_path) {
                if let Some(idir) = parse_vdf_key(&content, "installdir") {
                    let candidate = sa.join("common").join(&idir);
                    if candidate.is_dir() {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    None
}

pub fn find_proton_dir(preferred_version: Option<&str>) -> Option<PathBuf> {
    if let Ok(p) = env::var("PROTON_PATH") {
        let pb = PathBuf::from(p);
        if is_executable(&pb.join("proton")) {
            return Some(pb);
        }
    }

    let cdirs = get_compat_dirs();
    let sdirs = get_steamapps_dirs();

    if let Some(version) = preferred_version {
        if !version.trim().is_empty() {
            let ver = version.trim();
            for base in &cdirs {
                let candidate = base.join(ver);
                if is_executable(&candidate.join("proton")) {
                    return Some(candidate);
                }
            }
            for base in &sdirs {
                let candidate = base.join("common").join(ver);
                if is_executable(&candidate.join("proton")) {
                    return Some(candidate);
                }
            }
        }
    }

    for base in &cdirs {
        for name in &["proton-cachyos-slr", "proton-cachyos-native"] {
            let candidate = base.join(name);
            if is_executable(&candidate.join("proton")) {
                return Some(candidate);
            }
        }
    }

    for base in &sdirs {
        for name in &["Proton - Experimental", "Proton Hotfix"] {
            let candidate = base.join("common").join(name);
            if is_executable(&candidate.join("proton")) {
                return Some(candidate);
            }
        }
    }

    for base in &cdirs {
        if let Ok(entries) = fs::read_dir(base) {
            let mut ge_candidates = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("GE-Proton") && is_executable(&path.join("proton")) {
                        ge_candidates.push(path);
                    }
                }
            }
            ge_candidates.sort();
            ge_candidates.reverse();
            if let Some(best) = ge_candidates.into_iter().next() {
                return Some(best);
            }
        }
    }

    for base in &sdirs {
        let common = base.join("common");
        if let Ok(entries) = fs::read_dir(&common) {
            let mut proton_candidates = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("Proton ") && is_executable(&path.join("proton")) {
                        proton_candidates.push(path);
                    }
                }
            }
            proton_candidates.sort();
            proton_candidates.reverse();
            if let Some(best) = proton_candidates.into_iter().next() {
                return Some(best);
            }
        }
    }

    None
}

pub fn find_slr_dir(proton_dir: &Path) -> Option<PathBuf> {
    let manifest_path = proton_dir.join("toolmanifest.vdf");
    if manifest_path.is_file() {
        if let Ok(content) = fs::read_to_string(&manifest_path) {
            if let Some(appid) = parse_vdf_key(&content, "require_tool_appid") {
                return get_app_install_dir(&appid);
            }
        }
    }
    None
}

pub fn ensure_app_id_envvars(app_id: &str) {
    let trimmed = app_id.trim();
    if trimmed.is_empty() {
        return;
    }

    for var in ["STEAM_COMPAT_APP_ID", "SteamAppId", "SteamGameId"] {
        let missing = match env::var(var) {
            Ok(val) => val.trim().is_empty(),
            Err(_) => true,
        };
        if missing {
            unsafe {
                env::set_var(var, trimmed);
            }
        }
    }
}

fn steamapps_parent_dir(path: &Path) -> Option<String> {
    let mut current = path;
    while let Some(parent) = current.parent() {
        if parent.file_name().and_then(|n| n.to_str()) == Some("common") {
            if let Some(steamapps) = parent.parent() {
                if steamapps.file_name().and_then(|n| n.to_str()) == Some("steamapps") {
                    return Some(steamapps.to_string_lossy().to_string());
                }
            }
        }
        current = parent;
    }
    None
}

pub fn setup_tool_paths(proton_dir: &Path, slr_dir: Option<&Path>) {
    let mut paths = vec![proton_dir.to_string_lossy().to_string()];
    if let Some(slr) = slr_dir {
        paths.push(slr.to_string_lossy().to_string());
    }
    if let Ok(existing) = env::var("STEAM_COMPAT_TOOL_PATHS") {
        if !existing.trim().is_empty() {
            paths.push(existing);
        }
    }
    unsafe {
        env::set_var("STEAM_COMPAT_TOOL_PATHS", paths.join(":"));
    }

    let mut lib_paths: Vec<String> = Vec::new();
    if let Ok(existing_libs) = env::var("STEAM_COMPAT_LIBRARY_PATHS") {
        for entry in existing_libs.split(':') {
            let trimmed = entry.trim();
            if !trimmed.is_empty() && !lib_paths.iter().any(|p| p == trimmed) {
                lib_paths.push(trimmed.to_string());
            }
        }
    }

    let install_path = env::var("STEAM_COMPAT_INSTALL_PATH").ok();
    let candidate_dirs = [install_path.as_deref().map(Path::new), Some(proton_dir), slr_dir];
    for candidate in candidate_dirs.into_iter().flatten() {
        if let Some(sa) = steamapps_parent_dir(candidate) {
            if !lib_paths.iter().any(|p| p == &sa) {
                lib_paths.push(sa);
            }
        }
    }

    if !lib_paths.is_empty() {
        unsafe {
            env::set_var("STEAM_COMPAT_LIBRARY_PATHS", lib_paths.join(":"));
        }
    }

    if let Ok(service) = env::var("STEAM_COMPAT_LAUNCHER_SERVICE") {
        if service.trim() == "auto" {
            unsafe {
                env::set_var("STEAM_COMPAT_LAUNCHER_SERVICE", "proton");
            }
        }
    }
}

pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        meta.is_file() && (meta.permissions().mode() & 0o111) != 0
    } else {
        false
    }
}

fn load_managed_features_from_dir(
    config_dir: &Path,
    app_id: &str,
) -> (Option<ManagedFeatures>, Option<ManagedFeatures>) {
    let global_cfg = GameConfig::load_from_file(&config_dir.join("global.env")).ok();
    let game_cfg = if !app_id.is_empty() {
        GameConfig::load_from_file(&config_dir.join(format!("{}.env", app_id))).ok()
    } else {
        None
    };

    let global_mf = global_cfg.as_ref().map(|c| ManagedFeatures::extract(&c.entries, true));
    let game_mf = game_cfg.as_ref().map(|c| ManagedFeatures::extract(&c.entries, false));
    (global_mf, game_mf)
}

fn load_managed_features(app_id: &str) -> (Option<ManagedFeatures>, Option<ManagedFeatures>) {
    load_managed_features_from_dir(&get_config_dir(), app_id)
}

fn preferred_proton_version<'a>(
    global_mf: &'a Option<ManagedFeatures>,
    game_mf: &'a Option<ManagedFeatures>,
) -> Option<&'a str> {
    game_mf
        .as_ref()
        .and_then(|g| g.proton_version.as_deref())
        .or_else(|| global_mf.as_ref().and_then(|g| g.proton_version.as_deref()))
}

fn build_proton_command(proton_dir: &Path, slr_dir: Option<&Path>, verb: &str, args: &[String]) -> Vec<String> {
    let proton_bin = proton_dir.join("proton");
    let mut proton_full_args = vec![proton_bin.to_string_lossy().to_string(), verb.to_string()];
    proton_full_args.extend(args.iter().cloned());

    let slr_entry = slr_dir.map(|d| d.join("_v2-entry-point"));
    let use_slr =
        env::var("PRESSURE_VESSEL_RUNTIME").is_err() && slr_entry.as_ref().map(|e| is_executable(e)).unwrap_or(false);

    if use_slr {
        if let Some(entry) = slr_entry {
            let mut full_cmd = vec![
                entry.to_string_lossy().to_string(),
                format!("--verb={}", verb),
                "--".to_string(),
            ];
            full_cmd.extend(proton_full_args);
            return full_cmd;
        }
    }

    proton_full_args
}

pub fn exec_compat_verb(verb: &str, args: &[String]) -> ! {
    let app_id = resolve_app_id(args);
    ensure_app_id_envvars(&app_id);

    let effective = compute_effective_envvars(&app_id);
    for (k, v) in &effective {
        unsafe {
            env::set_var(k, v);
        }
    }

    let (global_mf, game_mf) = load_managed_features(&app_id);
    let preferred_proton = preferred_proton_version(&global_mf, &game_mf);

    let proton_dir = find_proton_dir(preferred_proton).unwrap_or_else(|| {
        eprintln!(
            "neutron: could not find a Proton installation for verb '{}' (preferred: {:?})",
            verb, preferred_proton
        );
        std::process::exit(1);
    });

    let slr_dir = find_slr_dir(&proton_dir);
    setup_tool_paths(&proton_dir, slr_dir.as_deref());

    let full_cmd = build_proton_command(&proton_dir, slr_dir.as_deref(), verb, args);
    let mut cmd = Command::new(&full_cmd[0]);
    cmd.args(&full_cmd[1..]);
    restore_renderer_env_for_child(&mut cmd, &effective);

    let err = cmd.exec();
    eprintln!("neutron: failed to execute {:?}: {}", full_cmd, err);
    std::process::exit(1);
}

/// Wraps `full_cmd` with `taskset -c <cores>` when Disable SMT is enabled.
fn wrap_with_taskset(
    full_cmd: &mut Vec<String>,
    global_mf: Option<&ManagedFeatures>,
    game_mf: Option<&ManagedFeatures>,
) {
    let disable_smt = game_mf
        .map(|g| &g.disable_smt)
        .filter(|m| !matches!(m, FeatureState::Inherit))
        .or_else(|| global_mf.map(|g| &g.disable_smt));

    if matches!(disable_smt, Some(FeatureState::Enabled(_))) && is_command_available("taskset") {
        let cores_to_use = crate::config::get_physical_cpu_cores_arg();
        if !cores_to_use.is_empty() {
            full_cmd.insert(0, cores_to_use);
            full_cmd.insert(0, "-c".to_string());
            full_cmd.insert(0, "taskset".to_string());
        }
    }
}

pub fn exec_compat_game(verb: &str, app_id: &str, game_args: &[String]) -> ! {
    ensure_app_id_envvars(app_id);

    let effective = compute_effective_envvars(app_id);
    for (k, v) in &effective {
        unsafe {
            env::set_var(k, v);
        }
    }

    let (global_mf, game_mf) = load_managed_features(app_id);
    let preferred_proton = preferred_proton_version(&global_mf, &game_mf);

    let proton_dir = find_proton_dir(preferred_proton).unwrap_or_else(|| {
        eprintln!("neutron: could not find Proton (preferred: {:?})", preferred_proton);
        std::process::exit(1);
    });

    let slr_dir = find_slr_dir(&proton_dir);
    setup_tool_paths(&proton_dir, slr_dir.as_deref());

    let mut full_cmd = build_proton_command(&proton_dir, slr_dir.as_deref(), verb, game_args);

    wrap_with_taskset(&mut full_cmd, global_mf.as_ref(), game_mf.as_ref());

    // Wrap with GameScope if enabled
    let gamescope = game_mf
        .as_ref()
        .map(|g| &g.gamescope)
        .filter(|m| !matches!(m, FeatureState::Inherit))
        .or_else(|| global_mf.as_ref().map(|g| &g.gamescope));

    if let Some(FeatureState::Enabled(custom_args)) = gamescope {
        if is_command_available("gamescope") {
            let mut gs_cmd = vec!["gamescope".to_string()];
            let args_to_use = if custom_args.trim().is_empty() {
                crate::config::compute_default_gamescope_args()
            } else {
                custom_args.clone()
            };
            gs_cmd.extend(split_args(&args_to_use));
            gs_cmd.push("--".to_string());
            gs_cmd.extend(full_cmd);
            full_cmd = gs_cmd;
        }
    }

    // Wrap with GameMode if enabled
    let gamemode = game_mf
        .as_ref()
        .map(|g| &g.gamemode)
        .filter(|m| !matches!(m, FeatureState::Inherit))
        .or_else(|| global_mf.as_ref().map(|g| &g.gamemode));

    if let Some(FeatureState::Enabled(_)) = gamemode {
        if is_command_available("gamemoderun") {
            full_cmd.insert(0, "gamemoderun".to_string());
        }
    }

    let mut cmd = Command::new(&full_cmd[0]);
    cmd.args(&full_cmd[1..]);
    restore_renderer_env_for_child(&mut cmd, &effective);

    let err = cmd.exec();
    eprintln!("neutron: failed to exec {:?}: {}", full_cmd, err);
    std::process::exit(1);
}

pub fn exec_command_wrap(app_id: &str, command: &[String]) -> ! {
    if command.is_empty() {
        eprintln!("neutron: no command specified to launch");
        std::process::exit(1);
    }

    ensure_app_id_envvars(app_id);

    let effective = compute_effective_envvars(app_id);
    for (k, v) in &effective {
        unsafe {
            env::set_var(k, v);
        }
    }

    let (global_mf, game_mf) = load_managed_features(app_id);

    let mut full_cmd = command.to_vec();

    wrap_with_taskset(&mut full_cmd, global_mf.as_ref(), game_mf.as_ref());

    // Wrap with GameScope if enabled
    let gamescope = game_mf
        .as_ref()
        .map(|g| &g.gamescope)
        .filter(|m| !matches!(m, FeatureState::Inherit))
        .or_else(|| global_mf.as_ref().map(|g| &g.gamescope));

    if let Some(FeatureState::Enabled(custom_args)) = gamescope {
        if is_command_available("gamescope") {
            let mut gs_cmd = vec!["gamescope".to_string()];
            let args_to_use = if custom_args.trim().is_empty() {
                crate::config::compute_default_gamescope_args()
            } else {
                custom_args.clone()
            };
            gs_cmd.extend(split_args(&args_to_use));
            gs_cmd.push("--".to_string());
            gs_cmd.extend(full_cmd);
            full_cmd = gs_cmd;
        }
    }

    // Wrap with GameMode if enabled
    let gamemode = game_mf
        .as_ref()
        .map(|g| &g.gamemode)
        .filter(|m| !matches!(m, FeatureState::Inherit))
        .or_else(|| global_mf.as_ref().map(|g| &g.gamemode));

    if let Some(FeatureState::Enabled(_)) = gamemode {
        if is_command_available("gamemoderun") {
            full_cmd.insert(0, "gamemoderun".to_string());
        }
    }

    let mut cmd = Command::new(&full_cmd[0]);
    cmd.args(&full_cmd[1..]);
    restore_renderer_env_for_child(&mut cmd, &effective);

    let err = cmd.exec();
    eprintln!("neutron: failed to exec {:?}: {}", full_cmd, err);
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_mode_preview_flag() {
        let args = vec!["neutron".to_string(), "--test".to_string(), "12345".to_string()];
        assert_eq!(
            detect_mode(&args),
            LaunchMode::Preview {
                app_id: "12345".to_string()
            }
        );

        let args_short = vec!["neutron".to_string(), "-t".to_string(), "12345".to_string()];
        assert_eq!(
            detect_mode(&args_short),
            LaunchMode::Preview {
                app_id: "12345".to_string()
            }
        );
    }

    #[test]
    fn test_detect_mode_compat_verb() {
        let args = vec![
            "neutron".to_string(),
            "getcompatpath".to_string(),
            "/path/to/game".to_string(),
        ];
        assert_eq!(
            detect_mode(&args),
            LaunchMode::CompatVerb {
                verb: "getcompatpath".to_string(),
                args: vec!["/path/to/game".to_string()],
            }
        );
    }

    #[test]
    fn test_is_helper_binary() {
        assert!(is_helper_binary(&[
            "/home/romulo/.local/share/Steam/legacycompat/iscriptevaluator.exe".to_string()
        ]));
        assert!(is_helper_binary(&[
            "C:\\_CommonRedist\\DirectX\\Jun2010\\DXSETUP.exe".to_string()
        ]));
        assert!(is_helper_binary(&["/path/to/vcredist_x64.exe".to_string()]));
        assert!(is_helper_binary(&["installscript.vdf".to_string()]));
        assert!(!is_helper_binary(&["/path/to/CONTROLResonant.exe".to_string()]));
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_detect_mode_silent_game_run() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            env::set_var("STEAM_COMPAT_APP_ID", "3669870");
        }
        let args = vec![
            "neutron".to_string(),
            "run".to_string(),
            "/home/romulo/.local/share/Steam/legacycompat/iscriptevaluator.exe".to_string(),
            "legacycompat/evaluatorscript_3669870.vdf".to_string(),
        ];
        assert_eq!(
            detect_mode(&args),
            LaunchMode::SilentGame {
                verb: "run".to_string(),
                app_id: "3669870".to_string(),
                args: vec![
                    "/home/romulo/.local/share/Steam/legacycompat/iscriptevaluator.exe".to_string(),
                    "legacycompat/evaluatorscript_3669870.vdf".to_string(),
                ],
            }
        );
        unsafe {
            env::remove_var("STEAM_COMPAT_APP_ID");
        }
    }

    #[test]
    fn test_detect_mode_silent_game_helper_waitforexitandrun() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            env::set_var("STEAM_COMPAT_APP_ID", "3669870");
        }
        let args = vec![
            "neutron".to_string(),
            "waitforexitandrun".to_string(),
            "/home/romulo/.local/share/Steam/legacycompat/iscriptevaluator.exe".to_string(),
        ];
        assert_eq!(
            detect_mode(&args),
            LaunchMode::SilentGame {
                verb: "waitforexitandrun".to_string(),
                app_id: "3669870".to_string(),
                args: vec!["/home/romulo/.local/share/Steam/legacycompat/iscriptevaluator.exe".to_string(),],
            }
        );
        unsafe {
            env::remove_var("STEAM_COMPAT_APP_ID");
        }
    }

    #[test]
    fn test_detect_mode_compat_game() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            env::set_var("STEAM_COMPAT_APP_ID", "3669870");
        }
        let args = vec![
            "neutron".to_string(),
            "waitforexitandrun".to_string(),
            "/path/to/game.exe".to_string(),
            "-dx12".to_string(),
        ];
        assert_eq!(
            detect_mode(&args),
            LaunchMode::CompatGame {
                verb: "waitforexitandrun".to_string(),
                app_id: "3669870".to_string(),
                args: vec!["/path/to/game.exe".to_string(), "-dx12".to_string()],
            }
        );
        unsafe {
            env::remove_var("STEAM_COMPAT_APP_ID");
        }
    }

    #[test]
    fn test_detect_mode_compat_game_steam_compat_app_id() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            env::set_var("STEAM_COMPAT_APP_ID", "3669870");
        }
        let args = vec![
            "neutron".to_string(),
            "waitforexitandrun".to_string(),
            "/path/to/game.exe".to_string(),
        ];
        assert_eq!(
            detect_mode(&args),
            LaunchMode::CompatGame {
                verb: "waitforexitandrun".to_string(),
                app_id: "3669870".to_string(),
                args: vec!["/path/to/game.exe".to_string()],
            }
        );
        unsafe {
            env::remove_var("STEAM_COMPAT_APP_ID");
        }
    }

    #[test]
    fn test_detect_mode_compat_game_data_path() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            env::set_var(
                "STEAM_COMPAT_DATA_PATH",
                "/home/romulo/.local/share/Steam/steamapps/compatdata/3669870",
            );
        }
        let args = vec![
            "neutron".to_string(),
            "waitforexitandrun".to_string(),
            "/path/to/game.exe".to_string(),
        ];
        assert_eq!(
            detect_mode(&args),
            LaunchMode::CompatGame {
                verb: "waitforexitandrun".to_string(),
                app_id: "3669870".to_string(),
                args: vec!["/path/to/game.exe".to_string()],
            }
        );
        unsafe {
            env::remove_var("STEAM_COMPAT_DATA_PATH");
        }
    }

    #[test]
    fn test_detect_mode_launch_with_steamappid() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            env::set_var("SteamAppId", "3669870");
        }
        let args = vec![
            "neutron".to_string(),
            "launch".to_string(),
            "/path/to/game.exe".to_string(),
            "-dx12".to_string(),
        ];
        assert_eq!(
            detect_mode(&args),
            LaunchMode::CommandWrap {
                app_id: "3669870".to_string(),
                command: vec!["/path/to/game.exe".to_string(), "-dx12".to_string()],
            }
        );
        unsafe {
            env::remove_var("SteamAppId");
        }
    }

    #[test]
    fn test_detect_mode_manager() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            env::remove_var("SteamAppId");
            env::remove_var("SteamGameId");
            env::remove_var("STEAM_COMPAT_APP_ID");
            env::remove_var("STEAM_COMPAT_DATA_PATH");
            env::remove_var("STEAM_COMPAT_INSTALL_PATH");
        }
        let args = vec!["neutron".to_string()];
        assert_eq!(detect_mode(&args), LaunchMode::Manager);
    }

    #[test]
    fn test_split_args() {
        assert_eq!(split_args(""), Vec::<String>::new());
        assert_eq!(split_args("-W 2560 -H 1440 -f"), vec!["-W", "2560", "-H", "1440", "-f"]);
        assert_eq!(
            split_args("--filter fsr --upscaler \"FSR 2.0\""),
            vec!["--filter", "fsr", "--upscaler", "FSR 2.0"]
        );
    }

    #[test]
    fn test_clap_cli_parsing() {
        // 1. --test and -t with and without value
        let cli = Cli::try_parse_from(["neutron", "-t", "3669870"]).unwrap();
        assert_eq!(cli.test, Some(Some("3669870".to_string())));
        assert!(cli.command.is_none());

        let cli = Cli::try_parse_from(["neutron", "--test"]).unwrap();
        assert_eq!(cli.test, Some(None));

        assert!(Cli::try_parse_from(["neutron", "--countdown", "3669870"]).is_err());

        // 2. run subcommand
        let cli = Cli::try_parse_from([
            "neutron",
            "run",
            "/home/romulo/.local/share/Steam/legacycompat/iscriptevaluator.exe",
            "script.vdf",
        ])
        .unwrap();
        assert_eq!(
            cli.command,
            Some(SteamCommand::Run {
                command: vec![
                    "/home/romulo/.local/share/Steam/legacycompat/iscriptevaluator.exe".to_string(),
                    "script.vdf".to_string(),
                ]
            })
        );

        // 3. waitforexitandrun subcommand with hyphenated args
        let cli =
            Cli::try_parse_from(["neutron", "waitforexitandrun", "/path/to/game.exe", "-dx12", "--novid"]).unwrap();
        assert_eq!(
            cli.command,
            Some(SteamCommand::Waitforexitandrun {
                command: vec![
                    "/path/to/game.exe".to_string(),
                    "-dx12".to_string(),
                    "--novid".to_string(),
                ]
            })
        );

        // 4. launch subcommand with %command% arguments
        let cli = Cli::try_parse_from(["neutron", "launch", "%command%", "-novid"]).unwrap();
        assert_eq!(
            cli.command,
            Some(SteamCommand::Launch {
                command: vec!["%command%".to_string(), "-novid".to_string()]
            })
        );

        // 5. Help (root and subcommands) and -v / --version flags
        let err_help = Cli::try_parse_from(["neutron", "--help"]).unwrap_err();
        assert_eq!(err_help.kind(), clap::error::ErrorKind::DisplayHelp);

        for subcmd in ["run", "waitforexitandrun", "launch"] {
            let err_sub_help = Cli::try_parse_from(["neutron", subcmd, "--help"]).unwrap_err();
            assert_eq!(err_sub_help.kind(), clap::error::ErrorKind::DisplayHelp);

            let err_sub_short = Cli::try_parse_from(["neutron", subcmd, "-h"]).unwrap_err();
            assert_eq!(err_sub_short.kind(), clap::error::ErrorKind::DisplayHelp);
        }

        let err_ver_short = Cli::try_parse_from(["neutron", "-v"]).unwrap_err();
        assert_eq!(err_ver_short.kind(), clap::error::ErrorKind::DisplayVersion);

        let err_ver_long = Cli::try_parse_from(["neutron", "--version"]).unwrap_err();
        assert_eq!(err_ver_long.kind(), clap::error::ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_preferred_proton_version_resolution() {
        let temp_dir = std::env::temp_dir().join(format!("neutron_launch_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        std::fs::write(
            temp_dir.join("global.env"),
            "NEUTRON_USE_PROTON=Proton - Experimental\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("1663850.env"),
            "NEUTRON_USE_PROTON=GE-Proton11-11-x86_64\n",
        )
        .unwrap();
        std::fs::write(temp_dir.join("12345.env"), "MANGOHUD=1\n").unwrap();

        let (global_mf, game_mf) = load_managed_features_from_dir(&temp_dir, "1663850");
        assert_eq!(
            preferred_proton_version(&global_mf, &game_mf),
            Some("GE-Proton11-11-x86_64")
        );

        let (global_mf, game_mf) = load_managed_features_from_dir(&temp_dir, "12345");
        assert_eq!(
            preferred_proton_version(&global_mf, &game_mf),
            Some("Proton - Experimental")
        );

        let (global_mf, game_mf) = load_managed_features_from_dir(&temp_dir, "");
        assert_eq!(
            preferred_proton_version(&global_mf, &game_mf),
            Some("Proton - Experimental")
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_clap_help_flag_passed_to_games() {
        let cli_wait = Cli::try_parse_from([
            "neutron",
            "waitforexitandrun",
            "/path/to/game.exe",
            "-w",
            "2560",
            "-h",
            "1440",
            "--help",
        ])
        .unwrap();
        assert_eq!(
            cli_wait.command,
            Some(SteamCommand::Waitforexitandrun {
                command: vec![
                    "/path/to/game.exe".to_string(),
                    "-w".to_string(),
                    "2560".to_string(),
                    "-h".to_string(),
                    "1440".to_string(),
                    "--help".to_string(),
                ],
            })
        );

        let cli_run = Cli::try_parse_from(["neutron", "run", "/path/to/installer.exe", "-h", "--help"]).unwrap();
        assert_eq!(
            cli_run.command,
            Some(SteamCommand::Run {
                command: vec![
                    "/path/to/installer.exe".to_string(),
                    "-h".to_string(),
                    "--help".to_string(),
                ],
            })
        );

        let cli_launch = Cli::try_parse_from(["neutron", "launch", "%command%", "-h", "1080", "--help"]).unwrap();
        assert_eq!(
            cli_launch.command,
            Some(SteamCommand::Launch {
                command: vec![
                    "%command%".to_string(),
                    "-h".to_string(),
                    "1080".to_string(),
                    "--help".to_string(),
                ],
            })
        );
    }

    #[test]
    fn test_ensure_app_id_envvars() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            env::remove_var("STEAM_COMPAT_APP_ID");
            env::remove_var("SteamAppId");
            env::remove_var("SteamGameId");
        }

        ensure_app_id_envvars("   ");
        assert!(env::var("STEAM_COMPAT_APP_ID").is_err());
        assert!(env::var("SteamAppId").is_err());
        assert!(env::var("SteamGameId").is_err());

        ensure_app_id_envvars(" 1663850 ");
        assert_eq!(env::var("STEAM_COMPAT_APP_ID").unwrap(), "1663850");
        assert_eq!(env::var("SteamAppId").unwrap(), "1663850");
        assert_eq!(env::var("SteamGameId").unwrap(), "1663850");

        // Fills empty/unset values while preserving non-empty existing ones
        unsafe {
            env::set_var("STEAM_COMPAT_APP_ID", "");
            env::set_var("SteamAppId", "1663850");
            env::set_var("SteamGameId", "999999");
        }
        ensure_app_id_envvars("1663850");
        assert_eq!(env::var("STEAM_COMPAT_APP_ID").unwrap(), "1663850");
        assert_eq!(env::var("SteamAppId").unwrap(), "1663850");
        assert_eq!(env::var("SteamGameId").unwrap(), "999999");

        unsafe {
            env::remove_var("STEAM_COMPAT_APP_ID");
            env::remove_var("SteamAppId");
            env::remove_var("SteamGameId");
        }
    }

    #[test]
    fn test_setup_tool_paths_library_paths_and_launcher_service() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            env::set_var(
                "STEAM_COMPAT_INSTALL_PATH",
                "/mnt/nvme/SteamLibrary/steamapps/common/Replaced",
            );
            env::set_var("STEAM_COMPAT_LIBRARY_PATHS", "/mnt/nvme/SteamLibrary/steamapps");
            env::set_var("STEAM_COMPAT_LAUNCHER_SERVICE", "auto");
            env::set_var(
                "STEAM_COMPAT_TOOL_PATHS",
                "/home/user/.local/share/Steam/compatibilitytools.d/neutron",
            );
        }

        setup_tool_paths(
            Path::new("/home/user/.local/share/Steam/steamapps/common/Proton - Experimental"),
            Some(Path::new(
                "/home/user/.local/share/Steam/steamapps/common/SteamLinuxRuntime_4",
            )),
        );

        assert_eq!(
            env::var("STEAM_COMPAT_TOOL_PATHS").unwrap(),
            "/home/user/.local/share/Steam/steamapps/common/Proton - Experimental:/home/user/.local/share/Steam/steamapps/common/SteamLinuxRuntime_4:/home/user/.local/share/Steam/compatibilitytools.d/neutron"
        );
        assert_eq!(
            env::var("STEAM_COMPAT_LIBRARY_PATHS").unwrap(),
            "/mnt/nvme/SteamLibrary/steamapps:/home/user/.local/share/Steam/steamapps"
        );
        assert_eq!(env::var("STEAM_COMPAT_LAUNCHER_SERVICE").unwrap(), "proton");

        // Existing non-auto service value and non-steamapps tool directory
        unsafe {
            env::set_var("STEAM_COMPAT_LAUNCHER_SERVICE", "container-runtime");
            env::set_var("STEAM_COMPAT_LIBRARY_PATHS", "/mnt/nvme/SteamLibrary/steamapps");
            env::remove_var("STEAM_COMPAT_INSTALL_PATH");
        }
        setup_tool_paths(
            Path::new("/usr/share/steam/compatibilitytools.d/proton-cachyos-slr"),
            None,
        );
        assert_eq!(env::var("STEAM_COMPAT_LAUNCHER_SERVICE").unwrap(), "container-runtime");
        assert_eq!(
            env::var("STEAM_COMPAT_LIBRARY_PATHS").unwrap(),
            "/mnt/nvme/SteamLibrary/steamapps"
        );

        unsafe {
            env::remove_var("STEAM_COMPAT_INSTALL_PATH");
            env::remove_var("STEAM_COMPAT_LIBRARY_PATHS");
            env::remove_var("STEAM_COMPAT_LAUNCHER_SERVICE");
            env::remove_var("STEAM_COMPAT_TOOL_PATHS");
        }
    }

    #[test]
    fn test_apply_restored_renderer_env() {
        use std::collections::HashMap;
        use std::ffi::OsStr;

        // 1. Originally unset vars are removed
        let mut cmd = Command::new("true");
        apply_restored_renderer_env(&mut cmd, &(None, None), &[]);
        let envs: HashMap<&OsStr, Option<&OsStr>> = cmd.get_envs().collect();
        assert_eq!(envs.get(OsStr::new("GSK_RENDERER")), Some(&None));
        assert_eq!(envs.get(OsStr::new("GDK_DISABLE")), Some(&None));

        // 2. Originally set vars are restored
        let mut cmd2 = Command::new("true");
        apply_restored_renderer_env(
            &mut cmd2,
            &(Some(OsString::from("ngl")), Some(OsString::from("offload"))),
            &[],
        );
        let envs2: HashMap<&OsStr, Option<&OsStr>> = cmd2.get_envs().collect();
        assert_eq!(envs2.get(OsStr::new("GSK_RENDERER")), Some(&Some(OsStr::new("ngl"))));
        assert_eq!(envs2.get(OsStr::new("GDK_DISABLE")), Some(&Some(OsStr::new("offload"))));

        // 3. Keys explicitly present in effective env are left untouched on cmd
        let mut cmd3 = Command::new("true");
        let effective = vec![("GSK_RENDERER".to_string(), "vulkan".to_string())];
        apply_restored_renderer_env(&mut cmd3, &(Some(OsString::from("ngl")), None), &effective);
        let envs3: HashMap<&OsStr, Option<&OsStr>> = cmd3.get_envs().collect();
        assert!(!envs3.contains_key(OsStr::new("GSK_RENDERER")));
        assert_eq!(envs3.get(OsStr::new("GDK_DISABLE")), Some(&None));
    }

    #[test]
    fn test_restore_appimage_env_for_child() {
        use std::collections::HashMap;
        use std::ffi::OsStr;

        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            env::set_var("NEUTRON_APPIMAGE_ENV", "1");
            env::set_var("NEUTRON_ORIG_LD_LIBRARY_PATH", "/usr/lib");
            env::set_var("NEUTRON_UNSET_GTK_PATH", "1");
            env::set_var("NEUTRON_ORIG_XDG_DATA_DIRS", "/usr/share");
        }

        let effective = vec![("XDG_DATA_DIRS".to_string(), "/custom/share".to_string())];
        let mut cmd = Command::new("true");
        restore_appimage_env_for_child(&mut cmd, &effective);

        let envs: HashMap<&OsStr, Option<&OsStr>> = cmd.get_envs().collect();
        assert_eq!(envs.get(OsStr::new("NEUTRON_APPIMAGE_ENV")), Some(&None));
        assert_eq!(
            envs.get(OsStr::new("LD_LIBRARY_PATH")),
            Some(&Some(OsStr::new("/usr/lib")))
        );
        assert_eq!(envs.get(OsStr::new("GTK_PATH")), Some(&None));
        // Overridden in effective -> untouched
        assert!(!envs.contains_key(OsStr::new("XDG_DATA_DIRS")));

        unsafe {
            env::remove_var("NEUTRON_APPIMAGE_ENV");
            env::remove_var("NEUTRON_ORIG_LD_LIBRARY_PATH");
            env::remove_var("NEUTRON_UNSET_GTK_PATH");
            env::remove_var("NEUTRON_ORIG_XDG_DATA_DIRS");
        }
    }
}
