use fleet_api::{check, default_ui_schema, release_create, release_edit, UiSchema};
use serde_json::json;

#[test]
fn release_forms_are_in_the_default_schema() {
    let s = default_ui_schema();
    assert_eq!(
        s.form("release_create").unwrap().submit.path,
        "/api/products/{product}/releases"
    );
    assert_eq!(s.form("release_edit").unwrap().submit.method, "PATCH");
    let form = release_create();
    let keys: Vec<_> = form.inputs().map(|i| i.key.as_str()).collect();
    assert_eq!(keys, ["version", "start", "target"]);
}

#[test]
fn check_accepts_a_good_release() {
    assert_eq!(
        check(
            &release_create(),
            &json!({"version": "1.0", "start": "2026-10-01", "target": "2026-11-01"})
        ),
        Ok(())
    );
}

#[test]
fn check_names_the_field_at_fault() {
    let f = release_create();
    let e = check(
        &f,
        &json!({"version": "   ", "start": "2026-10-01", "target": "2026-11-01"}),
    )
    .unwrap_err();
    assert_eq!(
        (e.field.as_str(), e.message.as_str()),
        ("version", "the release needs a version")
    );
    let e = check(
        &f,
        &json!({"version": "1", "start": "2026-10-01", "target": "soon"}),
    )
    .unwrap_err();
    assert_eq!(
        (e.field.as_str(), e.message.as_str()),
        ("target", "\"soon\" is not a date; use YYYY-MM-DD")
    );
    let e = check(
        &f,
        &json!({"version": "1", "start": "2026-10-02", "target": "2026-10-01"}),
    )
    .unwrap_err();
    assert_eq!(
        (e.field.as_str(), e.message.as_str()),
        ("target", "the target date is before the start date")
    );
}

#[test]
fn check_ignores_absent_keys() {
    // Mark released sends only `released`.
    assert_eq!(check(&release_edit(), &json!({"released": true})), Ok(()));
}

#[test]
fn unknown_parts_parse_and_are_flagged() {
    let mut v = serde_json::to_value(default_ui_schema()).unwrap();
    let create = v["forms"]
        .as_array()
        .unwrap()
        .iter()
        .position(|f| f["id"] == "release_create")
        .unwrap();
    let items = &mut v["forms"][create]["items"];
    items[0]["fields"][0]["kind"] = json!("colour");
    items[0]["fields"][1]["rules"] = json!([{"type": "luhn"}]);
    items
        .as_array_mut()
        .unwrap()
        .push(json!({"type": "carousel"}));
    let s: UiSchema = serde_json::from_value(v).unwrap();
    assert!(!s.form("release_create").unwrap().is_known());
    assert!(s.form("release_edit").unwrap().is_known());
}
