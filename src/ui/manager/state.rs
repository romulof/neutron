use super::ManagerWindow;
use crate::config::EnvEntry;
use gtk4::prelude::*;

pub fn are_entries_equal(a: &[EnvEntry], b: &[EnvEntry]) -> bool {
    let normalize = |entries: &[EnvEntry]| -> Vec<(String, String, bool)> {
        let mut normalized: Vec<(String, String, bool)> = entries
            .iter()
            .filter_map(|e| {
                let k = e.key.trim();
                if k.is_empty() {
                    None
                } else {
                    Some((k.to_string(), e.value.trim().to_string(), e.enabled))
                }
            })
            .collect();
        normalized.sort();
        normalized
    };
    normalize(a) == normalize(b)
}

pub fn merge_managed_entries(
    custom_entries: &[EnvEntry],
    managed: &crate::config::ManagedFeatures,
    is_global: bool,
) -> Vec<EnvEntry> {
    let mut entries = custom_entries.to_vec();
    let mut mf = managed.clone();
    mf.sync_from_entries(&entries);
    mf.apply_to_entries(&mut entries, is_global);
    entries.retain(|e| !e.key.trim().is_empty());
    entries
}

pub fn compute_effective_entries(mgr: &ManagerWindow, idx: usize) -> Vec<EnvEntry> {
    let is_global = mgr.configs.borrow()[idx].id == "global";
    let entries = mgr.configs.borrow()[idx].entries.clone();
    let current_mf = mgr.current_managed.borrow().clone();
    if let Some(mf) = current_mf {
        merge_managed_entries(&entries, &mf, is_global)
    } else {
        entries.into_iter().filter(|e| !e.key.trim().is_empty()).collect()
    }
}

pub fn is_config_dirty(mgr: &ManagerWindow, idx: usize) -> bool {
    let disk_configs = mgr.disk_configs.borrow();
    if idx >= disk_configs.len() {
        return false;
    }
    let disk = &disk_configs[idx].entries;
    let selected = *mgr.selected_idx.borrow();
    if selected == Some(idx) {
        let current = compute_effective_entries(mgr, idx);
        !are_entries_equal(&current, disk)
    } else {
        let configs = mgr.configs.borrow();
        if idx >= configs.len() {
            return false;
        }
        let current = &configs[idx].entries;
        !are_entries_equal(current, disk)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BottomToolbarState {
    pub revert_enabled: bool,
    pub save_enabled: bool,
    pub launch_visible: bool,
    pub launch_enabled: bool,
    pub reveal_toolbar: bool,
}

pub fn compute_bottom_toolbar_state(
    selected_id: Option<&str>,
    cur_dirty: bool,
    launching_app_id: Option<&str>,
    has_prelaunch_callback: bool,
) -> BottomToolbarState {
    let is_game = selected_id.is_some_and(|id| !id.is_empty() && id != "global");
    let launch_visible = is_game;
    let launch_enabled = if !is_game {
        false
    } else if has_prelaunch_callback {
        launching_app_id.is_some() && launching_app_id == selected_id
    } else {
        true
    };
    let revert_enabled = cur_dirty;
    let save_enabled = cur_dirty;
    let reveal_toolbar = revert_enabled || save_enabled || (launch_visible && launch_enabled);

    BottomToolbarState {
        revert_enabled,
        save_enabled,
        launch_visible,
        launch_enabled,
        reveal_toolbar,
    }
}

fn selected_config_id(mgr: &ManagerWindow) -> Option<String> {
    let selected = *mgr.selected_idx.borrow();
    if let Some(idx) = selected {
        let configs = mgr.configs.borrow();
        configs.get(idx).map(|cfg| cfg.id.clone())
    } else {
        None
    }
}

pub fn can_launch(mgr: &ManagerWindow) -> bool {
    let selected_id = selected_config_id(mgr);
    let toolbar = compute_bottom_toolbar_state(
        selected_id.as_deref(),
        false,
        mgr.launching_app_id.as_deref(),
        mgr.on_launch.is_some(),
    );
    toolbar.launch_visible && toolbar.launch_enabled
}

pub fn update_dirty_state(mgr: &ManagerWindow) {
    let dirty_labels = mgr.sidebar_dirty_labels.borrow();
    for (idx, dot) in dirty_labels.iter().enumerate() {
        let dirty = is_config_dirty(mgr, idx);
        dot.set_visible(dirty);
    }

    let cur_dirty = match *mgr.selected_idx.borrow() {
        Some(idx) => is_config_dirty(mgr, idx),
        None => false,
    };
    let selected_id = selected_config_id(mgr);
    let toolbar = compute_bottom_toolbar_state(
        selected_id.as_deref(),
        cur_dirty,
        mgr.launching_app_id.as_deref(),
        mgr.on_launch.is_some(),
    );

    mgr.save_button.set_sensitive(toolbar.save_enabled);
    mgr.revert_button.set_sensitive(toolbar.revert_enabled);
    mgr.launch_button.set_visible(toolbar.launch_visible);
    mgr.launch_button.set_sensitive(toolbar.launch_enabled);

    if toolbar.save_enabled {
        mgr.save_button.add_css_class("suggested-action");
        mgr.launch_button.remove_css_class("suggested-action");
    } else {
        mgr.save_button.remove_css_class("suggested-action");
        if toolbar.launch_visible && toolbar.launch_enabled {
            mgr.launch_button.add_css_class("suggested-action");
        } else {
            mgr.launch_button.remove_css_class("suggested-action");
        }
    }

    if !toolbar.launch_visible {
        mgr.launch_button.set_tooltip_text(None);
    } else if toolbar.launch_enabled && mgr.on_launch.is_some() {
        mgr.launch_button.set_tooltip_text(Some("Launch game"));
    } else if toolbar.launch_enabled {
        mgr.launch_button.set_tooltip_text(Some("Launch game via Steam"));
    } else {
        mgr.launch_button
            .set_tooltip_text(Some("Another game launch is in progress"));
    }

    mgr.content_toolbar_view.set_reveal_bottom_bars(toolbar.reveal_toolbar);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, value: &str, enabled: bool) -> EnvEntry {
        EnvEntry {
            key: key.to_string(),
            value: value.to_string(),
            enabled,
            comment_above: None,
        }
    }

    #[test]
    fn test_are_entries_equal_enabled_toggle_and_order() {
        let a = vec![entry("FOO", "1", true), entry("BAR", "2", false)];
        let b = vec![entry("BAR", "2", false), entry("FOO", "1", true)];
        assert!(are_entries_equal(&a, &b));

        let c = vec![entry("FOO", "1", false), entry("BAR", "2", false)];
        assert!(!are_entries_equal(&a, &c));
    }

    #[test]
    fn test_are_entries_equal_ignores_empty_keys_and_handles_duplicates() {
        let a = vec![entry("  ", "ignored", true), entry("FOO", "1", true)];
        let b = vec![entry("FOO", "1", true)];
        assert!(are_entries_equal(&a, &b));

        let dup_a = vec![entry("FOO", "1", true), entry("FOO", "2", false)];
        let dup_b = vec![entry("FOO", "2", false), entry("FOO", "1", true)];
        let dup_c = vec![entry("FOO", "2", true), entry("FOO", "1", true)];
        assert!(are_entries_equal(&dup_a, &dup_b));
        assert!(!are_entries_equal(&dup_a, &dup_c));
    }

    #[test]
    fn test_managed_override_save_then_toggle_or_uncheck_is_dirty() {
        use crate::config::{FeatureState, ManagedFeatures};

        // Initial game config on disk has only a custom envvar; gamemode inherits global.
        let mut disk = vec![entry("DXVK_HDR", "1", true)];
        let mut mf = ManagedFeatures::extract(&disk, false);
        let mut custom: Vec<EnvEntry> = disk
            .iter()
            .filter(|e| !ManagedFeatures::is_managed_key(e.key.trim()))
            .cloned()
            .collect();
        assert_eq!(mf.gamemode, FeatureState::Inherit);
        assert!(are_entries_equal(&merge_managed_entries(&custom, &mf, false), &disk));

        // 1. User checks GameMode override (switch stays enabled).
        mf.gamemode = FeatureState::Enabled(String::new());
        let effective_before_save = merge_managed_entries(&custom, &mf, false);
        assert!(!are_entries_equal(&effective_before_save, &disk));

        // 2. Simulate Save: disk becomes saved effective entries, and active custom list strips managed keys.
        disk = effective_before_save;
        mf = ManagedFeatures::extract(&disk, false);
        custom = disk
            .iter()
            .filter(|e| !ManagedFeatures::is_managed_key(e.key.trim()))
            .cloned()
            .collect();
        assert!(are_entries_equal(&merge_managed_entries(&custom, &mf, false), &disk));

        // 3. Flip GameMode toggle OFF -> must be dirty (NEUTRON_USE_GAMEMODE=0 vs 1).
        mf.gamemode = FeatureState::Disabled;
        assert!(!are_entries_equal(&merge_managed_entries(&custom, &mf, false), &disk));

        // 4. Uncheck GameMode override -> must be dirty (absent vs 1).
        mf.gamemode = FeatureState::Inherit;
        assert!(!are_entries_equal(&merge_managed_entries(&custom, &mf, false), &disk));

        // 5. Re-enable GameMode override -> matches disk cleanly again.
        mf.gamemode = FeatureState::Enabled(String::new());
        assert!(are_entries_equal(&merge_managed_entries(&custom, &mf, false), &disk));

        // 6. Manually typing a managed key in the custom table is absorbed via sync_from_entries.
        custom.push(entry("MANGOHUD", "1", true));
        assert!(!are_entries_equal(&merge_managed_entries(&custom, &mf, false), &disk));
    }

    #[test]
    fn test_compute_bottom_toolbar_state() {
        // Standalone mode: None selected
        assert_eq!(
            compute_bottom_toolbar_state(None, false, None, false),
            BottomToolbarState {
                revert_enabled: false,
                save_enabled: false,
                launch_visible: false,
                launch_enabled: false,
                reveal_toolbar: false,
            }
        );

        // Standalone mode: Global clean -> hidden, no launch button
        assert_eq!(
            compute_bottom_toolbar_state(Some("global"), false, None, false),
            BottomToolbarState {
                revert_enabled: false,
                save_enabled: false,
                launch_visible: false,
                launch_enabled: false,
                reveal_toolbar: false,
            }
        );

        // Standalone mode: Global dirty -> revealed, no launch button
        assert_eq!(
            compute_bottom_toolbar_state(Some("global"), true, None, false),
            BottomToolbarState {
                revert_enabled: true,
                save_enabled: true,
                launch_visible: false,
                launch_enabled: false,
                reveal_toolbar: true,
            }
        );

        // Standalone mode: Game clean -> launch visible+enabled, revealed
        assert_eq!(
            compute_bottom_toolbar_state(Some("100"), false, None, false),
            BottomToolbarState {
                revert_enabled: false,
                save_enabled: false,
                launch_visible: true,
                launch_enabled: true,
                reveal_toolbar: true,
            }
        );

        // Pre-launch mode (launching 100): Global clean -> hidden, no launch button
        assert_eq!(
            compute_bottom_toolbar_state(Some("global"), false, Some("100"), true),
            BottomToolbarState {
                revert_enabled: false,
                save_enabled: false,
                launch_visible: false,
                launch_enabled: false,
                reveal_toolbar: false,
            }
        );

        // Pre-launch mode (launching 100): Global dirty -> revealed, no launch button
        assert_eq!(
            compute_bottom_toolbar_state(Some("global"), true, Some("100"), true),
            BottomToolbarState {
                revert_enabled: true,
                save_enabled: true,
                launch_visible: false,
                launch_enabled: false,
                reveal_toolbar: true,
            }
        );

        // Pre-launch mode (launching 100): Launching game clean -> launch visible+enabled, revealed
        assert_eq!(
            compute_bottom_toolbar_state(Some("100"), false, Some("100"), true),
            BottomToolbarState {
                revert_enabled: false,
                save_enabled: false,
                launch_visible: true,
                launch_enabled: true,
                reveal_toolbar: true,
            }
        );

        // Pre-launch mode (launching 100): Other game clean -> launch visible+disabled, toolbar hidden
        assert_eq!(
            compute_bottom_toolbar_state(Some("200"), false, Some("100"), true),
            BottomToolbarState {
                revert_enabled: false,
                save_enabled: false,
                launch_visible: true,
                launch_enabled: false,
                reveal_toolbar: false,
            }
        );

        // Pre-launch mode (launching 100): Other game dirty -> launch visible+disabled, toolbar revealed
        assert_eq!(
            compute_bottom_toolbar_state(Some("200"), true, Some("100"), true),
            BottomToolbarState {
                revert_enabled: true,
                save_enabled: true,
                launch_visible: true,
                launch_enabled: false,
                reveal_toolbar: true,
            }
        );
    }
}
