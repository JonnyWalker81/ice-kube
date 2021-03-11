use crate::util::OptionEx;
use anyhow::Result;
use clap::Clap;

mod logs;
mod ui;
mod util;

#[derive(Clap)]
#[clap(version = "0.1.0", author = "Jonathan Rothberg")]
struct Opts {
    #[clap(subcommand)]
    subcmd: SubCmd,
}

#[derive(Clap)]
enum SubCmd {
    #[clap(name = "logs")]
    Logs(LogsOpts),
    #[clap(name = "ui")]
    UI(UIOpts),
}

#[derive(Debug, Clap)]
pub struct LogsOpts {
    #[clap(long = "pod")]
    pod: Option<String>,
    #[clap(short = 'f')]
    follow: bool,
    #[clap(short = 'n', default_value = "nuwolf")]
    namespace: String,
    #[clap(short = 't', long = "tail-length", default_value = "100")]
    tail_length: i64,
    #[clap(short = 'p', long = "pattern")]
    pattern: Option<String>,
    #[clap(short = 'r', long = "terms")]
    terms: Option<String>,
    #[clap(short = 'l', long = "highlight")]
    highlight: Option<String>,
}

#[derive(Debug, Clap)]
pub struct UIOpts {
    #[clap(short = 'n', default_value = "nuwolf")]
    namespace: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts: Opts = Opts::parse();

    match run(&opts).await {
        Ok(_) => Ok(()),
        Err(e) => {
            eprintln!("Error in main: {:?}", e);
            Err(e)
        }
    }
}

async fn run(opts: &Opts) -> Result<()> {
    match &opts.subcmd {
        SubCmd::Logs(o) => match &o.pod {
            Some(p) => {
                let c = logs::get_color()?;

                let h = o.highlight.to_str();

                logs::stream_logs(o.namespace.clone(), p.to_string(), o.tail_length, c, h).await?;
            }
            None => match o.pattern {
                Some(ref p) => {
                    logs::follow_logs(&o, &p).await?;
                }
                None => {
                    logs::select_pod(&o).await?;
                }
            },
        },
        SubCmd::UI(o) => {
            ui::load_ui(&o.namespace, &o).await?;
        }
    }

    Ok(())
}
