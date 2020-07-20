use anyhow::{anyhow, Result};
use clap::Clap;
use futures::{StreamExt, TryStreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams, LogParams, Meta},
    Client, Config,
};
// use log::info;
use std::collections::HashMap;
use std::io;
use std::io::Write;
use termion::{color, style};

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
}

#[derive(Debug, Clap)]
struct LogsOpts {
    #[clap(long = "pod")]
    pod: Option<String>,
    #[clap(short = 'f')]
    follow: bool,
    #[clap(short = 'n', default_value = "nuwolf")]
    namespace: String,
    #[clap(short = 't', long = "tail-length", default_value = "100")]
    tail_length: i64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts: Opts = Opts::parse();

    match opts.subcmd {
        SubCmd::Logs(o) => match o.pod {
            Some(p) => {
                stream_logs(&o.namespace, &p, o.tail_length).await?;
            }
            None => {
                // println!("Ops: {:?}", o);
                let pods = list_pods(&o.namespace).await?;
                let mut input = String::new();
                print!("Enter Pod number: ");
                io::stdout().flush()?;
                match io::stdin().read_line(&mut input) {
                    Ok(_) => {
                        let index = input.trim().parse::<usize>()?;
                        if let Some(p) = pods.get(&index) {
                            println!("{}", p);
                            stream_logs(&o.namespace, &p, o.tail_length).await?;
                        }
                    }
                    Err(error) => println!("error: {}", error),
                }
            }
        },
    }

    Ok(())
}

async fn list_pods(namespace: &str) -> Result<HashMap<usize, String>> {
    std::env::set_var("RUST_LOG", "info");
    env_logger::init();
    let mut client_config = Config::infer().await?;
    client_config.timeout = std::time::Duration::from_secs(60 * 60);
    let client = Client::new(client_config);

    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let mut lp = ListParams::default();
    lp.timeout = None;
    let mut pod_map = HashMap::new();
    for (i, p) in (pods.list(&lp).await?).into_iter().enumerate() {
        println!("\t{}: {}", i, Meta::name(&p));
        pod_map.insert(i, Meta::name(&p));
    }

    Ok(pod_map)
}

async fn stream_logs(namespace: &str, pod_name: &str, tail_lines: i64) -> Result<()> {
    let mut client_config = Config::infer().await?;
    client_config.timeout = std::time::Duration::from_secs(60 * 60);
    let client = Client::new(client_config);

    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let mut lp = LogParams::default();
    lp.follow = true;
    lp.pretty = true;
    lp.tail_lines = Some(tail_lines);
    let mut logs = pods.log_stream(pod_name, &lp).await?.boxed();
    println!("{}Test", color::Fg(color::White));
    while let Some(line) = logs.try_next().await? {
        let line_str = String::from_utf8((&line).to_vec())?;
        if line_str.contains("ERROR") || line_str.contains("error") {
            println!(
                "{}{}{}{}",
                color::Fg(color::Red),
                style::Bold,
                line_str,
                style::Reset
            );
        } else {
            println!("{}{}", color::Fg(color::White), line_str);
        }
    }

    Ok(())
}
