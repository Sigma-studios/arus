//! Legal pages, reachable without logging in: the Impressum (§ 5 DDG) and how
//! to delete an account (linked from the app stores).

use axum::response::{IntoResponse, Response};

/// Who runs Eightcount (§ 5 DDG).
pub const OPERATOR_NAME: &str = "Mathis Aibache";
pub const OPERATOR_STREET: &str = "Stephanstraße 23";
pub const OPERATOR_CITY: &str = "52064 Aachen";
pub const OPERATOR_COUNTRY: &str = "Deutschland / Germany";
pub const CONTACT_EMAIL: &str = "contact@eightcount.de";

/// The small links under the login and sign-up forms.
pub const FOOTER: &str = r#"<p class="legal"><a href="/impressum">Impressum / Legal notice</a></p>"#;

const BACK: &str = r#"<p class="alt"><a href="/">Open Eightcount</a> · <a href="/login">Log in</a></p>"#;

pub fn impressum_html() -> String {
    format!(
        r#"<h3>Angaben gemäß § 5 DDG</h3>
  <p>{name}<br>{street}<br>{city}<br>{country}</p>
  <h3>Kontakt / Contact</h3>
  <p>E-Mail: <a href="mailto:{mail}">{mail}</a></p>
  <h3>Verantwortlich für den Inhalt</h3>
  <p>{name}, Anschrift wie oben.</p>
  <h3>Verbraucherstreitbeilegung</h3>
  <p class="small">Wir sind nicht bereit und nicht verpflichtet, an Streitbeilegungsverfahren vor einer
  Verbraucherschlichtungsstelle teilzunehmen.</p>
  <h3>Legal notice (English)</h3>
  <p class="small">Eightcount is run by {name}, {street}, {city}, Germany. Contact: {mail}.
  We are neither willing nor obliged to take part in dispute resolution before a consumer arbitration board.</p>
  {BACK}"#,
        name = OPERATOR_NAME,
        street = OPERATOR_STREET,
        city = OPERATOR_CITY,
        country = OPERATOR_COUNTRY,
        mail = CONTACT_EMAIL,
    )
}

pub fn delete_account_html() -> String {
    format!(
        r#"<p class="sub">You can delete your Eightcount account yourself, at any time.</p>
  <h3>In the app or on the website</h3>
  <ol>
    <li>Log in.</li>
    <li>Open the menu with your nickname (top right) → <b>Account settings…</b></li>
    <li>Click <b>Delete my account…</b>, enter your password and confirm.</li>
  </ol>
  <h3>What is deleted</h3>
  <p class="small">Right away and for good: your account (nickname, email, password, settings), the teams you own
  with their choreographies and music, your memberships in other teams, your sessions and your usage
  statistics. Choreographies in teams owned by others stay in those teams. We send one last email to confirm it.</p>
  <h3>Can't log in?</h3>
  <p class="small">Reset your password with <a href="/forgot">Forgot your password?</a>, or write from your
  account's email address to <a href="mailto:{mail}">{mail}</a> and we delete it for you.</p>
  {BACK}"#,
        mail = CONTACT_EMAIL,
    )
}

pub const ACCOUNT_DELETED: &str = r#"<p class="sub">Your account and everything that belonged only to it
  were deleted. We sent a last email to confirm it. Thanks for using Eightcount.</p>
  <p class="alt"><a href="/signup">Create a new account</a></p>"#;

pub async fn impressum() -> Response {
    crate::web::page("Impressum", &impressum_html())
}

pub async fn delete_account() -> Response {
    crate::web::page("Delete your account", &delete_account_html())
}

pub async fn account_deleted() -> Response {
    crate::web::page("Account deleted", ACCOUNT_DELETED)
}

/// `/imprint` and `/legal` are common guesses.
pub async fn to_impressum() -> Response {
    axum::response::Redirect::permanent("/impressum").into_response()
}
