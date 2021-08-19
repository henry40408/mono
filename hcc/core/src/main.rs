#![deny(
    missing_docs,
    missing_debug_implementations,
    missing_copy_implementations,
    trivial_casts,
    trivial_numeric_casts,
    unsafe_code,
    unstable_features,
    unused_import_braces,
    unused_qualifications
)]

//! HTTPS Certificate Check

use structopt::StructOpt;

use hcc::{CheckClient, CheckResultJSON};

#[derive(Debug, Default, StructOpt)]
#[structopt(author, about)]
struct Opts {
    /// Output in JSON format
    #[structopt(short, long)]
    json: bool,
    /// Verbose mode
    #[structopt(short, long)]
    verbose: bool,
    #[structopt(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, StructOpt)]
enum Command {
    /// Check domain name(s) immediately
    #[structopt()]
    Check {
        /// Grace period in days
        #[structopt(short, long = "grace", default_value = "7")]
        grace_in_days: i64,
        /// One or many domain names to check
        #[structopt()]
        domain_names: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts: Opts = Opts::from_args();
    match opts.command {
        Some(Command::Check {
            ref domain_names,
            grace_in_days,
        }) => {
            let domain_names: Vec<&str> = domain_names.iter().map(AsRef::as_ref).collect();
            check_command(&opts, &domain_names, grace_in_days).await
        }
        None => Ok(()),
    }
}

async fn check_command<'a>(
    opts: &Opts,
    domain_names: &'a [&str],
    grace_in_days: i64,
) -> anyhow::Result<()> {
    let client = CheckClient::builder()
        .elapsed(opts.verbose)
        .grace_in_days(grace_in_days)
        .build();

    let results = client.check_certificates(domain_names).await?;

    if opts.json {
        let s = if results.len() > 1 {
            let json: Vec<CheckResultJSON> = results.iter().map(CheckResultJSON::new).collect();
            serde_json::to_string(&json)?
        } else {
            let result = results.get(0).unwrap();
            let json = CheckResultJSON::new(result);
            serde_json::to_string(&json)?
        };
        println!("{0}", s);
    } else {
        for r in results {
            println!("{0}", r);
        }
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use crate::{check_command, Opts};

    fn build_opts(json: bool) -> Opts {
        Opts {
            json,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_check_command() -> anyhow::Result<()> {
        let opts = build_opts(false);
        check_command(&opts, &vec!["sha512.badssl.com"], 7).await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_check_command_json() -> anyhow::Result<()> {
        let opts = build_opts(true);
        check_command(&opts, &vec!["sha512.badssl.com"], 7).await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_check_command_expired() -> anyhow::Result<()> {
        let opts = build_opts(false);
        check_command(&opts, &vec!["expired.badssl.com"], 7).await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_check_command_expired_json() -> anyhow::Result<()> {
        let opts = build_opts(true);
        check_command(&opts, &vec!["expired.badssl.com"], 7).await?;
        Ok(())
    }
}
