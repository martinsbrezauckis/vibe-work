//! Standard argv parsing for read-only gh requests, used at the broker's execution boundary.
//! Only supported commands, flags and GitHub destinations are permitted.

use clap::{Arg, ArgAction, Command};
use url::Url;

const READ_ONLY: &str = "only supported read-only gh commands and flags are allowed; gh api requires GET without fields or input";
const DESTINATION: &str =
    "reads must target github.com; API reads must target https://api.github.com";

pub(crate) fn validate(argv: &[String]) -> Result<(), &'static str> {
    let matches = schema().try_get_matches_from(argv).map_err(|_| READ_ONLY)?;
    if let Some(repos) = matches.get_many::<String>("repo") {
        for repo in repos {
            repository(repo)?;
        }
    }
    let (group, command) = matches.subcommand().ok_or(READ_ONLY)?;
    let command = if group == "api" {
        command
    } else {
        command.subcommand().ok_or(READ_ONLY)?.1
    };
    if command
        .try_get_many::<String>("jq")
        .ok()
        .flatten()
        .is_some()
    {
        return Err(
            "in-process --jq can read credentials; pipe JSON output to a separate jq process",
        );
    }
    if group == "api" {
        let endpoint = command.get_one::<String>("selector").ok_or(READ_ONLY)?;
        let url = Url::parse("https://api.github.com/")
            .and_then(|base| base.join(endpoint))
            .map_err(|_| DESTINATION)?;
        destination(&url, "api.github.com")?;
        if url.path() == "/graphql" {
            return Err(
                "raw GraphQL is not supported; use the listed gh read commands or REST GET",
            );
        }
        if let Some(headers) = command.get_many::<String>("header") {
            for header in headers {
                let (name, value) = header.split_once(':').ok_or(READ_ONLY)?;
                let name = name.parse::<http::HeaderName>().map_err(|_| READ_ONLY)?;
                let _value = value.parse::<http::HeaderValue>().map_err(|_| READ_ONLY)?;
                if name != http::header::ACCEPT && name != "x-github-api-version" {
                    return Err("API headers are limited to Accept and X-GitHub-Api-Version");
                }
            }
        }
    } else if let Some(selector) = command.try_get_one::<String>("selector").ok().flatten() {
        if group == "repo" {
            repository(selector)?;
        } else if let Ok(url) = Url::parse(selector) {
            destination(&url, "github.com")?;
        }
    }
    Ok(())
}

fn destination(url: &Url, host: &str) -> Result<(), &'static str> {
    if url.scheme() != "https"
        || url.host_str() != Some(host)
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(DESTINATION);
    }
    Ok(())
}

fn repository(repo: &str) -> Result<(), &'static str> {
    let parts: Vec<_> = repo.split('/').collect();
    let url = match parts.as_slice() {
        [owner, name] if !owner.is_empty() && !name.is_empty() => {
            Url::parse(&format!("https://github.com/{repo}"))
        }
        ["github.com", owner, name] if !owner.is_empty() && !name.is_empty() => {
            Url::parse(&format!("https://{repo}"))
        }
        _ => Url::parse(repo),
    }
    .map_err(|_| DESTINATION)?;
    destination(&url, "github.com")?;
    if url.path_segments().map_or(0, Iterator::count) != 2 || url.query().is_some() {
        return Err(DESTINATION);
    }
    Ok(())
}

fn schema() -> Command {
    Command::new("gh")
        .no_binary_name(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .subcommand_required(true)
        .arg(value("repo", Some('R')).global(true))
        .arg(flag("help", Some('h')).global(true))
        .subcommand(
            Command::new("pr")
                .subcommand_required(true)
                .subcommand(json(selector("view", false)).arg(flag("comments", Some('c'))))
                .subcommand(pr_list())
                .subcommand(flags(
                    values(json(selector("checks", false)), &[("interval", Some('i'))]),
                    &[("required", None), ("watch", None), ("fail-fast", None)],
                ))
                .subcommand(flags(
                    values(selector("diff", false), &[("color", None)]),
                    &[("patch", None), ("name-only", None)],
                )),
        )
        .subcommand(
            Command::new("run")
                .subcommand_required(true)
                .subcommand(flags(
                    values(
                        json(selector("view", false)),
                        &[("attempt", Some('a')), ("job", Some('j'))],
                    ),
                    &[
                        ("log", None),
                        ("log-failed", None),
                        ("exit-status", None),
                        ("verbose", Some('v')),
                    ],
                ))
                .subcommand(flags(
                    values(
                        json(Command::new("list")),
                        &[
                            ("branch", Some('b')),
                            ("commit", Some('c')),
                            ("created", None),
                            ("event", Some('e')),
                            ("limit", Some('L')),
                            ("status", Some('s')),
                            ("user", Some('u')),
                            ("workflow", Some('w')),
                        ],
                    ),
                    &[("all", Some('a'))],
                )),
        )
        .subcommand(
            Command::new("issue")
                .subcommand_required(true)
                .subcommand(json(selector("view", true)).arg(flag("comments", Some('c'))))
                .subcommand(flags(
                    values(
                        json(Command::new("list")),
                        &[
                            ("assignee", Some('a')),
                            ("author", Some('A')),
                            ("label", Some('l')),
                            ("limit", Some('L')),
                            ("mention", None),
                            ("milestone", Some('m')),
                            ("search", Some('S')),
                            ("state", Some('s')),
                        ],
                    ),
                    &[],
                )),
        )
        .subcommand(
            Command::new("repo")
                .subcommand_required(true)
                .subcommand(json(selector("view", false)).arg(value("branch", Some('b')))),
        )
        .subcommand(api())
}

fn pr_list() -> Command {
    flags(
        values(
            json(Command::new("list")),
            &[
                ("assignee", Some('a')),
                ("author", Some('A')),
                ("base", Some('B')),
                ("head", Some('H')),
                ("label", Some('l')),
                ("limit", Some('L')),
                ("search", Some('S')),
                ("state", Some('s')),
            ],
        ),
        &[("draft", Some('d'))],
    )
}

fn api() -> Command {
    flags(
        values(
            selector("api", true),
            &[
                ("jq", Some('q')),
                ("template", Some('t')),
                ("cache", None),
                ("preview", Some('p')),
            ],
        ),
        &[
            ("include", Some('i')),
            ("paginate", None),
            ("slurp", None),
            ("silent", None),
            ("verbose", None),
            ("allow-escape-sequences", None),
        ],
    )
    .arg(value("method", Some('X')).value_parser(["GET"]))
    .arg(value("hostname", None).value_parser(["github.com"]))
    .arg(value("header", Some('H')))
}

fn selector(name: &'static str, required: bool) -> Command {
    Command::new(name).arg(Arg::new("selector").num_args(1).required(required))
}

fn json(command: Command) -> Command {
    values(
        command,
        &[("json", None), ("jq", Some('q')), ("template", Some('t'))],
    )
}

fn value(name: &'static str, short: Option<char>) -> Arg {
    let arg = Arg::new(name)
        .long(name)
        .action(ArgAction::Append)
        .num_args(1);
    if let Some(short) = short {
        arg.short(short)
    } else {
        arg
    }
}

fn flag(name: &'static str, short: Option<char>) -> Arg {
    let arg = Arg::new(name).long(name).action(ArgAction::SetTrue);
    if let Some(short) = short {
        arg.short(short)
    } else {
        arg
    }
}

fn values(mut command: Command, options: &[(&'static str, Option<char>)]) -> Command {
    for (name, short) in options {
        command = command.arg(value(name, *short));
    }
    command
}

fn flags(mut command: Command, options: &[(&'static str, Option<char>)]) -> Command {
    for (name, short) in options {
        command = command.arg(flag(name, *short));
    }
    command
}
