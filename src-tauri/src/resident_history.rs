//! Keep durable history identities/order but release old transcript bodies.
//! SQLite owns cold bodies; paging/fork/detail reads materialize them on demand.
use crate::model::Snapshot;
use std::sync::Arc;

pub(crate) const HOT_MESSAGES: usize = 64;
pub(crate) const HOT_EVENTS: usize = 128;

pub(crate) fn evict(snapshot: &mut Snapshot) {
    for scope in snapshot.messages.scope_ids() {
        let indices = snapshot.messages.scope_indices(&scope);
        for index in indices.iter().rev().skip(HOT_MESSAGES).copied() {
            let message = &snapshot.messages[index];
            if snapshot.messages.payload_is_cold(&message.id) { break; }
            // System context boundaries and streaming recovery remain readable.
            if !matches!(message.role.as_str(), "user" | "assistant")
                || message.stream_status.as_deref() == Some("streaming") { continue; }
            let mut shell = message.clone();
            shell.text.clear();
            snapshot.messages.evict_payload(index, shell);
        }
    }
    for channel in &mut snapshot.channels {
        for index in (0..channel.messages.len().saturating_sub(HOT_MESSAGES)).rev() {
            let message=&channel.messages[index];
            if channel.messages.payload_is_cold(&message.id) {break;}
            let mut shell=message.clone();shell.text.clear();
            channel.messages.evict_payload(index,shell);
        }
    }
    for scope in snapshot.events.scope_ids() {
        let indices = snapshot.events.scope_indices(&scope);
        for index in indices.iter().rev().skip(HOT_EVENTS).copied() {
            let event = &snapshot.events[index];
            if snapshot.events.payload_is_cold(&event.id) { break; }
            // Mutable advisory receipts are small and must remain parseable.
            if event.kind == "jevDecision" { continue; }
            let mut shell = (**event).clone();
            // Retain the bounded summary used by the live UI; the full body
            // remains exclusively in SQLite.
            shell.detail = crate::lan_sync::compact_event(event).detail;
            snapshot.events.evict_payload(index, Arc::new(shell));
        }
    }
}
