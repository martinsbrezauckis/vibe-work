//! The command text a Windows shell call runs, for the Git and process guards: `cmd /c|/k|/r` and PowerShell.
//! PowerShell's command position is found the way the program itself reads its parameters (Windows PowerShell 5.1
//! `powershell` and PowerShell 7 `pwsh` differ); a call whose command text cannot be read (standard input, a server
//! mode, an undecodable `-EncodedCommand`, a `pwsh` spelling not known here) is reported so the guards ask instead of
//! allowing it. Depends on nothing else in the crate. Called by `gitparse` and `killparse`.

/// What one Windows shell call runs.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ShellText {
    /// Command texts the call may run, each read again as a command line.
    pub(crate) scripts: Vec<String>,
    /// Why the call runs command text that cannot be read here; the guards then ask the user.
    pub(crate) unreadable: Option<&'static str>,
}

/// The command texts of `cmd /c|/k|/r` (also `//c` from Git Bash and `/cTEXT` without a space, `^` escapes removed)
/// and of PowerShell. PowerShell is read exactly ([`exact`]) and, as a second reading, with its `-Command` and
/// `-EncodedCommand` wherever they appear (and, for `powershell`, its first plain-word positions): reading more texts
/// can only add asks, never remove one.
pub(crate) fn command_text(name: &str, args: &[&str]) -> ShellText {
    let edition = match name {
        "cmd" => {
            return ShellText {
                scripts: cmd_script(args).into_iter().collect(),
                unreadable: None,
            };
        }
        "powershell" => Edition::Windows,
        "pwsh" => Edition::Seven,
        _ => return ShellText::default(),
    };
    let mut text = exact(edition, args);
    let loose = wherever(args);
    text.unreadable = text.unreadable.or(loose.unreadable);
    let positions = (edition == Edition::Windows).then(|| positional(args));
    for script in loose
        .scripts
        .into_iter()
        .chain(positions.into_iter().flatten())
    {
        if !text.scripts.contains(&script) {
            text.scripts.push(script);
        }
    }
    text
}

/// Reasons a PowerShell call's command text cannot be read.
pub(crate) const FROM_STDIN: &str = "PowerShell reads its commands from standard input";
/// See [`FROM_STDIN`].
pub(crate) const SERVER_MODE: &str =
    "PowerShell runs as a server that takes its commands from another program";
/// See [`FROM_STDIN`].
pub(crate) const UNDECODED: &str = "PowerShell's -EncodedCommand text could not be decoded";
/// See [`FROM_STDIN`].
pub(crate) const UNKNOWN_SPELLING: &str =
    "a PowerShell 7 parameter spelling Vibe Work does not know";

fn cmd_script(args: &[&str]) -> Option<String> {
    for (at, arg) in args.iter().enumerate() {
        let Some(switch) = arg.strip_prefix("//").or_else(|| arg.strip_prefix('/')) else {
            continue;
        };
        let mut chars = switch.chars();
        if chars
            .next()
            .is_some_and(|c| matches!(c.to_ascii_lowercase(), 'c' | 'k' | 'r'))
        {
            let mut words: Vec<&str> = Vec::new();
            let attached = chars.as_str();
            if !attached.is_empty() {
                words.push(attached);
            }
            words.extend_from_slice(&args[at + 1..]);
            let script = words.join(" ").replace('^', "");
            return (!script.trim().is_empty()).then_some(script);
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edition {
    /// Windows PowerShell 5.1, `powershell.exe`: the first plain word or unknown spelling starts the command.
    Windows,
    /// PowerShell 7, `pwsh`: the first plain word is a script file; an unknown spelling is an error.
    Seven,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Param {
    /// Takes no value.
    Switch,
    /// Takes the next argument as its value.
    Value,
    /// The rest of the line is the command (`-` = standard input).
    Command,
    /// The next argument is base64 of UTF-16LE command text.
    Encoded,
    /// The next argument is a script file (`-` = standard input); the rest are its arguments, not command text.
    File,
    /// Takes its commands from another program.
    Server,
    /// Prints help or the version; nothing runs.
    Help,
}

/// Windows PowerShell 5.1's parameters in the order `powershell.exe` tries them: a spelling selects the first entry it
/// is a prefix of and is at least as long as that entry's shortest form. Checked spelling by spelling against
/// `powershell.exe` 5.1 (which take a value, which abbreviations it accepts; every spelling it does not know starts
/// the command).
const WINDOWS_POWERSHELL: [(&str, &str, Param); 30] = [
    ("help", "h", Param::Help),
    ("?", "?", Param::Help),
    ("noexit", "noe", Param::Switch),
    ("importsystemmodules", "imp", Param::Switch),
    ("noprofile", "nop", Param::Switch),
    ("nologo", "nol", Param::Switch),
    ("noninteractive", "noni", Param::Switch),
    ("socketservermode", "so", Param::Server),
    ("servermode", "s", Param::Server),
    ("namedpipeservermode", "nam", Param::Server),
    ("configurationname", "config", Param::Value),
    ("command", "c", Param::Command),
    ("windowstyle", "w", Param::Value),
    ("workingdirectory", "wo", Param::Value),
    ("wd", "wd", Param::Value),
    ("file", "f", Param::File),
    ("outputformat", "o", Param::Value),
    ("of", "o", Param::Value),
    ("inputformat", "i", Param::Value),
    ("if", "if", Param::Value),
    ("executionpolicy", "ex", Param::Value),
    ("ep", "ep", Param::Value),
    ("encodedcommand", "e", Param::Encoded),
    ("ec", "e", Param::Encoded),
    ("encodedarguments", "encodeda", Param::Value),
    ("ea", "ea", Param::Value),
    ("sta", "s", Param::Switch),
    ("mta", "mta", Param::Switch),
    ("psconsolefile", "p", Param::Value),
    ("version", "v", Param::Value),
];

/// PowerShell 7's parameters, read the same way; checked spelling by spelling against `pwsh` 7.6.
const POWERSHELL_SEVEN: [(&str, &str, Param); 35] = [
    ("help", "h", Param::Help),
    ("?", "?", Param::Help),
    ("version", "v", Param::Help),
    ("command", "c", Param::Command),
    ("commandwithargs", "commandwithargs", Param::Command),
    ("cwa", "cwa", Param::Command),
    ("configurationname", "config", Param::Value),
    ("configurationfile", "configurationfile", Param::Value),
    ("custompipename", "cus", Param::Value),
    ("encodedcommand", "e", Param::Encoded),
    ("ec", "ec", Param::Encoded),
    ("encodedarguments", "encodeda", Param::Value),
    ("ea", "ea", Param::Value),
    ("executionpolicy", "ex", Param::Value),
    ("ep", "ep", Param::Value),
    ("file", "f", Param::File),
    ("inputformat", "inp", Param::Value),
    ("if", "if", Param::Value),
    ("interactive", "i", Param::Switch),
    ("login", "l", Param::Switch),
    ("mta", "mta", Param::Switch),
    ("sta", "sta", Param::Switch),
    ("noexit", "noe", Param::Switch),
    ("nologo", "nol", Param::Switch),
    ("noprofile", "nop", Param::Switch),
    ("noprofileloadtime", "noprofileloadtime", Param::Switch),
    ("noninteractive", "noni", Param::Switch),
    ("outputformat", "o", Param::Value),
    ("of", "of", Param::Value),
    ("settingsfile", "settings", Param::Value),
    ("sshservermode", "sshs", Param::Server),
    ("servermode", "s", Param::Server),
    ("socketservermode", "so", Param::Server),
    ("namedpipeservermode", "nam", Param::Server),
    ("windowstyle", "w", Param::Value),
];

/// PowerShell 7 spellings the main table leaves out to stay small; same matching.
const POWERSHELL_SEVEN_MORE: [(&str, &str, Param); 3] = [
    ("workingdirectory", "wo", Param::Value),
    ("wd", "wd", Param::Value),
    (
        "removeworkingdirectorytrailingcharacter",
        "removeworkingdirectorytrailingcharacter",
        Param::Switch,
    ),
];

/// How one argument reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Word {
    /// No parameter spelling.
    Plain,
    /// A parameter spelling the edition does not know.
    Unknown,
    /// A dash alone: standard input.
    Dash,
    Param(Param),
}

const DASHES: [char; 4] = ['-', '\u{2013}', '\u{2014}', '\u{2015}'];

/// Both editions take `-`, `/` and the Unicode dashes as a parameter prefix. PowerShell 7 also drops a second dash
/// (`--command`); for Windows PowerShell `--name` is no parameter and starts the command like a plain word.
fn word(edition: Edition, arg: &str) -> Word {
    let mut chars = arg.trim().chars();
    let Some(first) = chars.next() else {
        return Word::Plain;
    };
    if first != '/' && !DASHES.contains(&first) {
        return Word::Plain;
    }
    let mut key = chars.as_str().to_lowercase();
    if key.is_empty() {
        return if first == '-' {
            Word::Dash
        } else {
            Word::Unknown
        };
    }
    if key.starts_with(DASHES) {
        if edition == Edition::Windows {
            return Word::Plain;
        }
        key = key.chars().skip(1).collect();
    }
    let table: &[(&str, &str, Param)] = match edition {
        Edition::Windows => &WINDOWS_POWERSHELL,
        Edition::Seven => &POWERSHELL_SEVEN,
    };
    let more: &[(&str, &str, Param)] = match edition {
        Edition::Windows => &[],
        Edition::Seven => &POWERSHELL_SEVEN_MORE,
    };
    table
        .iter()
        .chain(more)
        .find(|(name, shortest, _)| {
            name.starts_with(key.as_str()) && key.chars().count() >= shortest.chars().count()
        })
        .map_or(Word::Unknown, |(_, _, param)| Word::Param(*param))
}

/// Reads PowerShell's arguments as the program does: parameters (with their values) until the command, a script file,
/// help or a server mode. A call that names none of them takes its commands from standard input (also when nothing
/// is piped in: an interactive session is still no readable command text).
fn exact(edition: Edition, args: &[&str]) -> ShellText {
    let mut text = ShellText::default();
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        if arg.trim().is_empty() {
            at += 1;
            continue;
        }
        let param = match (edition, word(edition, arg)) {
            (_, Word::Param(param)) => param,
            (_, Word::Dash) => {
                text.unreadable = Some(FROM_STDIN);
                return text;
            }
            (Edition::Windows, Word::Plain | Word::Unknown) => {
                text.scripts.push(args[at..].join(" "));
                return text;
            }
            // The first plain word is the script file `pwsh` runs; its text is not read here.
            (Edition::Seven, Word::Plain) => return text,
            (Edition::Seven, Word::Unknown) => {
                text.unreadable = Some(UNKNOWN_SPELLING);
                return text;
            }
        };
        match param {
            Param::Switch => {}
            Param::Value => at += 1,
            Param::Help => return text,
            Param::Server => {
                text.unreadable = Some(SERVER_MODE);
                return text;
            }
            Param::Command => {
                let rest = &args[at + 1..];
                if rest.first().is_none_or(|word| word.trim() == "-") {
                    text.unreadable = Some(FROM_STDIN);
                } else {
                    text.scripts.push(rest.join(" "));
                }
                return text;
            }
            Param::File => {
                if args.get(at + 1).is_none_or(|word| word.trim() == "-") {
                    text.unreadable = Some(FROM_STDIN);
                }
                return text;
            }
            Param::Encoded => {
                at += 1;
                match args.get(at).map(|value| decode(value)) {
                    Some(Some(script)) => {
                        text.scripts.push(script);
                        return text;
                    }
                    Some(None) => {
                        text.unreadable = Some(UNDECODED);
                        return text;
                    }
                    None => {}
                }
            }
        }
        at += 1;
    }
    text.unreadable = Some(FROM_STDIN);
    text
}

/// `-Command` (any prefix, also `-CommandWithArgs`/`-cwa`) and `-EncodedCommand` wherever they appear: the second
/// reading of both editions.
fn wherever(args: &[&str]) -> ShellText {
    let mut text = ShellText::default();
    for (at, arg) in args.iter().enumerate() {
        let Some(flag) = loose_flag(arg) else {
            continue;
        };
        if "command".starts_with(flag.as_str())
            || "commandwithargs".starts_with(flag.as_str()) && flag.len() > "command".len()
            || flag == "cwa"
        {
            let rest = &args[at + 1..];
            if rest.first().is_some_and(|word| word.trim() == "-") {
                text.unreadable = Some(FROM_STDIN);
            } else if !rest.is_empty() {
                text.scripts.push(rest.join(" "));
            }
            break;
        }
        if flag == "e"
            || flag == "ec"
            || (flag.starts_with("en") && "encodedcommand".starts_with(flag.as_str()))
        {
            match args.get(at + 1).map(|value| decode(value)) {
                Some(Some(script)) => text.scripts.push(script),
                Some(None) => text.unreadable = Some(UNDECODED),
                None => {}
            }
            break;
        }
    }
    text
}

/// A parameter spelling for the second reading: one or two leading dashes (any of [`DASHES`]) or a slash.
fn loose_flag(arg: &str) -> Option<String> {
    let lower = arg.trim().to_lowercase();
    let flag = lower
        .strip_prefix(DASHES)
        .map(|rest| rest.strip_prefix(DASHES).unwrap_or(rest))
        .or_else(|| lower.strip_prefix('/'))?;
    (!flag.is_empty()).then(|| flag.to_owned())
}

/// At most this many plain-word positions are read by the second reading; the exact reading has no limit.
const MAX_POSITIONAL_CANDIDATES: usize = 8;

fn positional<'a>(args: &'a [&'a str]) -> impl Iterator<Item = String> + 'a {
    (0..args.len())
        .filter(|&at| loose_flag(args[at]).is_none())
        .take(MAX_POSITIONAL_CANDIDATES)
        .map(|at| args[at..].join(" "))
}

/// `-EncodedCommand` is base64 of UTF-16LE text; `None` when it is not.
fn decode(text: &str) -> Option<String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text.trim().trim_matches(['"', '\'']))
        .ok()?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    Some(String::from_utf16_lossy(&units))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(edition: Edition, line: &str) -> ShellText {
        exact(edition, &line.split(' ').collect::<Vec<_>>())
    }

    #[test]
    fn windows_spellings_select_the_parameter_powershell_selects() {
        let param = |spelling| match word(Edition::Windows, spelling) {
            Word::Param(param) => Some(param),
            _ => None,
        };
        for (spelling, expected) in [
            ("-c", Some(Param::Command)),
            ("-com", Some(Param::Command)),
            ("/command", Some(Param::Command)),
            ("\u{2013}c", Some(Param::Command)),
            ("-config", Some(Param::Value)),
            ("-con", None),
            ("-conf", None),
            ("-w", Some(Param::Value)),
            ("-wi", Some(Param::Value)),
            ("-wo", Some(Param::Value)),
            ("-wd", Some(Param::Value)),
            ("-e", Some(Param::Encoded)),
            ("-ec", Some(Param::Encoded)),
            ("-ex", Some(Param::Value)),
            ("-encodeda", Some(Param::Value)),
            ("-s", Some(Param::Server)),
            ("-st", Some(Param::Switch)),
            ("-so", Some(Param::Server)),
            ("-nam", Some(Param::Server)),
            ("-na", None),
            ("-n", None),
            ("-non", None),
            ("-noni", Some(Param::Switch)),
            ("-nopr", Some(Param::Switch)),
            ("-im", None),
            ("-m", None),
            ("-mta", Some(Param::Switch)),
            ("-i", Some(Param::Value)),
            ("-p", Some(Param::Value)),
            ("-v", Some(Param::Value)),
            ("-f", Some(Param::File)),
            ("-?", Some(Param::Help)),
            ("-show", None),
            ("-zzz", None),
            ("--command", None),
        ] {
            assert_eq!(param(spelling), expected, "{spelling}");
        }
        assert_eq!(word(Edition::Windows, "--command"), Word::Plain);
        assert_eq!(word(Edition::Windows, "git"), Word::Plain);
        assert_eq!(word(Edition::Windows, "-"), Word::Dash);
        assert_eq!(word(Edition::Windows, "/"), Word::Unknown);
    }

    #[test]
    fn seven_spellings_select_the_parameter_pwsh_selects() {
        for (spelling, expected) in [
            ("-c", Word::Param(Param::Command)),
            ("--command", Word::Param(Param::Command)),
            ("\u{2013}Command", Word::Param(Param::Command)),
            ("-cwa", Word::Param(Param::Command)),
            ("-CommandWithArgs", Word::Param(Param::Command)),
            ("-configf", Word::Unknown),
            ("-cus", Word::Param(Param::Value)),
            ("-e", Word::Param(Param::Encoded)),
            ("-ea", Word::Param(Param::Value)),
            ("-ex", Word::Param(Param::Value)),
            ("-i", Word::Param(Param::Switch)),
            ("-in", Word::Param(Param::Switch)),
            ("-inp", Word::Param(Param::Value)),
            ("-l", Word::Param(Param::Switch)),
            ("-m", Word::Unknown),
            ("-st", Word::Unknown),
            ("-sta", Word::Param(Param::Switch)),
            ("-s", Word::Param(Param::Server)),
            ("-sshs", Word::Param(Param::Server)),
            ("-o", Word::Param(Param::Value)),
            ("-settings", Word::Param(Param::Value)),
            ("-v", Word::Param(Param::Help)),
            ("-wd", Word::Param(Param::Value)),
            ("-n", Word::Unknown),
            ("-p", Word::Unknown),
            ("script.ps1", Word::Plain),
            ("-", Word::Dash),
        ] {
            assert_eq!(word(Edition::Seven, spelling), expected, "{spelling}");
        }
    }

    #[test]
    fn the_command_starts_where_powershell_starts_it() {
        let windows = |line| read(Edition::Windows, line).scripts;
        assert_eq!(windows("-w Hidden -ep Bypass git status"), ["git status"]);
        assert_eq!(windows("-zzz git status"), ["-zzz git status"]);
        assert_eq!(windows("--command git status"), ["--command git status"]);
        assert_eq!(windows("-NoProfile -c git status"), ["git status"]);
        assert_eq!(windows("-f run.ps1 git status"), Vec::<String>::new());
        assert_eq!(windows("-? git status"), Vec::<String>::new());
        let seven = |line| read(Edition::Seven, line).scripts;
        assert_eq!(seven("-w Hidden --command git status"), ["git status"]);
        assert_eq!(seven("-cwa git status"), ["git status"]);
        assert_eq!(seven("run.ps1 -Command git status"), Vec::<String>::new());
    }

    #[test]
    fn unreadable_command_text_is_reported() {
        for edition in [Edition::Windows, Edition::Seven] {
            for line in [
                "-NoProfile -Command -",
                "-",
                "-File -",
                "-NoProfile",
                "-w Hidden",
                "-c",
            ] {
                assert_eq!(
                    read(edition, line).unreadable,
                    Some(FROM_STDIN),
                    "{edition:?} {line}"
                );
            }
            assert_eq!(read(edition, "-s").unreadable, Some(SERVER_MODE));
            assert_eq!(
                read(edition, "-NamedPipeServerMode").unreadable,
                Some(SERVER_MODE)
            );
            assert_eq!(read(edition, "-enc not*base64").unreadable, Some(UNDECODED));
            assert_eq!(
                read(edition, "-NoProfile -Command git status").unreadable,
                None
            );
            assert_eq!(read(edition, "-?").unreadable, None);
        }
        assert_eq!(
            read(Edition::Seven, "-zzz").unreadable,
            Some(UNKNOWN_SPELLING)
        );
        assert_eq!(read(Edition::Seven, "build.ps1").unreadable, None);
        assert_eq!(wherever(&["-Command", "-"]).unreadable, Some(FROM_STDIN));
        assert_eq!(
            wherever(&["\u{2013}Command", "git status"]).scripts,
            ["git status"]
        );
        assert_eq!(wherever(&["-cwa", "git status"]).scripts, ["git status"]);
    }
}
