use anyhow::Result;
use clap::Clap;
use futures::{StreamExt, TryStreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams, LogParams, Meta},
    Client, Config,
};
// use log::info;
use futures::future::join_all;
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::HashMap;
use std::io;
use std::io::Write;
use termion::{color, style};
use tokio::task;

lazy_static! {
    static ref COLORS: Vec<color::Rgb> = vec![
        color::Rgb(0, 255, 0),
        color::Rgb(128, 128, 0),
        color::Rgb(0, 255, 255),
        color::Rgb(255, 192, 203),
        color::Rgb(245, 255, 250),
        color::Rgb(221, 160, 221),
        color::Rgb(154, 205, 50),
        color::Rgb(230, 230, 250),
    ];
}

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
    #[clap(short = 'p', long = "pattern")]
    pattern: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts: Opts = Opts::parse();

    match opts.subcmd {
        SubCmd::Logs(o) => match o.pod {
            Some(p) => {
                let c = get_color()?;
                stream_logs(o.namespace, p, o.tail_length, c).await?;
            }
            None => {
                match o.pattern {
                    Some(p) => {
                        let pods = collect_pods(&o.namespace, &p).await?;
                        println!("Pods: {:?}", pods);
                        let mut tasks = vec![];
                        for name in pods {
                            let c = get_color()?;
                            let n = o.namespace.clone();
                            let pn = name.clone();
                            let t = task::spawn(stream_logs(n, pn, o.tail_length, c));
                            tasks.push(t);
                        }

                        join_all(tasks).await;
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
                                    let c = get_color()?;
                                    stream_logs(o.namespace, p.clone(), o.tail_length, c).await?;
                                }
                            }
                            Err(error) => println!("error: {}", error),
                        }
                    }
                }
            }
        },
    }

    Ok(())
}

fn get_color() -> Result<color::Rgb> {
    use rand::Rng;

    let mut rng = rand::thread_rng();
    let index = rng.gen_range(0.0, COLORS.len() as f32) as usize;
    let rgb = COLORS[index];

    Ok(rgb)
}

async fn list_pods(namespace: &str) -> Result<HashMap<usize, String>> {
    std::env::set_var("RUST_LOG", "info");
    env_logger::init();
    let mut client_config = Config::infer().await?;
    // client_config.timeout = std::time::Duration::from_secs(60 * 60 * 24);
    client_config.timeout = None;
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

async fn collect_pods(namespace: &str, pattern: &str) -> Result<Vec<String>> {
    let mut client_config = Config::infer().await?;
    // client_config.timeout = std::time::Duration::from_secs(60 * 60 * 24);
    client_config.timeout = None;
    let client = Client::new(client_config);

    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let mut lp = ListParams::default();
    lp.timeout = None;
    let mut matching_pods = vec![];
    let re: Regex = Regex::new(pattern).unwrap();
    for (_, p) in (pods.list(&lp).await?).into_iter().enumerate() {
        // lazy_static! {
        //     static ref RE: Regex = Regex::new(pattern).unwrap();
        // }
        let name = Meta::name(&p).to_string();
        if re.is_match(&name) {
            matching_pods.push(name);
        }
    }

    Ok(matching_pods)
}

async fn stream_logs(
    namespace: String,
    pod_name: String,
    tail_lines: i64,
    c: color::Rgb,
) -> Result<()> {
    let mut client_config = Config::infer().await?;
    // client_config.timeout = std::time::Duration::from_secs(60 * 60 * 24);
    client_config.timeout = None;
    let client = Client::new(client_config);

    let pods: Api<Pod> = Api::namespaced(client, &namespace);
    let mut lp = LogParams::default();
    lp.follow = true;
    lp.pretty = true;
    lp.tail_lines = Some(tail_lines);
    let mut logs = pods.log_stream(&pod_name, &lp).await?.boxed();
    println!("{}", color::Fg(color::White));
    while let Some(line) = logs.try_next().await? {
        let line_str = String::from_utf8((&line).to_vec())?;
        if line_str.contains("ERROR") || line_str.contains("error") {
            print!("{}{}  ", color::Fg(c), pod_name);
            println!(
                "{}{}{}{}",
                color::Fg(color::Red),
                style::Bold,
                line_str,
                style::Reset
            );
        } else {
            print!("{}{}  ", color::Fg(c), pod_name);
            println!("{}{}", color::Fg(color::White), line_str);
        }
    }

    Ok(())
}
