use serde_json::json;

use super::Kind;
use crate::PhotocraftApp;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn draw(app: &mut PhotocraftApp) -> usize {
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput::default());
    super::draw(app, &ctx);
    let mut out = ctx.end_pass();
    out.textures_delta.clear();
    out.shapes.len()
}

#[test]
fn a_played_action_shows_a_notice_that_fades_away() {
    let mut app = app();
    super::played(&mut app, &json!({"action": "Print Letter", "ran": 1}));
    let n = app.action_notice.clone().unwrap();
    assert_eq!(n.kind, Kind::Done);
    assert!(n.text.contains("Print Letter"), "{}", n.text);
    assert!(draw(&mut app) > 0, "drawn");
    // Gone once its time is up.
    app.action_notice.as_mut().unwrap().since -= super::SHOW_MS + 1.0;
    draw(&mut app);
    assert!(app.action_notice.is_none());
}

#[test]
fn a_failed_action_says_why() {
    let mut app = app();
    super::played(&mut app, &json!({"action": "A", "ran": 0, "failed": {"step": 0, "id": "x", "error": "no document"}}));
    let n = app.action_notice.clone().unwrap();
    assert_eq!(n.kind, Kind::Error);
    assert!(n.text.contains("no document"), "{}", n.text);
}

#[test]
fn a_print_ending_updates_only_a_waiting_notice() {
    let mut app = app();
    super::print_ended(&mut app, None);
    assert!(app.action_notice.is_none(), "no notice: nothing to update");
    super::show(&mut app, "P", "printing".into(), Kind::Busy);
    // A busy notice stays up past the usual time.
    app.action_notice.as_mut().unwrap().since -= super::SHOW_MS + 1.0;
    draw(&mut app);
    assert!(app.action_notice.is_some());
    super::print_ended(&mut app, Some("the printer is offline"));
    let n = app.action_notice.clone().unwrap();
    assert_eq!(n.kind, Kind::Error);
    assert!(n.text.contains("offline"), "{}", n.text);
}

/// An action's print step runs on a worker in the app (playback returns at once) and the notice
/// reports how the print ended.
#[test]
fn an_action_that_prints_returns_at_once_and_reports_the_print() {
    photocraft_engine::print_cmds::disable_printing();
    let mut app = app();
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.background_jobs = true;
    app.session.actions.list.push(photocraft_engine::actions_cmds::Action::new("Print it", vec![("file.printOneCopy".into(), json!({}))]));
    let v = crate::actions::play(&mut app, &json!({"action": "Print it"})).unwrap();
    assert_eq!(v["ran"], 1, "{v}");
    assert!(app.printing(), "the print runs as a background job");
    crate::actions::report_play(&mut app, &v);
    assert_eq!(app.action_notice.as_ref().unwrap().kind, Kind::Busy);
    let ctx = egui::Context::default();
    let t0 = std::time::Instant::now();
    while app.printing() && t0.elapsed() < std::time::Duration::from_secs(20) {
        crate::jobs_ui::tick(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    crate::jobs_ui::tick(&mut app, &ctx);
    // Printing is disabled in tests, so the job fails, and the notice says so.
    let n = app.action_notice.clone().unwrap();
    assert_eq!(n.kind, Kind::Error, "{}", n.text);
    assert!(n.text.contains("Print it"), "{}", n.text);
}
