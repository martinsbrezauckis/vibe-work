//! Protected field labels in more languages: PIN and password words withheld, one-time codes readable.
//! Uses the headless browser driver; all fixtures are offline and no form is submitted.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

async fn page(html: &str) -> Rig {
    let rig = Rig::new().await;
    rig.engine.viewport(1280, 800).await;
    rig.engine.eval(&format!("document.head.innerHTML=''; document.body.style='margin:0'; document.body.innerHTML={}; scrollTo(0,0); document.addEventListener('submit', e=>e.preventDefault())", json!(html))).await;
    rig
}

async fn read(rig: &Rig, find: Option<&str>) -> Value {
    rig.browser
        .read(
            SESSION,
            ReadOptions {
                find: find.map(str::to_owned),
                ..ReadOptions::default()
            },
            false,
            Actor::Agent,
        )
        .await
        .unwrap()
        .0
}

fn refs(answer: &Value) -> &[Value] {
    answer["untrustedPageContent"]["refs"].as_array().unwrap()
}

// Field label -> expected redaction kind ("" = readable). PIN and password words in more languages, the one-time
// codes in those languages, and ordinary labels that share a word with them.
const MORE_LANGUAGE_CASES: [(&str, &str); 66] = [
    ("Geheimzahl", "password"),
    ("Persönliche Geheimzahl", "password"),
    ("Geheimnummer", "password"),
    ("PIN-Nummer", "password"),
    ("PIN-Code", "password"),
    ("Code secret", "password"),
    ("Code confidentiel", "password"),
    ("Número secreto", "password"),
    ("Clave", "password"),
    ("Clave de acceso", "password"),
    ("Codice segreto", "password"),
    ("ПИН", "password"),
    ("ПИН-код", "password"),
    ("PIN-kood", "password"),
    ("PIN kodas", "password"),
    ("PIN kods", "password"),
    ("密码", "password"),
    ("请输入密码", "password"),
    ("密码Password", "password"),
    ("密碼", "password"),
    ("パスワード", "password"),
    ("パスワードを入力", "password"),
    ("暗証番号", "password"),
    ("비밀번호", "password"),
    ("비밀번호를 입력하세요", "password"),
    ("Hasło", "password"),
    ("Wachtwoord", "password"),
    ("Senha", "password"),
    ("Lösenord", "password"),
    ("Salasana", "password"),
    ("Heslo", "password"),
    ("Şifre", "password"),
    ("Parola", "password"),
    ("Passord", "password"),
    ("Adgangskode", "password"),
    ("Jelszó", "password"),
    ("Κωδικός πρόσβασης", "password"),
    ("Συνθηματικό", "password"),
    ("Contraseña", "password"),
    ("Mot de passe", "password"),
    ("Пароль", "password"),
    ("密码或一次性密码", "password"),
    ("Verification code", ""),
    ("Code postal", ""),
    ("Código postal", ""),
    ("Número de pedido", ""),
    ("Enclave", ""),
    ("Parolee name", ""),
    ("验证码", ""),
    ("一次性密码", ""),
    ("動態密碼", ""),
    ("ワンタイムパスワード", ""),
    ("認証コード", ""),
    ("인증번호", ""),
    ("일회용 비밀번호", ""),
    ("Hasło jednorazowe", ""),
    ("Eenmalig wachtwoord", ""),
    ("Senha de uso único", ""),
    ("Kertakäyttöinen salasana", ""),
    ("Jednorázové heslo", ""),
    ("Tek kullanımlık şifre", ""),
    ("Clave dinámica", ""),
    ("Κωδικός επιβεβαίωσης", ""),
    ("Κωδικός", ""),
    ("Kods", ""),
    ("Ort", ""),
];

// PREDICT (before the PIN and password words in more languages): of the protected rows, every label without an English
// PIN/password word or an existing local word is read in clear, both as a visible <label> and as an aria-label:
// "Geheimzahl", "Persönliche Geheimzahl", "Geheimnummer", "Code secret", "Code confidentiel", "Número secreto",
// "Clave", "Clave de acceso", "Codice segreto", "ПИН", "ПИН-код", the bare Chinese, Japanese and Korean words (a word
// list of whole words never matches inside "请输入密码"), "Hasło", "Wachtwoord", "Senha", "Lösenord", "Salasana",
// "Heslo", "Şifre", "Parola", "Passord", "Adgangskode", "Jelszó", "Κωδικός πρόσβασης", "Συνθηματικό" and
// "密码或一次性密码". "PIN-Nummer", "PIN-Code", "PIN-kood", "PIN kodas", "PIN kods", "密码Password", "Contraseña",
// "Mot de passe" and "Пароль" are already withheld (English PIN/password words, existing local words). Every readable
// row is already readable.
#[tokio::test]
async fn pin_and_password_words_in_more_languages_protect_fields_while_codes_stay_readable() {
    // (field HTML, accessible name, expected kind, value): each label once as a visible <label>, once as an aria-label.
    let mut fields: Vec<(String, &str, &str, String)> = Vec::new();
    for (i, (label, kind)) in MORE_LANGUAGE_CASES.iter().enumerate() {
        let (shown, aria) = (format!("zq-l{i:02}-x"), format!("zq-a{i:02}-x"));
        fields.push((
            format!("<p><label for='l{i}'>{label}</label> <input id='l{i}' type='text' value='{shown}'></p>"),
            label,
            kind,
            shown,
        ));
        fields.push((
            format!("<p><input type='text' aria-label='{label}' value='{aria}'></p>"),
            label,
            kind,
            aria,
        ));
    }
    let mut wrong = Vec::new();
    for group in fields.chunks(12) {
        let html: String = group.iter().map(|f| f.0.as_str()).collect();
        let rig = page(&html).await;
        let answer = read(&rig, None).await;
        let shown = answer.to_string();
        for (_, label, kind, value) in group {
            let ok = if kind.is_empty() {
                refs(&answer).iter().any(|r| {
                    r["name"] == *label && r["value"] == value.as_str() && r["redacted"].is_null()
                })
            } else {
                refs(&answer)
                    .iter()
                    .any(|r| r["name"] == *label && r["redacted"] == *kind && r["value"].is_null())
                    && !shown.contains(value.as_str())
            };
            if !ok {
                wrong.push(format!("{label} ({value}): want {kind:?}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

// The two field-check pages of the 0.1.11 interface scan, as served there, plus a sign-in form whose "forgot password"
// links follow an ordinary e-mail field.
// PREDICT (before the PIN and password words in more languages): "DeGeheim-82002", "AriaGeheim-94004" and
// "DePin-11011" ("Geheimzahl" as a visible label and as an aria-label) appear in the read; every other withheld value
// is already withheld, and the ordinary, one-time-code and e-mail values are already readable.
#[tokio::test]
async fn interface_scan_field_pages_withhold_the_german_pin() {
    // One page each, so the read's size budget never drops a field.
    let pages = [
        concat!(
            "<form>",
            "<p><label for=\"l1\">Parole</label> <input id=\"l1\" name=\"l1\" type=\"text\" value=\"LvParole-71001\"></p>",
            "<p><label for=\"l2\">PIN kods</label> <input id=\"l2\" name=\"l2\" type=\"text\" inputmode=\"numeric\" value=\"LvPin-72002\"></p>",
            "<p><label for=\"l3\">Vārds</label> <input id=\"l3\" name=\"l3\" type=\"text\" value=\"LvVards-73003\"></p>",
            "<p><label for=\"d1\">Passwort</label> <input id=\"d1\" name=\"d1\" type=\"text\" value=\"DePass-81001\"></p>",
            "<p><label for=\"d2\">Geheimzahl</label> <input id=\"d2\" name=\"d2\" type=\"text\" inputmode=\"numeric\" value=\"DeGeheim-82002\"></p>",
            "<p><label for=\"d3\">Ort</label> <input id=\"d3\" name=\"d3\" type=\"text\" value=\"DeOrt-83003\"></p>",
            "<p>X1 <input id=\"x1\" name=\"x1\" type=\"text\" aria-label=\"Parole\" value=\"AriaParole-91001\"></p>",
            "<p>X2 <input id=\"x2\" name=\"x2\" type=\"text\" aria-label=\"PIN kods\" value=\"AriaPinLv-92002\"></p>",
            "<p>X3 <input id=\"x3\" name=\"x3\" type=\"text\" aria-label=\"Passwort\" value=\"AriaPass-93003\"></p>",
            "<p>X4 <input id=\"x4\" name=\"x4\" type=\"text\" aria-label=\"Geheimzahl\" value=\"AriaGeheim-94004\"></p>",
            "</form>",
        ),
        concat!(
            "<form>",
            "<p><label for=\"f01\">Account password</label> <input id=\"f01\" name=\"f01\" type=\"password\" value=\"TypePw-10101\"></p>",
            "<p><label for=\"f02\">Password</label> <input id=\"f02\" name=\"f02\" type=\"text\" value=\"LblPw-20202\"></p>",
            "<p><label for=\"f03\">Card PIN</label> <input id=\"f03\" name=\"f03\" type=\"text\" inputmode=\"numeric\" value=\"LblPin-30303\"></p>",
            "<p><label for=\"f04\">Verification code</label> <input id=\"f04\" name=\"f04\" type=\"text\" inputmode=\"numeric\" value=\"LblOtp-40404\"></p>",
            "<p><label for=\"f05\">Login code</label> <input id=\"f05\" name=\"f05\" type=\"text\" autocomplete=\"one-time-code\" value=\"AcOtc-50505\"></p>",
            "<p><label for=\"f06\">密码Password</label> <input id=\"f06\" name=\"f06\" type=\"text\" value=\"LblCjk-60606\"></p>",
            "<p><label for=\"f07\">Parole</label> <input id=\"f07\" name=\"f07\" type=\"text\" value=\"LvParole-70707\"></p>",
            "<p><label for=\"f08\">PIN kods</label> <input id=\"f08\" name=\"f08\" type=\"text\" value=\"LvPin-80808\"></p>",
            "<p><label for=\"f09\">Passwort</label> <input id=\"f09\" name=\"f09\" type=\"text\" value=\"DePw-90909\"></p>",
            "<p><label for=\"f10\">Geheimzahl</label> <input id=\"f10\" name=\"f10\" type=\"text\" value=\"DePin-11011\"></p>",
            "<p><label for=\"f11\">Password</label> <input id=\"f11\" name=\"f11\" type=\"text\" autocomplete=\"one-time-code\" value=\"LblPwOtc-12012\"></p>",
            "<p><label for=\"f12\">City</label> <input id=\"f12\" name=\"f12\" type=\"text\" value=\"Plain-13013\"></p>",
            "</form>",
        ),
        concat!(
            "<form>",
            "<p><label for=\"m1\">E-Mail</label> <input id=\"m1\" name=\"m1\" type=\"text\" value=\"MailDe-14014\"> <a href=\"#\">Passwort vergessen?</a></p>",
            "<p><label for=\"m2\">电子邮件</label> <input id=\"m2\" name=\"m2\" type=\"text\" value=\"MailZh-15015\"> <a href=\"#\">忘记密码?</a></p>",
            "</form>",
        ),
    ];
    let mut rows = Vec::new();
    let mut shown = String::new();
    for html in pages {
        let answer = read(&page(html).await, None).await;
        shown.push_str(&answer.to_string());
        rows.extend(refs(&answer).iter().cloned());
    }
    let mut wrong = Vec::new();
    for value in [
        "LvParole-71001",
        "LvPin-72002",
        "DePass-81001",
        "DeGeheim-82002",
        "AriaParole-91001",
        "AriaPinLv-92002",
        "AriaPass-93003",
        "AriaGeheim-94004",
        "TypePw-10101",
        "LblPw-20202",
        "LblPin-30303",
        "LblCjk-60606",
        "LvParole-70707",
        "LvPin-80808",
        "DePw-90909",
        "DePin-11011",
        "LblPwOtc-12012",
    ] {
        if shown.contains(value) {
            wrong.push(format!("{value} leaked"));
        }
    }
    for value in [
        "LvVards-73003",
        "DeOrt-83003",
        "LblOtp-40404",
        "AcOtc-50505",
        "Plain-13013",
        "MailDe-14014",
        "MailZh-15015",
    ] {
        if !rows.iter().any(|r| r["value"] == value) {
            wrong.push(format!("{value} not readable"));
        }
    }
    let german: Vec<&Value> = rows.iter().filter(|r| r["name"] == "Geheimzahl").collect();
    if german.len() != 3 || german.iter().any(|r| r["redacted"] != "password") {
        wrong.push(format!(
            "Geheimzahl: want 3 rows withheld as password, got {german:?}"
        ));
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

// Nearby label -> expected redaction kind ("" = readable), for a text field whose only label is an unassociated one in
// the same row (a preceding <div> or a <label> with no `for`). Each row is its own form, so no row borrows another's label.
const NEARBY_CASES: [(&str, &str); 8] = [
    ("Password", "password"),
    ("PIN", "password"),
    ("Card PIN", "password"),
    ("Passwort", "password"),
    ("One-time password", ""),
    ("Pass number", ""),
    ("Verification code", ""),
    ("Password or one-time password", "password"),
];

// PREDICT (before the English words counted in a nearby label): "Password", "PIN", "Card PIN" and "Password or one-time
// password" as a nearby-only label are read in clear, both as a preceding <div> and as an unassociated <label> (the
// English words were looked for in the attributes and the field's own labels only); "Passwort" is already withheld
// (local words already included the nearby label) and the three readable rows are already readable.
#[tokio::test]
async fn english_password_and_pin_words_in_a_nearby_label_protect_the_field() {
    let mut fields: Vec<(String, &str, &str, String)> = Vec::new();
    for (i, (label, kind)) in NEARBY_CASES.iter().enumerate() {
        let (div, lbl) = (format!("zq-nd{i:02}-x"), format!("zq-nl{i:02}-x"));
        fields.push((
            format!("<form><div>{label}</div><input type='text' value='{div}'></form>"),
            label,
            kind,
            div,
        ));
        fields.push((
            format!("<form><label>{label}</label><input type='text' value='{lbl}'></form>"),
            label,
            kind,
            lbl,
        ));
    }
    let html: String = fields.iter().map(|f| f.0.as_str()).collect();
    let answer = read(&page(&html).await, None).await;
    let shown = answer.to_string();
    let mut wrong = Vec::new();
    for (_, label, kind, value) in &fields {
        let ok = if kind.is_empty() {
            refs(&answer)
                .iter()
                .any(|r| r["value"] == value.as_str() && r["redacted"].is_null())
        } else {
            refs(&answer)
                .iter()
                .any(|r| r["name"] == *label && r["redacted"] == *kind && r["value"].is_null())
                && !shown.contains(value.as_str())
        };
        if !ok {
            wrong.push(format!("{label} ({value}): want {kind:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

// PREDICT (before a row's unassociated label stopped naming the next row's field): the second field of each page takes
// the "City" label of the row above as its nearby label instead of its own "Password"/"Passwort"/"PIN" text, so its
// value is read in clear, also when the row above holds a custom text box; the City field is already named "City".
#[tokio::test]
async fn a_previous_rows_label_never_names_the_next_rows_field() {
    let mut wrong = Vec::new();
    // The row above holds a native field or a custom text box.
    let above = [
        "<input type='text' value='{city}'>",
        "<div role='textbox' contenteditable='true'>{city}</div>",
    ];
    for (i, (word, control)) in [
        ("Password", 0),
        ("Passwort", 0),
        ("PIN", 0),
        ("Password", 1),
    ]
    .into_iter()
    .enumerate()
    {
        let (city, secret) = (format!("zq-city{i}-x"), format!("zq-secret{i}-x"));
        let above = above[control].replace("{city}", &city);
        let answer = read(
            &page(&format!(
                "<div><label>City</label>{above}</div>\
                 <div><div>{word}</div><input type='text' value='{secret}'></div>"
            ))
            .await,
            None,
        )
        .await;
        let shown = answer.to_string();
        if control == 0
            && !refs(&answer)
                .iter()
                .any(|r| r["name"] == "City" && r["value"] == city.as_str())
        {
            wrong.push(format!("{word}: City field not readable as City"));
        }
        if shown.contains(secret.as_str())
            || !refs(&answer).iter().any(|r| r["redacted"] == "password")
        {
            wrong.push(format!("{word}: field not withheld"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
