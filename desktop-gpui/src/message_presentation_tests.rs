//! Real transcript layout fixtures. The detached performance facade cannot run IPC.
#![cfg(feature = "performance-profiling")]
use super::{ChatPane, message_attachments, DesktopService};
use crate::design;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};
use serde_json::json;
use std::{cell::RefCell, rc::Rc};

fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
}

fn assert_message_presentation(cx: &mut TestAppContext, theme: &str) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
    });
    // Identical multiline content makes the rendered size comparison meaningful,
    // including markdown layout rather than merely a stored font-size constant.
    let text = "One short line.\n\n**Two short lines.**\n\nThree short lines.\n\nFour short lines.";
    let snapshot = Rc::new(json!({
        "settings":{"theme":theme,"accent":"#6594db","userName":"Alex","tintUserMessages":true},
        "tasks":[{"id":"presentation-task","agentId":"codex-agent","title":"Origin presentation","provider":"codex","status":"idle","queue":[]}],
        "agents":[{"id":"codex-agent","name":"Codex","provider":"codex"}],
        "messages":[
            {"id":"human","taskId":"presentation-task","role":"user","text":text,"createdAt":1},
            {"id":"peer","taskId":"presentation-task","role":"user","text":text,"senderAgentId":"codex-agent","collaborationId":"fixture-collaboration","createdAt":2},
            {"id":"context","taskId":"presentation-task","role":"user","text":"Copied **context** with a [reference](https://example.invalid/fixture).","senderAgentId":"codex-agent","createdAt":3,
             "attachments":[{"id":"fixture-file","name":"Notes.txt","mimeType":"text/plain","size":42}]},
            {"id":"unknown","taskId":"presentation-task","role":"user","text":"Unknown peer context.","collaborationId":"fixture-collaboration","createdAt":4},
            {"id":"unlinked","taskId":"presentation-task","role":"user","text":"Unresolved recorded exchange.","collaborationId":"missing-record","createdAt":4},
            {"id":"assistant","taskId":"presentation-task","role":"assistant","text":text,"createdAt":5},
            {"id":"system","taskId":"presentation-task","role":"system","text":text,"createdAt":6},
            {"id":"reply","taskId":"presentation-task","role":"system","text":"Peer result with details.","senderAgentId":"codex-agent","collaborationId":"fixture-collaboration","createdAt":6},
            {"id":"cleared","taskId":"presentation-task","role":"system","text":"Context Cleared","createdAt":7},
            {"id":"settings","taskId":"presentation-task","role":"system","text":"[Monitter settings change] Model changed","createdAt":8},
            {"id":"fork","taskId":"presentation-task","role":"system","text":"[Monitter fork] Previous task","createdAt":9}
        ],
        "events":[{"id":"collaboration-event","taskId":"presentation-task","kind":"collaboration","title":"Collaboration queued","detail":"fixture-collaboration","createdAt":10}],
        "collaborations":[{"id":"fixture-collaboration","fromTaskId":"presentation-task","toAgentId":"codex-agent","text":"Please check the fixture.","status":"queued"}],
        "approvalRequests":[],"queuedMessages":[],"sharing":{"primary":{"name":"Alex"}},"hosts":[],"channels":[]
    }));
    let mounted = Rc::new(RefCell::new(None));
    let mounted_slot = mounted.clone();
    let (_, visual) = cx.add_window_view(move |window, cx| {
        let pane = cx.new(|cx| ChatPane::performance_fixture("presentation-task".into(), snapshot, window, cx));
        *mounted_slot.borrow_mut() = Some(pane.clone());
        gpui_kit::component::Root::new(pane, window, cx)
    });
    let pane = mounted.borrow().as_ref().expect("mounted chat pane").clone();
    for width in [600., 360.] {
        visual.simulate_resize(size(px(width), px(2400.)));
        draw(visual);
        for selector in ["peer-message-peer", "peer-message-context", "peer-message-unknown", "system-message-system", "system-notice-cleared", "system-notice-settings", "system-notice-fork"] {
            let bounds = visual.debug_bounds(selector).unwrap_or_else(|| panic!("{theme}/{width}: missing rendered {selector}"));
            assert!(bounds.size.width > px(0.));
            assert!(bounds.left() >= px(0.) && bounds.right() <= px(width + 1.), "{theme}/{width}: {selector} overflows viewport: {bounds:?}");
        }
        assert!(visual.debug_bounds("peer-message-human").is_none(), "human input keeps its user presentation");
        assert!(visual.debug_bounds("peer-message-assistant").is_none(), "ordinary assistant output keeps its presentation");
        for selector in [
            "message-origin-human-Alex",
            "message-origin-assistant-Codex",
            "message-origin-system-System",
        ] {
            assert!(visual.debug_bounds(selector).is_some(), "{theme}/{width}: missing rendered origin {selector}");
        }
        for selector in ["message-origin-peer-Alex", "message-origin-context-Alex", "message-origin-unknown-Alex", "message-origin-system-Alex"] {
            assert!(visual.debug_bounds(selector).is_none(), "peer/system messages must never inherit human attribution");
        }
        let human = visual.debug_bounds("message-body-human").expect("human markdown laid out");
        assert!(visual.debug_bounds("message-body-peer").is_none(), "collapsed exchange hides its recorded body");
        assert!(visual.debug_bounds("message-body-reply").is_none());
        for selector in ["exchange-direction-peer-incoming", "exchange-direction-context-incoming", "exchange-direction-unknown-incoming", "exchange-direction-unlinked-unknown", "exchange-direction-collaboration-event-outgoing"] {
            assert!(visual.debug_bounds(selector).is_some(), "recorded route: {selector}");
        }
        let peer_width = visual.debug_bounds("peer-message-peer").unwrap().size.width;
        let human_width = visual.debug_bounds("message-bubble-human").unwrap().size.width;
        if width == 360. {
            // Narrow panes deliberately give human and peer messages the full column.
            assert!((peer_width - human_width).abs() <= px(1.),
                "{theme}/{width}: narrow messages share the reading column");
        } else {
            assert!(peer_width > human_width,
                "{theme}/{width}: peer communication fills the wider reading column");
        }
        assert!(visual.debug_bounds("collaboration-event-collaboration-event").is_some());
        let status = visual.debug_bounds("collaboration-status-collaboration-event")
            .expect("collapsed collaboration keeps its queued status visible");
        assert!(status.size.width > px(0.) && status.size.height > px(0.));
        let card = visual.debug_bounds("collaboration-event-collaboration-event").unwrap();
        assert!(status.left() >= card.left() && status.right() <= card.right());
        let toggle = visual.debug_bounds("peer-message-toggle-peer").expect("whole-row peer toggle");
        visual.simulate_click(toggle.center(), Default::default());
        draw(visual);
        pane.read_with(visual, |pane, _| assert!(pane.peer_details_expanded.borrow().contains("peer")));
        let expanded_peer = visual.debug_bounds("message-body-peer").expect("expanded peer body");
        assert!(expanded_peer.size.height > px(0.), "whole-row disclosure reveals the recorded markdown body");
        let toggle = visual.debug_bounds("peer-message-toggle-peer").expect("whole-row peer toggle after expansion");
        visual.simulate_click(toggle.center(), Default::default());
        draw(visual);
        let toggle = visual.debug_bounds("collaboration-toggle-collaboration-event").expect("whole-row collaboration toggle");
        visual.simulate_click(toggle.center(), Default::default());
        draw(visual);
        pane.read_with(visual, |pane, _| assert!(pane.peer_details_expanded.borrow().contains("event:collaboration-event")));
        let toggle = visual.debug_bounds("collaboration-toggle-collaboration-event").expect("collaboration toggle after expansion");
        visual.simulate_click(toggle.center(), Default::default());
        draw(visual);
        pane.read_with(visual, |pane, _| assert!(pane.peer_details_expanded.borrow().is_empty(), "prior peer and event disclosures are closed before context expansion"));
        let context_toggle = visual.debug_bounds("peer-message-toggle-context").unwrap();
        let viewport = visual.debug_bounds("retained-transcript-viewport").expect("transcript viewport");
        let other_peer = visual.debug_bounds("peer-message-toggle-peer");
        let other_event = visual.debug_bounds("collaboration-toggle-collaboration-event");
        assert!(viewport.contains(&context_toggle.center()), "context pointer must be within clipped viewport: context={context_toggle:?}, viewport={viewport:?}, peer={other_peer:?}, event={other_event:?}");
        visual.simulate_click(context_toggle.center(), Default::default());
        draw(visual);
        pane.read_with(visual, |pane, _| assert!(pane.peer_details_expanded.borrow().contains("context"), "context whole-row pointer must toggle the actual mounted disclosure: selected={:?}, context={context_toggle:?}, viewport={viewport:?}, peer={other_peer:?}, event={other_event:?}", pane.peer_details_expanded.borrow()));
        assert!(visual.debug_bounds("message-body-context").is_some(), "expanded markdown and attachment row remain renderable");
        // Expanding a virtual row can move the tail-anchored transcript. Resolve
        // its current header again rather than clicking the pre-expansion point.
        let context_close = visual.debug_bounds("peer-message-toggle-context").expect("expanded context header remains mounted");
        visual.simulate_click(context_close.center(), Default::default());
        draw(visual);
        pane.read_with(visual, |pane, _| assert!(!pane.peer_details_expanded.borrow().contains("context"), "context must be closed before the next viewport width"));
        assert!(visual.debug_bounds("message-body-context").is_none(), "closing context unmounts its expanded body");
    }
}

#[gpui_kit::test]
fn message_presentation_dark_narrow_and_wide(cx: &mut TestAppContext) {
    assert_message_presentation(cx, "dark");
}

#[gpui_kit::test]
fn message_presentation_light_narrow_and_wide(cx: &mut TestAppContext) {
    assert_message_presentation(cx, "light");
}

fn assert_grouped_exchange_presentation(cx: &mut TestAppContext, theme: &str, activity: bool) {
    cx.update(|cx| {gpui_kit::init(cx);crate::surface_blend::initialize_for_ui_fixture();crate::native_motion::initialize_for_ui_fixture(cx);});
    let base=1_760_000_000_000_i64;
    let snapshot=Rc::new(json!({"settings":{"theme":theme},
        "tasks":[{"id":"group-task","agentId":"main","title":"Grouped exchanges","provider":"codex","status":"idle","queue":[]},{"id":"peer-task","agentId":"peer","title":"Real recorded peer","status":"idle"}],
        "agents":[{"id":"main","name":"Main","provider":"codex"},{"id":"peer","name":"Codex recorded peer","provider":"codex"}],
        "messages":[
            {"id":"first","taskId":"group-task","role":"user","senderAgentId":"peer","collaborationId":"c1","createdAt":base,"text":"First **recorded** delivery.","attachments":[{"id":"file","name":"Recorded.txt","mimeType":"text/plain","size":42}]},
            {"id":"second","taskId":"group-task","role":"system","collaborationId":"c2","createdAt":base+2000,"text":"Second recorded reply."},
            {"id":"barrier","taskId":"group-task","role":"assistant","createdAt":base+3000,"text":"Assistant turn remains outside the group."}],
        "events":[{"id":"route","taskId":"group-task","kind":"collaboration","detail":"c1","title":"Collaboration queued","createdAt":base+1000}],
        "collaborations":[{"id":"c1","kind":"message","fromTaskId":"group-task","fromAgentId":"main","toTaskId":"peer-task","toAgentId":"peer","text":"Recorded first request","status":"queued"},
            {"id":"c2","kind":"message","fromTaskId":"peer-task","fromAgentId":"peer","toTaskId":"group-task","toAgentId":"main","text":"Recorded second request","result":"Recorded second result","status":"completed"}],
        "approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]}));
    let snapshot = if activity {
        let mut value=(*snapshot).clone();
        value["events"].as_array_mut().unwrap().push(json!({"id":"cluster-tool","taskId":"group-task","kind":"tool","title":"commandExecution","createdAt":base-500,"detail":"{\"type\":\"commandExecution\",\"command\":\"pwd\"}"}));
        Rc::new(value)
    } else {snapshot};
    let mounted=Rc::new(RefCell::new(None));let slot=mounted.clone();
    let (_,visual)=cx.add_window_view(move|window,cx|{
        let pane=cx.new(|cx| {
            let recorded_attachments=snapshot["messages"][0]["attachments"].as_array().unwrap().clone();
            let attachment_palette=design::palette(&snapshot["settings"],window.appearance());
            let mut pane=ChatPane::performance_fixture("group-task".into(),snapshot,window,cx);
            if activity { pane.service=Some(DesktopService::disconnected_for_ui_fixture()); }
            // Performance fixtures deliberately remove the parent service. Mount
            // a detached real attachment entity so this checks rendering rather
            // than assuming the service-backed preparation path ran offline.
            let row=cx.new(|cx|message_attachments::MessageAttachments::new(
                DesktopService::disconnected_for_ui_fixture(),&recorded_attachments,cx));
            row.update(cx,|row,cx|row.set_appearance(attachment_palette,cx));
            pane.message_attachment_rows.insert("first".into(),row);
            pane
        });*slot.borrow_mut()=Some(pane.clone());
        gpui_kit::component::Root::new(pane,window,cx)
    });
    let pane=mounted.borrow().as_ref().unwrap().clone();
    let disclosure=if activity {"tool-group:cluster-tool"} else {"first"};
    let group_selector=if activity {"exchange-group-tool-group:cluster-tool"} else {"exchange-group-first"};
    let toggle_selector=if activity {"exchange-group-toggle-tool-group:cluster-tool"} else {"exchange-group-toggle-first"};
    for width in [600.,360.] {
        visual.simulate_resize(size(px(width),px(1600.)));draw(visual);
        pane.read_with(visual,|pane,_|{
            let state=&pane.transcripts["group-task"];
            assert_eq!(state.projection.len(),2,"one real grouped virtual row plus assistant barrier");
            assert_eq!(state.list.item_count(),2);
            assert_eq!(state.source_index("message:second"),Some(0));
            assert_eq!(state.source_index("activity:route"),Some(0));
            if activity {assert_eq!(state.projection[0].stable_key(),"tool-group:cluster-tool");}
            assert!(matches!(&state.projection[0],crate::transcript::ConversationItem::ExchangeGroup{exchange_count:2,members,..} if members.len()==if activity {4}else{3}));
        });
        if activity {
            pane.update(visual,|pane,cx| {
                let row=pane.activity_rows.get("cluster-tool").expect("real source_rows prepare nested event").clone();
                row.update(cx,|row,cx|row.seed_fixture_detail("{\"type\":\"commandExecution\",\"command\":\"printf fixture\",\"output\":\"synthetic command output retained\"}",cx));
            });
        }
        let group=visual.debug_bounds(group_selector).expect("single outer diagram");
        assert!(group.left()>=px(0.)&&group.right()<=px(width));
        let direction = visual.debug_bounds(if activity {"exchange-direction-tool-group:cluster-tool-mixed"} else {"exchange-direction-first-mixed"}).unwrap();
        assert!((direction.left() - group.left()).abs() <= px(0.5), "exchange icon is flush with the transcript column");
        let assistant = visual.debug_bounds("message-bubble-barrier").expect("normal assistant row shares avatar column");
        let dot = visual.debug_bounds(if activity {"exchange-leading-dot-tool-group:cluster-tool"} else {"exchange-leading-dot-first"}).expect("actual leading activity circle");
        let origin = visual.debug_bounds("message-identity-barrier").expect("assistant identity row");
        assert!((dot.left() - assistant.left()).abs() <= px(0.5), "assistant bubble aligns with first circle pixel");
        assert!((dot.left() - origin.left()).abs() <= px(0.5), "assistant avatar/name row shares dot inset");
        assert!(visual.debug_bounds(if activity {"exchange-direction-tool-group:cluster-tool-mixed"}else{"exchange-direction-first-mixed"}).is_some());
        let label=if activity {"exchange-peer-tool-group:cluster-tool-Agent activity · 1 tool · 2 exchanges"}else{"exchange-peer-first-Codex recorded peer · 2 exchanges"};
        assert!(visual.debug_bounds(label).is_some(),"cluster summary reports observed activity without assigning tools to a peer");
        assert!(visual.debug_bounds("message-body-first").is_none());
        let toggle=visual.debug_bounds(toggle_selector).unwrap();
        visual.simulate_click(toggle.center(),Default::default());draw(visual);
        pane.read_with(visual,|pane,_|assert!(pane.peer_details_expanded.borrow().contains(disclosure)));
        pane.update(visual,|pane,cx| {
            pane.invalidate_transcript_list(cx);
            cx.notify();
        });
        draw(visual);
        pane.read_with(visual,|pane,_|assert!(pane.peer_details_expanded.borrow().contains(disclosure),"leading activity disclosure survives pane preparation"));
        assert!(visual.debug_bounds("message-body-first").is_some(),"expanded cluster remains mounted after pane update");
        for selector in ["message-body-first","message-body-second","message-attachment-cards-first","exchange-member-route"] {
            let body=visual.debug_bounds(selector).unwrap_or_else(||panic!("{theme}/{width}: recorded content missing: {selector}"));
            assert!(body.size.height>px(0.));assert!(body.left()>=px(0.)&&body.right()<=px(width));
        }
        let expanded_group = visual.debug_bounds(group_selector).unwrap();
        let timeline = visual.debug_bounds(if activity {"exchange-timeline-tool-group:cluster-tool"} else {"exchange-timeline-first"}).expect("one shared timeline container");
        let rail = visual.debug_bounds(if activity {"exchange-rail-tool-group:cluster-tool"} else {"exchange-rail-first"}).expect("one uninterrupted group rail");
        let expanded_toggle = visual.debug_bounds(toggle_selector).unwrap();
        let inset = design::ui_scaled_px(12.);
        assert!(timeline.left() - expanded_group.left() >= inset, "{theme}/{width}: expanded contents need left breathing room");
        assert!(expanded_group.right() - timeline.right() >= inset, "{theme}/{width}: expanded contents need right breathing room");
        assert!(timeline.top() - expanded_toggle.bottom() >= design::ui_scaled_px(10.), "{theme}/{width}: timeline clears the summary");
        assert!(expanded_group.bottom() - timeline.bottom() >= inset, "{theme}/{width}: final content clears the box bottom");
        assert!((rail.bottom() - timeline.bottom()).abs() <= px(0.5), "{theme}/{width}: shared rail continues through member bodies and gaps");
        for (id, member_selector, heading_selector, dot_selector) in [
            ("first", "exchange-member-first", "exchange-member-heading-first", "exchange-member-dot-first"),
            ("route", "exchange-member-route", "exchange-member-heading-route", "exchange-member-dot-route"),
            ("second", "exchange-member-second", "exchange-member-heading-second", "exchange-member-dot-second"),
        ] {
            let member = visual.debug_bounds(member_selector).unwrap();
            let heading = visual.debug_bounds(heading_selector).expect("rendered first-line heading");
            let dot = visual.debug_bounds(dot_selector).expect("rendered timeline dot");
            assert!((dot.center().x - rail.center().x).abs() <= px(0.5), "{theme}/{width}: {id} dot stays centered on rail");
            assert!((dot.center().y - heading.top() - design::ui_scaled_px(9.)).abs() <= px(0.5), "{theme}/{width}: {id} dot aligns with the first heading line, including wrapped headings");
            assert!(rail.top() <= dot.center().y && rail.bottom() >= member.bottom(), "{theme}/{width}: rail spans every member without a gap");
            assert!(member.left() >= timeline.left() && member.right() <= timeline.right() + px(0.5), "{theme}/{width}: timeline preserves member width inside the inset");
            if width == 360. && id == "first" {
                assert!(heading.size.height > design::ui_scaled_px(18.), "narrow fixture exercises an actual wrapped heading");
            }
        }
        if activity {
            let turn=visual.debug_bounds("run-group-cluster-tool").expect("real nested tool turn is mounted");
            let leading = visual.debug_bounds("run-leading-cluster-tool").expect("nested tool-turn icon");
            let caret = visual.debug_bounds("run-caret-cluster-tool").expect("nested trailing caret");
            assert!((leading.center().x - rail.center().x).abs() <= px(0.5), "nested tool icon centers on the inset activity rail: {leading:?} vs {rail:?}");
            assert!(caret.left() > leading.right(), "nested disclosure caret is on the right");
            assert!((caret.right() - turn.right()).abs() <= px(0.5), "nested caret stays at row's right edge");
            visual.simulate_click(gpui_kit::point(turn.left()+px(20.),turn.top()+px(12.)),Default::default());draw(visual);
            assert!(visual.debug_bounds("run-branch-tools:cluster-tool").is_none(), "single duplicate command branch is flattened");
            let step=visual.debug_bounds("run-single-cluster-tool").expect("turn opens its individual step directly");
            visual.simulate_click(gpui_kit::point(step.left()+px(20.),step.top()+px(10.)),Default::default());draw(visual);
            assert!(visual.debug_bounds("run-detail-cluster-tool").is_some(),"native selectable command detail is mounted");
            pane.read_with(visual,|pane,cx|assert!(pane.activity_rows["cluster-tool"].read(cx).fixture_loaded_detail().contains("synthetic command output retained")));
            let step=visual.debug_bounds("run-single-cluster-tool").unwrap();
            visual.simulate_click(gpui_kit::point(step.left()+px(20.),step.top()+px(10.)),Default::default());draw(visual);
            let turn=visual.debug_bounds("run-group-cluster-tool").unwrap();
            visual.simulate_click(gpui_kit::point(turn.left()+px(20.),turn.top()+px(12.)),Default::default());draw(visual);
        }
        let toggle=visual.debug_bounds(toggle_selector).unwrap();
        visual.simulate_click(toggle.center(),Default::default());draw(visual);
        pane.read_with(visual,|pane,_|assert!(!pane.peer_details_expanded.borrow().contains(disclosure)));
        assert!(visual.debug_bounds("message-body-first").is_none());
    }
}
#[gpui_kit::test]
fn exchange_grouping_dark_narrow_and_wide(cx:&mut TestAppContext){assert_grouped_exchange_presentation(cx,"dark",false);}
#[gpui_kit::test]
fn exchange_grouping_light_narrow_and_wide(cx:&mut TestAppContext){assert_grouped_exchange_presentation(cx,"light",false);}

#[gpui_kit::test]
fn exchange_group_activity_cluster_dark_narrow_and_wide(cx:&mut TestAppContext){assert_grouped_exchange_presentation(cx,"dark",true);}

#[gpui_kit::test]
fn exchange_group_activity_cluster_light_narrow_and_wide(cx:&mut TestAppContext){assert_grouped_exchange_presentation(cx,"light",true);}
