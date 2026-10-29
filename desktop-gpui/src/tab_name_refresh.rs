//! Periodic owner-local title checks for open chat panes. The backend remains
//! authoritative: it decides whether a title is stale and applies guarded edits.
use super::*;

const FIRST_CHECK: std::time::Duration = std::time::Duration::from_secs(2 * 60);
const CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15 * 60);
const MAX_PANES_PER_CHECK: usize = 32;

fn next_batch(mut targets: Vec<Value>, cursor: &mut usize) -> Vec<Value> {
    if targets.is_empty() {
        return targets;
    }
    let total = targets.len();
    targets.rotate_left(*cursor % total);
    let count = total.min(MAX_PANES_PER_CHECK);
    *cursor = (*cursor + count) % total;
    targets.truncate(count);
    targets
}

impl Desktop {
    pub(super) fn start_tab_name_refresh(&mut self, cx: &mut Context<Self>) {
        if self.tab_name_refresh.is_some() || cfg!(test) {
            return;
        }
        self.tab_name_refresh = Some(cx.spawn(async move |view, cx| {
            let mut delay = FIRST_CHECK;
            loop {
                cx.background_executor().timer(delay).await;
                delay = CHECK_INTERVAL;
                let attempt = view.update(cx, |view, cx| {
                    if view.closing || view.busy || view.window.preferences_restore || view.window.preferences_restoring {
                        return None;
                    }
                    let service = view.service.clone()?;
                    let targets = palette_catalog::rename_all_targets(view, cx);
                    let targets = next_batch(targets, &mut view.tab_name_refresh_cursor);
                    (!targets.is_empty()).then_some((service, targets))
                });
                let (service, targets) = match attempt {
                    Ok(Some(attempt)) => attempt,
                    Ok(None) => continue,
                    Err(_) => break, // The desktop entity was closed.
                };
                let check = cx.background_executor().spawn(async move {
                    service.invoke("rename_all", json!({"targets": targets}))
                });
                if let Err(error) = check.await {
                    // A missing decision backend or Admin configuration must
                    // not block the UI or trigger an immediate retry storm.
                    eprintln!("[tab-names] background refresh failed: {error}");
                }
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::next_batch;
    use serde_json::json;

    #[test]
    fn batch_rotates_across_more_than_thirty_two_open_panes() {
        let targets = (0..35).map(|index| json!({"taskId":index.to_string()})).collect();
        let mut cursor = 0;
        let first = next_batch(targets, &mut cursor);
        assert_eq!(first.len(), 32);
        assert_eq!(first[0]["taskId"], "0");
        let targets = (0..35).map(|index| json!({"taskId":index.to_string()})).collect();
        let second = next_batch(targets, &mut cursor);
        assert_eq!(second.len(), 32);
        assert_eq!(second[0]["taskId"], "32");
        assert!(second.iter().any(|target| target["taskId"] == "34"));
    }
}
