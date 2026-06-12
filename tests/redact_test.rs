use histq::redact::{redact, REDACTED};

#[test]
fn authorization_bearer_header_is_redacted() {
    let cmd = r#"curl -H "Authorization: Bearer sk-abc123XYZ" https://api.example.com"#;
    let out = redact(cmd);
    assert!(!out.contains("sk-abc123XYZ"), "got: {out}");
    assert!(out.contains(REDACTED));
    assert!(out.contains("Authorization: Bearer "));
    assert!(out.contains("https://api.example.com"));
}

#[test]
fn authorization_basic_header_is_redacted() {
    let out = redact(r#"curl -H 'Authorization: Basic dXNlcjpwYXNz' http://x"#);
    assert!(!out.contains("dXNlcjpwYXNz"), "got: {out}");
}

#[test]
fn api_key_assignment_is_redacted_but_key_name_kept() {
    let out = redact("export API_KEY=sk-secret-value-123");
    assert!(!out.contains("sk-secret-value-123"), "got: {out}");
    assert_eq!(out, format!("export API_KEY={REDACTED}"));
}

#[test]
fn password_flag_with_equals_is_redacted() {
    let out = redact("mysql --password=hunter2 -u root db");
    assert!(!out.contains("hunter2"), "got: {out}");
    assert!(out.contains("-u root db"));
}

#[test]
fn password_flag_with_space_is_redacted() {
    let out = redact("deploy-tool --password hunter2 --target prod");
    assert!(!out.contains("hunter2"), "got: {out}");
    assert!(out.contains("--target prod"));
}

#[test]
fn token_key_value_is_redacted() {
    let out = redact("http POST /login token:abc123def");
    assert!(!out.contains("abc123def"), "got: {out}");
}

#[test]
fn aws_access_key_id_is_redacted() {
    let out = redact("aws configure set aws_access_key_id AKIAIOSFODNN7EXAMPLE");
    assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"), "got: {out}");
}

#[test]
fn github_token_is_redacted() {
    let out = redact("git clone https://ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ012345@github.com/x/y");
    assert!(
        !out.contains("ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ012345"),
        "got: {out}"
    );
}

#[test]
fn slack_token_is_redacted() {
    let out = redact("curl -d token=xoxb-1234567890-abcdefghij");
    assert!(!out.contains("xoxb-1234567890"), "got: {out}");
}

#[test]
fn jwt_is_redacted() {
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.SflKxwRJSMeKKF2QT4fwpM";
    let out = redact(&format!("curl https://x -d '{jwt}'"));
    assert!(!out.contains(jwt), "got: {out}");
}

#[test]
fn innocent_commands_pass_through_unchanged() {
    for cmd in [
        "git push origin main",
        "echo reading an article about password managers",
        "cargo test --release",
        "ls -la /tmp",
        "grep -r token_count src/",
        "man passwd",
    ] {
        assert_eq!(redact(cmd), cmd, "should not modify: {cmd}");
    }
}

#[test]
fn redaction_is_idempotent() {
    let once = redact("export SECRET=value123");
    assert_eq!(redact(&once), once);
}
