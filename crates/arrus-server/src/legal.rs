//! Legal pages, reachable without logging in: the Impressum (§ 5 DDG, with
//! the DSA contact point), the privacy policy, the terms of use, and how to
//! delete an account (linked from the app stores).
//!
//! The German texts are the binding ones; the English ones are translations.

use axum::response::{IntoResponse, Response};

/// Who runs Eightcount (§ 5 DDG).
pub const OPERATOR_NAME: &str = "Mathis Aibache";
pub const OPERATOR_STREET: &str = "Stephanstraße 23";
pub const OPERATOR_CITY: &str = "52064 Aachen";
pub const OPERATOR_COUNTRY: &str = "Deutschland / Germany";
pub const CONTACT_EMAIL: &str = "contact@eightcount.de";
/// When the privacy policy and the terms last changed.
pub const LEGAL_DATE_DE: &str = "12. November 2026";
pub const LEGAL_DATE_EN: &str = "12 November 2026";

/// The small links under the login and sign-up forms and every small page.
pub const FOOTER: &str = r#"<p class="legal"><a href="/impressum">Impressum</a> · <a href="/datenschutz">Datenschutz / Privacy</a> · <a href="/nutzungsbedingungen">Nutzungsbedingungen / Terms</a></p>"#;

const BACK: &str = r#"<p class="alt"><a href="/">Open Eightcount</a> · <a href="/login">Log in</a></p>"#;

/// Fill in the operator's details: `{name}`, `{street}`, `{city}`, `{mail}`, `{date}`.
fn fill(s: &str, date: &str) -> String {
    s.replace("{name}", OPERATOR_NAME)
        .replace("{street}", OPERATOR_STREET)
        .replace("{city}", OPERATOR_CITY)
        .replace("{country}", OPERATOR_COUNTRY)
        .replace("{mail}", CONTACT_EMAIL)
        .replace("{date}", date)
}

const IMPRESSUM: &str = r##"<h3>Angaben gemäß § 5 DDG</h3>
  <p>{name}<br>{street}<br>{city}<br>{country}</p>
  <h3>Kontakt / Contact</h3>
  <p>E-Mail: <a href="mailto:{mail}">{mail}</a></p>
  <h3>Verantwortlich für den Inhalt</h3>
  <p>{name}, Anschrift wie oben.</p>
  <h3>Kontaktstelle nach dem Digital Services Act (Art. 11, 12 DSA)</h3>
  <p class="small">Für Behörden und Nutzer: <a href="mailto:{mail}">{mail}</a>, auf Deutsch oder Englisch.
  Rechtswidrige Inhalte kannst du ebenfalls dorthin melden (siehe <a href="/nutzungsbedingungen#melden">Nutzungsbedingungen</a>).</p>
  <h3>Verbraucherstreitbeilegung</h3>
  <p class="small">Wir sind nicht bereit und nicht verpflichtet, an Streitbeilegungsverfahren vor einer
  Verbraucherschlichtungsstelle teilzunehmen.</p>
  <h3>Legal notice (English)</h3>
  <p class="small">Eightcount is run by {name}, {street}, {city}, Germany. Contact: {mail}. This address is also the
  single point of contact under the EU Digital Services Act (Art. 11, 12 DSA), in German or English, and where to
  report illegal content. We are neither willing nor obliged to take part in dispute resolution before a consumer
  arbitration board.</p>
  <p class="small"><a href="/datenschutz">Datenschutzerklärung / Privacy policy</a> ·
  <a href="/nutzungsbedingungen">Nutzungsbedingungen / Terms of use</a></p>"##;

const PRIVACY_DE: &str = r##"<p class="small">Stand: {date} · <a href="#en">English version below</a></p>
  <p class="small">Eightcount ist eine Web-App (mit Apps für Windows, macOS, Linux, Android und iPhone), mit der Teams
  Choreografien planen. Wir zeigen keine Werbung, setzen keine Tracking- oder Analyse-Dienste Dritter ein und verkaufen
  keine Daten. Hier steht, welche Daten wir verarbeiten, wozu und wie lange.</p>

  <h3>1. Verantwortlicher</h3>
  <p class="small">{name}, {street}, {city}, Deutschland. E-Mail: <a href="mailto:{mail}">{mail}</a>.
  Ein Datenschutzbeauftragter ist nicht benannt (nicht vorgeschrieben).</p>

  <h3>2. Hosting und Aufruf der Website</h3>
  <p class="small">Eightcount läuft auf einem eigenen Server bei der OVH SAS, 2 rue Kellermann, 59100 Roubaix, Frankreich,
  in einem Rechenzentrum in Gravelines (Frankreich). Mit OVH besteht ein Vertrag über die Auftragsverarbeitung
  (Art. 28 DSGVO). Alle unten genannten Daten werden dort gespeichert.</p>
  <p class="small">Bei jedem Aufruf verarbeitet der Server technisch notwendig deine IP-Adresse, um die Verbindung
  herzustellen und die Seite auszuliefern. IP-Adressen werden <b>nicht</b> in Protokollen gespeichert. Für die
  Betriebsüberwachung zählt die Server-Software nur: die ungefähre Zahl verschiedener Besucher pro Tag (über einen
  geheimen, nicht umkehrbaren Hash; keine Adresse wird gespeichert, 62 Tage), fehlgeschlagene Anfragen nach Pfad ohne
  Parameter und Kennungen (8 Tage) und die übertragene Datenmenge. Fehlermeldungen des Programms werden in
  Server-Protokollen festgehalten, die automatisch rotiert werden; sie enthalten keine IP- und keine E-Mail-Adressen.
  Rechtsgrundlage ist Art. 6 Abs. 1 lit. f DSGVO (sicherer und stabiler Betrieb).</p>

  <h3>3. Dein Konto</h3>
  <p class="small">Für ein Konto brauchen wir einen Spitznamen, eine E-Mail-Adresse und ein Passwort. Das Passwort
  speichern wir nur als Hash (Argon2); niemand kann es lesen. Hat der Betreiber dein Konto angelegt, bleibt das
  erzeugte Startpasswort lesbar, bis du ein eigenes wählst. Außerdem speichern wir das Anlegedatum, ob deine
  E-Mail-Adresse bestätigt ist, deine Einstellungen (z.&nbsp;B. Farben), deinen Tarif (Free/Premium) und wann du den
  Nutzungsbedingungen zugestimmt hast. Rechtsgrundlage: Art. 6 Abs. 1 lit. b DSGVO (Vertrag). Gespeichert, bis du dein
  Konto löschst.</p>

  <h3>4. Deine Inhalte</h3>
  <p class="small">Teams und ihre Mitglieder mit Rollen, Einladungslinks, Choreografien (mit Namen der Tänzerinnen und
  Tänzer, Formationen, Notizen, Hintergrundbildern), hochgeladene Musik, und wer eine Choreografie zuletzt geändert hat.
  Die Mitglieder eines Teams sehen deinen Spitznamen, deine Änderungen und – während ihr gleichzeitig arbeitet – wer die
  Choreografie gerade geöffnet hat (nur im Arbeitsspeicher). Rechtsgrundlage: Art. 6 Abs. 1 lit. b DSGVO. Gespeichert,
  bis du die Inhalte, das Team oder dein Konto löschst. Trägst du Namen anderer Personen ein (z.&nbsp;B. Tänzer), achte
  bitte darauf, dass sie damit einverstanden sind.</p>

  <h3>5. Cookie und Speicher im Browser</h3>
  <p class="small">Nach dem Anmelden setzen wir ein einziges Cookie (<code>arus_session</code>, HttpOnly, 30 Tage, bei
  Nutzung verlängert), das dich angemeldet hält. Einige Geräte-Einstellungen legt die App im lokalen Speicher deines
  Browsers ab. Beides ist für den Dienst unbedingt erforderlich (§ 25 Abs. 2 Nr. 2 TDDDG); eine Einwilligung ist
  dafür nicht nötig. Es gibt keine Tracking- oder Werbe-Cookies.</p>

  <h3>6. E-Mails</h3>
  <p class="small">Wir senden nur E-Mails, die zu deinem Konto gehören: Bestätigung der E-Mail-Adresse, Link zum
  Zurücksetzen des Passworts sowie Hinweise, wenn deine E-Mail-Adresse oder dein Passwort geändert oder dein Konto
  gelöscht wurde. Versendet werden sie über Brevo (Sendinblue SAS, 106 boulevard Haussmann, 75008 Paris, Frankreich)
  als Auftragsverarbeiter; Brevo erhält dazu deine E-Mail-Adresse, deinen Spitznamen und den Inhalt der E-Mail und
  speichert Versandprotokolle nach eigenen Fristen. Rechtsgrundlage: Art. 6 Abs. 1 lit. b DSGVO, für die
  Sicherheitshinweise auch lit. f. Links in E-Mails sind 24 Stunden (Bestätigung) bzw. 1 Stunde (Passwort) gültig;
  wir speichern sie nur als Hash.</p>

  <h3>7. Kontakt</h3>
  <p class="small">Schreibst du uns an {mail}, verarbeiten wir deine Nachricht und Adresse, um zu antworten. Das
  Postfach liegt bei der Proton AG, Route de la Galaise 32, 1228 Plan-les-Ouates, Schweiz (für die Schweiz besteht
  ein Angemessenheitsbeschluss der EU-Kommission). Rechtsgrundlage: Art. 6 Abs. 1 lit. b oder f DSGVO. Wir löschen
  die Nachrichten, wenn sie erledigt sind, außer wir müssen sie aufbewahren.</p>

  <h3>8. Nutzungsstatistik</h3>
  <p class="small">Damit wir verstehen, wie Eightcount genutzt wird, Kapazität planen und Missbrauch erkennen, zählen
  wir pro Konto und Tag: Minuten in der App (in 5-Minuten-Schritten), Anmeldungen, gespeicherte Änderungen, neu
  angelegte Choreografien, Downloads (.arrus/PDF) und Musik-Uploads. Nur der Betreiber sieht diese Zahlen; es gibt
  keine Weitergabe und kein Profiling zu Werbezwecken. Rechtsgrundlage: Art. 6 Abs. 1 lit. f DSGVO. Die Zahlen werden
  nach höchstens 730 Tagen gelöscht, mit deinem Konto sofort. Du kannst jederzeit widersprechen (Art. 21 DSGVO),
  eine E-Mail genügt.</p>

  <h3>9. App-Downloads und Weiterleitung</h3>
  <p class="small">Die Installationsdateien der Apps liegen bei GitHub (GitHub, Inc., USA). Lädst du eine App über den
  Download-Bereich herunter, verarbeitet GitHub deine IP-Adresse; GitHub ist nach dem EU-US Data Privacy Framework
  zertifiziert. Lädst du die App aus dem App Store oder Google Play, verarbeiten Apple bzw. Google Daten in eigener
  Verantwortung nach ihren Datenschutzbestimmungen. Die Adresse eightcount.de wird vorübergehend über den
  Domain-Anbieter Porkbun LLC (USA) zu unserer Website weitergeleitet; dabei verarbeitet Porkbun technisch deine
  IP-Adresse. Rechtsgrundlage jeweils Art. 6 Abs. 1 lit. f DSGVO.</p>

  <h3>10. Weitere Empfänger</h3>
  <p class="small">Darüber hinaus geben wir keine Daten weiter, außer wir sind gesetzlich dazu verpflichtet.</p>

  <h3>11. Deine Rechte</h3>
  <p class="small">Du hast das Recht auf Auskunft (Art. 15), Berichtigung (Art. 16), Löschung (Art. 17), Einschränkung
  der Verarbeitung (Art. 18), Datenübertragbarkeit (Art. 20; Choreografien kannst du selbst als .arrus oder PDF
  exportieren) und Widerspruch (Art. 21 DSGVO). Dein Konto kannst du in den Kontoeinstellungen jederzeit selbst löschen
  (<a href="/delete-account">so geht's</a>). Für alles andere genügt eine E-Mail an {mail}. Außerdem kannst du dich
  bei einer Datenschutz-Aufsichtsbehörde beschweren, z.&nbsp;B. der für uns zuständigen: Landesbeauftragte für
  Datenschutz und Informationsfreiheit Nordrhein-Westfalen, Kavalleriestraße 2–4, 40213 Düsseldorf.</p>

  <h3>12. Sonstiges</h3>
  <p class="small">Die Angabe von Spitzname, E-Mail-Adresse und Passwort ist für ein Konto erforderlich; ohne sie kann
  Eightcount nicht genutzt werden. Es gibt keine automatisierte Entscheidungsfindung im Sinne von Art. 22 DSGVO.
  Eightcount richtet sich an Personen ab 16 Jahren (jüngere mit Zustimmung der Eltern). Ändern wir diese Erklärung,
  steht das neue Datum oben.</p>

  <h3 id="en">Privacy policy (English)</h3>
  <p class="small">Last updated: {date_en}. This is a translation; the German version above is binding.</p>
  <p class="small"><b>Controller:</b> {name}, {street}, {city}, Germany, {mail}. No ads, no third-party tracking or
  analytics, no selling of data.</p>
  <p class="small"><b>Hosting:</b> our own server at OVH SAS (Roubaix, France), data centre in Gravelines, France, under a
  data processing agreement. Your IP address is processed to deliver the site but not stored in logs; the server only
  keeps an approximate daily visitor count (via a secret one-way hash, no addresses, 62 days), failed requests by path
  without parameters (8 days) and traffic volume. Error logs are rotated automatically and contain no IP or email
  addresses. Legal basis: Art. 6(1)(f) GDPR.</p>
  <p class="small"><b>Account:</b> nickname, email, password (stored only as an Argon2 hash; a start password created by
  the operator stays readable until you choose your own), sign-up date, email confirmation, settings, plan and when you
  accepted the terms. Art. 6(1)(b) GDPR; kept until you delete your account.</p>
  <p class="small"><b>Your content:</b> teams, members and roles, invite links, choreographies (including dancers'
  names, formations, notes, background images), music and who last changed what. Team members see your nickname,
  changes and live presence (in memory only). Art. 6(1)(b); kept until you delete it. Please only enter other people's
  names with their consent.</p>
  <p class="small"><b>Cookie and browser storage:</b> one session cookie (<code>arus_session</code>, 30 days) and a few
  device settings in local storage, both strictly necessary (§ 25(2) no. 2 TDDDG); no tracking cookies.</p>
  <p class="small"><b>Emails:</b> only account emails (confirm address, password reset, notices when your email or
  password changed or your account was deleted), sent via Brevo (Sendinblue SAS, Paris, France) as processor, which
  receives your email address, nickname and the message. Art. 6(1)(b), security notices also (f). Links expire after
  24 h / 1 h and are stored only as hashes.</p>
  <p class="small"><b>Contact:</b> messages to {mail} are kept in a mailbox at Proton AG (Switzerland, adequacy
  decision) to answer you, and deleted when done. Art. 6(1)(b) or (f).</p>
  <p class="small"><b>Usage statistics:</b> per account and day: minutes in the app (5-minute steps), logins, saved
  edits, choreographies created, downloads and music uploads, to understand usage, plan capacity and spot abuse. Only
  the operator sees them. Art. 6(1)(f); deleted after at most 730 days, and with your account. You can object at any
  time (Art. 21) by email.</p>
  <p class="small"><b>Downloads and forwarding:</b> app installers are hosted on GitHub (GitHub, Inc., USA, certified
  under the EU-US Data Privacy Framework), which processes your IP address when you download one. App stores (Apple,
  Google) process data under their own responsibility. eightcount.de is temporarily forwarded by the registrar Porkbun
  LLC (USA), which processes your IP address for that. Art. 6(1)(f).</p>
  <p class="small"><b>Your rights:</b> access, rectification, erasure, restriction, data portability (export your
  choreographies as .arrus or PDF) and objection (Art. 15–18, 20, 21 GDPR). Delete your account yourself in the account
  settings (<a href="/delete-account">how</a>); for anything else email {mail}. You may complain to a supervisory
  authority, e.g. ours: Landesbeauftragte für Datenschutz und Informationsfreiheit Nordrhein-Westfalen, Kavalleriestraße
  2–4, 40213 Düsseldorf, Germany.</p>
  <p class="small"><b>Other:</b> nickname, email and password are needed for an account. No automated decision-making
  (Art. 22). Eightcount is meant for people aged 16 and over (younger with a parent's consent).</p>"##;

const TERMS_DE: &str = r##"<p class="small">Stand: {date} · <a href="#en">English version below</a></p>

  <h3>1. Anbieter und Geltung</h3>
  <p class="small">Eightcount wird von {name}, {street}, {city}, Deutschland ({mail}) betrieben („wir“). Diese
  Bedingungen gelten für die Nutzung der Website und der Apps von Eightcount.</p>

  <h3>2. Leistungen</h3>
  <p class="small">Mit Eightcount planst du Choreografien allein oder im Team. Der kostenlose Tarif (Free) umfasst ein
  eigenes Team mit bis zu zwei Choreografien, ohne Musik. Der Tarif Premium umfasst bis zu 20 Teams mit je bis zu 20
  Choreografien und 250 MB Musikspeicher. Premium wird derzeit nicht verkauft, sondern von uns im Einzelfall
  freigeschaltet; wird Premium kostenpflichtig angeboten, gelten dafür zusätzliche Bedingungen, denen du vor dem Kauf
  zustimmst. Wir entwickeln Eightcount weiter und können Funktionen ändern. Eine bestimmte Verfügbarkeit schulden wir
  bei der kostenlosen Nutzung nicht; Wartungen und Störungen sind möglich.</p>

  <h3>3. Konto</h3>
  <p class="small">Du musst mindestens 16 Jahre alt sein; Jüngere brauchen die Zustimmung ihrer Eltern. Gib eine
  E-Mail-Adresse an, die dir gehört, halte dein Passwort geheim und nutze dein Konto nur selbst. Du kannst dein Konto
  jederzeit in den Kontoeinstellungen löschen; dabei werden auch die Teams gelöscht, die dir gehören.</p>

  <h3>4. Deine Inhalte</h3>
  <p class="small">Was du hochlädst oder anlegst (Choreografien, Namen, Bilder, Musik), bleibt deins. Du erlaubst uns,
  es zu speichern, zu verarbeiten und den Mitgliedern deiner Teams anzuzeigen, soweit das für Eightcount nötig ist.
  Du bist dafür verantwortlich, dass du die Rechte daran hast: Lade nur Musik und Bilder hoch, die du nutzen darfst,
  und trage Namen anderer nur mit deren Einverständnis ein. Exportiere wichtige Choreografien regelmäßig (.arrus oder
  PDF); wir können nicht garantieren, dass nie Daten verloren gehen.</p>

  <h3>5. Nicht erlaubt</h3>
  <p class="small">Rechtswidrige Inhalte (z.&nbsp;B. Urheberrechtsverletzungen, Beleidigungen, Hass, Gewaltdarstellungen),
  Schadsoftware, Versuche, Grenzen oder Sicherheitsmaßnahmen zu umgehen, automatisierte Massenzugriffe, das Überlasten
  des Dienstes und der Weiterverkauf des Zugangs.</p>

  <h3 id="melden">6. Rechtswidrige Inhalte melden</h3>
  <p class="small">Hältst du einen Inhalt auf Eightcount für rechtswidrig, schreib an {mail}: welcher Inhalt (Team,
  Choreografie), warum er rechtswidrig ist, deinen Namen und deine E-Mail-Adresse (außer bei bestimmten Straftaten) und
  dass deine Angaben nach bestem Wissen richtig sind. Wir prüfen jede Meldung zügig, sorgfältig und objektiv, sperren
  oder entfernen rechtswidrige Inhalte und informieren dich und die betroffene Person über unsere Entscheidung und die
  Gründe. Gegen eine Entscheidung kannst du per E-Mail Einwände erheben; wir prüfen sie erneut.</p>

  <h3>7. Sperren und Löschen durch uns</h3>
  <p class="small">Verstößt du gegen diese Bedingungen, können wir Inhalte entfernen und dein Konto sperren oder
  löschen; außer bei schweren Verstößen weisen wir dich vorher darauf hin. Den kostenlosen Dienst können wir mit einer
  Frist von vier Wochen per E-Mail beenden; du kannst deine Choreografien vorher exportieren.</p>

  <h3>8. Haftung</h3>
  <p class="small">Da die Nutzung kostenlos ist, haften wir nur für Vorsatz und grobe Fahrlässigkeit. Unberührt bleibt
  die Haftung für Schäden an Leben, Körper oder Gesundheit, nach dem Produkthaftungsgesetz und bei arglistig
  verschwiegenen Mängeln.</p>

  <h3>9. Änderungen</h3>
  <p class="small">Ändern wir diese Bedingungen, informieren wir dich mindestens vier Wochen vorher per E-Mail. Bist du
  nicht einverstanden, kannst du dein Konto vorher löschen. Wesentliche Änderungen deiner Rechte und Pflichten gelten
  nur, wenn du ihnen zustimmst.</p>

  <h3>10. Schlussbestimmungen</h3>
  <p class="small">Es gilt deutsches Recht unter Ausschluss des UN-Kaufrechts. Bist du Verbraucher, bleiben zwingende
  Schutzvorschriften des Landes, in dem du lebst, unberührt. Sollte eine Bestimmung unwirksam sein, bleiben die übrigen
  wirksam. Wie wir mit Daten umgehen, steht in der <a href="/datenschutz">Datenschutzerklärung</a>.</p>

  <h3 id="en">Terms of use (English)</h3>
  <p class="small">Last updated: {date_en}. This is a translation; the German version above is binding.</p>
  <p class="small"><b>1. Provider.</b> Eightcount is run by {name}, {street}, {city}, Germany ({mail}). These terms
  apply to the website and the apps.</p>
  <p class="small"><b>2. Service.</b> Plan choreographies alone or as a team. Free: one team of your own with up to two
  choreographies, no music. Premium: up to 20 teams with up to 20 choreographies each and 250 MB of music. Premium is
  not sold yet but granted by us case by case; once it is sold, additional terms apply that you accept before buying.
  Features may change; the free service comes without a guaranteed availability.</p>
  <p class="small"><b>3. Account.</b> You must be 16 or older (younger with a parent's consent), use an email address
  of your own, keep your password secret and not share your account. Delete your account at any time in the account
  settings; the teams you own are deleted with it.</p>
  <p class="small"><b>4. Your content</b> stays yours. You allow us to store and process it and show it to your teams'
  members as far as Eightcount needs. You are responsible for having the rights to it: only upload music and images you
  may use, and only enter other people's names with their consent. Export important choreographies regularly; we cannot
  guarantee that data is never lost.</p>
  <p class="small"><b>5. Not allowed:</b> illegal content (e.g. copyright infringement, insults, hate, violence),
  malware, getting around limits or security, automated mass access, overloading the service, reselling access.</p>
  <p class="small"><b>6. Reporting illegal content.</b> Email {mail} with the content concerned (team, choreography),
  why it is illegal, your name and email (except for certain crimes) and a statement that your report is accurate to the
  best of your knowledge. We review reports promptly, diligently and objectively, block or remove illegal content, and
  tell you and the person concerned what we decided and why. You can contest a decision by email.</p>
  <p class="small"><b>7. Suspension.</b> If you break these terms we may remove content and suspend or delete your
  account, warning you first except for serious violations. We may end the free service with four weeks' notice by
  email; you can export your choreographies before.</p>
  <p class="small"><b>8. Liability.</b> As the service is free, we are liable only for intent and gross negligence,
  except for injury to life, body or health, under the Product Liability Act, and for fraudulently concealed defects.</p>
  <p class="small"><b>9. Changes.</b> We announce changes to these terms by email at least four weeks ahead; if you
  disagree you can delete your account. Material changes to your rights and duties need your consent.</p>
  <p class="small"><b>10. Final provisions.</b> German law applies, excluding the UN Convention on the International
  Sale of Goods; mandatory consumer protection of your country of residence remains. If a provision is invalid, the rest
  stays valid. See the <a href="/datenschutz">privacy policy</a> for how we handle data.</p>"##;

pub fn impressum_html() -> String {
    format!("{}\n  {BACK}", fill(IMPRESSUM, LEGAL_DATE_DE))
}

pub fn privacy_html() -> String {
    format!("{}\n  {BACK}", fill(PRIVACY_DE, LEGAL_DATE_DE).replace("{date_en}", LEGAL_DATE_EN))
}

pub fn terms_html() -> String {
    format!("{}\n  {BACK}", fill(TERMS_DE, LEGAL_DATE_DE).replace("{date_en}", LEGAL_DATE_EN))
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
    crate::web::doc_page("Impressum", &impressum_html())
}

pub async fn privacy() -> Response {
    crate::web::doc_page("Datenschutzerklärung / Privacy policy", &privacy_html())
}

pub async fn terms() -> Response {
    crate::web::doc_page("Nutzungsbedingungen / Terms of use", &terms_html())
}

pub async fn delete_account() -> Response {
    crate::web::page("Delete your account", &delete_account_html())
}

pub async fn account_deleted() -> Response {
    crate::web::page("Account deleted", ACCOUNT_DELETED)
}

/// Common guesses for the pages' addresses.
pub async fn to_impressum() -> Response {
    axum::response::Redirect::permanent("/impressum").into_response()
}

pub async fn to_privacy() -> Response {
    axum::response::Redirect::permanent("/datenschutz").into_response()
}

pub async fn to_terms() -> Response {
    axum::response::Redirect::permanent("/nutzungsbedingungen").into_response()
}
