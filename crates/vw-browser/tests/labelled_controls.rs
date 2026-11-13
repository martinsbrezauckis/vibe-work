//! Labelled form discovery and read budgeting with a saved public form and small label fixtures.
//! Uses the headless browser driver; all fixtures are offline and no form is submitted.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use std::fmt::Write as _;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

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

#[tokio::test]
async fn each_label_kind_is_searchable_alongside_placeholder_and_value() {
    for (html, query) in [
        (
            "<label for=x>Explicit label</label><input id=x>",
            "Explicit label",
        ),
        (
            "<span id=t>Referenced label</span><input aria-labelledby=t>",
            "Referenced label",
        ),
        ("<label>Wrapping label<input></label>", "Wrapping label"),
        ("<input aria-label='Accessible label'>", "Accessible label"),
        (
            "<label>Account<input placeholder='Email address'></label>",
            "Email address",
        ),
        (
            "<input aria-label='Account' value='Readable value'>",
            "Readable value",
        ),
        (
            "<div><div><label>Nearby label</label></div><div><input></div></div>",
            "Nearby label",
        ),
        ("<div><span>Nearby text</span><input></div>", "Nearby text"),
        (
            "<fieldset><legend>Hobbies</legend><label><input type=checkbox>Reading</label></fieldset>",
            "Hobbies",
        ),
        (
            "<div><label>Subject</label><div><div><input role=combobox></div></div></div>",
            "Subject",
        ),
        (
            "<div><div id=p>Select City</div><div><input role=combobox aria-describedby=p></div></div>",
            "Select City",
        ),
        (
            "<div><div class=custom__single-value>NCR</div><div><input role=combobox aria-label=State></div></div>",
            "NCR",
        ),
    ] {
        let rig = page(html).await;
        let answer = read(&rig, Some(query)).await;
        assert!(!refs(&answer).is_empty(), "{query}: {answer}");
    }
}

#[tokio::test]
async fn saved_practice_form_exposes_labels_and_controls() {
    let rig = page(include_str!("support/practice_form.html")).await;
    for query in [
        "Date of Birth",
        "hobbies",
        "subject",
        "Select State",
        "Select City",
    ] {
        let answer = read(&rig, Some(query)).await;
        assert!(!refs(&answer).is_empty(), "{query}: {answer}");
    }
    let answer = read(&rig, None).await;
    assert!(
        refs(&answer).iter().any(|r| r["role"] == "textbox"),
        "full read missed inputs: {answer}"
    );
    assert!(
        refs(&answer).iter().any(|r| r["name"]
            .as_str()
            .is_some_and(|s| s.contains("Date of Birth"))),
        "full read missed date field: {answer}"
    );
}

#[tokio::test]
async fn form_and_main_content_precede_long_navigation_lists() {
    let mut links = String::new();
    for i in 0..1500 {
        write!(links, "<a href='/nav/{i}'>Navigation {i}</a><br>").unwrap();
    }
    let rig = page(&format!("<nav>{links}</nav><main><a href='/article'>Main article</a><form><label>Date of Birth<input></label><label>Subjects<input role=combobox></label></form></main>")).await;
    let answer = read(&rig, None).await;
    for name in ["Date of Birth", "Subjects", "Main article"] {
        assert!(
            refs(&answer).iter().any(|r| r["name"] == name),
            "{name} starved: {answer}"
        );
    }
    assert!(
        answer["trimmed"]["how"].is_string()
            || answer["note2"].as_str().is_some_and(|s| s.contains("find")),
        "missing continuation: {answer}"
    );
}

#[tokio::test]
async fn hidden_checkbox_label_has_an_actionable_ref() {
    let rig = page("<style>input{opacity:0;position:absolute}</style><label for=x>Visible choice</label><input id=x type=checkbox>").await;
    let answer = read(&rig, Some("Visible choice")).await;
    assert!(
        refs(&answer).iter().any(|r| r["visible"] != false),
        "visible label was missed: {answer}"
    );
    let label = refs(&answer)
        .iter()
        .find(|r| r["visible"] != false)
        .unwrap();
    let action = vw_browser::parse(&json!({"action":"click", "ref":label["ref"]})).unwrap();
    let result = rig
        .browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await;
    assert!(result.is_ok(), "label must be safely clickable: {result:?}");
    assert_eq!(
        rig.engine
            .eval("document.querySelector('input').checked")
            .await,
        true
    );
}

#[tokio::test]
async fn visible_toggle_label_reports_and_sets_the_hidden_controls_state() {
    let rig = page("<style>input{opacity:0;position:absolute}</style><label for=x>Visible choice</label><input id=x type=checkbox checked>").await;
    let answer = read(&rig, Some("Visible choice")).await;
    let label = refs(&answer).iter().find(|r| r["tag"] == "label").unwrap();
    assert_eq!(label["checked"], true, "{answer}");
    assert_eq!(label["role"], "checkbox", "{answer}");
    let action =
        vw_browser::parse(&json!({"action":"check", "ref":label["ref"],"checked":false})).unwrap();
    let result = rig
        .browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await
        .unwrap();
    assert_eq!(result["result"]["checked"], false, "{result}");
    assert_eq!(
        rig.engine
            .eval("document.querySelector('input').checked")
            .await,
        false
    );
}

#[tokio::test]
async fn disabled_hidden_toggle_keeps_its_visible_label_and_state() {
    let rig = page("<style>input{opacity:0;position:absolute}</style><label for=x>Required storage</label><input id=x type=checkbox checked disabled>").await;
    let answer = read(&rig, Some("Required storage")).await;
    let label = refs(&answer)
        .iter()
        .find(|r| r["tag"] == "label")
        .expect("visible disabled choice");
    assert_eq!(label["checked"], true, "{answer}");
    assert_eq!(label["disabled"], true, "{answer}");
}

#[tokio::test]
async fn selected_custom_values_are_found_without_exposing_secret_values() {
    let rig = page(include_str!("support/practice_form.html")).await;
    rig.engine.eval(r"for (const [id, text] of [['state','NCR'],['city','Delhi']]) {
        const input = document.querySelector('#'+id+' input'); input.disabled=false;
        const value = document.createElement('div'); value.className='custom__single-value'; value.textContent=text;
        input.parentElement.before(value);
    }").await;
    for value in ["NCR", "Delhi"] {
        let answer = read(&rig, Some(value)).await;
        assert!(
            refs(&answer).iter().any(|r| r["role"] == "combobox"),
            "{value}: {answer}"
        );
    }
    let secret = page("<label>Password<input type=password value='private-phrase'></label><label>Card number<input value='4111111111111111'></label><label>One-time code<input value='934821'></label><div><label>Card number</label><div><input value='5555555555554444'></div></div><div><span>One-time code</span><input value='318927'></div>").await;
    for value in ["private-phrase", "4111111111111111", "5555555555554444"] {
        let answer = read(&secret, Some(value)).await;
        assert!(
            refs(&answer).is_empty(),
            "find must not leak a secret through matching: {answer}"
        );
    }
    // One-time codes are ordinary fields: their values are read, and find matches a labelled one.
    let answer = read(&secret, Some("934821")).await;
    assert!(!refs(&answer).is_empty(), "one-time code: {answer}");
    let answer = read(&secret, None).await;
    assert!(
        refs(&answer).iter().any(|r| r["value"] == "934821"),
        "one-time code: {answer}"
    );
}

// Accessible name -> expected redaction kind ("" = readable).
const LANGUAGE_CASES: [(&str, &str); 30] = [
    ("Parole", "password"),
    ("Kartes numurs", "card"),
    ("CVV/CVC kods", "card"),
    ("Derīguma termiņš", "card"),
    ("Drošības kods", "card"),
    ("Vienreizējais kods", ""),
    ("Kods", ""),
    ("Vārds", ""),
    ("E-pasts", ""),
    ("Slaptažodis", "password"),
    ("Kortelės numeris", "card"),
    ("Patvirtinimo kodas", ""),
    ("Parool", "password"),
    ("Kaardi number", "card"),
    ("Kinnituskood", ""),
    ("Passwort", "password"),
    ("Kartennummer", "card"),
    ("Bestätigungscode", ""),
    ("Mot de passe", "password"),
    ("Numéro de carte", "card"),
    ("Code de vérification", ""),
    ("Contraseña", "password"),
    ("Número de tarjeta", "card"),
    ("Código de verificación", ""),
    ("Пароль", "password"),
    ("Номер карты", "card"),
    ("Код подтверждения", ""),
    ("Verification code", ""),
    ("OTP", ""),
    ("Enter 2FA code", ""),
];

// (field HTML, accessible name, expected redaction kind or "" for readable, value)
const EXTRA_FIELDS: [(&str, &str, &str, &str); 32] = [
    (
        "<input name='kartes_numurs' aria-label='Lauks A' value='zq-e1-x'>",
        "Lauks A",
        "card",
        "zq-e1-x",
    ),
    (
        "<input id='derigumaTermins' aria-label='Lauks B' value='zq-e2-x'>",
        "Lauks B",
        "card",
        "zq-e2-x",
    ),
    (
        "<input autocomplete='one-time-code' aria-label='Lauks C' value='zq-e3-x'>",
        "Lauks C",
        "",
        "zq-e3-x",
    ),
    (
        "<input name='otp' aria-label='Lauks D' value='zq-e4-x'>",
        "Lauks D",
        "",
        "zq-e4-x",
    ),
    (
        "<input type='password' aria-label='Code' value='zq-e5-x'>",
        "Code",
        "password",
        "zq-e5-x",
    ),
    // Review cases: a password, PIN or security-code label wins over a one-time-code declaration; birth/document/due
    // dates stay readable; invisible characters do not split a word; English card expiry phrases count.
    (
        "<input autocomplete='one-time-code' aria-label='Security code' value='zq-e6-x'>",
        "Security code",
        "card",
        "zq-e6-x",
    ),
    (
        "<input autocomplete='one-time-code' name='pin' aria-label='PIN' value='zq-e7-x'>",
        "PIN",
        "password",
        "zq-e7-x",
    ),
    (
        "<input type='password' autocomplete='one-time-code' aria-label='Kods 2' value='zq-e8-x'>",
        "Kods 2",
        "password",
        "zq-e8-x",
    ),
    (
        "<input placeholder='DD/MM/YY' aria-label='Dzimšanas datums' value='zq-f1-x'>",
        "Dzimšanas datums",
        "",
        "zq-f1-x",
    ),
    (
        "<input aria-label='Fecha de vencimiento' value='zq-f3-x'>",
        "Fecha de vencimiento",
        "",
        "zq-f3-x",
    ),
    (
        "<input aria-label='Kreditkarten&shy;nummer' value='zq-f4-x'>",
        "Kreditkartennummer",
        "card",
        "zq-f4-x",
    ),
    (
        "<input aria-label='Valid thru' value='zq-f5-x'>",
        "Valid thru",
        "card",
        "zq-f5-x",
    ),
    // One-time passwords are codes, not passwords; a declared code field named as a card number is a card.
    (
        "<input aria-label='One-time password' value='zq-g1-x'>",
        "One-time password",
        "",
        "zq-g1-x",
    ),
    (
        "<input aria-label='Vienreizējā parole' value='zq-g2-x'>",
        "Vienreizējā parole",
        "",
        "zq-g2-x",
    ),
    (
        "<input aria-label='Одноразовый пароль' value='zq-g3-x'>",
        "Одноразовый пароль",
        "",
        "zq-g3-x",
    ),
    (
        "<input autocomplete='one-time-code' aria-label='Password' value='zq-g4-x'>",
        "Password",
        "password",
        "zq-g4-x",
    ),
    (
        "<input autocomplete='one-time-code' aria-label='Card number' value='zq-g5-x'>",
        "Card number",
        "card",
        "zq-g5-x",
    ),
    (
        "<input aria-label='Password or one-time password' value='zq-g6-x'>",
        "Password or one-time password",
        "password",
        "zq-g6-x",
    ),
    // The three protected-field rules (built-in browser, Safari/Firefox page rule, Chrome/Edge row rule) agree on
    // these: a declared code named as a PIN, a password or a card code is protected; plain declared codes stay
    // readable; English keywords glued to Chinese, Korean or Japanese text count.
    (
        "<input autocomplete='one-time-code' aria-label='Card PIN' value='zq-p1-x'>",
        "Card PIN",
        "password",
        "zq-p1-x",
    ),
    (
        "<input autocomplete='one-time-code' name='password' aria-label='Lauks P2' value='zq-p2-x'>",
        "Lauks P2",
        "password",
        "zq-p2-x",
    ),
    (
        "<input aria-label='Kartes PIN' value='zq-p5-x'>",
        "Kartes PIN",
        "password",
        "zq-p5-x",
    ),
    (
        "<input aria-label='One-time PIN' value='zq-p6-x'>",
        "One-time PIN",
        "",
        "zq-p6-x",
    ),
    (
        "<input autocomplete='one-time-code' aria-label='Verification code' value='zq-p7-x'>",
        "Verification code",
        "",
        "zq-p7-x",
    ),
    (
        "<input autocomplete='one-time-code' aria-label='CVC2' value='zq-p8-x'>",
        "CVC2",
        "card",
        "zq-p8-x",
    ),
    (
        "<input aria-label='CVV码' value='zq-c1-x'>",
        "CVV码",
        "card",
        "zq-c1-x",
    ),
    (
        "<input aria-label='CVC번호' value='zq-c2-x'>",
        "CVC번호",
        "card",
        "zq-c2-x",
    ),
    (
        "<input aria-label='CVVコード' value='zq-c3-x'>",
        "CVVコード",
        "card",
        "zq-c3-x",
    ),
    (
        "<input aria-label='信用卡Card number' value='zq-c4-x'>",
        "信用卡Card number",
        "card",
        "zq-c4-x",
    ),
    (
        "<input type='text' aria-label='请输入Password' value='zq-c5-x'>",
        "请输入Password",
        "password",
        "zq-c5-x",
    ),
    (
        "<input aria-label='CVV 码' value='zq-c6-x'>",
        "CVV 码",
        "card",
        "zq-c6-x",
    ),
    (
        "<input aria-label='验证码Code' value='zq-c7-x'>",
        "验证码Code",
        "",
        "zq-c7-x",
    ),
    (
        "<input autocomplete='one-time-code' aria-label='Kods 3' value='zq-p9-x'>",
        "Kods 3",
        "",
        "zq-p9-x",
    ),
];

// PREDICT (before the rule change): the password and card-number fields labelled in Latvian, Lithuanian, Estonian,
// German, French, Spanish and Russian carry their value (the label words are English only), and the one-time-code
// fields ("Verification code", "OTP", autocomplete=one-time-code, name=otp) are redacted as "otp". A password-type
// field labelled "Code" is already redacted as "password"; "Vārds", "E-pasts" and "Kods" are already readable.
// PREDICT (declared codes, before password/PIN/security-code labels won over the declaration): "Security code",
// "PIN", "Password", "Card PIN" and the name=password field declared one-time-code are readable; the undeclared
// "Kartes PIN", the CJK-glued labels, "One-time PIN", the declared "CVC2" and the plain declared codes already behave
// as expected (the English patterns use ASCII word boundaries).
// PREDICT (review cases, against the previous commit): "Security code" and "PIN" declared one-time-code are redacted
// (card, password); "Fecha de vencimiento" is redacted as card; "Kreditkarten<soft hyphen>nummer" and "Valid thru" are
// readable. "Vienreizējā parole", "Одноразовый пароль" and the declared code field labelled "Password" are redacted as
// password, and so is "One-time password"; "Password or one-time password" already is.
#[tokio::test]
async fn labels_in_other_languages_protect_passwords_and_cards_while_codes_stay_readable() {
    // (field HTML, accessible name, expected redaction kind or "" for readable, value)
    let mut fields: Vec<(String, String, &str, String)> = LANGUAGE_CASES
        .iter()
        .enumerate()
        .map(|(i, (label, kind))| {
            let value = format!("zq-v{i:02}-x");
            (
                format!("<input aria-label='{label}' value='{value}'>"),
                (*label).to_owned(),
                *kind,
                value,
            )
        })
        .collect();
    for (field, label, kind, value) in EXTRA_FIELDS {
        fields.push((field.to_owned(), label.to_owned(), kind, value.to_owned()));
    }
    let mut wrong = Vec::new();
    // A few fields per page, so the read's size budget never drops one.
    for group in fields.chunks(10) {
        let html: String = group.iter().map(|f| f.0.as_str()).collect();
        let rig = page(&html).await;
        let answer = read(&rig, None).await;
        let shown = answer.to_string();
        for (_, label, kind, value) in group {
            let row = refs(&answer).iter().find(|r| r["name"] == label.as_str());
            let ok = row.is_some_and(|r| {
                if kind.is_empty() {
                    r["value"] == value.as_str() && r["redacted"].is_null()
                } else {
                    r["redacted"] == *kind
                        && r["value"].is_null()
                        && !shown.contains(value.as_str())
                }
            });
            if !ok {
                wrong.push(format!("{label}: want {kind:?}, got {row:?}"));
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

// PREDICT (before the card-form context): "Fecha de vencimiento" and "Ablaufdatum" in a form with a card number are
// readable, and so is a field declared a one-time code but named "Security code" in that form. A bare declared "Code"
// in the card form and "Gültig bis" in a form without a card stay readable either way.
#[tokio::test]
async fn expiry_words_and_declared_security_codes_count_only_in_a_card_form() {
    let rig = page("<form><input aria-label='Kartes numurs' value='zq-h1-x'><input aria-label='Fecha de vencimiento' value='zq-h2-x'><input aria-label='Ablaufdatum' value='zq-h3-x'><input autocomplete='one-time-code' aria-label='Security code' value='zq-h4-x'><input autocomplete='one-time-code' aria-label='Code' value='zq-h5-x'></form><form><input aria-label='Gültig bis' value='zq-h6-x'></form>").await;
    let answer = read(&rig, None).await;
    let shown = answer.to_string();
    let row = |name: &str| {
        refs(&answer)
            .iter()
            .find(|r| r["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("{name}: {answer}"))
    };
    for (name, value) in [
        ("Kartes numurs", "zq-h1-x"),
        ("Fecha de vencimiento", "zq-h2-x"),
        ("Ablaufdatum", "zq-h3-x"),
        ("Security code", "zq-h4-x"),
    ] {
        assert_eq!(row(name)["redacted"], "card", "{name}: {}", row(name));
        assert!(!shown.contains(value), "{name} leaked");
    }
    assert_eq!(row("Code")["value"], "zq-h5-x");
    assert_eq!(row("Gültig bis")["value"], "zq-h6-x");
}

// A plain-text field named only by its visible label (label[for], a wrapping <label>, aria-labelledby): the same
// password/PIN words as for attribute labels, with the same precedence over autocomplete="one-time-code".
// (field HTML, accessible name, expected redaction kind or "" for readable, value)
const VISIBLE_LABEL_FIELDS: [(&str, &str, &str, &str); 14] = [
    (
        "<label for='v1'>PIN</label><input id='v1' value='zq-v1-x'>",
        "PIN",
        "password",
        "zq-v1-x",
    ),
    (
        "<label>Card PIN <input autocomplete='one-time-code' value='zq-v2-x'></label>",
        "Card PIN",
        "password",
        "zq-v2-x",
    ),
    (
        "<span id='v3n'>Account password</span><input aria-labelledby='v3n' autocomplete='one-time-code' value='zq-v3-x'>",
        "Account password",
        "password",
        "zq-v3-x",
    ),
    (
        "<label for='v4'>Password or one-time password</label><input id='v4' autocomplete='one-time-code' value='zq-v4-x'>",
        "Password or one-time password",
        "password",
        "zq-v4-x",
    ),
    (
        "<label for='v5'>请输入Password</label><input id='v5' value='zq-v5-x'>",
        "请输入Password",
        "password",
        "zq-v5-x",
    ),
    (
        "<label for='v6'>Passwd</label><input id='v6' value='zq-v6-x'>",
        "Passwd",
        "password",
        "zq-v6-x",
    ),
    (
        "<label for='v7'>Parole</label><input id='v7' autocomplete='one-time-code' value='zq-v7-x'>",
        "Parole",
        "password",
        "zq-v7-x",
    ),
    (
        "<label for='v8'>Security code</label><input id='v8' autocomplete='one-time-code' value='zq-v8-x'>",
        "Security code",
        "card",
        "zq-v8-x",
    ),
    (
        "<label for='v9'>One-time password</label><input id='v9' value='zq-v9-x'>",
        "One-time password",
        "",
        "zq-v9-x",
    ),
    (
        "<label for='w1'>One-time PIN</label><input id='w1' autocomplete='one-time-code' value='zq-w1-x'>",
        "One-time PIN",
        "",
        "zq-w1-x",
    ),
    (
        "<label for='w2'>SMS code</label><input id='w2' autocomplete='one-time-code' value='zq-w2-x'>",
        "SMS code",
        "",
        "zq-w2-x",
    ),
    (
        "<label for='w3'>Passcode</label><input id='w3' value='zq-w3-x'>",
        "Passcode",
        "",
        "zq-w3-x",
    ),
    (
        "<label for='w4'>Pinterest user</label><input id='w4' value='zq-w4-x'>",
        "Pinterest user",
        "",
        "zq-w4-x",
    ),
    (
        "<label for='w5'>Vārds</label><input id='w5' value='zq-w5-x'>",
        "Vārds",
        "",
        "zq-w5-x",
    ),
];

// PREDICT (before visible labels carried the English password/PIN words): "PIN", "Card PIN", "Account password",
// "请输入Password" and "Passwd" named only by a visible label are readable (the English words were looked for in the
// name, id, aria-label, placeholder and test id only). "Password or one-time password", "Parole" and "Security code"
// are already protected (the declared one-time code no longer lowers protection; "parole" and the card phrases were
// already matched on label text); the code labels, "Passcode", "Pinterest user" and "Vārds" are already readable.
// Red run: as predicted, except "Password or one-time password" was readable too (no English password word was
// looked for in label text at all).
#[tokio::test]
async fn visible_labels_protect_passwords_and_pins_like_attribute_labels() {
    let mut wrong = Vec::new();
    for group in VISIBLE_LABEL_FIELDS.chunks(7) {
        let html = group.iter().fold(String::new(), |mut html, f| {
            let _ = write!(html, "<p>{}</p>", f.0);
            html
        });
        let rig = page(&html).await;
        let answer = read(&rig, None).await;
        let shown = answer.to_string();
        for (_, label, kind, value) in group {
            let row = refs(&answer).iter().find(|r| r["name"] == *label);
            let ok = row.is_some_and(|r| {
                if kind.is_empty() {
                    r["value"] == *value && r["redacted"].is_null()
                } else {
                    r["redacted"] == *kind && r["value"].is_null() && !shown.contains(value)
                }
            });
            if !ok {
                wrong.push(format!("{label}: want {kind:?}, got {row:?}"));
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

// PREDICT (before visible labels carried the English password/PIN words): of this page's fields, the four named by a
// visible <label for> "Password" (declared one-time code and plain), "Card PIN" (declared one-time code) and
// "密码Password" are read in clear; the aria-label twins are already withheld and both "Verification code" fields are
// already readable.
#[tokio::test]
async fn visible_label_and_aria_label_twins_are_withheld_alike() {
    let rig = page(concat!(
        "<form>",
        "<p><label for=\"a1\">Password</label> <input id=\"a1\" name=\"a1\" type=\"text\" autocomplete=\"one-time-code\" value=\"LblPw-11001\"></p>",
        "<p><label for=\"a2\">Card PIN</label> <input id=\"a2\" name=\"a2\" type=\"text\" inputmode=\"numeric\" autocomplete=\"one-time-code\" value=\"LblPin-22002\"></p>",
        "<p><label for=\"a3\">Verification code</label> <input id=\"a3\" name=\"a3\" type=\"text\" inputmode=\"numeric\" autocomplete=\"one-time-code\" value=\"LblOtp-33003\"></p>",
        "<p><label for=\"a4\">密码Password</label> <input id=\"a4\" name=\"a4\" type=\"text\" value=\"LblCjk-44004\"></p>",
        "<p><label for=\"a5\">Password</label> <input id=\"a5\" name=\"a5\" type=\"text\" value=\"LblPlain-55005\"></p>",
        "<p>B1 <input id=\"b1\" name=\"b1\" type=\"text\" aria-label=\"Password\" autocomplete=\"one-time-code\" value=\"AriaPw-61001\"></p>",
        "<p>B2 <input id=\"b2\" name=\"b2\" type=\"text\" aria-label=\"Card PIN\" autocomplete=\"one-time-code\" value=\"AriaPin-62002\"></p>",
        "<p>B3 <input id=\"b3\" name=\"b3\" type=\"text\" aria-label=\"Verification code\" autocomplete=\"one-time-code\" value=\"AriaOtp-63003\"></p>",
        "<p>B4 <input id=\"b4\" name=\"b4\" type=\"text\" aria-label=\"密码Password\" value=\"AriaCjk-64004\"></p>",
        "</form>",
    ))
    .await;
    let answer = read(&rig, None).await;
    let shown = answer.to_string();
    let rows = |name: &str| -> Vec<Value> {
        refs(&answer)
            .iter()
            .filter(|r| r["name"] == name)
            .cloned()
            .collect()
    };
    let mut wrong = Vec::new();
    for (name, count) in [("Password", 3), ("Card PIN", 2), ("密码Password", 2)] {
        let found = rows(name);
        if found.len() != count
            || found
                .iter()
                .any(|r| r["redacted"] != "password" || !r["value"].is_null())
        {
            wrong.push(format!("{name}: want {count} withheld, got {found:?}"));
        }
    }
    for value in [
        "LblPw-11001",
        "LblPin-22002",
        "LblCjk-44004",
        "LblPlain-55005",
        "AriaPw-61001",
        "AriaPin-62002",
        "AriaCjk-64004",
    ] {
        if shown.contains(value) {
            wrong.push(format!("{value} leaked"));
        }
    }
    let codes = rows("Verification code");
    for value in ["LblOtp-33003", "AriaOtp-63003"] {
        if !codes.iter().any(|r| r["value"] == value) {
            wrong.push(format!("{value} not readable: {codes:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
