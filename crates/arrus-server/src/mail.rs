//! Emails: confirming an address and resetting a password.
//!
//! Sent through Brevo's transactional API (HTTPS, so no SMTP port has to be
//! open), or kept in memory for tests. Sending never blocks a request: a
//! failure is logged, and people can ask for the link again.

use std::sync::Mutex;
use std::time::Duration;

use crate::config::{Mail as MailConfig, MailTransport};

/// One email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub to: String,
    pub to_name: String,
    pub subject: String,
    pub text: String,
    pub html: String,
}

pub struct Mailer {
    config: MailConfig,
    client: reqwest::Client,
    /// Mails kept by the in-memory transport (tests).
    outbox: Mutex<Vec<Mail>>,
}

const BREVO_URL: &str = "https://api.brevo.com/v3/smtp/email";

impl Mailer {
    pub fn new(config: MailConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, client, outbox: Mutex::default() }
    }

    /// Send in the background.
    pub fn send(&self, mail: Mail) {
        match &self.config.transport {
            MailTransport::Memory => {
                tracing::info!(to = %mail.to, subject = %mail.subject, "mail (not sent):\n{}", mail.text);
                self.outbox.lock().unwrap_or_else(|p| p.into_inner()).push(mail);
            }
            MailTransport::Brevo { api_key } => {
                let mut body = serde_json::json!({
                    "sender": { "name": self.config.from_name, "email": self.config.from },
                    "to": [{ "email": mail.to, "name": mail.to_name }],
                    "subject": mail.subject,
                    "textContent": mail.text,
                    "htmlContent": mail.html,
                });
                if let Some(r) = &self.config.reply_to {
                    body["replyTo"] = serde_json::json!({ "email": r });
                }
                let req = self
                    .client
                    .post(BREVO_URL)
                    .header("api-key", api_key)
                    .header("accept", "application/json")
                    .json(&body);
                tokio::spawn(async move {
                    match req.send().await {
                        Ok(r) if r.status().is_success() => {
                            tracing::info!(to = %mail.to, subject = %mail.subject, "mail sent");
                        }
                        Ok(r) => {
                            let status = r.status();
                            let text = r.text().await.unwrap_or_default();
                            tracing::error!(to = %mail.to, %status, "Brevo refused the mail: {text}");
                        }
                        Err(e) => tracing::error!(to = %mail.to, "mail not sent: {e}"),
                    }
                });
            }
        }
    }

    /// What the in-memory transport kept (tests).
    pub fn outbox(&self) -> Vec<Mail> {
        self.outbox.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A short HTML mail in the site's colours, with one button.
fn html(heading: &str, lines: &[&str], button: &str, link: &str, after: &str) -> String {
    let paras: String = lines.iter().map(|l| format!("<p style=\"margin:0 0 14px\">{}</p>", esc(l))).collect();
    format!(
        "<!doctype html><html><body style=\"margin:0;background:#f3f3f5;padding:24px 12px;\
         font-family:system-ui,-apple-system,'Segoe UI',Roboto,sans-serif;color:#1a1a1d\">\
         <div style=\"max-width:480px;margin:0 auto;background:#fff;border-radius:12px;padding:28px 24px\">\
         <div style=\"font-size:22px;font-weight:700;color:#7c4ff0;margin-bottom:18px\">Eightcount</div>\
         <h1 style=\"font-size:18px;margin:0 0 14px\">{heading}</h1>{paras}\
         <p style=\"margin:22px 0\"><a href=\"{href}\" style=\"background:#7c4ff0;color:#fff;text-decoration:none;\
         padding:12px 20px;border-radius:8px;font-weight:600;display:inline-block\">{button}</a></p>\
         <p style=\"margin:0 0 14px;font-size:13px;color:#707078\">If the button doesn't work, open this link:<br>\
         <a href=\"{href}\" style=\"color:#7c4ff0;word-break:break-all\">{href}</a></p>\
         <p style=\"margin:0;font-size:13px;color:#707078\">{after}</p></div></body></html>",
        heading = esc(heading),
        button = esc(button),
        href = esc(link),
        after = esc(after),
    )
}

/// "Confirm your email" (after signing up or changing the address).
pub fn confirm_email(to: &str, nickname: &str, link: &str) -> Mail {
    let hello = format!("Hi {nickname},");
    let intro = "please confirm that this is your email address for Eightcount. \
                 With it you can get back into your account if you forget your password.";
    let after = "The link works for 24 hours. If you didn't create an Eightcount account, ignore this email.";
    Mail {
        to: to.to_string(),
        to_name: nickname.to_string(),
        subject: "Confirm your email for Eightcount".into(),
        text: format!("{hello}\n\n{intro}\n\nConfirm it here:\n{link}\n\n{after}\n"),
        html: html("Confirm your email", &[&hello, intro], "Confirm my email", link, after),
    }
}

/// "Reset your password", with the nickname (in case that was forgotten too).
pub fn reset_password(to: &str, nickname: &str, link: &str) -> Mail {
    let hello = format!("Hi {nickname},");
    let intro = "someone (hopefully you) asked to reset the password of your Eightcount account.";
    let name = format!("Your nickname is {nickname}. You can log in with it or with this email address.");
    let after = "The link works for 1 hour and only once. If you didn't ask for this, ignore this email: \
                 your password stays as it is.";
    Mail {
        to: to.to_string(),
        to_name: nickname.to_string(),
        subject: "Reset your Eightcount password".into(),
        text: format!("{hello}\n\n{intro}\n\n{name}\n\nChoose a new password here:\n{link}\n\n{after}\n"),
        html: html("Reset your password", &[&hello, intro, &name], "Choose a new password", link, after),
    }
}
