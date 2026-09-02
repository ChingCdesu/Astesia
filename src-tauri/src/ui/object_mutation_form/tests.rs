use super::*;

#[test]
fn creation_names_inherit_the_selected_schema_once() {
    assert_eq!(qualify_name(Some("audit"), "events"), "audit.events");
    assert_eq!(
        qualify_name(Some("audit"), "reporting.events"),
        "reporting.events"
    );
}

#[test]
fn rename_defaults_strip_schema_and_routine_signatures() {
    assert_eq!(rename_default_name("audit.events"), "events");
    assert_eq!(rename_default_name("billing.total(uuid)"), "total");
}
