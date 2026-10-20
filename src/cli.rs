//! Command-line arguments. The bridge runs with none; `--help` and
//! `--version` answer and exit, so trying one never starts a second bridge.

pub const USAGE: &str = "\
soloist-mpris: Spotify Soloist as an MPRIS player

Usage: soloist-mpris [--help | --version]

Run without arguments, normally by soloist-mpris.service. It follows
Soloist's local WebSocket (from ws.addr and ws.port in Soloist's data
directory, or $SOLOIST_DATA_DIR) and publishes org.mpris.MediaPlayer2.soloist.";

#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    Run,
    Help,
    Version,
    Unknown(String),
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Cli {
    let mut args = args.into_iter();
    let cli = match args.next().as_deref() {
        None => return Cli::Run,
        Some("--help" | "-h") => Cli::Help,
        Some("--version" | "-V") => Cli::Version,
        Some(other) => return Cli::Unknown(other.to_owned()),
    };
    match args.next() {
        None => cli,
        Some(extra) => Cli::Unknown(extra),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(args: &[&str]) -> Cli {
        parse(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn no_arguments_runs_the_bridge() {
        assert_eq!(parse_str(&[]), Cli::Run);
    }

    #[test]
    fn help_and_version_answer_instead_of_running() {
        assert_eq!(parse_str(&["--help"]), Cli::Help);
        assert_eq!(parse_str(&["-h"]), Cli::Help);
        assert_eq!(parse_str(&["--version"]), Cli::Version);
        assert_eq!(parse_str(&["-V"]), Cli::Version);
    }

    #[test]
    fn anything_else_is_refused() {
        assert_eq!(parse_str(&["--verbose"]), Cli::Unknown("--verbose".into()));
        assert_eq!(
            parse_str(&["--help", "extra"]),
            Cli::Unknown("extra".into())
        );
    }
}
