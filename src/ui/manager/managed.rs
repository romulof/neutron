use super::ManagerWindow;
use crate::config::{FeatureState, ManagedFeatures, compute_shader_cache_envvars, compute_vram_envvars};
use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

pub fn build_managed_groups(mgr: &ManagerWindow, idx: usize) -> Vec<adw::PreferencesGroup> {
    let is_global = mgr.configs.borrow()[idx].id == "global";
    let managed = mgr
        .current_managed
        .borrow()
        .clone()
        .unwrap_or_else(|| ManagedFeatures::extract(&mgr.configs.borrow()[idx].entries, is_global));
    let disk_managed = ManagedFeatures::extract(&mgr.disk_configs.borrow()[idx].entries, is_global);

    // Global managed features reference for games inheriting global settings
    let global_managed = if !is_global {
        let configs = mgr.configs.borrow();
        configs
            .iter()
            .find(|c| c.id == "global")
            .map(|c| ManagedFeatures::extract(&c.entries, true))
    } else {
        None
    };

    let mut groups = Vec::new();

    // 0. Launcher timeout (global only)
    if is_global {
        let timeout_group = adw::PreferencesGroup::builder().build();
        let timeout_row = build_timeout_row(mgr, &managed, &disk_managed);
        timeout_group.add(&timeout_row);
        groups.push(timeout_group);
    }

    // 1. Proton version
    let proton_group = adw::PreferencesGroup::builder().build();
    let proton_row = build_proton_row(mgr, &managed, &disk_managed, global_managed.as_ref(), is_global);
    proton_group.add(&proton_row);
    groups.push(proton_group);

    // 2. Wrappers: GameMode, MangoHud, GameScope
    let wrappers_group = adw::PreferencesGroup::builder().build();

    let has_gamemode = crate::config::is_command_available("gamemoderun");
    let has_mangohud = crate::config::is_command_available("mangohud");
    let has_gamescope = crate::config::is_command_available("gamescope");

    // GameMode
    {
        let mgr_gm = mgr.clone();
        let row = build_expander_feature_row(
            "GameMode",
            None,
            "Arguments",
            "",
            None,
            &managed.gamemode,
            &disk_managed.gamemode,
            global_managed.as_ref().map(|gm| &gm.gamemode),
            has_gamemode,
            Some("gamemoderun"),
            is_global,
            mgr.clone(),
            move |new_state| {
                if let Some(mf) = mgr_gm.current_managed.borrow_mut().as_mut() {
                    mf.gamemode = new_state;
                }
            },
        );
        wrappers_group.add(&row);
    }

    // MangoHud
    {
        let mgr_mh = mgr.clone();
        let row = build_expander_feature_row(
            "MangoHud",
            None,
            "Config",
            "",
            None,
            &managed.mangohud,
            &disk_managed.mangohud,
            global_managed.as_ref().map(|gm| &gm.mangohud),
            has_mangohud,
            Some("mangohud"),
            is_global,
            mgr.clone(),
            move |new_state| {
                if let Some(mf) = mgr_mh.current_managed.borrow_mut().as_mut() {
                    mf.mangohud = new_state;
                }
            },
        );
        wrappers_group.add(&row);
    }

    // GameScope
    {
        let mgr_gs = mgr.clone();
        let default_gamescope_args = crate::config::compute_default_gamescope_args();
        let row = build_expander_feature_row(
            "GameScope",
            None,
            "Arguments",
            &default_gamescope_args,
            Some(&default_gamescope_args),
            &managed.gamescope,
            &disk_managed.gamescope,
            global_managed.as_ref().map(|gm| &gm.gamescope),
            has_gamescope,
            Some("gamescope"),
            is_global,
            mgr.clone(),
            move |new_state| {
                if let Some(mf) = mgr_gs.current_managed.borrow_mut().as_mut() {
                    mf.gamescope = new_state;
                }
            },
        );
        wrappers_group.add(&row);
    }

    // Disable SMT
    {
        let mgr_smt = mgr.clone();
        let has_taskset = crate::config::is_command_available("taskset");
        let has_smt = crate::config::is_smt_supported_and_active();
        let is_smt_available = has_taskset && has_smt;

        let subtitle = if !has_taskset {
            "taskset not found on PATH".to_string()
        } else if has_smt {
            let phys_count = crate::config::get_physical_cpu_count();
            format!(
                "Run game exclusively on physical CPU cores ({} cores, {} threads)",
                phys_count, phys_count
            )
        } else {
            "SMT is not active on system".to_string()
        };

        let tool_binary = if !has_taskset { Some("taskset") } else { None };

        let row = build_toggle_feature_row(
            "Disable SMT",
            Some(&subtitle),
            &managed.disable_smt,
            &disk_managed.disable_smt,
            global_managed.as_ref().map(|gm| &gm.disable_smt),
            is_smt_available,
            tool_binary,
            is_global,
            mgr.clone(),
            move |new_state| {
                if let Some(mf) = mgr_smt.current_managed.borrow_mut().as_mut() {
                    mf.disable_smt = new_state;
                }
            },
        );
        wrappers_group.add(&row);
    }
    groups.push(wrappers_group);

    // 3. Managed configurations: headroom, shader cache size, and DLSS overrides
    let managed_configs_group = adw::PreferencesGroup::builder().build();
    let vram_row = build_vram_row(mgr, &managed, &disk_managed, global_managed.as_ref(), is_global);
    managed_configs_group.add(&vram_row);

    let shader_row = build_shader_cache_row(mgr, &managed, &disk_managed, global_managed.as_ref(), is_global);
    managed_configs_group.add(&shader_row);

    if crate::config::supports_dlss() {
        let dlss_row = build_dlss_row(mgr, &managed, &disk_managed, global_managed.as_ref(), is_global);
        managed_configs_group.add(&dlss_row);
    }

    groups.push(managed_configs_group);

    groups
}

fn build_proton_row(
    mgr: &ManagerWindow,
    managed: &ManagedFeatures,
    disk_managed: &ManagedFeatures,
    global_managed: Option<&ManagedFeatures>,
    is_global: bool,
) -> adw::ActionRow {
    let mut available_versions = crate::config::discover_proton_versions();
    if let Some(ref current_pv) = managed.proton_version {
        if !available_versions.contains(current_pv) {
            available_versions.push(current_pv.clone());
        }
    }
    if let Some(ref gm) = global_managed {
        if let Some(ref g_pv) = gm.proton_version {
            if !available_versions.contains(g_pv) {
                available_versions.push(g_pv.clone());
            }
        }
    }

    let is_overridden = !is_global && managed.proton_version.is_some();
    let current_ver = if is_overridden {
        managed
            .proton_version
            .as_deref()
            .or_else(|| global_managed.as_ref().and_then(|gm| gm.proton_version.as_deref()))
            .unwrap_or("proton-cachyos-slr")
    } else {
        global_managed
            .as_ref()
            .and_then(|gm| gm.proton_version.as_deref())
            .unwrap_or("proton-cachyos-slr")
    };

    let selected_ver_idx = available_versions.iter().position(|v| v == current_ver).unwrap_or(0);
    let string_list = gtk4::StringList::new(&available_versions.iter().map(|s| s.as_str()).collect::<Vec<_>>());

    let row = adw::ActionRow::builder()
        .title("Proton version")
        .subtitle("Applies when running as Steam compatibility tool")
        .build();

    if managed.proton_version != disk_managed.proton_version {
        row.add_css_class("modified-row");
    }

    let dropdown = gtk4::DropDown::builder()
        .model(&string_list)
        .selected(selected_ver_idx as u32)
        .valign(gtk4::Align::Center)
        .sensitive(is_global || is_overridden)
        .build();

    if !is_global {
        let override_check = gtk4::CheckButton::builder()
            .active(is_overridden)
            .valign(gtk4::Align::Center)
            .tooltip_text("Override global configuration")
            .build();

        let row_clone = row.clone();
        let dropdown_clone = dropdown.clone();
        let mgr_check = mgr.clone();
        let versions = available_versions.clone();
        let g_ver_str = global_managed
            .as_ref()
            .and_then(|gm| gm.proton_version.clone())
            .unwrap_or_else(|| "proton-cachyos-slr".to_string());
        let g_ver_idx = versions.iter().position(|v| v == &g_ver_str).unwrap_or(0);
        let disk_pv = disk_managed.proton_version.clone();

        override_check.connect_toggled(move |btn| {
            let is_active = btn.is_active();
            dropdown_clone.set_sensitive(is_active);
            if is_active {
                let sel = dropdown_clone.selected() as usize;
                let ver = versions.get(sel).cloned().unwrap_or_else(|| g_ver_str.clone());
                if let Some(mf) = mgr_check.current_managed.borrow_mut().as_mut() {
                    mf.proton_version = Some(ver);
                }
            } else {
                dropdown_clone.set_selected(g_ver_idx as u32);
                if let Some(mf) = mgr_check.current_managed.borrow_mut().as_mut() {
                    mf.proton_version = None;
                }
            }
            let cur_pv = mgr_check
                .current_managed
                .borrow()
                .as_ref()
                .and_then(|mf| mf.proton_version.clone());
            if cur_pv != disk_pv {
                row_clone.add_css_class("modified-row");
            } else {
                row_clone.remove_css_class("modified-row");
            }
            mgr_check.update_dirty_state();
        });

        row.add_prefix(&override_check);
    }

    row.add_suffix(&dropdown);

    {
        let mgr_combo = mgr.clone();
        let versions = available_versions;
        let row_clone = row.clone();
        let disk_pv = disk_managed.proton_version.clone();
        dropdown.connect_selected_notify(move |combo| {
            if !combo.is_sensitive() {
                return;
            }
            let sel = combo.selected() as usize;
            if let Some(ver) = versions.get(sel) {
                if let Some(mf) = mgr_combo.current_managed.borrow_mut().as_mut() {
                    mf.proton_version = Some(ver.clone());
                }
            }
            let cur_pv = mgr_combo
                .current_managed
                .borrow()
                .as_ref()
                .and_then(|mf| mf.proton_version.clone());
            if cur_pv != disk_pv {
                row_clone.add_css_class("modified-row");
            } else {
                row_clone.remove_css_class("modified-row");
            }
            mgr_combo.update_dirty_state();
        });
    }

    row
}

fn non_activatable_row() -> adw::PreferencesRow {
    adw::PreferencesRow::builder()
        .activatable(false)
        .selectable(false)
        .focusable(false)
        .build()
}

fn build_expander_feature_row(
    title: &str,
    subtitle: Option<&str>,
    extra_title: &str,
    extra_placeholder: &str,
    inject_defaults: Option<&str>,
    initial_state: &FeatureState,
    disk_state: &FeatureState,
    global_state: Option<&FeatureState>,
    is_available: bool,
    tool_binary: Option<&str>,
    is_global: bool,
    mgr: ManagerWindow,
    on_change: impl Fn(FeatureState) + 'static,
) -> adw::ExpanderRow {
    let on_change = Rc::new(on_change);
    let extra_text = Rc::new(RefCell::new(initial_state.extra_args().to_string()));

    let expander = adw::ExpanderRow::builder().title(title).build();

    if initial_state != disk_state {
        expander.add_css_class("modified-row");
    }

    if let Some(sub) = subtitle {
        expander.set_subtitle(sub);
    } else if !is_available {
        expander.set_subtitle("Not installed on system");
    }

    let is_overridden = is_global || !matches!(initial_state, FeatureState::Inherit);
    let effective_state = if is_overridden {
        initial_state
    } else {
        global_state.unwrap_or(&FeatureState::Disabled)
    };

    let is_controls_sensitive = is_available && is_overridden;

    if !is_available {
        expander.set_sensitive(false);
        if let Some(tool) = tool_binary {
            expander.set_tooltip_text(Some(&format!("'{}' not found on PATH. Install it to enable.", tool)));
        }
    }

    let pref_row = non_activatable_row();
    let vbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(4)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .build();

    let header_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(6)
        .build();

    let title_label = gtk4::Label::builder()
        .label(extra_title)
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .css_classes(["dim-label"])
        .build();

    header_box.append(&title_label);

    let entry = gtk4::Entry::builder()
        .text(effective_state.extra_args())
        .placeholder_text(extra_placeholder)
        .hexpand(true)
        .css_classes(["monospace"])
        .editable(is_controls_sensitive && effective_state.is_enabled())
        .sensitive(is_controls_sensitive && effective_state.is_enabled())
        .build();

    let inject_btn = if let Some(def_args) = inject_defaults {
        let def_str = def_args.to_string();
        let entry_clone = entry.clone();
        let btn = gtk4::Button::builder()
            .label("Insert defaults")
            .tooltip_text("Insert default configuration into input")
            .valign(gtk4::Align::Center)
            .halign(gtk4::Align::End)
            .css_classes(["flat", "caption"])
            .sensitive(is_controls_sensitive && effective_state.is_enabled())
            .build();

        btn.connect_clicked(move |_| {
            entry_clone.set_text(&def_str);
            entry_clone.grab_focus();
        });

        header_box.append(&btn);
        Some(btn)
    } else {
        None
    };

    vbox.append(&header_box);
    vbox.append(&entry);
    pref_row.set_child(Some(&vbox));

    expander.add_row(&pref_row);

    let has_args = !effective_state.extra_args().trim().is_empty();
    if is_available && effective_state.is_enabled() && has_args {
        expander.set_expanded(true);
    }

    let switch = gtk4::Switch::builder()
        .active(effective_state.is_enabled())
        .valign(gtk4::Align::Center)
        .sensitive(is_controls_sensitive)
        .build();

    let override_check = if !is_global {
        let check = gtk4::CheckButton::builder()
            .active(is_overridden)
            .valign(gtk4::Align::Center)
            .tooltip_text("Override global configuration")
            .sensitive(is_available)
            .build();

        let expander_clone = expander.clone();
        let switch_clone = switch.clone();
        let entry_clone = entry.clone();
        let inject_btn_clone = inject_btn.clone();
        let on_change_clone = on_change.clone();
        let g_state = global_state.cloned().unwrap_or(FeatureState::Disabled);
        let disk_state_clone = disk_state.clone();
        let mgr_toggle = mgr.clone();

        check.connect_toggled(move |btn| {
            let is_active = btn.is_active();
            if is_active {
                switch_clone.set_sensitive(is_available);
                let active = switch_clone.is_active();
                entry_clone.set_sensitive(is_available && active);
                entry_clone.set_editable(is_available && active);
                if let Some(b) = inject_btn_clone.as_ref() {
                    b.set_sensitive(is_available && active);
                }
                let current_args = entry_clone.text().to_string();
                let new_state = if active {
                    FeatureState::Enabled(current_args)
                } else {
                    FeatureState::Disabled
                };
                if new_state != disk_state_clone {
                    expander_clone.add_css_class("modified-row");
                } else {
                    expander_clone.remove_css_class("modified-row");
                }
                on_change_clone(new_state);
            } else {
                switch_clone.set_sensitive(false);
                entry_clone.set_sensitive(false);
                entry_clone.set_editable(false);
                if let Some(b) = inject_btn_clone.as_ref() {
                    b.set_sensitive(false);
                }
                switch_clone.set_active(g_state.is_enabled());
                entry_clone.set_text(g_state.extra_args());
                if !g_state.extra_args().trim().is_empty() && g_state.is_enabled() {
                    expander_clone.set_expanded(true);
                } else {
                    expander_clone.set_expanded(false);
                }
                on_change_clone(FeatureState::Inherit);
                if disk_state_clone != FeatureState::Inherit {
                    expander_clone.add_css_class("modified-row");
                } else {
                    expander_clone.remove_css_class("modified-row");
                }
            }
            mgr_toggle.update_dirty_state();
        });

        expander.add_prefix(&check);
        Some(check)
    } else {
        None
    };

    expander.add_suffix(&switch);

    {
        let expander = expander.clone();
        let on_change = on_change.clone();
        let extra_text = extra_text.clone();
        let entry = entry.clone();
        let inject_btn = inject_btn;
        let override_check = override_check.clone();
        let disk_state = disk_state.clone();
        let mgr_switch = mgr.clone();

        switch.connect_state_set(move |_, active| {
            if !is_available || !override_check.as_ref().map(|c| c.is_active()).unwrap_or(true) {
                return glib::Propagation::Proceed;
            }
            entry.set_editable(active);
            entry.set_sensitive(active);
            if let Some(btn) = inject_btn.as_ref() {
                btn.set_sensitive(active);
            }
            let new_state = if active {
                let text = extra_text.borrow().clone();
                if !text.trim().is_empty() {
                    expander.set_expanded(true);
                }
                FeatureState::Enabled(text)
            } else {
                expander.set_expanded(false);
                FeatureState::Disabled
            };
            if new_state != disk_state {
                expander.add_css_class("modified-row");
            } else {
                expander.remove_css_class("modified-row");
            }
            on_change(new_state);
            mgr_switch.update_dirty_state();
            glib::Propagation::Proceed
        });
    }

    {
        let switch = switch;
        let expander = expander.clone();
        let on_change = on_change;
        let extra_text = extra_text;
        let override_check = override_check;
        let disk_state = disk_state.clone();
        let mgr_entry = mgr;

        entry.connect_changed(move |e| {
            if !e.is_editable() || !override_check.as_ref().map(|c| c.is_active()).unwrap_or(true) {
                return;
            }
            let raw = e.text().to_string();
            *extra_text.borrow_mut() = raw.clone();
            if switch.is_active() {
                let new_state = FeatureState::Enabled(raw);
                if new_state != disk_state {
                    expander.add_css_class("modified-row");
                } else {
                    expander.remove_css_class("modified-row");
                }
                on_change(new_state);
                mgr_entry.update_dirty_state();
            }
        });
    }

    expander
}

fn build_toggle_feature_row(
    title: &str,
    subtitle: Option<&str>,
    initial_state: &FeatureState,
    disk_state: &FeatureState,
    global_state: Option<&FeatureState>,
    is_available: bool,
    tool_binary: Option<&str>,
    is_global: bool,
    mgr: ManagerWindow,
    on_change: impl Fn(FeatureState) + 'static,
) -> adw::ActionRow {
    let on_change = Rc::new(on_change);
    let row = adw::ActionRow::builder().title(title).build();

    if initial_state != disk_state {
        row.add_css_class("modified-row");
    }

    if let Some(sub) = subtitle {
        row.set_subtitle(sub);
    } else if !is_available {
        row.set_subtitle("Not installed on system");
    }

    let is_overridden = is_global || !matches!(initial_state, FeatureState::Inherit);
    let effective_state = if is_overridden {
        initial_state
    } else {
        global_state.unwrap_or(&FeatureState::Disabled)
    };

    let is_controls_sensitive = is_available && is_overridden;

    if !is_available {
        row.set_sensitive(false);
        if let Some(tool) = tool_binary {
            row.set_tooltip_text(Some(&format!("'{}' not found on PATH. Install it to enable.", tool)));
        }
    }

    let switch = gtk4::Switch::builder()
        .active(effective_state.is_enabled())
        .valign(gtk4::Align::Center)
        .sensitive(is_controls_sensitive)
        .build();

    let override_check = if !is_global {
        let check = gtk4::CheckButton::builder()
            .active(is_overridden)
            .valign(gtk4::Align::Center)
            .tooltip_text("Override global configuration")
            .sensitive(is_available)
            .build();

        let row_clone = row.clone();
        let switch_clone = switch.clone();
        let on_change_clone = on_change.clone();
        let g_state = global_state.cloned().unwrap_or(FeatureState::Disabled);
        let disk_state_clone = disk_state.clone();
        let mgr_toggle = mgr.clone();

        check.connect_toggled(move |btn| {
            let is_active = btn.is_active();
            if is_active {
                switch_clone.set_sensitive(is_available);
                let active = switch_clone.is_active();
                let new_state = if active {
                    FeatureState::Enabled(String::new())
                } else {
                    FeatureState::Disabled
                };
                if new_state != disk_state_clone {
                    row_clone.add_css_class("modified-row");
                } else {
                    row_clone.remove_css_class("modified-row");
                }
                on_change_clone(new_state);
            } else {
                switch_clone.set_sensitive(false);
                switch_clone.set_active(g_state.is_enabled());
                on_change_clone(FeatureState::Inherit);
                if disk_state_clone != FeatureState::Inherit {
                    row_clone.add_css_class("modified-row");
                } else {
                    row_clone.remove_css_class("modified-row");
                }
            }
            mgr_toggle.update_dirty_state();
        });

        row.add_prefix(&check);
        Some(check)
    } else {
        None
    };

    row.add_suffix(&switch);

    {
        let row = row.clone();
        let disk_state = disk_state.clone();
        switch.connect_state_set(move |_, active| {
            if !is_available || !override_check.as_ref().map(|c| c.is_active()).unwrap_or(true) {
                return glib::Propagation::Proceed;
            }
            let new_state = if active {
                FeatureState::Enabled(String::new())
            } else {
                FeatureState::Disabled
            };
            if new_state != disk_state {
                row.add_css_class("modified-row");
            } else {
                row.remove_css_class("modified-row");
            }
            on_change(new_state);
            mgr.update_dirty_state();
            glib::Propagation::Proceed
        });
    }

    row
}

fn build_env_table(entries: &[(&str, String)]) -> gtk4::Widget {
    if entries.is_empty() {
        return gtk4::Label::builder()
            .label("No environment variables set")
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label"])
            .margin_top(4)
            .margin_bottom(4)
            .build()
            .upcast();
    }

    let grid = gtk4::Grid::builder()
        .column_homogeneous(true)
        .column_spacing(12)
        .row_spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .build();

    let key_hdr = gtk4::Label::builder()
        .label("VARIABLE")
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .css_classes(["heading", "dim-label", "table-header"])
        .build();
    let val_hdr = gtk4::Label::builder()
        .label("VALUE")
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .css_classes(["heading", "dim-label", "table-header"])
        .build();

    grid.attach(&key_hdr, 0, 0, 1, 1);
    grid.attach(&val_hdr, 1, 0, 1, 1);

    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    grid.attach(&sep, 0, 1, 2, 1);

    for (row, (k, v)) in entries.iter().enumerate() {
        let key_label = gtk4::Label::builder()
            .label(*k)
            .halign(gtk4::Align::Start)
            .valign(gtk4::Align::Center)
            .hexpand(true)
            .selectable(true)
            .css_classes(["monospace"])
            .build();
        let val_label = gtk4::Label::builder()
            .label(v)
            .halign(gtk4::Align::Start)
            .valign(gtk4::Align::Center)
            .hexpand(true)
            .selectable(true)
            .css_classes(["monospace"])
            .build();

        grid.attach(&key_label, 0, (row + 2) as i32, 1, 1);
        grid.attach(&val_label, 1, (row + 2) as i32, 1, 1);
    }

    grid.upcast()
}

fn build_vram_row(
    mgr: &ManagerWindow,
    managed: &ManagedFeatures,
    disk_managed: &ManagedFeatures,
    global_managed: Option<&ManagedFeatures>,
    is_global: bool,
) -> adw::ExpanderRow {
    let is_overridden = is_global || !matches!(managed.reserve_vram, FeatureState::Inherit);
    let disk_vram = disk_managed.reserve_vram.clone();

    let global_mb = global_managed
        .as_ref()
        .and_then(|gm| {
            if gm.reserve_vram.is_enabled() {
                Some(gm.reserve_vram.extra_args().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "0".to_string());

    let initial_mb = if is_overridden {
        match &managed.reserve_vram {
            FeatureState::Enabled(mb) => mb.clone(),
            FeatureState::Disabled => "0".to_string(),
            FeatureState::Inherit => global_mb.clone(),
        }
    } else {
        global_mb.clone()
    };

    let vendor = crate::config::detect_gpu_vendor();

    let expander = adw::ExpanderRow::builder().title("Reserve VRAM headroom").build();

    if managed.reserve_vram != disk_managed.reserve_vram {
        expander.add_css_class("modified-row");
    }

    let is_vram_editable = is_global || is_overridden;

    let overlay = gtk4::Overlay::builder()
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::End)
        .build();

    let vram_entry = gtk4::Entry::builder()
        .text(&initial_mb)
        .placeholder_text("0")
        .tooltip_text("Use 0 to disable")
        .input_purpose(gtk4::InputPurpose::Digits)
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::End)
        .width_chars(6)
        .max_width_chars(8)
        .editable(is_vram_editable)
        .sensitive(is_vram_editable)
        .css_classes(["monospace", "vram-entry"])
        .build();

    gtk4::prelude::EditableExt::set_alignment(&vram_entry, 1.0);

    let unit_label = gtk4::Label::builder()
        .label("MiB")
        .can_target(false)
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::End)
        .margin_end(8)
        .css_classes(["dim-label"])
        .sensitive(is_vram_editable)
        .build();

    overlay.set_child(Some(&vram_entry));
    overlay.add_overlay(&unit_label);

    // Expanded section
    let pref_row = non_activatable_row();
    let vbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(6)
        .margin_top(8)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();

    let table_box = gtk4::Box::builder().orientation(gtk4::Orientation::Vertical).build();

    let update_table = {
        let table_box = table_box.clone();
        move |mb_str: &str| {
            while let Some(child) = table_box.first_child() {
                table_box.remove(&child);
            }
            let envs = compute_vram_envvars(mb_str, vendor);
            let widget = build_env_table(&envs);
            table_box.append(&widget);
        }
    };

    update_table(&initial_mb);

    vbox.append(&table_box);
    pref_row.set_child(Some(&vbox));
    expander.add_row(&pref_row);

    if !is_global {
        let override_check = gtk4::CheckButton::builder()
            .active(is_overridden)
            .valign(gtk4::Align::Center)
            .tooltip_text("Override global configuration")
            .build();

        let expander_clone = expander.clone();
        let vram_entry_clone = vram_entry.clone();
        let unit_label_clone = unit_label.clone();
        let update_table_clone = update_table.clone();
        let mgr_check = mgr.clone();
        let g_mb = global_mb;
        let disk_vram_clone = disk_vram.clone();

        override_check.connect_toggled(move |btn| {
            let is_active = btn.is_active();
            vram_entry_clone.set_sensitive(is_active);
            vram_entry_clone.set_editable(is_active);
            unit_label_clone.set_sensitive(is_active);

            if is_active {
                let digits: String = vram_entry_clone.text().chars().filter(|c| c.is_ascii_digit()).collect();
                let state = if digits.is_empty() || digits == "0" {
                    FeatureState::Disabled
                } else {
                    FeatureState::Enabled(digits.clone())
                };
                update_table_clone(&digits);
                if let Some(mf) = mgr_check.current_managed.borrow_mut().as_mut() {
                    mf.reserve_vram = state;
                }
            } else {
                vram_entry_clone.set_text(&g_mb);
                update_table_clone(&g_mb);
                if let Some(mf) = mgr_check.current_managed.borrow_mut().as_mut() {
                    mf.reserve_vram = FeatureState::Inherit;
                }
            }

            let cur_state = mgr_check
                .current_managed
                .borrow()
                .as_ref()
                .map(|mf| mf.reserve_vram.clone())
                .unwrap_or(FeatureState::Inherit);
            if cur_state != disk_vram_clone {
                expander_clone.add_css_class("modified-row");
            } else {
                expander_clone.remove_css_class("modified-row");
            }
            mgr_check.update_dirty_state();
        });

        expander.add_prefix(&override_check);
    }

    expander.add_suffix(&overlay);

    {
        let expander_clone = expander.clone();
        let mgr_entry = mgr.clone();
        let disk_vram_clone = disk_vram;
        vram_entry.connect_changed(move |e| {
            if !e.is_editable() {
                return;
            }
            let raw = e.text().to_string();
            let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits != raw {
                e.set_text(&digits);
                return;
            }
            update_table(&digits);
            let state = if digits.is_empty() || digits == "0" {
                FeatureState::Disabled
            } else {
                FeatureState::Enabled(digits)
            };
            if let Some(mf) = mgr_entry.current_managed.borrow_mut().as_mut() {
                mf.reserve_vram = state.clone();
            }
            if state != disk_vram_clone {
                expander_clone.add_css_class("modified-row");
            } else {
                expander_clone.remove_css_class("modified-row");
            }
            mgr_entry.update_dirty_state();
        });
    }

    expander
}

fn build_shader_cache_row(
    mgr: &ManagerWindow,
    managed: &ManagedFeatures,
    disk_managed: &ManagedFeatures,
    global_managed: Option<&ManagedFeatures>,
    is_global: bool,
) -> adw::ExpanderRow {
    let is_overridden = is_global || !matches!(managed.shader_cache_size, FeatureState::Inherit);
    let disk_shader = disk_managed.shader_cache_size.clone();

    let global_mb = global_managed
        .as_ref()
        .and_then(|gm| {
            if gm.shader_cache_size.is_enabled() {
                Some(gm.shader_cache_size.extra_args().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "10240".to_string());

    let initial_mb = if is_overridden {
        match &managed.shader_cache_size {
            FeatureState::Enabled(mb) => mb.clone(),
            FeatureState::Disabled => "0".to_string(),
            FeatureState::Inherit => global_mb.clone(),
        }
    } else {
        global_mb.clone()
    };

    let vendor = crate::config::detect_gpu_vendor();

    let expander = adw::ExpanderRow::builder().title("Shader cache size").build();

    if managed.shader_cache_size != disk_managed.shader_cache_size {
        expander.add_css_class("modified-row");
    }

    let is_editable = is_global || is_overridden;

    let overlay = gtk4::Overlay::builder()
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::End)
        .build();

    let entry = gtk4::Entry::builder()
        .text(&initial_mb)
        .placeholder_text("0")
        .tooltip_text("Use 0 to disable")
        .input_purpose(gtk4::InputPurpose::Digits)
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::End)
        .width_chars(6)
        .max_width_chars(8)
        .editable(is_editable)
        .sensitive(is_editable)
        .css_classes(["monospace", "vram-entry"])
        .build();

    gtk4::prelude::EditableExt::set_alignment(&entry, 1.0);

    let unit_label = gtk4::Label::builder()
        .label("MiB")
        .can_target(false)
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::End)
        .margin_end(8)
        .css_classes(["dim-label"])
        .sensitive(is_editable)
        .build();

    overlay.set_child(Some(&entry));
    overlay.add_overlay(&unit_label);

    // Expanded section
    let pref_row = non_activatable_row();
    let vbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(6)
        .margin_top(8)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();

    let vendor_text = match vendor {
        crate::config::GpuVendor::Unknown => "Generic GPU detected".to_string(),
        v => format!("{} detected", v.name()),
    };
    let vendor_label = gtk4::Label::builder()
        .label(&vendor_text)
        .halign(gtk4::Align::Start)
        .css_classes(["dim-label"])
        .build();

    let table_box = gtk4::Box::builder().orientation(gtk4::Orientation::Vertical).build();

    let update_table = {
        let table_box = table_box.clone();
        move |mb_str: &str| {
            while let Some(child) = table_box.first_child() {
                table_box.remove(&child);
            }
            let envs = compute_shader_cache_envvars(mb_str, vendor);
            let widget = build_env_table(&envs);
            table_box.append(&widget);
        }
    };

    update_table(&initial_mb);

    vbox.append(&vendor_label);
    vbox.append(&table_box);
    pref_row.set_child(Some(&vbox));
    expander.add_row(&pref_row);

    if !is_global {
        let override_check = gtk4::CheckButton::builder()
            .active(is_overridden)
            .valign(gtk4::Align::Center)
            .tooltip_text("Override global configuration")
            .build();

        let expander_clone = expander.clone();
        let entry_clone = entry.clone();
        let unit_label_clone = unit_label.clone();
        let update_table_clone = update_table.clone();
        let mgr_check = mgr.clone();
        let g_mb = global_mb;
        let disk_shader_clone = disk_shader.clone();

        override_check.connect_toggled(move |btn| {
            let is_active = btn.is_active();
            entry_clone.set_sensitive(is_active);
            entry_clone.set_editable(is_active);
            unit_label_clone.set_sensitive(is_active);

            if is_active {
                let digits: String = entry_clone.text().chars().filter(|c| c.is_ascii_digit()).collect();
                let state = if digits.is_empty() || digits == "0" {
                    FeatureState::Disabled
                } else {
                    FeatureState::Enabled(digits.clone())
                };
                update_table_clone(&digits);
                if let Some(mf) = mgr_check.current_managed.borrow_mut().as_mut() {
                    mf.shader_cache_size = state;
                }
            } else {
                entry_clone.set_text(&g_mb);
                update_table_clone(&g_mb);
                if let Some(mf) = mgr_check.current_managed.borrow_mut().as_mut() {
                    mf.shader_cache_size = FeatureState::Inherit;
                }
            }

            let cur_state = mgr_check
                .current_managed
                .borrow()
                .as_ref()
                .map(|mf| mf.shader_cache_size.clone())
                .unwrap_or(FeatureState::Inherit);
            if cur_state != disk_shader_clone {
                expander_clone.add_css_class("modified-row");
            } else {
                expander_clone.remove_css_class("modified-row");
            }
            mgr_check.update_dirty_state();
        });

        expander.add_prefix(&override_check);
    }

    expander.add_suffix(&overlay);

    {
        let expander_clone = expander.clone();
        let mgr_entry = mgr.clone();
        let disk_shader_clone = disk_shader;
        entry.connect_changed(move |e| {
            if !e.is_editable() {
                return;
            }
            let raw = e.text().to_string();
            let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits != raw {
                e.set_text(&digits);
                return;
            }
            update_table(&digits);
            let state = if digits.is_empty() || digits == "0" {
                FeatureState::Disabled
            } else {
                FeatureState::Enabled(digits)
            };
            if let Some(mf) = mgr_entry.current_managed.borrow_mut().as_mut() {
                mf.shader_cache_size = state.clone();
            }
            if state != disk_shader_clone {
                expander_clone.add_css_class("modified-row");
            } else {
                expander_clone.remove_css_class("modified-row");
            }
            mgr_entry.update_dirty_state();
        });
    }

    expander
}

fn build_timeout_row(mgr: &ManagerWindow, managed: &ManagedFeatures, disk_managed: &ManagedFeatures) -> adw::ActionRow {
    let initial_val = managed.launcher_timeout.as_deref().unwrap_or("2").to_string();
    let disk_val = disk_managed.launcher_timeout.as_deref().unwrap_or("2").to_string();

    let row = adw::ActionRow::builder().title("Launcher timeout").build();

    if managed.launcher_timeout != disk_managed.launcher_timeout {
        row.add_css_class("modified-row");
    }

    let overlay = gtk4::Overlay::builder()
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::End)
        .build();

    let entry = gtk4::Entry::builder()
        .text(&initial_val)
        .placeholder_text("2")
        .tooltip_text("Use 0 to disable")
        .input_purpose(gtk4::InputPurpose::Digits)
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::End)
        .width_chars(5)
        .max_width_chars(6)
        .editable(true)
        .sensitive(true)
        .css_classes(["monospace", "timeout-entry"])
        .build();

    gtk4::prelude::EditableExt::set_alignment(&entry, 1.0);

    let unit_label = gtk4::Label::builder()
        .label("s")
        .can_target(false)
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::End)
        .margin_end(8)
        .css_classes(["dim-label"])
        .build();

    overlay.set_child(Some(&entry));
    overlay.add_overlay(&unit_label);

    row.add_suffix(&overlay);

    let row_clone = row.clone();
    let mgr_entry = mgr.clone();
    entry.connect_changed(move |e| {
        if !e.is_editable() {
            return;
        }
        let raw = e.text().to_string();
        let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
        let clean = if digits.is_empty() {
            if raw.is_empty() { "0".to_string() } else { raw.clone() }
        } else {
            digits
        };

        if clean != disk_val {
            row_clone.add_css_class("modified-row");
        } else {
            row_clone.remove_css_class("modified-row");
        }

        if let Some(mf) = mgr_entry.current_managed.borrow_mut().as_mut() {
            mf.launcher_timeout = Some(clean);
        }
        mgr_entry.update_dirty_state();
    });

    row
}

fn create_preset_dropdown(
    presets: &'static [crate::config::DlssPresetOption],
    selected_key: &str,
    extra_custom_key: Option<&str>,
) -> (gtk4::DropDown, Rc<Vec<String>>) {
    let mut keys: Vec<String> = presets.iter().map(|p| p.key.to_string()).collect();
    let mut strings: Vec<String> = presets.iter().map(|p| format!("{}|||{}", p.title, p.note)).collect();

    let mut selected_idx = presets.iter().position(|p| p.key == selected_key);
    if selected_idx.is_none() && !selected_key.is_empty() {
        keys.push(selected_key.to_string());
        strings.push(format!("{}|||Custom value", selected_key));
        selected_idx = Some(keys.len() - 1);
    }

    if let Some(extra) = extra_custom_key {
        if !extra.is_empty() && !keys.iter().any(|k| k == extra) {
            keys.push(extra.to_string());
            strings.push(format!("{}|||Custom value", extra));
        }
    }

    let str_slices: Vec<&str> = strings.iter().map(|s| s.as_str()).collect();
    let string_list = gtk4::StringList::new(&str_slices);

    let dropdown = gtk4::DropDown::builder()
        .model(&string_list)
        .selected(selected_idx.unwrap_or(0) as u32)
        .valign(gtk4::Align::Center)
        .build();

    let button_factory = gtk4::SignalListItemFactory::new();
    button_factory.connect_setup(|_, list_item| {
        let title = gtk4::Label::builder()
            .halign(gtk4::Align::Start)
            .valign(gtk4::Align::Center)
            .build();
        list_item.set_child(Some(&title));
    });

    button_factory.connect_bind(|_, list_item| {
        if let Some(obj) = list_item.item().and_then(|i| i.downcast::<gtk4::StringObject>().ok()) {
            let text = obj.string();
            if let Some(label) = list_item.child().and_then(|c| c.downcast::<gtk4::Label>().ok()) {
                let title = text.split_once("|||").map(|(t, _)| t).unwrap_or(&text);
                label.set_text(title);
            }
        }
    });

    let list_factory = gtk4::SignalListItemFactory::new();
    list_factory.connect_setup(|_, list_item| {
        let vbox = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(2)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .build();
        let title = gtk4::Label::builder().halign(gtk4::Align::Start).build();
        let note = gtk4::Label::builder()
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label", "dlss-note"])
            .build();
        vbox.append(&title);
        vbox.append(&note);
        list_item.set_child(Some(&vbox));
    });

    list_factory.connect_bind(|_, list_item| {
        if let Some(obj) = list_item.item().and_then(|i| i.downcast::<gtk4::StringObject>().ok()) {
            let text = obj.string();
            if let Some(vbox) = list_item.child().and_then(|c| c.downcast::<gtk4::Box>().ok()) {
                if let Some(title_lbl) = vbox.first_child().and_then(|c| c.downcast::<gtk4::Label>().ok()) {
                    if let Some(note_lbl) = title_lbl.next_sibling().and_then(|c| c.downcast::<gtk4::Label>().ok()) {
                        if let Some((t, n)) = text.split_once("|||") {
                            title_lbl.set_text(t);
                            if n.is_empty() {
                                note_lbl.set_visible(false);
                            } else {
                                note_lbl.set_visible(true);
                                note_lbl.set_text(n);
                            }
                        } else {
                            title_lbl.set_text(&text);
                            note_lbl.set_visible(false);
                        }
                    }
                }
            }
        }
    });

    dropdown.set_factory(Some(&button_factory));
    dropdown.set_list_factory(Some(&list_factory));

    (dropdown, Rc::new(keys))
}

fn build_dlss_row(
    mgr: &ManagerWindow,
    managed: &ManagedFeatures,
    disk_managed: &ManagedFeatures,
    global_managed: Option<&ManagedFeatures>,
    is_global: bool,
) -> adw::ExpanderRow {
    let is_overridden = is_global || managed.dlss.is_some();
    let disk_dlss = disk_managed.dlss.clone();

    let default_dlss = crate::config::DlssSettings::default();
    let global_dlss = global_managed.and_then(|gm| gm.dlss.as_ref()).unwrap_or(&default_dlss);

    let effective_dlss = if is_overridden {
        managed.dlss.as_ref().unwrap_or(global_dlss).clone()
    } else {
        global_dlss.clone()
    };

    let expander = adw::ExpanderRow::builder().title("DLSS overrides").build();

    if managed.dlss != disk_managed.dlss {
        expander.add_css_class("modified-row");
    }

    let is_editable = is_global || is_overridden;

    // Expand if overridden with active settings
    if is_overridden
        && (effective_dlss.upgrade
            || effective_dlss.indicator
            || !effective_dlss.super_resolution.is_empty()
            || !effective_dlss.ray_reconstruction.is_empty()
            || !effective_dlss.frame_generation.is_empty())
    {
        expander.set_expanded(true);
    }

    // 1. DLSS Upgrade
    let upgrade_row = adw::ActionRow::builder().title("DLSS upgrade").build();
    let upgrade_switch = gtk4::Switch::builder()
        .active(effective_dlss.upgrade)
        .valign(gtk4::Align::Center)
        .sensitive(is_editable)
        .build();
    upgrade_row.add_suffix(&upgrade_switch);
    expander.add_row(&upgrade_row);

    // 2. DLSS Indicator
    let indicator_row = adw::ActionRow::builder().title("DLSS indicator").build();
    let indicator_switch = gtk4::Switch::builder()
        .active(effective_dlss.indicator)
        .valign(gtk4::Align::Center)
        .sensitive(is_editable)
        .build();
    indicator_row.add_suffix(&indicator_switch);
    expander.add_row(&indicator_row);

    // 3. Super resolution model
    let sr_row = adw::ActionRow::builder().title("Super resolution model").build();
    let (sr_dropdown, sr_keys) = create_preset_dropdown(
        crate::config::DLSS_SR_PRESETS,
        &effective_dlss.super_resolution,
        global_managed
            .and_then(|gm| gm.dlss.as_ref())
            .map(|d| d.super_resolution.as_str()),
    );
    sr_dropdown.set_sensitive(is_editable);
    sr_row.add_suffix(&sr_dropdown);
    expander.add_row(&sr_row);

    // 4. Ray reconstruction model
    let rr_row = adw::ActionRow::builder().title("Ray reconstruction model").build();
    let (rr_dropdown, rr_keys) = create_preset_dropdown(
        crate::config::DLSS_RR_PRESETS,
        &effective_dlss.ray_reconstruction,
        global_managed
            .and_then(|gm| gm.dlss.as_ref())
            .map(|d| d.ray_reconstruction.as_str()),
    );
    rr_dropdown.set_sensitive(is_editable);
    rr_row.add_suffix(&rr_dropdown);
    expander.add_row(&rr_row);

    // 5. Frame generation model (only if GPU supports it)
    let has_fg = crate::config::supports_dlss_frame_generation();
    let (fg_dropdown_opt, fg_keys_opt) = if has_fg {
        let fg_row = adw::ActionRow::builder().title("Frame generation model").build();
        let (fg_dropdown, fg_keys) = create_preset_dropdown(
            crate::config::DLSS_FG_PRESETS,
            &effective_dlss.frame_generation,
            global_managed
                .and_then(|gm| gm.dlss.as_ref())
                .map(|d| d.frame_generation.as_str()),
        );
        fg_dropdown.set_sensitive(is_editable);
        fg_row.add_suffix(&fg_dropdown);
        expander.add_row(&fg_row);
        (Some(fg_dropdown), Some(fg_keys))
    } else {
        (None, None)
    };

    let update_state = {
        let mgr = mgr.clone();
        let expander = expander.clone();
        let disk_dlss = disk_dlss.clone();
        let upgrade_switch = upgrade_switch.clone();
        let indicator_switch = indicator_switch.clone();
        let sr_dropdown = sr_dropdown.clone();
        let rr_dropdown = rr_dropdown.clone();
        let fg_dropdown_opt = fg_dropdown_opt.clone();
        let sr_keys = sr_keys.clone();
        let rr_keys = rr_keys.clone();
        let fg_keys_opt = fg_keys_opt.clone();

        move || {
            let sr_idx = sr_dropdown.selected() as usize;
            let sr_key = sr_keys.get(sr_idx).cloned().unwrap_or_default();

            let rr_idx = rr_dropdown.selected() as usize;
            let rr_key = rr_keys.get(rr_idx).cloned().unwrap_or_default();

            let fg_key = if let (Some(fg_dd), Some(fg_k)) = (&fg_dropdown_opt, &fg_keys_opt) {
                let fg_idx = fg_dd.selected() as usize;
                fg_k.get(fg_idx).cloned().unwrap_or_default()
            } else {
                String::new()
            };

            let new_settings = crate::config::DlssSettings {
                upgrade: upgrade_switch.is_active(),
                indicator: indicator_switch.is_active(),
                super_resolution: sr_key,
                ray_reconstruction: rr_key,
                frame_generation: fg_key,
            };

            if let Some(mf) = mgr.current_managed.borrow_mut().as_mut() {
                mf.dlss = Some(new_settings);
            }

            let cur_dlss = mgr.current_managed.borrow().as_ref().and_then(|mf| mf.dlss.clone());
            if cur_dlss != disk_dlss {
                expander.add_css_class("modified-row");
            } else {
                expander.remove_css_class("modified-row");
            }
            mgr.update_dirty_state();
        }
    };

    // Connect signals
    {
        let update_state = update_state.clone();
        upgrade_switch.connect_state_set(move |sw, _| {
            if !sw.is_sensitive() {
                return glib::Propagation::Proceed;
            }
            update_state();
            glib::Propagation::Proceed
        });
    }

    {
        let update_state = update_state.clone();
        indicator_switch.connect_state_set(move |sw, _| {
            if !sw.is_sensitive() {
                return glib::Propagation::Proceed;
            }
            update_state();
            glib::Propagation::Proceed
        });
    }

    {
        let update_state = update_state.clone();
        sr_dropdown.connect_selected_notify(move |combo| {
            if !combo.is_sensitive() {
                return;
            }
            update_state();
        });
    }

    {
        let update_state = update_state.clone();
        rr_dropdown.connect_selected_notify(move |combo| {
            if !combo.is_sensitive() {
                return;
            }
            update_state();
        });
    }

    if let Some(ref fg_dd) = fg_dropdown_opt {
        let update_state = update_state.clone();
        fg_dd.connect_selected_notify(move |combo| {
            if !combo.is_sensitive() {
                return;
            }
            update_state();
        });
    }

    if !is_global {
        let override_check = gtk4::CheckButton::builder()
            .active(is_overridden)
            .valign(gtk4::Align::Center)
            .tooltip_text("Override global configuration")
            .build();

        let upgrade_switch_c = upgrade_switch;
        let indicator_switch_c = indicator_switch;
        let sr_dropdown_c = sr_dropdown;
        let rr_dropdown_c = rr_dropdown;
        let fg_dd_c = fg_dropdown_opt;
        let sr_keys_c = sr_keys;
        let rr_keys_c = rr_keys;
        let fg_keys_c = fg_keys_opt;
        let update_state_c = update_state;
        let g_dlss = global_dlss.clone();
        let mgr_toggle = mgr.clone();
        let disk_dlss_c = disk_dlss;
        let expander_c = expander.clone();

        override_check.connect_toggled(move |btn| {
            let is_active = btn.is_active();
            upgrade_switch_c.set_sensitive(is_active);
            indicator_switch_c.set_sensitive(is_active);
            sr_dropdown_c.set_sensitive(is_active);
            rr_dropdown_c.set_sensitive(is_active);
            if let Some(ref fg_dd) = fg_dd_c {
                fg_dd.set_sensitive(is_active);
            }

            if is_active {
                update_state_c();
            } else {
                upgrade_switch_c.set_active(g_dlss.upgrade);
                indicator_switch_c.set_active(g_dlss.indicator);

                let g_sr_idx = sr_keys_c
                    .iter()
                    .position(|k| k == &g_dlss.super_resolution)
                    .unwrap_or(0);
                sr_dropdown_c.set_selected(g_sr_idx as u32);

                let g_rr_idx = rr_keys_c
                    .iter()
                    .position(|k| k == &g_dlss.ray_reconstruction)
                    .unwrap_or(0);
                rr_dropdown_c.set_selected(g_rr_idx as u32);

                if let (Some(fg_dd), Some(fg_k)) = (&fg_dd_c, &fg_keys_c) {
                    let g_fg_idx = fg_k.iter().position(|k| k == &g_dlss.frame_generation).unwrap_or(0);
                    fg_dd.set_selected(g_fg_idx as u32);
                }

                if let Some(mf) = mgr_toggle.current_managed.borrow_mut().as_mut() {
                    mf.dlss = None;
                }

                let cur_dlss = mgr_toggle
                    .current_managed
                    .borrow()
                    .as_ref()
                    .and_then(|mf| mf.dlss.clone());
                if cur_dlss != disk_dlss_c {
                    expander_c.add_css_class("modified-row");
                } else {
                    expander_c.remove_css_class("modified-row");
                }
                mgr_toggle.update_dirty_state();
            }
        });

        expander.add_prefix(&override_check);
    }

    expander
}
