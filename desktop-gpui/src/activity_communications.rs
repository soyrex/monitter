//! Directed, real collaboration requests. Lifetimes measure painted foreground time.
use super::{MapRect, rows, text};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

pub(super) type Pair = (String, String);
const LIFETIME: Duration = Duration::from_secs(30);
const MAX_LINKS: usize = 128;

#[derive(Clone, Debug)]
pub(super) struct Link {
    pub pair: Pair,
    pub forward: bool,
    pub reverse: bool,
    pub viewed: Duration,
    sequence: u64,
}
impl Link {
    pub fn opacity(&self) -> f32 {
        // Keep the request legible, then gently disappear during its last five viewed seconds.
        ((LIFETIME.saturating_sub(self.viewed).as_secs_f32()) / 5.).clamp(0., 1.)
    }
}
#[derive(Default)]
pub(super) struct Communications {
    known: HashSet<String>,
    initialized: bool,
    sequence: u64,
    pub links: BTreeMap<Pair, Link>,
    pub painted: HashSet<Pair>,
}
impl Communications {
    pub fn ingest(&mut self, snapshot: &Value, now: i64) {
        let mut known = HashSet::new();
        for record in rows(snapshot, "collaborations") {
            let id = text(record, "id");
            if id.is_empty() { continue; }
            known.insert(id.to_owned());
            if self.known.contains(id) { continue; }
            let from = text(record, "fromTaskId");
            let to = text(record, "toTaskId");
            if from.is_empty() || to.is_empty() || from == to
                || !matches!(text(record, "kind"), "message" | "delegation")
                || !matches!(text(record, "status"), "queued" | "running" | "completed") {
                continue;
            }
            // First opening starts with recent/in-flight requests, not years of history.
            // Once retained, every newly observed request waits for actual viewing.
            if !self.initialized && record["createdAt"].as_i64().unwrap_or(0) < now - 30_000
                && !matches!(text(record, "status"), "queued" | "running") { continue; }
            let (pair, forward) = if from < to {
                ((from.to_owned(), to.to_owned()), true)
            } else { ((to.to_owned(), from.to_owned()), false) };
            self.sequence += 1;
            let link = self.links.entry(pair.clone()).or_insert(Link {
                pair, forward: false, reverse: false, viewed: Duration::ZERO, sequence: 0,
            });
            link.forward |= forward;
            link.reverse |= !forward;
            link.viewed = Duration::ZERO;
            link.sequence = self.sequence;
        }
        self.known = known;
        self.initialized = true;
        while self.links.len() > MAX_LINKS {
            let oldest = self.links.values().min_by_key(|l| l.sequence).unwrap().pair.clone();
            self.links.remove(&oldest);
        }
    }
    pub fn tick(&mut self, elapsed: Duration, foreground: bool) -> bool {
        let painted = std::mem::take(&mut self.painted);
        if !foreground || painted.is_empty() { return false; }
        // Sleep, a stalled UI thread or a hidden window must never consume unseen time.
        let elapsed = elapsed.min(Duration::from_millis(100));
        for pair in painted {
            if let Some(link) = self.links.get_mut(&pair) { link.viewed += elapsed; }
        }
        self.links.retain(|_, link| link.viewed < LIFETIME);
        true
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Curve { pub start: (f32, f32), pub control: (f32, f32), pub end: (f32, f32) }
impl Curve {
    pub fn between(a: MapRect, b: MapRect) -> Self { Self::routed(a,b,&[]) }
    pub fn routed(a: MapRect, b: MapRect, obstacles: &[MapRect]) -> Self {
        let ac = (a.x + a.w / 2., a.y + a.h / 2.);
        let bc = (b.x + b.w / 2., b.y + b.h / 2.);
        let dx = bc.0-ac.0; let dy = bc.1-ac.1;
        let length = dx.hypot(dy).max(1.);
        let bend = (length*0.16).clamp(24.,84.);
        fn edge(rect: MapRect, center: (f32,f32), toward: (f32,f32)) -> (f32,f32) {
            let dx = toward.0-center.0; let dy = toward.1-center.1;
            let t = ((rect.w/2.+4.)/dx.abs().max(0.001)).min((rect.h/2.+4.)/dy.abs().max(0.001));
            (center.0+dx*t,center.1+dy*t)
        }
        let mut best = None;
        let mut best_score = usize::MAX;
        // Choose the shallowest clear arc whose painted path stays within the map.
        // A control point may sit outside the map while its entire curve stays inside.
        // Each directed route prefers its own side of the task pair.
        for extra in [0.,80.,180.,300.,500.] {
            for sign in [1.,-1.] {
                let control = (((ac.0+bc.0)/2.-dy/length*(bend+extra)*sign).max(-ac.0.min(bc.0)*0.25),
                    ((ac.1+bc.1)/2.+dx/length*(bend+extra)*sign).max(-ac.1.min(bc.1)*0.25));
                let curve = Self{start:edge(a,ac,control),control,end:edge(b,bc,control)};
                let score = (1..32).filter(|step| {
                    let (x,y) = curve.at(*step as f32/32.);
                    x < 0. || y < 0. || obstacles.iter().any(|r| x>r.x-9. && x<r.x+r.w+9. && y>r.y-9. && y<r.y+r.h+9.)
                }).count();
                if score == 0 { return curve; }
                if score < best_score { best_score=score; best=Some(curve); }
            }
        }
        best.expect("at least one route candidate")
    }
    pub fn directed(a: MapRect, b: MapRect, obstacles: &[MapRect], forward: bool, reverse: bool) -> Vec<Self> {
        let mut routes = Vec::new();
        if forward { routes.push(Self::routed(a, b, obstacles)); }
        if reverse {
            let mut reply_obstacles = obstacles.to_vec();
            if let Some(outbound) = routes.first() {
                // Keep the reply visibly separate, including when a project label
                // forces both directions into the same general corridor.
                reply_obstacles.extend((2..15).map(|step| {
                    let (x, y) = outbound.at(step as f32 / 16.);
                    MapRect { x: x - 3., y: y - 3., w: 6., h: 6. }
                }));
            }
            routes.push(Self::routed(b, a, &reply_obstacles));
        }
        routes
    }
    pub fn dots(&self, phase: f32) -> Vec<(f32, f32)> {
        let length_bound = (self.control.0 - self.start.0).hypot(self.control.1 - self.start.1)
            + (self.end.0 - self.control.0).hypot(self.end.1 - self.control.1);
        let steps = (length_bound / 2.).ceil().clamp(1., 4096.) as usize;
        let mut dots = Vec::new();
        let mut previous = self.start;
        let mut remaining = phase.rem_euclid(6.);
        for step in 1..=steps {
            let next = self.at(step as f32 / steps as f32);
            let distance = (next.0 - previous.0).hypot(next.1 - previous.1);
            while remaining <= distance && distance > 0. {
                let t = remaining / distance;
                dots.push((previous.0 + (next.0 - previous.0) * t,
                    previous.1 + (next.1 - previous.1) * t));
                remaining += 6.;
            }
            remaining -= distance;
            previous = next;
        }
        dots
    }
    pub fn at(&self, t: f32) -> (f32, f32) {
        let u = 1. - t;
        (u*u*self.start.0 + 2.*u*t*self.control.0 + t*t*self.end.0,
         u*u*self.start.1 + 2.*u*t*self.control.1 + t*t*self.end.1)
    }
    pub fn tangent(&self, t: f32) -> (f32, f32) {
        (2.*(1.-t)*(self.control.0-self.start.0)+2.*t*(self.end.0-self.control.0),
         2.*(1.-t)*(self.control.1-self.start.1)+2.*t*(self.end.1-self.control.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn activity_overview_reciprocal_dots_follow_separate_directed_arcs() {
        let a = MapRect { x: 100., y: 180., w: 120., h: 80. };
        let b = MapRect { x: 600., y: 180., w: 120., h: 80. };
        assert!(Curve::directed(a, b, &[], false, false).is_empty());
        assert_eq!(Curve::directed(a, b, &[], true, false).len(), 1);
        let routes = Curve::directed(a, b, &[], true, true);
        assert_eq!(routes.len(), 2);
        let p = routes[0].at(0.5); let q = routes[1].at(0.5);
        assert!((p.0-q.0).hypot(p.1-q.1) > 10., "reply paints a distinct arc");
        for curve in routes {
            let before = curve.dots(0.)[0]; let after = curve.dots(2.)[0];
            let tangent = curve.tangent(0.);
            assert!((after.0-before.0)*tangent.0 + (after.1-before.1)*tangent.1 > 0.,
                "dots advance from the real sender toward the recipient");
            assert!((after.0-before.0).hypot(after.1-before.1) > 1.9);
        }
    }
    fn record(id: &str, from: &str, to: &str) -> Value {
        json!({"id":id,"kind":"message","fromTaskId":from,"toTaskId":to,
            "fromAgentId":"same-profile","toAgentId":"same-profile","status":"completed","createdAt":100_000})
    }
    #[test]
    fn activity_overview_communication_replies_reuse_pair_and_retries_do_not_refresh() {
        let mut state = Communications::default();
        let a = record("one", "a", "b");
        state.ingest(&json!({"collaborations":[a]}), 100_000);
        let pair = ("a".into(), "b".into());
        for _ in 0..100 { state.painted.insert(pair.clone()); state.tick(Duration::from_millis(100), true); }
        state.ingest(&json!({"collaborations":[a]}), 101_000);
        assert_eq!(state.links[&pair].viewed, Duration::from_secs(10));
        state.ingest(&json!({"collaborations":[a,record("reply","b","a")]}), 101_000);
        assert_eq!(state.links.len(), 1);
        assert!(state.links[&pair].forward && state.links[&pair].reverse);
        assert_eq!(state.links[&pair].viewed, Duration::ZERO);
    }
    #[test]
    fn activity_overview_communication_clock_waits_for_visible_foreground_endpoints() {
        let mut state = Communications::default();
        state.ingest(&json!({"collaborations":[record("one","a","b")]}),100_000);
        let pair = ("a".into(),"b".into());
        state.tick(Duration::from_secs(900), true); // hidden, list or filtered out
        state.painted.insert(pair.clone()); state.tick(Duration::from_secs(900), false);
        assert_eq!(state.links[&pair].viewed, Duration::ZERO);
        for _ in 0..299 { state.painted.insert(pair.clone()); state.tick(Duration::from_millis(100),true); }
        assert!(state.links[&pair].opacity() < 0.03);
        state.painted.insert(pair.clone()); state.tick(Duration::from_millis(100),true);
        assert!(state.links.is_empty());
        state.ingest(&json!({"collaborations":[record("one","a","b")]}),999_000);
        assert!(state.links.is_empty());
        state.ingest(&json!({"collaborations":[record("one","a","b"),record("new","a","b")]}),999_000);
        assert_eq!(state.links[&pair].viewed,Duration::ZERO); // unseen while away
    }
    #[test]
    fn activity_overview_communication_rejects_ambiguous_and_failed_records() {
        let mut records = vec![record("old","a","b"),record("self","a","a"),record("missing","a","")];
        records[0]["createdAt"] = json!(1);
        let mut failed = record("failed","a","b"); failed["status"] = json!("error"); records.push(failed);
        records.push(json!({"id":"native","parentTaskId":"a","agentThreadId":"b"}));
        let mut state = Communications::default();
        state.ingest(&json!({"collaborations":records}),100_000);
        assert!(state.links.is_empty());
    }
    #[test]
    fn activity_overview_communication_curve_starts_and_ends_outside_cards() {
        let a = MapRect{x:10.,y:20.,w:180.,h:100.};
        let b = MapRect{x:500.,y:200.,w:180.,h:100.};
        let curve = Curve::between(a,b);
        assert!(curve.start.0 > a.x+a.w || curve.start.1 > a.y+a.h);
        assert!(curve.end.0 < b.x || curve.end.1 < b.y);
        assert_eq!(curve.at(0.),curve.start); assert_eq!(curve.at(1.),curve.end);
        assert!(curve.tangent(0.5).0 > 0.);
    }
    #[test]
    fn activity_overview_communication_curve_routes_around_project_labels() {
        let a=MapRect{x:10.,y:10.,w:196.,h:102.};
        let b=MapRect{x:18.,y:222.,w:196.,h:102.};
        let hub=MapRect{x:44.,y:138.,w:144.,h:60.};
        let other=MapRect{x:220.,y:104.,w:196.,h:102.};
        let curve=Curve::routed(a,b,&[hub,other]);
        for step in 1..32 {
            let (x,y)=curve.at(step as f32/32.);
            for r in [hub,other] { assert!(!(x>r.x-9. && x<r.x+r.w+9. && y>r.y-9. && y<r.y+r.h+9.)); }
        }
    }

}
