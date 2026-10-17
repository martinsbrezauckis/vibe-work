//! The page server's answer policies (Handbook Format 1.2 §6.4): the Content-Security-Policy per kind of
//! answer ([`Policy`]), the visual check route's name and the error-recording prelude it puts into a prototype page.
//! Split out of `server` (the serving rules are documented there).
//!
//! Depends on: nothing. Called by: `server`, tests.

/// The start of the visual check's route segment (`.vw-check-<token>`; a hidden segment, so no handbook file can live
/// there and the viewer's routes refuse it).
pub const CHECK_PREFIX: &str = ".vw-check-";

/// The script the check route puts at the start of a prototype page: records uncaught errors, unhandled rejections,
/// `console.error` calls and failed resource loads in `window.__vwCheckErrors` (at most 20, 300 characters each) for
/// the visual check to read. Never served by the viewer's routes.
pub const ERROR_PRELUDE: &str = "<script>(function(){var e=window.__vwCheckErrors=[];function p(m){if(e.length<20)\
e.push(String(m).slice(0,300))}addEventListener('error',function(v){var t=v.target;if(t&&t!==window&&(t.src||t.href))\
{p('failed to load '+(t.src||t.href));return}p((v.message||'error')+(v.filename?' ('+v.filename.split('/').pop()+':'\
+v.lineno+')':''))},true);addEventListener('unhandledrejection',function(v){var r=v.reason;p('unhandled rejection: '+\
(r&&r.message?r.message:r))});var c=console.error;console.error=function(){p('console.error: '+Array.prototype.map.\
call(arguments,String).join(' '));return c.apply(console,arguments)}})();</script>";

/// What an answer may do (the CSP it gets).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// No script at all (handbook pages of every kind but `prototype`, Markdown, the live index, errors).
    Document,
    /// A prototype page in the viewer: its own scripts (when `scripts`), sandboxed, no network, no frames.
    Prototype {
        /// The project lets prototype pages run their script.
        scripts: bool,
    },
    /// A prototype page on the check route: as [`Policy::Prototype`] but not sandboxed (see the module header).
    CheckPrototype {
        /// The project lets prototype pages run their script.
        scripts: bool,
    },
    /// The visual check's own empty document: no script of its own (the engine's script measures), frames of this
    /// server only.
    CheckHost,
    /// App-owned parent of a sandboxed prototype with inline controls: no page scripts, one local frame.
    InlineHost,
    /// The File tab's servers: [`csp`] with `scripts`.
    Files {
        /// Inline and same-origin scripts may run.
        scripts: bool,
    },
}

impl Policy {
    /// The Content-Security-Policy header value for an answer of the server at `origin`.
    #[must_use]
    pub fn header(self, origin: &str) -> String {
        let prototype = |scripts: bool, sandbox: bool| {
            // `'self'` is listed with the explicit origin: a sandboxed document has an opaque origin, and the
            // explicit origin keeps its own files loading on every engine.
            let script = if scripts {
                format!("script-src 'self' {origin} 'unsafe-inline'")
            } else {
                "script-src 'none'".to_owned()
            };
            format!(
                "default-src 'none'; {script}; style-src 'self' {origin} 'unsafe-inline'; img-src 'self' {origin} \
                 data: blob:; font-src 'self' {origin} data:; media-src 'self' {origin} data: blob:; connect-src \
                 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; manifest-src 'none'; form-action \
                 'none'; base-uri 'none'; object-src 'none'{}",
                if sandbox {
                    "; sandbox allow-scripts"
                } else {
                    ""
                }
            )
        };
        match self {
            Self::Document | Self::CheckHost | Self::InlineHost => format!(
                "default-src 'self'; script-src 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
                 font-src 'self' data:; media-src 'self'; connect-src 'none'; frame-src {}; child-src {0}; \
                 worker-src 'none'; form-action 'none'; base-uri 'none'; object-src 'none'",
                if matches!(self, Self::CheckHost | Self::InlineHost) {
                    "'self'"
                } else {
                    "'none'"
                }
            ),
            Self::Prototype { scripts } => prototype(scripts, true),
            Self::CheckPrototype { scripts } => prototype(scripts, false),
            Self::Files { scripts } => csp(scripts),
        }
    }
}

/// The Content-Security-Policy of the File tab's answers. `scripts`: inline and same-origin scripts may
/// run; otherwise none. Handbook servers use [`Policy`] per page kind.
#[must_use]
pub fn csp(scripts: bool) -> String {
    let script = if scripts {
        "script-src 'self' 'unsafe-inline'"
    } else {
        "script-src 'none'"
    };
    format!(
        "default-src 'self'; {script}; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' \
         data:; media-src 'self'; connect-src 'self'; frame-src 'self'; worker-src 'none'; form-action 'none'; \
         base-uri 'none'; object-src 'none'"
    )
}
