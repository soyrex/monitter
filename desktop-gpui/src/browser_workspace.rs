//! Pure migration for saved dock layouts when legacy browser tabs become panes.
use serde_json::{Value, json};
use std::collections::HashSet;

const PANE_NAME: &str = "MonitterNativePane";
const TAB_GROUP_NAME: &str = "TabPanel";
const LEGACY_BROWSER_ID: &str = "route:browser";
const MAX_PANES: usize = 64;
const MAX_BROWSER_TABS: usize = 64;
const LAYOUT_VERSION: u64 = 1;

/// Returns the stable pane ID for a validated nonzero browser tab ID.
pub fn browser_pane_id(id: u64) -> String {
    format!("browser:{id}")
}

pub fn parse_browser_pane_id(id: &str) -> Option<u64> {
    let number = id.strip_prefix("browser:")?;
    if number.is_empty()
        || number.starts_with('0')
        || !number.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    number.parse::<u64>().ok().filter(|id| *id != 0)
}

#[derive(Default)]
struct LayoutIds {
    panes: HashSet<String>,
    browsers: HashSet<u64>,
    legacy_browser_count: usize,
}

pub fn migrate_browser_layout(
    layout: &Value,
    browser_ids: &[u64],
    selected_id: Option<u64>,
) -> Result<Value, String> {
    validate_browser_metadata(browser_ids, selected_id)?;
    let layout_ids = validate_layout(layout)?;
    let metadata_ids: HashSet<u64> = browser_ids.iter().copied().collect();

    if layout_ids
        .browsers
        .iter()
        .any(|id| !metadata_ids.contains(id))
    {
        return Err("Saved browser pane is absent from browser metadata.".into());
    }
    if layout_ids.legacy_browser_count > 0 {
        if browser_ids.is_empty() {
            return Err("Legacy browser pane has no saved browser tabs to restore.".into());
        }
        if !layout_ids.browsers.is_empty() {
            return Err("Saved layout mixes legacy and canonical browser panes.".into());
        }
        let final_pane_count = layout_ids
            .panes
            .len()
            .checked_sub(layout_ids.legacy_browser_count)
            .and_then(|count| count.checked_add(browser_ids.len()))
            .ok_or("Saved workspace pane count is invalid.")?;
        if final_pane_count > MAX_PANES {
            return Err(
                "Native workspace supports at most 64 panes after browser migration.".into(),
            );
        }

        let selected_index = browser_ids
            .iter()
            .position(|id| Some(*id) == selected_id)
            .ok_or("Saved browser selection is missing from browser metadata.")?;
        let mut migrated = layout.clone();
        migrate_panel_tree(&mut migrated["center"], browser_ids, selected_index)?;
        for key in ["left_dock", "right_dock", "bottom_dock"] {
            if migrated.get(key).is_some_and(|dock| !dock.is_null()) {
                migrate_panel_tree(&mut migrated[key]["panel"], browser_ids, selected_index)?;
            }
        }
        Ok(migrated)
    } else {
        Ok(layout.clone())
    }
}

fn validate_browser_metadata(browser_ids: &[u64], selected_id: Option<u64>) -> Result<(), String> {
    if browser_ids.len() > MAX_BROWSER_TABS {
        return Err("Saved browser tabs exceed their limits.".into());
    }
    let unique: HashSet<u64> = browser_ids.iter().copied().collect();
    if unique.len() != browser_ids.len() || browser_ids.contains(&0) {
        return Err("Saved browser tab IDs are invalid or duplicated.".into());
    }
    match (browser_ids.is_empty(), selected_id) {
        (true, None) => Ok(()),
        (false, Some(selected)) if unique.contains(&selected) => Ok(()),
        _ => Err("Saved browser tab selection is invalid or missing.".into()),
    }
}

fn validate_layout(layout: &Value) -> Result<LayoutIds, String> {
    if layout.get("version").and_then(Value::as_u64) != Some(LAYOUT_VERSION) {
        return Err("Unsupported native workspace layout version.".into());
    }
    let mut ids = LayoutIds::default();
    scan_panel(
        layout
            .get("center")
            .ok_or("Saved workspace has no center panel tree.")?,
        None,
        0,
        &mut ids,
    )?;
    for key in ["left_dock", "right_dock", "bottom_dock"] {
        if let Some(dock) = layout.get(key).filter(|dock| !dock.is_null()) {
            let panel = dock
                .get("panel")
                .ok_or("Saved workspace dock has no panel tree.")?;
            scan_panel(panel, None, 0, &mut ids)?;
        }
    }
    if ids.panes.len() > MAX_PANES {
        return Err("Native workspace supports at most 64 panes.".into());
    }
    Ok(ids)
}

fn scan_panel(
    panel: &Value,
    parent_name: Option<&str>,
    depth: usize,
    ids: &mut LayoutIds,
) -> Result<(), String> {
    if depth > 32 {
        return Err("Native layout nesting is too deep.".into());
    }
    let name = panel
        .get("panel_name")
        .and_then(Value::as_str)
        .ok_or("Saved workspace panel has an invalid name.")?;
    let children = panel
        .get("children")
        .and_then(Value::as_array)
        .ok_or("Saved workspace panel has invalid children.")?;
    if children.len() > MAX_PANES {
        return Err("Native layout contains too many groups.".into());
    }
    if name == PANE_NAME {
        let id = panel
            .pointer("/info/panel/id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or("Invalid native pane identity.")?;
        if !ids.panes.insert(id.to_owned()) {
            return Err("Duplicate native pane identity in layout.".into());
        }
        if id == LEGACY_BROWSER_ID {
            if parent_name != Some(TAB_GROUP_NAME) {
                return Err("Legacy browser pane is not a direct child of a tab group.".into());
            }
            ids.legacy_browser_count += 1;
        } else if id.starts_with("browser:") {
            let browser_id =
                parse_browser_pane_id(id).ok_or("Saved browser pane ID is not canonical.")?;
            ids.browsers.insert(browser_id);
        }
    }
    for child in children {
        scan_panel(child, Some(name), depth + 1, ids)?;
    }
    Ok(())
}

fn migrate_panel_tree(
    panel: &mut Value,
    browser_ids: &[u64],
    selected_index: usize,
) -> Result<(), String> {
    if panel.get("panel_name").and_then(Value::as_str) == Some(TAB_GROUP_NAME) {
        let legacy_index = panel
            .get("children")
            .and_then(Value::as_array)
            .ok_or("Saved tab group has invalid children.")?
            .iter()
            .position(|child| {
                child.pointer("/info/panel/id").and_then(Value::as_str) == Some(LEGACY_BROWSER_ID)
            });
        if let Some(legacy_index) = legacy_index {
            let child_count = panel["children"]
                .as_array()
                .map(Vec::len)
                .ok_or("Saved tab group has invalid children.")?;
            let active_index = panel
                .pointer("/info/tabs/active_index")
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
                .ok_or("Saved tab group has an invalid active index.")?;
            if active_index >= child_count {
                return Err("Saved tab group active index is outside its children.".into());
            }
            let next_active_index = if active_index == legacy_index {
                legacy_index + selected_index
            } else if active_index > legacy_index {
                active_index + browser_ids.len() - 1
            } else {
                active_index
            };
            let browser_panels = browser_ids
                .iter()
                .map(|id| {
                    json!({
                        "panel_name": PANE_NAME,
                        "children": [],
                        "info": {"panel": {"id": browser_pane_id(*id)}}
                    })
                })
                .collect::<Vec<_>>();
            drop(
                panel["children"]
                    .as_array_mut()
                    .ok_or("Saved tab group has invalid children.")?
                    .splice(legacy_index..=legacy_index, browser_panels),
            );
            panel["info"]["tabs"]["active_index"] = json!(next_active_index);
        }
    }
    let children = panel
        .get_mut("children")
        .and_then(Value::as_array_mut)
        .ok_or("Saved workspace panel has invalid children.")?;
    for child in children {
        migrate_panel_tree(child, browser_ids, selected_index)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{browser_pane_id, migrate_browser_layout, parse_browser_pane_id};
    use serde_json::{Value, json};

    fn pane(id: &str) -> Value {
        json!({
            "panel_name":"MonitterNativePane",
            "children":[],
            "info":{"panel":{"id":id}}
        })
    }

    fn tabs(children: Vec<Value>, active_index: usize) -> Value {
        json!({
            "panel_name":"TabPanel",
            "children":children,
            "info":{"tabs":{"active_index":active_index}}
        })
    }

    fn layout(center: Value) -> Value {
        json!({"version":1,"center":center})
    }

    fn migrated_active(children: Vec<Value>, active: usize, selected: u64) -> (Value, usize) {
        let migrated = migrate_browser_layout(
            &layout(json!({
                "panel_name":"StackPanel",
                "children":[tabs(children, active)],
                "info":{"stack":{"sizes":[200.0],"axis":0}}
            })),
            &[10, 20, 30],
            Some(selected),
        )
        .unwrap();
        let group = &migrated["center"]["children"][0];
        (
            group.clone(),
            group["info"]["tabs"]["active_index"].as_u64().unwrap() as usize,
        )
    }

    #[::core::prelude::v1::test]
    fn canonical_browser_ids_are_strict_and_round_trip() {
        assert_eq!(browser_pane_id(42), "browser:42");
        assert_eq!(parse_browser_pane_id("browser:42"), Some(42));
        for invalid in [
            "browser:0",
            "browser:01",
            "browser:+1",
            "browser:",
            "chat:42",
        ] {
            assert_eq!(parse_browser_pane_id(invalid), None, "{invalid}");
        }
    }

    #[::core::prelude::v1::test]
    fn nested_migration_preserves_active_identity_before_at_and_after_legacy() {
        let (before, before_active) = migrated_active(
            vec![
                pane("chat:before"),
                pane("route:browser"),
                pane("chat:after"),
            ],
            0,
            20,
        );
        assert_eq!(before_active, 0);
        assert_eq!(before["children"][0]["info"]["panel"]["id"], "chat:before");
        assert_eq!(before["children"][1]["info"]["panel"]["id"], "browser:10");
        assert_eq!(before["children"][4]["info"]["panel"]["id"], "chat:after");

        let (at, at_active) = migrated_active(
            vec![
                pane("chat:before"),
                pane("route:browser"),
                pane("chat:after"),
            ],
            1,
            20,
        );
        assert_eq!(at_active, 2);
        assert_eq!(at["children"][2]["info"]["panel"]["id"], "browser:20");

        let (after, after_active) = migrated_active(
            vec![
                pane("chat:before"),
                pane("route:browser"),
                pane("chat:after"),
            ],
            2,
            20,
        );
        assert_eq!(after_active, 4);
        assert_eq!(after["children"][4]["info"]["panel"]["id"], "chat:after");
    }

    #[::core::prelude::v1::test]
    fn canonical_layout_is_cloned_only_after_metadata_identity_validation() {
        let saved = layout(tabs(vec![pane("browser:10")], 0));
        assert_eq!(
            migrate_browser_layout(&saved, &[10, 20], Some(10)).unwrap(),
            saved
        );
        assert!(migrate_browser_layout(&saved, &[20], Some(20)).is_err());
    }

    #[::core::prelude::v1::test]
    fn migration_rejects_empty_legacy_tabs_and_alias_collisions() {
        let legacy = layout(tabs(vec![pane("route:browser")], 0));
        assert!(migrate_browser_layout(&legacy, &[], None).is_err());

        let mixed = layout(tabs(vec![pane("route:browser"), pane("browser:10")], 0));
        assert!(migrate_browser_layout(&mixed, &[10, 20], Some(10)).is_err());
    }

    #[::core::prelude::v1::test]
    fn migration_rejects_duplicate_invalid_unknown_and_unselected_metadata() {
        let legacy = layout(tabs(vec![pane("route:browser")], 0));
        for ids in [&[10, 10][..], &[0, 10][..]] {
            assert!(migrate_browser_layout(&legacy, ids, Some(10)).is_err());
        }
        assert!(migrate_browser_layout(&legacy, &[10], None).is_err());
        assert!(migrate_browser_layout(&legacy, &[10], Some(11)).is_err());
        let unknown = layout(tabs(vec![pane("browser:11")], 0));
        assert!(migrate_browser_layout(&unknown, &[10], Some(10)).is_err());
    }

    #[::core::prelude::v1::test]
    fn migration_enforces_global_pane_cap_before_returning() {
        let mut children = vec![pane("route:browser")];
        children.extend((0..63).map(|index| pane(&format!("chat:{index}"))));
        let saved = layout(tabs(children, 0));
        assert!(migrate_browser_layout(&saved, &[10, 20], Some(10)).is_err());
        assert_eq!(saved["center"]["children"].as_array().unwrap().len(), 64);
    }
}
