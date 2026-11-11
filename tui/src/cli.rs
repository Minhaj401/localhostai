use anyhow::{bail, Context, Result};

#[derive(Debug, Default)]
pub struct Options {
    pub help: bool,
    pub version: bool,
    pub status: bool,
    pub headless: bool,
    pub once: Option<String>,
    pub connect: Option<String>,
    pub port: Option<u16>,
    pub name: Option<String>,
    pub no_lan: bool,
    pub no_discovery: bool,
    pub require_pairing: bool,
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut args = args.into_iter();
        let mut options = Self::default();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => options.help = true,
                "--version" | "-V" => options.version = true,
                "--status" => options.status = true,
                "--headless" => options.headless = true,
                "--no-lan" => options.no_lan = true,
                "--no-discovery" => options.no_discovery = true,
                "--require-pairing" => options.require_pairing = true,
                "--port" => {
                    options.port = Some(
                        args.next()
                            .context("--port needs a number")?
                            .parse()
                            .context("--port must be between 0 and 65535")?,
                    )
                }
                "--name" => options.name = Some(args.next().context("--name needs a name")?),
                "--once" => {
                    let prompt = args.next().context("--once needs a prompt")?;
                    if prompt.trim().is_empty() {
                        bail!("--once needs a nonempty prompt");
                    }
                    options.once = Some(prompt);
                }
                "--connect" => {
                    let url = args.next().context("--connect needs an HTTP(S) URL")?;
                    let parsed = reqwest::Url::parse(&url).context("invalid --connect URL")?;
                    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                        bail!("--connect needs an HTTP(S) URL");
                    }
                    options.connect = Some(url);
                }
                _ => bail!("unknown option {arg}\n\nRun `privateml --help` for usage."),
            }
        }
        if usize::from(options.status)
            + usize::from(options.headless)
            + usize::from(options.once.is_some())
            > 1
        {
            bail!("choose only one of --status, --once, and --headless");
        }
        if options.headless && options.connect.is_some() {
            bail!("--headless starts a node and cannot be combined with --connect");
        }
        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options> {
        Options::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn values_are_not_reinterpreted_as_flags() {
        let options = parse(&["--once", "--help", "--require-pairing"]).unwrap();
        assert_eq!(options.once.as_deref(), Some("--help"));
        assert!(!options.help);
        assert!(options.require_pairing);
    }

    #[test]
    fn rejects_missing_and_invalid_arguments_before_starting_a_node() {
        for args in [
            vec!["--port"],
            vec!["--port", "oops"],
            vec!["--port", "65536"],
            vec!["--name"],
            vec!["--once"],
            vec!["--once", " "],
            vec!["--typo"],
            vec!["--connect", "http-wrong"],
            vec!["--connect", "file:///tmp/x"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn remote_nodes_support_single_shot_and_status_modes() {
        assert!(parse(&["--connect", "http://localhost:8765", "--once", "hi"]).is_ok());
        assert!(parse(&["--connect", "http://localhost:8765", "--status"]).is_ok());
        assert!(parse(&["--headless", "--once", "hi"]).is_err());
    }
}
